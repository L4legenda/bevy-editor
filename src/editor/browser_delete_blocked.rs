//! A transient protective seal on a card whose document is still open.
use super::*;

pub(super) const DURATION: f32 = 2.6;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(super) struct BlockedMaterial {
    #[uniform(0)]
    pub state: Vec4,
}
impl UiMaterial for BlockedMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("asset_delete_blocked.wesl"))
                .with_source("embedded"),
        )
    }
}

#[derive(Message, Clone)]
pub(super) struct Attempt(pub Target);

pub(super) struct Active {
    target: Target,
    pub root: Entity,
    material: Handle<BlockedMaterial>,
    labels: [Entity; 2],
    pub age: f32,
}
#[derive(Resource, Default)]
pub(super) struct Feedback {
    pub active: Option<Active>,
}

pub(super) fn register(app: &mut App) {
    embedded_asset!(app, "asset_delete_blocked.wesl");
    app.add_plugins(UiMaterialPlugin::<BlockedMaterial>::default())
        .add_message::<Attempt>()
        .init_resource::<Feedback>()
        .add_systems(
            Update,
            animate
                .after(super::update)
                .after(crate::editor::ui::refresh_browser),
        );
}

fn spawn(
    commands: &mut Commands,
    materials: &mut Assets<BlockedMaterial>,
    interface: &Interface,
    target: Target,
) -> Active {
    let material = materials.add(BlockedMaterial { state: Vec4::ZERO });
    // Parenting to the card preserves scrolling, native clipping and layout.
    // The original card and its controls stay available throughout the effect.
    let root = commands
        .spawn((
            MaterialNode(material.clone()),
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: percent(100),
                height: percent(100),
                border_radius: BorderRadius::all(px(9)),
                padding: UiRect {
                    top: px(54),
                    left: px(8),
                    right: px(8),
                    ..default()
                },
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..column(6.0)
            },
            ZIndex(20),
            Pickable::IGNORE,
            AccessibleLabel("Удаление невозможно: файл открыт в редакторе".into()),
            ChildOf(target.card),
        ))
        .id();
    let title = text(
        commands,
        root,
        &interface.font,
        "Удаление недоступно",
        13.0,
        TEXT,
    );
    let hint = text(
        commands,
        root,
        &interface.font,
        "Сначала закройте\nвкладку файла",
        11.5,
        TEXT,
    );
    for entity in [title, hint] {
        commands.entity(entity).insert((
            TextLayout {
                justify: Justify::Center,
                ..default()
            },
            TextColor(TEXT.with_alpha(0.0)),
            Node {
                max_width: percent(100),
                ..default()
            },
        ));
    }
    Active {
        target,
        root,
        material,
        labels: [title, hint],
        age: 0.0,
    }
}

fn animate(
    mut commands: Commands,
    mut attempts: MessageReader<Attempt>,
    mut feedback: ResMut<Feedback>,
    editor: Res<Editor>,
    browser: Res<Browser>,
    interface: Res<Interface>,
    time: Res<Time>,
    window: Single<&Window, With<PrimaryWindow>>,
    nodes: Query<&Node>,
    mut colors: Query<&mut TextColor>,
    mut materials: ResMut<Assets<BlockedMaterial>>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    if let Some(Attempt(target)) = attempts.read().last().cloned() {
        if let Some(active) = feedback
            .active
            .as_mut()
            .filter(|active| active.target.card == target.card && nodes.contains(active.root))
        {
            active.age = 0.0;
        } else {
            if let Some(old) = feedback.active.take() {
                commands.entity(old.root).try_despawn();
            }
            feedback.active = Some(spawn(&mut commands, &mut materials, &interface, target));
        }
    }
    let Some(mut active) = feedback.active.take() else {
        return;
    };
    active.age += time.delta_secs().min(0.05);
    if active.age >= DURATION
        || !editor.drawer
        || editor.pending.is_some()
        || editor.hdri_library_open
        || active.target.revision != browser.scan_revision
        || !nodes.contains(active.target.card)
        || !super::is_open(&editor, &active.target.path)
    {
        commands.entity(active.root).try_despawn();
        redraw.write(RequestRedraw);
        return;
    }
    let smooth = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    let opacity = smooth(active.age / 0.10) * (1.0 - smooth((active.age - 1.9) / 0.7));
    if let Some(mut material) = materials.get_mut(&active.material) {
        material.state = Vec4::new(active.age, window.scale_factor(), opacity, 0.0);
    }
    for label in active.labels {
        if let Ok(mut color) = colors.get_mut(label) {
            color.0 = TEXT.with_alpha(opacity);
        }
    }
    redraw.write(RequestRedraw);
    feedback.active = Some(active);
}
