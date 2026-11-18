//! A solid colour swatch, a hold ring, elastic pickup/return, and a brief landing ripple.
use super::*;
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

pub(super) const OFFSET: Vec2 = Vec2::new(19.0, -25.0);

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
struct DropMaterial {
    // Kind, local age, reveal/progress, valid destination.
    #[uniform(0)]
    state: Vec4,
    // Linear colour and physical pixels per logical pixel.
    #[uniform(1)]
    tint: Vec4,
}
impl UiMaterial for DropMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("image_color_drop_fx.wesl"))
                .with_source("embedded"),
        )
    }
}

pub(in crate::editor) struct ColorDropPlugin;
impl Plugin for ColorDropPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "image_color_drop_fx.wesl");
        app.init_resource::<ColorDrop>()
            .init_resource::<Motion>()
            .add_plugins(UiMaterialPlugin::<DropMaterial>::default())
            .add_systems(
                Update,
                animate
                    .after(crate::editor::ui::refresh_photo_editor)
                    .after(crate::editor::ui::image_viewer_cursor),
            );
    }
}

#[derive(Resource)]
struct Overlay {
    orb: Entity,
    charge: Entity,
    target: Entity,
    splash: Entity,
    hint: Entity,
    caption: Entity,
}
#[derive(Resource, Default)]
struct Motion {
    showing: bool,
    handles: HashMap<Entity, Handle<DropMaterial>>,
    previous: Option<Vec2>,
    speed: f32,
    direction: f32,
}

pub(in crate::editor) fn setup(commands: &mut Commands, root: Entity, font: &Handle<Font>) {
    let mut surface = |label: &str, side: f32| {
        commands
            .spawn((
                Node {
                    display: Display::None,
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(0),
                    width: px(side),
                    height: px(side),
                    ..default()
                },
                GlobalZIndex(1800),
                Pickable::IGNORE,
                UiTransform::default(),
                AccessibleLabel(label.into()),
                ChildOf(root),
            ))
            .id()
    };
    let orb = surface("Переносимый цвет", 80.0);
    let charge = surface("Удержание цвета", 72.0);
    let target = surface("Точка заливки", 24.0);
    let splash = surface("Волна заливки", 184.0);
    let hint = commands
        .spawn((
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                padding: UiRect::axes(px(9), px(5)),
                border_radius: BorderRadius::all(px(8)),
                border: UiRect::all(px(1)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.027, 0.035, 0.94)),
            BorderColor::all(EDGE),
            GlobalZIndex(1801),
            Pickable::IGNORE,
            UiTransform::default(),
            ChildOf(root),
        ))
        .id();
    let caption = text(commands, hint, font, "Залить область", 11.0, TEXT);
    commands.insert_resource(Overlay {
        orb,
        charge,
        target,
        splash,
        hint,
        caption,
    });
}

fn ease_out_back(t: f32) -> f32 {
    let p = t.clamp(0.0, 1.0) - 1.0;
    1.0 + 2.35 * p.powi(3) + 1.35 * p.powi(2)
}

fn animate(
    mut commands: Commands,
    time: Res<Time<Real>>,
    editor: Res<Editor>,
    viewer: Res<ImageViewer>,
    window: Single<&Window, With<PrimaryWindow>>,
    overlay: Res<Overlay>,
    mut drop: ResMut<ColorDrop>,
    mut motion: ResMut<Motion>,
    mut nodes: Query<&mut Node>,
    mut transforms: Query<&mut UiTransform>,
    mut labels: Query<&mut Text>,
    mut materials: ResMut<Assets<DropMaterial>>,
) {
    if drop.carry.is_none() && drop.feedback.is_none() && !motion.showing {
        return;
    }
    let now = time.elapsed_secs_f64();
    let dt = time.delta_secs().clamp(0.001, 0.05);
    // Animation never survives a closed tab, a modal, or a lost OS focus.
    let visible = ready(&editor, &viewer) && window.focused && window.cursor_position().is_some();
    let mut surfaces = Vec::with_capacity(3);
    let mut hint = None;
    let mut bead_scale = Vec2::ONE;
    let mut bead_angle = 0.0;
    if visible && let Some(carry) = drop.carry.as_ref() {
        if let Some(lifted) = carry.lifted {
            let age = (now - lifted) as f32;
            let pop = ease_out_back(age / 0.2);
            let position = carry
                .origin
                .lerp(drop.cursor + OFFSET, (age / 0.12).clamp(0.0, 1.0));
            let velocity = motion
                .previous
                .map_or(Vec2::ZERO, |previous| (drop.cursor - previous) / dt);
            let speed = (velocity.length() / 1800.0).clamp(0.0, 1.0);
            motion.speed += (speed - motion.speed) * (1.0 - (-dt * 18.0).exp());
            if velocity.length_squared() > 25.0 {
                motion.direction = velocity.y.atan2(velocity.x);
            }
            motion.previous = Some(drop.cursor);
            bead_scale = Vec2::new(1.0 + motion.speed * 0.19, 1.0 - motion.speed * 0.075)
                * (0.65 + pop * 0.35);
            bead_angle = motion.direction;
            surfaces.push((overlay.orb, position, 1.0, age, 1.0, carry.color));
            if drop.target.is_some() {
                surfaces.push((overlay.target, drop.cursor, 2.0, age, 1.0, carry.color));
            }
            hint = Some((
                drop.cursor,
                if drop.target.is_some() {
                    "Залить область"
                } else {
                    "Перенесите на изображение"
                },
            ));
        } else {
            motion.previous = None;
            motion.speed = 0.0;
            let progress = ((now - carry.started) / HOLD_SECONDS).clamp(0.0, 1.0) as f32;
            surfaces.push((
                overlay.charge,
                carry.origin,
                0.0,
                0.0,
                progress,
                carry.color,
            ));
        }
    } else {
        motion.previous = None;
        if visible && let Some(feedback) = drop.feedback {
            match feedback {
                Feedback::Land { at, color, started } => {
                    let age = (now - started) as f32;
                    if age < 0.52 {
                        surfaces.push((overlay.splash, at, 3.0, age, 1.0, color));
                    } else {
                        drop.feedback = None;
                    }
                }
                Feedback::Return {
                    from,
                    to,
                    color,
                    started,
                } => {
                    let age = (now - started) as f32;
                    if age < 0.25 {
                        let p = (age / 0.25).clamp(0.0, 1.0);
                        let position = from.lerp(to, 1.0 - (1.0 - p).powi(3));
                        bead_scale = Vec2::splat(1.0 - p * 0.7);
                        surfaces.push((overlay.orb, position, 1.0, age, 1.0 - p * p, color));
                    } else {
                        drop.feedback = None;
                    }
                }
            }
        } else if !visible {
            drop.feedback = None;
        }
    }
    motion.showing = !surfaces.is_empty() || hint.is_some();
    for entity in [overlay.orb, overlay.charge, overlay.target, overlay.splash] {
        let Ok(mut node) = nodes.get_mut(entity) else {
            continue;
        };
        let Some(&(_, position, kind, age, reveal, color)) =
            surfaces.iter().find(|s| s.0 == entity)
        else {
            if node.display != Display::None {
                node.display = Display::None;
            }
            continue;
        };
        if node.display != Display::Flex {
            node.display = Display::Flex;
        }
        let side = if entity == overlay.splash {
            184.0
        } else if entity == overlay.target {
            24.0
        } else if entity == overlay.charge {
            72.0
        } else {
            80.0
        };
        let handle = motion
            .handles
            .entry(entity)
            .or_insert_with(|| {
                let handle = materials.add(DropMaterial {
                    state: Vec4::ZERO,
                    tint: Vec4::ZERO,
                });
                commands.entity(entity).insert(MaterialNode(handle.clone()));
                handle
            })
            .clone();
        if let Some(mut material) = materials.get_mut(&handle) {
            let rgb = Color::srgba(color[0], color[1], color[2], 1.0).to_linear();
            material.state = Vec4::new(
                kind,
                age,
                reveal,
                if drop.target.is_some() { 1.0 } else { 0.0 },
            );
            material.tint = Vec4::new(rgb.red, rgb.green, rgb.blue, window.scale_factor());
        }
        if let Ok(mut transform) = transforms.get_mut(entity) {
            // UI transforms move the visual without invalidating flex layout on
            // every pointer sample. The overlay never takes part in picking.
            let translation = Val2::px(position.x - side * 0.5, position.y - side * 0.5);
            if transform.translation != translation {
                transform.translation = translation;
            }
            let scale = if entity == overlay.orb {
                bead_scale
            } else {
                Vec2::ONE
            };
            let rotation = Rot2::radians(if entity == overlay.orb {
                bead_angle
            } else {
                0.0
            });
            if transform.scale != scale {
                transform.scale = scale;
            }
            if transform.rotation != rotation {
                transform.rotation = rotation;
            }
        }
    }
    if let Ok(mut node) = nodes.get_mut(overlay.hint) {
        let display = if hint.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        if let Some((point, caption)) = hint {
            if let Ok(mut transform) = transforms.get_mut(overlay.hint) {
                let translation = Val2::px(
                    (point.x + 27.0).min((window.width() - 205.0).max(4.0)),
                    (point.y + 15.0).min((window.height() - 36.0).max(4.0)),
                );
                if transform.translation != translation {
                    transform.translation = translation;
                }
            }
            if let Ok(mut text) = labels.get_mut(overlay.caption) {
                text.set_if_neq(Text(caption.into()));
            }
        }
    }
}
