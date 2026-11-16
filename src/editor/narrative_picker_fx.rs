//! Short UI transitions only: the picker does not require an idle animation loop.
use super::*;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(super) struct Material {
    #[uniform(0)]
    metrics: Vec4,
    #[uniform(1)]
    motion: Vec4,
    #[uniform(2)]
    tint: Vec4,
}
impl UiMaterial for Material {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("narrative_picker.wesl"))
                .with_source("embedded"),
        )
    }
}
#[derive(Resource, Default)]
pub(super) struct State {
    pub confirmation: Option<(usize, u32, f64)>,
}
#[derive(Component)]
pub(super) struct Glow {
    owner: Entity,
    mode: u8,
    selected: bool,
    position: Option<usize>,
    node: u32,
    level: f32,
    entered: f64,
    wanted: bool,
}
#[derive(Component)]
pub(super) struct Fade {
    text: Option<f32>,
    image: Option<f32>,
    background: Option<f32>,
    panel: bool,
}

pub(super) fn register(app: &mut App) {
    embedded_asset!(app, "narrative_picker.wesl");
    app.add_plugins(UiMaterialPlugin::<Material>::default())
        .init_resource::<State>();
}
pub(super) fn layer(
    commands: &mut Commands,
    owner: Entity,
    mode: u8,
    selected: bool,
    position: Option<usize>,
    node_id: u32,
) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            left: px(0),
            top: px(0),
            ..default()
        },
        Glow {
            owner,
            mode,
            selected,
            position,
            node: node_id,
            level: 0.,
            entered: 0.,
            wanted: false,
        },
        Pickable::IGNORE,
        ZIndex(1),
        ChildOf(owner),
    ));
}
pub(super) fn tag(commands: &mut Commands, panel: Entity) {
    commands.queue(move |world: &mut World| {
        let mut pending = vec![panel];
        while let Some(entity) = pending.pop() {
            if let Some(children) = world.get::<Children>(entity) {
                pending.extend(children.iter());
            }
            if world.get::<Fade>(entity).is_some() {
                continue;
            }
            let fade = Fade {
                panel: entity == panel,
                text: world
                    .get::<TextColor>(entity)
                    .map(|c| c.0.to_linear().alpha),
                image: world
                    .get::<ImageNode>(entity)
                    .map(|c| c.color.to_linear().alpha),
                background: world
                    .get::<BackgroundColor>(entity)
                    .map(|c| c.0.to_linear().alpha),
            };
            if fade.text.is_some() || fade.image.is_some() || fade.background.is_some() {
                world.entity_mut(entity).insert(fade);
            }
        }
    });
}
pub(super) fn animate(
    mut commands: Commands,
    menu: Res<speaker_menu::Menu>,
    state: Res<State>,
    editor: Res<Editor>,
    time: Res<Time>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut redraw: MessageWriter<bevy::window::RequestRedraw>,
    mut materials: ResMut<Assets<Material>>,
    owners: Query<(&ComputedNode, Option<&Hovered>)>,
    mut layers: Query<(
        Entity,
        &mut Glow,
        Option<&MaterialNode<Material>>,
        &mut Node,
    )>,
    mut fades: Query<(
        &Fade,
        Option<&mut TextColor>,
        Option<&mut ImageNode>,
        Option<&mut BackgroundColor>,
    )>,
    mut panels: Query<&mut UiTransform>,
    mut borders: Query<&mut BorderColor>,
) {
    let now = time.elapsed_secs_f64();
    let progress = menu.progress(now);
    let mut active = menu.visible() && progress < 0.999;
    if let Ok(mut transform) = panels.get_mut(menu.panel()) {
        let mut next = UiTransform::from_scale(Vec2::splat(0.988 + 0.012 * progress));
        next.translation = Val2::px(
            0.,
            (1. - progress) * if menu.opens_down() { -5. } else { 5. },
        );
        transform.set_if_neq(next);
    }
    if let Ok(mut border) = borders.get_mut(menu.panel()) {
        border.set_if_neq(BorderColor::all(FOCUS.with_alpha(0.32 * progress)));
    }
    for (fade, text, image, background) in &mut fades {
        if let (Some(base), Some(mut color)) = (fade.text, text) {
            let next = TextColor(color.0.with_alpha(base * progress));
            color.set_if_neq(next);
        }
        if let (Some(base), Some(mut image)) = (fade.image, image) {
            let next = image.color.with_alpha(base * progress);
            if image.color != next {
                image.color = next;
            }
        }
        if let (Some(base), Some(mut color)) = (fade.background, background) {
            let next = BackgroundColor(
                color
                    .0
                    .with_alpha(base * if fade.panel { 1. } else { progress }),
            );
            color.set_if_neq(next);
        }
    }
    for (entity, mut glow, handle, mut node) in &mut layers {
        let Ok((computed, hovered)) = owners.get(glow.owner) else {
            continue;
        };
        let confirmation = state
            .confirmation
            .filter(|(doc, n, _)| Some(*doc) == editor.narrative.active && *n == glow.node);
        let phase = if glow.mode == 2 {
            confirmation.map_or(1., |(_, _, at)| ((now - at) / 0.34).clamp(0., 1.) as f32)
        } else if glow.mode == 0 {
            progress
        } else {
            ((now - glow.entered) / 0.28).clamp(0., 1.) as f32
        };
        let wanted = menu.visible()
            && (hovered.is_some_and(|h| h.0)
                || glow.position.is_some_and(|p| menu.keyboard_row(p)));
        if wanted && !glow.wanted {
            glow.entered = now;
        }
        glow.wanted = wanted;
        let target = if wanted { 1. } else { 0. };
        let next = glow.level + (target - glow.level) * (1. - (-time.delta_secs() / 0.045).exp());
        glow.level = if (next - target).abs() < 0.002 {
            target
        } else {
            next
        };
        active |= glow.mode == 1 && ((glow.level - target).abs() > 0.002 || wanted && phase < 1.);
        active |= glow.mode == 2 && phase < 1.;
        let display = if glow.mode == 2 && phase >= 1. {
            Display::None
        } else {
            Display::Flex
        };
        if node.display != display {
            node.display = display;
        }
        let size = computed.size() / window.scale_factor();
        let tint = FOCUS.to_linear();
        let value = Material {
            metrics: Vec4::new(
                size.x,
                size.y,
                if glow.mode == 2 { 1. } else { progress },
                glow.level,
            ),
            motion: Vec4::new(
                phase,
                if glow.selected { 1. } else { 0. },
                glow.mode as f32,
                0.,
            ),
            tint: Vec4::new(tint.red, tint.green, tint.blue, 1.),
        };
        if let Some(handle) = handle {
            if let Some(mut material) = materials.get_mut(&handle.0) {
                if material.metrics != value.metrics || material.motion != value.motion {
                    *material = value;
                }
            }
        } else {
            commands
                .entity(entity)
                .insert(MaterialNode(materials.add(value)));
        }
    }
    if active {
        redraw.write(bevy::window::RequestRedraw);
    }
}
