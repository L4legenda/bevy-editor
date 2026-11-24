use super::*;
use crate::editor::{
    model_document::Mode,
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::{fixture_project, mouse, move_pointer},
};
use bevy::input::{
    ButtonState,
    keyboard::{Key, KeyboardInput},
};

#[test]
fn shader_insertion_keeps_the_open_graph_target_and_rejects_incompatible_nodes() {
    let library = material_graph::library();
    let texture = library.defs.iter().position(|node| node.id == "color").unwrap();
    let shading = library.defs.iter().position(|node| node.id == "shade_output").unwrap();
    let revision = node_library::revision();
    let mut session = Session::default();
    let mut layer = Layer::new("Материал".into(), LayerKind::Fill);
    let graph = material_graph::mask_starter();
    let mut fill = texture_fill::Fill::default();
    fill.graph = Some(Arc::new(material_graph::encode(&graph)));
    layer.texture_fill = Some(Arc::new(fill));
    session.stack = Stack { layers: vec![layer], selected: 0 };
    session.binding = Some((1, 0));
    session.edit(PaintAction::Panel(Panel::Shader));
    assert!(session.insert_shader_node(texture, revision).is_err());
    session.edit(PaintAction::Panel(Panel::Layers));
    session.edit(PaintAction::GraphCanvas);
    session.edit(PaintAction::Panel(Panel::Shader));
    assert!(session.insert_shader_node(shading, revision).is_err());
    session.binding = Some((2, 0));
    assert!(session.insert_shader_node(texture, revision).is_err());
    session.binding = Some((1, 0));
    assert_eq!(session.insert_shader_node(texture, revision).unwrap(),
        PaintAction::GraphEdit(material_graph::Edit::Add(texture, ui::position_next_to_graph(&graph))));
    assert!(session.graph_canvas_open());
}

#[test]
fn paint_workspace_groups_move_and_fade_as_blocks() {
    let mut session = Session::default();
    session.edit(PaintAction::Add(LayerKind::Folder));
    session.edit(PaintAction::Add(LayerKind::Folder));
    session.edit(PaintAction::Add(LayerKind::Fill));
    assert_eq!(
        session
            .stack
            .layers
            .iter()
            .map(|layer| layer.depth)
            .collect::<Vec<_>>(),
        [0, 2, 1, 0]
    );
    assert!(session.stack.valid_hierarchy());
    session.edit(PaintAction::Select(3));
    assert!(!session.available(PaintAction::MoveInto(2)));
    session.edit(PaintAction::Duplicate);
    assert_eq!(session.stack.block(session.stack.selected).len(), 3);
    session.edit(PaintAction::Undo);
    session.edit(PaintAction::Select(1));
    session.edit(PaintAction::MoveOut);
    assert_eq!(session.stack.layers[session.stack.selected].depth, 1);
    session.edit(PaintAction::Undo);
    session.edit(PaintAction::Visible(2));
    assert!(!session.stack.shown(1));
    session.edit(PaintAction::Undo);

    // Two overlapping white children under a 50% group yield 50% white.
    // Applying the group's opacity to each child would incorrectly yield 75%.
    let mut white = Layer::new("White".into(), LayerKind::Fill);
    white.depth = 2;
    white.channels[0] = Some(crate::editor::model_paint_data::solid(1, [255; 4]));
    let mut stack = session.stack;
    stack.layers[1] = white.clone();
    stack.layers.insert(1, white);
    stack.layers[3].opacity = 50.;
    stack.layers[4].opacity = 50.;
    let surface = Surface {
        size: 1,
        base: std::array::from_fn(|_| crate::editor::model_paint_data::solid(1, [0, 0, 0, 255])),
        stack,
        material: serde_json::Value::Null,
        baked: None,
    };
    assert_eq!(surface.composite(0), [64, 64, 64, 255]);
    let mut masked = surface.clone();
    masked.stack.layers[4].mask = Some(Arc::new(Mask::new(
        crate::editor::model_paint_data::solid(1, [128, 128, 128, 255]),
        Vec::new(),
    )));
    assert_eq!(masked.composite(0), [32, 32, 32, 255]);
}

#[test]
fn grouping_a_nested_folder_preserves_the_composite_and_is_one_history_step() {
    let mut session = Session::default();
    session.texture_size = 1;
    session.edit(PaintAction::Add(LayerKind::Folder));
    session.edit(PaintAction::Add(LayerKind::Folder));
    session.edit(PaintAction::Add(LayerKind::Fill));
    session.edit(PaintAction::Select(2));
    let before = session.stack.clone();
    let history = session.undo.len();
    let mut surface = Surface {
        size: 1,
        base: std::array::from_fn(|_| crate::editor::model_paint_data::solid(1, [0, 0, 0, 255])),
        stack: before.clone(),
        material: serde_json::Value::Null,
        baked: None,
    };
    let pixels = surface.composite(0);
    session.edit(PaintAction::GroupLayer);
    let after = session.stack.clone();
    assert!(after.valid_hierarchy());
    assert_eq!(after.selected, 3);
    assert_eq!(after.block(3), 1..4);
    assert_eq!(after.parents(2).collect::<Vec<_>>(), [3, 4]);
    assert_eq!(after.layers[2].name, before.layers[2].name);
    assert_eq!(after.layers[1].channels, before.layers[1].channels);
    assert_eq!(session.undo.len(), history + 1);
    surface.stack = after.clone();
    assert_eq!(surface.composite(0), pixels);
    session.edit(PaintAction::Undo);
    assert_eq!(session.stack, before);
    session.edit(PaintAction::Redo);
    assert_eq!(session.stack, after);

    let mut deepest = Layer::new("Краска".into(), LayerKind::Paint);
    deepest.depth = MAX_LAYER_DEPTH;
    session.stack.layers = std::iter::once(deepest).chain((0..MAX_LAYER_DEPTH).rev().map(|depth| {
        let mut folder = Layer::new(format!("Группа {depth}"), LayerKind::Folder);
        folder.depth = depth;
        folder
    })).collect();
    session.stack.selected = MAX_LAYER_DEPTH;
    assert!(session.stack.valid_hierarchy());
    assert!(!session.available(PaintAction::GroupLayer));
}

#[test]
fn moving_layers_by_destination_rejects_descendants_and_no_ops() {
    let mut session = Session::default();
    session.edit(PaintAction::Add(LayerKind::Folder));
    session.edit(PaintAction::Add(LayerKind::Fill));
    let before = session.stack.clone();
    let history = session.undo.len();
    for action in [
        PaintAction::MoveLayer(2, ShelfPlacement::Above(1)),
        PaintAction::MoveLayer(2, ShelfPlacement::Inside(2)),
        PaintAction::MoveLayer(0, ShelfPlacement::Below(2)),
        PaintAction::MoveLayer(99, ShelfPlacement::Above(0)),
        PaintAction::MoveLayer(1, ShelfPlacement::Inside(0)),
    ] {
        assert!(!session.available(action));
        session.edit(action);
    }
    assert_eq!(session.stack, before);
    assert_eq!(session.undo.len(), history);
    session.edit(PaintAction::MoveLayer(1, ShelfPlacement::Above(2)));
    assert_eq!(session.stack.selected, 2);
    assert_eq!(session.stack.layers[2].kind, LayerKind::Fill);
    assert_eq!(session.stack.layers[2].depth, 0);
    assert!(session.stack.valid_hierarchy());
    session.edit(PaintAction::Undo);
    assert_eq!(session.stack, before);
}

#[test]
fn paint_layer_rename_and_folder_collapse_do_not_change_compositing() {
    let mut session = Session::default();
    session.edit(PaintAction::Add(LayerKind::Folder));
    session.edit(PaintAction::Add(LayerKind::Folder));
    session.edit(PaintAction::Add(LayerKind::Fill));
    let initial = session.stack.clone();
    let undo = session.undo.len();
    assert!(session.rename_layer("  ").is_err());
    assert!(session.rename_layer("слой\nслой").is_err());
    assert!(session.rename_layer(&"я".repeat(121)).is_err());
    assert_eq!(session.undo.len(), undo);
    session.rename_layer("  Синий металл  ").unwrap();
    assert_eq!(session.stack.layers[1].name, "Синий металл");
    assert!(crate::editor::model_paint_data::same_composite(&initial, &session.stack));
    session.edit(PaintAction::Undo);
    assert_eq!(session.stack.layers[1].name, initial.layers[1].name);
    session.edit(PaintAction::Redo);
    assert_eq!(session.stack.layers[1].name, "Синий металл");
    session.edit(PaintAction::CollapseFolder(2));
    assert_eq!(session.stack.selected, 2);
    assert!(!session.stack.listed(1));
    session.edit(PaintAction::CollapseFolder(3));
    assert_eq!(session.stack.selected, 3);
    assert!(!session.stack.listed(2));
    assert!(session.stack.shown(1));
    session.edit(PaintAction::CollapseFolder(3));
    assert!(session.stack.listed(2));
    assert!(!session.stack.listed(1));
    assert!(crate::editor::model_paint_data::same_composite(&initial, &session.stack));
    session.edit(PaintAction::Select(2));
    session.edit(PaintAction::Add(LayerKind::Paint));
    assert!(session.stack.listed(session.stack.selected));
    assert!(!session.stack.layers[3].collapsed);
}

#[test]
fn paint_workspace_layer_stack_keeps_order_visibility_and_undo() {
    let mut session = Session::default();
    session.edit(PaintAction::Add(LayerKind::Fill));
    session.edit(PaintAction::Duplicate);
    assert_eq!(session.stack.layers.len(), 3);
    assert_eq!(session.stack.selected, 2);
    session.edit(PaintAction::Visible(2));
    assert!(!session.stack.layers[2].visible);
    session.edit(PaintAction::Undo);
    assert!(session.stack.layers[2].visible);
    session.edit(PaintAction::Undo);
    assert_eq!(session.stack.layers.len(), 2);
    session.edit(PaintAction::Redo);
    assert_eq!(session.stack.layers.len(), 3);
    session.edit(PaintAction::Move(-1));
    assert_eq!(session.stack.selected, 1);
    assert!(session.stack.layers[1].name.ends_with("копия"));
    session.edit(PaintAction::Remove);
    assert_eq!(session.stack.layers.len(), 2);
}

#[test]
fn paint_workspace_mask_filters_keep_raw_pixels_and_undo_order() {
    let mut session = Session {
        texture_size: 1,
        ..default()
    };
    let raw = Arc::new(vec![64, 64, 64, 255]);
    session.stack.layers[0].mask = Some(Arc::new(Mask::new(raw.clone(), Vec::new())));
    session.edit(PaintAction::AddFilter(FilterKind::Invert));
    session.edit(PaintAction::AddFilter(FilterKind::Levels));
    session.set_value(Parameter::FilterValue(1, FilterKind::Levels, 0), 55.);
    let effective = |session: &Session| {
        let mask = session.stack.layers[0].mask.as_ref().unwrap();
        assert!(Arc::ptr_eq(&mask.pixels, &raw));
        mask.effective()[0]
    };
    let original_order = effective(&session);
    session.set_value(Parameter::FilterValue(1, FilterKind::Levels, 3), 75.);
    session.set_value(Parameter::FilterValue(1, FilterKind::Levels, 4), 25.);
    assert!((effective(&session) as f32 - (191.25 - 0.5 * original_order as f32)).abs() <= 1.);
    session.edit(PaintAction::Undo);
    session.edit(PaintAction::Undo);
    assert_eq!(effective(&session), original_order);
    session.edit(PaintAction::MoveFilter(1, -1));
    assert_eq!(effective(&session), 255);
    assert_ne!(original_order, 255);
    session.edit(PaintAction::Undo);
    assert_eq!(effective(&session), original_order);
    session.edit(PaintAction::ToggleFilter(1));
    assert_eq!(effective(&session), 191);
    session.edit(PaintAction::Undo);
    assert_eq!(effective(&session), original_order);
    session.edit(PaintAction::AddMaskLayer);
    assert_eq!(effective(&session), 0);
    session.edit(PaintAction::MaskLayer(0, MaskLayerEdit::Blend));
    assert_eq!(effective(&session), original_order);
    session.edit(PaintAction::MaskLayer(0, MaskLayerEdit::Value(-1)));
    assert!(effective(&session) > original_order);
    session.edit(PaintAction::Undo);
    assert_eq!(effective(&session), original_order);
}

#[test]
fn paint_workspace_spatial_mask_filters_sample_neighbors_and_preserve_raw_mask() {
    let size = 128;
    let center = size / 2;
    let edge = center * size + center;
    let raw: Arc<Vec<u8>> = Arc::new(
        (0..size * size)
            .flat_map(|i| {
                let value = if i % size >= center { 170 } else { 85 };
                [value, value, value, 255]
            })
            .collect(),
    );
    let filtered = |kind| Mask::new(raw.clone(), vec![Filter::new(kind)]);
    let value = |filter: &Mask, i: usize| filter.effective()[i * 4];
    let blur = filtered(FilterKind::Blur);
    assert!(value(&blur, edge - 1) > 85 && value(&blur, edge) < 170);
    let mut wrap = filtered(FilterKind::Blur);
    wrap.edit_filters()[0].set_value(1, 1.);
    assert_eq!(value(&blur, center * size), 85);
    assert!(value(&wrap, center * size) > 85);
    assert!(value(&wrap, center * size + size - 1) < 170);
    let sharp = filtered(FilterKind::Sharpen);
    assert!(value(&sharp, edge - 1) < 85 && value(&sharp, edge) > 170);

    let mut warp = filtered(FilterKind::Warp);
    warp.edit_filters()[0].values[0] = 100.;
    assert_ne!(warp.effective(), raw.as_slice());
    assert!(Arc::ptr_eq(&warp.pixels, &raw));
    let unseeded = warp.effective().to_vec();
    warp.edit_filters()[0].set_value(2, 7.);
    assert_ne!(warp.effective(), unseeded);
    let mut noise = filtered(FilterKind::Noise);
    let first_seed = noise.effective().to_vec();
    assert_ne!(noise.effective(), raw.as_slice());
    noise.edit_filters()[0].values[2] = 9.;
    assert_ne!(noise.effective(), first_seed);
}

#[test]
fn paint_workspace_mask_outline_keeps_editable_inner_and_outer_bands() {
    let size = 32;
    let raw: Arc<Vec<u8>> = Arc::new(
        (0..size * size)
            .flat_map(|i| {
                let v = if (8..24).contains(&(i % size)) && (8..24).contains(&(i / size)) {
                    200
                } else {
                    30
                };
                [v, v, v, 255]
            })
            .collect(),
    );
    let mut filter = Filter::new(FilterKind::Outline);
    filter.set_value(1, 3.);
    for (side, outer, inner) in [(1., 255, 0), (2., 0, 255), (3., 255, 255)] {
        filter.set_value(2, side);
        let mask = Mask::new(raw.clone(), vec![filter]);
        let at = |x: usize, y: usize| mask.effective()[(y * size + x) * 4];
        assert_eq!([at(7, 12), at(8, 12), at(12, 7), at(12, 8)], [outer, inner, outer, inner]);
        assert_eq!([at(6, 12), at(9, 12), at(12, 6), at(12, 9)], [0; 4]);
        assert!(Arc::ptr_eq(&mask.pixels, &raw));
    }
    filter.set_value(1, 19.);
    let wide = Mask::new(raw.clone(), vec![filter]);
    for y in 0..size {
        for x in 0..size {
            let range = |p: usize| p.saturating_sub(6)..=(p + 6).min(size - 1);
            let mut inside = false;
            let mut outside = false;
            for row in range(y) {
                for col in range(x) {
                    let covered = (8..24).contains(&row) && (8..24).contains(&col);
                    inside |= covered;
                    outside |= !covered;
                }
            }
            assert_eq!(wide.effective()[(y * size + x) * 4], if inside && outside { 255 } else { 0 });
        }
    }
    filter.set_value(1, 3.);
    filter.set_value(3, 3.);
    filter.set_value(4, -50.);
    filter.set_value(5, 1.);
    let mask = Mask::new(raw, vec![filter]);
    assert_eq!(mask.effective()[0], 255);
    assert!((1..255).contains(&mask.effective()[(12 * size + 8) * 4]));
}

#[test]
fn paint_workspace_generators_use_surface_height_and_keep_paint_overrides() {
    use crate::editor::{
        model_document::MeshData,
        model_mesh::{Corner, Face, Geometry},
        model_paint_data::mask::GeneratorSurface,
    };
    let positions = vec![Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y];
    let mesh = MeshData::from(Geometry {
        positions,
        faces: vec![Face {
            corners: [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]
                .into_iter()
                .enumerate()
                .map(|(i, uv)| {
                    let mut corner = Corner::new(i);
                    corner.uv = uv;
                    corner
                })
                .collect(),
            material: 0,
            smooth: false,
        }],
        ..default()
    });
    let context = Arc::new(GeneratorSurface::new(mesh, 0, 16, Vec2::ZERO, Transform::IDENTITY, None));
    let mut height = Generator::new(GeneratorKind::Height);
    height.values[3] = 0.;
    let map = context.map(height);
    assert_eq!((map[8], map[15 * 16 + 8]), (0, 255));
    assert!(Arc::ptr_eq(&map, &context.map(height)));
    let cloud = context.map(Generator::new(GeneratorKind::Clouds));
    assert!(cloud.windows(2).any(|p| p[0] != p[1]));

    let mut raw = [255, 255, 255, 255].repeat(16 * 16);
    let index = 8 * 4;
    let mut mask = Mask::new(Arc::new(raw.clone()), Vec::new());
    mask.use_affine();
    *mask.edit_generator() = Some(height);
    mask.bind(context);
    assert_eq!(mask.effective()[index], 0);
    raw[index..index + 4].copy_from_slice(&[0, 204, 0, 255]);
    let mut painted = mask.with_pixels(Arc::new(raw));
    assert_eq!(painted.effective()[index], 204);
    painted.edit_layers().push(MaskLayer {
        generator: Some(height),
        ..default()
    });
    assert_eq!(painted.effective()[index], 0);
    assert_eq!(painted.pixels[index + 1], 204);
}

#[test]
fn paint_workspace_imports_alpha_and_keeps_a_rectangular_tips_aspect() {
    let path = std::env::temp_dir().join(format!(
        "bevy-brush-tip-{}.png",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    image::RgbaImage::from_fn(32, 16, |x, _| {
        image::Rgba(if x < 16 {
            [255, 255, 255, 0]
        } else {
            [0, 0, 0, 255]
        })
    })
    .save(&path)
    .unwrap();
    let tip = tips::load(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(tip.dimensions(), (256, 256));
    assert_eq!(tip.get_pixel(64, 128)[0], 0);
    assert_eq!(tip.get_pixel(192, 128)[0], 255);
    assert_eq!(tip.get_pixel(192, 32)[0], 0);
}

fn warm(app: &mut App, n: usize) {
    for _ in 0..n {
        frame(app);
    }
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
fn click(app: &mut App, window: Entity, action: impl Fn(Action) -> bool) {
    let (position, matched) = {
        let world = app.world_mut();
        let (control, node, transform) = world
            .query::<(&Control, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find(|(control, node, _)| node.size().min_element() > 0. && action(control.action))
            .expect("Visible paint control");
        (
            transform.affine().translation * node.inverse_scale_factor(),
            control.action,
        )
    };
    let viewport = app.world().get::<Window>(window).unwrap();
    assert!(
        Rect::from_corners(Vec2::ZERO, Vec2::new(viewport.width(), viewport.height()))
            .contains(position),
        "control {matched:?} is outside the window at {position:?}"
    );
    move_pointer(app, window, position);
    mouse(app, window, ButtonState::Pressed);
    mouse(app, window, ButtonState::Released);
    warm(app, 4);
}
fn paint_click(app: &mut App, window: Entity, action: PaintAction) {
    paint_ready(app);
    click(
        app,
        window,
        |a| matches!(a, Action::ModelView(ModelViewAction::Paint(value)) if value == action),
    );
    paint_ready(app);
}
fn set_shelf_query(app: &mut App, value: &str) {
    let world = app.world_mut();
    let mut inputs = world.query::<(&AccessibleLabel, &mut bevy::text::EditableText)>();
    let (_, mut input) = inputs.iter_mut(world)
        .find(|(label, _)| label.0 == "Поиск ресурсов библиотеки")
        .unwrap();
    input.editor_mut().set_text(value);
    warm(app, 4);
}

fn paint_button_rect(app: &mut App, action: PaintAction) -> Rect {
    let world = app.world_mut();
    let mut query = world.query::<(&Control, &ComputedNode, &UiGlobalTransform)>();
    let (_, node, transform) = query.iter(world)
        .find(|(control, node, _)| node.size().min_element() > 0.
            && matches!(control.action, Action::ModelView(ModelViewAction::Paint(value)) if value == action))
        .unwrap_or_else(|| panic!("Visible paint control {action:?}"));
    Rect::from_center_size(
        transform.affine().translation * node.inverse_scale_factor(),
        node.size() * node.inverse_scale_factor(),
    )
}

fn scroll_to_paint_control(app: &mut App, action: PaintAction) {
    use bevy::ui::{OverflowAxis, ScrollPosition};
    let target = paint_button_rect(app, action).center().y;
    let world = app.world_mut();
    let mut buttons = world.query::<(Entity, &Control, &ComputedNode)>();
    let mut entity = buttons.iter(world).find_map(|(entity, control, node)| {
        (node.size().min_element() > 0.
            && matches!(control.action, Action::ModelView(ModelViewAction::Paint(value)) if value == action))
            .then_some(entity)
    }).unwrap();
    while !world.get::<Node>(entity).is_some_and(|node| node.overflow.y == OverflowAxis::Scroll) {
        entity = world.get::<ChildOf>(entity).unwrap().parent();
    }
    let node = world.get::<ComputedNode>(entity).unwrap();
    let current = node.scroll_position.y * node.inverse_scale_factor();
    let y = world.get::<UiGlobalTransform>(entity).unwrap().affine().translation.y
        * node.inverse_scale_factor();
    let mut scroll = world.get_mut::<ScrollPosition>(entity).unwrap();
    scroll.0.y = (current + target - y).max(0.);
    warm(app, 4);
}

fn drag_shelf_item(app: &mut App, window: Entity, action: PaintAction, to: Vec2) {
    let from = paint_button_rect(app, action).center();
    move_pointer(app, window, from);
    mouse(app, window, ButtonState::Pressed);
    move_pointer(app, window, from + Vec2::X * 7.);
    move_pointer(app, window, to);
    mouse(app, window, ButtonState::Released);
    warm(app, 4);
}

fn paint_message(app: &mut App, action: PaintAction) {
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Paint(action)));
    warm(app, 3);
    paint_ready(app);
}
fn fill_modifier(app: &mut App, window: Entity, code: KeyCode, key: Key, state: ButtonState) {
    app.world_mut().write_message(KeyboardInput {
        key_code: code,
        logical_key: key,
        state,
        text: None,
        repeat: false,
        window,
    });
    frame(app);
}
fn shift_fill_drag(app: &mut App, window: Entity, points: &[Vec2]) {
    fill_modifier(app, window, KeyCode::ShiftLeft, Key::Shift, ButtonState::Pressed);
    move_pointer(app, window, points[0]);
    mouse(app, window, ButtonState::Pressed);
    for &point in &points[1..] {
        move_pointer(app, window, point);
    }
    mouse(app, window, ButtonState::Released);
    fill_modifier(app, window, KeyCode::ShiftLeft, Key::Shift, ButtonState::Released);
    paint_ready(app);
}
#[track_caller]
fn paint_ready(app: &mut App) {
    for _ in 0..600 {
        frame(app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .values()
            .all(|d| !d.paint_busy)
        {
            return;
        }
    }
    panic!("Paint worker did not finish");
}

#[test]
#[ignore = "requires GPU; uses the paint layer name field, folder disclosure and graph buttons"]
fn gpu_paint_folder_graph_opens_from_the_renamed_layer_group() {
    let (mut app, window, _) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    {
        use crate::editor::model_primitives::Primitive;
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        doc.objects[0].geometry = Primitive::Cube.geometry(&Primitive::Cube.defaults()).unwrap().into();
        doc.revision += 1;
    }
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(
        editing::EditAction::Mode(Mode::Paint),
    )));
    warm(&mut app, 10);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::Add(LayerKind::Folder));
    paint_message(&mut app, PaintAction::Add(LayerKind::Fill));
    let field = {
        let world = app.world_mut();
        let mut fields = world.query_filtered::<Entity, With<LayerNameField>>();
        fields.single(world).unwrap()
    };
    app.world_mut().resource_mut::<InputFocus>().set(field, bevy::input_focus::FocusCause::Auto);
    warm(&mut app, 2);
    app.world_mut().get_mut::<bevy::text::EditableText>(field).unwrap()
        .editor_mut().set_text("Царапины на корпусе");
    fill_modifier(&mut app, window, KeyCode::Enter, Key::Enter, ButtonState::Pressed);
    fill_modifier(&mut app, window, KeyCode::Enter, Key::Enter, ButtonState::Released);
    paint_ready(&mut app);
    let (child, folder) = {
        let stack = &app.world().resource::<PaintState>().sessions[&0].stack;
        assert_eq!(stack.layers[stack.selected].name, "Царапины на корпусе");
        (stack.selected, stack.parents(stack.selected).next().unwrap())
    };
    let collapse = PaintAction::CollapseFolder(folder);
    scroll_to_paint_control(&mut app, collapse);
    paint_click(&mut app, window, collapse);
    {
        let world = app.world_mut();
        let stack = &world.resource::<PaintState>().sessions[&0].stack;
        assert!(stack.layers[folder].collapsed);
        assert!(stack.shown(child));
        assert_eq!(stack.selected, folder);
        let mut controls = world.query::<&Control>();
        assert!(!controls.iter(world).any(|control| matches!(
            control.action, Action::ModelView(ModelViewAction::Paint(PaintAction::Select(i))) if i == child
        )));
    }
    paint_click(&mut app, window, collapse);
    assert!(app.world().resource::<PaintState>().sessions[&0].stack.listed(child));
    scroll_to_paint_control(&mut app, PaintAction::GraphCanvas);
    paint_click(&mut app, window, PaintAction::GraphCanvas);
    warm(&mut app, 4);
    let world = app.world_mut();
    let session = &world.resource::<PaintState>().sessions[&0];
    assert!(session.graph_canvas_open());
    let view = session.active_folder_view().unwrap();
    assert!(view.labels.values().any(|label| label == "Цвет · Царапины на корпусе"));
    let mut labels = world.query::<&Text>();
    assert!(labels.iter(world).any(|text| text.0 == "Цвет · Царапины на корпусе"));

    use material_graph::Edit;
    let graph_edit = |app: &mut App, edit| paint_message(app, PaintAction::GraphEdit(edit));
    let add = |app: &mut App, name, pos| {
        let index = material_graph::library().defs.iter().position(|def| def.id == name).unwrap();
        graph_edit(app, Edit::Add(index, pos));
        app.world().resource::<PaintState>().sessions[&0].graph_node.unwrap()
    };
    let source = add(&mut app, "color", IVec2::new(-600, 0));
    let invert = add(&mut app, "invert", IVec2::new(-300, 0));
    graph_edit(&mut app, Edit::Connect(source, invert, 0));
    let layer_node = {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        session.active_folder_view().unwrap().graph
            .tagged(session.stack.layers[child].graph_id.unwrap(), material_painter_graph::Role::Channel(0))
            .unwrap()
    };
    let variation = material_graph::library().find("layer").unwrap().params.iter()
        .position(|param| param.name == "variation").unwrap();
    paint_message(&mut app, PaintAction::GraphAdjust(layer_node, variation, 0, 1));
    let mut changed_pixels = false;
    for _ in 0..300 {
        frame(&mut app);
        let session = &app.world().resource::<PaintState>().sessions[&0];
        let (object, slot) = session.binding.unwrap();
        let doc = &app.world().resource::<editing::State>().documents[&0];
        let layer = doc.objects.iter().find(|item| item.id == object)
            .and_then(|item| item.materials[slot].paint.as_ref())
            .map(|paint| &paint.stack.layers[child]);
        if let Some(layer) = layer
            && let Some(sampled) = layer.texture_fill.as_ref().and_then(|fill| fill.sampled[0].as_ref())
            && sampled != layer.channels[0].as_ref().unwrap()
        {
            changed_pixels = true;
            break;
        }
    }
    assert!(changed_pixels, "Правка узла должна изменить пиксели заливки");
    paint_ready(&mut app);
    graph_edit(&mut app, Edit::Connect(invert, layer_node, 1));
    for _ in 0..240 {
        frame(&mut app);
        if !app.world().resource::<PaintState>().sessions[&0].smart_loading { break; }
    }
    paint_ready(&mut app);
    {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        let mask = session.stack.layers[child].mask.as_ref().expect("узлы применились к маске слоя");
        let mask_graph = mask.graph.as_ref().unwrap();
        let graph = material_graph::read(&mask_graph.payload).unwrap();
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.node(invert).unwrap().links["input"], source);
        assert_eq!(session.active_folder_view().unwrap().graph.node(layer_node).unwrap().links["mask"], invert);
        assert!((125..=130).contains(&mask.effective()[0]));
    }
    graph_edit(&mut app, Edit::Disconnect(layer_node, 1));
    assert!(app.world().resource::<PaintState>().sessions[&0].stack.layers[child]
        .mask.as_ref().unwrap().graph.is_none());
    paint_message(&mut app, PaintAction::Undo);
    assert!(app.world().resource::<PaintState>().sessions[&0].stack.layers[child]
        .mask.as_ref().unwrap().graph.is_some());

    use crate::editor::model_document::glb::Glb;
    use material_graph::ParameterTarget;
    let directory = std::env::temp_dir().join(format!("folder-bitmap-{}", bevy::asset::uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("Маска.png");
    let bitmap = add(&mut app, "bitmap", IVec2::new(-900, 0));
    let bitmap_target = ParameterTarget::Node(bitmap, 0);
    let pick = PaintAction::GraphBitmapPick(bitmap_target);
    let settle = |app: &mut App| {
        for _ in 0..360 {
            frame(app);
            if !app.world().resource::<PaintState>().sessions[&0].smart_loading
                && !app.world().resource::<editing::State>().documents[&0].paint_pending
            {
                return;
            }
        }
        panic!("Граф маски не завершил пересчёт");
    };

    image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255])).save(&path).unwrap();
    app.world_mut().resource_mut::<PaintState>().graph_bitmap_test_path = Some(Some(path.clone()));
    paint_message(&mut app, pick);
    settle(&mut app);
    graph_edit(&mut app, Edit::Connect(bitmap, invert, 0));
    settle(&mut app);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].stack.layers[child]
        .mask.as_ref().unwrap().effective()[0], 255);

    image::RgbaImage::from_pixel(4, 4, image::Rgba([255; 4])).save(&path).unwrap();
    app.world_mut().resource_mut::<PaintState>().graph_bitmap_test_path = Some(Some(path.clone()));
    paint_message(&mut app, pick);
    settle(&mut app);
    std::fs::remove_file(path).unwrap();
    let image_parameter = material_graph::library().find("layer").unwrap().params.iter()
        .position(|param| param.name == "image").unwrap();
    let layer_image = ParameterTarget::Node(layer_node, image_parameter);
    let layer_pick = PaintAction::GraphBitmapPick(layer_image);
    let layer_path = directory.join("Цвет.png");
    let color = [40, 70, 200, 255];
    image::RgbaImage::from_pixel(4, 4, image::Rgba(color)).save(&layer_path).unwrap();
    assert!(paint_button_rect(&mut app, layer_pick).width() > 0.);
    app.world_mut().resource_mut::<PaintState>().graph_bitmap_test_path = Some(Some(layer_path.clone()));
    paint_message(&mut app, layer_pick);
    settle(&mut app);
    std::fs::remove_file(layer_path).unwrap();
    let bitmap_layers = &app.world().resource::<PaintState>().sessions[&0].stack.layers;
    assert_eq!(bitmap_layers[folder].folder_images.len(), 1);
    assert_eq!(bitmap_layers[child].mask.as_ref().unwrap().effective()[0], 0);
    let fill = bitmap_layers[child].texture_fill.as_ref().unwrap();
    assert_eq!(fill.settings.projection, texture_fill::Projection::Triplanar);
    assert_eq!(fill.images[0].as_ref().unwrap().pixels().get_pixel(0, 0).0, color);
    assert!(fill.images[1..].iter().all(Option::is_none));

    let saved = directory.join("Сохранённая модель.glb");
    let doc = &app.world().resource::<editing::State>().documents[&0];
    std::fs::write(&saved, doc.source.encode(&doc.objects, &doc.original).unwrap()).unwrap();
    let glb = Glb::load(&saved).unwrap();
    let surface = glb.json["meshes"].as_array().unwrap().iter()
        .flat_map(|mesh| mesh["primitives"].as_array().into_iter().flatten())
        .find_map(|primitive| glb.paint_surface(primitive).unwrap()).unwrap();
    assert_eq!(surface.stack.layers[folder].folder_images, bitmap_layers[folder].folder_images);
    assert_eq!(surface.stack.layers[child].mask.as_ref().unwrap().effective()[0], 0);
    assert_eq!(surface.stack.layers[child].texture_fill.as_ref().unwrap().images[0].as_ref()
        .unwrap().pixels().get_pixel(0, 0).0, color);
    let mut reopened = surface.stack.clone();
    reopened.selected = folder;
    let graph = folder_graph::start(&mut reopened, surface.size, 0);
    assert_eq!(graph.graph.node(invert).unwrap().links["input"], bitmap);
    assert_eq!(graph.images.len(), 2);
    std::fs::remove_file(saved).unwrap();
    std::fs::remove_dir(directory).unwrap();

    graph_edit(&mut app, Edit::ClearBitmap(layer_image));
    settle(&mut app);
    assert!(app.world().resource::<PaintState>().sessions[&0].stack.layers[child]
        .texture_fill.as_ref().unwrap().images[0].is_none());
    paint_message(&mut app, PaintAction::Undo);
    assert!(app.world().resource::<PaintState>().sessions[&0].stack.layers[child]
        .texture_fill.as_ref().unwrap().images[0].is_some());

    graph_edit(&mut app, Edit::ClearBitmap(bitmap_target));
    settle(&mut app);
    assert!(app.world().resource::<PaintState>().sessions[&0].stack.layers[folder].folder_images.is_empty());
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].stack.layers[folder].folder_images.len(), 1);
    paint_message(&mut app, PaintAction::Select(child));
    paint_message(&mut app, PaintAction::RemoveMask);
    paint_message(&mut app, PaintAction::ApplyGenerator(GeneratorKind::Clouds, Some(child)));
    paint_message(&mut app, PaintAction::Select(folder));
    let (generator_node, before) = {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        let layer = &session.stack.layers[child];
        let node = session.active_folder_view().unwrap().graph.tagged(
            layer.graph_id.unwrap(), material_painter_graph::Role::Mask,
        ).unwrap();
        let (object, slot) = session.binding.unwrap();
        let value = move |app: &App| {
            let doc = &app.world().resource::<editing::State>().documents[&0];
            doc.objects.iter().find(|item| item.id == object).unwrap()
                .materials[slot].paint.as_ref().unwrap().stack.layers[child]
                .mask.as_ref().unwrap().effective().to_vec()
        };
        (node, (value(&app), value))
    };
    let weight = material_graph::library().find("smart_mask").unwrap().params.iter()
        .position(|param| param.name == "weight_1").unwrap();
    paint_message(&mut app, PaintAction::GraphAdjust(generator_node, weight, 0, 1));
    settle(&mut app);
    let session = &app.world().resource::<PaintState>().sessions[&0];
    assert_eq!(session.stack.layers[child].mask.as_ref().unwrap().generator.unwrap()
        .weight(GeneratorInput::Noise), 105);
    let (before_pixels, pixels) = before;
    assert_ne!(pixels(&app), before_pixels);
}

#[test]
#[ignore = "requires GPU; traces the real paint viewport and model stroke"]
fn gpu_paint_trace_view_uses_the_live_brush_and_camera() {
    use crate::editor::{model_primitives::Primitive, tests::lighting_ui_render::fixture};
    use super::pathtrace::viewport::{CanvasImage, dispatches, samples};

    let (mut app, window, target) = fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    let diffuse = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
        base_color: Color::srgb(0.67, 0.67, 0.67),
        perceptual_roughness: 0.7,
        ..default()
    });
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        doc.objects[0].geometry = Primitive::Cube.geometry(&Primitive::Cube.defaults()).unwrap().into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.objects[0].materials[0].material = diffuse;
        doc.revision += 1;
        doc.mode = Mode::Paint;
    }
    warm(&mut app, 12);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::View(View::Three));
    paint_message(&mut app, PaintAction::Panel(Panel::Preview));
    warm(&mut app, 5);
    let canvas_image = |app: &mut App| {
        let world = app.world_mut();
        world.query_filtered::<&ImageNode, With<CanvasImage>>()
            .single(world).unwrap().image.clone()
    };
    let raster = canvas_image(&mut app);
    scroll_to_paint_control(&mut app, PaintAction::PathTrace);
    paint_click(&mut app, window, PaintAction::PathTrace);
    for _ in 0..180 {
        frame(&mut app);
        if samples(app.world()).is_some_and(|count| count >= 8) {
            break;
        }
    }
    assert!(samples(app.world()).is_some_and(|count| count >= 8));
    let traced = canvas_image(&mut app);
    assert_ne!(raster, traced);
    let center = labelled(&mut app, "3D-вид модели").center();
    let point = center.round().as_uvec2();
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/paint-perf/path-trace");
    std::fs::create_dir_all(&output).unwrap();
    let before = capture_when(&mut app, &target, &output.join("viewport-before.png"), |_| true);

    paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.picked = Some([245, 0, 210]);
        session.size = 80.0;
        session.hardness = 100.0;
        session.flow = 100.0;
    }
    move_pointer(&mut app, window, center);
    let batches = app.world().resource::<engine::gpu::Queue>().statistics().0;
    mouse(&mut app, window, ButtonState::Pressed);
    assert_eq!(canvas_image(&mut app), raster, "a held stroke uses the raster canvas");
    move_pointer(&mut app, window, center + Vec2::X * 24.0);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert!(app.world().resource::<engine::gpu::Queue>().statistics().0 > batches, "{}", app.world().resource::<Editor>().status);
    move_pointer(&mut app, window, Vec2::new(8.0, 512.0));
    let painted = |image: &image::RgbaImage| {
        let [r, g, b, _] = image.get_pixel(point.x, point.y).0;
        r > g.saturating_add(35) && b > g.saturating_add(25)
    };
    assert!(!painted(&before));
    capture_when(&mut app, &target, &output.join("viewport-painted.png"), painted);
    assert_eq!(canvas_image(&mut app), traced);
    warm(&mut app, 10);
    let accumulated = samples(app.world()).unwrap();
    let dispatched = dispatches(app.world());
    app.world_mut().resource_mut::<Viewer>().yaw += 0.3;
    warm(&mut app, 4);
    assert_eq!(canvas_image(&mut app), raster);
    assert_eq!(dispatches(app.world()), dispatched);
    for _ in 0..30 {
        frame(&mut app);
        if samples(app.world()).is_some() {
            break;
        }
    }
    assert!(samples(app.world()).is_some_and(|count| count < accumulated));
    app.world_mut().resource_mut::<Viewer>().yaw -= 0.3;
    use super::environment::{EnvironmentAction, Setting};
    use super::post::Field;
    for _ in 0..5 {
        paint_message(&mut app, PaintAction::TraceSamples(-1));
    }
    for _ in 0..60 {
        if samples(app.world()).is_some_and(|count| count >= 16) {
            break;
        }
        frame(&mut app);
    }
    let before_grade = samples(app.world()).unwrap();
    assert!(before_grade >= 16);
    let saturation = |value| Action::ModelView(ModelViewAction::Environment(
        EnvironmentAction::Set(Setting::Post(Field::Saturation), value, false),
    ));
    let gray = |image: &image::RgbaImage| {
        let [r, g, b, _] = image.get_pixel(point.x, point.y).0;
        r.abs_diff(g) < 3 && r.abs_diff(b) < 3
    };
    scroll_to_paint_control(&mut app, PaintAction::PostPanel);
    paint_click(&mut app, window, PaintAction::PostPanel);
    let saturation_slider = {
        let world = app.world_mut();
        world.query::<(Entity, &AccessibleLabel)>().iter(world)
            .find(|(_, label)| label.0 == "Просмотр: Насыщенность").unwrap().0
    };
    app.world_mut().trigger(bevy::ui_widgets::ValueChange::<f32> {
        source: saturation_slider, value: 0.0, is_final: false,
    });
    move_pointer(&mut app, window, Vec2::new(8.0, 512.0));
    capture_when(&mut app, &target, &output.join("viewport-graded.png"), gray);
    assert_eq!(samples(app.world()), Some(before_grade), "post effects reuse the accumulated HDR frame");
    paint_message(&mut app, PaintAction::PreviewShader(None));
    assert_eq!(canvas_image(&mut app), raster);
    capture_when(&mut app, &target, &output.join("viewport-raster-graded.png"), gray);
    app.world_mut().write_message(saturation(1.0));
    capture_when(&mut app, &target, &output.join("viewport-raster-reset.png"), |image| !gray(image));
}

#[test]
#[ignore = "requires GPU; renders a bitmap graph and checks persisted source images"]
fn gpu_mpgraph_bitmap_renders_and_survives_presets_and_glb() {
    use crate::editor::model_document::glb::Glb;
    use bevy::render::renderer::{RenderDevice, RenderQueue};
    use material_graph::file::{self, tests as graph_fixture};
    use std::{fs, path::Path};

    let directory = std::env::temp_dir().join(format!(
        "bevy-mpgraph-gpu-{}",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    let textures = directory.join("textures");
    let (source, _) = graph_fixture::fixture(&directory, &textures);
    let (mut graph, images) = file::load(&source).unwrap();
    graph.res = 2048;
    fs::remove_dir_all(textures).unwrap();

    let project = crate::EditorProject::load(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let (mut app, window, target) = fixture_project(project);
    let device = app.world().resource::<RenderDevice>().clone();
    let queue = app.world().resource::<RenderQueue>().clone();
    let layer = material_graph::prepare_graph(graph, &images, 64, Default::default(), &device, &queue).unwrap();
    let fill = layer.texture_fill.as_ref().unwrap();
    assert_eq!(fill.graph.as_ref().unwrap()["res"], 2048);
    assert_eq!(fill.graph.as_ref().unwrap()["match_set"], true);
    let color = fill.images[0].as_ref().unwrap().pixels();
    assert_eq!(color.dimensions(), (64, 64));
    for (actual, expected) in color.get_pixel(32, 32).0.into_iter().zip(graph_fixture::PIXEL) {
        assert!(actual.abs_diff(expected) <= 1, "Bitmap rendered as {:?}",
            color.get_pixel(32, 32));
    }

    let (preset, skipped) = presets::save(&directory, presets::Capture::Material {
        name: layer.name.clone(),
        layers: vec![layer.clone()],
    }).unwrap();
    assert_eq!(skipped, 0);
    let reopened = presets::load_material(&preset.path, 64).unwrap();
    let restored = reopened[0].texture_fill.as_ref().unwrap();
    assert_eq!(restored.graph, fill.graph);
    assert_eq!(restored.graph_images, fill.graph_images);

    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    let doc = &app.world().resource::<editing::State>().documents[&0];
    let mut objects = doc.objects.clone();
    objects[0].materials[0].paint = Some(Arc::new(Surface {
        size: 64,
        base: std::array::from_fn(|_| crate::editor::model_paint_data::solid(64, [255; 4])),
        stack: Stack { layers: vec![layer.clone()], selected: 0 },
        material: serde_json::Value::Null,
        baked: None,
    }));
    let glb_path = directory.join("graph.glb");
    fs::write(&glb_path, doc.source.encode(&objects, &doc.original).unwrap()).unwrap();
    let loaded = Glb::load(&glb_path).unwrap();
    let surface = loaded.json["meshes"].as_array().unwrap().iter()
        .flat_map(|mesh| mesh["primitives"].as_array().into_iter().flatten())
        .find_map(|primitive| loaded.paint_surface(primitive).unwrap()).unwrap();
    let restored = surface.stack.layers[0].texture_fill.as_ref().unwrap();
    assert_eq!(restored.graph, fill.graph);
    assert_eq!(restored.graph_images, fill.graph_images);
    let graph = material_graph::read(restored.graph.as_ref().unwrap()).unwrap();
    let bitmap = graph.nodes.first().unwrap().id;
    let output = graph.nodes.last().unwrap().id;
    let regenerated = material_graph::prepare_graph(graph, &restored.graph_images, 64, Default::default(), &device, &queue)
        .unwrap().texture_fill.unwrap();
    assert_eq!(regenerated.images[0].as_ref().unwrap().bytes, fill.images[0].as_ref().unwrap().bytes);

    if let Some(output) = std::env::var_os("BEVY_MPGRAPH_QA_DIR") {
        fs::create_dir_all(&output).unwrap();
        let export = file::export(Path::new(&output), restored.graph.as_ref().unwrap(), &restored.graph_images).unwrap();
        println!("Material Painter graph: {}", export.display());
    }

    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 12);
    paint_ready(&mut app);
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.insert_smart_material(vec![layer], session.insertion());
        session.graph_canvas = true;
    }
    warm(&mut app, 12);
    let preview_at = |app: &mut App, id| {
        let world = app.world_mut();
        let mut previews = world.query::<(&ui::NodePreview, &ComputedNode, &UiGlobalTransform)>();
        let (_, shape, transform) = previews.iter(world).find(|(node, _, _)| node.0 == id).unwrap();
        assert!(shape.size().min_element() > 0.);
        transform.affine().translation.round().as_uvec2()
    };
    let bitmap_center = preview_at(&mut app, bitmap);
    let output_center = preview_at(&mut app, output);
    let matches = |image: &image::RgbaImage, point: UVec2, expected: [u8; 4]| {
        image.get_pixel(point.x, point.y).0.into_iter().zip(expected)
            .all(|(actual, expected)| actual.abs_diff(expected) <= 2)
    };
    capture_when(&mut app, &target, &directory.join("preview-bitmap.png"), |image| {
        matches(image, bitmap_center, graph_fixture::PIXEL)
            && matches(image, output_center, graph_fixture::PIXEL)
    });

    use material_graph::{ParameterTarget, Edit};
    let current = |app: &App| {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        session.stack.layers[session.stack.selected].texture_fill.as_ref().unwrap().clone()
    };
    let settle = |app: &mut App| {
        let start = std::time::Instant::now();
        loop {
            frame(app);
            if !app.world().resource::<PaintState>().sessions[&0].smart_loading {
                break;
            }
            assert!(start.elapsed().as_secs() < 60, "Bitmap change did not finish");
        }
        paint_ready(app);
    };
    let original = current(&app);
    let from_node = PaintAction::GraphBitmapPick(ParameterTarget::Node(bitmap, 0));
    assert!(paint_button_rect(&mut app, from_node).width() > 0.);
    paint_ready(&mut app);
    assert!(app.world().resource::<PaintState>().sessions[&0].available(from_node));
    app.world_mut().resource_mut::<PaintState>().graph_bitmap_test_path = Some(None);
    move_pointer(&mut app, window, bitmap_center.as_vec2());
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    settle(&mut app);
    assert_eq!(current(&app).graph, original.graph);
    assert!(app.world().resource::<PaintState>().graph_bitmap_test_path.is_none());

    let replacement = [32, 162, 88, 255];
    let picked = directory.join("Новая ткань.png");
    image::RgbaImage::from_pixel(4, 4, image::Rgba(replacement)).save(&picked).unwrap();
    app.world_mut().resource_mut::<PaintState>().graph_bitmap_test_path = Some(Some(picked.clone()));
    let published_picker = PaintAction::GraphBitmapPick(ParameterTarget::Published(0));
    scroll_to_paint_control(&mut app, published_picker);
    paint_click(&mut app, window, published_picker);
    settle(&mut app);
    assert!(app.world().resource::<PaintState>().graph_bitmap_test_path.is_none());
    fs::remove_file(picked).unwrap();
    let updated = current(&app);
    assert_eq!(updated.graph_images.len(), 2, "the published default retains the first bitmap");
    assert!(updated.graph_images.values().any(|image| image.name == "Новая ткань.png"));
    assert_eq!(updated.graph.as_ref().unwrap()["res"], 2048);
    capture_when(&mut app, &target, &directory.join("preview-replaced.png"), |image| {
        matches(image, bitmap_center, replacement) && matches(image, output_center, replacement)
    });
    {
        let doc = &app.world().resource::<editing::State>().documents[&0];
        fs::write(&glb_path, doc.source.encode(&doc.objects, &doc.original).unwrap()).unwrap();
    }
    let saved = Glb::load(&glb_path).unwrap();
    let saved_surface = saved.json["meshes"].as_array().unwrap().iter()
        .flat_map(|mesh| mesh["primitives"].as_array().into_iter().flatten())
        .find_map(|primitive| saved.paint_surface(primitive).unwrap()).unwrap();
    let saved_bitmap = saved_surface.stack.layers.iter()
        .find_map(|layer| layer.texture_fill.as_ref()).unwrap();
    assert_eq!(saved_bitmap.graph_images, updated.graph_images);
    assert_eq!(saved_bitmap.images[0].as_ref().unwrap().bytes, updated.images[0].as_ref().unwrap().bytes);

    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::GraphEdit(Edit::ClearBitmap(ParameterTarget::Node(bitmap, 0))));
    settle(&mut app);
    let cleared = current(&app);
    let graph = material_graph::read(cleared.graph.as_ref().unwrap()).unwrap();
    assert_eq!(graph.param_value(0, &material_graph::library()).unwrap().1,
        material_painter_graph::Value::Image(None));
    assert_eq!(cleared.graph_images.len(), 1);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(current(&app).graph_images, updated.graph_images);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(current(&app).graph, original.graph);

    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::GraphEdit(material_graph::Edit::Disconnect(output, 0)));
    capture_when(&mut app, &target, &directory.join("preview-disconnected.png"), |image| {
        matches(image, bitmap_center, graph_fixture::PIXEL)
            && matches(image, output_center, [128, 128, 128, 255])
    });
    paint_message(&mut app, PaintAction::GraphCanvas);
    let mut previews = app.world_mut().query::<&ui::NodePreview>();
    assert_eq!(previews.iter(app.world()).count(), 0);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires GPU; applies a mesh-map graph, bakes a texture set and verifies automatic refresh"]
fn gpu_mesh_map_graph_updates_after_a_real_bake() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    use bevy::render::renderer::{RenderDevice, RenderQueue};
    use material_graph::{Edit, ParameterTarget};
    use material_painter_graph::Graph;
    use std::{path::Path, time::Instant};

    let project = crate::EditorProject::load(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let (mut app, window, render_target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    {
        let mut state = app.world_mut().resource_mut::<editing::State>();
        let doc = state.documents.get_mut(&0).unwrap();
        let mut geometry = Geometry::default();
        for part in 0..2 {
            let left = part as f32;
            geometry.positions.extend([
                Vec3::new(left * 2., 0., 0.), Vec3::new(left * 2. + 1., 0., 0.),
                Vec3::new(left * 2. + 1., 1., 0.), Vec3::new(left * 2., 1., 0.),
            ]);
            let corners = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y].into_iter().enumerate()
                .map(|(i, uv)| {
                    let mut corner = Corner::new(part * 4 + i);
                    corner.uv = Vec2::new((left + uv.x) * 0.5, uv.y);
                    corner
                }).collect();
            geometry.faces.push(Face { corners, material: 0, smooth: false });
        }
        doc.objects[0].geometry = geometry.into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 12);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::Resolution(512));
    let bake_size = app.world().resource::<PaintState>().sessions[&0].texture_size;
    assert_eq!(bake_size, 512);
    paint_message(&mut app, PaintAction::BakeSelectAllMaps(false));
    let mut graph = Graph::new("Mesh Map QA");
    graph.res = 64;
    let map_node = graph.add("mesh_map", [12., 24.]);
    let output = graph.add("output", [250., 24.]);
    assert!(graph.connect(map_node, output, "input"));
    let world = app.world();
    let layer = material_graph::prepare_graph(
        graph, &Default::default(), bake_size, Default::default(),
        world.resource::<RenderDevice>(), world.resource::<RenderQueue>(),
    ).unwrap();
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.insert_smart_material(vec![layer], session.insertion());
    }
    warm(&mut app, 12);
    paint_ready(&mut app);
    let target = ParameterTarget::Node(map_node, 0);
    let menu = PaintAction::GraphMapMenu(target);
    scroll_to_paint_control(&mut app, menu);
    paint_click(&mut app, window, menu);
    let choose = PaintAction::GraphEdit(Edit::SetMeshMap(target, 7));
    scroll_to_paint_control(&mut app, choose);
    paint_click(&mut app, window, choose);
    let wait = |app: &mut App, condition: &dyn Fn(&World) -> bool| {
        let start = Instant::now();
        loop {
            frame(app);
            if condition(app.world()) {
                break;
            }
            assert!(start.elapsed().as_secs() < 60, "{}", app.world().resource::<Editor>().status);
        }
        paint_ready(app);
    };
    wait(&mut app, &|world| world.resource::<PaintState>().sessions[&0].graph()
        .is_some_and(|graph| graph["nodes"][0]["params"]["map"] == "Part Random"));
    let initial = app.world().resource::<PaintState>().sessions[&0]
        .active_texture_fill().unwrap().images[0].as_ref().unwrap().clone();
    assert_eq!(initial.pixels().get_pixel(32, 32).0[0], 128);
    paint_message(&mut app, menu);
    paint_message(&mut app, choose);
    assert!(app.world().resource::<PaintState>().sessions[&0].graph_map_menu.is_none());

    paint_message(&mut app, PaintAction::BakeStart);
    let binding = app.world().resource::<PaintState>().sessions[&0].binding.unwrap();
    wait(&mut app, &|world| {
        let session = &world.resource::<PaintState>().sessions[&0];
        let doc = &world.resource::<editing::State>().documents[&0];
        let current = world.resource::<bake::State>().maps(0, doc, binding, bake_size);
        current.is_some_and(|maps| session.graph_bake.as_ref()
            .is_some_and(|rendered| rendered.ptr_eq(&Arc::downgrade(maps))))
    });
    let world = app.world();
    let doc = &world.resource::<editing::State>().documents[&0];
    let maps = world.resource::<bake::State>().maps(0, doc, binding, bake_size).unwrap();
    let fill = world.resource::<PaintState>().sessions[&0].active_texture_fill().unwrap();
    let updated = fill.images[0].as_ref().unwrap().pixels();
    let painted = doc.objects.iter().find(|object| object.id == binding.0).unwrap()
        .materials[binding.1].paint.as_ref().unwrap();
    let saved_fill = painted.stack.layers[painted.stack.selected].texture_fill.as_ref().unwrap();
    assert_eq!(saved_fill.images[0].as_ref().unwrap().pixels(), updated);
    let bytes = maps.rgba(bake::BakeMap::PartRandom);
    assert_ne!(initial.pixels(), updated);
    assert_eq!(updated.dimensions(), (64, 64));
    for (x, y, pixel) in updated.enumerate_pixels() {
        let coord = |p| (2 * p + 1) * bake_size / 128;
        let index = (coord(y) * bake_size + coord(x)) as usize * 4;
        assert!(pixel.0[0].abs_diff(bytes[index]) <= 1);
    }
    let expected = updated.get_pixel(16, 32).0[0];
    paint_message(&mut app, PaintAction::GraphCanvas);
    warm(&mut app, 8);
    let point = {
        let world = app.world_mut();
        let mut previews = world.query::<(&ui::NodePreview, &ComputedNode, &UiGlobalTransform)>();
        let (_, node, transform) = previews.iter(world).find(|(node, _, _)| node.0 == map_node).unwrap();
        (transform.affine().translation - Vec2::new(node.size().x * 0.25, 0.)).round().as_uvec2()
    };
    let screenshot = std::env::temp_dir().join(format!("bevy-mesh-map-{}.png", bevy::asset::uuid::Uuid::new_v4()));
    capture_when(&mut app, &render_target, &screenshot, |image| {
        image.get_pixel(point.x, point.y).0[0].abs_diff(expected) <= 1
    });
    std::fs::remove_file(screenshot).unwrap();

    let map = bake::BakeMap::PartRandom;
    paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
    paint_message(&mut app, PaintAction::Tool(PaintTool::Brush));
    paint_message(&mut app, PaintAction::View(View::Three));
    paint_message(&mut app, PaintAction::Preview3d(Preview3d::Baked(map)));
    wait(&mut app, &|world| world.resource::<bake::State>().preview(map).is_some());
    warm(&mut app, 4);
    {
        let world = app.world_mut();
        let image = world.resource::<bake::State>().preview(map).unwrap().clone();
        let displayed = world.query::<(&sets::SetPart, &MeshMaterial3d<StandardMaterial>)>()
            .iter(world).find(|(part, _)| (part.0, part.1) == binding).unwrap().1.0.clone();
        let material = world.resource::<Assets<StandardMaterial>>().get(&displayed).unwrap();
        assert!(material.unlit);
        assert_eq!(material.base_color_texture.as_ref(), Some(&image));
    }
    let history = app.world().resource::<PaintState>().sessions[&0].undo.len();
    let center = labelled(&mut app, "3D-вид модели").center();
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].undo.len(), history);
    assert!(app.world().resource::<Editor>().status.contains("Запечённая карта открыта для просмотра"));
    paint_message(&mut app, PaintAction::Preview3d(Preview3d::Material));
    paint_message(&mut app, PaintAction::Undo);
    paint_message(&mut app, PaintAction::Panel(Panel::Maps));
    paint_message(&mut app, PaintAction::BakePreview(Some(map)));
    let gamma = PaintAction::BakeAdjustLevels(map, bake::levels::Field::Gamma, 10);
    scroll_to_paint_control(&mut app, gamma);
    paint_click(&mut app, window, gamma);
    wait(&mut app, &|world| {
        let doc = &world.resource::<editing::State>().documents[&0];
        let Some(current) = world.resource::<bake::State>().maps(0, doc, binding, bake_size) else {
            return false;
        };
        world.resource::<PaintState>().sessions[&0].graph_bake.as_ref()
            .is_some_and(|rendered| rendered.ptr_eq(&Arc::downgrade(current)))
    });
    let world = app.world();
    let doc = &world.resource::<editing::State>().documents[&0];
    let material = &doc.objects.iter().find(|object| object.id == binding.0).unwrap().materials[binding.1];
    assert_eq!(bake::levels::get(material, map).gamma, 110);
    let current = world.resource::<bake::State>().maps(0, doc, binding, bake_size).unwrap();
    let corrected = current.rgba(map);
    assert_ne!(corrected, bytes);
    let fill = world.resource::<PaintState>().sessions[&0].active_texture_fill().unwrap();
    for (x, y, pixel) in fill.images[0].as_ref().unwrap().pixels().enumerate_pixels() {
        let coord = |p| (2 * p + 1) * bake_size / 128;
        let index = (coord(y) * bake_size + coord(x)) as usize * 4;
        assert!(pixel.0[0].abs_diff(corrected[index]) <= 1);
    }
    paint_message(&mut app, PaintAction::View(View::Two));
    paint_message(&mut app, PaintAction::BakeFixTool(map));
    paint_message(&mut app, PaintAction::BakeFixValue(100));
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        assert_eq!(session.active_bake_fix(), Some(map));
        assert!(!session.graph_canvas_open());
        session.size = 52.;
        session.flow = 100.;
        session.hardness = 100.;
        session.opacity = 100.;
    }
    let uv_point = Vec2::new(0.25, 0.5);
    let at = {
        let world = app.world_mut();
        let (node, transform) = world.query_filtered::<
            (&ComputedNode, &UiGlobalTransform), With<engine::TextureCanvas>,
        >().single(world).unwrap();
        let size = node.size() * node.inverse_scale_factor();
        transform.affine().translation * node.inverse_scale_factor() + (uv_point - Vec2::splat(0.5)) * size
    };
    let readbacks = app.world().resource::<engine::gpu::Queue>().statistics().2;
    move_pointer(&mut app, window, at);
    mouse(&mut app, window, ButtonState::Pressed);
    let screenshot = std::env::temp_dir().join(format!("bevy-bake-fix-{}.png", bevy::asset::uuid::Uuid::new_v4()));
    capture_when(&mut app, &render_target, &screenshot, |image| {
        let p = at.as_uvec2();
        image.get_pixel(p.x, p.y).0[..3].iter().all(|&value| value >= 250)
    });
    std::fs::remove_file(screenshot).unwrap();
    assert_eq!(app.world().resource::<engine::gpu::Queue>().statistics().2, readbacks);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    warm(&mut app, 3);
    let pixel = ((uv_point.y * bake_size as f32) as usize * bake_size as usize
        + (uv_point.x * bake_size as f32) as usize) * 4;
    let read_map = |app: &App| {
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        world.resource::<bake::State>().maps(0, doc, binding, bake_size).unwrap().rgba(map)
    };
    assert!(read_map(&app)[pixel] >= 250);
    assert_eq!(app.world().resource::<engine::gpu::Queue>().statistics().2, readbacks + 1);

    paint_message(&mut app, PaintAction::Tool(PaintTool::Eraser));
    // A slightly larger eraser clears all coverage, including the antialiased rim.
    app.world_mut().resource_mut::<PaintState>().sessions.get_mut(&0).unwrap().size = 64.;
    move_pointer(&mut app, window, at);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    warm(&mut app, 3);
    assert_eq!(read_map(&app), corrected);
    paint_message(&mut app, PaintAction::Undo);
    assert!(read_map(&app)[pixel] >= 250, "undo must restore the correction before the eraser");
    paint_message(&mut app, PaintAction::BakeClearFix(map));
    assert_eq!(read_map(&app), corrected);

    paint_message(&mut app, PaintAction::View(View::Three));
    paint_message(&mut app, PaintAction::Tool(PaintTool::Brush));
    {
        let mut camera = app.world_mut().resource_mut::<Viewer>();
        camera.center = Vec3::new(1.5, 0.5, 0.);
        camera.distance = 5.;
        camera.yaw = 0.;
        camera.pitch = 0.;
        camera.orthographic = true;
    }
    warm(&mut app, 4);
    let on_model = {
        let world = app.world_mut();
        let (node, transform) = world.query_filtered::<
            (&ComputedNode, &UiGlobalTransform), With<Canvas>,
        >().single(world).unwrap();
        let rect = Rect::from_center_size(
            transform.affine().translation * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        );
        editing::input::project(world.resource::<Viewer>(), rect, Vec3::new(2.5, 0.5, 0.)).unwrap()
    };
    move_pointer(&mut app, window, on_model);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    warm(&mut app, 3);
    let other_part = (bake_size as usize / 2 * bake_size as usize + bake_size as usize * 3 / 4) * 4;
    let painted = read_map(&app);
    assert!(painted[other_part] >= 250);
    assert_eq!(painted[pixel], corrected[pixel]);

    use bevy::{input::mouse::MouseButtonInput, window::{CursorMoved, WindowEvent}};
    paint_message(&mut app, PaintAction::View(View::Two));
    app.world_mut().resource_mut::<PaintState>().sessions.get_mut(&0).unwrap().size = 14.;
    let (origin, size) = {
        let world = app.world_mut();
        let (node, transform) = world.query_filtered::<
            (&ComputedNode, &UiGlobalTransform), With<engine::TextureCanvas>,
        >().single(world).unwrap();
        let size = node.size() * node.inverse_scale_factor();
        (transform.affine().translation * node.inverse_scale_factor() - size * 0.5, size)
    };
    let taps = [Vec2::new(0.16, 0.22), Vec2::new(0.34, 0.22),
        Vec2::new(0.16, 0.78), Vec2::new(0.34, 0.78)];
    let image = app.world().resource::<bake::State>().fix_preview.as_ref().unwrap().image.id();
    let readbacks = app.world().resource::<engine::gpu::Queue>().statistics().2;
    for tap in taps {
        app.world_mut().write_message(WindowEvent::CursorMoved(CursorMoved {
            window, position: origin + tap * size, delta: None,
        }));
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(WindowEvent::MouseButtonInput(MouseButtonInput {
                window, button: MouseButton::Left, state,
            }));
        }
    }
    frame(&mut app);
    assert!(app.world().resource::<editing::State>().documents[&0].paint_busy);
    assert_eq!(app.world().resource::<engine::gpu::Queue>().statistics().2, readbacks);
    paint_ready(&mut app);
    warm(&mut app, 3);
    assert_eq!(app.world().resource::<engine::gpu::Queue>().statistics().2, readbacks + taps.len() as u64);
    assert_eq!(app.world().resource::<bake::State>().fix_preview.as_ref().unwrap().image.id(), image);
    let indices = taps.map(|uv| {
        ((uv.y * bake_size as f32) as usize * bake_size as usize + (uv.x * bake_size as f32) as usize) * 4
    });
    let burst = read_map(&app);
    assert!(indices.iter().all(|&i| burst[i] >= 250 && painted[i] < 250));

    // A canceled stroke restores the GPU preview without publishing another snapshot.
    let at = origin + uv_point * size;
    move_pointer(&mut app, window, at);
    mouse(&mut app, window, ButtonState::Pressed);
    let screenshot = std::env::temp_dir().join(format!("bevy-bake-cancel-{}.png", bevy::asset::uuid::Uuid::new_v4()));
    let point = at.as_uvec2();
    capture_when(&mut app, &render_target, &screenshot, |image| image.get_pixel(point.x, point.y).0[0] >= 250);
    fill_modifier(&mut app, window, KeyCode::Escape, Key::Escape, ButtonState::Pressed);
    fill_modifier(&mut app, window, KeyCode::Escape, Key::Escape, ButtonState::Released);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    capture_when(&mut app, &render_target, &screenshot, |image| {
        image.get_pixel(point.x, point.y).0[0].abs_diff(burst[pixel]) <= 2
    });
    std::fs::remove_file(screenshot).unwrap();
    assert_eq!(read_map(&app), burst);
    assert_eq!(app.world().resource::<engine::gpu::Queue>().statistics().2, readbacks + taps.len() as u64);

    paint_message(&mut app, PaintAction::Undo);
    let undone = read_map(&app);
    assert!(indices[..3].iter().all(|&i| undone[i] >= 250));
    assert_eq!(undone[indices[3]], painted[indices[3]]);
}

#[test]
#[ignore = "requires GPU; checks automatic bake for new texture sets and preserves saved maps"]
fn gpu_paint_uses_import_resolution_and_bakes_unprepared_sets() {
    use crate::editor::{
        model_mesh::{Corner, Face, Geometry},
        ui::model_viewer::editing::uv::UvState,
    };
    use crate::editor::model_paint_data::{mask::BakedInputs, solid};
    let (mut app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    {
        let mut prefs = app.world_mut().resource_mut::<UvState>();
        while prefs.texture_size_on_import() != 512 {
            prefs.cycle_texture_size().unwrap();
        }
        prefs.toggle_bake_on_load().unwrap();
    }
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    let saved = Arc::new(Surface {
        size: 16,
        base: std::array::from_fn(|_| solid(16, [128; 4])),
        stack: Stack::default(),
        material: serde_json::Value::Null,
        baked: Some(Arc::new(BakedInputs {
            size: 16,
            edge_reach: 1.,
            disabled_inputs: 0,
            texels: vec![[128; crate::editor::model_paint_data::mask::INPUT_CHANNELS]; 16 * 16],
            part_ids: None,
            custom: vec![],
        })),
    });
    let object_id = {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        assert_eq!(doc.paint_default_size, 512);
        assert!(doc.auto_bake_pending);
        let object = &mut doc.objects[0];
        object.materials = vec![object.materials[0].clone(); 3];
        object.materials[2].paint = Some(saved.clone());
        let mut geometry = Geometry::default();
        for slot in 0..3 {
            let positions = [Vec3::ZERO, Vec3::X, Vec3::Y];
            geometry.positions.extend(positions.map(|p| p + Vec3::X * (slot * 2) as f32));
            let corners = [Vec2::ZERO, Vec2::X, Vec2::Y].into_iter().enumerate()
                .map(|(i, uv)| {
                    let mut corner = Corner::new(slot * 3 + i);
                    corner.uv = uv;
                    corner
                }).collect();
            geometry.faces.push(Face { corners, material: slot, smooth: false });
        }
        object.geometry = geometry.into();
        let id = object.id;
        doc.saved = doc.objects.clone();
        doc.revision += 1;
        id
    };
    assert!(app.world().resource::<bake::State>().progress().is_none());
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(
        editing::EditAction::Mode(Mode::Paint),
    )));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(75);
    loop {
        frame(&mut app);
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        let baked = world.resource::<bake::State>();
        if (0..2).all(|slot| baked.maps(0, doc, (object_id, slot), 512).is_some()) {
            assert!(baked.maps(0, doc, (object_id, 2), 16).is_none());
            assert_eq!(world.resource::<PaintState>().sessions[&0].texture_size, 512);
            assert!(!doc.dirty(), "caching mesh maps must leave the asset clean");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{}", world.resource::<Editor>().status);
    }
    paint_message(&mut app, PaintAction::Set(2));
    let world = app.world();
    let doc = &world.resource::<editing::State>().documents[&0];
    assert_eq!(world.resource::<PaintState>().sessions[&0].texture_size, 16);
    assert!(Arc::ptr_eq(doc.objects[0].materials[2].paint.as_ref().unwrap(), &saved));
}

#[test]
#[ignore = "requires GPU; saves bake settings through the editor and restores them after reopening"]
fn gpu_paint_bake_settings_save_and_reopen() {
    use crate::editor::model_document::glb::Glb;
    let directory = std::env::temp_dir().join(format!("bevy-bake-settings-{}", bevy::asset::uuid::Uuid::new_v4()));
    let mut project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    copy_tree(&project.asset_root, &directory);
    project.asset_root = directory.clone();
    let path = directory.join("models/block.glb");
    let (mut app, _, _) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    let doc_loaded = |app: &mut App| {
        for _ in 0..240 {
            frame(app);
            if app.world().resource::<editing::State>().documents.contains_key(&0) {
                return;
            }
        }
        panic!("Model did not load: {}", app.world().resource::<Editor>().status);
    };
    let enter_paint = |app: &mut App| {
        app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Mode(Mode::Paint))));
        warm(app, 10);
        paint_ready(app);
    };
    app.world_mut().write_message(Action::ViewModel(0));
    doc_loaded(&mut app);
    enter_paint(&mut app);
    paint_message(&mut app, PaintAction::BakeQuality(bake::Quality::Standard));
    paint_message(&mut app, PaintAction::BakeSelectAllMaps(false));
    paint_message(&mut app, PaintAction::BakeRange { front: true, percent: 17 });
    let doc = &app.world().resource::<editing::State>().documents[&0];
    assert_eq!(doc.bake_settings.quality, bake::Quality::Standard);
    assert_eq!(doc.bake_settings.map_bits, 0);
    assert_eq!(doc.bake_settings.front, 17);
    assert!(doc.dirty());
    let expected = doc.bake_settings.clone();

    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Save)));
    let start = std::time::Instant::now();
    loop {
        frame(&mut app);
        if !app.world().resource::<editing::State>().documents[&0].dirty() {
            break;
        }
        assert!(start.elapsed().as_secs() < 45, "{}", app.world().resource::<Editor>().status);
    }
    let glb = Glb::load(&path).unwrap();
    assert_eq!(crate::editor::model_bake_settings::read(&glb.json, &path), expected);
    app.world_mut().write_message(Action::CloseModelTab(0));
    warm(&mut app, 8);
    assert!(!app.world().resource::<editing::State>().documents.contains_key(&0));
    app.world_mut().write_message(Action::ViewModel(0));
    doc_loaded(&mut app);
    enter_paint(&mut app);
    let world = app.world();
    let projection = &world.resource::<PaintState>().sessions[&0].bake_projection;
    assert_eq!(projection.quality, bake::Quality::Standard);
    assert_eq!(projection.maps.bits(), 0);
    assert_eq!(projection.front, 17);
    assert!(!world.resource::<editing::State>().documents[&0].dirty());
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires GPU; saves a cage brush stroke and restores its scales in the reopened workspace"]
fn gpu_paint_cage_stroke_saves_and_reopens() {
    use crate::editor::{model_document::glb::Glb, model_mesh::{Corner, Face, Geometry}};
    let dir = std::env::temp_dir().join(format!("bevy-cage-save-{}", bevy::asset::uuid::Uuid::new_v4()));
    let mut project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    copy_tree(&project.asset_root, &dir);
    project.asset_root = dir.clone();
    let (mut app, window, _) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    let reopen = |app: &mut App| {
        app.world_mut().write_message(Action::ViewModel(0));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
        loop {
            frame(app);
            if app.world().resource::<editing::State>().documents.contains_key(&0) {
                return;
            }
            assert!(std::time::Instant::now() < deadline, "{}", app.world().resource::<Editor>().status);
        }
    };
    reopen(&mut app);
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        let object = &mut doc.objects[0];
        let corners = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y].into_iter().enumerate()
            .map(|(i, uv)| {
                let mut corner = Corner::new(i);
                corner.uv = uv;
                corner
            }).collect();
        object.geometry = Geometry {
            positions: vec![
                Vec3::new(-1., -1., 0.), Vec3::new(1., -1., 0.),
                Vec3::new(1., 1., 0.), Vec3::new(-1., 1., 0.),
            ],
            faces: vec![Face { corners, material: 0, smooth: false }],
            ..default()
        }.into();
        object.transform = Transform::IDENTITY;
        let mut high = object.clone();
        high.source_node = None;
        high.transform.translation.z = 0.03;
        let source = dir.join("models/high.glb");
        std::fs::write(&source, Glb::new_asset(&[high]).unwrap()).unwrap();
        doc.bake_settings.high_poly = Some(source);
        doc.mode = Mode::Paint;
        doc.paint_default_size = 512;
        doc.revision += 1;
    }
    warm(&mut app, 12);
    paint_message(&mut app, PaintAction::Panel(Panel::Maps));
    paint_message(&mut app, PaintAction::View(View::Split));
    paint_message(&mut app, PaintAction::BakePaintCage);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(35);
    while !app.world().resource::<bake::cage_brush::Brush>().pick_ready() {
        frame(&mut app);
        assert!(std::time::Instant::now() < deadline);
    }
    let viewport = labelled(&mut app, "3D-вид модели");
    let point = editing::input::project(
        app.world().resource::<Viewer>(), viewport, Vec3::new(-0.94, -0.94, 0.),
    ).unwrap();
    move_pointer(&mut app, window, point);
    mouse(&mut app, window, ButtonState::Pressed);
    warm(&mut app, 10);
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 3);
    let painted_scale = {
        let doc = &app.world().resource::<editing::State>().documents[&0];
        let scale = doc.cage_edits[&doc.objects[0].id].scales.values[0];
        assert!(scale > 1.);
        scale
    };
    paint_message(&mut app, PaintAction::Undo);
    assert!(app.world().resource::<editing::State>().documents[&0].cage_edits.is_empty());
    paint_message(&mut app, PaintAction::Redo);
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Save)));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    while app.world().resource::<editing::State>().documents[&0].dirty() {
        frame(&mut app);
        assert!(std::time::Instant::now() < deadline, "{}", app.world().resource::<Editor>().status);
    }
    app.world_mut().write_message(Action::CloseModelTab(0));
    warm(&mut app, 8);
    assert!(!app.world().resource::<editing::State>().documents.contains_key(&0));
    reopen(&mut app);
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Mode(Mode::Paint))));
    warm(&mut app, 12);
    let world = app.world();
    let object = &world.resource::<editing::State>().documents[&0].objects[0];
    let corner = object.geometry.positions.iter().position(|&p| p == Vec3::new(-1., -1., 0.)).unwrap();
    let session = &world.resource::<PaintState>().sessions[&0];
    assert_eq!(bake::projection_for(session, object).manual.unwrap().values[corner], painted_scale);
    drop(app);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires GPU; verifies world-space graph inputs and the live preview after rotation"]
fn gpu_surface_graph_follows_model_geometry_without_baking() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    use bevy::render::renderer::{RenderDevice, RenderQueue};
    use material_painter_graph::{Graph, Value};
    use std::{path::Path, time::Instant};

    let project = crate::EditorProject::load(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (mut app, _, render_target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        let mut geometry = Geometry::default();
        geometry.positions = vec![
            Vec3::ZERO,
            Vec3::Z,
            Vec3::X + Vec3::Z,
            Vec3::X,
            Vec3::new(2., 2., 0.),
            Vec3::new(3., 2., 0.),
            Vec3::new(3., 3., 0.),
            Vec3::new(2., 3., 0.),
        ];
        for island in 0..2 {
            let corners = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]
                .into_iter()
                .enumerate()
                .map(|(i, uv)| {
                    let mut corner = Corner::new(island * 4 + i);
                    corner.uv = Vec2::new((island as f32 + uv.x) * 0.5, uv.y);
                    corner
                })
                .collect();
            geometry.faces.push(Face {
                corners,
                material: 0,
                smooth: false,
            });
        }
        doc.objects[0].geometry = geometry.into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 12);
    paint_ready(&mut app);
    let mut graph = Graph::new("Smart Mask QA");
    graph.res = 64;
    let mask = graph.add("smart_mask", [12., 24.]);
    let output = graph.add("output", [250., 24.]);
    assert!(graph.connect(mask, output, "input"));
    let params = &mut graph
        .nodes
        .iter_mut()
        .find(|node| node.id == mask)
        .unwrap()
        .params;
    params.insert("noise".into(), Value::Float(0.));
    params.insert("input_1".into(), Value::Enum(10));

    let world = app.world();
    let binding = world.resource::<PaintState>().sessions[&0].binding.unwrap();
    let doc = &world.resource::<editing::State>().documents[&0];
    let source = material_graph::surface::Source::capture(doc, binding).unwrap();
    let render = |graph, images: &texture_fill::GraphImages| {
        material_graph::prepare_graph(
            graph,
            images,
            64,
            material_graph::Inputs {
                maps: None,
                surface: Some(&source),
            },
            world.resource::<RenderDevice>(),
            world.resource::<RenderQueue>(),
        )
        .unwrap()
    };
    let samples = |fill: &texture_fill::Fill| {
        let pixels = fill.images[0].as_ref().unwrap().pixels();
        [pixels.get_pixel(16, 32).0[0], pixels.get_pixel(48, 32).0[0]]
    };
    let height = render(graph.clone(), &Default::default());
    assert_eq!(samples(height.texture_fill.as_ref().unwrap()), [0, 255]);
    graph
        .nodes
        .iter_mut()
        .find(|node| node.id == mask)
        .unwrap()
        .params
        .insert("input_1".into(), Value::Enum(7));
    let facing = render(graph, &Default::default());
    assert_eq!(samples(facing.texture_fill.as_ref().unwrap()), [255, 0]);

    let mut projected = Graph::new("Triplanar QA");
    projected.res = 64;
    let layer = projected.add("layer", [0., 0.]);
    let output = projected.add("output", [200., 0.]);
    assert!(projected.connect(layer, output, "input"));
    let path = "graph-images/bands.png";
    projected.nodes[0]
        .params
        .insert("image".into(), Value::Image(Some(path.into())));
    let bands = image::RgbaImage::from_fn(16, 16, |_, y| {
        image::Rgba(if y < 8 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        })
    });
    let images = [(
        path.to_owned(),
        Arc::new(texture_fill::Bitmap::from_rgba("bands.png".into(), bands).unwrap()),
    )]
    .into_iter()
    .collect();
    let projected = render(projected, &images);
    let pixels = projected.texture_fill.as_ref().unwrap().images[0]
        .as_ref()
        .unwrap()
        .pixels();
    // On the upward face, moving along U follows world Z and crosses the bands.
    assert_eq!(pixels.get_pixel(4, 32).0, [255, 0, 0, 255]);
    assert_eq!(pixels.get_pixel(27, 32).0, [0, 0, 255, 255]);
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.insert_smart_material(vec![facing], session.insertion());
    }
    warm(&mut app, 12);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::GraphCanvas);
    warm(&mut app, 8);
    let point = {
        let world = app.world_mut();
        let mut previews = world.query::<(&ui::NodePreview, &ComputedNode, &UiGlobalTransform)>();
        let (_, node, transform) = previews
            .iter(world)
            .find(|(node, _, _)| node.0 == mask)
            .unwrap();
        (transform.affine().translation - Vec2::new(node.size().x * 0.25, 0.))
            .round()
            .as_uvec2()
    };
    let screenshot = std::env::temp_dir().join(format!(
        "bevy-surface-{}.png",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    capture_when(&mut app, &render_target, &screenshot, |image| {
        image.get_pixel(point.x, point.y).0[0] >= 250
    });
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        doc.objects[0].transform.rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        doc.revision += 1;
    }
    let started = Instant::now();
    loop {
        frame(&mut app);
        let session = &app.world().resource::<PaintState>().sessions[&0];
        if samples(session.active_texture_fill().unwrap()) == [0, 255] {
            break;
        }
        assert!(
            started.elapsed().as_secs() < 60,
            "{}",
            app.world().resource::<Editor>().status
        );
    }
    capture_when(&mut app, &render_target, &screenshot, |image| {
        image.get_pixel(point.x, point.y).0[0] <= 5
    });
    std::fs::remove_file(screenshot).unwrap();
}

#[test]
#[ignore = "requires GPU; creates, paints, edits and saves a graph mask through the paint workspace"]
fn gpu_graph_mask_keeps_painted_alpha_filters_and_graph_on_save() {
    use crate::editor::{model_document::glb::Glb, model_primitives::Primitive};
    use material_graph::Edit;
    use std::{fs, path::Path, time::Instant};

    let project = crate::EditorProject::load(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (mut app, window, _) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        doc.objects[0].geometry = Primitive::Cube
            .geometry(&Primitive::Cube.defaults())
            .unwrap()
            .into();
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 12);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::Resolution(256));
    paint_message(&mut app, PaintAction::Add(LayerKind::Fill));
    paint_message(&mut app, PaintAction::AddMask(true));
    paint_message(&mut app, PaintAction::AddFilter(FilterKind::Invert));
    paint_message(&mut app, PaintAction::AddFilter(FilterKind::Blur));
    paint_message(&mut app, PaintAction::AdjustFilter(1, 1, 1));
    paint_message(&mut app, PaintAction::AddFilter(FilterKind::Warp));
    paint_message(&mut app, PaintAction::AdjustFilter(2, 2, 1));
    let wait = |app: &mut App, done: &dyn Fn(&Session) -> bool| {
        let start = Instant::now();
        loop {
            frame(app);
            let session = &app.world().resource::<PaintState>().sessions[&0];
            if !session.smart_loading && !session.busy && done(session) {
                break;
            }
            assert!(
                start.elapsed().as_secs() < 45,
                "{}",
                app.world().resource::<Editor>().status
            );
        }
    };
    let mask = |app: &App| {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        session.stack.layers[session.stack.selected]
            .mask
            .as_ref()
            .unwrap()
            .clone()
    };
    scroll_to_paint_control(&mut app, PaintAction::AddGraphMask);
    paint_click(&mut app, window, PaintAction::AddGraphMask);
    wait(&mut app, &|session| session.graph_canvas_open());
    let output = material_graph::read(&mask(&app).graph.as_ref().unwrap().payload)
        .unwrap()
        .outputs(&material_graph::library())[material_painter_graph::MASK_USAGE]
        .unwrap();
    let color_def = material_graph::library()
        .defs
        .iter()
        .position(|def| def.id == "color")
        .unwrap();
    paint_message(
        &mut app,
        PaintAction::GraphEdit(Edit::Add(color_def, IVec2::new(200, 200))),
    );
    let color = app.world().resource::<PaintState>().sessions[&0]
        .graph_node
        .unwrap();
    paint_message(
        &mut app,
        PaintAction::GraphEdit(Edit::Connect(color, output, 0)),
    );
    wait(&mut app, &|session| {
        session.graph().is_some_and(|payload| {
            material_graph::read(payload)
                .unwrap()
                .node(output)
                .unwrap()
                .links
                .get("input")
                == Some(&color)
        })
    });
    let connected = mask(&app);
    assert!((125..=130).contains(&connected.effective()[0]));

    paint_message(&mut app, PaintAction::GraphCanvas);
    paint_message(&mut app, PaintAction::MaskValue(0));
    let center = labelled(&mut app, "Рисование: 2D-вид UV").center();
    move_pointer(&mut app, window, center - Vec2::new(15., 0.));
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, center + Vec2::new(15., 0.));
    mouse(&mut app, window, ButtonState::Released);
    wait(&mut app, &|session| {
        session.stack.layers[session.stack.selected]
            .mask
            .as_ref()
            .unwrap()
            .pixels
            != connected.pixels
    });
    let painted = mask(&app);
    assert_eq!(painted.filters[0].kind, FilterKind::Invert);
    assert!(painted.filters[1].wrap);
    assert_eq!(painted.filters[2].value(2), 1.);
    assert_ne!(painted.effective(), connected.effective());
    paint_message(&mut app, PaintAction::GraphAdjust(color, 0, 0, 1));
    let swatch = |mask: &Mask| {
        mask.graph
            .as_ref()
            .unwrap()
            .output
            .as_ref()
            .unwrap()
            .pixels()
            .get_pixel(0, 0)
            .0[0]
    };
    wait(&mut app, &|session| {
        swatch(
            session.stack.layers[session.stack.selected]
                .mask
                .as_ref()
                .unwrap(),
        ) > swatch(&painted)
    });
    let changed = mask(&app);
    assert_eq!(changed.pixels, painted.pixels);
    assert_eq!(changed.filters, painted.filters);
    assert_ne!(changed.effective(), painted.effective());
    paint_message(&mut app, PaintAction::Undo);
    wait(&mut app, &|session| {
        session.stack.layers[session.stack.selected]
            .mask
            .as_ref()
            .unwrap()
            .graph
            == painted.graph
    });
    assert_eq!(mask(&app).effective(), painted.effective());

    paint_message(&mut app, PaintAction::AddFilter(FilterKind::Outline));
    let outline = mask(&app);
    assert_eq!(outline.filters.last().unwrap().kind, FilterKind::Outline);
    assert_ne!(outline.effective(), painted.effective());
    paint_message(&mut app, PaintAction::MaskValue(100));
    move_pointer(&mut app, window, center - Vec2::new(15., 0.));
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, center + Vec2::new(15., 0.));
    mouse(&mut app, window, ButtonState::Released);
    wait(&mut app, &|session| {
        session.stack.layers[session.stack.selected].mask.as_ref().unwrap().pixels != outline.pixels
    });
    let painted = mask(&app);

    let folder = std::env::temp_dir().join(format!(
        "bevy-graph-mask-{}",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&folder).unwrap();
    let doc = &app.world().resource::<editing::State>().documents[&0];
    let bytes = doc.source.encode(&doc.objects, &doc.original).unwrap();
    let path = folder.join("graph-mask.glb");
    fs::write(&path, bytes).unwrap();
    let loaded = Glb::load(&path).unwrap();
    let surface = loaded.json["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|mesh| mesh["primitives"].as_array().into_iter().flatten())
        .find_map(|primitive| loaded.paint_surface(primitive).unwrap())
        .unwrap();
    let restored = surface.stack.layers[surface.stack.selected]
        .mask
        .as_ref()
        .unwrap();
    assert_eq!(restored.graph, painted.graph);
    assert_eq!(restored.pixels, painted.pixels);
    assert_eq!(restored.effective(), painted.effective());

    let layer = surface.stack.layers[surface.stack.selected].clone();
    let (preset, skipped) = presets::save(
        &folder,
        presets::Capture::Material {
            name: "Граф маски QA".into(),
            layers: vec![layer],
        },
    )
    .unwrap();
    assert_eq!(skipped, 1); // Mesh-specific brush strokes are omitted from a portable preset.
    let preset = presets::load_material(&preset.path, 64).unwrap();
    let restored = preset[0].mask.as_ref().unwrap();
    assert_eq!(restored.graph, painted.graph);
    assert_eq!(restored.filters, painted.filters);
    let (preset, _) = presets::save(
        &folder,
        presets::Capture::Mask {
            name: "Отдельный граф маски".into(),
            mask: painted.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        presets::load_mask(&preset.path, 64).unwrap().graph,
        painted.graph
    );
    fs::remove_dir_all(folder).unwrap();
}

#[test]
#[ignore = "requires GPU; applies a graph from the shelf, edits a node, undoes the edit and reopens the GLB"]
fn gpu_model_paint_graph_presets_and_node_edit_survive_glb() {
    use bevy::render::renderer::{RenderDevice, RenderQueue};
    use crate::editor::model_document::glb::Glb;

    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let (mut app, window, target) = fixture_project(project);
    let device = app.world().resource::<RenderDevice>().clone();
    let queue = app.world().resource::<RenderQueue>().clone();
    for (i, recipe) in material_graph::PRESETS.iter().enumerate() {
        let layer = material_graph::prepare(i, 64, Default::default(), &device, &queue)
            .unwrap_or_else(|error| panic!("{} · {error}", recipe.name));
        let fill = layer.texture_fill.unwrap();
        assert_eq!(fill.graph.as_ref().unwrap()["name"], recipe.name);
        assert_eq!(fill.images[0].as_ref().unwrap().pixels().dimensions(), (64, 64));
    }
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) { break; }
    }
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 12);
    paint_ready(&mut app);
    paint_click(&mut app, window, PaintAction::Category(Category::Graphs));
    paint_click(&mut app, window, PaintAction::AddGraphMaterial(0));
    let settle = |app: &mut App| {
        let start = std::time::Instant::now();
        loop {
            frame(app);
            let session = &app.world().resource::<PaintState>().sessions[&0];
            let doc = &app.world().resource::<editing::State>().documents[&0];
            if !session.smart_loading && !doc.paint_busy { break; }
            assert!(start.elapsed().as_secs() < 60, "{}", app.world().resource::<Editor>().status);
        }
        warm(app, 5);
    };
    settle(&mut app);
    let fill = |app: &App| -> Arc<texture_fill::Fill> {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        session.stack.layers[session.stack.selected].texture_fill.as_ref()
            .unwrap_or_else(|| panic!("{}", app.world().resource::<Editor>().status)).clone()
    };
    let original = fill(&app);
    assert!(original.images[3].is_some());
    let graph = material_graph::read(original.graph.as_ref().unwrap()).unwrap();
    let tiles = graph.nodes.iter().find(|node| node.def == "tiles").unwrap();
    let columns = material_graph::library().find("tiles").unwrap().params.iter()
        .position(|param| param.name == "columns").unwrap();
    let inspector_click = |app: &mut App, action| {
        scroll_to_paint_control(app, action);
        paint_click(app, window, action);
    };
    inspector_click(&mut app, PaintAction::GraphSelectNode(tiles.id));
    inspector_click(&mut app, PaintAction::GraphAdjust(tiles.id, columns, 0, 1));
    settle(&mut app);
    let adjusted = fill(&app);
    assert!(original.graph != adjusted.graph, "{}", app.world().resource::<Editor>().status);
    assert_ne!(original.images[3].as_ref().unwrap().bytes, adjusted.images[3].as_ref().unwrap().bytes);

    let path = std::env::temp_dir().join(format!("bevy-material-graph-{}.glb", bevy::asset::uuid::Uuid::new_v4()));
    {
        let doc = &app.world().resource::<editing::State>().documents[&0];
        std::fs::write(&path, doc.source.encode(&doc.objects, &doc.original).unwrap()).unwrap();
    }
    let loaded = Glb::load(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let restored = loaded.json["meshes"].as_array().unwrap().iter().flat_map(|mesh| {
        mesh["primitives"].as_array().into_iter().flatten()
    }).find_map(|primitive| loaded.paint_surface(primitive).unwrap()).unwrap();
    let saved = restored.stack.layers.iter().find_map(|layer| layer.texture_fill.as_ref()).unwrap();
    assert_eq!(saved.graph, adjusted.graph);
    assert_eq!(saved.images[3].as_ref().unwrap().bytes, adjusted.images[3].as_ref().unwrap().bytes);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(fill(&app).graph, original.graph);

    inspector_click(&mut app, PaintAction::GraphCanvas);
    warm(&mut app, 8);
    let stage = labelled(&mut app, "Холст графа материала");
    let inspector = labelled(&mut app, "Параметры графа");
    assert!(inspector.min.x >= stage.max.x - 1.);
    let settings_tab = labelled(&mut app, "Параметры узлов").center();
    move_pointer(&mut app, window, settings_tab);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, Vec2::new(stage.center().x, stage.min.y + 8.));
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 8);
    let stage = labelled(&mut app, "Холст графа материала");
    let inspector = labelled(&mut app, "Параметры графа");
    assert!(inspector.max.y <= stage.min.y + 1.);
    let reset = labelled(&mut app, "Сбросить раскладку графа").center();
    move_pointer(&mut app, window, reset);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 8);
    let stage = labelled(&mut app, "Холст графа материала");
    assert!(labelled(&mut app, "Параметры графа").min.x >= stage.max.x - 1.);
    let center = |app: &mut App, name: &str| {
        let world = app.world_mut();
        let mut elements = world.query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>();
        let (_, node, transform) = elements.iter(world).find(|(label, node, _)| {
            label.0 == name && node.size().min_element() > 0.
        }).unwrap_or_else(|| panic!("No visible control: {name}"));
        transform.affine().translation * node.inverse_scale_factor()
    };
    let click_label = |app: &mut App, name: &str| {
        let p = center(app, name);
        move_pointer(app, window, p);
        mouse(app, window, ButtonState::Pressed);
        mouse(app, window, ButtonState::Released);
        warm(app, 4);
    };
    let current_graph = |app: &App| material_graph::read(fill(app).graph.as_ref().unwrap()).unwrap();
    let p = center(&mut app, &format!("Tiles #{}", tiles.id));
    move_pointer(&mut app, window, p);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, p + Vec2::new(38., 11.));
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 6);
    assert_ne!(current_graph(&app).node(tiles.id).unwrap().pos, tiles.pos);
    assert_eq!(fill(&app).images[3].as_ref().unwrap().bytes, original.images[3].as_ref().unwrap().bytes);

    settle(&mut app);
    click_label(&mut app, "Добавить узел");
    click_label(&mut app, "Checker");
    settle(&mut app);
    let checker = current_graph(&app).nodes.into_iter().find(|node| node.def == "checker").unwrap();
    let ramp = graph.nodes.iter().find(|node| node.def == "gradient_map").unwrap();
    click_label(&mut app, "Checker · выход");
    assert_eq!(
        app.world().resource::<ui::PaintUi>().pending_graph_source(),
        Some(checker.id),
        "The Checker output should select the source port"
    );
    let connection = PaintAction::GraphEdit(material_graph::Edit::Connect(checker.id, ramp.id, 0));
    assert!(app.world().resource::<PaintState>().sessions[&0].available(connection));
    click_label(&mut app, "Gradient Map · input");
    settle(&mut app);
    assert_eq!(
        current_graph(&app).node(ramp.id).unwrap().links["input"],
        checker.id,
        "Pending source {:?}, status: {}",
        app.world().resource::<ui::PaintUi>().pending_graph_source(),
        app.world().resource::<Editor>().status
    );
    assert_ne!(fill(&app).images[0].as_ref().unwrap().bytes, original.images[0].as_ref().unwrap().bytes);
    if let Some(path) = std::env::var_os("BEVY_GRAPH_CANVAS_QA") {
        capture_when(&mut app, &target, std::path::Path::new(&path), |_| true);
    }
    paint_message(&mut app, PaintAction::GraphEdit(material_graph::Edit::Disconnect(ramp.id, 0)));
    settle(&mut app);
    assert!(!current_graph(&app).node(ramp.id).unwrap().links.contains_key("input"));
    click_label(&mut app, &format!("Checker #{}", checker.id));
    click_label(&mut app, "Удалить узел");
    settle(&mut app);
    assert!(current_graph(&app).node(checker.id).is_none());
    paint_message(&mut app, PaintAction::Undo);
    assert!(current_graph(&app).node(checker.id).is_some());

    use material_graph::Edit;
    let rows = material_graph::library().find("tiles").unwrap().params.iter()
        .position(|param| param.name == "rows").unwrap();
    click_label(&mut app, &format!("Tiles #{}", tiles.id));
    inspector_click(&mut app, PaintAction::GraphEdit(Edit::Publish(tiles.id, columns)));
    settle(&mut app);
    assert_eq!(current_graph(&app).params.len(), 1);
    inspector_click(&mut app, PaintAction::GraphEdit(Edit::Bind(tiles.id, rows, 0)));
    settle(&mut app);
    let published = current_graph(&app);
    assert_eq!(published.params[0].binds.len(), 2);
    {
        let world = app.world_mut();
        let mut fields = world.query::<(&GraphLabel, &mut bevy::text::EditableText)>();
        fields.iter_mut(world).find(|(key, _)| **key == GraphLabel(0, false))
            .unwrap().1.editor_mut().set_text("Плотность кирпича");
    }
    inspector_click(&mut app, PaintAction::GraphEdit(Edit::LabelParameter(0, false)));
    settle(&mut app);
    assert_eq!(current_graph(&app).params[0].name, "Плотность кирпича");
    inspector_click(&mut app, PaintAction::GraphEdit(Edit::AdjustParameter(0, 0, 1)));
    settle(&mut app);
    let graph = current_graph(&app);
    let library = material_graph::library();
    let library = library.as_ref();
    let tiles = graph.node(tiles.id).unwrap();
    assert_eq!(library.value(tiles, "columns"), library.value(tiles, "rows"));
    assert_ne!(fill(&app).images[3].as_ref().unwrap().bytes, original.images[3].as_ref().unwrap().bytes);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(current_graph(&app).params[0].name, "Плотность кирпича");
}

#[test]
#[ignore = "requires GPU; verifies that Shader Maker presets render on the actual scene model"]
fn gpu_shader_preview_uses_the_painted_material_and_scene_lighting() {
    use crate::editor::model_primitives::Primitive;
    use std::{fs, path::Path};

    let project = crate::EditorProject::load(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        doc.objects[0].geometry = Primitive::Cube.geometry(&Primitive::Cube.defaults()).unwrap().into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::ZERO;
        viewer.distance = 3.0;
    }
    warm(&mut app, 12);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::Resolution(256));
    paint_message(&mut app, PaintAction::Add(LayerKind::Fill));
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.set_solid_fill(0, [233, 52, 22]);
        session.revision += 1;
    }
    warm(&mut app, 8);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::Panel(Panel::Preview));
    paint_message(&mut app, PaintAction::View(View::Three));

    let full = labelled(&mut app, "3D-вид модели");
    let point = full.center().round().as_uvec2();
    let path = std::env::temp_dir().join(format!(
        "bevy-shading-preview-{}.png", bevy::asset::uuid::Uuid::new_v4(),
    ));
    let pixel = |image: &image::RgbaImage| image.get_pixel(point.x, point.y).0;
    let red = |p: [u8; 4]| p[0] > p[1].saturating_add(20) && p[0] > p[2].saturating_add(25);
    let baseline = pixel(&capture_when(&mut app, &target, &path, |image| red(pixel(image))));
    let preset = |name| shader_preview::presets().iter().position(|graph| graph.name == name).unwrap();
    let pbr = PaintAction::PreviewShader(Some(preset("PBR")));
    scroll_to_paint_control(&mut app, pbr);
    paint_click(&mut app, window, pbr);
    warm(&mut app, 12);
    let close = |p: [u8; 4]| p.iter().zip(baseline).all(|(&a, b)| a.abs_diff(b) <= 5);
    capture_when(&mut app, &target, &path, |image| close(pixel(image)));

    let unlit = PaintAction::PreviewShader(Some(preset("Unlit")));
    scroll_to_paint_control(&mut app, unlit);
    paint_click(&mut app, window, unlit);
    capture_when(&mut app, &target, &path, |image| {
        let p = pixel(image);
        red(p) && p.iter().zip(baseline).any(|(&a, b)| a.abs_diff(b) > 10)
    });
    paint_message(&mut app, PaintAction::GraphCanvas);
    let split = labelled(&mut app, "3D-вид модели");
    assert!(split.width() < full.width() * 0.65);
    let split_point = split.center().round().as_uvec2();
    let preview = |image: &image::RgbaImage| image.get_pixel(split_point.x, split_point.y).0;
    capture_when(&mut app, &target, &path, |image| red(preview(image)));
    let graph = |app: &App| {
        let paint = app.world().resource::<PaintState>();
        material_graph::read(paint.sessions[&0].graph().unwrap()).unwrap()
    };
    let library = material_graph::library();
    let library = library.as_ref();
    let output = graph(&app).nodes.iter().find(|node| node.def == "shade_output").unwrap().id;
    let color = library.defs.iter().position(|def| def.id == "shade_color").unwrap();
    use material_graph::Edit;
    paint_message(&mut app, PaintAction::GraphEdit(Edit::Add(color, IVec2::new(180, 200))));
    let color = graph(&app).nodes.iter().find(|node| node.def == "shade_color").unwrap().id;
    paint_message(&mut app, PaintAction::GraphEdit(Edit::Connect(color, output, 0)));
    let neutral = |p: [u8; 4]| p[0].abs_diff(p[1]) < 5 && p[0].abs_diff(p[2]) < 5;
    let white = preview(&capture_when(&mut app, &target, &path, |image| {
        let p = preview(image);
        neutral(p) && p[0] > 140
    }));
    assert_eq!(graph(&app).node(output).unwrap().links["color"], color);
    for _ in 0..6 {
        app.world_mut().write_message(Action::ModelView(ModelViewAction::Paint(
            PaintAction::GraphAdjust(color, 1, 0, -1),
        )));
    }
    let dim = preview(&capture_when(&mut app, &target, &path, |image| {
        let p = preview(image);
        neutral(p) && p[0] < white[0].saturating_sub(10)
    }));
    paint_message(&mut app, PaintAction::GraphEdit(Edit::Remove(output)));
    assert!(app.world().resource::<Editor>().status.contains("Не удалось собрать шейдер"));
    {
        let world = app.world_mut();
        let mut text = world.query::<(&Text, &ComputedNode)>();
        assert!(text.iter(world).any(|(text, node)| {
            text.0.starts_with("Не удалось собрать шейдер") && node.size().y > 0.0
        }));
    }
    capture_when(&mut app, &target, &path, |image| preview(image) == dim);
    paint_message(&mut app, PaintAction::Undo);
    assert!(graph(&app).node(output).is_some());
    assert!(!app.world().resource::<Editor>().status.contains("Не удалось собрать шейдер"));
    paint_message(&mut app, PaintAction::Redo);
    assert!(graph(&app).node(output).is_none());
    paint_message(&mut app, PaintAction::Undo);
    let edited = graph(&app);
    paint_message(&mut app, PaintAction::GraphCanvas);
    paint_message(&mut app, PaintAction::PreviewShader(None));
    capture_when(&mut app, &target, &path, |image| close(pixel(image)));

    let directory = path.with_extension("");
    fs::create_dir(&directory).unwrap();
    let manifest = edited.to_json(library);
    let saved = material_graph::file::export(&directory, &manifest, &Default::default()).unwrap();
    assert_eq!(material_painter_graph::load_file(&saved, library).unwrap(), edited);
    app.world_mut().resource_mut::<PaintState>().shader_graph_test_path = Some(Some(saved));
    scroll_to_paint_control(&mut app, PaintAction::ImportShaderGraph);
    paint_click(&mut app, window, PaintAction::ImportShaderGraph);
    let start = std::time::Instant::now();
    let imported = loop {
        frame(&mut app);
        let session = &app.world().resource::<PaintState>().sessions[&0];
        if let Some(id) = session.preview_shader.filter(|&id| id >= shader_preview::presets().len()) {
            assert!(session.graph_canvas_open());
            assert!(!session.available(PaintAction::Undo));
            break id;
        }
        assert!(start.elapsed().as_secs() < 20, "{:?}", session.shader_import_error);
    };
    assert_eq!(graph(&app), edited);
    capture_when(&mut app, &target, &path, |image| preview(image) == dim);
    paint_message(&mut app, unlit);
    assert_ne!(app.world().resource::<PaintState>().sessions[&0].preview_shader, Some(imported));
    paint_message(&mut app, PaintAction::PreviewShader(Some(imported)));
    assert_eq!(graph(&app), edited);
    fs::remove_dir_all(directory).unwrap();
    fs::remove_file(path).unwrap();

    let mut shader = material_painter_graph::shading::compile(
        &shader_preview::presets()[0], library,
    ).unwrap();
    shader.code.push_str("\nfn invalid() -> f32 { return missing_value; }\n");
    let device = app.world().resource::<bevy::render::renderer::RenderDevice>();
    let error = material_painter_graph::shading::validate(device.wgpu_device(), &shader).unwrap_err();
    assert!(error.contains("missing_value"), "{error}");
}

#[test]
#[ignore = "requires GPU; writes a shared offscreen fixture for Material Painter viewport comparison"]
fn gpu_compare_paint_shading_viewport() {
    use crate::editor::model_primitives::Primitive;
    use bevy::{render::render_resource::TextureUsages, render::view::Tonemapping};
    use std::{fmt::Write, path::{Path, PathBuf}};
    let directory = PathBuf::from(std::env::var_os("PAINT_SHADING_DIR").unwrap());
    std::fs::create_dir_all(&directory).unwrap();
    let project = crate::EditorProject::load(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let (mut app, _, _) = fixture_project(project);
    let adapter = app.world().resource::<bevy::render::renderer::RenderAdapterInfo>();
    std::fs::write(directory.join("bevy-adapter.txt"), format!("{} {:?}", adapter.name, adapter.backend)).unwrap();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    let mut cube = Primitive::Cube.geometry(&Primitive::Cube.defaults()).unwrap();
    let (min, max) = cube.positions.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(min, max), &p| (min.min(p), max.max(p)),
    );
    let center = (min + max) * 0.5;
    let scale = 2.0 / (max - min).max_element();
    let mut obj = String::new();
    for position in &mut cube.positions {
        *position = (*position - center) * scale;
        writeln!(obj, "v {} {} {}", position.x, position.y, position.z).unwrap();
    }
    let mut uv = 1;
    for (i, face) in cube.faces.iter().enumerate() {
        let normal = cube.normal(face).normalize();
        writeln!(obj, "vn {} {} {}", normal.x, normal.y, normal.z).unwrap();
        for corner in &face.corners {
            writeln!(obj, "vt {} {}", corner.uv.x, 1.0 - corner.uv.y).unwrap();
        }
        for triangle in cube.triangles(face) {
            obj.push('f');
            for corner in triangle {
                write!(obj, " {}/{}/{}", face.corners[corner].vertex + 1, uv + corner, i + 1).unwrap();
            }
            obj.push('\n');
        }
        uv += face.corners.len();
    }
    std::fs::write(directory.join("cube.obj"), obj).unwrap();
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        doc.objects[0].geometry = cube.into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 12);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::Resolution(256));
    paint_message(&mut app, PaintAction::Add(LayerKind::Fill));
    let color = [145, 94, 53];
    let roughness = 128;
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.set_solid_fill(0, color);
        session.set_solid_fill(1, [roughness; 3]);
        session.set_solid_fill(2, [0; 3]);
        session.revision += 1;
    }
    paint_message(&mut app, PaintAction::Panel(Panel::Preview));
    paint_message(&mut app, PaintAction::View(View::Three));
    let yaw = 0.65;
    let pitch = 0.35;
    let distance = 3.8;
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::ZERO;
        viewer.yaw = yaw;
        viewer.pitch = pitch;
        viewer.distance = distance;
        viewer.orthographic = false;
    }
    warm(&mut app, 12);
    paint_ready(&mut app);
    let target = app.world().resource::<ViewerUi>().image.clone();
    let size = {
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let mut image = images.get_mut(&target).unwrap();
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        image.texture_descriptor.size
    };
    std::fs::write(directory.join("input.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "color": color, "roughness": f32::from(roughness) / 255.0, "metallic": 0.0,
        "yaw": yaw, "pitch": pitch, "distance": distance, "fov": FOV,
        "width": size.width, "height": size.height,
    })).unwrap()).unwrap();
    warm(&mut app, 4);
    let mut coverage = material_painter_graph::Graph::new("Coverage");
    let color = coverage.add("shade_color", [0., 0.]);
    coverage.node_mut(color).unwrap().params.insert("color".into(),
        material_painter_graph::Value::Color([0., 1., 0.]));
    let output = coverage.add("shade_output", [200., 0.]);
    coverage.connect(color, output, "color");
    let presets = shader_preview::presets();
    let coverage_id = presets.len();
    app.world_mut().resource_mut::<PaintState>().sessions.get_mut(&0).unwrap().shader_graphs
        .insert(coverage_id, shader_preview::Document::new(&coverage));
    paint_message(&mut app, PaintAction::PreviewShader(Some(coverage_id)));
    let mid = |image: &image::RgbaImage| image.get_pixel(image.width() / 2, image.height() / 2).0;
    capture_when(&mut app, &target, &directory.join("bevy-Coverage.png"), |image| {
        let color = mid(image);
        color[1] > color[0].saturating_add(80) && color[1] > color[2].saturating_add(80)
    });
    let unlit = presets.iter().position(|preset| preset.name == "Unlit").unwrap();
    for (index, name) in presets.iter().enumerate().map(|(i, g)| (i, g.name.as_str()))
        .chain([(unlit, "Unlit linear")])
    {
        if name == "Unlit linear" {
            let world = app.world_mut();
            let mut cameras = world.query_filtered::<&mut Tonemapping, With<ViewCamera>>();
            for mut tonemapping in cameras.iter_mut(world) {
                *tonemapping = Tonemapping::None;
            }
        }
        paint_message(&mut app, PaintAction::PreviewShader(Some(index)));
        warm(&mut app, 12);
        capture_when(&mut app, &target, &directory.join(format!("bevy-{name}.png")), |_| true);
    }
}

#[test]
#[ignore = "requires GPU; checks the native WGSL editor and its live preview"]
fn gpu_shader_maker_updates_texture_and_reports_source_errors() {
    use crate::editor::model_document::glb::Glb;
    use crate::editor::model_primitives::Primitive;
    use std::{path::Path, time::Instant};

    let project = crate::EditorProject::load(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let (mut app, window, target) = fixture_project(project);
    let directory = std::env::temp_dir().join(format!(
        "bevy-wgsl-library-{}", bevy::asset::uuid::Uuid::new_v4(),
    ));
    app.world_mut().resource_mut::<ShaderMaker>().node_directory = Some(directory.clone());
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        doc.objects[0].geometry = Primitive::Cube.geometry(&Primitive::Cube.defaults()).unwrap().into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::ZERO;
        viewer.distance = 3.0;
    }
    paint_message(&mut app, PaintAction::Panel(Panel::Shader));
    let wait = |app: &mut App, message: &str| {
        let start = Instant::now();
        loop {
            frame(app);
            let world = app.world_mut();
            let mut texts = world.query::<(&Text, &ComputedNode)>();
            if texts.iter(world).any(|(text, node)| text.0.contains(message) && node.size().y > 0.) {
                break;
            }
            assert!(start.elapsed().as_secs() < 30,
                "Не появилось сообщение {message}; {:?}",
                texts.iter(world).filter(|(text, _)| text.0.contains("WGSL")).map(|(text, _)| &text.0).collect::<Vec<_>>());
        }
    };
    let set_source = |app: &mut App, code: &str| {
        let world = app.world_mut();
        let mut fields = world.query::<(&AccessibleLabel, &mut bevy::text::EditableText)>();
        let (_, mut field) = fields.iter_mut(world)
            .find(|(label, _)| label.0 == "Исходный код WGSL").unwrap();
        assert!(field.allow_newlines);
        field.editor_mut().set_text(code);
    };
    let image = |app: &mut App| {
        let world = app.world_mut();
        let mut nodes = world.query::<(&AccessibleLabel, &ImageNode)>();
        nodes.iter(world).find(|(label, _)| label.0 == "Предпросмотр узла WGSL").unwrap().1.image.clone()
    };
    let click_label = |app: &mut App, label: &str| {
        let rect = labelled(app, label);
        move_pointer(app, window, rect.center());
        mouse(app, window, ButtonState::Pressed);
        mouse(app, window, ButtonState::Released);
        warm(app, 4);
    };
    let preview_number = |app: &mut App, label: &str, value: &str| {
        click_label(app, label);
        let world = app.world_mut();
        let focused = world.resource::<InputFocus>().get();
        let mut inputs = world.query::<(Entity, &AccessibleLabel, &mut bevy::text::EditableText)>();
        let (entity, _, mut input) = inputs.iter_mut(world).find(|(_, name, _)| name.0 == label).unwrap();
        assert_eq!(focused, Some(entity), "{label} must receive keyboard focus");
        input.editor_mut().set_text(value);
        fill_modifier(app, window, KeyCode::Enter, Key::Enter, ButtonState::Pressed);
        fill_modifier(app, window, KeyCode::Enter, Key::Enter, ButtonState::Released);
    };
    wait(&mut app, "Текстурный узел собран");
    assert!(labelled(&mut app, "Исходный код WGSL").width() > 200.0);
    {
        let world = app.world_mut();
        let mut code = world.query::<(&AccessibleLabel, &bevy::text::TextLayoutInfo, &bevy::text::ComputedTextBlock)>();
        let (_, glyphs, sections) = code.iter(world).find(|(label, _, _)| label.0 == "Исходный код WGSL").unwrap();
        assert!(glyphs.glyphs.iter().any(|glyph| glyph.section_index > 0 && sections.entities().get(glyph.section_index as usize).is_some()));
    }
    let first = image(&mut app);
    click_label(&mut app, "Плитка 2×2");
    {
        let world = app.world_mut();
        let mut nodes = world.query::<(&AccessibleLabel, &Children)>();
        let children = nodes.iter(world).find(|(label, _)| label.0 == "Предпросмотр узла WGSL").unwrap().1;
        assert_eq!(children.len(), 4);
        for child in children.iter() {
            assert_eq!(world.get::<Node>(child).unwrap().display, Display::Flex);
            assert_eq!(world.get::<ImageNode>(child).unwrap().image, first);
        }
    }
    click_label(&mut app, "Плитка 2×2");
    let test_source = "// @name Test values\n// @output color\n// @param gain: float(0, 1) = 0\nfn node(uv: vec2<f32>) -> vec4<f32> {\n return vec4<f32>(1.0 - params.gain, params.gain, 0.0, 1.0);\n}\n";
    set_source(&mut app, test_source);
    wait(&mut app, "Текстурный узел собран");
    let texture_pixel = |app: &mut App| {
        let handle = image(app);
        app.world().resource::<Assets<Image>>().get(&handle).unwrap()
            .data.as_ref().unwrap()[..4].to_vec()
    };
    assert_eq!(texture_pixel(&mut app), [255, 0, 0, 255]);
    preview_number(&mut app, "WGSL · gain", "1");
    wait(&mut app, "Текстурный узел собран");
    assert_eq!(texture_pixel(&mut app), [0, 255, 0, 255]);
    click_label(&mut app, "Сбросить значения");
    wait(&mut app, "Проверка WGSL…");
    wait(&mut app, "Текстурный узел собран");
    assert_eq!(texture_pixel(&mut app), [255, 0, 0, 255]);
    {
        let world = app.world_mut();
        let mut fields = world.query::<(&AccessibleLabel, &bevy::text::EditableText)>();
        let (_, code) = fields.iter(world).find(|(label, _)| label.0 == "Исходный код WGSL").unwrap();
        assert_eq!(code.value(), test_source);
    }
    set_source(&mut app, "// @name Test texture\n// @output color\nfn node(uv: vec2<f32>) -> vec4<f32> {\n return vec4<f32>(1.0, 0.0, 0.0, 1.0);\n}\n");
    wait(&mut app, "Текстурный узел собран");
    let red = image(&mut app);
    assert_ne!(first, red);
    let assets = app.world().resource::<Assets<Image>>();
    assert_eq!(&assets.get(&red).unwrap().data.as_ref().unwrap()[..4], &[255, 0, 0, 255]);
    click_label(&mut app, "Добавить в библиотеку");
    wait(&mut app, "Узел добавлен");
    let reopened = node_library::reopen(directory.clone());
    let texture_def = reopened.defs.iter().find(|def| def.id.starts_with("user/") && def.name == "Test texture").unwrap();
    let mut texture_graph = material_painter_graph::Graph::new("User texture");
    texture_graph.res = 64;
    let custom_texture = texture_graph.add(&texture_def.id, [0., 0.]);
    let output = texture_graph.add(material_painter_graph::OUTPUT_DEF, [240., 0.]);
    assert!(texture_graph.connect(custom_texture, output, "input"));
    let payload = texture_graph.to_json(&reopened);
    let reopened_graph = material_painter_graph::Graph::from_json(&payload, &reopened).unwrap();
    assert_eq!(reopened_graph, texture_graph);
    let device = app.world().resource::<bevy::render::renderer::RenderDevice>();
    let queue = app.world().resource::<bevy::render::renderer::RenderQueue>();
    let layer = material_graph::prepare_graph(
        reopened_graph, &Default::default(), 64,
        material_graph::Inputs { maps: None, surface: None }, device, queue,
    ).unwrap();
    assert_eq!(layer.texture_fill.as_ref().unwrap().images[0].as_ref().unwrap()
        .pixels().get_pixel(20, 20).0, [255, 0, 0, 255]);
    let mut object = app.world().resource::<editing::State>().documents[&0].objects[0].clone();
    object.source_node = None;
    let mut layer = layer;
    let fill = Arc::make_mut(layer.texture_fill.as_mut().unwrap());
    let graph = Arc::make_mut(fill.graph.as_mut().unwrap());
    let code = graph["bevy_wgsl_nodes"].as_object_mut().unwrap().remove(&texture_def.id).unwrap();
    let foreign_texture = "user/Foreign texture shader";
    graph["bevy_wgsl_nodes"][foreign_texture] = code;
    graph["nodes"][0]["def"] = foreign_texture.into();
    object.materials[0].paint = Some(Arc::new(Surface {
        size: 64,
        base: std::array::from_fn(|_| crate::editor::model_paint_data::solid(64, [255; 4])),
        stack: Stack { layers: vec![layer], selected: 0 },
        material: serde_json::Value::Null,
        baked: None,
    }));
    let path = directory.join("foreign.glb");
    std::fs::write(&path, Glb::new_asset(&[object]).unwrap()).unwrap();
    let glb = Glb::load(&path).unwrap();
    let paint = glb.paint_surface(&glb.json["meshes"][0]["primitives"][0]).unwrap().unwrap();
    let payload = paint.stack.layers[0].texture_fill.as_ref().unwrap().graph.as_ref().unwrap();
    let restored = material_graph::read(payload).unwrap();
    let def = &restored.node(custom_texture).unwrap().def;
    assert!(node_library::is_embedded(def));
    assert_eq!(material_graph::library().find(def).unwrap().source, texture_def.source);
    let source_field = {
        let world = app.world_mut();
        world.query::<(Entity, &AccessibleLabel)>().iter(world)
            .find(|(_, label)| label.0 == "Исходный код WGSL").unwrap().0
    };
    let ctrl = |app: &mut App, code: KeyCode, letter: &'static str| {
        app.world_mut().resource_mut::<InputFocus>()
            .set(source_field, bevy::input_focus::FocusCause::Auto);
        fill_modifier(app, window, KeyCode::ControlLeft, Key::Control, ButtonState::Pressed);
        fill_modifier(app, window, code, Key::Character(letter.into()), ButtonState::Pressed);
        fill_modifier(app, window, code, Key::Character(letter.into()), ButtonState::Released);
        fill_modifier(app, window, KeyCode::ControlLeft, Key::Control, ButtonState::Released);
    };
    let updated = texture_def.source.replace("1.0, 0.0, 0.0, 1.0", "0.0, 0.0, 1.0, 1.0");
    set_source(&mut app, &updated);
    ctrl(&mut app, KeyCode::KeyS, "s");
    wait(&mut app, "Правки узла сохранены");
    assert_eq!(material_graph::library().find(&texture_def.id).unwrap().source, updated);
    ctrl(&mut app, KeyCode::KeyA, "a");
    assert_eq!(
        app.world().get::<bevy::text::EditableText>(source_field).unwrap()
            .editor().raw_selection().text_range(),
        0..updated.len(),
    );
    click_label(&mut app, "Вставить фрагмент");
    click_label(&mut app, "@param float");
    let source = |app: &App| {
        app.world().get::<bevy::text::EditableText>(source_field).unwrap().value().to_string()
    };
    assert_eq!(source(&app).trim(), "// @param amount: float(0, 1) = 0.5");
    click_label(&mut app, "Вернуть исходник");
    assert_eq!(source(&app), updated);
    paint_message(&mut app, PaintAction::Panel(Panel::Layers));
    paint_message(&mut app, PaintAction::AddGraphMaterial(0));
    let started = Instant::now();
    loop {
        frame(&mut app);
        let session = &app.world().resource::<PaintState>().sessions[&0];
        if !session.smart_loading {
            assert!(session.graph().is_some(), "{}", app.world().resource::<Editor>().status);
            break;
        }
        assert!(started.elapsed().as_secs() < 45, "Не загрузился граф материала");
    }
    paint_message(&mut app, PaintAction::GraphCanvas);
    assert!(app.world().resource::<PaintState>().sessions[&0].graph_canvas_open());
    paint_message(&mut app, PaintAction::Panel(Panel::Shader));
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].panel, Panel::Shader);
    click_label(&mut app, "Добавить в граф");
    {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        assert_eq!(session.panel, Panel::Layers);
        assert!(session.graph_canvas_open());
        let graph = material_graph::read(session.graph().unwrap()).unwrap();
        let inserted = graph.node(session.graph_node.unwrap()).unwrap();
        assert_eq!(inserted.def, texture_def.id);
        assert!(inserted.params.is_empty());
    }
    paint_message(&mut app, PaintAction::Undo);
    {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        let graph = material_graph::read(session.graph().unwrap()).unwrap();
        assert!(graph.nodes.iter().all(|node| node.def != texture_def.id));
    }
    paint_message(&mut app, PaintAction::Panel(Panel::Shader));
    let node_path = node_library::source_path(&directory, &texture_def.id).unwrap();
    let external = updated.replace("0.0, 0.0, 1.0, 1.0", "0.0, 1.0, 0.0, 1.0");
    let wait_catalog = |app: &mut App, expected: &str| {
        let started = Instant::now();
        while material_graph::library().find(&texture_def.id).unwrap().source != expected {
            frame(app);
            assert!(started.elapsed().as_secs() < 30, "Библиотека не подхватила исходник");
        }
    };
    std::fs::write(&node_path, &external).unwrap();
    wait(&mut app, "WGSL обновлён с диска");
    assert_eq!(source(&app), external);
    wait_catalog(&mut app, &external);
    assert_eq!(texture_pixel(&mut app), [0, 255, 0, 255]);

    let local = updated.replace("0.0, 0.0, 1.0, 1.0", "1.0, 0.0, 1.0, 1.0");
    set_source(&mut app, &local);
    std::fs::write(&node_path, &updated).unwrap();
    wait(&mut app, "локальные правки остались в редакторе");
    assert_eq!(source(&app), local);
    ctrl(&mut app, KeyCode::KeyS, "s");
    wait(&mut app, "Не удалось изменить узел");
    assert_eq!(std::fs::read_to_string(&node_path).unwrap(), updated);
    assert_eq!(source(&app), local);
    click_label(&mut app, "Вернуть исходник");
    wait(&mut app, "WGSL обновлён с диска");
    assert_eq!(source(&app), updated);
    wait_catalog(&mut app, &updated);
    click_label(&mut app, "Убрать из палитры…");
    wait(&mut app, "Убрать узел из палитры?");
    assert!(node_library::available(&texture_def.id));
    click_label(&mut app, "Убрать узел");
    wait(&mut app, "Узел убран из палитры");
    assert!(!node_library::available(&texture_def.id));
    let last_preview = image(&mut app);
    set_source(&mut app, "// @name Invalid\n// @output color\nfn node(uv: vec2<f32>) -> vec4<f32> {\n return missing_value;\n}\n");
    wait(&mut app, "missing_value");
    assert_eq!(image(&mut app), last_preview);
    set_source(&mut app, "// @name UV coverage\n// @surface\n// @output gray\nfn node(uv: vec2<f32>) -> vec4<f32> {\n return vec4<f32>(select(0.0, 1.0, covered(uv)));\n}\n");
    wait(&mut app, "Текстурный узел собран");
    let surface = image(&mut app);
    assert!(app.world().resource::<Assets<Image>>().get(&surface).unwrap().data.as_ref().unwrap()
        .chunks_exact(4).any(|pixel| pixel == [255, 255, 255, 255]));

    let shade = |expression| format!(
        "// @name Test shading\n// @shading\n// @output color\nfn node(c: Shade) -> vec4<f32> {{\n return {expression};\n}}\n",
    );
    set_source(&mut app, &shade("vec4<f32>(1.0, 0.0, 0.0, 1.0)"));
    wait(&mut app, "Узел шейдинга прошёл проверку WGSL");
    warm(&mut app, 8);
    let original_preview = labelled(&mut app, "3D-вид модели");
    let code = labelled(&mut app, "Исходный код WGSL");
    assert!(original_preview.min.x > code.min.x);
    let preview_tab = labelled(&mut app, "Предпросмотр WGSL").center();
    move_pointer(&mut app, window, preview_tab);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, Vec2::new(code.min.x + 5., code.center().y));
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 8);
    let point = labelled(&mut app, "3D-вид модели").center().round().as_uvec2();
    assert!((point.x as f32) < labelled(&mut app, "Исходный код WGSL").min.x);
    let screenshot = std::env::temp_dir().join(format!("bevy-wgsl-model-{}.png", bevy::asset::uuid::Uuid::new_v4()));
    let model_pixel = |app: &mut App| {
        let point = labelled(app, "3D-вид модели").center().round().as_uvec2();
        move |image: &image::RgbaImage| image.get_pixel(point.x, point.y).0
    };
    let pixel = model_pixel(&mut app);
    let is_red = |p: [u8; 4]| p[0] > p[1].saturating_add(80) && p[0] > p[2].saturating_add(80);
    let is_green = |p: [u8; 4]| p[1] > p[0].saturating_add(80) && p[1] > p[2].saturating_add(80);
    capture_when(&mut app, &target, &screenshot, |image| is_red(pixel(image)));
    let parameterized = shade("vec4<f32>(1.0 - params.blend, params.blend, 0.0, 1.0)")
        .replace("// @output color", "// @output color\n// @param blend: float(0, 1) = 0");
    set_source(&mut app, &parameterized);
    wait(&mut app, "Узел шейдинга прошёл проверку WGSL");
    warm(&mut app, 4);
    preview_number(&mut app, "WGSL · blend", "1");
    wait(&mut app, "Узел шейдинга прошёл проверку WGSL");
    let changed_pixel = model_pixel(&mut app);
    capture_when(&mut app, &target, &screenshot, |image| is_green(changed_pixel(image)));
    set_source(&mut app, &shade("vec4<f32>(0.0, 1.0, 0.0, 1.0)"));
    wait(&mut app, "Проверка WGSL…");
    wait(&mut app, "Узел шейдинга прошёл проверку WGSL");
    capture_when(&mut app, &target, &screenshot, |image| is_green(pixel(image)));
    click_label(&mut app, "Добавить в библиотеку");
    wait(&mut app, "Узел добавлен");
    let reopened = node_library::reopen(directory.clone());
    let shading_def = reopened.defs.iter().find(|def| def.id.starts_with("user/") && def.name == "Test shading").unwrap();
    assert!(shading_def.shading);
    set_source(&mut app, &shade("missing_shading_value"));
    wait(&mut app, "missing_shading_value");
    {
        let world = app.world_mut();
        let mut markers = world.query::<(&bevy::text::TextSpan, &bevy::text::TextBackgroundColor)>();
        assert!(markers.iter(world).any(|(line, _)| line.0 == "5\n"));
    }
    capture_when(&mut app, &target, &screenshot, |image| is_green(pixel(image)));
    paint_message(&mut app, PaintAction::Panel(Panel::Preview));
    let pixel = model_pixel(&mut app);
    capture_when(&mut app, &target, &screenshot, |image| !is_green(pixel(image)));
    let unlit = shader_preview::presets().iter().position(|graph| graph.name == "Unlit").unwrap();
    paint_message(&mut app, PaintAction::PreviewShader(Some(unlit)));
    paint_message(&mut app, PaintAction::GraphCanvas);
    paint_message(&mut app, PaintAction::Panel(Panel::Shader));
    click_label(&mut app, "Добавить в граф");
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].panel, Panel::Shader);
    click_label(&mut app, "Вернуть исходник");
    click_label(&mut app, "Добавить в граф");
    let (node, output) = {
        let state = app.world().resource::<PaintState>();
        let graph = material_graph::read(state.sessions[&0].graph().unwrap()).unwrap();
        let inserted = graph.nodes.iter().find(|node| node.def == shading_def.id).unwrap();
        assert_eq!(state.sessions[&0].graph_node, Some(inserted.id));
        assert!(inserted.pos[0] > graph.nodes.iter().filter(|node| node.id != inserted.id)
            .map(|node| node.pos[0]).fold(0., f32::max));
        (inserted.id, graph.nodes.iter().find(|node| node.def == "shade_output").unwrap().id)
    };
    paint_message(&mut app, PaintAction::GraphEdit(material_graph::Edit::Connect(node, output, 0)));
    let pixel = model_pixel(&mut app);
    capture_when(&mut app, &target, &screenshot, |image| is_green(pixel(image)));
    // Simulate a foreign copy whose user node is absent from this installation.
    let foreign = {
        let paint = app.world().resource::<PaintState>();
        let mut payload = paint.sessions[&0].graph().unwrap().as_ref().clone();
        let source = payload["bevy_wgsl_nodes"].as_object_mut().unwrap().remove(&shading_def.id).unwrap();
        let foreign_id = "user/Foreign preview shader";
        assert!(material_graph::library().find(foreign_id).is_none());
        payload["bevy_wgsl_nodes"][foreign_id] = source;
        let node = payload["nodes"].as_array_mut().unwrap().iter_mut().find(|entry| entry["id"].as_u64() == Some(node)).unwrap();
        node["def"] = foreign_id.into();
        payload
    };
    let file = directory.join("foreign.mpgraph");
    std::fs::write(&file, serde_json::to_vec(&foreign).unwrap()).unwrap();
    let imported = material_graph::file::load_shader(&file).unwrap();
    let alias = &imported.node(node).unwrap().def;
    assert!(node_library::is_embedded(alias));
    assert_eq!(material_graph::library().find(alias).unwrap().source, shading_def.source);
    let saved = material_graph::file::export(&directory, &foreign, &Default::default()).unwrap();
    assert_eq!(material_graph::file::load_shader(&saved).unwrap(), imported);
    let package = material_graph::file::export_package(&directory, &foreign, &Default::default()).unwrap();
    assert_eq!(material_graph::file::load_shader(&package).unwrap(), imported);
    paint_message(&mut app, PaintAction::GraphCanvas);
    paint_message(&mut app, PaintAction::PreviewShader(None));
    let pixel = model_pixel(&mut app);
    capture_when(&mut app, &target, &screenshot, |image| !is_green(pixel(image)));
    app.world_mut().resource_mut::<PaintState>().shader_graph_test_path = Some(Some(package));
    scroll_to_paint_control(&mut app, PaintAction::ImportShaderGraph);
    paint_click(&mut app, window, PaintAction::ImportShaderGraph);
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    while !app.world().resource::<PaintState>().sessions[&0].graph_canvas_open() {
        frame(&mut app);
        assert!(Instant::now() < deadline, "Imported shader graph did not open");
    }
    warm(&mut app, 4);
    let pixel = model_pixel(&mut app);
    capture_when(&mut app, &target, &screenshot, |image| is_green(pixel(image)));
    std::fs::remove_file(screenshot).unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires GPU; applies archived Bitmap and Facing Up graphs through the material shelf"]
fn gpu_mpsmart_graphs_apply_to_target_geometry() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    use std::{fs, path::Path, time::Instant};

    let directory = std::env::temp_dir().join(format!(
        "bevy-mpsmart-apply-{}",
        bevy::asset::uuid::Uuid::new_v4(),
    ));
    fs::create_dir_all(&directory).unwrap();
    let mut archive = directory.join("graphs.mpsmart");
    if let Some(path) = std::env::var_os("BEVY_NATIVE_MPSMART_GRAPH_FILE") {
        fs::copy(path, &archive).unwrap();
    } else {
        presets::mpsmart_graph_fixture(&archive);
        let layers = presets::load_material(&archive, 64).unwrap();
        archive = presets::export_material_package(
            &directory,
            presets::Capture::Material { name: "Packaged MP Graphs".into(), layers },
        ).unwrap().path;
    }
    let shelf = directory.join("shelf");
    let (entry, skipped) = presets::import_material(&shelf, &archive).unwrap();
    assert_eq!(skipped, 0);
    fs::remove_file(archive).unwrap();
    let project = crate::EditorProject::load(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (mut app, _, _) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        let mut geometry = Geometry::default();
        geometry.positions = vec![Vec3::ZERO, Vec3::Z, Vec3::X + Vec3::Z, Vec3::X];
        let corners = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]
            .into_iter()
            .enumerate()
            .map(|(i, uv)| {
                let mut corner = Corner::new(i);
                corner.uv = uv;
                corner
            })
            .collect();
        geometry.faces.push(Face {
            corners,
            material: 0,
            smooth: false,
        });
        doc.objects[0].geometry = geometry.into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 12);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::Resolution(256));
    app.world_mut()
        .resource_mut::<PaintState>()
        .library
        .entries
        .push(entry);
    paint_message(&mut app, PaintAction::ApplyShelfMaterial(0));

    let wait_for_mask = |app: &mut App, expected| {
        let start = Instant::now();
        loop {
            frame(app);
            let session = &app.world().resource::<PaintState>().sessions[&0];
            let layer = &session.stack.layers[session.stack.selected];
            let output = layer
                .mask
                .as_ref()
                .and_then(|m| m.graph.as_ref())
                .and_then(|g| g.output.as_ref());
            if !session.smart_loading
                && !session.busy
                && output.is_some_and(|image| image.pixels().get_pixel(32, 32).0[0] == expected)
            {
                return layer.clone();
            }
            assert!(
                start.elapsed().as_secs() < 60,
                "{}",
                app.world().resource::<Editor>().status
            );
        }
    };
    let upward = wait_for_mask(&mut app, 255);
    let fill = upward.texture_fill.as_ref().unwrap();
    assert_eq!(
        fill.images[0]
            .as_ref()
            .unwrap()
            .pixels()
            .get_pixel(32, 32)
            .0,
        [0, 255, 0, 255]
    );
    assert!(
        fill.images[1].is_none(),
        "the archive disables graph roughness"
    );
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        doc.objects[0].transform.rotation = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        doc.revision += 1;
    }
    let sideways = wait_for_mask(&mut app, 0);
    let exported = presets::export_material(
        &directory,
        presets::Capture::Material {
            name: "Re-exported MP Graphs".into(),
            layers: vec![sideways.clone()],
        },
    )
    .unwrap();
    let reloaded = presets::load_material(&exported.path, 256).unwrap();
    let original = sideways.texture_fill.as_ref().unwrap();
    let reread = reloaded[0].texture_fill.as_ref().unwrap();
    assert_eq!(reread.graph, original.graph);
    assert_eq!(reread.graph_images, original.graph_images);
    assert_eq!(
        reread.settings.graph_channels,
        original.settings.graph_channels
    );
    assert_eq!(
        reloaded[0].mask.as_ref().unwrap().graph,
        sideways.mask.as_ref().unwrap().graph
    );
    if std::env::var_os("BEVY_NATIVE_MPSMART_GRAPH_FILE").is_none() {
        let child = app.world().resource::<PaintState>().sessions[&0].stack.selected;
        paint_message(&mut app, PaintAction::Add(LayerKind::Folder));
        let folder = app.world().resource::<PaintState>().sessions[&0].stack.selected;
        paint_message(&mut app, PaintAction::Select(child));
        paint_message(&mut app, PaintAction::MoveInto(folder));
        paint_message(&mut app, PaintAction::Select(folder));
        paint_message(&mut app, PaintAction::GraphCanvas);
        let source = {
            let session = &app.world().resource::<PaintState>().sessions[&0];
            let view = session.active_folder_view().unwrap();
            let fill = session.stack.layers[child].graph_id.unwrap();
            let source = view.graph.tagged(fill, material_painter_graph::Role::Source(0)).unwrap();
            let channel = view.graph.tagged(fill, material_painter_graph::Role::Channel(0)).unwrap();
            assert_eq!(view.graph.node(channel).unwrap().links["mask"], source);
            assert_eq!(view.graph.node(source).unwrap().def, "smart_mask");
            assert_eq!(session.stack.layers[child].mask, sideways.mask);
            source
        };
        let invert = material_graph::library().find("smart_mask").unwrap().params.iter()
            .position(|param| param.name == "invert").unwrap();
        paint_message(&mut app, PaintAction::GraphAdjust(source, invert, 0, 1));
        let start = Instant::now();
        loop {
            frame(&mut app);
            let session = &app.world().resource::<PaintState>().sessions[&0];
            let mask = session.stack.layers[child].mask.as_ref().unwrap().graph.as_ref().unwrap();
            if !session.smart_loading && mask.output.as_ref().unwrap().pixels().get_pixel(32, 32).0[0] == 255 {
                assert_eq!(mask.payload["res"], sideways.mask.as_ref().unwrap().graph.as_ref().unwrap().payload["res"]);
                break;
            }
            assert!(start.elapsed().as_secs() < 45, "{}", app.world().resource::<Editor>().status);
        }
        paint_message(&mut app, PaintAction::Select(child));
        paint_message(&mut app, PaintAction::EditMask);
        assert!(app.world().resource::<PaintState>().sessions[&0].editing_mask);
        paint_message(&mut app, PaintAction::GraphAdjust(source, invert, 0, -1));
        wait_for_mask(&mut app, 0);
        paint_message(&mut app, PaintAction::Select(folder));
        let session = &app.world().resource::<PaintState>().sessions[&0];
        let graph = &session.active_folder_view().unwrap().graph;
        let updated = graph.tagged(session.stack.layers[child].graph_id.unwrap(),
            material_painter_graph::Role::Source(0)).unwrap();
        assert_eq!(graph.node(updated).unwrap().params["invert"], material_painter_graph::Value::Bool(false));
        assert_eq!(graph.node(updated).unwrap().pos, [0., -450.]);
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires GPU; paints through the real model viewport, composites layers, saves and reopens GLB"]
fn gpu_model_paint_strokes_layers_save_and_reopen() {
    use crate::editor::{model_document::glb::Glb, model_primitives::Primitive};
    let directory = std::env::temp_dir().join(format!(
        "bevy-model-paint-{}",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    let mut project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    copy_tree(&project.asset_root, &directory);
    project.asset_root = directory.clone();
    let path = directory.join("models/block.glb");
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    let scene = app.world().resource::<Editor>().edits.clone();
    {
        let mut state = app.world_mut().resource_mut::<editing::State>();
        let doc = state.documents.get_mut(&0).unwrap();
        let mut geometry = Primitive::Cube
            .geometry(&Primitive::Cube.defaults())
            .unwrap();
        // Give each triangle its own UV tile so the test also checks isolation.
        for (i, face) in geometry.faces.iter_mut().enumerate() {
            for c in &mut face.corners {
                c.uv = (c.uv * 0.8 + Vec2::splat(0.1) + Vec2::new((i % 4) as f32, (i / 4) as f32))
                    / Vec2::new(4., 3.);
            }
        }
        doc.objects[0].geometry = geometry.into();
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 8);
    paint_ready(&mut app);
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.size = 110.;
        session.hardness = 80.;
        session.color = 3;
    }
    let rect = labelled(&mut app, "3D-вид модели");
    let world = Vec3::new(0., 0., 0.5);
    let center = editing::input::project(app.world().resource::<Viewer>(), rect, world).unwrap();
    move_pointer(&mut app, window, center - Vec2::new(24., 0.));
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, center + Vec2::new(24., 0.));
    let wait_worker = |app: &mut App| {
        let started = std::time::Instant::now();
        loop {
            frame(app);
            if app.world().resource::<engine::Runtime>().worker_idle(0) {
                break;
            }
            assert!(
                started.elapsed().as_secs() < 20,
                "paint worker did not settle"
            );
        }
    };
    wait_worker(&mut app);
    warm(&mut app, 3);
    let ids = app.world().resource::<engine::Runtime>().texture_ids(0);
    let revision = app.world().resource::<PaintState>().sessions[&0].revision;
    let mut events = bevy::ecs::message::MessageCursor::<bevy::asset::AssetEvent<Image>>::default();
    events.clear(
        app.world()
            .resource::<Messages<bevy::asset::AssetEvent<Image>>>(),
    );
    move_pointer(&mut app, window, center + Vec2::new(38., 0.));
    wait_worker(&mut app);
    warm(&mut app, 3);
    assert_eq!(
        app.world().resource::<PaintState>().sessions[&0].revision,
        revision,
        "pointer samples must not rebuild layer panels"
    );
    assert_eq!(
        app.world().resource::<engine::Runtime>().texture_ids(0),
        ids
    );
    assert!(
        !events
            .read(
                app.world()
                    .resource::<Messages<bevy::asset::AssetEvent<Image>>>()
            )
            .any(|event| matches!(event,bevy::asset::AssetEvent::Modified{id}if ids.contains(id))),
        "strokes must patch existing GPU textures without full image reuploads"
    );
    warm(&mut app, 12);
    assert!(
        app.world().resource::<engine::Runtime>().worker_idle(0),
        "holding a stationary brush must not enqueue more stamps"
    );
    let (batches, stamps, readbacks) = app.world().resource::<engine::gpu::Queue>().statistics();
    assert!(
        batches > 0 && stamps > 0,
        "live strokes must execute on the GPU"
    );
    assert_eq!(
        readbacks, 0,
        "painting a held stroke must not read pixel data back to the CPU"
    );
    mouse(&mut app, window, ButtonState::Released);
    // Saving immediately after release must include queued worker stamps.
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Edit(
            editing::EditAction::Save,
        )));
    let saving = std::time::Instant::now();
    loop {
        frame(&mut app);
        if !app.world().resource::<editing::State>().documents[&0].dirty() {
            break;
        }
        assert!(
            saving.elapsed().as_secs() < 90,
            "queued stroke save failed: {}",
            app.world().resource::<Editor>().status
        );
    }
    paint_ready(&mut app);
    let painted = app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
        .paint
        .clone()
        .expect("3D stroke must produce editable paint pixels");
    assert!(
        painted.stack.layers[0].channels[0]
            .as_ref()
            .unwrap()
            .chunks_exact(4)
            .any(|p| p[3] > 0)
    );
    assert_eq!(app.world().resource::<Editor>().edits, scene);
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/model-paint-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(
        &mut app,
        &target,
        &output.join("05-real-3d-paint.png"),
        |_| true,
    );
    paint_click(&mut app, window, PaintAction::Add(LayerKind::Folder));
    app.world_mut()
        .resource_mut::<PaintState>()
        .sessions
        .get_mut(&0)
        .unwrap()
        .set_value(Parameter::LayerOpacity, 50.);
    paint_ready(&mut app);
    paint_click(&mut app, window, PaintAction::Add(LayerKind::Paint));
    paint_click(&mut app, window, PaintAction::BlendMenu);
    paint_click(&mut app, window, PaintAction::Blend(Blend::Multiply));
    // Paint a second independent layer directly in the 2D texture view.
    let uv = {
        let w = app.world_mut();
        let (n, t) = w
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<engine::TextureCanvas>>()
            .single(w)
            .unwrap();
        Rect::from_center_size(
            t.affine().translation * n.inverse_scale_factor(),
            n.size() * n.inverse_scale_factor(),
        )
    };
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        paint.sessions.get_mut(&0).unwrap().color = 4;
    }
    let at = {
        let doc = &app.world().resource::<editing::State>().documents[&0];
        let face = &doc.objects[0].geometry.faces[0];
        let center = face.corners.iter().map(|c| c.uv).sum::<Vec2>() / face.corners.len() as f32;
        uv.min + uv.size() * center
    };
    move_pointer(&mut app, window, at);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    let two = app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
        .paint
        .clone()
        .unwrap();
    assert!(
        two.stack.layers[1].channels[0].is_some(),
        "2D view must also paint"
    );
    assert_eq!(two.stack.layers[1].blend, Blend::Multiply);
    assert_eq!(two.stack.layers[1].depth, 1);
    assert_eq!(two.stack.layers[2].opacity, 50.);
    paint_click(&mut app, window, PaintAction::Visible(1));
    assert!(
        !app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
            .paint
            .as_ref()
            .unwrap()
            .stack
            .layers[1]
            .visible
    );
    paint_click(&mut app, window, PaintAction::Undo);
    assert!(
        app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
            .paint
            .as_ref()
            .unwrap()
            .stack
            .layers[1]
            .visible
    );
    paint_click(&mut app, window, PaintAction::Select(2));
    paint_click(&mut app, window, PaintAction::AddMask(false));
    let surface = |app: &App| {
        app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
            .paint
            .clone()
            .unwrap()
    };
    let hidden = surface(&app);
    assert!(
        hidden.stack.layers[2]
            .mask
            .as_ref()
            .unwrap()
            .pixels
            .chunks_exact(4)
            .all(|p| p[0] == 0)
    );
    move_pointer(&mut app, window, at);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    let masked = surface(&app);
    let mask = masked.stack.layers[2].mask.as_ref().unwrap();
    assert!(mask.pixels.chunks_exact(4).any(|p| p[0] == 255));
    assert_eq!(
        masked.stack.layers[1].channels,
        two.stack.layers[1].channels
    );
    assert_ne!(masked.composite(0), hidden.composite(0));
    paint_click(&mut app, window, PaintAction::Undo);
    assert_eq!(surface(&app).stack, hidden.stack);
    paint_click(&mut app, window, PaintAction::Redo);
    assert_eq!(surface(&app).stack, masked.stack);
    capture_when(
        &mut app,
        &target,
        &output.join("06-layered-paint.png"),
        |_| true,
    );
    paint_click(&mut app, window, PaintAction::PreviewMask);
    let send_paint = |app: &mut App, action| {
        app.world_mut()
            .write_message(Action::ModelView(ModelViewAction::Paint(action)));
        warm(app, 4);
        let started = std::time::Instant::now();
        while app.world().resource::<editing::State>().documents[&0].paint_busy {
            frame(app);
            assert!(started.elapsed().as_secs() < 90, "Paint worker did not finish {action:?}");
        }
    };
    send_paint(&mut app, PaintAction::AddFilter(FilterKind::Invert));
    let filtered = surface(&app);
    let filtered_mask = filtered.stack.layers[2].mask.as_ref().unwrap();
    assert_eq!(filtered_mask.pixels, mask.pixels);
    assert!(
        filtered_mask
            .effective()
            .chunks_exact(4)
            .zip(mask.pixels.chunks_exact(4))
            .all(|(shown, raw)| shown[0] == 255 - raw[0])
    );
    send_paint(&mut app, PaintAction::AddMaskLayer);
    send_paint(&mut app, PaintAction::MaskLayer(0, MaskLayerEdit::Blend));
    send_paint(
        &mut app,
        PaintAction::MaskLayer(0, MaskLayerEdit::Value(-1)),
    );
    for kind in [
        FilterKind::Blur,
        FilterKind::Sharpen,
        FilterKind::Warp,
        FilterKind::Noise,
    ] {
        send_paint(&mut app, PaintAction::AddFilter(kind));
    }
    let filtered = surface(&app);
    let filtered_mask = filtered.stack.layers[2].mask.as_ref().unwrap();
    assert_eq!(filtered_mask.layers[0].value, 95);
    let at_pixel = |image: &image::RgbaImage| {
        let p = at.as_uvec2();
        image.get_pixel(p.x, p.y)[0]
    };
    capture_when(
        &mut app,
        &target,
        &output.join("07-filtered-mask.png"),
        |image| at_pixel(image) < 45,
    );
    send_paint(&mut app, PaintAction::MaskValue(0));
    let readbacks = app.world().resource::<engine::gpu::Queue>().statistics().2;
    move_pointer(&mut app, window, at);
    mouse(&mut app, window, ButtonState::Pressed);
    capture_when(
        &mut app,
        &target,
        &output.join("08-live-filtered-mask.png"),
        |image| at_pixel(image) > 90,
    );
    assert_eq!(
        app.world().resource::<engine::gpu::Queue>().statistics().2,
        readbacks
    );
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    let two = surface(&app);
    let changed = two.stack.layers[2].mask.as_ref().unwrap();
    assert_ne!(changed.pixels, filtered_mask.pixels);
    assert_eq!(changed.filters, filtered_mask.filters);
    assert_eq!(changed.layers, filtered_mask.layers);
    assert_eq!(
        two.stack.layers[1].channels,
        filtered.stack.layers[1].channels
    );
    paint_click(&mut app, window, PaintAction::Undo);
    assert_eq!(surface(&app).stack, filtered.stack);
    paint_click(&mut app, window, PaintAction::Redo);
    assert_eq!(surface(&app).stack, two.stack);
    send_paint(
        &mut app,
        PaintAction::Generator(None, GeneratorEdit::Set(GeneratorKind::Height)),
    );
    send_paint(
        &mut app,
        PaintAction::AddGeneratorLayer(GeneratorKind::Clouds),
    );
    send_paint(&mut app, PaintAction::MaskLayer(1, MaskLayerEdit::Blend));
    send_paint(&mut app, PaintAction::MaskValue(255));
    move_pointer(&mut app, window, at);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    let generated = surface(&app);
    let generated_mask = generated.stack.layers[2].mask.as_ref().unwrap();
    assert!(generated_mask.affine && generated_mask.context.is_some());
    assert_eq!(
        generated_mask.generator.unwrap().kind,
        GeneratorKind::Height
    );
    assert_eq!(
        generated_mask.layers[1].generator.unwrap().kind,
        GeneratorKind::Clouds
    );
    assert!(
        generated_mask
            .pixels
            .chunks_exact(4)
            .any(|p| p[0] == 0 && p[1] == 255)
    );
    send_paint(&mut app, PaintAction::Tool(PaintTool::Eraser));
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    let two = surface(&app);
    let erased = two.stack.layers[2].mask.as_ref().unwrap();
    assert!(
        erased
            .pixels
            .chunks_exact(4)
            .zip(generated_mask.pixels.chunks_exact(4))
            .any(|(after, before)| before[1] == 255 && after[0] == 255 && after[1] == 0)
    );
    capture_when(
        &mut app,
        &target,
        &output.join("09-generator-mask.png"),
        |_| true,
    );
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Edit(
            editing::EditAction::Save,
        )));
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        if !app.world().resource::<editing::State>().documents[&0].dirty() {
            break;
        }
        assert!(
            started.elapsed().as_secs() < 90,
            "save failed: {}",
            app.world().resource::<Editor>().status
        );
    }
    let saved = Glb::load(&path).unwrap();
    let node = saved.json["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["mesh"].is_number())
        .unwrap();
    let mesh = node["mesh"].as_u64().unwrap() as usize;
    let restored = saved
        .paint_surface(&saved.json["meshes"][mesh]["primitives"][0])
        .unwrap()
        .unwrap();
    assert_eq!(restored.stack, two.stack);
    app.world_mut().write_message(Action::CloseModelTab(0));
    warm(&mut app, 8);
    app.world_mut().write_message(Action::ViewModel(0));
    let started = std::time::Instant::now();
    while !app
        .world()
        .resource::<editing::State>()
        .documents
        .contains_key(&0)
    {
        frame(&mut app);
        assert!(started.elapsed().as_secs() < 40, "reopen failed");
    }
    assert_eq!(
        app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
            .paint
            .as_ref()
            .unwrap()
            .stack,
        two.stack
    );
    assert_eq!(app.world().resource::<Editor>().edits, scene);
    drop(app);
    assert_eq!(directory.parent(), Some(std::env::temp_dir().as_path()));
    std::fs::remove_dir_all(directory).unwrap();
}
fn labelled(app: &mut App, label: &str) -> Rect {
    let world = app.world_mut();
    let (_, node, transform) = world
        .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
        .iter(world)
        .find(|(value, _, _)| value.0 == label)
        .unwrap();
    Rect::from_center_size(
        transform.affine().translation * node.inverse_scale_factor(),
        node.size() * node.inverse_scale_factor(),
    )
}

#[test]
#[ignore = "requires GPU; projects textured fill layers through the material UI and compositor"]
fn gpu_model_paint_texture_fill_reprojects_and_undoes() {
    use crate::editor::model_primitives::Primitive;
    use image::ImageEncoder;

    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let (mut app, window, _) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) { break; }
    }
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        let mut geometry = Primitive::Cube.geometry(&Primitive::Cube.defaults()).unwrap();
        for (i, face) in geometry.faces.iter_mut().enumerate() {
            for corner in &mut face.corners {
                corner.uv = (corner.uv * 0.8 + Vec2::splat(0.1) + Vec2::new((i % 4) as f32, (i / 4) as f32))
                    / Vec2::new(4., 3.);
            }
        }
        doc.objects[0].geometry = geometry.into();
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 8);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::Add(LayerKind::Fill));
    paint_click(&mut app, window, PaintAction::Panel(Panel::Layers));
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(
        &[255, 0, 20, 255, 0, 80, 200, 255, 12, 5, 27, 255, 190, 133, 20, 255],
        2, 2, image::ExtendedColorType::Rgba8,
    ).unwrap();
    let bitmap = Arc::new(texture_fill::Bitmap::decode("pattern.png".into(), &png).unwrap());
    app.world_mut().resource_mut::<PaintState>().sessions.get_mut(&0).unwrap()
        .edit_texture_fill(|fill| fill.images[0] = Some(bitmap));
    warm(&mut app, 4);
    paint_ready(&mut app);
    let pixels = |app: &App| {
        app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
            .paint.as_ref().unwrap().composite(0)
    };
    let uv = pixels(&app);
    let controls = app.world_mut().query::<&Control>().iter(app.world()).filter(|control| {
        matches!(control.action, Action::ModelView(ModelViewAction::Paint(PaintAction::FillTextureProjection(_))))
    }).count();
    assert_eq!(controls, texture_fill::Projection::ALL.len());
    for projection in texture_fill::Projection::ALL.into_iter().skip(1) {
        paint_message(&mut app, PaintAction::FillTextureProjection(projection));
        let current = app.world().resource::<PaintState>().sessions[&0].active_texture_fill().unwrap();
        assert_eq!(current.settings.projection, projection);
        assert!(uv != pixels(&app), "{projection:?} did not change the texture");
    }
    paint_message(&mut app, PaintAction::FillTextureProjection(texture_fill::Projection::Uv));
    assert!(uv == pixels(&app));
    paint_message(&mut app, PaintAction::Undo);
    assert!(uv != pixels(&app));
    let field = {
        let world = app.world_mut();
        let mut inputs = world.query::<(Entity, &AccessibleLabel, &mut bevy::text::EditableText)>();
        let (entity, _, mut text) = inputs.iter_mut(world)
            .find(|(_, label, _)| label.0 == "Рисование: Повтор").unwrap();
        text.editor_mut().set_text("2.25");
        entity
    };
    app.world_mut().resource_mut::<bevy::input_focus::InputFocus>()
        .set(field, bevy::input_focus::FocusCause::Auto);
    warm(&mut app, 3);
    app.world_mut().resource_mut::<bevy::input_focus::InputFocus>().clear();
    warm(&mut app, 4);
    paint_ready(&mut app);
    let fill = app.world().resource::<PaintState>().sessions[&0].active_texture_fill().unwrap();
    assert_eq!(fill.settings.tiling, 2.25);
    let before_variation = pixels(&app);
    paint_message(&mut app, PaintAction::FillVariationOptions);
    let field = {
        let world = app.world_mut();
        let mut inputs = world.query::<(Entity, &AccessibleLabel, &mut bevy::text::EditableText)>();
        let (entity, _, mut text) = inputs.iter_mut(world)
            .find(|(_, label, _)| label.0 == "Рисование: Разброс цвета, %").unwrap();
        text.editor_mut().set_text("85");
        entity
    };
    app.world_mut().resource_mut::<bevy::input_focus::InputFocus>()
        .set(field, bevy::input_focus::FocusCause::Auto);
    warm(&mut app, 3);
    app.world_mut().resource_mut::<bevy::input_focus::InputFocus>().clear();
    warm(&mut app, 4);
    paint_ready(&mut app);
    let fill = app.world().resource::<PaintState>().sessions[&0].active_texture_fill().unwrap();
    assert_eq!(fill.settings.variation.color, 0.85);
    let doc = &app.world().resource::<editing::State>().documents[&0];
    let surface = doc.objects[0].materials[0].paint.as_ref().unwrap();
    let fill = surface.stack.layers[surface.stack.selected].texture_fill.as_ref().unwrap();
    assert_eq!(fill.settings.variation.color, 0.85);
    assert!(pixels(&app) != before_variation);
    paint_message(&mut app, PaintAction::Undo);
    assert!(pixels(&app) == before_variation);

    paint_click(&mut app, window, PaintAction::Category(Category::Smart));
    set_shelf_query(&mut app, "Oak Aged");
    let selected = app.world().resource::<PaintState>().sessions[&0].stack.selected;
    let row = paint_button_rect(&mut app, PaintAction::Select(selected));
    let drop_at = Vec2::new(row.center().x, row.min.y + row.height() * 0.2);
    drag_shelf_item(&mut app, window, PaintAction::AddSmartMaterial(0), drop_at);
    let start = std::time::Instant::now();
    loop {
        frame(&mut app);
        let session = &app.world().resource::<PaintState>().sessions[&0];
        let doc = &app.world().resource::<editing::State>().documents[&0];
        if !session.smart_loading && !doc.paint_busy {
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(150),
            "smart material did not finish, preparing={}, composing={}", session.smart_loading, doc.paint_busy);
    }
    println!("Smart material composition in test profile: {:?}", start.elapsed());
    let doc = &app.world().resource::<editing::State>().documents[&0];
    let surface = doc.objects[0].materials[0].paint.as_ref().unwrap();
    let folder = surface.stack.selected;
    assert_eq!(surface.stack.layers[folder].name, SMART_MATERIALS[0].name);
    assert_eq!(surface.stack.block(folder).len(), SMART_MATERIALS[0].len());
    assert!(surface.stack.layers[surface.stack.block(folder)].iter()
        .any(|layer| layer.mask.as_ref().is_some_and(|mask| mask.context.is_some())));
    assert!(pixels(&app) != before_variation, "the applied material must change the model");
    {
        use crate::editor::model_document::glb::Glb;
        let directory = std::env::temp_dir().join(format!("bevy-smart-glb-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("smart.glb");
        std::fs::write(&path, doc.source.encode(&doc.objects, &doc.original).unwrap()).unwrap();
        let restored = Glb::load(&path).unwrap();
        let node = doc.objects[0].source_node.unwrap();
        let mesh = restored.json["nodes"][node]["mesh"].as_u64().unwrap() as usize;
        let primitive = &restored.json["meshes"][mesh]["primitives"][0];
        let restored = restored.paint_surface(primitive).unwrap().unwrap();
        let restored_layers = &restored.stack.layers;
        assert_eq!(restored_layers.len(), surface.stack.layers.len());
        assert!(restored.stack.valid_hierarchy());
        for (restored, original) in restored_layers.iter().zip(&surface.stack.layers) {
            assert_eq!((&restored.name, restored.depth, restored.blend), (&original.name, original.depth, original.blend));
            assert_eq!(restored.mask.as_ref().and_then(|mask| mask.generator),
                original.mask.as_ref().and_then(|mask| mask.generator));
        }
        drop(restored);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    paint_message(&mut app, PaintAction::Undo);
    assert!(pixels(&app) == before_variation);
}
#[test]
#[ignore = "requires GPU; verifies UDIM set painting, resizing, visibility, highlighting and GLB persistence"]
fn gpu_texture_set_resolution_is_independent_and_saved() {
    use crate::editor::{model_document::glb::Glb, model_mesh::{Corner, Face, Geometry}};
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    {
        let mut state = app.world_mut().resource_mut::<editing::State>();
        let doc = state.documents.get_mut(&0).unwrap();
        let object = &mut doc.objects[0];
        let mut geometry = Geometry::default();
        for material in 0..2 {
            let z = -(material as f32);
            geometry.positions.extend([
                Vec3::new(-1., -1., z), Vec3::new(1., -1., z),
                Vec3::new(1., 1., z), Vec3::new(-1., 1., z),
            ]);
            geometry.faces.push(Face {
                corners: [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y].into_iter().enumerate()
                    .map(|(i, uv)| {
                        let mut corner = Corner::new(material * 4 + i);
                        corner.uv = uv + if material == 0 { Vec2::ZERO } else { Vec2::new(2., -1.) };
                        corner
                    }).collect(),
                material: 0,
                smooth: false,
            });
        }
        object.geometry = geometry.into();
        object.transform = Transform::IDENTITY;
        crate::editor::model_document::udim::import(object, true);
        assert_eq!(object.materials.len(), 2);
        assert_eq!(object.materials[0].udim().unwrap().number(), 1001);
        assert_eq!(object.materials[1].udim().unwrap().number(), 1013);
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::ZERO;
        viewer.distance = 4.;
        viewer.radius = 2.;
        viewer.yaw = 0.;
        viewer.pitch = 0.;
    }
    warm(&mut app, 5);
    paint_ready(&mut app);
    let uv_rect = {
        let world = app.world_mut();
        let (node, transform) = world
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<engine::TextureCanvas>>()
            .single(world).unwrap();
        Rect::from_center_size(
            transform.affine().translation * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        )
    };
    let dot = |app: &mut App, uv: Vec2| {
        move_pointer(app, window, uv_rect.min + uv_rect.size() * uv);
        mouse(app, window, ButtonState::Pressed);
        mouse(app, window, ButtonState::Released);
        paint_ready(app);
    };
    let surface = |app: &App, slot: usize| {
        app.world().resource::<editing::State>().documents[&0].objects[0]
            .materials[slot].paint.as_ref().unwrap().clone()
    };
    let alpha = |app: &App, slot, uv: Vec2| {
        let surface = surface(app, slot);
        let pixel = (uv * surface.size as f32).as_uvec2();
        let index = (pixel.y * surface.size + pixel.x) as usize * 4 + 3;
        surface.stack.layers[0].channels[0].as_ref().unwrap()[index]
    };
    dot(&mut app, Vec2::splat(0.5));
    assert_eq!(surface(&app, 0).size, 1024);
    paint_click(&mut app, window, PaintAction::Panel(Panel::Sets));
    paint_click(&mut app, window, PaintAction::Resolution(512));
    assert_eq!(surface(&app, 0).size, 512);
    assert_eq!(alpha(&app, 0, Vec2::splat(0.5)), 255);
    assert!(app.world().resource::<PaintState>().sessions[&0].undo.is_empty());
    dot(&mut app, Vec2::new(0.3, 0.5));
    assert_eq!(alpha(&app, 0, Vec2::new(0.3, 0.5)), 255, "GPU paints at the resized resolution");
    paint_click(&mut app, window, PaintAction::Set(1));
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].texture_size, 1024);
    dot(&mut app, Vec2::new(0.7, 0.5));
    assert_eq!(surface(&app, 1).size, 1024);
    assert_eq!(alpha(&app, 1, Vec2::new(0.7, 0.5)), 255);
    assert_eq!(alpha(&app, 0, Vec2::new(0.7, 0.5)), 0);
    let object = app.world().resource::<editing::State>().documents[&0].objects[0].id;
    let center = labelled(&mut app, "3D-вид модели").center();
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(alpha(&app, 1, Vec2::splat(0.5)), 0, "visible front set blocks the brush");
    let pixel = |image: &image::RgbaImage| image.get_pixel(center.x as u32, center.y as u32)[2];
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/model-paint-qa");
    std::fs::create_dir_all(&output).unwrap();
    let base = capture_when(&mut app, &target, &output.join("10-texture-set-base.png"), |_| true);
    paint_click(&mut app, window, PaintAction::SetPin(object, 0));
    move_pointer(&mut app, window, center);
    capture_when(&mut app, &target, &output.join("11-texture-set-highlight.png"), |image| {
        pixel(image) > pixel(&base).saturating_add(12)
    });
    paint_click(&mut app, window, PaintAction::SetHide(object, 0));
    let visible = |app: &mut App, slot| {
        let world = app.world_mut();
        world.query::<(&sets::SetPart, &InheritedVisibility)>().iter(world)
            .find(|(part, _)| part.0 == object && part.1 == slot).unwrap().1.get()
    };
    assert!(!visible(&mut app, 0));
    assert!(visible(&mut app, 1));
    paint_message(&mut app, PaintAction::SetHide(object, 1));
    assert!(visible(&mut app, 1), "the last shown set remains visible");
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(alpha(&app, 1, Vec2::splat(0.5)), 255, "brush reaches the set behind the hidden one");
    paint_click(&mut app, window, PaintAction::Set(0));
    dot(&mut app, Vec2::new(0.7, 0.5));
    assert_eq!(alpha(&app, 0, Vec2::new(0.7, 0.5)), 0, "the hidden active set cannot be painted in UV");
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].texture_size, 512);
    paint_click(&mut app, window, PaintAction::SetHide(object, 0));
    assert!(visible(&mut app, 0));
    let bytes = {
        let doc = &app.world().resource::<editing::State>().documents[&0];
        doc.source.encode(&doc.objects, &doc.original).unwrap()
    };
    let file = std::env::temp_dir().join(format!(
        "bevy-paint-set-{}.glb", bevy::asset::uuid::Uuid::new_v4()
    ));
    std::fs::write(&file, bytes).unwrap();
    let saved = Glb::load(&file).unwrap();
    std::fs::remove_file(file).unwrap();
    let sizes: Vec<_> = saved.json["meshes"].as_array().unwrap().iter().flat_map(|mesh| {
        mesh["primitives"].as_array().unwrap().iter()
    }).filter_map(|primitive| saved.paint_surface(primitive).unwrap().map(|s| {
        (primitive["extras"]["bevy_editor_udim"].as_u64().unwrap(), s.size)
    })).collect();
    assert_eq!(sizes, [(1001, 512), (1013, 1024)]);

    let directory = std::env::temp_dir().join(format!(
        "bevy-udim-export-{}", bevy::asset::uuid::Uuid::new_v4()
    ));
    std::fs::create_dir(&directory).unwrap();
    paint_click(&mut app, window, PaintAction::ExportPadding(export::Padding::Transparent));
    scroll_to_paint_control(&mut app, PaintAction::ExportTemplateMenu);
    paint_click(&mut app, window, PaintAction::ExportTemplateMenu);
    let godot = PaintAction::ExportTemplate(export::Template::Godot);
    scroll_to_paint_control(&mut app, godot);
    paint_click(&mut app, window, godot);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].export_settings.template, export::Template::Godot);
    use export::recipe::{Edit, Layout, Source};
    scroll_to_paint_control(&mut app, PaintAction::ExportTemplateMenu);
    paint_click(&mut app, window, PaintAction::ExportTemplateMenu);
    scroll_to_paint_control(&mut app, PaintAction::CopyExportTemplate);
    paint_click(&mut app, window, PaintAction::CopyExportTemplate);
    let recipe = {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        let export::Template::Custom(recipe) = session.export_settings.template else { panic!("Редактируемая копия не выбрана") };
        recipe
    };
    let edit = |output, channel, edit| PaintAction::EditExportTemplate { recipe, output, channel, edit };
    let added = app.world().resource::<export::prefs::Store>().recipe(recipe).unwrap().maps.len();
    for action in [
        edit(0, 0, Edit::Add),
        edit(added, 0, Edit::SetLayout(Layout::Rgb)),
        PaintAction::ExportComponent(1),
        PaintAction::ExportSourceMenu,
        edit(added, 1, Edit::SetSource(Source::Roughness)),
        edit(added, 1, Edit::ToggleInvert),
    ] {
        scroll_to_paint_control(&mut app, action);
        paint_click(&mut app, window, action);
    }
    {
        let mut fields = app.world_mut().query::<(&export::RecipeName, &mut bevy::text::EditableText)>();
        let (_, mut field) = fields.iter_mut(app.world_mut()).find(|(field, _)| {
            field.recipe == recipe && field.output == Some(added)
        }).unwrap();
        field.editor_mut().set_text("$mesh_Example_$udim");
    }
    scroll_to_paint_control(&mut app, edit(added, 1, Edit::RenameOutput));
    paint_click(&mut app, window, edit(added, 1, Edit::RenameOutput));
    {
        let store = app.world().resource::<export::prefs::Store>();
        let map = &store.recipe(recipe).unwrap().maps[added];
        assert_eq!(map.name, "$mesh_Example_$udim");
        assert_eq!(map.layout, Layout::Rgb);
        assert!(map.channels[1].invert && map.channels[1].src == Source::Roughness);
    }
    scroll_to_paint_control(&mut app, godot);
    paint_click(&mut app, window, godot);
    {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        assert_eq!(session.panel, Panel::Sets);
        assert_eq!(session.export_settings.template, export::Template::Godot);
        assert!(!session.export_template_menu);
    }
    for action in [
        PaintAction::ExportFileMenu,
        PaintAction::ExportFormat(export::Format::Tiff),
        PaintAction::ExportDepth(export::Depth::U16),
        PaintAction::ExportSize(Some(512)),
    ] {
        scroll_to_paint_control(&mut app, action);
        paint_click(&mut app, window, action);
    }
    scroll_to_paint_control(&mut app, PaintAction::ExportBakedNormal);
    paint_click(&mut app, window, PaintAction::ExportBakedNormal);
    assert!(!app.world().resource::<PaintState>().sessions[&0].export_settings.file.normal.use_baked);
    paint_click(&mut app, window, PaintAction::ExportBakedNormal);
    let capture = |selected| {
        let world = app.world();
        export::capture(
            &world.resource::<editing::State>().documents[&0],
            selected,
            world.resource::<Assets<Image>>(),
            world.resource::<Assets<StandardMaterial>>(),
            Some((world.resource::<bake::State>(), 0)),
        )
    };
    let session = &app.world().resource::<PaintState>().sessions[&0];
    assert_eq!(session.export_settings.padding, export::Padding::Transparent);
    assert_eq!(session.export_settings.template, export::Template::Godot);
    assert!(!session.export_template_menu);
    assert_eq!(session.export_settings.file, export::FileOptions {
        format: export::Format::Tiff, depth: export::Depth::U16, size: Some(512),
        ..export::FileOptions::default()
    });
    let report = export::write(
        &directory, "Block", capture(None), session.export_settings.padding, session.export_settings.width,
    ).unwrap();
    assert_eq!(report.files, 16);
    for (tile, slot, size) in [(1001, 0, 512), (1013, 1, 1024)] {
        let paths: Vec<_> = std::fs::read_dir(&report.path).unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.to_string_lossy().ends_with(&format!(".{tile}.png")))
            .collect();
        assert_eq!(paths.len(), 8);
        assert!(paths.iter().all(|path| image::image_dimensions(path).unwrap() == (size, size)));
        let base = paths.iter().find(|path| path.to_string_lossy().contains("_BaseColor.")).unwrap();
        let exported = image::open(base).unwrap().into_rgba8();
        let expected = surface(&app, slot).composite(0);
        assert_eq!(&exported.get_pixel(size / 2, size / 2).0[..3],
            &expected[((size / 2 * size + size / 2) * 4) as usize..][..3]);
    }
    let rerun = export::write(&directory, "Block", capture(Some((object, 1))), export::Padding::None, 8).unwrap();
    assert_ne!(rerun.path, report.path);
    assert_eq!(rerun.files, 8);
    assert!(std::fs::read_dir(&rerun.path).unwrap().all(|entry|
        entry.unwrap().file_name().to_string_lossy().ends_with(".1013.png")
    ));
    let godot = export::write_with_template(
        &directory, "Block", capture(None), session.export_settings.padding,
        session.export_settings.width, session.export_settings.template,
    ).unwrap();
    for (tile, size) in [(1001, 512), (1013, 1024)] {
        let orm = std::fs::read_dir(&godot.path).unwrap().map(|entry| entry.unwrap().path())
            .find(|path| path.to_string_lossy().ends_with(&format!("_ORM.{tile}.png"))).unwrap();
        assert_eq!(image::image_dimensions(orm).unwrap(), (size, size));
    }
    let tiff = export::write_with_options(
        &directory, "Block", capture(None), session.export_settings.padding,
        session.export_settings.width, session.export_settings.template, session.export_settings.file,
    ).unwrap();
    for tile in [1001, 1013] {
        let orm = std::fs::read_dir(&tiff.path).unwrap().map(|entry| entry.unwrap().path())
            .find(|path| path.to_string_lossy().ends_with(&format!("_ORM.{tile}.tif"))).unwrap();
        let orm = image::open(orm).unwrap();
        assert_eq!((orm.width(), orm.height()), (512, 512));
        assert_eq!(orm.color(), image::ColorType::Rgb16);
    }
    app.world_mut().resource_mut::<bake::State>().set_disk_cache(directory.join("bake-cache"));
    paint_click(&mut app, window, PaintAction::Panel(Panel::Maps));
    fn doc(app: &App) -> &crate::editor::model_document::Document {
        &app.world().resource::<editing::State>().documents[&0]
    }
    let wait_handle = |app: &mut App, id| {
        let start = std::time::Instant::now();
        loop {
            frame(app);
            let world = app.world();
            let recipe = world.resource::<editing::State>().documents[&0].custom_bakes
                .iter().find(|recipe| recipe.id == id).unwrap();
            if let Some(point) = world.resource::<bake::custom_gizmo::Gizmo>()
                .target(world.resource::<Viewer>(), recipe)
            {
                break point;
            }
            assert!(start.elapsed().as_secs() < 10);
        }
    };
    paint_message(&mut app, PaintAction::BakeCustomAdd(custom::Kind::Point));
    let point_id = doc(&app).custom_bakes[0].id;
    paint_message(&mut app, PaintAction::BakeCustomGizmo(point_id));
    wait_handle(&mut app, point_id);
    let preview_pixels = |app: &mut App, different: Option<&Vec<u8>>, size: Option<u32>| {
        let start = std::time::Instant::now();
        loop {
            frame(app);
            let world = app.world();
            let baked = world.resource::<bake::State>();
            if let Some(handle) = baked.preview(bake::BakeMap::Custom(point_id))
                && let Some(image) = world.resource::<Assets<Image>>().get(handle)
                && size.is_none_or(|size| image.texture_descriptor.size.width == size)
                && let Some(pixels) = &image.data
                && different.is_none_or(|before| pixels != before)
            {
                return pixels.clone();
            }
            assert!(start.elapsed().as_secs() < 12, "{:?}", baked.live.status(point_id));
        }
    };
    let initial_preview = preview_pixels(&mut app, None, Some(256));
    assert!(app.world().resource::<bake::State>().live.uses_gpu(point_id));
    let gizmo_rect = labelled(&mut app, "3D-вид модели");
    let on_plane = editing::input::project(app.world().resource::<Viewer>(), gizmo_rect, Vec3::new(0.45, -0.25, 0.)).unwrap();
    let history = doc(&app).undo.len();
    move_pointer(&mut app, window, on_plane);
    mouse(&mut app, window, ButtonState::Pressed);
    assert_eq!(doc(&app).custom_bakes[0].point, [0; 3]);
    assert_ne!(preview_pixels(&mut app, Some(&initial_preview), Some(256)), initial_preview);
    assert!(app.world().resource::<bake::State>().maps(0, doc(&app), (object, 0), 512).is_none());
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(doc(&app).custom_bakes[0].point, [45, -25, 50]);
    assert_eq!(doc(&app).undo.len(), history + 1);
    preview_pixels(&mut app, None, Some(256));
    let displayed = |app: &mut App, slot| {
        let world = app.world_mut();
        world.query::<(&sets::SetPart, &MeshMaterial3d<StandardMaterial>)>().iter(world)
            .find(|(part, _)| (part.0, part.1) == (object, slot)).unwrap().1.0.clone()
    };
    let working = |app: &App, slot| {
        app.world().resource::<engine::Runtime>().working_material(0, object, slot).unwrap().clone()
    };
    let preview = app.world().resource::<bake::State>().preview(bake::BakeMap::Custom(point_id)).unwrap().clone();
    let overlay = displayed(&mut app, 0);
    {
        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let map = materials.get(&overlay).unwrap();
        assert!(map.unlit);
        assert_eq!(map.base_color_texture.as_ref(), Some(&preview));
    }
    assert_ne!(overlay, working(&app, 0));
    assert_eq!(displayed(&mut app, 1), working(&app, 1));
    paint_message(&mut app, PaintAction::View(View::Three));
    assert_eq!(displayed(&mut app, 0), overlay);
    let three = labelled(&mut app, "3D-вид модели");
    let projections = {
        let viewer = app.world().resource::<Viewer>();
        [Vec3::new(0.22, -0.17, 0.), Vec3::new(-0.77, 0.37, 0.)]
            .map(|point| editing::input::project(viewer, three, point).unwrap().as_uvec2())
    };
    capture_when(&mut app, &target, &output.join("11-bake-map-on-model.png"), |image| {
        let [near, far] = projections.map(|pixel| image.get_pixel(pixel.x, pixel.y).0);
        near[0].saturating_sub(far[0]) > 60 && near[0].abs_diff(near[1]) <= 3 && near[1].abs_diff(near[2]) <= 3
    });
    paint_message(&mut app, PaintAction::BakeShowOnModel);
    assert_eq!(displayed(&mut app, 0), working(&app, 0));
    assert!(app.world().resource::<bake::State>().preview(bake::BakeMap::Custom(point_id)).is_some());
    paint_message(&mut app, PaintAction::BakeShowOnModel);
    paint_message(&mut app, PaintAction::View(View::Split));
    paint_message(&mut app, PaintAction::BakeLivePreview);
    assert!(app.world().resource::<bake::State>().preview(bake::BakeMap::Custom(point_id)).is_none());
    paint_message(&mut app, PaintAction::BakeLivePreview);
    paint_message(&mut app, PaintAction::AddMask(true));
    paint_message(&mut app, PaintAction::Generator(None, GeneratorEdit::Set(GeneratorKind::CustomMap)));
    paint_message(&mut app, PaintAction::Generator(None, GeneratorEdit::CustomWeight(point_id, 100)));
    let persisted = surface(&app, 0);
    assert!(persisted.baked.is_none());
    paint_message(&mut app, PaintAction::BakeLiveMasks);
    paint_message(&mut app, PaintAction::PreviewMask);
    let wait_mask = |app: &mut App, different_from: Option<blake3::Hash>| {
        let start = std::time::Instant::now();
        loop {
            frame(app);
            if let Some((256, hash)) = app.world().resource::<bake::State>().masks.mask_digest()
                && different_from != Some(hash)
            {
                return hash;
            }
            assert!(start.elapsed().as_secs() < 15, "live UV mask did not update");
        }
    };
    let first_mask = wait_mask(&mut app, None);
    warm(&mut app, 3);
    let mask_handle = {
        let world = app.world();
        let paint = world.resource::<PaintState>();
        world.resource::<bake::State>().masks.mask(0, doc(&app), &paint.sessions[&0]).unwrap()
    };
    {
        let world = app.world_mut();
        let image = world.query_filtered::<&ImageNode, With<engine::TextureCanvas>>().single(world).unwrap();
        assert_eq!(image.image, mask_handle);
    }
    assert_ne!(displayed(&mut app, 0), working(&app, 0));
    let pick = editing::input::project(app.world().resource::<Viewer>(), gizmo_rect, Vec3::new(-0.45, 0.25, 0.)).unwrap();
    move_pointer(&mut app, window, pick);
    mouse(&mut app, window, ButtonState::Pressed);
    assert_ne!(wait_mask(&mut app, Some(first_mask)), first_mask);
    assert!(Arc::ptr_eq(&persisted, &surface(&app, 0)));
    assert_eq!(doc(&app).custom_bakes[0].point, [45, -25, 50]);
    fill_modifier(&mut app, window, KeyCode::Escape, Key::Escape, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    fill_modifier(&mut app, window, KeyCode::Escape, Key::Escape, ButtonState::Released);
    paint_message(&mut app, PaintAction::BakeLiveMasks);
    assert_eq!(displayed(&mut app, 0), working(&app, 0));
    paint_message(&mut app, PaintAction::BakeShowOnModel);
    paint_message(&mut app, PaintAction::RemoveMask);
    paint_message(&mut app, PaintAction::BakeCustomRemove(point_id));

    paint_message(&mut app, PaintAction::BakeCustomAdd(custom::Kind::Flow));
    let custom_id = app.world().resource::<editing::State>().documents[&0].custom_bakes[0].id;
    paint_message(&mut app, PaintAction::BakeCustomGizmo(custom_id));
    let arrow = wait_handle(&mut app, custom_id);
    let wait_flow = std::time::Instant::now();
    while !app.world().resource::<bake::State>().live.uses_gpu(custom_id) {
        frame(&mut app);
        assert!(wait_flow.elapsed().as_secs() < 15, "GPU flow preview did not arrive");
    }
    let start = editing::input::project(app.world().resource::<Viewer>(), gizmo_rect, arrow).unwrap();
    let history = doc(&app).undo.len();
    move_pointer(&mut app, window, start);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, start + Vec2::new(35., 35.));
    assert!(app.world().resource::<bake::custom_gizmo::Gizmo>().draft().unwrap().elevation < 90);
    fill_modifier(&mut app, window, KeyCode::Escape, Key::Escape, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    fill_modifier(&mut app, window, KeyCode::Escape, Key::Escape, ButtonState::Released);
    assert_eq!(doc(&app).custom_bakes[0].elevation, 90);
    assert_eq!(doc(&app).undo.len(), history);
    paint_message(&mut app, PaintAction::BakeCustomGizmo(custom_id));
    for (field, step, count) in [
        (custom::Field::Elevation, -1, 18),
        (custom::Field::Spread, -1, 3),
        (custom::Field::Strength, 1, 1),
    ] {
        for _ in 0..count {
            app.world_mut().write_message(Action::ModelView(ModelViewAction::Paint(
                PaintAction::BakeCustomAdjust(custom_id, field, step),
            )));
        }
    }
    warm(&mut app, 3);
    paint_message(&mut app, PaintAction::BakeCustomDuplicate(custom_id));
    let copy_id = {
        let maps = &app.world().resource::<editing::State>().documents[&0].custom_bakes;
        assert_eq!(maps.len(), 2);
        assert_ne!(maps[0].id, maps[1].id);
        assert_ne!(maps[0].name, maps[1].name);
        assert_eq!(maps[0].strength, maps[1].strength);
        maps[1].id
    };
    paint_message(&mut app, PaintAction::BakePreview(None));
    paint_click(&mut app, window, PaintAction::BakeStart);
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        if app.world().resource::<bake::State>().preview(bake::BakeMap::Occlusion).is_some() {
            break;
        }
        assert!(started.elapsed().as_secs() < 60, "{}", app.world().resource::<Editor>().status);
    }
    assert!(app.world().resource::<bake::State>()
        .maps(0, doc(&app), (object, 0), 512).unwrap().gpu_rays);
    paint_message(&mut app, PaintAction::BakeStart);
    assert_eq!(app.world().resource::<Editor>().status, "Использованы готовые карты меша");
    assert!(app.world().resource::<bake::State>().progress().is_none());
    paint_message(&mut app, PaintAction::BakePreview(Some(bake::BakeMap::Custom(custom_id))));
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        if let Some(handle) = world.resource::<bake::State>().preview(bake::BakeMap::Custom(custom_id)) {
            let image = world.resource::<Assets<Image>>().get(handle).unwrap();
            assert!(image.data.as_ref().unwrap().chunks_exact(4).any(|pixel| pixel[0] > 200));
            break;
        }
        assert!(started.elapsed().as_secs() < 60, "{}", app.world().resource::<Editor>().status);
    }
    let flow_map = bake::BakeMap::Custom(custom_id);
    let flow_preview = app.world().resource::<bake::State>().preview(flow_map).unwrap().clone();
    paint_message(&mut app, PaintAction::BakeLivePreview);
    let flow_overlay = displayed(&mut app, 0);
    assert_eq!(
        app.world().resource::<Assets<StandardMaterial>>().get(&flow_overlay).unwrap().base_color_texture.as_ref(),
        Some(&flow_preview),
    );
    paint_message(&mut app, PaintAction::BakeLivePreview);
    let field = {
        let world = app.world_mut();
        let mut names = world.query::<(Entity, &ui::CustomMapName, &mut bevy::text::EditableText)>();
        let (entity, _, mut text) = names.iter_mut(world).find(|(_, name, _)| name.0 == custom_id).unwrap();
        text.editor_mut().set_text("Rain trails");
        entity
    };
    app.world_mut().resource_mut::<bevy::input_focus::InputFocus>()
        .set(field, bevy::input_focus::FocusCause::Auto);
    fill_modifier(&mut app, window, KeyCode::Enter, Key::Enter, ButtonState::Pressed);
    fill_modifier(&mut app, window, KeyCode::Enter, Key::Enter, ButtonState::Released);
    paint_message(&mut app, PaintAction::BakeCustomMove(copy_id, -1));
    {
        let world = app.world();
        let maps = &world.resource::<editing::State>().documents[&0].custom_bakes;
        assert_eq!((maps[0].id, maps[1].id), (copy_id, custom_id));
        assert_eq!(maps[1].name, "Rain trails");
        assert_eq!(world.resource::<bake::State>().preview(flow_map), Some(&flow_preview));
    }
    paint_message(&mut app, PaintAction::BakePreview(Some(bake::BakeMap::Occlusion)));
    let world = app.world();
    let baked_sets = export::capture(
        &world.resource::<editing::State>().documents[&0], Some((object, 0)),
        world.resource::<Assets<Image>>(), world.resource::<Assets<StandardMaterial>>(),
        Some((world.resource::<bake::State>(), 0)),
    );
    let baked = export::write(&directory, "Block", baked_sets, export::Padding::default(), 8).unwrap();
    assert_eq!(baked.files, 23);
    assert!(std::fs::read_dir(&baked.path).unwrap().any(|entry|
        entry.unwrap().file_name().to_string_lossy().ends_with(&format!("_Rain_trails_{custom_id:016x}.1001.png"))
    ));
    let occlusion = std::fs::read_dir(&baked.path).unwrap().map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with("_AmbientOcclusion.1001.png")).unwrap();
    assert_eq!(image::image_dimensions(&occlusion).unwrap(), (512, 512));
    let baked_state = world.resource::<bake::State>();
    let maps = baked_state.maps(0, doc(&app), (object, 0), 512).unwrap();
    let packed = export::write_with_template(
        &directory, "Block", export::capture(
            doc(&app), Some((object, 0)), world.resource::<Assets<Image>>(),
            world.resource::<Assets<StandardMaterial>>(), Some((baked_state, 0)),
        ), export::Padding::None, 8, export::Template::Godot,
    ).unwrap();
    let orm = std::fs::read_dir(&packed.path).unwrap().map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with("_ORM.1001.png")).unwrap();
    let orm = image::open(orm).unwrap().into_rgb8();
    let ao = image::open(occlusion).unwrap().into_rgba8();
    for ((packed, reference), covered) in orm.pixels().zip(ao.pixels()).zip(maps.island_texels()) {
        if covered {
            assert_eq!(packed[0], reference[0]);
        }
    }
    assert!(std::fs::read_dir(&baked.path).unwrap().any(|entry|
        entry.unwrap().file_name().to_string_lossy().ends_with("_Curvature.1001.png")
    ));
    let source = directory.join("high.glb");
    let cage = directory.join("cage.glb");
    {
        let doc = &app.world().resource::<editing::State>().documents[&0];
        let mut high = doc.objects.iter().find(|candidate| candidate.id == object).unwrap().clone();
        high.source_node = None;
        high.materials.iter_mut().for_each(|slot| slot.paint = None);
        let glb = crate::editor::model_document::glb::Glb::new_asset(std::slice::from_ref(&high)).unwrap();
        std::fs::write(&source, glb).unwrap();
        high.name.push_str("_cage");
        high.transform.translation.z += 0.12;
        let glb = crate::editor::model_document::glb::Glb::new_asset(&[high]).unwrap();
        std::fs::write(&cage, glb).unwrap();
    }
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        doc.bake_settings.high_poly = Some(source.clone());
        doc.bake_settings.cage = Some(cage.clone());
    }
    warm(&mut app, 3);
    let center = labelled(&mut app, "3D-вид модели").center().as_uvec2();
    let base = capture_when(&mut app, &target, &output.join("12-cage-disabled.png"), |_| true);
    paint_click(&mut app, window, PaintAction::BakeInspectCage);
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let mesh = {
            let world = app.world_mut();
            world.query_filtered::<&Mesh3d, With<bake::cage_preview::Shell>>()
                .iter(world).next().map(|mesh| mesh.0.clone())
        };
        if let Some(mesh) = mesh {
            let meshes = app.world().resource::<Assets<Mesh>>();
            assert!(meshes.get(&mesh).unwrap().attribute(Mesh::ATTRIBUTE_COLOR).is_some());
            break;
        }
        assert!(started.elapsed().as_secs() < 60, "{}", app.world().resource::<PaintState>().sessions[&0].bake_cage_status);
    }
    capture_when(&mut app, &target, &output.join("13-cage-preview.png"), |image| {
        let changed = |x: u32, y: u32| {
            let a = base.get_pixel(x, y);
            let b = image.get_pixel(x, y);
            (0..3).any(|channel| a[channel].abs_diff(b[channel]) > 12)
        };
        (center.y - 20..center.y + 20).map(|y| {
            (center.x - 20..center.x + 20).filter(|&x| changed(x, y)).count()
        }).sum::<usize>() > 500
    });
    paint_click(&mut app, window, PaintAction::BakeInspectCage);
    {
        let world = app.world_mut();
        assert_eq!(world.query_filtered::<Entity, With<bake::cage_preview::Shell>>().iter(world).count(), 0);
    }
    paint_click(&mut app, window, PaintAction::BakeStart);
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        if app.world().resource::<bake::State>().preview(bake::BakeMap::ProjectedNormal).is_some() {
            break;
        }
        assert!(started.elapsed().as_secs() < 60, "{}", app.world().resource::<Editor>().status);
    }
    let maps = app.world().resource::<bake::State>()
        .maps(0, doc(&app), (object, 0), 512).unwrap();
    assert!(maps.gpu_rays && maps.gpu_projection && maps.gpu_denoise && maps.gpu_padding);
    paint_message(&mut app, PaintAction::BakeStart);
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        if world.resource::<bake::State>().preview(bake::BakeMap::ProjectedNormal).is_some() {
            assert!(world.resource::<Editor>().status.starts_with("Карты загружены из дискового кэша"));
            break;
        }
        assert!(started.elapsed().as_secs() < 60, "{}", world.resource::<Editor>().status);
    }
    let world = app.world();
    let high_sets = export::capture(
        &world.resource::<editing::State>().documents[&0], Some((object, 0)),
        world.resource::<Assets<Image>>(), world.resource::<Assets<StandardMaterial>>(),
        Some((world.resource::<bake::State>(), 0)),
    );
    let high = export::write(&directory, "Block", high_sets, export::Padding::default(), 8).unwrap();
    assert_eq!(high.files, 26);
    let projected = std::fs::read_dir(&high.path).unwrap().map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with("_ProjectionCoverage.1001.png")).unwrap();
    let projected = image::open(projected).unwrap().into_rgba8();
    assert_eq!(projected.dimensions(), (512, 512));
    assert!(projected.pixels().any(|pixel| pixel.0 == [255; 4]));
    let normal_path = |suffix: &str| {
        std::fs::read_dir(&high.path).unwrap().map(|entry| entry.unwrap().path())
            .find(|path| path.to_string_lossy().ends_with(suffix)).unwrap()
    };
    let normal = image::open(normal_path("_Normal.1001.png")).unwrap().into_rgb8();
    let bake_normal = image::open(normal_path("_BakedNormal.1001.png")).unwrap().into_rgba8();
    for ((material, baked), coverage) in normal.pixels().zip(bake_normal.pixels()).zip(projected.pixels()) {
        if coverage.0 == [255; 4] {
            assert_eq!(material.0, baked.0[..3]);
        }
    }
    paint_message(&mut app, PaintAction::BakeClearCage);
    assert!(app.world().resource::<PaintState>().sessions[&0].bake_projection.cage.is_none());
    paint_click(&mut app, window, PaintAction::BakePaintCage);
    let shell_height = |app: &mut App| {
        let world = app.world_mut();
        let handle = world.query_filtered::<&Mesh3d, With<bake::cage_preview::Shell>>()
            .iter(world).next()?.0.clone();
        let mesh = world.resource::<Assets<Mesh>>().get(&handle)?;
        let bevy::mesh::VertexAttributeValues::Float32x3(points) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)? else { return None };
        Some(points.iter().map(|point| point[2]).fold(f32::NEG_INFINITY, f32::max))
    };
    let started = std::time::Instant::now();
    let original = loop {
        frame(&mut app);
        if let Some(height) = shell_height(&mut app)
            && app.world().resource::<bake::cage_brush::Brush>().pick_ready()
        { break height; }
        assert!(started.elapsed().as_secs() < 60, "{}", app.world().resource::<PaintState>().sessions[&0].bake_cage_status);
    };
    // The completed probe keeps its source BVH for brush strokes.
    let parked = source.with_extension("parked");
    std::fs::rename(&source, &parked).unwrap();
    let viewport = labelled(&mut app, "3D-вид модели");
    let point = editing::input::project(app.world().resource::<Viewer>(), viewport, Vec3::new(-0.94, -0.94, 0.)).unwrap();
    let initial_alpha = alpha(&app, 0, Vec2::splat(0.04));
    move_pointer(&mut app, window, point);
    mouse(&mut app, window, ButtonState::Pressed);
    warm(&mut app, 12);
    mouse(&mut app, window, ButtonState::Released);
    let scales = |app: &App| {
        let world = app.world();
        let session = &world.resource::<PaintState>().sessions[&0];
        let doc = &world.resource::<editing::State>().documents[&0];
        let item = doc.objects.iter().find(|item| item.id == object).unwrap();
        bake::projection_for(session, item).manual.unwrap().values
    };
    let raised = scales(&app);
    assert!(raised[0] > 1., "{raised:?}");
    assert_eq!(raised[1], 1.);
    assert_eq!(*doc(&app).cage_edits[&object].scales.values, *raised);
    paint_message(&mut app, PaintAction::Undo);
    assert!(!doc(&app).cage_edits.contains_key(&object));
    paint_message(&mut app, PaintAction::Redo);
    assert_eq!(*doc(&app).cage_edits[&object].scales.values, *raised);
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        if shell_height(&mut app).is_some_and(|height| height > original + 0.0001) { break; }
        assert!(started.elapsed().as_secs() < 30, "{}", app.world().resource::<PaintState>().sessions[&0].bake_cage_status);
    }
    fill_modifier(&mut app, window, KeyCode::ControlLeft, Key::Control, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Pressed);
    warm(&mut app, 6);
    mouse(&mut app, window, ButtonState::Released);
    fill_modifier(&mut app, window, KeyCode::ControlLeft, Key::Control, ButtonState::Released);
    assert!(scales(&app)[0] < raised[0]);
    let before_restore = scales(&app)[0];
    fill_modifier(&mut app, window, KeyCode::ShiftLeft, Key::Shift, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Pressed);
    warm(&mut app, 6);
    mouse(&mut app, window, ButtonState::Released);
    fill_modifier(&mut app, window, KeyCode::ShiftLeft, Key::Shift, ButtonState::Released);
    assert!((scales(&app)[0] - 1.).abs() < (before_restore - 1.).abs());
    assert_eq!(alpha(&app, 0, Vec2::splat(0.04)), initial_alpha);
    std::fs::rename(&parked, &source).unwrap();
    paint_message(&mut app, PaintAction::BakeResetCagePaint);
    assert!(!app.world().resource::<PaintState>().sessions[&0].cage_edits.contains_key(&object));
    assert!(!doc(&app).cage_edits.contains_key(&object));
    paint_message(&mut app, PaintAction::BakeInspectCage);
    paint_message(&mut app, PaintAction::BakeAutoFitCage);
    assert!(app.world().resource::<PaintState>().sessions[&0].bake_projection.auto_fit);
    paint_message(&mut app, PaintAction::BakeStart);
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        if world.resource::<bake::State>().maps(0, doc, (object, 0), 512)
            .is_some_and(|maps| maps.cage_fit.is_some())
        {
            break;
        }
        assert!(started.elapsed().as_secs() < 60, "{}", world.resource::<Editor>().status);
    }
    for action in [
        PaintAction::AddMask(true),
        PaintAction::Generator(None, GeneratorEdit::Set(GeneratorKind::PartGradient)),
    ] {
        app.world_mut().write_message(Action::ModelView(ModelViewAction::Paint(action)));
    }
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        if let Some(surface) = doc.objects.iter().find(|item| item.id == object)
            .and_then(|item| item.materials[0].paint.as_ref())
            && let Some(inputs) = &surface.baked
        {
            let maps = world.resource::<bake::State>().maps(0, doc, (object, 0), 512).unwrap();
            assert!(Arc::ptr_eq(inputs, maps.generator_inputs()));
            let mask = surface.stack.layers[surface.stack.selected].mask.as_ref().unwrap().effective();
            assert!(mask.chunks_exact(4).any(|p| (1..255).contains(&p[0])));
            break;
        }
        assert!(started.elapsed().as_secs() < 60, "{}", world.resource::<Editor>().status);
    }
    let color = app.world().resource::<PaintState>().sessions[&0].id_palette()[0];
    for action in [
        PaintAction::Generator(None, GeneratorEdit::Set(GeneratorKind::IdMask)),
        PaintAction::Generator(None, GeneratorEdit::IdColor(color)),
        PaintAction::Generator(None, GeneratorEdit::IdTolerance(-1)),
    ] {
        app.world_mut().write_message(Action::ModelView(ModelViewAction::Paint(action)));
    }
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        if let Some(mask) = doc.objects.iter().find(|item| item.id == object)
            .and_then(|item| item.materials[0].paint.as_ref())
            .and_then(|surface| surface.stack.layers[surface.stack.selected].mask.as_ref())
            && let Some(generator) = mask.generator
            && generator.kind == GeneratorKind::IdMask
        {
            assert_eq!(generator.id_color, Some(color));
            assert_eq!(generator.id_tolerance, 14);
            assert!(mask.effective().chunks_exact(4).any(|p| p[0] == 255));
            break;
        }
        assert!(started.elapsed().as_secs() < 60, "{}", world.resource::<Editor>().status);
    }
    paint_message(&mut app, PaintAction::Generator(None, GeneratorEdit::Weight(GeneratorInput::Id, -100)));
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        if let Some(mask) = doc.objects.iter().find(|item| item.id == object)
            .and_then(|item| item.materials[0].paint.as_ref())
            .and_then(|surface| surface.stack.layers[surface.stack.selected].mask.as_ref())
            && mask.generator.is_some_and(|g| g.weight(GeneratorInput::Id) == -100)
        {
            assert!(mask.effective().chunks_exact(4).all(|p| p[0] == 0));
            break;
        }
        assert!(started.elapsed().as_secs() < 60, "{}", world.resource::<Editor>().status);
    }
    for action in [
        PaintAction::Generator(None, GeneratorEdit::Set(GeneratorKind::CustomMap)),
        PaintAction::Generator(None, GeneratorEdit::CustomWeight(custom_id, 100)),
    ] {
        app.world_mut().write_message(Action::ModelView(ModelViewAction::Paint(action)));
    }
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        if let Some(mask) = doc.objects.iter().find(|item| item.id == object)
            .and_then(|item| item.materials[0].paint.as_ref())
            .and_then(|surface| surface.stack.layers[surface.stack.selected].mask.as_ref())
            && mask.generator.is_some_and(|g| g.custom[0].id == custom_id)
        {
            assert!(mask.effective().chunks_exact(4).any(|p| p[0] > 0));
            break;
        }
        assert!(started.elapsed().as_secs() < 60, "{}", world.resource::<Editor>().status);
    }
    paint_click(&mut app, window, PaintAction::BakeQuality(bake::Quality::Standard));
    {
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        assert_eq!(world.resource::<PaintState>().sessions[&0].bake_projection.quality, bake::Quality::Standard);
        assert!(world.resource::<bake::State>().maps(0, doc, (object, 0), 512).is_none());
    }
    paint_click(&mut app, window, PaintAction::BakeMapOptions);
    paint_click(&mut app, window, PaintAction::BakeSelectAllMaps(false));
    paint_click(&mut app, window, PaintAction::BakeStart);
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        if let Some(maps) = world.resource::<bake::State>().maps(0, doc(&app), (object, 0), 512) {
            assert!(maps.gpu_projection && !maps.gpu_denoise);
            assert!(maps.has_map(bake::BakeMap::WorldNormal));
            assert!(maps.has_map(bake::BakeMap::Custom(custom_id)));
            assert!(!maps.has_map(bake::BakeMap::Occlusion));
            assert!(!maps.has_map(bake::BakeMap::Curvature));
            assert_ne!(maps.generator_inputs().disabled_inputs, 0);
            break;
        }
        assert!(started.elapsed().as_secs() < 60, "{}", world.resource::<Editor>().status);
    }
    let world = app.world();
    let minimum_sets = export::capture(
        doc(&app), Some((object, 0)),
        world.resource::<Assets<Image>>(), world.resource::<Assets<StandardMaterial>>(),
        Some((world.resource::<bake::State>(), 0)),
    );
    let minimum = export::write(&directory, "Minimum", minimum_sets, export::Padding::default(), 8).unwrap();
    assert!(!std::fs::read_dir(&minimum.path).unwrap().any(|entry|
        entry.unwrap().file_name().to_string_lossy().contains("_AmbientOcclusion.")
    ));
    let cache_dir = directory.join("bake-cache");
    assert!(std::fs::read_dir(&cache_dir).unwrap().next().is_some());
    paint_click(&mut app, window, PaintAction::BakeClearDiskCache);
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        let status = &world.resource::<Editor>().status;
        if status.starts_with("Дисковый кэш запекания очищен") {
            assert!(std::fs::read_dir(&cache_dir).unwrap().next().is_none());
            assert!(world.resource::<bake::State>().maps(0, doc(&app), (object, 0), 512).is_some());
            break;
        }
        assert!(started.elapsed().as_secs() < 10, "{status}");
    }
    paint_click(&mut app, window, PaintAction::BakeStart);
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        let status = &world.resource::<Editor>().status;
        if status.starts_with("Карты меша готовы") {
            assert!(std::fs::read_dir(&cache_dir).unwrap().next().is_some());
            break;
        }
        assert!(started.elapsed().as_secs() < 60, "{status}");
    }
    std::fs::remove_file(cage).unwrap();
    std::fs::remove_file(source).unwrap();
    for results in [
        report.path, rerun.path, godot.path, tiff.path, baked.path, packed.path, high.path, minimum.path,
        directory.join("bake-cache"),
    ] {
        for entry in std::fs::read_dir(&results).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(results).unwrap();
    }
    std::fs::remove_dir(directory).unwrap();
}

#[test]
#[ignore = "requires GPU; checks brush controls and independent stacks across model tabs"]
fn gpu_material_paint_keeps_brush_when_switching_model_tabs() {
    let (mut app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    let wait = |app: &mut App, condition: &dyn Fn(&World) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !condition(app.world()) {
            assert!(std::time::Instant::now() < deadline, "{}", app.world().resource::<Editor>().status);
            frame(app);
        }
    };
    let show = |app: &mut App, index| {
        app.world_mut().write_message(Action::ViewModel(index));
        wait(app, &|world| world.resource::<Editor>().viewing_model == Some(index)
            && world.resource::<editing::State>().documents.contains_key(&index));
        app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Mode(Mode::Paint))));
        wait(app, &|world| world.resource::<PaintState>().last_painted_model == Some(index)
            && world.resource::<PaintState>().sessions[&index].binding.is_some());
        paint_ready(app);
    };
    show(&mut app, 0);
    let initial_layers = app.world().resource::<PaintState>().sessions[&0].stack.layers.len();
    paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
    paint_message(&mut app, PaintAction::BrushProjection(BrushProjection::Surface));
    paint_message(&mut app, PaintAction::PressureFlow);
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let first = paint.sessions.get_mut(&0).unwrap();
        first.set_value(Parameter::Size, 132.);
        first.set_value(Parameter::Roughness, 73.);
        first.picked = Some([219, 81, 43]);
    }
    show(&mut app, 1);
    {
        let paint = app.world().resource::<PaintState>();
        let second = &paint.sessions[&1];
        assert_eq!(second.stack.layers.len(), initial_layers);
        assert_eq!(second.projection, BrushProjection::Surface);
        assert!(second.pressure_flow);
        assert_eq!(second.size, 132.);
        assert_eq!(second.roughness, 73.);
        assert_eq!(second.picked, Some([219, 81, 43]));
    }
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        paint.sessions.get_mut(&1).unwrap().set_value(Parameter::Size, 45.);
    }
    app.world_mut().write_message(Action::CloseModelTab(1));
    wait(&mut app, &|world| !world.resource::<Editor>().model_tabs.contains(&1));
    show(&mut app, 0);
    let paint = app.world().resource::<PaintState>();
    assert_eq!(paint.sessions[&0].stack.layers.len(), initial_layers + 1);
    assert_eq!(paint.sessions[&0].size, 45.);
    assert_eq!(paint.sessions[&0].picked, Some([219, 81, 43]));
}

#[test]
#[ignore = "requires GPU; samples a live texture in UV and an unvisited UDIM tile in 3D"]
fn gpu_material_picker_copies_color_roughness_and_metallic_across_sets() {
    use crate::editor::{
        model_document::{MaterialDraft, udim},
        model_mesh::{Corner, Face, Geometry},
    };
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let (mut app, window, _) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    let wait = |app: &mut App, condition: &dyn Fn(&World) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !condition(app.world()) {
            assert!(std::time::Instant::now() < deadline, "Picker timed out: {}", app.world().resource::<Editor>().status);
            frame(app);
        }
    };
    wait(&mut app, &|world| world.resource::<editing::State>().documents.contains_key(&0));
    let colors = [[40, 100, 180], [206, 71, 24]];
    let materials = {
        let mut assets = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
        colors.into_iter().zip([(0.2, 0.6), (0.8, 0.4)]).enumerate().map(|(index, (rgb, (roughness, metallic)))| {
            let [r, g, b] = rgb.map(|component| f32::from(component) / 255.);
            MaterialDraft { name: format!("Материал {index}"), color: bevy::color::Srgba::new(r, g, b, 1.),
                roughness, metallic }.slot(&mut assets)
        }).collect()
    };
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        let object = &mut doc.objects[0];
        let mut geometry = Geometry::default();
        for material in 0..2 {
            let left = material as f32 * 1.6 - 1.5;
            geometry.positions.extend([
                Vec3::new(left, -0.75, 0.), Vec3::new(left + 1.4, -0.75, 0.),
                Vec3::new(left + 1.4, 0.75, 0.), Vec3::new(left, 0.75, 0.),
            ]);
            geometry.faces.push(Face {
                corners: [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y].into_iter().enumerate().map(|(i, uv)| {
                    let mut corner = Corner::new(material * 4 + i);
                    corner.uv = uv + if material == 0 { Vec2::ZERO } else { Vec2::new(2., -1.) };
                    corner
                }).collect(),
                material, smooth: false,
            });
        }
        object.geometry = geometry.into();
        object.transform = Transform::IDENTITY;
        object.materials = materials;
        udim::import(object, true);
        assert!(object.materials.iter().any(|slot| slot.udim().is_some_and(|tile| tile.number() == 1013)));
        doc.paint_default_size = 512;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 6);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::View(View::Two));
    let binding = app.world().resource::<PaintState>().sessions[&0].binding;
    let pick = |app: &mut App, point| {
        paint_message(app, PaintAction::Tool(PaintTool::Picker));
        move_pointer(app, window, point);
        mouse(app, window, ButtonState::Pressed);
        mouse(app, window, ButtonState::Released);
        wait(app, &|world| world.resource::<PaintState>().sessions[&0].tool == PaintTool::Brush);
    };
    let uv = {
        let world = app.world_mut();
        let (node, transform) = world.query_filtered::<
            (&ComputedNode, &UiGlobalTransform), With<engine::TextureCanvas>,
        >().single(world).unwrap();
        transform.affine().translation * node.inverse_scale_factor()
    };
    pick(&mut app, uv);
    let verify = |app: &App, color, roughness: f32, metallic: f32| {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        assert_eq!(session.binding, binding);
        assert_eq!(session.picked, Some(color));
        assert!((session.roughness - roughness).abs() < 0.05);
        assert!((session.metallic - metallic).abs() < 0.05);
    };
    verify(&app, colors[0], 20., 60.);
    paint_message(&mut app, PaintAction::View(View::Three));
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::ZERO;
        viewer.distance = 5.;
        viewer.yaw = 0.;
        viewer.pitch = 0.;
        viewer.orthographic = true;
    }
    warm(&mut app, 4);
    let rect = labelled(&mut app, "3D-вид модели");
    let target = editing::input::project(app.world().resource::<Viewer>(), rect, Vec3::new(0.8, 0., 0.)).unwrap();
    pick(&mut app, target);
    verify(&app, colors[1], 80., 40.);

    paint_message(&mut app, PaintAction::Channel(2));
    for (code, key) in [(KeyCode::KeyC, "c"), (KeyCode::KeyC, "c")] {
        fill_modifier(&mut app, window, code, Key::Character(key.into()), ButtonState::Pressed);
        fill_modifier(&mut app, window, code, Key::Character(key.into()), ButtonState::Released);
    }
    let parts = |app: &mut App| {
        let world = app.world_mut();
        world.query::<(&sets::SetPart, &MeshMaterial3d<StandardMaterial>)>()
            .iter(world).map(|(part, material)| ((part.0, part.1), material.0.clone()))
            .collect::<Vec<_>>()
    };
    let parts_before = parts(&mut app);
    assert_eq!(parts_before.len(), 2);
    wait(&mut app, &|world| parts_before.iter().all(|(key, _)| {
        world.resource::<engine::Runtime>().preview(0, key.0, key.1, 1).is_some()
    }));
    warm(&mut app, 4);
    for (key, handle) in parts(&mut app) {
        let world = app.world();
        let image = world.resource::<engine::Runtime>().preview(0, key.0, key.1, 1).unwrap();
        let material = world.resource::<Assets<StandardMaterial>>().get(&handle).unwrap();
        assert!(material.unlit);
        assert_eq!(material.base_color_texture.as_ref(), Some(&image));
    }
    let session = &app.world().resource::<PaintState>().sessions[&0];
    assert_eq!(session.preview_3d, Preview3d::Channel(1));
    assert_eq!(session.channel, 2);
    assert_eq!(session.binding, binding);
    fill_modifier(&mut app, window, KeyCode::KeyM, Key::Character("m".into()), ButtonState::Pressed);
    fill_modifier(&mut app, window, KeyCode::KeyM, Key::Character("m".into()), ButtonState::Released);
    warm(&mut app, 4);
    for (_, handle) in parts(&mut app) {
        assert!(!app.world().resource::<Assets<StandardMaterial>>().get(&handle).unwrap().unlit);
    }

    paint_message(&mut app, PaintAction::View(View::Two));
    paint_message(&mut app, PaintAction::AddMask(false));
    let session = &app.world().resource::<PaintState>().sessions[&0];
    assert!(session.editing_mask);
    assert_eq!(session.mask_value, 255);
    pick(&mut app, uv);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].mask_value, 0);
    verify(&app, colors[1], 80., 40.);

    let first_stack = app.world().resource::<PaintState>().sessions[&0].stack.clone();
    let first_history = app.world().resource::<PaintState>().sessions[&0].undo.len();
    paint_message(&mut app, PaintAction::View(View::Three));
    paint_message(&mut app, PaintAction::Category(Category::Materials));
    set_shelf_query(&mut app, "Gold");
    let rect = labelled(&mut app, "3D-вид модели");
    let target = editing::input::project(app.world().resource::<Viewer>(), rect, Vec3::new(0.8, 0., 0.)).unwrap();
    drag_shelf_item(&mut app, window, PaintAction::Preset(0), target);
    wait(&mut app, &|world| {
        let session = &world.resource::<PaintState>().sessions[&0];
        session.binding != binding && session.stack.layers.iter().any(|layer| layer.name == "Gold")
    });
    paint_ready(&mut app);
    let second = app.world().resource::<PaintState>().sessions[&0].binding.unwrap();
    let doc = &app.world().resource::<editing::State>().documents[&0];
    let object = doc.objects.iter().find(|object| object.id == second.0).unwrap();
    assert_eq!(object.materials[second.1].udim().unwrap().number(), 1013);
    let first = binding.unwrap();
    assert_eq!(object.materials[first.1].paint.as_ref().unwrap().stack, first_stack);
    paint_message(&mut app, PaintAction::Undo);
    assert!(!app.world().resource::<PaintState>().sessions[&0].stack.layers.iter().any(|layer| layer.name == "Gold"));

    paint_click(&mut app, window, PaintAction::SetMenu);
    let target = paint_button_rect(&mut app, PaintAction::Set(first.1)).center();
    move_pointer(&mut app, window, target);
    warm(&mut app, 3);
    {
        let world = app.world_mut();
        let part = world.query::<(Entity, &sets::SetPart)>().iter(world)
            .find(|(_, part)| (part.0, part.1) == first).unwrap().0;
        let visible = world.query::<(&ChildOf, &Visibility, &MeshMaterial3d<sets::HighlightMaterial>)>()
            .iter(world).any(|(parent, visible, _)| parent.parent() == part && *visible != Visibility::Hidden);
        assert!(visible, "Hovering the layer selector must highlight the matching tile");
    }
    paint_click(&mut app, window, PaintAction::Set(first.1));
    let session = &app.world().resource::<PaintState>().sessions[&0];
    assert_eq!(session.panel, Panel::Layers);
    assert_eq!(session.binding, binding);
    assert_eq!(session.stack, first_stack);
    assert_eq!(session.undo.len(), first_history);
    assert!(!session.set_menu);
}

#[test]
#[ignore = "requires GPU; verifies actual keyboard routing in the paint workspace"]
fn gpu_paint_shortcuts_switch_tools_and_views_without_editing_layers() {
    use bevy::input::mouse::MouseButtonInput;
    let (mut app, window, _) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(
        editing::EditAction::Mode(Mode::Paint),
    )));
    warm(&mut app, 6);
    paint_ready(&mut app);
    let press = |app: &mut App, code, key: Key| {
        fill_modifier(app, window, code, key.clone(), ButtonState::Pressed);
        fill_modifier(app, window, code, key, ButtonState::Released);
        warm(app, 2);
    };
    let tool = |app: &App| app.world().resource::<PaintState>().sessions[&0].tool;
    let undo = app.world().resource::<PaintState>().sessions[&0].undo.len();
    for (code, character, expected) in [
        (KeyCode::Digit2, "2", PaintTool::Eraser),
        (KeyCode::Digit1, "1", PaintTool::Brush),
        (KeyCode::Digit3, "3", PaintTool::Fill),
        (KeyCode::Digit4, "4", PaintTool::Path),
        (KeyCode::KeyI, "i", PaintTool::Picker),
        (KeyCode::KeyP, "p", PaintTool::Fill),
        (KeyCode::KeyL, "l", PaintTool::Path),
        (KeyCode::KeyB, "b", PaintTool::Brush),
        (KeyCode::KeyE, "e", PaintTool::Eraser),
    ] {
        press(&mut app, code, Key::Character(character.into()));
        assert_eq!(tool(&app), expected);
    }
    for (code, key, expected) in [
        (KeyCode::F1, Key::F1, View::Split),
        (KeyCode::F3, Key::F3, View::Two),
        (KeyCode::F2, Key::F2, View::Three),
    ] {
        press(&mut app, code, key);
        assert_eq!(app.world().resource::<PaintState>().sessions[&0].view, expected);
    }
    for expected in [Symmetry::X, Symmetry::Off] {
        press(&mut app, KeyCode::KeyX, Key::Character("x".into()));
        assert_eq!(app.world().resource::<PaintState>().sessions[&0].symmetry, expected);
    }
    for expected in [true, false] {
        press(&mut app, KeyCode::F10, Key::F10);
        assert_eq!(app.world().resource::<PaintState>().sessions[&0].path_trace, expected);
    }
    let wireframe = app.world().resource::<editing::State>().wireframe;
    for expected in [!wireframe, wireframe] {
        press(&mut app, KeyCode::KeyW, Key::Character("w".into()));
        assert_eq!(app.world().resource::<editing::State>().wireframe, expected);
    }
    let distance = app.world().resource::<Viewer>().distance;
    app.world_mut().resource_mut::<Viewer>().distance *= 1.5;
    press(&mut app, KeyCode::KeyF, Key::Character("f".into()));
    assert!((app.world().resource::<Viewer>().distance - distance).abs() < 0.001);

    let field = {
        let world = app.world_mut();
        world.query_filtered::<Entity, With<LayerNameField>>().single(world).unwrap()
    };
    app.world_mut().resource_mut::<InputFocus>().set(field, bevy::input_focus::FocusCause::Auto);
    press(&mut app, KeyCode::Digit1, Key::Character("1".into()));
    assert_eq!(tool(&app), PaintTool::Eraser);
    app.world_mut().resource_mut::<InputFocus>().clear();

    let right = |app: &mut App, state| {
        app.world_mut().write_message(MouseButtonInput { window, button: MouseButton::Right, state });
        frame(app);
    };
    right(&mut app, ButtonState::Pressed);
    press(&mut app, KeyCode::KeyW, Key::Character("w".into()));
    assert_eq!(app.world().resource::<editing::State>().wireframe, wireframe);
    right(&mut app, ButtonState::Released);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].undo.len(), undo);
}

#[test]
#[ignore = "requires GPU; exercises brush adjustment with real viewport input in 3D and UV"]
fn gpu_paint_brush_adjusts_in_3d_and_uv_without_moving_camera() {
    use bevy::input::mouse::MouseButtonInput;
    let (mut app, window, _) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Mode(Mode::Paint))));
    warm(&mut app, 6);
    paint_ready(&mut app);

    fn center<T: Component>(app: &mut App) -> Vec2 {
        let world = app.world_mut();
        let (node, transform) = world.query_filtered::<
            (&ComputedNode, &UiGlobalTransform), With<T>,
        >().single(world).unwrap();
        transform.affine().translation * node.inverse_scale_factor()
    }
    let button = |app: &mut App, state| {
        app.world_mut().write_message(MouseButtonInput { button: MouseButton::Right, state, window });
        frame(app);
    };
    let params = |app: &App| {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        (session.size, session.hardness, session.undo.len())
    };
    for view in [View::Three, View::Two] {
        paint_message(&mut app, PaintAction::View(view));
        let origin = if view == View::Three { center::<Canvas>(&mut app) }
            else { center::<engine::TextureCanvas>(&mut app) };
        let (size, hardness, undo) = params(&app);
        let pose = view_pose(app.world().resource::<Viewer>());
        move_pointer(&mut app, window, origin);
        for (key, character, expected) in [
            (KeyCode::BracketRight, "]", size * 1.15),
            (KeyCode::BracketLeft, "[", size),
        ] {
            fill_modifier(&mut app, window, key, Key::Character(character.into()), ButtonState::Pressed);
            assert!((params(&app).0 - expected).abs() < 0.01);
            fill_modifier(&mut app, window, key, Key::Character(character.into()), ButtonState::Released);
        }
        fill_modifier(&mut app, window, KeyCode::ControlLeft, Key::Control, ButtonState::Pressed);
        button(&mut app, ButtonState::Pressed);
        move_pointer(&mut app, window, origin + Vec2::new(20., 20.));
        let changes = params(&app);
        assert!((changes.0 - size - 20.).abs() < 0.01);
        assert!((changes.1 - (hardness - 10.).max(0.)).abs() < 0.01);
        assert_eq!(changes.2, undo);
        assert_eq!(view_pose(app.world().resource::<Viewer>()), pose);
        if view == View::Three {
            fill_modifier(&mut app, window, KeyCode::Escape, Key::Escape, ButtonState::Pressed);
            fill_modifier(&mut app, window, KeyCode::Escape, Key::Escape, ButtonState::Released);
            assert!((params(&app).0 - size).abs() < 0.01);
            assert_eq!(params(&app).1, hardness);
        }
        button(&mut app, ButtonState::Released);
        if view == View::Two {
            assert_eq!(params(&app), changes);
        }
        fill_modifier(&mut app, window, KeyCode::ControlLeft, Key::Control, ButtonState::Released);
    }
    app.world_mut().resource_mut::<PaintState>().sessions.get_mut(&0).unwrap()
        .set_value(Parameter::Size, 790.);
    fill_modifier(&mut app, window, KeyCode::BracketRight, Key::Character("]".into()), ButtonState::Pressed);
    assert_eq!(params(&app).0, 800.);
}

#[test]
#[ignore = "requires GPU; paints and erases several PBR channels with one stroke and one history entry"]
fn gpu_paint_multi_channel_strokes_are_atomic_in_uv_and_3d() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    use bevy::render::gpu_readback::{Readback, ReadbackComplete};
    let (mut app, window, _) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        let mut mesh = Geometry::default();
        mesh.positions = vec![
            Vec3::new(-1., -1., 0.), Vec3::new(1., -1., 0.),
            Vec3::new(1., 1., 0.), Vec3::new(-1., 1., 0.),
        ];
        mesh.faces.push(Face {
            corners: [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y].into_iter().enumerate()
                .map(|(vertex, uv)| {
                    let mut corner = Corner::new(vertex);
                    corner.uv = uv;
                    corner
                }).collect(),
            material: 0, smooth: false,
        });
        doc.objects[0].geometry = mesh.into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.paint_default_size = 512;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 6);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::View(View::Two));
    paint_message(&mut app, PaintAction::MultiChannelBrush);
    for channel in 3..8 {
        paint_message(&mut app, PaintAction::BrushChannel(channel));
    }
    paint_message(&mut app, PaintAction::Channel(7));
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.size = 42.;
        session.hardness = 100.;
        session.flow = 100.;
        session.opacity = 100.;
        session.picked = Some([120, 50, 220]);
        session.set_value(Parameter::Roughness, 40.);
        session.set_value(Parameter::Metallic, 60.);
        session.set_value(Parameter::BrushHeight, 20.);
        session.set_value(Parameter::BrushAlpha, 35.);
        session.set_value(Parameter::BrushSpecular, 80.);
        for (i, value) in [20., 60., 100.].into_iter().enumerate() {
            session.set_value(Parameter::BrushEmission(i), value);
        }
        assert_eq!(session.stroke_channels(), 255);
    }
    fn uv_rect<T: Component>(app: &mut App) -> Rect {
        let world = app.world_mut();
        let (node, transform) = world.query_filtered::<
            (&ComputedNode, &UiGlobalTransform), With<T>,
        >().single(world).unwrap();
        Rect::from_center_size(
            transform.affine().translation * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        )
    }
    let fitted = uv_rect::<engine::TextureCanvas>(&mut app);
    assert!((fitted.width() - fitted.height()).abs() < 0.01);
    let anchor = fitted.center() + Vec2::new(26., -16.);
    let anchor_uv = (anchor - fitted.min) / fitted.size();
    move_pointer(&mut app, window, anchor);
    app.world_mut().write_message(bevy::input::mouse::MouseWheel {
        window, unit: bevy::input::mouse::MouseScrollUnit::Line, x: 0., y: 6.,
        phase: bevy::input::touch::TouchPhase::Moved,
    });
    warm(&mut app, 4);
    let zoomed = uv_rect::<engine::TextureCanvas>(&mut app);
    assert!(zoomed.width() > fitted.width() * 2.);
    assert!((zoomed.min + anchor_uv * zoomed.size()).distance(anchor) < 1.5);
    let button = |app: &mut App, state| {
        app.world_mut().write_message(bevy::input::mouse::MouseButtonInput {
            window, button: MouseButton::Right, state,
        });
        frame(app);
    };
    let delta = Vec2::new(35., 23.);
    button(&mut app, ButtonState::Pressed);
    move_pointer(&mut app, window, anchor + delta);
    button(&mut app, ButtonState::Released);
    let panned = uv_rect::<engine::TextureCanvas>(&mut app);
    assert!(panned.center().distance(zoomed.center() + delta) < 1.5);
    let clip = uv_rect::<engine::TextureViewport>(&mut app);
    // The 29 px button immediately above the canvas opens UV editing. Use the
    // static title area to check a clipped texel without triggering a control.
    let clipped_point = Vec2::new(clip.center().x, clip.min.y - 40.);
    assert!(panned.contains(clipped_point));
    let stroke = |app: &mut App, point| {
        move_pointer(app, window, point);
        mouse(app, window, ButtonState::Pressed);
        mouse(app, window, ButtonState::Released);
        paint_ready(app);
    };
    let current = |app: &App| {
        app.world().resource::<editing::State>().documents[&0].objects[0].materials[0].paint.clone()
    };
    let undo_len = |app: &App| app.world().resource::<PaintState>().sessions[&0].undo.len();
    let before_undo = undo_len(&app);
    let readbacks = app.world().resource::<engine::gpu::Queue>().statistics().2;
    stroke(&mut app, clipped_point);
    assert_eq!(app.world().resource::<editing::State>().mode(Some(0)), Mode::Paint);
    assert_eq!(undo_len(&app), before_undo);
    stroke(&mut app, panned.center());
    assert_eq!(undo_len(&app), before_undo + 1);
    assert_eq!(app.world().resource::<engine::gpu::Queue>().statistics().2, readbacks + 1);
    let expected = [
        [120, 50, 220], [102; 3], [153; 3], [51; 3], [89; 3],
        [51, 153, 255], [128, 128, 255], [204; 3],
    ];
    let center = (256 * 512 + 256) * 4;
    let side = (256 * 512 + 128) * 4;
    let verify = |app: &App, offset| {
        let surface = current(app).unwrap();
        for (channel, expected) in expected.iter().enumerate() {
            let pixels = surface.stack.layers[0].channels[channel].as_ref().unwrap();
            assert_eq!(&pixels[offset..offset + 3], expected, "channel {channel}");
            assert_eq!(pixels[offset + 3], 255, "coverage of channel {channel}");
        }
    };
    verify(&app, center);
    let material_maps = {
        let world = app.world();
        let id = &world.resource::<editing::State>().documents[&0].objects[0].materials[0].material;
        let material = world.resource::<Assets<StandardMaterial>>().get(id).unwrap();
        assert_eq!(material.alpha_mode, AlphaMode::Blend);
        [material.base_color_texture.clone().unwrap(), material.metallic_roughness_texture.clone().unwrap()]
    };
    let read = |app: &mut App, image, offset: usize| {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        app.world_mut().spawn(Readback::texture(image))
            .observe(move |event: On<ReadbackComplete>, mut commands: Commands| {
                let pixel: [u8; 4] = event.data[offset..offset + 4].try_into().unwrap();
                let _ = send.try_send(pixel);
                commands.entity(event.entity).try_despawn();
            });
        let now = std::time::Instant::now();
        loop {
            frame(app);
            if let Ok(pixel) = receive.try_recv() {
                break pixel;
            }
            assert!(now.elapsed().as_secs() < 15, "No composited GPU texture");
        }
    };
    assert_eq!(read(&mut app, material_maps[0].clone(), center), [120, 50, 220, 89]);
    assert_eq!(read(&mut app, material_maps[1].clone(), center), [255, 102, 153, 255]);
    paint_message(&mut app, PaintAction::Undo);
    assert!(current(&app).is_none_or(|surface| surface.stack.layers[0].channels.iter().all(Option::is_none)));
    paint_message(&mut app, PaintAction::Redo);
    verify(&app, center);
    fill_modifier(&mut app, window, KeyCode::KeyF, Key::Character("f".into()), ButtonState::Pressed);
    fill_modifier(&mut app, window, KeyCode::KeyF, Key::Character("f".into()), ButtonState::Released);
    warm(&mut app, 3);
    let reset = uv_rect::<engine::TextureCanvas>(&mut app);
    assert!(reset.size().distance(fitted.size()) < 1. && reset.center().distance(fitted.center()) < 1.);

    paint_message(&mut app, PaintAction::View(View::Three));
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::ZERO;
        viewer.distance = 4.;
        viewer.yaw = 0.;
        viewer.pitch = 0.;
        viewer.orthographic = true;
    }
    warm(&mut app, 4);
    let rect = labelled(&mut app, "3D-вид модели");
    let point = editing::input::project(app.world().resource::<Viewer>(), rect, Vec3::new(-0.5, 0., 0.)).unwrap();
    stroke(&mut app, point);
    verify(&app, side);
    paint_message(&mut app, PaintAction::Tool(PaintTool::Eraser));
    stroke(&mut app, point);
    let erased = current(&app).unwrap();
    assert!(erased.stack.layers[0].channels.iter().flatten().all(|pixels| pixels[side + 3] == 0));
    verify(&app, center);
    paint_message(&mut app, PaintAction::Undo);
    verify(&app, side);

    paint_message(&mut app, PaintAction::AddMask(false));
    paint_message(&mut app, PaintAction::Tool(PaintTool::Brush));
    let before_mask = current(&app).unwrap();
    stroke(&mut app, point);
    let with_mask = current(&app).unwrap();
    assert_eq!(before_mask.stack.layers[0].channels, with_mask.stack.layers[0].channels);
    assert_eq!(with_mask.stack.layers[0].mask.as_ref().unwrap().pixels[side], 255);

    fn thumbnail_pixels(app: &mut App, action: PaintAction, ready: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            frame(app);
            let world = app.world_mut();
            let button = world.query::<(Entity, &Control)>().iter(world).find_map(|(entity, control)| {
                matches!(control.action, Action::ModelView(ModelViewAction::Paint(value)) if value == action)
                    .then_some(entity)
            }).unwrap();
            let image = world.get::<Children>(button).unwrap().iter()
                .filter_map(|child| world.get::<Children>(child)).flat_map(|children| children.iter())
                .find_map(|child| world.get::<ImageNode>(child))
                .and_then(|node| world.resource::<Assets<Image>>().get(&node.image));
            if let Some(image) = image {
                assert_eq!(image.size(), UVec2::splat(64));
                if let Some(pixels) = image.data.as_ref().filter(|pixels| ready(pixels)) {
                    return pixels.clone();
                }
            }
            assert!(std::time::Instant::now() < deadline, "No thumbnail for {action:?}");
        }
    }
    let raw_color = thumbnail_pixels(&mut app, PaintAction::Select(0), |_| true);
    let thumb_center = (32 * 64 + 32) * 4;
    assert_eq!(&raw_color[thumb_center..thumb_center + 3], &[120, 50, 220]);
    assert!(raw_color[thumb_center + 3] > 0);
    assert_eq!(raw_color[3], 0);
    let thumb_side = (32 * 64 + 16) * 4;
    let mask = thumbnail_pixels(&mut app, PaintAction::SelectLayerMask(0), |pixels| pixels[thumb_side] > 0);
    assert_eq!(&mask[..4], &[0, 0, 0, 255]);
    let value = mask[thumb_side];
    assert!(value > 0);
    assert_eq!(&mask[thumb_side..thumb_side + 4], &[value, value, value, 255]);
    paint_message(&mut app, PaintAction::Channel(0));
    paint_click(&mut app, window, PaintAction::SelectLayerMask(0));
    assert!(app.world().resource::<PaintState>().sessions[&0].editing_mask);

    let history = undo_len(&app);
    paint_click(&mut app, window, PaintAction::ClearLayer);
    let cleared = current(&app).unwrap();
    let mut expected_stack = with_mask.stack.clone();
    expected_stack.layers[0].channels = Default::default();
    assert_eq!(cleared.stack, expected_stack);
    assert_eq!(undo_len(&app), history + 1);
    assert!(!app.world().resource::<PaintState>().sessions[&0].available(PaintAction::ClearLayer));
    let color = cleared.composite(0);
    let opacity = cleared.composite(4);
    assert_eq!(read(&mut app, material_maps[0].clone(), center), [
        color[center], color[center + 1], color[center + 2], opacity[center],
    ]);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(current(&app).unwrap().stack, with_mask.stack);
    assert_eq!(thumbnail_pixels(&mut app, PaintAction::Select(0), |pixels| pixels == raw_color), raw_color);
    paint_message(&mut app, PaintAction::Redo);
    assert_eq!(current(&app).unwrap().stack, expected_stack);
}

#[test]
#[ignore = "requires GPU; verifies all PBR channels, occlusion, opacity, erasing, fill and orthographic painting"]
fn gpu_model_paint_channels_erase_and_occlusion() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    let mut mesh = Geometry::default();
    for (z, left, right) in [(0., 0.05, 0.45), (-1., 0.451, 0.851)] {
        let start = mesh.positions.len();
        mesh.positions.extend([
            Vec3::new(-1., -1., z),
            Vec3::new(1., -1., z),
            Vec3::new(1., 1., z),
            Vec3::new(-1., 1., z),
        ]);
        let uv = [
            Vec2::new(left, 0.05),
            Vec2::new(right, 0.05),
            Vec2::new(right, 0.95),
            Vec2::new(left, 0.95),
        ];
        mesh.faces.push(Face {
            corners: (0..4)
                .map(|i| {
                    let mut c = Corner::new(start + i);
                    c.uv = uv[i];
                    c
                })
                .collect(),
            material: 0,
            smooth: false,
        });
    }
    {
        let mut state = app.world_mut().resource_mut::<editing::State>();
        let doc = state.documents.get_mut(&0).unwrap();
        doc.objects[0].geometry = mesh.into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::ZERO;
        viewer.distance = 4.;
        viewer.radius = 2.;
        viewer.yaw = 0.;
        viewer.pitch = 0.;
    }
    warm(&mut app, 5);
    paint_ready(&mut app);
    let center = labelled(&mut app, "3D-вид модели").center();
    // The mesh material pass can consume the finished import on the next frame.
    frame(&mut app);
    let imported_material = {
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        let object = &doc.objects[0];
        assert!(object.materials[0].paint.is_none());
        world.resource::<engine::Runtime>()
            .working_material(0, object.id, 0).unwrap().clone()
    };
    assert!(
        app.world_mut().query::<(&sets::SetPart, &MeshMaterial3d<StandardMaterial>)>()
            .iter(app.world()).any(|(_, material)| material.0 == imported_material),
        "the baked import is shown in 3D before the first stroke"
    );
    for channel in 0..8 {
        {
            let mut paint = app.world_mut().resource_mut::<PaintState>();
            let s = paint.sessions.get_mut(&0).unwrap();
            s.channel = channel;
            s.color = 3;
            s.opacity = if channel == 4 { 0. } else { 100. };
            s.roughness = 80.;
            s.metallic = 60.;
            s.flow = 100.;
            s.hardness = 100.;
            s.size = 90.;
            s.tool = PaintTool::Brush;
        }
        move_pointer(&mut app, window, center);
        mouse(&mut app, window, ButtonState::Pressed);
        for _ in 0..20 {
            frame(&mut app);
        }
        if channel == 0 {
            assert_eq!(
                app.world().resource::<engine::gpu::Queue>().statistics().2,
                0
            );
            let uv = {
                let w = app.world_mut();
                let(n,t)=w.query_filtered::<(&ComputedNode,&UiGlobalTransform),With<engine::TextureCanvas>>().single(w).unwrap();
                Rect::from_center_size(
                    t.affine().translation * n.inverse_scale_factor(),
                    n.size() * n.inverse_scale_factor(),
                )
            };
            let point = uv.min + uv.size() * Vec2::new(0.25, 0.5);
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/model-paint-qa/07-gpu-live-before-readback.png");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            capture_when(&mut app, &target, &path, |image| {
                let p = image.get_pixel(point.x as u32, point.y as u32);
                p[0].abs_diff(164) < 4 && p[1].abs_diff(61) < 4 && p[2].abs_diff(68) < 4
            });
            assert!(
                app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
                    .paint
                    .is_none(),
                "live color must be visible before a CPU snapshot exists"
            );
        }
        if channel == 7 {
            use bevy::render::gpu_readback::{Readback, ReadbackComplete};
            let doc = &app.world().resource::<editing::State>().documents[&0];
            let slot = &doc.objects[0].materials[0];
            assert!(slot.paint.as_ref().unwrap().stack.layers[0].channels[7].is_none());
            let materials = app.world().resource::<Assets<StandardMaterial>>();
            let material = materials.get(&slot.material).unwrap();
            assert_eq!(material.reflectance, 2.);
            let texture = material.specular_texture.clone().unwrap();
            let (send, receive) = std::sync::mpsc::sync_channel(1);
            app.world_mut()
                .spawn(Readback::texture(texture))
                .observe(move |mut event: On<ReadbackComplete>, mut commands: Commands| {
                    let _ = send.try_send(std::mem::take(&mut event.event_mut().data));
                    commands.entity(event.entity).try_despawn();
                });
            let start = std::time::Instant::now();
            let pixels = loop {
                frame(&mut app);
                if let Ok(pixels) = receive.try_recv() {
                    break pixels;
                }
                assert!(start.elapsed().as_secs() < 15, "missing GPU specular map");
            };
            let stride = 1024 * 4;
            assert_eq!(pixels[512 * stride + 256 * 4 + 3], 180);
            assert_eq!(pixels[512 * stride + 768 * 4 + 3], 128);
        }
        mouse(&mut app, window, ButtonState::Released);
        paint_ready(&mut app);
        let surface = app.world().resource::<editing::State>().documents[&0].objects[0].materials
            [0]
        .paint
        .clone()
        .unwrap();
        let p = surface.stack.layers[0].channels[channel].as_ref().unwrap();
        let size = surface.size as usize;
        let front = (size / 2 * size + size / 4) * 4;
        let back = (size / 2 * size + size * 3 / 4) * 4;
        let expected = match channel {
            0 | 5 => [164, 61, 68, 255],
            1 => [204, 204, 204, 255],
            2 => [153, 153, 153, 255],
            4 => [0, 0, 0, 255],
            6 => [128, 128, 255, 255],
            _ => [255; 4],
        };
        assert_eq!(&p[front..front + 4], &expected, "GPU channel {channel}");
        assert_eq!(p[back + 3], 0, "GPU stroke must not paint the rear plane");
    }
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Paint(
            PaintAction::Channel(0),
        )));
    warm(&mut app, 2);
    paint_ready(&mut app);
    paint_click(&mut app, window, PaintAction::Tool(PaintTool::Eraser));
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    let pixel = |app: &App, x: usize, y: usize| {
        let surface = app.world().resource::<editing::State>().documents[&0].objects[0].materials
            [0]
        .paint
        .as_ref()
        .unwrap();
        let i = (y * surface.size as usize + x) * 4;
        surface.stack.layers[0].channels[0].as_ref().unwrap()[i..i + 4].to_vec()
    };
    assert_eq!(pixel(&app, 256, 512)[3], 0);
    paint_click(&mut app, window, PaintAction::Undo);
    assert_eq!(pixel(&app, 256, 512)[3], 255);
    paint_click(&mut app, window, PaintAction::Color(4));
    paint_click(&mut app, window, PaintAction::Tool(PaintTool::Fill));
    paint_click(&mut app, window, PaintAction::FillMode(FillMode::Triangle));
    let uv_rect = {
        let world = app.world_mut();
        let (node, transform) = world
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<engine::TextureCanvas>>()
            .single(world)
            .unwrap();
        let center = transform.affine().translation * node.inverse_scale_factor();
        Rect::from_center_size(center, node.size() * node.inverse_scale_factor())
    };
    move_pointer(&mut app, window, uv_rect.min + uv_rect.size() * Vec2::new(0.11, 0.31));
    assert!((0..90).any(|_| {
        frame(&mut app);
        let world = app.world();
        world.resource::<engine::fill_preview::Preview>().uv_lines()
            .iter()
            .filter(|&&entity| world.get::<Node>(entity).is_some_and(|n| n.display == Display::Flex))
            .count() == 3
    }), "The UV triangle needs a visible three-edge hover preview");
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(pixel(&app, 113, 317), [67, 118, 166, 255]);
    assert_eq!(pixel(&app, 399, 707)[3], 0);

    paint_message(&mut app, PaintAction::FillMode(FillMode::Part));
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(pixel(&app, 399, 707), [67, 118, 166, 255]);
    assert_eq!(pixel(&app, 768, 512)[3], 0);

    paint_message(&mut app, PaintAction::FillMode(FillMode::Object));
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(pixel(&app, 768, 512), [67, 118, 166, 255]);
    paint_message(&mut app, PaintAction::Undo);
    paint_message(&mut app, PaintAction::FillMode(FillMode::Polygon));
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(pixel(&app, 103, 103), [67, 118, 166, 255]);
    assert_eq!(pixel(&app, 768, 512)[3], 0);
    assert_eq!(pixel(&app, 50, 512), [67, 118, 166, 255], "UV padding covers empty texels");
    assert_eq!(pixel(&app, 48, 512)[3], 0, "padding stays within two texels");
    assert_eq!(pixel(&app, 461, 512)[3], 255, "padding fills the gap between charts");
    assert_eq!(pixel(&app, 462, 512)[3], 0, "padding preserves the neighboring chart");
    fill_modifier(&mut app, window, KeyCode::ControlLeft, Key::Control, ButtonState::Pressed);
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    fill_modifier(&mut app, window, KeyCode::ControlLeft, Key::Control, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(pixel(&app, 103, 103)[3], 0, "Ctrl temporarily erases the face");
    assert_eq!(pixel(&app, 50, 512)[3], 0, "erase removes the same UV padding");
    paint_message(&mut app, PaintAction::Undo);
    let box_points = [center - Vec2::splat(80.), center + Vec2::splat(80.)];
    paint_message(&mut app, PaintAction::FillErase);
    shift_fill_drag(&mut app, window, &box_points);
    assert_eq!(pixel(&app, 103, 103)[3], 0, "rectangle erase uses the same face selection");
    paint_message(&mut app, PaintAction::Undo);
    paint_message(&mut app, PaintAction::FillErase);
    shift_fill_drag(&mut app, window, &box_points);
    assert_eq!(pixel(&app, 768, 512)[3], 0);
    paint_message(&mut app, PaintAction::FillThrough);
    shift_fill_drag(&mut app, window, &box_points);
    assert_eq!(pixel(&app, 768, 512), [67, 118, 166, 255]);
    paint_message(&mut app, PaintAction::Undo);
    paint_message(&mut app, PaintAction::FillThrough);
    paint_click(&mut app, window, PaintAction::FillLasso);
    let lasso = [
        Vec2::new(0.58, 0.15),
        Vec2::new(0.92, 0.15),
        Vec2::new(0.92, 0.85),
        Vec2::new(0.58, 0.85),
    ].map(|p| uv_rect.min + uv_rect.size() * p);
    shift_fill_drag(&mut app, window, &lasso);
    assert_eq!(pixel(&app, 768, 512), [67, 118, 166, 255]);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(pixel(&app, 768, 512)[3], 0);
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.orthographic = true;
        viewer.distance = 0.05;
    }
    paint_click(&mut app, window, PaintAction::Tool(PaintTool::Brush));
    paint_click(&mut app, window, PaintAction::Color(3));
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(pixel(&app, 256, 512), [164, 61, 68, 255]);
    paint_click(&mut app, window, PaintAction::Add(LayerKind::Paint));
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let s = paint.sessions.get_mut(&0).unwrap();
        s.opacity = 50.;
        s.flow = 100.;
        s.hardness = 100.;
    }
    // Queue two strokes without waiting for the first readback: GPU snapshots
    // and CPU undo records must retain their submission order.
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    let alpha = |app: &App| {
        let surface = app.world().resource::<editing::State>().documents[&0].objects[0].materials
            [0]
        .paint
        .as_ref()
        .unwrap();
        let i = ((surface.size / 2) * surface.size + surface.size / 4) as usize * 4 + 3;
        surface.stack.layers[1].channels[0].as_ref().unwrap()[i]
    };
    assert_eq!(
        alpha(&app),
        192,
        "second stroke must build on the first GPU layer snapshot"
    );
    paint_click(&mut app, window, PaintAction::Undo);
    assert_eq!(alpha(&app), 128, "one undo record per completed GPU stroke");

    paint_click(&mut app, window, PaintAction::Category(Category::Brushes));
    let ring = app
        .world()
        .resource::<PaintState>()
        .tips
        .entries
        .iter()
        .position(|tip| tip.name == "Кольцо")
        .unwrap();
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Paint(
            PaintAction::BrushTip(Some(ring)),
        )));
    warm(&mut app, 4);
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.orthographic = false;
        viewer.distance = 4.;
    }
    app.world_mut()
        .resource_mut::<PaintState>()
        .sessions
        .get_mut(&0)
        .unwrap()
        .opacity = 100.;
    let uv = {
        let w = app.world_mut();
        let (n, t) = w
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<engine::TextureCanvas>>()
            .single(w)
            .unwrap();
        Rect::from_center_size(
            t.affine().translation * n.inverse_scale_factor(),
            n.size() * n.inverse_scale_factor(),
        )
    };
    for point in [center, uv.min + uv.size() * Vec2::new(0.25, 0.5)] {
        paint_click(&mut app, window, PaintAction::Add(LayerKind::Paint));
        move_pointer(&mut app, window, point);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
        paint_ready(&mut app);
        let surface = app.world().resource::<editing::State>().documents[&0].objects[0].materials
            [0]
        .paint
        .as_ref()
        .unwrap();
        let pixels = surface.stack.layers[surface.stack.selected].channels[0]
            .as_ref()
            .unwrap();
        let row = surface.size as usize / 2 * surface.size as usize;
        let x = surface.size as usize / 4;
        assert_eq!(
            pixels[(row + x) * 4 + 3],
            0,
            "ring must keep a hole at the stamp center"
        );
        assert!(
            (x + 8..x + 175).any(|x| pixels[(row + x) * 4 + 3] > 180),
            "ring must paint around the center"
        );
    }

    use bevy::{
        input::touch::{ForceTouch, TouchInput, TouchPhase},
        window::WindowEvent,
    };
    paint_message(&mut app, PaintAction::BrushTip(None));
    paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
    paint_message(&mut app, PaintAction::PressureFlow);
    for (id, y, force) in [(41, 0.33, 0.25), (42, 0.67, 1.)] {
        let position = uv.min + uv.size() * Vec2::new(0.25, y);
        for phase in [TouchPhase::Started, TouchPhase::Ended] {
            app.world_mut()
                .write_message(WindowEvent::TouchInput(TouchInput {
                    phase,
                    position,
                    window,
                    force: (phase == TouchPhase::Started).then_some(ForceTouch::Normalized(force)),
                    id,
                }));
        }
        paint_ready(&mut app);
    }
    let brush_alpha = |app: &App, x: usize, y: usize| {
        let p = app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
            .paint
            .as_ref()
            .unwrap();
        let pixels = p.stack.layers[p.stack.selected].channels[0]
            .as_ref()
            .unwrap();
        pixels[(y * p.size as usize + x) * 4 + 3]
    };
    assert!(
        brush_alpha(&app, 256, 337).abs_diff(64) <= 1,
        "pen force controls per-stamp flow"
    );
    assert_eq!(brush_alpha(&app, 256, 686), 255);
    assert_eq!(
        brush_alpha(&app, 300, 337),
        0,
        "low pressure shrinks the stamp"
    );
    assert!(
        brush_alpha(&app, 300, 686) > 200,
        "full pressure keeps the normal diameter"
    );
    paint_message(&mut app, PaintAction::Undo);
    assert!(brush_alpha(&app, 256, 337).abs_diff(64) <= 1);
    assert_eq!(
        brush_alpha(&app, 256, 686),
        0,
        "undo removes exactly one pen contact"
    );

    paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
    paint_message(&mut app, PaintAction::Symmetry(Symmetry::X));
    app.world_mut()
        .resource_mut::<PaintState>()
        .sessions
        .get_mut(&0)
        .unwrap()
        .size = 32.;
    let view = labelled(&mut app, "3D-вид модели");
    let viewer = app.world().resource::<Viewer>();
    let left = editing::input::project(viewer, view, Vec3::new(-0.55, 0., 0.)).unwrap();
    move_pointer(&mut app, window, left);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(
        brush_alpha(&app, 143, 512),
        255,
        "source of mirrored stroke"
    );
    assert_eq!(
        brush_alpha(&app, 369, 512),
        255,
        "reflected stroke on the visible plane"
    );
    assert_eq!(
        brush_alpha(&app, 655, 512),
        0,
        "occluded source stays untouched"
    );
    assert_eq!(
        brush_alpha(&app, 881, 512),
        0,
        "occluded mirror stays untouched"
    );
    assert_eq!(brush_alpha(&app, 256, 512), 0);

    paint_message(&mut app, PaintAction::Symmetry(Symmetry::Off));
    paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
    paint_message(&mut app, PaintAction::Category(Category::Textures));
    let grain = app
        .world()
        .resource::<PaintState>()
        .tips
        .grains
        .iter()
        .position(|tip| tip.name == "Шахматная клетка")
        .unwrap();
    paint_message(&mut app, PaintAction::Grain(Some(grain)));
    paint_message(&mut app, PaintAction::GrainUv);
    app.world_mut()
        .resource_mut::<PaintState>()
        .sessions
        .get_mut(&0)
        .unwrap()
        .size = 90.;
    let point = uv.min + uv.size() * Vec2::new(0.25, 0.5);
    move_pointer(&mut app, window, point);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert!(
        brush_alpha(&app, 271, 527).abs_diff(217) <= 3,
        "grain keeps its light UV cell"
    );
    assert!(
        brush_alpha(&app, 303, 527).abs_diff(38) <= 3,
        "grain keeps its dark UV cell"
    );
    paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
    paint_message(&mut app, PaintAction::GrainUv);
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert!((227..284).any(|x| brush_alpha(&app, x, 517) > 190));
    assert!((227..284).any(|x| (30..80).contains(&brush_alpha(&app, x, 517))));

    paint_message(&mut app, PaintAction::Grain(None));
    let px_per_unit = {
        let viewer = app.world().resource::<Viewer>();
        let corner = editing::input::project(viewer, view, Vec3::X).unwrap();
        corner.distance(center)
    };
    let delta = (0.26 * (45. / px_per_unit) * 1024.).round() as usize;
    for mode in [BrushProjection::Surface, BrushProjection::Uv] {
        paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
        paint_message(&mut app, PaintAction::BrushProjection(mode));
        move_pointer(&mut app, window, center);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
        paint_ready(&mut app);
        assert_eq!(brush_alpha(&app, 256, 512), 255);
        assert_eq!(brush_alpha(&app, 256, 512 + delta), 255);
        assert_eq!(
            brush_alpha(&app, 256 + delta, 512),
            if mode == BrushProjection::Uv { 255 } else { 0 }
        );
        assert_eq!(brush_alpha(&app, 768, 512), 0);
    }
    paint_message(&mut app, PaintAction::Symmetry(Symmetry::X));
    app.world_mut()
        .resource_mut::<PaintState>()
        .sessions
        .get_mut(&0)
        .unwrap()
        .size = 32.;
    for mode in [BrushProjection::Surface, BrushProjection::Uv] {
        paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
        paint_message(&mut app, PaintAction::BrushProjection(mode));
        move_pointer(&mut app, window, left);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
        paint_ready(&mut app);
        assert_eq!(brush_alpha(&app, 143, 512), 255);
        assert_eq!(brush_alpha(&app, 369, 512), 255, "mirrored {mode:?} stamp");
        assert_eq!(brush_alpha(&app, 655, 512), 0);
    }
    paint_message(&mut app, PaintAction::Symmetry(Symmetry::Off));
    paint_message(
        &mut app,
        PaintAction::BrushProjection(BrushProjection::Screen),
    );
    paint_message(&mut app, PaintAction::PaintThrough);
    paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(brush_alpha(&app, 256, 512), 255);
    assert_eq!(
        brush_alpha(&app, 667, 512),
        255,
        "paint through covers the hidden plane"
    );

    paint_message(&mut app, PaintAction::PaintThrough);
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.size = 20.;
        session.stabilizer = 120.;
    }
    let uv_left = uv.min + uv.size() * Vec2::new(0.14, 0.5);
    let uv_right = uv.min + uv.size() * Vec2::new(0.36, 0.5);
    let right = {
        let viewer = app.world().resource::<Viewer>();
        editing::input::project(viewer, view, Vec3::new(0.55, 0., 0.)).unwrap()
    };
    for (start, end) in [(left, right), (uv_left, uv_right)] {
        paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
        move_pointer(&mut app, window, start);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
        paint_ready(&mut app);
        assert_eq!(brush_alpha(&app, 256, 512), 0);
        fill_modifier(&mut app, window, KeyCode::ShiftLeft, Key::Shift, ButtonState::Pressed);
        move_pointer(&mut app, window, end);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
        fill_modifier(&mut app, window, KeyCode::ShiftLeft, Key::Shift, ButtonState::Released);
        paint_ready(&mut app);
        assert_eq!(brush_alpha(&app, 256, 512), 255, "line reaches the midpoint");
        assert_eq!(brush_alpha(&app, 369, 512), 255, "line reaches the Shift-click endpoint");
        paint_message(&mut app, PaintAction::Undo);
        assert_eq!(brush_alpha(&app, 256, 512), 0, "one undo clears the line");
        assert_eq!(brush_alpha(&app, 143, 512), 255, "undo keeps the previous dab");
    }

    let coverage = |app: &App| {
        let surface = app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
            .paint.as_ref().unwrap();
        surface.stack.layers[surface.stack.selected].channels[0].as_ref().map_or(0, |pixels| {
            pixels.chunks_exact(4).filter(|p| p[3] > 0).count()
        })
    };
    let mut filled = [0; 2];
    for (i, variation) in [0., 100.].into_iter().enumerate() {
        paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
        app.world_mut().resource_mut::<PaintState>().sessions.get_mut(&0).unwrap()
            .set_value(Parameter::BrushSizeJitter, variation);
        move_pointer(&mut app, window, uv.center() - Vec2::X * uv.width() * 0.25);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
        paint_ready(&mut app);
        filled[i] = coverage(&app);
    }
    assert!(filled[1] > 0 && filled[1] < filled[0], "size variation reduces the GPU footprint");

    paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
    app.world_mut().resource_mut::<PaintState>().sessions.get_mut(&0).unwrap()
        .set_value(Parameter::BrushSizeJitter, 0.);
    paint_click(&mut app, window, PaintAction::Tool(PaintTool::Path));
    let uv_path_points = [Vec2::new(0.14, 0.35), Vec2::new(0.36, 0.35), Vec2::new(0.36, 0.55)]
        .map(|uv_point| uv.min + uv.size() * uv_point);
    for point in &uv_path_points[..2] {
        move_pointer(&mut app, window, *point);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
    }
    move_pointer(&mut app, window, uv_path_points[1]);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, uv_path_points[2]);
    mouse(&mut app, window, ButtonState::Released);
    // Winit also emits the aggregate event when the mouse takes over after a pen contact.
    app.world_mut().write_message(WindowEvent::CursorMoved(bevy::window::CursorMoved {
        window,
        position: uv_path_points[2],
        delta: None,
    }));
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].path_count, 2);
    assert_eq!(coverage(&app), 0, "editing path points leaves the layer empty");
    assert!((0..8).any(|_| {
        frame(&mut app);
        let world = app.world();
        world.resource::<engine::fill_preview::Preview>().path_dabs().iter()
            .any(|&entity| world.get::<Node>(entity).is_some_and(|n| n.width == px(20) && n.display == Display::Flex))
    }), "the editable path previews brush imprints");
    {
        let world = app.world();
        let markers = world.resource::<engine::fill_preview::Preview>().path_points();
        assert_eq!(markers.len(), 2);
        assert_eq!(world.get::<BackgroundColor>(markers[0]).unwrap().0, Color::WHITE);
        assert_ne!(world.get::<BackgroundColor>(markers[1]).unwrap().0, Color::WHITE);
        assert_eq!(world.get::<Node>(markers[0]).unwrap().width, px(10));
        assert_eq!(world.get::<Node>(markers[1]).unwrap().width, px(13));
    }
    fill_modifier(&mut app, window, KeyCode::Enter, Key::Enter, ButtonState::Pressed);
    fill_modifier(&mut app, window, KeyCode::Enter, Key::Enter, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(brush_alpha(&app, 256, 461), 255, "a dragged path point changes the applied UV stroke");
    assert_eq!(brush_alpha(&app, 256, 358), 0);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(coverage(&app), 0);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].path_count, 2, "undo retains the editable curve");
    fill_modifier(&mut app, window, KeyCode::Backspace, Key::Backspace, ButtonState::Pressed);
    fill_modifier(&mut app, window, KeyCode::Backspace, Key::Backspace, ButtonState::Released);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].path_count, 1);

    for point in [left, right] {
        move_pointer(&mut app, window, point);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
    }
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].path_count, 2);
    paint_click(&mut app, window, PaintAction::PathApply);
    assert_eq!(brush_alpha(&app, 256, 512), 255, "3D control points apply on the model");
    paint_message(&mut app, PaintAction::PathClear);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].path_count, 0);
    assert_eq!(brush_alpha(&app, 256, 512), 255);
}
#[test]
#[ignore = "GPU throughput benchmark on 40k triangles; no readback during the held stroke"]
fn gpu_model_paint_benchmark_40k() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    use bevy::render::{RenderApp, render_resource::PollType, renderer::RenderDevice};
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (mut app, window, _) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    let mut mesh = Geometry::default();
    let nx = 100;
    let ny = 200;
    for y in 0..ny {
        for x in 0..nx {
            let start = mesh.positions.len();
            let uv = [
                Vec2::new(x as f32 / nx as f32, y as f32 / ny as f32),
                Vec2::new((x + 1) as f32 / nx as f32, y as f32 / ny as f32),
                Vec2::new((x + 1) as f32 / nx as f32, (y + 1) as f32 / ny as f32),
                Vec2::new(x as f32 / nx as f32, (y + 1) as f32 / ny as f32),
            ];
            mesh.positions
                .extend(uv.map(|p| Vec3::new(p.x * 2. - 1., p.y * 2. - 1., 0.)));
            mesh.faces.push(Face {
                corners: (0..4)
                    .map(|i| {
                        let mut c = Corner::new(start + i);
                        c.uv = uv[i];
                        c
                    })
                    .collect(),
                material: 0,
                smooth: false,
            });
        }
    }
    {
        let mut state = app.world_mut().resource_mut::<editing::State>();
        let doc = state.documents.get_mut(&0).unwrap();
        doc.objects[0].geometry = mesh.into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 5);
    paint_ready(&mut app);
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::ZERO;
        viewer.distance = 4.;
        viewer.radius = 2.;
        viewer.yaw = 0.;
        viewer.pitch = 0.;
    }
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let s = paint.sessions.get_mut(&0).unwrap();
        s.size = 80.;
        s.flow = 35.;
        s.hardness = 60.;
        s.color = 3;
    }
    warm(&mut app, 3);
    let center = labelled(&mut app, "3D-вид модели").center();
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    let warmup = std::time::Instant::now();
    loop {
        frame(&mut app);
        if app.world().resource::<engine::Runtime>().worker_idle(0) {
            break;
        }
        assert!(warmup.elapsed().as_secs() < 30);
    }
    let wait = |app: &mut App| {
        app.sub_app(RenderApp)
            .world()
            .resource::<RenderDevice>()
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
    };
    wait(&mut app);
    let baseline_start = std::time::Instant::now();
    for _ in 0..120 {
        app.update();
    }
    wait(&mut app);
    let baseline = baseline_start.elapsed().as_secs_f64();
    let start = std::time::Instant::now();
    let frames = 120;
    for i in 0..frames {
        let x = ((i % 60) as f32 - 30.) * 2.;
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(center + Vec2::new(x, 0.)));
        app.update();
    }
    wait(&mut app);
    let seconds = start.elapsed().as_secs_f64();
    let (batches, stamps, readbacks) = app.world().resource::<engine::gpu::Queue>().statistics();
    assert_eq!(readbacks, 0);
    assert!(batches > 10 && stamps > 20);
    eprintln!(
        "GPU PAINT: triangles=40000 texture=1024 frames={frames} total_ms={:.2} avg_frame_ms={:.2} batches={batches} stamps={stamps} in_stroke_readbacks={readbacks} baseline_avg_ms={:.2} paint_overhead_ms={:.2}",
        seconds * 1000.,
        seconds * 1000. / frames as f64,
        baseline * 1000. / 120.,
        (seconds - baseline) * 1000. / 120.
    );
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(center));
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(
        app.world().resource::<engine::gpu::Queue>().statistics().2,
        1
    );
}

#[test]
#[ignore = "requires GPU; exercises the native Material Paint layout, panel controls and mode isolation"]
fn gpu_model_paint_workspace_layout_keeps_model_and_scene_intact() {
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    warm(&mut app, 8);
    let original = app.world().resource::<editing::State>().documents[&0]
        .objects
        .clone();
    let scene = app.world().resource::<Editor>().edits.clone();
    click(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(editing::EditAction::Menu))
        )
    });
    click(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(editing::EditAction::Mode(
                Mode::Paint
            )))
        )
    });
    warm(&mut app, 15);
    // Keeping the same UI values must not request text shaping/image extraction.
    let stable_labels = app.world().resource::<ui::PaintUi>().retained_labels();
    let ticks = stable_labels.map(|e| {
        app.world()
            .entity(e)
            .get_ref::<Text>()
            .unwrap()
            .last_changed()
    });
    warm(&mut app, 3);
    assert_eq!(
        ticks,
        stable_labels.map(|e| app
            .world()
            .entity(e)
            .get_ref::<Text>()
            .unwrap()
            .last_changed()),
        "idle paint labels must not be rewritten every frame"
    );
    assert_eq!(
        app.world().resource::<editing::State>().mode(Some(0)),
        Mode::Paint
    );
    assert!(!app.world().resource::<editing::State>().available(
        editing::EditAction::Operate(crate::editor::model_document::Operation::Extrude),
        Some(0)
    ));
    let side = labelled(&mut app, "Рисование: слои и свойства");
    let shelf = labelled(&mut app, "Рисование: библиотека ресурсов");
    let uv = labelled(&mut app, "Рисование: 2D-вид UV");
    let view = labelled(&mut app, "3D-вид модели");
    assert!(
        side.width() > 200. && shelf.height() > 150. && uv.width() > 200. && view.width() > 200.
    );
    assert!(view.max.x <= uv.min.x + 2. && uv.max.x <= side.min.x + 2.);
    assert!(view.max.y <= shelf.min.y + 2. && uv.max.y <= shelf.min.y + 2.);
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/model-paint-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(
        &mut app,
        &target,
        &output.join("01-paint-workspace.png"),
        |_| true,
    );
    paint_click(&mut app, window, PaintAction::Add(LayerKind::Paint));
    assert_eq!(
        app.world().resource::<PaintState>().sessions[&0]
            .stack
            .layers
            .len(),
        2
    );
    paint_click(&mut app, window, PaintAction::Undo);
    assert_eq!(
        app.world().resource::<PaintState>().sessions[&0]
            .stack
            .layers
            .len(),
        1
    );
    paint_click(&mut app, window, PaintAction::Panel(Panel::Sets));
    paint_click(&mut app, window, PaintAction::Set(0));
    capture_when(
        &mut app,
        &target,
        &output.join("02-texture-sets.png"),
        |_| true,
    );
    paint_click(&mut app, window, PaintAction::Panel(Panel::Preview));
    paint_click(&mut app, window, PaintAction::View(View::Three));
    assert_eq!(
        labelled(&mut app, "Рисование: 2D-вид UV").size(),
        Vec2::ZERO
    );
    assert!(labelled(&mut app, "3D-вид модели").width() > view.width() * 1.7);
    paint_click(&mut app, window, PaintAction::Reset);
    paint_click(&mut app, window, PaintAction::Category(Category::Brushes));
    paint_click(&mut app, window, PaintAction::BrushTip(Some(0)));
    assert_eq!(app.world().resource::<PaintState>().tips.entries.len(), 24);
    assert_eq!(
        app.world().resource::<PaintState>().sessions[&0].tip,
        Some(0)
    );
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::KeyE,
            logical_key: Key::Character("e".into()),
            state,
            text: None,
            repeat: false,
            window,
        });
        warm(&mut app, 3);
    }
    assert_eq!(
        app.world().resource::<PaintState>().sessions[&0].tool,
        PaintTool::Eraser
    );
    capture_when(&mut app, &target, &output.join("03-brushes.png"), |_| true);
    assert_eq!(
        app.world().resource::<editing::State>().documents[&0].objects,
        original
    );
    assert_eq!(app.world().resource::<Editor>().edits, scene);
    paint_click(&mut app, window, PaintAction::Category(Category::Materials));
    set_shelf_query(&mut app, "  GoLD ");
    let materials = |app: &mut App| {
        let world = app.world_mut();
        let mut query = world.query::<(&Control, &UiGlobalTransform)>();
        let mut cards: Vec<_> = query.iter(world).filter_map(|(control, transform)| match control.action {
            Action::ModelView(ModelViewAction::Paint(PaintAction::Preset(index))) => {
                Some((transform.affine().translation.x, &MATERIALS[index]))
            }
            _ => None,
        }).collect();
        cards.sort_by(|a, b| a.0.total_cmp(&b.0));
        cards.into_iter().map(|(_, material)| material).collect::<Vec<_>>()
    };
    let gold = materials(&mut app);
    assert_eq!(gold.iter().map(|preset| preset.name).collect::<Vec<_>>(), ["Gold", "Rose Gold"]);
    paint_click(&mut app, window, PaintAction::ShelfTypeMenu);
    paint_click(&mut app, window, PaintAction::ShelfType(Some(MaterialType::Wood)));
    assert!(materials(&mut app).is_empty());
    paint_click(&mut app, window, PaintAction::ShelfClearSearch);
    let wood = materials(&mut app);
    assert!(!wood.is_empty() && wood.iter().all(|preset| preset.kind() == MaterialType::Wood));
    capture_when(&mut app, &target, &output.join("05-material-library-filter.png"), |_| true);
    paint_click(&mut app, window, PaintAction::Category(Category::Smart));
    set_shelf_query(&mut app, "oAk aGeD");
    let smart = {
        let world = app.world_mut();
        let mut query = world.query::<&Control>();
        query.iter(world).filter_map(|control| match control.action {
            Action::ModelView(ModelViewAction::Paint(PaintAction::AddSmartMaterial(index))) => {
                Some(SMART_MATERIALS[index].name)
            }
            _ => None,
        }).collect::<Vec<_>>()
    };
    assert_eq!(smart, ["Oak Aged"]);
    paint_click(&mut app, window, PaintAction::Category(Category::Generators));
    set_shelf_query(&mut app, "ОБЛАКА");
    let generators = {
        let world = app.world_mut();
        let mut query = world.query::<&Control>();
        query.iter(world).filter_map(|control| match control.action {
            Action::ModelView(ModelViewAction::Paint(PaintAction::ApplyGenerator(kind, None))) => Some(kind),
            _ => None,
        }).collect::<Vec<_>>()
    };
    assert_eq!(generators, [GeneratorKind::Clouds]);
    paint_click(&mut app, window, PaintAction::ShelfClearSearch);
    click(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(editing::EditAction::Menu))
        )
    });
    click(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(editing::EditAction::Mode(
                Mode::Object
            )))
        )
    });
    assert_eq!(
        app.world().resource::<editing::State>().mode(Some(0)),
        Mode::Object
    );
    let shelf_hidden = {
        let world = app.world_mut();
        world
            .query::<(&AccessibleLabel, &Node)>()
            .iter(world)
            .find(|(label, _)| label.0 == "Рисование: библиотека ресурсов")
            .unwrap()
            .1
            .display
    };
    assert_eq!(shelf_hidden, Display::None);
    assert_eq!(
        app.world().resource::<editing::State>().documents[&0].objects,
        original
    );
    capture_when(
        &mut app,
        &target,
        &output.join("04-return-to-object.png"),
        |_| true,
    );
    eprintln!("Paint layout screenshots: {}", output.display());
}

#[test]
fn paint_generator_shelf_preserves_existing_mask_and_undoes_the_drop() {
    let mut session = Session {
        binding: Some((0, 0)),
        texture_size: 4,
        ..Session::default()
    };
    session.edit(PaintAction::Add(LayerKind::Fill));
    let target = session.stack.selected;
    session.edit(PaintAction::AddMask(true));
    session.edit(PaintAction::AddMaskLayer);
    session.edit(PaintAction::AddFilter(FilterKind::Invert));
    session.edit(PaintAction::Add(LayerKind::Paint));
    let before = session.stack.clone();
    let mask = before.layers[target].mask.as_ref().unwrap();
    session.apply_generator(GeneratorKind::CustomMap, Some(target), Some(9));
    assert_eq!(session.stack.selected, target);
    assert!(session.editing_mask);
    let layer = &session.stack.layers[target];
    let applied = layer.mask.as_ref().unwrap();
    assert_eq!(applied.filters, mask.filters);
    assert_eq!(applied.layers, mask.layers);
    assert_eq!(&applied.pixels[..4], &[255, 0, 0, 255]);
    let generator = applied.generator.unwrap();
    assert_eq!(generator.custom[0], custom::Input { id: 9, weight: 100 });
    session.edit(PaintAction::Undo);
    assert_eq!(session.stack, before);
}

#[test]
#[ignore = "requires GPU; uses the live layer list to move a layer into and out of a folder"]
fn gpu_paint_layer_rows_drag_into_and_across_groups() {
    let (mut app, window, _) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(
        editing::EditAction::Mode(Mode::Paint),
    )));
    warm(&mut app, 8);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::Add(LayerKind::Fill));
    paint_message(&mut app, PaintAction::Add(LayerKind::Folder));
    paint_message(&mut app, PaintAction::CollapseFolder(2));

    let snapshot = |app: &App| app.world().resource::<PaintState>().sessions[&0].stack.clone();
    let layers = |app: &App| snapshot(app).layers.into_iter()
        .map(|layer| (layer.kind, layer.depth)).collect::<Vec<_>>();
    let before = snapshot(&app);
    let position = paint_button_rect(&mut app, PaintAction::Select(2)).center();
    drag_shelf_item(&mut app, window, PaintAction::Select(0), position);
    paint_ready(&mut app);
    assert_eq!(layers(&app), [(LayerKind::Fill, 0), (LayerKind::Paint, 1), (LayerKind::Folder, 0)]);
    assert!(snapshot(&app).listed(1));
    assert_eq!(snapshot(&app).selected, 1);
    let grouped = snapshot(&app);

    let child = paint_button_rect(&mut app, PaintAction::Select(1)).center();
    let outside = paint_button_rect(&mut app, PaintAction::Select(0));
    let below = Vec2::new(outside.center().x, outside.max.y - 5.);
    drag_shelf_item(&mut app, window, PaintAction::Select(2), child);
    assert_eq!(snapshot(&app), grouped);
    drag_shelf_item(&mut app, window, PaintAction::Select(2), below);
    paint_ready(&mut app);
    assert_eq!(layers(&app), [(LayerKind::Paint, 1), (LayerKind::Folder, 0), (LayerKind::Fill, 0)]);
    assert_eq!(snapshot(&app).selected, 1);

    let outside = paint_button_rect(&mut app, PaintAction::Select(2));
    let above = Vec2::new(outside.center().x, outside.min.y + 5.);
    drag_shelf_item(&mut app, window, PaintAction::Select(0), above);
    paint_ready(&mut app);
    assert_eq!(layers(&app), [(LayerKind::Folder, 0), (LayerKind::Fill, 0), (LayerKind::Paint, 0)]);
    assert_eq!(snapshot(&app).selected, 2);
    assert!(snapshot(&app).valid_hierarchy());

    for _ in 0..3 {
        paint_message(&mut app, PaintAction::Undo);
    }
    assert_eq!(snapshot(&app), before);
}

#[test]
#[ignore = "requires GPU; uses real click and drag-and-drop actions on the generator shelf"]
fn gpu_model_paint_shelf_material_and_generator_drop() {
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 15);
    paint_ready(&mut app);
    paint_click(&mut app, window, PaintAction::Add(LayerKind::Fill));
    paint_click(&mut app, window, PaintAction::Category(Category::Generators));
    let card = PaintAction::ApplyGenerator(GeneratorKind::Clouds, None);
    let shelf_labels = {
        let world = app.world_mut();
        let mut labels = world.query::<&AccessibleLabel>();
        labels.iter(world).filter(|label| label.0.starts_with("Применить маску ·")).count()
    };
    assert_eq!(shelf_labels, GeneratorKind::ALL.len());
    paint_click(&mut app, window, card);
    {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        assert_eq!(session.generator(None).unwrap().kind, GeneratorKind::Clouds);
        assert!(session.editing_mask);
    }

    let to = paint_button_rect(&mut app, PaintAction::Select(0)).center();
    drag_shelf_item(&mut app, window, PaintAction::ApplyGenerator(GeneratorKind::Height, None), to);
    paint_ready(&mut app);
    {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        assert_eq!(session.stack.selected, 0);
        assert_eq!(session.generator(None).unwrap().kind, GeneratorKind::Height);
        assert_eq!(session.stack.layers[1].mask.as_ref().unwrap().generator.unwrap().kind,
            GeneratorKind::Clouds);
    }
    let screenshot = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/model-paint-qa/generator-shelf.png");
    std::fs::create_dir_all(screenshot.parent().unwrap()).unwrap();
    capture_when(&mut app, &target, &screenshot, |_| true);
    paint_click(&mut app, window, PaintAction::Undo);
    let session = &app.world().resource::<PaintState>().sessions[&0];
    assert_eq!(session.stack.selected, 1);
    assert!(session.stack.layers[0].mask.is_none());

    paint_click(&mut app, window, PaintAction::Category(Category::Materials));
    set_shelf_query(&mut app, "Gold");
    paint_click(&mut app, window, PaintAction::Add(LayerKind::Folder));
    let folder = app.world().resource::<PaintState>().sessions[&0].stack.selected;
    let to = paint_button_rect(&mut app, PaintAction::Select(folder)).center();
    drag_shelf_item(&mut app, window, PaintAction::Preset(0), to);
    paint_ready(&mut app);
    {
        let stack = &app.world().resource::<PaintState>().sessions[&0].stack;
        assert!(stack.valid_hierarchy());
        assert_eq!(stack.layers[stack.selected].name, "Gold");
        assert_eq!(stack.layers[stack.selected].depth, 1);
        assert_eq!(stack.parents(stack.selected).next(), Some(folder + 1));
    }
    for label in ["3D-вид модели", "Рисование: 2D-вид UV"] {
        // The UV target starts below its header and hint.
        let to = labelled(&mut app, label).center();
        drag_shelf_item(&mut app, window, PaintAction::Preset(0), to);
        paint_ready(&mut app);
        let stack = &app.world().resource::<PaintState>().sessions[&0].stack;
        assert_eq!(stack.selected, stack.layers.len() - 1);
        assert_eq!(stack.layers[stack.selected].name, "Gold");
        assert_eq!(stack.layers[stack.selected].depth, 0);
    }
    paint_click(&mut app, window, PaintAction::Undo);
    let stack = &app.world().resource::<PaintState>().sessions[&0].stack;
    assert_eq!(stack.layers.iter().filter(|layer| layer.name == "Gold").count(), 2);
}

#[test]
fn paint_shelf_material_placements_respect_complete_folder_blocks() {
    let mut session = Session {
        binding: Some((0, 0)),
        texture_size: 4,
        ..Session::default()
    };
    session.edit(PaintAction::Add(LayerKind::Folder));
    session.edit(PaintAction::Add(LayerKind::Paint));
    assert_eq!(session.stack.layers.iter().map(|layer| layer.depth).collect::<Vec<_>>(), [0, 1, 0]);
    let original = session.stack.clone();
    for (placement, position, depth) in [
        (ShelfPlacement::Above(2), 3, 0),
        (ShelfPlacement::Below(2), 1, 0),
        (ShelfPlacement::Inside(2), 2, 1),
        (ShelfPlacement::Above(1), 2, 1),
        (ShelfPlacement::Top, 3, 0),
    ] {
        let action = PaintAction::DropMaterial(ShelfMaterial::Basic(0), placement);
        assert!(session.available(action));
        session.edit(action);
        assert_eq!(session.stack.selected, position);
        assert_eq!(session.stack.layers[position].depth, depth);
        assert!(session.stack.valid_hierarchy());
        session.edit(PaintAction::Undo);
        assert_eq!(session.stack, original);
    }
    assert!(!session.available(PaintAction::DropMaterial(
        ShelfMaterial::Basic(0), ShelfPlacement::Inside(0),
    )));
    session.stack.layers[2].depth = MAX_LAYER_DEPTH - 1;
    assert!(session.available(PaintAction::DropMaterial(ShelfMaterial::Basic(0), ShelfPlacement::Inside(2))));
    assert!(!session.available(PaintAction::DropMaterial(ShelfMaterial::Smart(0), ShelfPlacement::Inside(2))));
}

#[test]
#[ignore = "requires GPU; saves reusable layer and mask presets through the real shelf controls"]
fn gpu_model_paint_user_presets_save_and_apply() {
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let (mut app, window, _) = fixture_project(project);
    let directory = std::env::temp_dir().join(format!("bevy-paint-shelf-ui-{}", std::process::id()));
    app.world_mut().resource_mut::<PaintState>().library.root = Some(directory.clone());
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) {
            break;
        }
    }
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 15);
    paint_ready(&mut app);
    paint_click(&mut app, window, PaintAction::Add(LayerKind::Fill));
    paint_click(&mut app, window, PaintAction::Category(Category::Generators));
    paint_click(&mut app, window, PaintAction::ApplyGenerator(GeneratorKind::Clouds, None));
    let saved_layer = {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        session.stack.layers[session.stack.selected].clone()
    };
    let wait_library = |app: &mut App, count| {
        let start = std::time::Instant::now();
        while app.world().resource::<PaintState>().library.entries.len() < count {
            frame(app);
            assert!(start.elapsed() < std::time::Duration::from_secs(12),
                "expected {count} presets · {}", app.world().resource::<Editor>().status);

        }
        warm(app, 4);
    };

    assert!(paint_button_rect(&mut app, PaintAction::ExportShelfMaterial).width() > 0.);
    paint_click(&mut app, window, PaintAction::SaveShelfMaterial);
    wait_library(&mut app, 1);
    paint_click(&mut app, window, PaintAction::SaveShelfMask);
    wait_library(&mut app, 2);
    let (material, mask) = {
        let resource = app.world().resource::<PaintState>();
        let entries = &resource.library.entries;
        let material = entries.iter().position(|entry| matches!(entry.kind, presets::Kind::Material { .. })).unwrap();
        let mask = entries.iter().position(|entry| matches!(entry.kind, presets::Kind::Mask(_))).unwrap();
        (material, mask)
    };
    assert_eq!(presets::list(&directory).unwrap().len(), 2);
    paint_click(&mut app, window, PaintAction::Add(LayerKind::Paint));
    paint_click(&mut app, window, PaintAction::ApplyShelfMask(mask, None));
    assert_eq!(
        app.world().resource::<PaintState>().sessions[&0].generator(None).unwrap().kind,
        GeneratorKind::Clouds
    );
    paint_click(&mut app, window, PaintAction::Undo);
    assert!(!app.world().resource::<PaintState>().sessions[&0].has_mask());

    paint_click(&mut app, window, PaintAction::Category(Category::Smart));
    {
        let world = app.world_mut();
        let mut cards = world.query::<(&Control, &Children)>();
        let image = cards.iter(world)
            .find(|(control, _)| matches!(control.action,
                Action::ModelView(ModelViewAction::Paint(PaintAction::ApplyShelfMaterial(i))) if i == material))
            .and_then(|(_, children)| children.iter().find_map(|child| world.get::<ImageNode>(child)))
            .expect("saved smart material has a shelf swatch");
        assert_eq!(world.resource::<Assets<Image>>().get(&image.image).unwrap().width(), 144);
    }
    paint_click(&mut app, window, PaintAction::ApplyShelfMaterial(material));
    let start = std::time::Instant::now();
    loop {
        frame(&mut app);
        let session = &app.world().resource::<PaintState>().sessions[&0];
        let doc = &app.world().resource::<editing::State>().documents[&0];
        if !session.smart_loading && !doc.paint_busy && session.stack.layers.len() == 4 {
            let layer = &session.stack.layers[session.stack.selected];
            assert_eq!((layer.kind, &layer.name, &layer.channels), (LayerKind::Fill, &saved_layer.name, &saved_layer.channels));
            assert_eq!(layer.mask.as_ref().unwrap().generator.unwrap().kind, GeneratorKind::Clouds);
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(65));
    }
    paint_click(&mut app, window, PaintAction::Undo);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].stack.layers.len(), 3);
    let source_dir = directory.join("foreign");
    std::fs::create_dir_all(&source_dir).unwrap();
    let path = source_dir.join("from-material-painter.mpsmart");
    presets::mpsmart_fixture(&path, false);
    app.world_mut().resource_mut::<PaintState>().library
        .start_import({
            let path = path.clone();
            move || Ok(Some(path))
        }).unwrap();
    wait_library(&mut app, 3);
    let imported = app.world().resource::<PaintState>().library.entries.iter()
        .position(|entry| entry.name == "MP Patinated brass").unwrap();
    paint_click(&mut app, window, PaintAction::ApplyShelfMaterial(imported));
    let start = std::time::Instant::now();
    loop {
        frame(&mut app);
        let session = &app.world().resource::<PaintState>().sessions[&0];
        let doc = &app.world().resource::<editing::State>().documents[&0];
        if !session.smart_loading && !doc.paint_busy && session.stack.layers.len() == 6 {
            let block = session.stack.block(session.stack.selected);
            assert_eq!(block.len(), 3);
            let layers = &session.stack.layers[block];
            assert_eq!(layers[0].name, "Brass coating");
            assert_eq!(layers[0].mask.as_ref().unwrap().filters.len(), 1);
            assert_eq!(layers[1].depth, 1);
            assert_eq!(layers[2].name, "MP Patinated brass");
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(90),
            "{}", app.world().resource::<Editor>().status);
    }
    paint_click(&mut app, window, PaintAction::Undo);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].stack.layers.len(), 3);
    for entry in presets::list(&directory).unwrap() {
        std::fs::remove_file(entry.path).unwrap();
    }
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(source_dir).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
