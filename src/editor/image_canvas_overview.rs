//! Full-document view sharing the canvas texture, independent of zoom and pan.
use super::*;
use bevy::asset::uuid_handle;

const MATERIAL: Handle<CanvasMaterial> = uuid_handle!("deb5e258-4a2e-47c9-af1e-5fc2c3c4c03e");

#[derive(Resource)]
pub(super) struct OverviewUi {
    section: Entity,
    frame: Entity,
    art: Entity,
    checker: Entity,
    visible_area: Entity,
}

pub(super) fn install(app: &mut App) {
    let _ = app
        .world_mut()
        .resource_mut::<Assets<CanvasMaterial>>()
        .insert(MATERIAL.id(), CanvasMaterial::default());
    app.add_systems(Update, refresh.after(super::refresh));
}

pub(super) fn setup(commands: &mut Commands, parent: Entity, checker_image: &Handle<Image>) {
    let section = node(
        commands,
        parent,
        Node {
            display: Display::None,
            width: percent(100),
            flex_shrink: 0.0,
            margin: UiRect::bottom(px(6)),
            ..column(4.0)
        },
    );
    let frame = surface(
        commands,
        section,
        Node {
            width: percent(100),
            height: px(128),
            flex_shrink: 0.0,
            border_radius: BorderRadius::all(px(6)),
            overflow: Overflow::clip(),
            ..default()
        },
        PANEL,
    );
    commands
        .entity(frame)
        .insert(AccessibleLabel("Обзор всего холста".into()));
    let art = surface(
        commands,
        frame,
        Node {
            position_type: PositionType::Absolute,
            overflow: Overflow::clip(),
            ..default()
        },
        Color::NONE,
    );
    let checker = super::checker(commands, art, checker_image);
    commands.spawn((
        MaterialNode(MATERIAL),
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(art),
    ));
    let visible_area = surface(
        commands,
        art,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            border: UiRect::all(px(1.5)),
            ..default()
        },
        Color::NONE,
    );
    commands.entity(visible_area).insert((
        ZIndex(1),
        Pickable::IGNORE,
        BorderColor::all(FOCUS),
        Outline::new(px(1), px(0), Color::srgba(0.0, 0.0, 0.0, 0.8)),
        AccessibleLabel("Видимая область холста".into()),
    ));
    commands.insert_resource(OverviewUi {
        section,
        frame,
        art,
        checker,
        visible_area,
    });
}

pub(super) fn refresh(
    editor: Res<Editor>,
    viewer: Res<ImageViewer>,
    ui: Option<Res<OverviewUi>>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
    computed: Query<&ComputedNode>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut nodes: Query<&mut Node>,
    mut backgrounds: Query<&mut BackgroundColor>,
    mut materials: ResMut<Assets<CanvasMaterial>>,
) {
    let Some(ui) = ui else { return };
    let source = materials
        .get(&canvas_material::HANDLE)
        .filter(|material| material.image.is_some());
    let visible = viewer.edit.editing(editor.viewing_image)
        && viewer.tab == editor.viewing_image
        && viewer.error.is_none()
        && source.is_some()
        && viewer.dimensions.is_some();
    if let Ok(mut section) = nodes.get_mut(ui.section) {
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if section.display != display {
            section.display = display;
        }
    }
    if !visible {
        if let Some(mut material) = materials.get_mut(&MATERIAL)
            && material.image.is_some()
        {
            *material = CanvasMaterial::default();
        }
        return;
    }
    let Some(source) = source else { return };
    let Some(dimensions) = viewer.dimensions else {
        return;
    };
    let sample = CanvasMaterial {
        image: source.image.clone(),
        // Use the selection-free composition, including a live transform or gradient.
        // A zero zoom flag suppresses the document pixel grid in the miniature.
        parameters: Vec4::new(
            dimensions.x as f32,
            dimensions.y as f32,
            source.parameters.z,
            0.0,
        ),
        transform_under: source.transform_under.clone(),
        transform_moving: source.transform_moving.clone(),
        transform_above: source.transform_above.clone(),
        transform_rows: source.transform_rows,
        transform_source: source.transform_source,
        transform_style: source.transform_style,
        transform_frame: source.transform_frame,
        transform_selection: source.transform_selection.clone(),
        ..default()
    };
    if materials
        .get(&MATERIAL)
        .is_some_and(|material| *material != sample)
        && let Some(mut material) = materials.get_mut(&MATERIAL)
    {
        *material = sample;
    }
    if let Ok(mut frame) = nodes.get_mut(ui.frame) {
        let height = px((window.height() * 0.17).clamp(80.0, 148.0));
        if frame.height != height {
            frame.height = height;
        }
    }
    if let Ok(mut checker) = nodes.get_mut(ui.checker) {
        let display = if viewer.background == Backdrop::Checker {
            Display::Flex
        } else {
            Display::None
        };
        if checker.display != display {
            checker.display = display;
        }
    }
    if let Ok(mut background) = backgrounds.get_mut(ui.art) {
        background.set_if_neq(BackgroundColor(if viewer.background == Backdrop::Light {
            Color::srgb(0.88, 0.88, 0.89)
        } else {
            WELL
        }));
    }
    let Ok(frame) = computed.get(ui.frame) else {
        return;
    };
    let available = frame.size() * frame.inverse_scale_factor();
    let art_size = dimensions.as_vec2();
    let size = art_size * ((available - Vec2::splat(8.0)).max(Vec2::ZERO) / art_size).min_element();
    if let Ok(mut art) = nodes.get_mut(ui.art) {
        place(&mut art, Rect::from_center_size(available * 0.5, size));
    }

    let viewport = canvas_rect(&canvas, &window);
    let image = viewer.image_rect(dimensions, viewport);
    let min = ((viewport.min - image.min) / image.size()).clamp(Vec2::ZERO, Vec2::ONE);
    let max = ((viewport.max - image.min) / image.size()).clamp(Vec2::ZERO, Vec2::ONE);
    if let Ok(mut indicator) = nodes.get_mut(ui.visible_area) {
        let partial = (max - min).cmpgt(Vec2::ZERO).all()
            && (min != Vec2::ZERO || max != Vec2::ONE)
            && size.cmpgt(Vec2::ZERO).all();
        let display = if partial {
            Display::Flex
        } else {
            Display::None
        };
        if indicator.display != display {
            indicator.display = display;
        }
        if partial {
            place(&mut indicator, Rect::from_corners(min * size, max * size));
        }
    }
}

fn place(node: &mut Mut<Node>, bounds: Rect) {
    let next = [
        px(bounds.min.x),
        px(bounds.min.y),
        px(bounds.width()),
        px(bounds.height()),
    ];
    if [node.left, node.top, node.width, node.height] != next {
        [node.left, node.top, node.width, node.height] = next;
    }
}
