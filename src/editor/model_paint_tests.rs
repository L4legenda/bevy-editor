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
fn shelf_material_activates_pbr_brush_without_changing_the_layer() {
    let mut session = Session::default();
    session.edit(PaintAction::Add(LayerKind::Paint));
    session.edit(PaintAction::AddMask(true));
    session.edit(PaintAction::Tool(PaintTool::Eraser));
    session.multi_channel_brush = false;
    session.brush_channels = (1 << 3) | (1 << 5);
    session.brush_height = 62.;
    session.brush_alpha = 83.;
    session.brush_emission = [11., 22., 33.];
    let stack = session.stack.clone();
    let history = session.undo.len();
    let material = &MATERIALS[0];

    session.edit(PaintAction::Preset(0));

    assert_eq!(session.tool, PaintTool::Brush);
    assert_eq!(session.stroke_channels(), 0b0010_1111);
    assert_eq!(session.brush_color(), material.rgb());
    assert_eq!(session.roughness, material.roughness * 100.);
    assert_eq!(session.metallic, material.metallic * 100.);
    assert_eq!(session.brush_height, 62.);
    assert_eq!(session.brush_alpha, 83.);
    assert_eq!(session.brush_emission, [11., 22., 33.]);
    assert_eq!(session.stack, stack);
    assert_eq!(session.undo.len(), history);
}

#[test]
#[ignore = "requires GPU; checks the Windows Ink pan input in both paint views"]
fn gpu_pen_barrel_pans_paint_views_without_painting() {
    use crate::editor::ui::image_viewer::paint_input::{PenPan, PointerFrame, Sample, collect_frame};

    #[derive(Resource, Default)]
    struct Pen(Option<PenPan>);

    fn inject(pen: Res<Pen>, mut pointer: ResMut<PointerFrame>) {
        if let Some(pan) = pen.0 {
            pointer.pen_pan = Some(pan);
            pointer.pen_pan_handled = true;
            if let Some(start) = pan.start {
                pointer.samples.push(Sample::Down(start));
            }
            pointer.samples.push(Sample::Move(pan.point));
        } else {
            pointer.pen_pan = None;
            pointer.pen_pan_handled = false;
        }
    }

    fn center<T: Component>(app: &mut App) -> Vec2 {
        let world = app.world_mut();
        let (node, transform) = world.query_filtered::<
            (&ComputedNode, &UiGlobalTransform), With<T>,
        >().single(world).unwrap();
        transform.affine().translation * node.inverse_scale_factor()
    }

    let (mut app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
    app.init_resource::<Pen>().add_systems(PreUpdate, inject.after(collect_frame));
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

    for view in [View::Three, View::Two] {
        paint_message(&mut app, PaintAction::View(view));
        let canvas = if view == View::Three { center::<Canvas>(&mut app) }
            else { center::<engine::TextureCanvas>(&mut app) };
        let camera = app.world().resource::<Viewer>().center;
        let undo = app.world().resource::<PaintState>().sessions[&0].undo.len();
        app.world_mut().resource_mut::<Pen>().0 = Some(PenPan { start: Some(canvas), point: canvas });
        frame(&mut app);
        app.world_mut().resource_mut::<Pen>().0 = Some(PenPan { start: None, point: canvas + Vec2::new(30., 15.) });
        frame(&mut app);
        app.world_mut().resource_mut::<Pen>().0 = None;
        warm(&mut app, 8);
        assert_eq!(app.world().resource::<PaintState>().sessions[&0].undo.len(), undo);
        if view == View::Three {
            assert_ne!(app.world().resource::<Viewer>().center, camera);
        } else {
            assert_eq!(app.world().resource::<Viewer>().center, camera);
            assert!(center::<engine::TextureCanvas>(&mut app).distance(canvas + Vec2::new(30., 15.)) < 2.);
        }
    }
}

#[test]
fn paint_history_limit_keeps_latest_edits_in_active_and_parked_sets() {
    let mut paint = PaintState::default();
    paint.activate(0, 24);
    let session = paint.sessions.get_mut(&0).unwrap();
    let initial_layers = session.stack.layers.len();
    for _ in 0..20 {
        session.edit(PaintAction::Add(LayerKind::Paint));
    }
    for _ in 0..10 {
        session.edit(PaintAction::Undo);
    }
    session.histories.insert((1, 2), (session.undo.clone(), session.redo.clone()));
    session.set_history_limit(8);
    let (parked_undo, parked_redo) = &session.histories[&(1, 2)];
    assert_eq!(parked_undo.len(), 8);
    assert_eq!(parked_undo[0].layer_count(), initial_layers + 2);
    assert_eq!(parked_redo.len(), 8);
    assert_eq!(parked_redo[0].layer_count(), initial_layers + 18);

    for _ in 0..9 {
        session.edit(PaintAction::Undo);
    }
    assert!(session.undo.is_empty());
    assert_eq!(session.stack.layers.len(), initial_layers + 2);
    for _ in 0..8 {
        session.edit(PaintAction::Redo);
    }
    assert_eq!(session.stack.layers.len(), initial_layers + 10);

    let previous = session.stack.clone();
    session.record_undo(previous, history::Label::LayerProperties);
    assert_eq!(session.undo.len(), 8);
    assert_eq!(session.undo.last().unwrap().layer_count(), initial_layers + 10);
    assert!(session.redo.is_empty());
    assert_eq!(session.histories[&(1, 2)].0.len(), 8);
    assert!(session.histories[&(1, 2)].1.is_empty());
}

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
    let map = context.map(height).values;
    assert_eq!((map[8], map[15 * 16 + 8]), (0, 255));
    assert!(Arc::ptr_eq(&map, &context.map(height).values));
    let cloud = context.map(Generator::new(GeneratorKind::Clouds)).values;
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

fn search_graph_palette(app: &mut App, value: &str) -> Entity {
    let world = app.world_mut();
    let mut fields = world.query::<(Entity, &AccessibleLabel, &mut bevy::text::EditableText)>();
    let (entity, _, mut field) = fields.iter_mut(world)
        .find(|(_, label, _)| label.0 == "Поиск узлов").unwrap();
    field.editor_mut().set_text(value);
    entity
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
    let world = app.world_mut();
    let mut buttons = world.query::<(Entity, &Control, &ComputedNode)>();
    let entity = buttons.iter(world).find_map(|(entity, control, node)| {
        (node.size().min_element() > 0.
            && matches!(control.action, Action::ModelView(ModelViewAction::Paint(value)) if value == action))
            .then_some(entity)
    }).unwrap();
    scroll_to_paint_entity(app, entity);
}

fn scroll_to_paint_entity(app: &mut App, mut entity: Entity) {
    use bevy::ui::{OverflowAxis, ScrollPosition};
    let world = app.world_mut();
    let target = world.get::<UiGlobalTransform>(entity).unwrap().affine().translation.y
        * world.get::<ComputedNode>(entity).unwrap().inverse_scale_factor();
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
pub(super) fn paint_ready(app: &mut App) {
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
#[ignore = "requires GPU; exercises the paint mesh replacement menu and live rebinding"]
fn gpu_paint_mesh_replacement_menu_restores_the_previous_model() {
    use paint_mesh_replacement::Shape;

    let (mut app, window, _) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    let started = std::time::Instant::now();
    while !app.world().resource::<editing::State>().documents.contains_key(&0) {
        frame(&mut app);
        assert!(started.elapsed().as_secs() < 20);
    }
    let original = app.world().resource::<editing::State>().documents[&0].objects[0].id;
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(
        editing::EditAction::Mode(Mode::Paint),
    )));
    warm(&mut app, 10);
    let mesh = PaintAction::MeshReplacement;
    paint_click(&mut app, window, mesh(MeshCommand::Toggle));
    paint_click(&mut app, window, mesh(MeshCommand::Primitive(Shape::Plane)));
    warm(&mut app, 8);
    let replacement = app.world().resource::<editing::State>().documents[&0].objects[0].id;
    assert_ne!(replacement, original);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].binding, Some((replacement, 0)));
    scroll_to_paint_control(&mut app, mesh(MeshCommand::Undo));
    paint_click(&mut app, window, mesh(MeshCommand::Undo));
    warm(&mut app, 8);
    assert_eq!(app.world().resource::<editing::State>().documents[&0].objects[0].id, original,
        "{}", app.world().resource::<Editor>().status);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].binding, Some((original, 0)));
    scroll_to_paint_control(&mut app, mesh(MeshCommand::Redo));
    paint_click(&mut app, window, mesh(MeshCommand::Redo));
    warm(&mut app, 8);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].binding, Some((replacement, 0)));
}

#[test]
#[ignore = "requires GPU; exercises Save As in the paint workspace and opens the new model tab"]
fn gpu_paint_save_copy_opens_a_separate_editable_tab() {
    let directory = std::env::temp_dir().join(format!("bevy-paint-save-copy-{}", bevy::asset::uuid::Uuid::new_v4()));
    let mut project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    copy_tree(&project.asset_root, &directory);
    project.asset_root = directory.clone();
    let (mut app, window, _) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    let started = std::time::Instant::now();
    while !app.world().resource::<editing::State>().documents.contains_key(&0) {
        frame(&mut app);
        assert!(started.elapsed().as_secs() < 20);
    }
    let source = app.world().resource::<editing::State>().documents[&0].source.path.clone();
    let original = std::fs::read(&source).unwrap();
    let destination = source.with_file_name("paint-copy-test.glb");
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Mode(Mode::Paint))));
    let add_fill = PaintAction::Add(LayerKind::Fill);
    let started = std::time::Instant::now();
    while !app.world().resource::<PaintState>().sessions.get(&0)
        .is_some_and(|session| session.binding.is_some() && session.available(add_fill))
    {
        frame(&mut app);
        assert!(started.elapsed().as_secs() < 20, "The paint surface did not become editable");
    }
    paint_message(&mut app, add_fill);
    let original_stack = app.world().resource::<PaintState>().sessions[&0].stack.clone();
    assert!(app.world().resource::<editing::State>().documents[&0].dirty());
    #[cfg(feature = "mcp")]
    {
        use bevy::ecs::system::RunSystemOnce;
        use serde_json::json;
        let (send, receive) = std::sync::mpsc::channel();
        let args: mcp_state::ExportArgs = serde_json::from_value(json!({
            "model_id": 0, "folder": ".", "format": "png", "bit_depth": 8,
            "template": "separate", "prefix": "mcp-export", "size": 512,
        })).unwrap();
        app.world_mut().run_system_once(move |mut output: mcp_state::McpOutput,
            editor: Res<Editor>, editing: Res<editing::State>, paint: Res<PaintState>| {
            let send = send.clone();
            mcp_state::export_textures(&mut output, &editor, &editing, &paint, args.clone(), Box::new(move |result| {
                send.send(result).unwrap();
            }));
        }).unwrap();
        let export = drive_paint_file_reply(&mut app, receive);
        let files = export["files"].as_array().unwrap();
        let color = files.iter().map(|path| path.as_str().unwrap())
            .find(|path| path.contains("BaseColor")).expect("Exported base color map");
        let color = image::open(color).unwrap().to_rgba8();
        assert_eq!(color.dimensions(), (512, 512));
        assert!(color.pixels().any(|pixel| pixel[3] > 0));

        let copy = mcp_save_reply(&mut app, json!({"model_id": 0, "copy_name": "mcp-paint-copy.glb"}));
        let saved = crate::editor::model_document::glb::Glb::load(
            std::path::Path::new(copy["path"].as_str().unwrap()),
        ).unwrap();
        let mesh = saved.json["nodes"][0]["mesh"].as_u64().unwrap() as usize;
        let primitive = &saved.json["meshes"][mesh]["primitives"][0];
        assert_eq!(saved.paint_surface(primitive).unwrap().unwrap().stack, original_stack);
        assert_eq!(std::fs::read(&source).unwrap(), original);
    }
    app.world_mut().resource_mut::<save_copy::State>().test_path = Some(destination.clone());
    paint_click(&mut app, window, PaintAction::SaveCopy);
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        if let Some(index) = app.world().resource::<Editor>().viewing_model.filter(|&index| index != 0)
            && let Some(doc) = app.world().resource::<editing::State>().documents.get(&index)
            && doc.mode == Mode::Paint
            && let Some(session) = app.world().resource::<PaintState>().sessions.get(&index)
            && session.binding.is_some()
        {
            assert_eq!(session.stack.layers.len(), original_stack.layers.len());
            assert_eq!(session.stack.layers.last().unwrap().kind, LayerKind::Fill);
            assert!(!doc.dirty());
            break;
        }
        assert!(started.elapsed().as_secs() < 30, "{}", app.world().resource::<Editor>().status);
    }
    assert!(destination.is_file());
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert!(app.world().resource::<editing::State>().documents[&0].dirty());
    assert!(app.world().resource::<Editor>().model_tabs.contains(&0));
    #[cfg(feature = "mcp")]
    {
        let model = app.world().resource::<Editor>().viewing_model.unwrap();
        paint_ready(&mut app);
        paint_message(&mut app, PaintAction::Add(LayerKind::Fill));
        let expected = original_stack.layers.len() + 1;
        let doc = &app.world().resource::<editing::State>().documents[&model];
        assert!(doc.dirty());
        assert_eq!(doc.objects[0].materials[0].paint.as_ref().unwrap().stack.layers.len(), expected);
        let saved = mcp_save_reply(&mut app, serde_json::json!({"model_id": model}));
        assert_eq!(saved["copy"], false);
        assert_eq!(std::path::Path::new(saved["path"].as_str().unwrap()).canonicalize().unwrap(), destination.canonicalize().unwrap());
        assert!(!app.world().resource::<editing::State>().documents[&model].dirty());
        let saved = crate::editor::model_document::glb::Glb::load(&destination).unwrap();
        let mesh = saved.json["nodes"][0]["mesh"].as_u64().unwrap() as usize;
        let primitive = &saved.json["meshes"][mesh]["primitives"][0];
        assert_eq!(saved.paint_surface(primitive).unwrap().unwrap().stack.layers.len(), expected);
    }
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(feature = "mcp")]
fn drive_paint_file_reply(app: &mut App, receive: std::sync::mpsc::Receiver<Result<serde_json::Value, String>>) -> serde_json::Value {
    let started = std::time::Instant::now();
    loop {
        if let Ok(result) = receive.try_recv() {
            return result.unwrap();
        }
        frame(app);
        assert!(started.elapsed().as_secs() < 30, "No paint file reply");
    }
}

#[cfg(feature = "mcp")]
fn mcp_save_reply(app: &mut App, request: serde_json::Value) -> serde_json::Value {
    use bevy::ecs::system::RunSystemOnce;
    let args: mcp_state::SaveArgs = serde_json::from_value(request).unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    app.world_mut().run_system_once(move |mut output: mcp_state::McpOutput, mut editor: ResMut<Editor>,
        mut editing: ResMut<editing::State>, paint: Res<PaintState>| {
        let send = send.clone();
        mcp_state::save_model(&mut output, &mut editor, &mut editing, &paint, args.clone(), Box::new(move |result| {
            send.send(result).unwrap();
        }));
    }).unwrap();
    drive_paint_file_reply(app, receive)
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
    let graph = folder_graph::start(&mut reopened, surface.size, 0, None);
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
    #[cfg(feature = "mcp")]
    {
        let expected = app.world().resource::<Assets<Image>>().get(&traced).unwrap().size();
        let capture = mcp_capture_model(&mut app);
        assert_eq!((capture.width(), capture.height()), (expected.x, expected.y));
        let pixel = capture.to_rgba8().get_pixel(expected.x / 2, expected.y / 2).0;
        assert!(pixel[0] > pixel[1].saturating_add(35) && pixel[2] > pixel[1].saturating_add(25));
    }
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
    let preferences = app.world().resource::<environment::Settings>().clone();
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
    #[cfg(feature = "mcp")]
    let request = |app: &mut App, settings| {
        use bevy::ecs::system::RunSystemOnce;
        let args: mcp_state::PaintViewArgs = serde_json::from_value(settings).unwrap();
        app.world_mut().run_system_once(move |mut input: mcp_state::McpPreview,
            editor: Res<Editor>, mut editing: ResMut<editing::State>, mut paint: ResMut<PaintState>| {
            mcp_state::set_view(&mut input, &editor, &mut editing, &mut paint, args.clone())
        }).unwrap()
    };
    scroll_to_paint_control(&mut app, PaintAction::PostPanel);
    paint_click(&mut app, window, PaintAction::PostPanel);
    let saturation_slider = {
        let world = app.world_mut();
        world.query::<(Entity, &AccessibleLabel)>().iter(world)
            .find(|(_, label)| label.0 == "Просмотр: Насыщенность").unwrap().0
    };
    scroll_to_paint_entity(&mut app, saturation_slider);
    assert!(app.world().get::<bevy::ui::InteractionDisabled>(saturation_slider).is_none());
    app.world_mut().trigger(bevy::ui_widgets::ValueChange::<f32> {
        source: saturation_slider, value: 0.0, is_final: false,
    });
    move_pointer(&mut app, window, Vec2::new(8.0, 512.0));
    capture_when(&mut app, &target, &output.join("viewport-graded.png"), gray);
    assert_eq!(samples(app.world()), Some(before_grade), "post effects reuse the accumulated HDR frame");
    assert_eq!(*app.world().resource::<environment::Settings>(), preferences);
    let doc = &app.world().resource::<editing::State>().documents[&0];
    assert_eq!(Field::Saturation.value(doc.paint_post.unwrap().effects), 0.0);
    #[cfg(feature = "mcp")]
    {
        request(&mut app, serde_json::json!({"model_id": 0, "post": {"saturation": 1.0}})).unwrap();
        capture_when(&mut app, &target, &output.join("viewport-mcp-color.png"), |image| !gray(image));
        let response = request(&mut app, serde_json::json!({"model_id": 0, "post": {"saturation": 0.0}})).unwrap();
        assert_eq!(response["post"]["effects"]["saturation"], 0.0);
        capture_when(&mut app, &target, &output.join("viewport-mcp-graded.png"), gray);
        assert_eq!(samples(app.world()), Some(before_grade));
        assert_eq!(*app.world().resource::<environment::Settings>(), preferences);
    }
    paint_message(&mut app, PaintAction::PreviewShader(None));
    assert_eq!(canvas_image(&mut app), raster);
    capture_when(&mut app, &target, &output.join("viewport-raster-graded.png"), gray);
    #[cfg(feature = "mcp")]
    {
        let capture = mcp_capture_model(&mut app);
        let pixel = capture.to_rgba8().get_pixel(capture.width() / 2, capture.height() / 2).0;
        assert!(pixel[0].abs_diff(pixel[1]) < 3 && pixel[0].abs_diff(pixel[2]) < 3);
    }
    app.world_mut().write_message(saturation(1.0));
    capture_when(&mut app, &target, &output.join("viewport-raster-reset.png"), |image| !gray(image));
    let exposure = Action::ModelView(ModelViewAction::Environment(
        EnvironmentAction::Set(Setting::PaintExposure, 1.5, true),
    ));
    app.world_mut().write_message(exposure);
    warm(&mut app, 4);
    let world = app.world_mut();
    let grade = world.query::<&super::post::RasterGrade>().single(world).unwrap();
    assert_eq!(grade.0.exposure, 1.5 - preferences.paint_post(None).exposure);
    assert_eq!(*world.resource::<environment::Settings>(), preferences);
    #[cfg(feature = "mcp")]
    {
        let previous = app.world().resource::<Viewer>().camera();
        assert!(request(&mut app, serde_json::json!({"model_id": 0, "yaw_degrees": 30, "fov_degrees": 0})).is_err());
        assert_eq!(app.world().resource::<Viewer>().camera(), previous);
        let view = request(&mut app, serde_json::json!({"model_id": 0, "yaw_degrees": 30, "fov_degrees": 60,
            "orthographic": true, "preview": {"mode": "channel", "channel": "roughness"}})).unwrap();
        assert_eq!(view["channel"], "roughness");
        assert_eq!(view["camera"]["orthographic"], true);
        let doc = &app.world().resource::<editing::State>().documents[&0];
        assert!((doc.model_camera.unwrap().yaw.to_degrees() - 30.).abs() < 0.01);
    }
}

#[cfg(feature = "mcp")]
fn mcp_capture_model(app: &mut App) -> image::DynamicImage {
    use bevy::ecs::system::RunSystemOnce;
    let args: mcp_state::CaptureArgs = serde_json::from_value(serde_json::json!({"model_id": 0})).unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    app.world_mut().run_system_once(move |mut commands: Commands, editor: Res<Editor>| {
        let send = send.clone();
        mcp_state::capture(&mut commands, &editor, args, Box::new(move |result| { send.send(result).unwrap(); }));
    }).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        frame(app);
        if let Ok(result) = receive.try_recv() { return result.unwrap(); }
        assert!(std::time::Instant::now() < deadline, "No model viewport capture");
    }
}

#[test]
#[ignore = "requires GPU; evaluates imported MPP graphs when entering Paint mode"]
fn gpu_mpp_bitmap_graph_refreshes_when_opening_paint() {
    use crate::editor::model_paint_data::solid;
    use material_graph::file::tests as graph_fixture;

    let directory = std::env::temp_dir().join(format!(
        "bevy-mpp-graph-gpu-{}",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    let textures = directory.join("textures");
    let (_, mut graph) = graph_fixture::fixture(&directory, &textures);
    let node = graph.nodes[0].id;
    let output = graph.nodes[1].id;
    let bitmap = graph.nodes[0].params["image"].clone();
    let second_bitmap = graph.add("bitmap", [40., 150.]);
    graph.node_mut(second_bitmap).unwrap().params.insert("image".into(), bitmap);
    let mut warnings = Vec::new();
    let imported = project_graph(
        &material_graph::encode(&graph), &directory, 1, |_| Ok(()), &mut warnings,
    ).unwrap();
    assert!(warnings.is_empty());
    std::fs::remove_dir_all(textures).unwrap();

    let size = 128;
    let mut layer = Layer::new("Imported bitmap".into(), LayerKind::Fill);
    project_fill(
        &mut layer,
        &serde_json::json!({"color": [1., 1., 1.]}),
        Default::default(),
        texture_fill::OpacitySource::Auto,
        Some(&imported),
        |color| Ok(solid(size, color)),
    ).unwrap();
    let mut omitted = 0;
    layer.mask = Some(project_mask(
        &serde_json::Value::Null,
        solid(size, [255, 0, 0, 255]),
        Some(&imported),
        None,
        &[],
        &mut omitted,
    ).unwrap());
    let size_px = image::RgbaImage::from_pixel(size, size, image::Rgba([96, 96, 96, 255]));
    let cached_output = Arc::new(texture_fill::Bitmap::from_rgba("Cached mask.png".into(), size_px).unwrap());
    let mut cached = Layer::new("Cached graph mask".into(), LayerKind::Paint);
    cached.mask = Some(project_mask(
        &serde_json::Value::Null,
        solid(size, [255, 32, 0, 255]),
        Some(&imported),
        Some(cached_output),
        &[],
        &mut omitted,
    ).unwrap());
    let (mut app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
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
        doc.objects[0].materials[0].paint = Some(Arc::new(Surface {
            size,
            base: std::array::from_fn(|_| solid(size, [255; 4])),
            stack: Stack { layers: vec![layer, cached], selected: 0 },
            material: serde_json::Value::Null,
            baked: None,
        }));
        doc.revision += 1;
    }
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(
        editing::EditAction::Mode(Mode::Paint),
    )));
    let start = std::time::Instant::now();
    loop {
        frame(&mut app);
        let paint = app.world().resource::<PaintState>();
        if let Some(session) = paint.sessions.get(&0)
            && let Some(layer) = session.stack.layers.first()
            && let Some(fill) = layer.texture_fill.as_ref()
            && let Some(color) = fill.images[0].as_ref()
        {
            let mask = layer.mask.as_ref().unwrap().graph.as_ref().unwrap();
            let mask_pixel = mask.output.as_ref().unwrap().pixels().get_pixel(0, 0).0;
            for pixel in [color.pixels().get_pixel(0, 0).0, mask_pixel] {
                assert!(pixel.into_iter().zip(graph_fixture::PIXEL)
                    .all(|(actual, expected)| actual.abs_diff(expected) <= 1), "{pixel:?}");
            }
            assert!(fill.graph.as_ref().unwrap().get(material_graph::MPP_GRAPH_PENDING).is_none());
            assert_eq!(session.stack.layers[1].mask.as_ref().unwrap().effective()[0], 128);
            break;
        }
        assert!(start.elapsed().as_secs() < 30, "{}", app.world().resource::<Editor>().status);
    }
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::SelectLayerMask(1));
    paint_message(&mut app, PaintAction::GraphEdit(material_graph::Edit::Move(node, IVec2::new(40, 52))));
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].stack.layers[1]
        .mask.as_ref().unwrap().effective()[0], 128);
    paint_message(&mut app, PaintAction::GraphEdit(material_graph::Edit::Connect(second_bitmap, output, 0)));
    let start = std::time::Instant::now();
    loop {
        frame(&mut app);
        let session = &app.world().resource::<PaintState>().sessions[&0];
        let mask = session.stack.layers[1].mask.as_ref().unwrap();
        let output = mask.graph.as_ref().unwrap().output.as_ref().unwrap().pixels().get_pixel(0, 0).0;
        if output[0].abs_diff(graph_fixture::PIXEL[0]) <= 1 {
            assert_eq!(&mask.pixels[..4], &[255, 32, 0, 255]);
            assert_eq!(mask.effective()[0], output[0] + 32);
            break;
        }
        assert!(start.elapsed().as_secs() < 30, "{}", app.world().resource::<Editor>().status);
    }
    std::fs::remove_dir_all(directory).unwrap();
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

    #[cfg(not(feature = "mcp"))]
    paint_message(&mut app, PaintAction::BakeStart);
    #[cfg(feature = "mcp")]
    {
        use bevy::ecs::system::RunSystemOnce;
        use serde_json::json;
        let start = |app: &mut App, front, reply: Option<std::sync::mpsc::Sender<_>>| {
            let args: mcp_state::BakeArgs = serde_json::from_value(json!({
                "target": {"model_id": 0, "set": 0}, "quality": "draft",
                "maps": ["part_random"], "front_percent": front,
            })).unwrap();
            app.world_mut().run_system_once(move |mut input: mcp_state::McpBake,
                mut editor: ResMut<Editor>, mut editing: ResMut<editing::State>, mut paint: ResMut<PaintState>| {
                let result = mcp_state::start_bake(&mut input, &mut editor, &mut editing, &mut paint, args.clone());
                if let Some(reply) = reply.clone() {
                    input.respond(true, result.clone(), Box::new(move |result| {
                        reply.send(result).unwrap();
                    }));
                }
                result.map(|(_, report)| report)
            }).unwrap()
        };
        assert!(start(&mut app, 0, None).is_err());
        let (reply, result) = std::sync::mpsc::channel();
        let started = start(&mut app, 6, Some(reply)).unwrap();
        assert_eq!(started["status"], "running");
        assert!(matches!(result.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
        let target = serde_json::from_value(json!({"model_id": 0, "set": 0})).unwrap();
        let cancelled = app.world_mut().run_system_once(move |mut input: mcp_state::McpBake,
            mut editor: ResMut<Editor>, mut editing: ResMut<editing::State>, paint: Res<PaintState>| {
            mcp_state::cancel_bake(&mut input, &mut editor, &mut editing, &paint, target)
        }).unwrap().unwrap();
        assert_eq!(cancelled["status"], "cancelled");
        assert_eq!(result.try_recv().unwrap().unwrap_err(), "Bake was cancelled before maps were published");
        let started = start(&mut app, 7, None).unwrap();
        assert_eq!(started["status"], "running");
        assert_eq!(started["resolution"], 512);
    }
    let binding = app.world().resource::<PaintState>().sessions[&0].binding.unwrap();
    wait(&mut app, &|world| {
        let session = &world.resource::<PaintState>().sessions[&0];
        let doc = &world.resource::<editing::State>().documents[&0];
        let current = world.resource::<bake::State>().maps(0, doc, binding, bake_size);
        current.is_some_and(|maps| session.graph_bake.as_ref()
            .is_some_and(|rendered| rendered.ptr_eq(&Arc::downgrade(maps))))
    });
    #[cfg(feature = "mcp")]
    {
        use bevy::ecs::system::RunSystemOnce;
        let target = serde_json::from_value(serde_json::json!({"model_id": 0, "set": 0})).unwrap();
        let state = app.world_mut().run_system_once(move |input: mcp_state::McpBake,
            editor: Res<Editor>, editing: Res<editing::State>, paint: Res<PaintState>| {
            mcp_state::bake_status(&input, &editor, &editing, &paint, target)
        }).unwrap().unwrap();
        assert_eq!(state["status"], "ready");
        assert!(state["maps"].as_array().unwrap().iter().any(|map| map == "part_random"));
        assert!(state["report"]["uv_texels"].as_u64().unwrap() > 0);
    }
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
#[ignore = "requires GPU; changes recovery preferences from the painting properties"]
fn gpu_paint_sets_recovery_preferences_from_properties() {
    let (mut app, window, _) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
    while !app.world().resource::<editing::State>().documents.contains_key(&0) {
        frame(&mut app);
        assert!(std::time::Instant::now() < deadline);
    }
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(
        editing::EditAction::Mode(Mode::Paint),
    )));
    warm(&mut app, 8);
    paint_ready(&mut app);
    scroll_to_paint_control(&mut app, PaintAction::ToggleRecovery);
    paint_click(&mut app, window, PaintAction::ToggleRecovery);
    assert!(!app.world().resource::<editing::recovery::Preferences>().settings().enabled);

    let field = {
        let world = app.world_mut();
        world.query::<(Entity, &ui::HistoryPreference)>().iter(world)
            .find_map(|(entity, preference)| (*preference == ui::HistoryPreference::Recovery).then_some(entity))
            .unwrap()
    };
    scroll_to_paint_entity(&mut app, field);
    let point = labelled(&mut app, "Рисование: Правок между автокопиями").center();
    let world = app.world();
    let scale = world.get::<Window>(window).unwrap().scale_factor();
    assert!(world.get::<bevy::ui::CalculatedClip>(field)
        .is_none_or(|clip| clip.contains_point(point * scale)), "threshold field is clipped at {point:?}");
    move_pointer(&mut app, window, point);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(field));
    for (typed, expected) in [("501", 10), ("37", 37)] {
        app.world_mut().get_mut::<bevy::text::EditableText>(field).unwrap()
            .editor_mut().set_text(typed);
        fill_modifier(&mut app, window, KeyCode::Enter, Key::Enter, ButtonState::Pressed);
        fill_modifier(&mut app, window, KeyCode::Enter, Key::Enter, ButtonState::Released);
        assert_eq!(app.world().resource::<editing::recovery::Preferences>().settings().edits, expected);
    }
    paint_click(&mut app, window, PaintAction::ToggleRecovery);
    let settings = app.world().resource::<editing::recovery::Preferences>().settings();
    assert!(settings.enabled);
    assert_eq!(settings.edits, 37);
    let field = {
        let world = app.world_mut();
        world.query::<(Entity, &ui::HistoryPreference)>().iter(world)
            .find_map(|(entity, preference)| (*preference == ui::HistoryPreference::Undo).then_some(entity))
            .unwrap()
    };
    scroll_to_paint_entity(&mut app, field);
    let point = labelled(&mut app, "Рисование: Шагов отмены на набор").center();
    move_pointer(&mut app, window, point);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(field));
    for (typed, expected) in [("7", 32), ("12", 12)] {
        app.world_mut().get_mut::<bevy::text::EditableText>(field).unwrap()
            .editor_mut().set_text(typed);
        fill_modifier(&mut app, window, KeyCode::Enter, Key::Enter, ButtonState::Pressed);
        fill_modifier(&mut app, window, KeyCode::Enter, Key::Enter, ButtonState::Released);
        assert_eq!(app.world().resource::<editing::recovery::Preferences>().settings().undo_steps, expected);
        assert_eq!(app.world().resource::<PaintState>().sessions[&0].history_limit, expected);
    }
    assert!(!app.world().resource::<editing::State>().documents[&0].dirty());
    let edits = app.world().resource::<editing::State>().documents[&0].recovery_edits;
    let add = PaintAction::Add(LayerKind::Paint);
    scroll_to_paint_control(&mut app, add);
    paint_click(&mut app, window, add);
    assert_eq!(app.world().resource::<editing::State>().documents[&0].recovery_edits, edits + 1);
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
        prefs.cycle_bake_quality().unwrap();
        prefs.toggle_bake_match_names().unwrap();
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
        assert_eq!(doc.bake_settings.quality, bake::Quality::Standard);
        assert!(!doc.bake_settings.match_names);
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
#[ignore = "requires GPU; opens and saves a model with multiple paint sets"]
fn gpu_paint_restores_saved_texture_set_and_its_selected_layer() {
    use crate::editor::{
        model_document::{MaterialSlot, Object, PAINT_SET_EXTRA, glb::Glb},
        model_paint_data::solid,
        model_primitives::Primitive,
    };
    use serde_json::json;

    let directory = std::env::temp_dir().join(format!(
        "bevy-paint-active-set-{}", bevy::asset::uuid::Uuid::new_v4(),
    ));
    let mut project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    copy_tree(&project.asset_root, &directory);
    project.asset_root = directory.clone();
    let path = directory.join("models/block.glb");

    let mut mesh = Primitive::Cube.geometry(&Primitive::Cube.defaults()).unwrap();
    for (index, face) in mesh.faces.iter_mut().enumerate() {
        face.material = index % 2;
    }
    let materials = (0..2).map(|set| MaterialSlot {
        material: Default::default(),
        source: json!({"material": 0}),
        paint: Some(Arc::new(Surface {
            size: 128,
            base: std::array::from_fn(|_| solid(128, [255; 4])),
            stack: Stack {
                layers: (0..2).map(|layer| Layer::new(format!("Слой {set}/{layer}"), LayerKind::Paint)).collect(),
                selected: set,
            },
            material: serde_json::Value::Null,
            baked: None,
        })),
        authored: None,
        bake_fixes: Default::default(),
    }).collect();
    let object = Object {
        id: 0,
        source_node: None,
        name: "Два набора".into(),
        geometry: mesh.into(),
        transform: Transform::IDENTITY,
        materials,
        hidden: false,
    };
    let bytes = Glb::new_asset_configured(&[object], &[], |root| {
        root["extras"][PAINT_SET_EXTRA] = json!(1);
        Ok(())
    }, None).unwrap();
    std::fs::write(&path, bytes).unwrap();

    let (mut app, _, _) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    let open = |app: &mut App| {
        app.world_mut().write_message(Action::ViewModel(0));
        for _ in 0..240 {
            frame(app);
            if app.world().resource::<editing::State>().documents.contains_key(&0) {
                break;
            }
        }
        app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(
            editing::EditAction::Mode(Mode::Paint),
        )));
        warm(app, 10);
        paint_ready(app);
    };
    let selection = |app: &App| {
        let paint = app.world().resource::<PaintState>();
        let session = &paint.sessions[&0];
        (session.set, session.stack.selected, session.stack.layers[session.stack.selected].name.clone())
    };
    open(&mut app);
    assert_eq!(selection(&app), (1, 1, "Слой 1/1".into()));
    paint_message(&mut app, PaintAction::Set(0));
    assert_eq!(selection(&app), (0, 0, "Слой 0/0".into()));
    assert!(app.world().resource::<editing::State>().available(editing::EditAction::Save, Some(0)));
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Save)));
    let start = std::time::Instant::now();
    loop {
        frame(&mut app);
        let doc = &app.world().resource::<editing::State>().documents[&0];
        if doc.source.json["extras"][PAINT_SET_EXTRA] == 0 {
            break;
        }
        assert!(start.elapsed().as_secs() < 45, "{}", app.world().resource::<Editor>().status);
    }
    assert_eq!(Glb::load(&path).unwrap().json["extras"][PAINT_SET_EXTRA], 0);
    paint_message(&mut app, PaintAction::Set(1));
    assert_eq!(selection(&app), (1, 1, "Слой 1/1".into()));
    app.world_mut().write_message(Action::CloseModelTab(0));
    warm(&mut app, 8);
    assert!(!app.world().resource::<editing::State>().documents.contains_key(&0));
    open(&mut app);
    assert_eq!(selection(&app), (0, 0, "Слой 0/0".into()));
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires GPU; opens an imported camera, saves framing and restores it after reopening"]
fn gpu_paint_imported_camera_uses_its_fov_and_reopens_after_a_camera_only_save() {
    use crate::editor::model_document::{MODEL_CAMERA_EXTRA, ModelCamera, glb::Glb};
    use bevy::camera::Projection;

    let directory = std::env::temp_dir().join(format!("bevy-mpp-camera-{}", bevy::asset::uuid::Uuid::new_v4()));
    let mut project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    copy_tree(&project.asset_root, &directory);
    project.asset_root = directory.clone();
    let path = directory.join("models/block.glb");
    let imported = ModelCamera {
        center: [0.2, 0.5, -0.1],
        distance: 3.5,
        yaw: 0.9,
        pitch: -0.2,
        fov: 0.73,
        orthographic: false,
    };
    let mut source = Glb::load(&path).unwrap();
    source.json["extras"][MODEL_CAMERA_EXTRA] = serde_json::json!(imported);
    std::fs::write(&path, source.encode(&[], &[]).unwrap()).unwrap();

    let (mut app, window, _) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    let wait = |app: &mut App, condition: &dyn Fn(&World) -> bool| {
        let started = std::time::Instant::now();
        loop {
            frame(app);
            if condition(app.world()) {
                return;
            }
            assert!(started.elapsed().as_secs() < 30, "{}", app.world().resource::<Editor>().status);
        }
    };
    app.world_mut().write_message(Action::ViewModel(0));
    wait(&mut app, &|world| world.resource::<Viewer>().camera() == imported);
    let mut projection = app.world_mut().query_filtered::<&Projection, With<ViewCamera>>();
    let Projection::Perspective(lens) = projection.single(app.world()).unwrap() else {
        panic!("MPP perspective camera");
    };
    assert!((lens.fov - imported.fov).abs() < 1e-6);
    assert!(!app.world().resource::<editing::State>().documents[&0].dirty());

    let reframed = ModelCamera { distance: 2.75, center: [0.1, 0.3, 0.2], ..imported };
    app.world_mut().resource_mut::<Viewer>().restore_camera(reframed);
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Save)));
    wait(&mut app, &|world| {
        let doc = &world.resource::<editing::State>().documents[&0];
        !doc.model_camera_changed() && doc.model_camera == Some(reframed)
    });
    let saved = Glb::load(&path).unwrap();
    assert_eq!(saved.json["extras"][MODEL_CAMERA_EXTRA], serde_json::json!(reframed));

    app.world_mut().write_message(Action::CloseModelTab(0));
    wait(&mut app, &|world| !world.resource::<editing::State>().documents.contains_key(&0));
    app.world_mut().write_message(Action::ViewModel(0));
    wait(&mut app, &|world| world.resource::<Viewer>().camera() == reframed);

    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Mode(Mode::Paint))));
    paint_message(&mut app, PaintAction::Panel(Panel::Preview));
    let field = {
        let world = app.world_mut();
        world.query::<(Entity, &ui::PreviewValue)>().iter(world)
            .find_map(|(entity, value)| (value.0 == PreviewParameter::FieldOfView).then_some(entity)).unwrap()
    };
    scroll_to_paint_entity(&mut app, field);
    app.world_mut().resource_mut::<InputFocus>().set(field, bevy::input_focus::FocusCause::Auto);
    warm(&mut app, 2);
    app.world_mut().get_mut::<bevy::text::EditableText>(field).unwrap().editor_mut().set_text("65");
    fill_modifier(&mut app, window, KeyCode::Enter, Key::Enter, ButtonState::Pressed);
    fill_modifier(&mut app, window, KeyCode::Enter, Key::Enter, ButtonState::Released);
    let adapted = ModelCamera { fov: 65_f32.to_radians(), ..reframed };
    wait(&mut app, &|world| world.resource::<Viewer>().camera() == adapted);
    let Projection::Perspective(lens) = projection.single(app.world()).unwrap() else {
        panic!("Paint perspective camera");
    };
    assert!((lens.fov - adapted.fov).abs() < 1e-6);
    assert!(app.world().resource::<editing::State>().documents[&0].dirty());
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Wireframe)));
    warm(&mut app, 4);
    assert!(app.world().resource::<editing::State>().documents[&0].paint_uv.show_wire_3d);
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Save)));
    wait(&mut app, &|world| !world.resource::<editing::State>().documents[&0].dirty());
    let saved = Glb::load(&path).unwrap();
    assert_eq!(saved.json["extras"][MODEL_CAMERA_EXTRA], serde_json::json!(adapted));
    assert_eq!(saved.json["extras"][crate::editor::model_document::PAINT_UV_EXTRA]["show_wire_3d"], true);

    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        doc.paint_preview.height_strength = Some(2.);
        doc.paint_preview.use_baked_normal = true;
        doc.paint_preview.emission_strength = 9.;
        doc.paint_uv.show_wire = true;
        doc.paint_uv.show_seams_3d = true;
        doc.paint_post = Some(DocumentPost { exposure: 1., ..default() });
        doc.ui_revision += 1;
    }
    let reset = PaintAction::PreviewMaterial(PreviewEdit::Reset { model: 0 });
    warm(&mut app, 4);
    scroll_to_paint_control(&mut app, reset);
    paint_click(&mut app, window, reset);
    assert_eq!(app.world().resource::<Viewer>().camera(), ModelCamera { fov: FOV, ..reframed });
    let doc = &app.world().resource::<editing::State>().documents[&0];
    assert_eq!(doc.paint_preview.height_strength, Some(2.));
    assert!(doc.paint_preview.use_baked_normal);
    assert_eq!(doc.paint_preview.emission_strength, 1.);
    assert!(doc.paint_uv.show_wire);
    assert!(!doc.paint_uv.show_wire_3d && !doc.paint_uv.show_seams_3d);
    assert_eq!(doc.paint_post, None);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(feature = "mcp")]
fn request_live_paint_view(app: &mut App, settings: serde_json::Value) -> Result<serde_json::Value, String> {
    use bevy::ecs::system::RunSystemOnce;
    let args: mcp_state::PaintViewArgs = serde_json::from_value(settings).unwrap();
    app.world_mut().run_system_once(move |mut input: mcp_state::McpPreview,
        editor: Res<Editor>, mut editing: ResMut<editing::State>, mut paint: ResMut<PaintState>| {
        mcp_state::set_view(&mut input, &editor, &mut editing, &mut paint, args.clone())
    }).unwrap()
}

#[cfg(feature = "mcp")]
fn live_paint_state(app: &mut App) -> serde_json::Value {
    use bevy::ecs::system::RunSystemOnce;
    app.world_mut().run_system_once(|input: mcp_state::McpPreview, editor: Res<Editor>,
        editing: Res<editing::State>, paint: Res<PaintState>| {
        mcp_state::state(&editor, &editing, &paint, &input).unwrap()["view"]["live_preview"].clone()
    }).unwrap()
}

#[test]
#[ignore = "requires GPU; previews built-in mesh maps before a full bake"]
fn gpu_builtin_maps_preview_without_full_bake() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let (mut app, _, _) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    let started = std::time::Instant::now();
    while !app.world().resource::<editing::State>().documents.contains_key(&0) {
        frame(&mut app);
        assert!(started.elapsed().as_secs() < 12);
    }
    {
        let mut state = app.world_mut().resource_mut::<editing::State>();
        let doc = state.documents.get_mut(&0).unwrap();
        let corners = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y].into_iter().enumerate()
            .map(|(index, uv)| {
                let mut corner = Corner::new(index);
                corner.uv = uv;
                corner
            }).collect();
        doc.objects[0].geometry = Geometry {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y],
            faces: vec![Face { corners, material: 0, smooth: false }],
            ..default()
        }.into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.revision += 1;
    }
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Mode(Mode::Paint))));
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::Resolution(512));
    paint_message(&mut app, PaintAction::View(View::Split));
    paint_message(&mut app, PaintAction::Panel(Panel::Maps));
    let image_for = |world: &World, map| {
        let doc = &world.resource::<editing::State>().documents[&0];
        let session = &world.resource::<PaintState>().sessions[&0];
        let baked = world.resource::<bake::State>();
        assert!(!session.bake_ready);
        assert!(baked.maps(0, doc, session.binding.unwrap(), session.texture_size).is_none());
        baked.preview_for(0, doc, session, map).cloned()
    };
    let wait_image = |app: &mut App, map| {
        let started = std::time::Instant::now();
        loop {
            frame(app);
            let world = app.world();
            if let Some(handle) = image_for(world, map)
                && let Some(image) = world.resource::<Assets<Image>>().get(&handle)
                && image.width() == 256
            {
                return (handle, image.data.clone().unwrap());
            }
            let status = world.resource::<bake::State>().live_status(map);
            assert!(started.elapsed().as_secs() < 15, "{}: {status:?}", map.label());
        }
    };
    let ao = bake::BakeMap::Occlusion;
    #[cfg(feature = "mcp")]
    {
        use serde_json::json;
        let camera = request_live_paint_view(&mut app, json!({"model_id": 0})).unwrap()["camera"].clone();
        let invalid = request_live_paint_view(&mut app, json!({"model_id": 0, "yaw_degrees": 123,
            "live_preview": {"mode": "map", "map": "projected_normal"}}));
        assert!(invalid.is_err());
        assert_eq!(request_live_paint_view(&mut app, json!({"model_id": 0})).unwrap()["camera"], camera);
        assert_eq!(live_paint_state(&mut app)["mode"], "off");
        let started = request_live_paint_view(&mut app, json!({"model_id": 0,
            "live_preview": {"mode": "map", "map": "occlusion"}})).unwrap();
        assert_eq!(started["live_preview"]["status"], "pending");
        assert_eq!(started["bake_map"], "occlusion");
    }
    #[cfg(not(feature = "mcp"))]
    paint_message(&mut app, PaintAction::BakePreview(Some(ao)));
    let (ao_image, ao_pixels) = wait_image(&mut app, ao);
    assert!(ao_pixels.chunks_exact(4).any(|p| p[3] == 255 && p[0] > 0));
    #[cfg(feature = "mcp")]
    assert_eq!(live_paint_state(&mut app), serde_json::json!({"mode": "map",
        "map": "occlusion", "status": "ready", "resolution": 256, "error": null}));
    let normal = bake::BakeMap::WorldNormal;
    paint_message(&mut app, PaintAction::BakePreview(Some(normal)));
    let (normal_image, normal_pixels) = wait_image(&mut app, normal);
    assert_ne!(ao_image, normal_image);
    assert!(normal_pixels.chunks_exact(4).any(|p| p[3] == 255 && p[0] != p[2]));
    assert!(image_for(app.world(), ao).is_none());
    paint_message(&mut app, PaintAction::BakeLivePreview);
    assert!(image_for(app.world(), normal).is_none());
    paint_message(&mut app, PaintAction::BakeLivePreview);
    assert_ne!(wait_image(&mut app, normal).0, normal_image);

    paint_message(&mut app, PaintAction::AddMask(true));
    paint_message(&mut app, PaintAction::Generator(None, GeneratorEdit::Set(GeneratorKind::CustomMap)));
    paint_message(&mut app, PaintAction::Generator(None, GeneratorEdit::Weight(GeneratorInput::Sky, 100)));
    let paint_surface = |app: &App| {
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        let (object, slot) = world.resource::<PaintState>().sessions[&0].binding.unwrap();
        doc.objects.iter().find(|item| item.id == object).unwrap().materials[slot].paint.as_ref().unwrap().clone()
    };
    let persisted = paint_surface(&app);
    assert!(persisted.baked.is_none());
    paint_message(&mut app, PaintAction::BakePreview(None));
    paint_message(&mut app, PaintAction::BakeLiveMasks);
    let started = std::time::Instant::now();
    let (maps, mask_image, material) = loop {
        frame(&mut app);
        let world = app.world();
        let baked = world.resource::<bake::State>();
        let session = &world.resource::<PaintState>().sessions[&0];
        let doc = &world.resource::<editing::State>().documents[&0];
        if let Some((256, _)) = baked.masks.mask_digest()
            && let Some(handle) = baked.masks.mask(0, doc, session)
        {
            let images = world.resource::<Assets<Image>>();
            let pixels = images.get(&handle).unwrap().data.as_ref().unwrap();
            assert!(pixels.chunks_exact(4).any(|p| p[0] > 0));
            let material = baked.masks.material(0, doc, session).unwrap();
            let maps = baked.live_inputs_for(0, doc, session.binding.unwrap(), normal).unwrap().clone();
            assert_eq!(session.bake_view, None);
            break (maps, handle, material);
        }
        assert!(started.elapsed().as_secs() < 20, "{:?}", baked.live_status(normal));
    };
    for map in [Some(ao), Some(normal), None] {
        paint_message(&mut app, PaintAction::BakePreview(map));
        let world = app.world();
        let baked = world.resource::<bake::State>();
        let session = &world.resource::<PaintState>().sessions[&0];
        let doc = &world.resource::<editing::State>().documents[&0];
        let displayed = baked.live_inputs_for(0, doc, session.binding.unwrap(), map.unwrap_or(normal)).unwrap();
        assert!(Arc::ptr_eq(&maps, displayed));
        assert_eq!(baked.masks.mask(0, doc, session), Some(mask_image.clone()));
        assert_eq!(baked.masks.material(0, doc, session), Some(material.clone()));
    }
    let rays = bake::RayOptions { sky_cone: 250, ..default() };
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Paint(PaintAction::BakeRayOptions(rays))));
    frame(&mut app);
    {
        let world = app.world();
        let baked = world.resource::<bake::State>();
        let session = &world.resource::<PaintState>().sessions[&0];
        let doc = &world.resource::<editing::State>().documents[&0];
        let displayed = baked.live_inputs_for(0, doc, session.binding.unwrap(), normal).unwrap();
        assert!(Arc::ptr_eq(&maps, displayed));
        assert_eq!(baked.masks.mask(0, doc, session), Some(mask_image));
        assert_eq!(baked.masks.material(0, doc, session), Some(material));
        assert!(baked.live_status(normal).unwrap().starts_with("Обновление"));
    }
    let revision_started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        let baked = world.resource::<bake::State>();
        let session = &world.resource::<PaintState>().sessions[&0];
        let doc = &world.resource::<editing::State>().documents[&0];
        if let Some(next) = baked.live_inputs_for(0, doc, session.binding.unwrap(), normal)
            && next.size == 256 && !Arc::ptr_eq(&maps, next)
        {
            break;
        }
        assert!(revision_started.elapsed().as_secs() < 20, "{:?}", baked.live_status(normal));
    }
    assert!(Arc::ptr_eq(&persisted, &paint_surface(&app)));
    assert!(persisted.baked.is_none());
    paint_message(&mut app, PaintAction::BakeLiveMasks);
    assert!(app.world().resource::<bake::State>().masks.mask_digest().is_none());

    let insert_map_graph = |app: &mut App, map_value| {
        use bevy::render::renderer::{RenderDevice, RenderQueue};
        let world = app.world();
        let device = world.resource::<RenderDevice>();
        let queue = world.resource::<RenderQueue>();
        let mut graph = material_painter_graph::Graph::new("Живой градиент части");
        graph.match_set = true;
        let input = graph.add("mesh_map", [0., 0.]);
        graph.node_mut(input).unwrap().params.insert("map".into(), material_painter_graph::Value::MeshMap(map_value));
        let output = graph.add("output", [200., 0.]);
        assert!(graph.connect(input, output, "input"));
        let mut layer = material_graph::prepare_graph(
            graph.clone(), &Default::default(), 512, Default::default(), device, queue,
        ).unwrap();
        graph.node_mut(output).unwrap().params.insert(
            "usage".into(), material_painter_graph::Value::Enum(material_painter_graph::MASK_USAGE),
        );
        material_graph::apply_mask(
            &mut layer, graph, &Default::default(), 512, Default::default(), device, queue,
        ).unwrap();
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.insert_smart_material(vec![layer], session.insertion());
    };
    insert_map_graph(&mut app, 8);
    warm(&mut app, 8);
    paint_ready(&mut app);
    let saved_graph_surface = paint_surface(&app);
    assert!(saved_graph_surface.stack.layers.iter().any(|layer| layer.mask.as_ref().is_some_and(|mask| mask.graph.is_some())));
    paint_message(&mut app, PaintAction::Panel(Panel::Maps));
    paint_message(&mut app, PaintAction::BakePreview(Some(normal)));
    paint_message(&mut app, PaintAction::BakeLiveMasks);
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        let session = &world.resource::<PaintState>().sessions[&0];
        let doc = &world.resource::<editing::State>().documents[&0];
        let baked = world.resource::<bake::State>();
        if let Some((256, _)) = baked.masks.mask_digest()
            && let Some(mask) = baked.masks.mask(0, doc, session)
            && let Some(material) = baked.masks.material(0, doc, session)
        {
            let images = world.resource::<Assets<Image>>();
            let materials = world.resource::<Assets<StandardMaterial>>();
            let color = materials.get(&material).unwrap().base_color_texture.as_ref().unwrap();
            let live = baked.live_inputs_for(0, doc, session.binding.unwrap(), normal).unwrap();
            let sample = live.graph_plane(bake::BakeMap::PartGradient).unwrap();
            let expected = (sample(64 * 256 + 128), sample(192 * 256 + 128));
            assert!(expected.0.abs_diff(expected.1) > 80, "baked gradient: {expected:?}");
            let sample_rows = |handle: &Handle<Image>| {
                let image = images.get(handle).unwrap();
                let pixels = image.data.as_ref().unwrap();
                let at = |y: usize| pixels[(y * 256 + 128) * 4];
                (at(64), at(192))
            };
            let mask = sample_rows(&mask);
            let color = sample_rows(color);
            assert!(mask.0.abs_diff(mask.1) > 80, "live graph mask: {mask:?}, baked: {expected:?}");
            // The gradient is blended through its graph mask over the underlying material.
            assert!(color.1.saturating_sub(color.0) > 10, "live graph color: {color:?}, baked: {expected:?}");
            break;
        }
        assert!(started.elapsed().as_secs() < 25, "{:?}", baked.live_status(normal));
    }
    assert!(Arc::ptr_eq(&saved_graph_surface, &paint_surface(&app)));
    assert!(saved_graph_surface.baked.is_none());
    paint_message(&mut app, PaintAction::BakeLiveMasks);

    paint_message(&mut app, PaintAction::BakeCustomAdd(custom::Kind::Point));
    let point = app.world().resource::<editing::State>().documents[&0].custom_bakes.last().unwrap().id;
    insert_map_graph(&mut app, material_painter_graph::CUSTOM_MAP_VALUE + usize::try_from(point).unwrap());
    warm(&mut app, 8);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::Panel(Panel::Maps));
    #[cfg(feature = "mcp")]
    {
        let reply = request_live_paint_view(&mut app, serde_json::json!({"model_id": 0,
            "live_preview": {"mode": "material", "map": bake::BakeMap::Custom(point)}})).unwrap();
        assert_eq!(reply["live_preview"]["status"], "pending");
    }
    #[cfg(not(feature = "mcp"))]
    {
        paint_message(&mut app, PaintAction::BakePreview(Some(bake::BakeMap::Custom(point))));
        paint_message(&mut app, PaintAction::BakeLiveMasks);
    }
    let wait_live_graph = |app: &mut App, prior| {
        let started = std::time::Instant::now();
        loop {
            frame(app);
            let world = app.world();
            let baked = world.resource::<bake::State>();
            if let Some((256, digest)) = baked.masks.mask_digest() && prior != Some(digest) {
                return digest;
            }
            assert!(started.elapsed().as_secs() < 20, "{:?}", baked.live_status(bake::BakeMap::Custom(point)));
        }
    };
    let digest = wait_live_graph(&mut app, None);
    #[cfg(feature = "mcp")]
    {
        let live = live_paint_state(&mut app);
        assert_eq!(live["mode"], "material");
        assert_eq!(live["status"], "ready");
        assert_eq!(live["resolution"], 256);
        assert_eq!(live["material_ready"], true);
    }
    let original_graph = paint_surface(&app);
    paint_message(&mut app, PaintAction::BakeCustomAdjust(point, custom::Field::X, 1));
    assert_ne!(wait_live_graph(&mut app, Some(digest)), digest);
    assert!(Arc::ptr_eq(&original_graph, &paint_surface(&app)));
    paint_message(&mut app, PaintAction::BakeLiveMasks);
    paint_message(&mut app, PaintAction::BakeStart);
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        if app.world().resource::<PaintState>().sessions[&0].bake_ready {
            break;
        }
        assert!(started.elapsed().as_secs() < 25, "{}", app.world().resource::<Editor>().status);
    }
    paint_message(&mut app, PaintAction::BakePreview(Some(normal)));
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        let session = &world.resource::<PaintState>().sessions[&0];
        let doc = &world.resource::<editing::State>().documents[&0];
        let baked = world.resource::<bake::State>();
        if let Some(handle) = baked.preview_for(0, doc, session, normal)
            && let Some(image) = world.resource::<Assets<Image>>().get(handle)
        {
            assert_eq!(image.width(), 512);
            assert!(baked.live_status(normal).is_none());
            break;
        }
        assert!(started.elapsed().as_secs() < 12);
    }
    #[cfg(feature = "mcp")]
    assert_eq!(live_paint_state(&mut app)["status"], "baked");
}

#[test]
#[ignore = "requires GPU; uses live AO and high-poly normals on an unpainted model"]
fn gpu_live_bake_lights_the_unpainted_material() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};

    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let high = dir.path().join("tilted.obj");
    std::fs::write(&high, "v 0 0 -0.05\nv 1 0 0.05\nv 1 1 0.05\nv 0 1 -0.05\nf 1 2 3 4\n").unwrap();
    let (mut app, _, _) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    let start = std::time::Instant::now();
    while !app.world().resource::<editing::State>().documents.contains_key(&0) {
        frame(&mut app);
        assert!(start.elapsed().as_secs() < 12);
    }
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        let corners = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y].into_iter().enumerate()
            .map(|(index, uv)| {
                let mut corner = Corner::new(index);
                corner.uv = uv;
                corner
            }).collect();
        doc.objects[0].geometry = Geometry {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y],
            faces: vec![Face { corners, material: 0, smooth: false }],
            ..default()
        }.into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.bake_settings.high_poly = Some(high);
        doc.bake_settings.match_names = false;
        doc.paint_preview.use_baked_normal = true;
        doc.paint_preview.ao_strength = 0.5;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 12);
    paint_ready(&mut app);
    #[cfg(feature = "mcp")]
    {
        let reply = request_live_paint_view(&mut app, serde_json::json!({"model_id": 0, "layout": "two",
            "live_preview": {"mode": "material"}})).unwrap();
        assert_eq!(reply["layout"], "two");
        assert!(reply["live_preview"]["map"].is_null());
        request_live_paint_view(&mut app, serde_json::json!({"model_id": 0, "layout": "split"})).unwrap();
    }
    #[cfg(not(feature = "mcp"))]
    {
        paint_message(&mut app, PaintAction::Panel(Panel::Maps));
        paint_message(&mut app, PaintAction::BakeLiveMasks);
    }
    let start = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        let session = &world.resource::<PaintState>().sessions[&0];
        let baked = world.resource::<bake::State>();
        if let Some((256, _)) = baked.masks.mask_digest()
            && let Some(overlay) = baked.masks.material(0, doc, session)
        {
            let material = world.resource::<Assets<StandardMaterial>>().get(&overlay).unwrap();
            let images = world.resource::<Assets<Image>>();
            let normal = images.get(material.normal_map_texture.as_ref().unwrap()).unwrap();
            let ao = images.get(material.occlusion_texture.as_ref().unwrap()).unwrap();
            let maps = baked.live_inputs_for(0, doc, session.binding.unwrap(), bake::BakeMap::WorldNormal).unwrap();
            let center = 128 * 256 + 128;
            let baked_normal = maps.projected_tangent_normals().unwrap()(center).unwrap();
            assert!(baked_normal[0].abs_diff(128) >= 5);
            assert_eq!(&normal.data.as_ref().unwrap()[center * 4..center * 4 + 3], &baked_normal);
            let baked_ao = maps.rgba(bake::BakeMap::Occlusion)[center * 4];
            assert_eq!(ao.data.as_ref().unwrap()[center], ((255. + f32::from(baked_ao)) * 0.5).round() as u8);
            assert!(!session.bake_ready && doc.objects[0].materials[0].paint.is_none());
            break;
        }
        assert!(start.elapsed().as_secs() < 25, "{:?}", baked.live_status(bake::BakeMap::WorldNormal));
    }
    #[cfg(feature = "mcp")]
    {
        let reply = request_live_paint_view(&mut app, serde_json::json!({"model_id": 0,
            "live_preview": {"mode": "off"}})).unwrap();
        assert_eq!(reply["live_preview"]["status"], "off");
        frame(&mut app);
    }
    #[cfg(not(feature = "mcp"))]
    paint_message(&mut app, PaintAction::BakeLiveMasks);
    assert!(app.world().resource::<bake::State>().masks.mask_digest().is_none());
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
    paint_message(&mut app, PaintAction::BakeIdSource(bake::IdSource::Materials));
    let rays = bake::RayOptions {
        occlusion_samples: Some(113),
        ..Default::default()
    };
    paint_message(&mut app, PaintAction::BakeRayOptions(rays));
    paint_message(&mut app, PaintAction::Panel(Panel::Maps));
    paint_message(&mut app, PaintAction::BakeMapOptions);
    warm(&mut app, 4);
    for (name, value) in [
        ("Рисование: Конус обзора неба, °", "53,1"),
        ("Рисование: Сетка проекции, проб по оси", "6"),
        ("Рисование: Радиус кривизны, пикс.", "4,3"),
    ] {
        let field = {
            let world = app.world_mut();
            let mut inputs = world.query::<(Entity, &AccessibleLabel, &mut bevy::text::EditableText)>();
            let (entity, _, mut input) = inputs.iter_mut(world)
                .find(|(_, label, _)| label.0 == name).unwrap();
            input.editor_mut().set_text(value);
            entity
        };
        app.world_mut().resource_mut::<bevy::input_focus::InputFocus>()
            .set(field, bevy::input_focus::FocusCause::Auto);
        warm(&mut app, 3);
        app.world_mut().resource_mut::<bevy::input_focus::InputFocus>().clear();
        warm(&mut app, 4);
    }
    paint_message(&mut app, PaintAction::BakeAutoEdgeReach);
    let rays = bake::RayOptions { sky_cone: 531, ..rays };
    let surface = bake::SurfaceOptions {
        grid: Some(6),
        radius: 43,
        auto_edge_reach: true,
        ..Default::default()
    };
    paint_message(&mut app, PaintAction::BakeSelectAllMaps(false));
    paint_message(&mut app, PaintAction::BakeRange { front: true, percent: 17 });
    paint_message(&mut app, PaintAction::BakePreferFrontFaces);
    paint_message(&mut app, PaintAction::BakeAverageNormals);
    let doc = &app.world().resource::<editing::State>().documents[&0];
    assert_eq!(doc.bake_settings.quality, bake::Quality::Standard);
    assert_eq!(doc.bake_settings.id_source, bake::IdSource::Materials);
    assert_eq!(doc.bake_settings.rays, rays);
    assert_eq!(doc.bake_settings.surface, surface);
    assert_eq!(doc.bake_settings.map_bits, bake::MapSelection::BASIC.bits());
    assert_eq!(doc.bake_settings.front, 17);
    assert!(!doc.bake_settings.prefer_front_faces && !doc.bake_settings.average_normals);
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
    assert_eq!(projection.id_source, bake::IdSource::Materials);
    assert_eq!(projection.rays, rays);
    assert_eq!(projection.surface, surface);
    assert_eq!(projection.maps, bake::MapSelection::BASIC);
    assert_eq!(projection.front, 17);
    assert!(!projection.prefer_front_faces && !projection.average_normals);
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
    #[cfg(feature = "mcp")]
    {
        use bevy::ecs::system::RunSystemOnce;
        let toggle = |app: &mut App, show: Option<bool>| {
            let args: mcp_state::CageArgs = serde_json::from_value(serde_json::json!({"model_id": 0, "show": show})).unwrap();
            app.world_mut().run_system_once(move |input: mcp_state::McpPreview, editor: Res<Editor>,
                mut editing: ResMut<editing::State>, mut paint: ResMut<PaintState>| {
                mcp_state::cage(&input, &editor, &mut editing, &mut paint, args.clone())
            }).unwrap().unwrap()
        };
        assert_eq!(toggle(&mut app, Some(true))["visible"], true);
        let start = std::time::Instant::now();
        loop {
            frame(&mut app);
            let report = toggle(&mut app, None);
            if report["report"]["status"] == "ready" {
                let samples = ["covered", "tight", "missed", "overlapping"].map(|key| report["report"][key].as_u64().unwrap());
                assert!(samples.into_iter().sum::<u64>() > 0);
                break;
            }
            assert_ne!(report["report"]["status"], "failed", "{report}");
            assert!(start.elapsed().as_secs() < 30, "{report}");
        }
        assert_eq!(toggle(&mut app, Some(false))["report"]["status"], "hidden");
    }
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
                live: None,
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
#[ignore = "requires GPU; exports the material graph's native output textures through the canvas"]
fn gpu_graph_canvas_exports_output_pngs_with_native_height_precision() {
    use bevy::render::renderer::{RenderDevice, RenderQueue};
    use material_painter_graph::{Graph, USAGES, Value};

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
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 15);
    let mut graph = Graph::new("Sample Graph");
    graph.res = 48;
    graph.match_set = false;
    let uniform = graph.add("color", [0., 0.]);
    // Exactly representable in RGBA16Float, including after a second render pass.
    let channels = [0.125, 0.375, 0.8125];
    graph.node_mut(uniform).unwrap().params.insert("color".into(), Value::Color(channels));
    for usage in 0..USAGES.len() {
        let output = graph.add("output", [220., usage as f32 * 140.]);
        graph.node_mut(output).unwrap().params.insert("usage".into(), Value::Enum(usage));
        assert!(graph.connect(uniform, output, "input"));
    }
    let device = app.world().resource::<RenderDevice>().clone();
    let queue = app.world().resource::<RenderQueue>().clone();
    let layer = material_graph::prepare_graph(graph, &Default::default(), 256, Default::default(), &device, &queue).unwrap();
    {
        let mut state = app.world_mut().resource_mut::<PaintState>();
        let session = state.sessions.get_mut(&0).unwrap();
        session.insert_layer(layer);
        session.revision += 1;
    }
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::GraphCanvas);
    let before = app.world().resource::<PaintState>().sessions[&0].stack.clone();

    let folder = tempfile::tempdir().unwrap();
    let existing = folder.path().join("Sample_Graph_outputs");
    std::fs::create_dir(&existing).unwrap();
    let existing_map = existing.join("Sample_Graph_Height.png");
    std::fs::write(&existing_map, b"previous export").unwrap();
    app.world_mut().resource_mut::<PaintState>().graph_outputs_test_folder = Some(Some(folder.path().into()));
    paint_click(&mut app, window, PaintAction::ExportGraphOutputs);
    let start = std::time::Instant::now();
    while app.world().resource::<PaintState>().graph_output_export.is_some() {
        frame(&mut app);
        assert!(start.elapsed().as_secs() < 60, "Graph output export did not finish");
    }
    let directory = folder.path().join("Sample_Graph_outputs_2");
    assert!(directory.is_dir(), "{}", app.world().resource::<Editor>().status);
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), USAGES.len());
    assert_eq!(std::fs::read(existing_map).unwrap(), b"previous export");
    let quantized = channels.map(|sample| half::f16::from_f32(sample).to_f32());
    let open = |usage: &str| image::open(directory.join(format!("Sample_Graph_{usage}.png"))).unwrap();
    let height = open("Height");
    assert_eq!(height.color(), image::ColorType::L16);
    let pixel = height.into_luma16().get_pixel(47, 47).0[0];
    assert!((i32::from(pixel) - (quantized[0] * 65535.).round() as i32).abs() <= 1);
    assert_ne!(pixel % 257, 0);
    let rgb = quantized.map(|sample| (sample * 255.).round() as u8);
    for usage in ["BaseColor", "Emission", "Normal"] {
        let output = open(usage);
        assert_eq!(output.color(), image::ColorType::Rgb8);
        assert_eq!(output.into_rgb8().get_pixel(47, 47).0, rgb);
    }
    for usage in ["Mask", "Opacity", "Roughness", "Metallic", "Specular"] {
        let output = open(usage);
        assert_eq!(output.color(), image::ColorType::L8);
        assert_eq!(output.into_luma8().get_pixel(47, 47).0, [rgb[0]]);
    }
    let after = &app.world().resource::<PaintState>().sessions[&0].stack;
    assert!(crate::editor::model_paint_data::same_stack(&before, after));
}

#[test]
#[ignore = "requires a GPU with 4K material graph support"]
fn gpu_graph_resolution_canvas_saves_4k_and_follows_the_texture_set() {
    use bevy::render::renderer::{RenderDevice, RenderQueue};
    use crate::editor::model_document::glb::Glb;
    use material_graph::Edit;
    use material_painter_graph::{Graph, Value};

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
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 15);
    paint_message(&mut app, PaintAction::Resolution(512));
    let mut graph = Graph::new("Graph resolution");
    graph.res = 256;
    graph.match_set = false;
    let color = graph.add("color", [0., 0.]);
    graph.node_mut(color).unwrap().params.insert("color".into(), Value::Color([0.125, 0.375, 0.8125]));
    // Each channel at 4K fills one 128 MiB readback batch.
    for usage in [0, 3] {
        let output = graph.add("output", [220., usage as f32 * 80.]);
        graph.node_mut(output).unwrap().params.insert("usage".into(), Value::Enum(usage));
        assert!(graph.connect(color, output, "input"));
    }
    let device = app.world().resource::<RenderDevice>();
    let queue = app.world().resource::<RenderQueue>();
    assert_eq!(material_graph::render_limit(device), 4096);
    let layer = material_graph::prepare_graph(graph, &Default::default(), 512, Default::default(), device, queue).unwrap();
    {
        let mut state = app.world_mut().resource_mut::<PaintState>();
        let session = state.sessions.get_mut(&0).unwrap();
        assert_eq!(session.texture_size, 512);
        session.insert_layer(layer);
        session.revision += 1;
    }
    paint_message(&mut app, PaintAction::GraphCanvas);
    let choose = |app: &mut App, size| {
        for action in [PaintAction::GraphResolutionMenu, PaintAction::GraphEdit(Edit::Resolution(size))] {
            scroll_to_paint_control(app, action);
            paint_click(app, window, action);
        }
    };
    let wait_for_size = |app: &mut App, graph_size, set_size| {
        let start = std::time::Instant::now();
        loop {
            frame(app);
            let session = &app.world().resource::<PaintState>().sessions[&0];
            let fill = session.stack.layers[session.stack.selected].texture_fill.as_ref().unwrap();
            if !session.smart_loading && session.texture_size == set_size
                && [0, 3].into_iter().all(|channel| {
                    fill.images[channel].as_ref().unwrap().pixels().dimensions() == (graph_size, graph_size)
                })
            {
                break;
            }
            assert!(start.elapsed().as_secs() < 180, "{}", app.world().resource::<Editor>().status);
        }
        paint_ready(app);
    };
    choose(&mut app, Some(4096));
    wait_for_size(&mut app, 4096, 512);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resolution.glb");
    let doc = &app.world().resource::<editing::State>().documents[&0];
    std::fs::write(&path, doc.source.encode(&doc.objects, &doc.original).unwrap()).unwrap();
    let saved = Glb::load(&path).unwrap();
    let surface = saved.json["meshes"].as_array().unwrap().iter()
        .flat_map(|mesh| mesh["primitives"].as_array().into_iter().flatten())
        .find_map(|primitive| saved.paint_surface(primitive).unwrap()).unwrap();
    let fill = surface.stack.layers.iter().find_map(|layer| layer.texture_fill.as_deref()).unwrap();
    let saved_graph = material_graph::read(fill.graph.as_ref().unwrap()).unwrap();
    assert_eq!(saved_graph.res, 4096);
    assert!(!saved_graph.match_set);
    for channel in [0, 3] {
        let pixels = fill.images[channel].as_ref().unwrap().pixels();
        assert_eq!(pixels.dimensions(), (4096, 4096));
        assert_eq!(pixels.get_pixel(4095, 4095)[0], 32);
    }
    drop(surface);
    drop(saved);

    choose(&mut app, None);
    wait_for_size(&mut app, 512, 512);
    let session = &app.world().resource::<PaintState>().sessions[&0];
    assert!(material_graph::read(session.graph().unwrap()).unwrap().match_set);
    paint_message(&mut app, PaintAction::Resolution(1024));
    wait_for_size(&mut app, 1024, 1024);
}

#[test]
#[ignore = "requires GPU; exercises contextual search, linked placement and one-step undo"]
fn gpu_graph_canvas_palette_places_and_connects_nodes() {
    use bevy::{input_focus::InputFocus, picking::pointer::{PointerAction, PointerButton},
        render::renderer::{RenderDevice, RenderQueue}};
    use material_painter_graph::Graph;

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
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 15);
    let mut original = Graph::new("Palette test");
    original.res = 64;
    let color = original.add("color", [0., 0.]);
    let output = original.add("output", [220., 0.]);
    assert!(original.connect(color, output, "input"));
    let layer = material_graph::prepare_graph(original.clone(), &Default::default(), 512, Default::default(),
        app.world().resource::<RenderDevice>(), app.world().resource::<RenderQueue>()).unwrap();
    {
        let mut state = app.world_mut().resource_mut::<PaintState>();
        let session = state.sessions.get_mut(&0).unwrap();
        session.insert_layer(layer);
        session.revision += 1;
    }
    paint_message(&mut app, PaintAction::GraphCanvas);
    warm(&mut app, 8);
    let graph = |app: &App| material_graph::read(app.world().resource::<PaintState>()
        .sessions[&0].graph().unwrap()).unwrap();
    let tap = |app: &mut App, code, key: Key| {
        fill_modifier(app, window, code, key.clone(), ButtonState::Pressed);
        fill_modifier(app, window, code, key, ButtonState::Released);
        warm(app, 4);
    };
    let empty = |app: &mut App| labelled(app, "Холст графа материала").max - Vec2::splat(15.);
    let cursor = empty(&mut app);
    move_pointer(&mut app, window, cursor);
    tap(&mut app, KeyCode::Space, Key::Space);
    let field = search_graph_palette(&mut app, "Checker");
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(field));
    warm(&mut app, 4);
    tap(&mut app, KeyCode::Enter, Key::Enter);
    let added = graph(&app);
    assert_eq!(added.nodes.len(), 3);
    let checker = added.nodes.last().unwrap();
    assert_eq!(checker.def, "checker");
    assert!(checker.pos[0] > 220. && checker.pos[1] > 70., "{:?}", checker.pos);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(graph(&app), original);

    let cursor = empty(&mut app);
    move_pointer(&mut app, window, cursor);
    tap(&mut app, KeyCode::Tab, Key::Tab);
    assert_eq!(app.world().resource::<editing::State>().documents[&0].mode, Mode::Paint);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(field));
    tap(&mut app, KeyCode::Escape, Key::Escape);
    assert_ne!(app.world().resource::<InputFocus>().get(), Some(field));
    for action in [PointerAction::Press(PointerButton::Secondary), PointerAction::Release(PointerButton::Secondary)] {
        crate::editor::tests::lighting_ui_render::pointer(&mut app, window, cursor, action);
        warm(&mut app, 2);
    }
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(field));
    tap(&mut app, KeyCode::Escape, Key::Escape);

    let drag_wire = |app: &mut App, origin, drop| {
        move_pointer(app, window, origin);
        mouse(app, window, ButtonState::Pressed);
        move_pointer(app, window, drop - Vec2::splat(10.));
        move_pointer(app, window, drop);
        mouse(app, window, ButtonState::Released);
        warm(app, 5);
    };
    let from_color = |app: &mut App| {
        let origin = labelled(app, "Uniform Color · выход").center();
        let drop = empty(app);
        drag_wire(app, origin, drop);
    };
    from_color(&mut app);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(field));
    tap(&mut app, KeyCode::Escape, Key::Escape);
    assert_eq!(graph(&app), original);
    from_color(&mut app);
    search_graph_palette(&mut app, "Checker");
    warm(&mut app, 4);
    tap(&mut app, KeyCode::Enter, Key::Enter);
    assert_eq!(graph(&app), original);
    search_graph_palette(&mut app, "Invert");
    warm(&mut app, 4);
    tap(&mut app, KeyCode::Enter, Key::Enter);
    let settle = |app: &mut App| {
        let start = std::time::Instant::now();
        while app.world().resource::<PaintState>().sessions[&0].smart_loading {
            frame(app);
            assert!(start.elapsed().as_secs() < 60, "{}", app.world().resource::<Editor>().status);
        }
        paint_ready(app);
    };
    settle(&mut app);
    let linked = graph(&app);
    assert_eq!(linked.nodes.len(), 3);
    let invert = linked.nodes.last().unwrap();
    assert_eq!(invert.def, "invert");
    assert_eq!(invert.links["input"], color);
    assert_eq!(linked.node(output).unwrap().links["input"], color);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(graph(&app), original);
    paint_message(&mut app, PaintAction::Redo);
    assert_eq!(graph(&app), linked);

    let canvas = labelled(&mut app, "Холст графа материала");
    move_pointer(&mut app, window, canvas.center());
    paint_message(&mut app, PaintAction::GraphClearSelection);
    tap(&mut app, KeyCode::KeyF, Key::Character("f".into()));
    let input = |app: &mut App| labelled(app, "Output · input").center();
    let history = |app: &App| app.world().resource::<PaintState>().sessions[&0].undo.len();
    let undo_steps = history(&app);
    let port = input(&mut app);
    drag_wire(&mut app, port, port);
    assert_eq!(graph(&app), linked);
    assert_eq!(history(&app), undo_steps);

    let port = input(&mut app);
    let blank = empty(&mut app);
    move_pointer(&mut app, window, port);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, blank - Vec2::splat(10.));
    assert_eq!(app.world().resource::<ui::PaintUi>().pending_graph_source(), Some(color));
    tap(&mut app, KeyCode::Escape, Key::Escape);
    move_pointer(&mut app, window, blank);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(graph(&app), linked);
    assert_eq!(history(&app), undo_steps);
    assert_ne!(app.world().resource::<InputFocus>().get(), Some(field));

    let port = input(&mut app);
    let blank = empty(&mut app);
    drag_wire(&mut app, port, blank);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(field));
    assert_eq!(graph(&app), linked);
    tap(&mut app, KeyCode::Escape, Key::Escape);
    assert_eq!(graph(&app), linked);

    let port = input(&mut app);
    let target = labelled(&mut app, "Invert · input").center();
    drag_wire(&mut app, port, target);
    settle(&mut app);
    let moved = graph(&app);
    assert!(!moved.node(output).unwrap().links.contains_key("input"));
    assert_eq!(moved.node(invert.id).unwrap().links["input"], color);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(graph(&app), linked);

    let port = input(&mut app);
    let blank = empty(&mut app);
    drag_wire(&mut app, port, blank);
    search_graph_palette(&mut app, "Invert");
    warm(&mut app, 4);
    tap(&mut app, KeyCode::Enter, Key::Enter);
    settle(&mut app);
    let extended = graph(&app);
    assert_eq!(extended.nodes.len(), 4);
    assert!(!extended.node(output).unwrap().links.contains_key("input"));
    assert_eq!(extended.nodes.last().unwrap().links["input"], color);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(graph(&app), linked);
}

#[test]
#[ignore = "requires GPU; drives group selection, dragging and clipboard on the graph canvas"]
fn gpu_graph_canvas_groups_move_copy_paste_and_delete() {
    use bevy::render::renderer::{RenderDevice, RenderQueue};
    use material_painter_graph::Graph;

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
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 15);
    let mut graph = Graph::new("Graph selection");
    graph.res = 64;
    graph.match_set = false;
    let color = graph.add("color", [0., 0.]);
    let invert = graph.add("invert", [220., 0.]);
    let output = graph.add("output", [440., 0.]);
    assert!(graph.connect(color, invert, "input"));
    assert!(graph.connect(invert, output, "input"));
    let layer = material_graph::prepare_graph(
        graph.clone(), &Default::default(), 512, Default::default(),
        app.world().resource::<RenderDevice>(), app.world().resource::<RenderQueue>(),
    ).unwrap();
    {
        let mut state = app.world_mut().resource_mut::<PaintState>();
        let session = state.sessions.get_mut(&0).unwrap();
        session.insert_layer(layer);
        session.revision += 1;
    }
    paint_message(&mut app, PaintAction::GraphCanvas);
    warm(&mut app, 8);

    let stage = |app: &mut App| labelled(app, "Холст графа материала");
    let title_on_canvas = |app: &mut App, name: &str| {
        let bounds = stage(app);
        let world = app.world_mut();
        world.query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>().iter(world)
            .filter(|(label, node, _)| label.0 == name && node.size().min_element() > 0.)
            .map(|(_, node, transform)| transform.affine().translation * node.inverse_scale_factor())
            .find(|point| bounds.contains(*point))
            .unwrap_or_else(|| panic!("No title {name} on the graph canvas"))
    };
    let graph_of = |app: &App| {
        material_graph::read(app.world().resource::<PaintState>().sessions[&0].graph().unwrap()).unwrap()
    };
    let drag = |app: &mut App, from, to| {
        move_pointer(app, window, from);
        mouse(app, window, ButtonState::Pressed);
        move_pointer(app, window, to);
        mouse(app, window, ButtonState::Released);
        warm(app, 8);
    };
    let key = |app: &mut App, code, key: Key, ctrl| {
        if ctrl { fill_modifier(app, window, KeyCode::ControlLeft, Key::Control, ButtonState::Pressed); }
        fill_modifier(app, window, code, key.clone(), ButtonState::Pressed);
        fill_modifier(app, window, code, key, ButtonState::Released);
        if ctrl { fill_modifier(app, window, KeyCode::ControlLeft, Key::Control, ButtonState::Released); }
        warm(app, 4);
    };
    let settle = |app: &mut App| {
        let start = std::time::Instant::now();
        while app.world().resource::<PaintState>().sessions[&0].smart_loading {
            frame(app);
            assert!(start.elapsed().as_secs() < 60, "{}", app.world().resource::<Editor>().status);
        }
        warm(app, 8);
        paint_ready(app);
    };
    let box_select = |app: &mut App| {
        let rect = stage(app);
        drag(app, rect.min + Vec2::splat(6.), rect.max - Vec2::splat(6.));
    };
    box_select(&mut app);
    assert_eq!(app.world().resource::<ui::PaintUi>().graph_selection(), &[color, invert, output]);
    let title = format!("Output #{output}");
    let output_center = title_on_canvas(&mut app, &title);
    fill_modifier(&mut app, window, KeyCode::ShiftLeft, Key::Shift, ButtonState::Pressed);
    move_pointer(&mut app, window, output_center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    fill_modifier(&mut app, window, KeyCode::ShiftLeft, Key::Shift, ButtonState::Released);
    warm(&mut app, 4);
    assert_eq!(app.world().resource::<ui::PaintUi>().graph_selection(), &[color, invert]);
    let from = title_on_canvas(&mut app, &format!("Uniform Color #{color}"));
    drag(&mut app, from, from + Vec2::new(30., 20.));
    let moved = graph_of(&app);
    let delta = |id| Vec2::from_array(moved.node(id).unwrap().pos) - Vec2::from_array(graph.node(id).unwrap().pos);
    assert!(delta(color).length() > 5.);
    assert!(delta(color).distance(delta(invert)) < 0.001);
    assert_eq!(delta(output), Vec2::ZERO);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(graph_of(&app), graph);

    key(&mut app, KeyCode::KeyD, Key::Character("d".into()), true);
    settle(&mut app);
    let duplicated = graph_of(&app);
    let end = duplicated.next_id - 1;
    assert_eq!(duplicated.nodes.len(), 5);
    assert_eq!(duplicated.node(end).unwrap().links["input"], end - 1);
    assert_eq!(app.world().resource::<ui::PaintUi>().graph_selection(), &[end - 1, end]);
    key(&mut app, KeyCode::KeyC, Key::Character("c".into()), true);
    let pointer = stage(&mut app).center();
    move_pointer(&mut app, window, pointer);
    key(&mut app, KeyCode::KeyV, Key::Character("v".into()), true);
    settle(&mut app);
    let pasted = graph_of(&app);
    let end = pasted.next_id - 1;
    assert_eq!(pasted.nodes.len(), 7);
    assert_eq!(pasted.node(end).unwrap().links["input"], end - 1);
    key(&mut app, KeyCode::Delete, Key::Delete, false);
    settle(&mut app);
    assert_eq!(graph_of(&app).nodes, duplicated.nodes);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(graph_of(&app), pasted);

    paint_message(&mut app, PaintAction::Panel(Panel::Preview));
    let preset = shader_preview::presets().iter().position(|graph| graph.name == "Unlit").unwrap();
    paint_message(&mut app, PaintAction::PreviewShader(Some(preset)));
    paint_message(&mut app, PaintAction::GraphCanvas);
    warm(&mut app, 8);
    let shader = graph_of(&app);
    box_select(&mut app);
    assert_eq!(app.world().resource::<ui::PaintUi>().graph_selection().len(), shader.nodes.len());
    key(&mut app, KeyCode::KeyD, Key::Character("d".into()), true);
    settle(&mut app);
    assert_eq!(graph_of(&app).nodes.len(), 2 * shader.nodes.len());
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(graph_of(&app), shader);
}

#[test]
#[ignore = "requires GPU; opens model graphs, templates and saved materials from the catalogue"]
fn gpu_graph_catalogue_opens_layers_across_sets_and_starts_materials() {
    use crate::editor::{model_document::{MaterialSlot, Object, glb::Glb}, model_primitives::Primitive};
    use crate::editor::model_paint_data::solid;
    use material_painter_graph::{Graph, MASK_USAGE, Value};
    use serde_json::json;

    let files = tempfile::tempdir().unwrap();
    let mut project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let assets = files.path().join("assets");
    copy_tree(&project.asset_root, &assets);
    project.asset_root = assets.clone();
    let graph = |name: &str, usage| {
        let mut graph = Graph::new(name);
        graph.res = 64;
        let color = graph.add("color", [0., 0.]);
        let output = graph.add("output", [220., 0.]);
        graph.node_mut(output).unwrap().params.insert("usage".into(), Value::Enum(usage));
        assert!(graph.connect(color, output, "input"));
        material_graph::encode(&graph)
    };
    let bitmap = Arc::new(texture_fill::Bitmap::from_rgba("Preview.png".into(),
        image::RgbaImage::from_pixel(64, 64, image::Rgba([70, 120, 190, 255]))).unwrap());
    let fill = |name: &str| {
        let mut layer = Layer::new(name.into(), LayerKind::Fill);
        layer.edit_texture_fill(|fill| {
            fill.graph = Some(Arc::new(graph(name, 0)));
            fill.images[0] = Some(bitmap.clone());
        });
        layer.channels[0] = Some(solid(64, [70, 120, 190, 255]));
        layer
    };
    let mut child = fill("Nested material");
    child.depth = 1;
    let mut mask = Mask::new(solid(64, [255; 4]), Vec::new());
    mask.set_graph(Arc::new(crate::editor::model_paint_data::mask::GraphMask::new(
        Arc::new(graph("Nested mask", MASK_USAGE)), Arc::default(), Some(bitmap.clone()),
    )));
    child.mask = Some(Arc::new(mask));
    let mut folder = Layer::new("Folder".into(), LayerKind::Folder);
    folder.collapsed = true;
    let material = |layers, selected| MaterialSlot {
        material: Default::default(), source: json!({"material": 0}), authored: None,
        bake_fixes: Default::default(),
        paint: Some(Arc::new(Surface {
            size: 64, base: std::array::from_fn(|_| solid(64, [255; 4])),
            stack: Stack { layers, selected }, material: json!(null), baked: None,
        })),
    };
    let mut mesh = Primitive::Cube.geometry(&Primitive::Cube.defaults()).unwrap();
    for (index, face) in mesh.faces.iter_mut().enumerate() { face.material = index % 2; }
    let object = Object {
        id: 0, source_node: None, name: "Два набора".into(), geometry: mesh.into(),
        transform: Transform::IDENTITY, hidden: false,
        materials: vec![material(vec![child, folder], 1), material(vec![fill("Second set")], 0)],
    };
    std::fs::write(assets.join("models/block.glb"), Glb::new_asset(&[object]).unwrap()).unwrap();
    let library = files.path().join("library");
    material_graph::file::package::export_shelf(&library.join("graphs"),
        &graph("Saved gallery material", 0), &Default::default(), Some(&bitmap.bytes)).unwrap();

    let (mut app, window, render_target) = fixture_project(project);
    app.world_mut().resource_mut::<PaintState>().library.root = Some(library);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) { break; }
    }
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 15);
    paint_ready(&mut app);
    let object = app.world().resource::<editing::State>().documents[&0].objects[0].id;
    let open = |slot, layer, kind| PaintAction::OpenProjectGraph {
        model: 0, target: ProjectGraphTarget { binding: (object, slot), layer, kind },
    };
    let catalogue_click = |app: &mut App, action| {
        scroll_to_paint_control(app, action);
        paint_click(app, window, action);
    };
    let wait_for_graph = |app: &mut App, name: &str| {
        let start = std::time::Instant::now();
        loop {
            frame(app);
            let session = &app.world().resource::<PaintState>().sessions[&0];
            if session.graph_canvas_open() && session.graph_catalogue().is_none() && !session.smart_loading
                && session.graph().is_some_and(|payload| payload["name"].as_str() == Some(name))
            { break; }
            assert!(start.elapsed().as_secs() < 60, "{name} · {}", app.world().resource::<Editor>().status);
        }
        paint_ready(app);
    };
    let before_gallery = {
        let paint = app.world().resource::<PaintState>();
        let session = &paint.sessions[&0];
        (session.binding, session.set, session.stack.clone(), session.undo.len())
    };
    paint_click(&mut app, window, PaintAction::GraphPage(GraphPage::Home));
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].graph_catalogue(), Some(GraphPage::Home));
    let swatches = |app: &mut App| {
        let world = app.world_mut();
        let mut images = world.query::<(&ui::PresetSwatch, &ImageNode, &ComputedNode, &UiGlobalTransform)>();
        let mut visible: Vec<_> = images.iter(world).filter(|(_, _, node, _)| node.size().min_element() > 0.)
            .map(|(swatch, image, node, transform)| {
                let scale = node.inverse_scale_factor();
                (swatch.0, image.image.id(), Rect::from_center_size(transform.affine().translation * scale, node.size() * scale))
            }).collect();
        visible.sort_unstable_by_key(|(index, _, _)| *index);
        visible
    };
    let start = std::time::Instant::now();
    let first_visit = loop {
        frame(&mut app);
        let ready = swatches(&mut app);
        if ready.len() == material_graph::PRESETS.len() { break ready; }
        assert!(start.elapsed().as_secs() < 30, "Preset previews did not render");
    };
    let brick = first_visit[0].2;
    capture_when(&mut app, &render_target, &files.path().join("preset-preview.png"), |image| {
        let sample = |u, v| {
            let p = (brick.min + brick.size() * Vec2::new(u, v)).as_uvec2();
            image.get_pixel(p.x, p.y).0
        };
        let reference = sample(0.2, 0.25);
        [0.35, 0.55, 0.75].into_iter().any(|v| [0.2, 0.4, 0.6, 0.8].into_iter().any(|u| {
            sample(u, v)[0].abs_diff(reference[0]) > 12
        }))
    });
    paint_message(&mut app, PaintAction::GraphCanvas);
    paint_message(&mut app, PaintAction::GraphPage(GraphPage::Home));
    let reopened = swatches(&mut app);
    let asset_ids = |swatches: &[(usize, _, Rect)]| swatches.iter().map(|(index, id, _)| (*index, *id)).collect::<Vec<_>>();
    assert_eq!(asset_ids(&first_visit), asset_ids(&reopened));
    {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        assert_eq!((session.binding, session.set, session.undo.len()), (before_gallery.0, before_gallery.1, before_gallery.3));
        assert!(crate::editor::model_paint_data::same_stack(&session.stack, &before_gallery.2));
    }
    catalogue_click(&mut app, open(0, 0, ProjectGraphKind::Mask));
    wait_for_graph(&mut app, "Nested mask");
    {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        assert!(session.editing_mask && session.stack.listed(0));
        assert_eq!(session.stack.layers.len(), 2);
        assert!(session.undo.is_empty());
    }
    paint_message(&mut app, PaintAction::GraphEdit(material_graph::Edit::Move(2, IVec2::new(410, 100))));
    paint_click(&mut app, window, PaintAction::GraphPage(GraphPage::Project));
    catalogue_click(&mut app, open(1, 0, ProjectGraphKind::Fill));
    wait_for_graph(&mut app, "Second set");
    {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        assert_eq!(session.binding, Some((object, 1)));
        assert_eq!(session.stack.layers.len(), 1);
        assert!(!session.editing_mask && session.undo.is_empty());
    }
    paint_click(&mut app, window, PaintAction::GraphPage(GraphPage::Project));
    catalogue_click(&mut app, open(0, 0, ProjectGraphKind::Mask));
    wait_for_graph(&mut app, "Nested mask");
    {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        assert_eq!(session.undo.len(), 1);
        let restored = material_graph::read(session.graph().unwrap()).unwrap();
        assert_eq!(restored.node(2).unwrap().pos, [410., 100.]);
    }

    paint_click(&mut app, window, PaintAction::GraphPage(GraphPage::Project));
    catalogue_click(&mut app, open(0, 1, ProjectGraphKind::Folder));
    wait_for_graph(&mut app, "Folder");
    assert!(app.world().resource::<PaintState>().sessions[&0].active_folder_view().is_some());
    paint_click(&mut app, window, PaintAction::GraphPage(GraphPage::Project));
    catalogue_click(&mut app, open(0, 0, ProjectGraphKind::Fill));
    wait_for_graph(&mut app, "Nested material");
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].stack.layers.len(), 2);
    paint_message(&mut app, PaintAction::GraphPage(GraphPage::Home));
    let blank = material_graph::PRESETS.iter().position(|preset| preset.name == "Blank").unwrap();
    catalogue_click(&mut app, PaintAction::AddGraphMaterial(blank));
    wait_for_graph(&mut app, "Blank");
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].stack.layers.len(), 3);
    paint_message(&mut app, PaintAction::GraphPage(GraphPage::Home));
    catalogue_click(&mut app, PaintAction::OpenSavedGraph(0));
    wait_for_graph(&mut app, "Saved gallery material");
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].stack.layers.len(), 4);
    paint_message(&mut app, PaintAction::GraphPage(GraphPage::Home));
    catalogue_click(&mut app, PaintAction::AddGraphMask);
    wait_for_graph(&mut app, "Граф маски");
    let session = &app.world().resource::<PaintState>().sessions[&0];
    assert!(session.editing_mask);
    assert_eq!(session.stack.layers.len(), 4);
}

#[test]
#[ignore = "requires GPU; saves, reloads, drags, shares and opens a local graph without network access"]
fn gpu_community_graph_from_installed_package_applies_as_a_fill() {
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let root = std::env::temp_dir().join(format!("bevy-community-ui-{}", bevy::asset::uuid::Uuid::new_v4()));
    let entries = online::community::catalog(&serde_json::json!({"version": 1, "items": [
        {"id": "blue", "name": "Community blue", "kind": "graph", "path": "items/blue", "files": ["blue.mpgraph", "nodes/tint.wgsl"]}
    ]})).unwrap();
    let graph = serde_json::json!({"name": "Community blue", "res": 64, "next_id": 3, "nodes": [
        {"id": 1, "def": "user/tint", "pos": [0, 0]},
        {"id": 2, "def": "output", "pos": [200, 0], "links": {"input": 1}}
    ]});
    let shader = "// @name Community blue\n// @output color\nfn node(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.15, 0.25, 0.8, 1.0); }\n";
    online::community::install(&root, &entries[0], |url, _| {
        if url.ends_with(".wgsl") { Ok(shader.as_bytes().to_vec()) }
        else { serde_json::to_vec(&graph).map_err(|error| error.to_string()) }
    }).unwrap();
    let (mut app, window, _) = fixture_project(project);
    app.world_mut().resource_mut::<PaintState>().library.root = Some(root.clone());
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) { break; }
    }
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 12);
    paint_click(&mut app, window, PaintAction::Category(Category::Graphs));
    set_shelf_query(&mut app, "Community blue");
    let started = std::time::Instant::now();
    while app.world().resource::<PaintState>().community.records.is_empty() {
        frame(&mut app);
        assert!(started.elapsed().as_secs() < 30, "Installed community graph did not appear");
    }
    warm(&mut app, 4);
    paint_click(&mut app, window, PaintAction::CommunityGraph(0));
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let session = &app.world().resource::<PaintState>().sessions[&0];
        let layer = &session.stack.layers[session.stack.selected];
        if let Some(fill) = &layer.texture_fill
            && layer.name == "Community blue"
        {
            let graph = material_graph::read(fill.graph.as_ref().unwrap()).unwrap();
            let id = &graph.node(1).unwrap().def;
            assert!(node_library::is_embedded(id));
            assert!(fill.images[0].as_ref().unwrap().pixels().get_pixel(0, 0)[2] > 128);
            break;
        }
        assert!(started.elapsed().as_secs() < 60, "{}", app.world().resource::<Editor>().status);
    }
    paint_message(&mut app, PaintAction::GraphCanvas);
    paint_click(&mut app, window, PaintAction::SaveGraphShelf);
    let started = std::time::Instant::now();
    while app.world().resource::<PaintState>().graph_shelf.entries.is_empty() {
        frame(&mut app);
        assert!(started.elapsed().as_secs() < 30, "{}", app.world().resource::<Editor>().status);
    }
    let entry = &app.world().resource::<PaintState>().graph_shelf.entries[0];
    assert_eq!(entry.name, "Community blue");
    assert_eq!(entry.path.parent(), Some(root.join("graphs").as_path()));
    assert!(entry.thumbnail.as_ref().unwrap().pixels().get_pixel(0, 0)[2] > 128);
    paint_message(&mut app, PaintAction::GraphCanvas);
    paint_message(&mut app, PaintAction::Undo);
    assert!(!app.world().resource::<PaintState>().sessions[&0].stack.layers.iter()
        .any(|layer| layer.name == "Community blue"));

    app.world_mut().resource_mut::<PaintState>().graph_shelf = graph_shelf::Library::default();
    paint_click(&mut app, window, PaintAction::Category(Category::Graphs));
    let started = std::time::Instant::now();
    while app.world().resource::<PaintState>().graph_shelf.entries.is_empty() {
        frame(&mut app);
        assert!(started.elapsed().as_secs() < 30, "Local graph did not reload");
    }
    warm(&mut app, 4);
    let target = labelled(&mut app, "3D-вид модели").center();
    drag_shelf_item(&mut app, window, PaintAction::ApplySavedGraph(0), target);
    let wait_for_count = |app: &mut App, count| {
        let started = std::time::Instant::now();
        loop {
            frame(app);
            let session = &app.world().resource::<PaintState>().sessions[&0];
            if !session.smart_loading && session.stack.layers.iter().filter(|layer| layer.name == "Community blue").count() == count {
                break;
            }
            assert!(started.elapsed().as_secs() < 60, "{}", app.world().resource::<Editor>().status);
        }
        paint_ready(app);
    };
    wait_for_count(&mut app, 1);
    paint_click(&mut app, window, PaintAction::SavedGraphMenu(Some(0)));
    paint_click(&mut app, window, PaintAction::ShareSavedGraph(0));
    let started = std::time::Instant::now();
    while app.world().resource::<PaintState>().community_submission.is_some() {
        frame(&mut app);
        assert!(started.elapsed().as_secs() < 60, "Community share package did not finish");
    }
    let output = std::fs::read_dir(root.join("online/community/submissions"))
        .unwrap_or_else(|error| panic!("{error} · {}", app.world().resource::<Editor>().status))
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_some_and(|ext| ext == "zip"))
        .unwrap_or_else(|| panic!("{}", app.world().resource::<Editor>().status));
    let (shared, _) = material_graph::file::load(&output.path()).unwrap();
    assert_eq!(shared.name, "Community blue");
    assert_eq!(material_graph::library().find(&shared.node(1).unwrap().def).unwrap().source, shader);
    paint_click(&mut app, window, PaintAction::OpenSavedGraph(0));
    wait_for_count(&mut app, 2);
    assert!(app.world().resource::<PaintState>().sessions[&0].graph_canvas_open());
    paint_message(&mut app, PaintAction::GraphCanvas);
    paint_message(&mut app, PaintAction::Undo);
    paint_message(&mut app, PaintAction::Undo);
    assert!(!app.world().resource::<PaintState>().sessions[&0].stack.layers.iter().any(|layer| layer.name == "Community blue"));
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires GPU; adds one graph from the smart-material shelf and verifies published edits"]
fn gpu_smart_recipe_graph_shelf_adds_one_layer_with_editable_parameters() {
    use material_graph::Edit;

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
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 15);
    paint_message(&mut app, PaintAction::Resolution(256));
    paint_ready(&mut app);
    let before = app.world().resource::<PaintState>().sessions[&0].stack.layers.len();
    let recipe = 0;
    let name = SMART_MATERIALS[recipe].name;
    paint_click(&mut app, window, PaintAction::Category(Category::Smart));
    set_shelf_query(&mut app, name);
    paint_click(&mut app, window, PaintAction::SmartRecipeMenu(Some(recipe)));
    assert!(paint_button_rect(&mut app, PaintAction::AddSmartMaterial(recipe)).width() > 0.);
    paint_click(&mut app, window, PaintAction::AddSmartGraph(recipe));

    let wait_for_graph = |app: &mut App| {
        let started = std::time::Instant::now();
        loop {
            frame(app);
            let session = &app.world().resource::<PaintState>().sessions[&0];
            if !session.smart_loading
                && let Some(fill) = session.active_texture_fill()
                && let Some(payload) = &fill.graph
                && let Some(image) = &fill.images[0]
            {
                let graph = material_graph::read(payload).unwrap();
                let bytes = image.bytes.clone();
                paint_ready(app);
                return (graph, bytes);
            }
            assert!(started.elapsed().as_secs() < 90, "{}", app.world().resource::<Editor>().status);
        }
    };

    let (initial, color) = wait_for_graph(&mut app);
    {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        assert_eq!(session.stack.layers.len(), before + 1);
        assert_eq!(session.stack.layers[session.stack.selected].kind, LayerKind::Fill);
        assert_eq!(session.stack.layers[session.stack.selected].name, name);
    }
    let base_color = initial.params.iter().position(|param| param.group == "Oak" && param.name == "Цвет").unwrap();
    paint_message(&mut app, PaintAction::GraphEdit(Edit::AdjustParameter(base_color, 0, 1)));
    let (edited, edited_color) = wait_for_graph(&mut app);
    let library = material_graph::library();
    assert_ne!(initial.param_value(base_color, &library).unwrap().1, edited.param_value(base_color, &library).unwrap().1);
    assert_ne!(color, edited_color);

    let doc = &app.world().resource::<editing::State>().documents[&0];
    let bytes = doc.source.encode(&doc.objects, &doc.original).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("smart-graph.glb");
    std::fs::write(&path, bytes).unwrap();
    let glb = crate::editor::model_document::glb::Glb::load(&path).unwrap();
    let restored = glb.json["meshes"].as_array().unwrap().iter().flat_map(|mesh| {
        mesh["primitives"].as_array().into_iter().flatten()
    }).find_map(|primitive| glb.paint_surface(primitive).unwrap()).unwrap();
    let fill = restored.stack.layers.iter().find(|layer| layer.name == name)
        .unwrap().texture_fill.as_ref().unwrap();
    assert_eq!(fill.images[0].as_ref().unwrap().bytes, edited_color);
    let reopened = material_graph::read(fill.graph.as_ref().unwrap()).unwrap();
    assert_eq!(reopened.params[base_color], edited.params[base_color]);

    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(wait_for_graph(&mut app).1, color);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].stack.layers.len(), before);
}

#[test]
#[ignore = "requires GPU; uses shelf drag and the channel picker, then reopens the GLB"]
fn gpu_shelf_textures_drop_pick_and_restore_fill_channels() {
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
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 15);
    paint_click(&mut app, window, PaintAction::Add(LayerKind::Fill));
    paint_message(&mut app, PaintAction::Channel(4));
    paint_click(&mut app, window, PaintAction::Category(Category::Textures));
    set_shelf_query(&mut app, "tiles");

    let wait_for_map = |app: &mut App, channel: usize, texture: usize| {
        let started = std::time::Instant::now();
        let name = format!("{}.png", tips::texture_name(texture));
        let bitmap = loop {
            frame(app);
            let session = &app.world().resource::<PaintState>().sessions[&0];
            if let Some(bitmap) = session.active_texture_fill()
                .and_then(|fill| fill.images[channel].as_ref())
                .filter(|bitmap| bitmap.name == name) {
                break bitmap.clone();
            }
            assert!(started.elapsed().as_secs() < 45, "{}", app.world().resource::<Editor>().status);
        };
        paint_ready(app);
        bitmap
    };

    paint_click(&mut app, window, PaintAction::Grain(Some(0)));
    let roughness = PaintAction::FillTexturePicker(Some(1));
    scroll_to_paint_control(&mut app, roughness);
    let target = paint_button_rect(&mut app, roughness).center();
    drag_shelf_item(&mut app, window, PaintAction::Grain(Some(0)), target);
    let original = wait_for_map(&mut app, 1, 0);
    assert_eq!(original.pixels().dimensions(), (1024, 1024));

    scroll_to_paint_control(&mut app, roughness);
    paint_click(&mut app, window, roughness);
    let replacement = PaintAction::TextureToChannel(2, 1);
    scroll_to_paint_control(&mut app, replacement);
    paint_click(&mut app, window, replacement);
    let brick = wait_for_map(&mut app, 1, 2);
    assert_ne!(original.bytes, brick.bytes);
    {
        let session = &app.world().resource::<PaintState>().sessions[&0];
        assert_eq!(session.channel, 4);
        assert_eq!(session.grain, Some(0));
        assert!(session.active_texture_fill().unwrap().images[4].is_none());
    }
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(wait_for_map(&mut app, 1, 0).bytes, original.bytes);
    paint_message(&mut app, PaintAction::Redo);
    assert_eq!(wait_for_map(&mut app, 1, 2).bytes, brick.bytes);
    let remove = PaintAction::FillTextureRemove(1);
    scroll_to_paint_control(&mut app, remove);
    paint_click(&mut app, window, remove);
    assert!(app.world().resource::<PaintState>().sessions[&0].active_texture_fill().is_none());
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(wait_for_map(&mut app, 1, 2).bytes, brick.bytes);

    paint_click(&mut app, window, PaintAction::AddShelfTexture(0));
    let height = wait_for_map(&mut app, 3, 0);
    let fill = app.world().resource::<PaintState>().sessions[&0].active_texture_fill().unwrap();
    assert_eq!(fill.settings.projection, texture_fill::Projection::Triplanar);
    assert_eq!(fill.settings.tiling, 2.);
    assert_eq!(height.bytes, original.bytes);

    let doc = &app.world().resource::<editing::State>().documents[&0];
    let bytes = doc.source.encode(&doc.objects, &doc.original).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shelf-texture.glb");
    std::fs::write(&path, bytes).unwrap();
    let glb = crate::editor::model_document::glb::Glb::load(&path).unwrap();
    let restored = glb.json["meshes"].as_array().unwrap().iter().flat_map(|mesh| {
        mesh["primitives"].as_array().into_iter().flatten()
    }).find_map(|primitive| glb.paint_surface(primitive).unwrap()).unwrap();
    for (channel, expected) in [(1, brick), (3, height)] {
        let restored_map = restored.stack.layers.iter().filter_map(|layer| {
            layer.texture_fill.as_ref()?.images[channel].as_ref()
        }).next().unwrap();
        assert_eq!(restored_map.bytes, expected.bytes);
    }
}

#[test]
#[ignore = "requires GPU; opens, searches, clicks and drags the offline material shelf"]
fn gpu_downloaded_material_shelf_inserts_editable_layers_without_network() {
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = online::cache_in(temp.path());
    online::downloaded::fixture(&root, online::Provider::PolyHaven, "red-rock", "Red Rock");
    online::downloaded::fixture(&root, online::Provider::AmbientCg, "unrelated", "Other Rock");
    let (mut app, window, _) = fixture_project(project);
    app.world_mut().resource_mut::<PaintState>().library.root = Some(temp.path().to_path_buf());
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<editing::State>().documents.contains_key(&0) { break; }
    }
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 15);
    paint_click(&mut app, window, PaintAction::Category(Category::Downloaded));
    set_shelf_query(&mut app, "red terrain");
    let started = std::time::Instant::now();
    let action = loop {
        frame(&mut app);
        let world = app.world_mut();
        let mut cards = world.query::<(&Control, &Children)>();
        let mut matching = cards.iter(world).filter_map(|(control, children)| {
            let Action::ModelView(ModelViewAction::Paint(action @ PaintAction::ApplyDownloaded(_))) =
                control.action else { return None };
            let ready = children.iter().any(|child| {
                world.get::<ImageNode>(child)
                    .and_then(|node| world.resource::<Assets<Image>>().get(&node.image))
                    .is_some_and(|image| image.width() == 128)
            });
            Some((action, ready))
        });
        if let Some((action, true)) = matching.next() {
            assert!(matching.next().is_none(), "Search left unrelated materials visible");
            break action;
        }
        assert!(started.elapsed().as_secs() < 30, "Offline material preview did not load");
    };
    assert!(app.world().resource::<PaintState>().online.entries.is_empty());
    let wait_for_layers = |app: &mut App, count| {
        let started = std::time::Instant::now();
        loop {
            frame(app);
            let session = &app.world().resource::<PaintState>().sessions[&0];
            if !session.smart_loading
                && session.stack.layers.iter().filter(|layer| layer.name == "Red Rock").count() == count
            {
                break;
            }
            assert!(started.elapsed().as_secs() < 45, "{}", app.world().resource::<Editor>().status);
        }
        paint_ready(app);
    };

    paint_click(&mut app, window, action);
    wait_for_layers(&mut app, 1);
    let texture = {
        let stack = &app.world().resource::<PaintState>().sessions[&0].stack;
        let layer = &stack.layers[stack.selected];
        assert_eq!(layer.name, "Red Rock");
        let fill = layer.texture_fill.as_ref().unwrap();
        assert_eq!(fill.settings.projection, texture_fill::Projection::Triplanar);
        fill.images[0].as_ref().unwrap().bytes.clone()
    };
    paint_message(&mut app, PaintAction::Add(LayerKind::Folder));
    let folder = app.world().resource::<PaintState>().sessions[&0].stack.selected;
    scroll_to_paint_control(&mut app, PaintAction::Select(folder));
    let destination = paint_button_rect(&mut app, PaintAction::Select(folder)).center();
    drag_shelf_item(&mut app, window, action, destination);
    wait_for_layers(&mut app, 2);
    let stack = &app.world().resource::<PaintState>().sessions[&0].stack;
    assert_eq!(stack.layers[stack.selected].name, "Red Rock");
    assert_eq!(stack.layers[stack.selected].depth, 1);
    assert!(stack.valid_hierarchy());

    let doc = &app.world().resource::<editing::State>().documents[&0];
    let bytes = doc.source.encode(&doc.objects, &doc.original).unwrap();
    let path = temp.path().join("offline-fill.glb");
    std::fs::write(&path, bytes).unwrap();
    let glb = crate::editor::model_document::glb::Glb::load(&path).unwrap();
    let restored = glb.json["meshes"].as_array().unwrap().iter().flat_map(|mesh| {
        mesh["primitives"].as_array().into_iter().flatten()
    }).find_map(|primitive| glb.paint_surface(primitive).unwrap()).unwrap();
    let layer = restored.stack.layers.iter().find(|layer| layer.name == "Red Rock" && layer.depth == 1).unwrap();
    assert_eq!(layer.texture_fill.as_ref().unwrap().images[0].as_ref().unwrap().bytes, texture);
    paint_message(&mut app, PaintAction::Undo);
    let stack = &app.world().resource::<PaintState>().sessions[&0].stack;
    assert_eq!(stack.layers.iter().filter(|layer| layer.name == "Red Rock").count(), 1);

    let PaintAction::ApplyDownloaded(id) = action else { unreachable!() };
    paint_click(&mut app, window, PaintAction::DownloadedMenu(Some(id)));
    paint_button_rect(&mut app, PaintAction::OpenDownloadedSource(id));
    paint_button_rect(&mut app, PaintAction::ShowDownloadedFiles(id));
    paint_click(&mut app, window, PaintAction::DeleteDownload(id));
    let started = std::time::Instant::now();
    while app.world().resource::<PaintState>().downloaded.record(id).is_some() {
        frame(&mut app);
        assert!(started.elapsed().as_secs() < 30, "{}", app.world().resource::<Editor>().status);
    }
    assert!(!root.join("polyhaven-red-rock-1k").exists());
    assert!(root.join("ambientcg-unrelated-1k").is_dir());
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].stack.layers.iter()
        .filter(|layer| layer.name == "Red Rock").count(), 1);
}

#[test]
#[ignore = "requires HTTPS and GPU; downloads and applies a real Poly Haven material from the paint shelf"]
fn gpu_online_material_applies_as_an_editable_glb_fill() {
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
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap().mode = Mode::Paint;
    warm(&mut app, 12);
    paint_click(&mut app, window, PaintAction::Category(Category::Online));
    let started = std::time::Instant::now();
    let index = loop {
        frame(&mut app);
        let shelf = &app.world().resource::<PaintState>().online;
        if let Some(error) = &shelf.error { panic!("{error}"); }
        if let Some(index) = shelf.entries.iter().position(|entry| entry.matches("brick_wall_001")) { break index; }
        assert!(started.elapsed().as_secs() < 60, "Online shelf did not load");
    };
    set_shelf_query(&mut app, "brick_wall_001");
    let action = PaintAction::OnlineMaterial(online::Provider::PolyHaven, index);
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world_mut();
        let mut cards = world.query::<(&Control, &Children)>();
        let loaded = cards.iter(world).any(|(control, children)| {
            matches!(control.action, Action::ModelView(ModelViewAction::Paint(value)) if value == action)
                && children.iter().any(|child| {
                    world.get::<ImageNode>(child).and_then(|node| world.resource::<Assets<Image>>().get(&node.image))
                        .is_some_and(|image| image.width() == 128)
                })
        });
        if loaded { break; }
        assert!(started.elapsed().as_secs() < 90, "Online card thumbnail did not load");
    }
    paint_click(&mut app, window, action);
    let started = std::time::Instant::now();
    let (name, texture) = loop {
        frame(&mut app);
        let session = &app.world().resource::<PaintState>().sessions[&0];
        let layer = &session.stack.layers[session.stack.selected];
        if let Some(fill) = &layer.texture_fill
            && [0, 1, 3, 6].into_iter().all(|slot| fill.images[slot].is_some())
        {
            break (layer.name.clone(), fill.images[0].as_ref().unwrap().bytes.clone());
        }
        assert!(started.elapsed().as_secs() < 90, "{}", app.world().resource::<Editor>().status);
    };
    paint_ready(&mut app);
    let doc = &app.world().resource::<editing::State>().documents[&0];
    let bytes = doc.source.encode(&doc.objects, &doc.original).unwrap();
    let path = std::env::temp_dir().join(format!("bevy-online-fill-{}.glb", bevy::asset::uuid::Uuid::new_v4()));
    std::fs::write(&path, bytes).unwrap();
    let glb = crate::editor::model_document::glb::Glb::load(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let restored = glb.json["meshes"].as_array().unwrap().iter().flat_map(|mesh| {
        mesh["primitives"].as_array().into_iter().flatten()
    }).find_map(|primitive| glb.paint_surface(primitive).unwrap()).unwrap();
    let layer = restored.stack.layers.iter().find(|layer| layer.name == name).unwrap();
    assert_eq!(layer.texture_fill.as_ref().unwrap().images[0].as_ref().unwrap().bytes, texture);
    paint_message(&mut app, PaintAction::Undo);
    assert!(!app.world().resource::<PaintState>().sessions[&0].stack.layers.iter().any(|layer| layer.name == name));
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
    search_graph_palette(&mut app, "Checker");
    warm(&mut app, 4);
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
    click_label(&mut app, "Удалить выбранные узлы");
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

    let files = tempfile::tempdir().unwrap();
    app.world_mut().resource_mut::<ShaderMaker>().node_directory = Some(files.path().join("nodes"));
    let before = fill(&app);
    let before_graph = current_graph(&app);
    let undo_depth = app.world().resource::<PaintState>().sessions[&0].undo.len();
    paint_message(&mut app, PaintAction::GraphSelectNode(tiles.id));
    click_label(&mut app, "Редактировать копию узла");
    let started = std::time::Instant::now();
    let (copy_id, code) = loop {
        frame(&mut app);
        let session = &app.world().resource::<PaintState>().sessions[&0];
        let graph = current_graph(&app);
        let node = graph.node(tiles.id).unwrap();
        if session.panel == Panel::Shader && node.def.starts_with("user/") {
            let world = app.world_mut();
            let mut inputs = world.query::<(&AccessibleLabel, &bevy::text::EditableText)>();
            let code = inputs.iter(world).find(|(label, _)| label.0 == "Исходный код WGSL").unwrap().1.value().to_string();
            if code.contains("@name Tiles (копия)") {
                break (node.def.clone(), code);
            }
        }
        assert!(started.elapsed().as_secs() < 30, "{}", app.world().resource::<Editor>().status);
    };
    let after = fill(&app);
    let mut expected = before_graph.clone();
    expected.node_mut(tiles.id).unwrap().def = copy_id.clone();
    assert_eq!(current_graph(&app), expected);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].undo.len(), undo_depth + 1);
    assert!(Arc::ptr_eq(&before.images[3].as_ref().unwrap(), &after.images[3].as_ref().unwrap()));
    assert_eq!(after.graph.as_ref().unwrap()["bevy_wgsl_nodes"][&copy_id], code);
    settle(&mut app);
    let path = files.path().join("fork.glb");
    {
        let doc = &app.world().resource::<editing::State>().documents[&0];
        std::fs::write(&path, doc.source.encode(&doc.objects, &doc.original).unwrap()).unwrap();
    }
    let loaded = Glb::load(&path).unwrap();
    let saved = loaded.json["meshes"].as_array().unwrap().iter()
        .flat_map(|mesh| mesh["primitives"].as_array().into_iter().flatten())
        .find_map(|primitive| loaded.paint_surface(primitive).unwrap()).unwrap();
    let saved = saved.stack.layers.iter().find_map(|layer| layer.texture_fill.as_ref()).unwrap();
    assert_eq!(saved.graph, after.graph);
    click_label(&mut app, "Вернуться к графу");
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].panel, Panel::Layers);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(current_graph(&app), before_graph);
    paint_message(&mut app, PaintAction::Redo);
    assert_eq!(current_graph(&app), expected);
    paint_message(&mut app, PaintAction::GraphSelectNode(tiles.id));
    click_label(&mut app, "Открыть исходник WGSL");
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].panel, Panel::Shader);
    let edited = code.replace("return vec4<f32>(h * lum);", "return vec4<f32>(0.12);");
    assert_ne!(edited, code);
    {
        let world = app.world_mut();
        let mut inputs = world.query::<(&AccessibleLabel, &mut bevy::text::EditableText)>();
        let (_, mut source) = inputs.iter_mut(world).find(|(label, _)| label.0 == "Исходный код WGSL").unwrap();
        assert_eq!(source.value(), code.as_str());
        source.editor_mut().set_text(&edited);
    }
    click_label(&mut app, "Сохранить правки");
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        if labelled(&mut app, "Применить к графу").width() > 0. { break; }
        assert!(started.elapsed().as_secs() < 30, "{}", app.world().resource::<Editor>().status);
    }
    click_label(&mut app, "Применить к графу");
    settle(&mut app);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].panel, Panel::Layers);
    let updated = fill(&app);
    assert_eq!(updated.graph.as_ref().unwrap()["bevy_wgsl_nodes"][&copy_id], edited);
    assert_eq!(current_graph(&app), expected);
    assert_ne!(updated.images[3].as_ref().unwrap().bytes, after.images[3].as_ref().unwrap().bytes);
    let pinned = material_graph::read(saved.graph.as_ref().unwrap()).unwrap();
    let pinned_def = &pinned.node(tiles.id).unwrap().def;
    assert!(node_library::is_embedded(pinned_def));
    assert_eq!(material_graph::library().find(pinned_def).unwrap().source, code);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(fill(&app).graph, after.graph);
    assert_eq!(fill(&app).images[3].as_ref().unwrap().bytes, after.images[3].as_ref().unwrap().bytes);
    paint_message(&mut app, PaintAction::Redo);
    assert_eq!(fill(&app).graph, updated.graph);
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

    let payload = Arc::new(project_preview_shader(&edited.to_json(library)).unwrap());
    app.world_mut().resource_mut::<editing::State>().documents.get_mut(&0).unwrap()
        .preview_shader = Some(payload.clone());
    warm(&mut app, 2);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].preview_shader,
        Some(shader_preview::presets().len()));
    assert_eq!(graph(&app), edited);
    assert!(!app.world().resource::<PaintState>().sessions[&0].available(PaintAction::Undo));
    capture_when(&mut app, &target, &path, |image| preview(image) == dim);
    paint_message(&mut app, PaintAction::GraphEdit(Edit::Move(output, IVec2::new(900, 45))));
    {
        let doc = &app.world().resource::<editing::State>().documents[&0];
        let moved = material_graph::read(doc.preview_shader.as_ref().unwrap()).unwrap();
        assert_eq!(moved.node(output).unwrap().pos, [900., 45.]);
    }
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(app.world().resource::<editing::State>().documents[&0].preview_shader.as_ref(), Some(&payload));

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
    let coverage_id = presets.len() + 1;
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
    std::fs::create_dir(&directory).unwrap();
    let broken_node = directory.join("broken.wgsl");
    let broken = "// @name Broken\n// @unknown\nfn node(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.3); }\n";
    std::fs::write(&broken_node, broken).unwrap();
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
    let swatch = |app: &mut App, label: &str| {
        let world = app.world_mut();
        let children = world.query::<(&AccessibleLabel, &Children)>().iter(world)
            .find_map(|(name, children)| (name.0 == label).then_some(children))?;
        children.iter().find_map(|child| {
            world.get::<ui::ShaderSwatch>(child)?;
            let image = world.get::<ImageNode>(child)?.image.id();
            let node = world.get::<ComputedNode>(child)?;
            let transform = world.get::<UiGlobalTransform>(child)?;
            let scale = node.inverse_scale_factor();
            Some((image, Rect::from_center_size(transform.affine().translation * scale, node.size() * scale)))
        })
    };
    let wait_swatch = |app: &mut App, label: &str, previous| {
        let start = Instant::now();
        loop {
            frame(app);
            if let Some((asset, rect)) = swatch(app, label)
                && Some(asset) != previous
            {
                return (asset, rect);
            }
            assert!(start.elapsed().as_secs() < 45, "Не загрузилась миниатюра {label}");
        }
    };
    assert!(labelled(&mut app, "Галерея узлов WGSL").width() > 200.0);
    assert_eq!(labelled(&mut app, "Исходный код WGSL").width(), 0.0);
    let (generator, rect) = wait_swatch(&mut app, "Открыть узел · Генератор", None);
    assert!(swatch(&mut app, "Шум").is_some());
    assert!(swatch(&mut app, "Шейдинг").is_none());
    let files = tempfile::tempdir().unwrap();
    capture_when(&mut app, &target, &files.path().join("shader-gallery.png"), |image| {
        let sample = |u, v| {
            let point = (rect.min + rect.size() * Vec2::new(u, v)).as_uvec2();
            image.get_pixel(point.x, point.y)[0]
        };
        let reference = sample(0.25, 0.25);
        [0.3, 0.5, 0.7].into_iter().any(|v| [0.2, 0.4, 0.6, 0.8].into_iter().any(|u| {
            sample(u, v).abs_diff(reference) > 20
        }))
    });
    paint_message(&mut app, PaintAction::Panel(Panel::Layers));
    paint_message(&mut app, PaintAction::Panel(Panel::Shader));
    assert_eq!(swatch(&mut app, "Открыть узел · Генератор").unwrap().0, generator);
    wait(&mut app, "Исправить · broken.wgsl");
    click_label(&mut app, "Исправить · broken.wgsl");
    wait(&mut app, "unknown tag");
    let repaired = "// @name Repaired node\n// @output gray\nfn node(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.3); }\n";
    set_source(&mut app, repaired);
    wait(&mut app, "Текстурный узел собран");
    click_label(&mut app, "Сохранить правки");
    wait_swatch(&mut app, "Repaired node", None);
    assert_eq!(std::fs::read_to_string(&broken_node).unwrap(), repaired);
    assert!(material_graph::library().find("user/broken").is_some());
    click_label(&mut app, "Стартовая страница WGSL");
    click_label(&mut app, "Открыть узел · Фильтр");
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
    click_label(&mut app, "Стартовая страница WGSL");
    assert_eq!(labelled(&mut app, "Исходный код WGSL").width(), 0.0);
    click_label(&mut app, "Вернуться к исходнику");
    {
        let world = app.world_mut();
        let mut fields = world.query::<(&AccessibleLabel, &bevy::text::EditableText)>();
        let (_, code) = fields.iter(world).find(|(label, _)| label.0 == "Исходный код WGSL").unwrap();
        assert_eq!(code.value(), test_source);
    }
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
    let (published_swatch, _) = wait_swatch(&mut app, "Test texture", None);
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
        material_graph::Inputs::default(), device, queue,
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
    wait_swatch(&mut app, "Test texture", Some(published_swatch));
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

    paint_message(&mut app, PaintAction::Panel(Panel::Shader));
    click_label(&mut app, "Стартовая страница WGSL");
    let noise_card = {
        let world = app.world_mut();
        world.query::<(Entity, &AccessibleLabel)>().iter(world)
            .find(|(_, label)| label.0 == "Открыть узел · Noise").unwrap().0
    };
    scroll_to_paint_entity(&mut app, noise_card);
    click_label(&mut app, "Открыть узел · Noise");
    wait(&mut app, "Текстурный узел собран");
    let builtin = material_graph::library().find("noise").unwrap().clone();
    let mode = |app: &App| *app.world().get::<bevy::text::TextReadWriteMode>(source_field).unwrap();
    assert_eq!(mode(&app), bevy::text::TextReadWriteMode::ReadOnly);
    assert_eq!(source(&app), builtin.source);
    ctrl(&mut app, KeyCode::KeyA, "a");
    fill_modifier(&mut app, window, KeyCode::Backspace, Key::Backspace, ButtonState::Pressed);
    fill_modifier(&mut app, window, KeyCode::Backspace, Key::Backspace, ButtonState::Released);
    assert_eq!(source(&app), builtin.source);
    click_label(&mut app, "Создать редактируемую копию");
    wait(&mut app, "Копия узла создана");
    assert_eq!(mode(&app), bevy::text::TextReadWriteMode::Editable);
    let reopened = node_library::reopen(directory.clone());
    let copy = reopened.defs.iter().find(|def| def.name == "Noise (копия)").unwrap();
    assert!(copy.id.starts_with("user/"));
    assert_eq!(copy.body, builtin.body);
    assert_eq!(copy.category, "Custom");
    assert_eq!(source(&app), copy.source);
    let renamed = copy.source.replace("// @name Noise (копия)", "// @name My Noise");
    set_source(&mut app, &renamed);
    ctrl(&mut app, KeyCode::KeyS, "s");
    wait(&mut app, "Правки узла сохранены");
    let reopened = node_library::reopen(directory.clone());
    assert_eq!(reopened.find(&copy.id).unwrap().source, renamed);
    assert_eq!(material_graph::library().find("noise").unwrap().source, builtin.source);
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
        .upsert(entry);
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
#[ignore = "requires GPU; checks independent 2D and depth-tested 3D UV seam controls"]
fn gpu_paint_uv_seams_show_island_borders_in_2d_and_depth_test_in_3d() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    let (mut app, window, target) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    let start = std::time::Instant::now();
    while !app.world().resource::<editing::State>().documents.contains_key(&0) {
        frame(&mut app);
        assert!(start.elapsed().as_secs() < 20);
    }
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        let object = &mut doc.objects[0];
        let mut mesh = Geometry::default();
        for (slot, (extent, z)) in [(0.9, 0.25), (0.35, -0.25)].into_iter().enumerate() {
            let vertices = [Vec2::new(-extent, -extent), Vec2::new(extent, -extent),
                Vec2::new(extent, extent), Vec2::new(-extent, extent)];
            mesh.positions.extend(vertices.map(|vertex| vertex.extend(z)));
            let uvs = [Vec2::new(0.2, 0.8), Vec2::new(0.8, 0.8),
                Vec2::new(0.8, 0.2), Vec2::new(0.2, 0.2)];
            for triangle in [[0, 1, 2], [0, 2, 3]] {
                mesh.faces.push(Face {
                    corners: triangle.map(|index| {
                        let mut corner = Corner::new(4 * slot + index);
                        corner.uv = uvs[index];
                        corner
                    }).into(),
                    material: slot,
                    smooth: false,
                });
            }
        }
        object.materials.push(object.materials[0].clone());
        object.geometry = mesh.into();
        object.transform = Transform::IDENTITY;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    paint_ready(&mut app);
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::ZERO;
        viewer.distance = 4.;
        viewer.yaw = 0.;
        viewer.pitch = 0.;
        viewer.orthographic = false;
    }
    paint_message(&mut app, PaintAction::Panel(Panel::Preview));
    paint_message(&mut app, PaintAction::View(View::Three));
    let viewport = labelled(&mut app, "3D-вид модели");
    let points = {
        let viewer = app.world().resource::<Viewer>();
        [Vec3::new(0.9, 0., 0.25), Vec3::new(0.35, 0., -0.25)]
            .map(|point| editing::input::project(viewer, viewport, point).unwrap().round().as_uvec2())
    };
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Edit(editing::EditAction::Wireframe)));
    warm(&mut app, 4);
    assert!(app.world().resource::<editing::State>().documents[&0].paint_uv.show_wire_3d);
    let output = std::env::temp_dir().join(format!("bevy-uv-seams-{}", bevy::asset::uuid::Uuid::new_v4()));
    std::fs::create_dir(&output).unwrap();
    let base = capture_when(&mut app, &target, &output.join("before.png"), |_| true);
    let toggle_3d = PaintAction::PreviewMaterial(PreviewEdit::UvSeams3d);
    scroll_to_paint_control(&mut app, toggle_3d);
    paint_click(&mut app, window, toggle_3d);
    let changed = |image: &image::RgbaImage, p: UVec2| {
        (p.y - 3..=p.y + 3).any(|y| (p.x - 3..=p.x + 3).any(|x| {
            let pixel = image.get_pixel(x, y);
            let original = base.get_pixel(x, y);
            pixel[0] > pixel[1].saturating_add(45) &&
                pixel[1].abs_diff(original[1]) > 25
        }))
    };
    let image = capture_when(&mut app, &target, &output.join("seams-3d.png"), |image| changed(image, points[0]));
    assert!(!changed(&image, points[1]), "the front surface must occlude seams on the rear surface");
    {
        let world = app.world_mut();
        let mut query = world.query_filtered::<&ChildOf, With<uv_seams::Overlay>>();
        assert_eq!(query.iter(world).count(), 2);
        assert!(!world.resource::<editing::State>().documents[&0].paint_uv.show_seams_2d);
    }

    paint_message(&mut app, PaintAction::View(View::Split));
    let toggle_2d = PaintAction::PreviewMaterial(PreviewEdit::UvSeams2d);
    scroll_to_paint_control(&mut app, toggle_2d);
    paint_click(&mut app, window, toggle_2d);
    let start = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world_mut();
        let (node, image) = world.query_filtered::<(&Node, &ImageNode), With<ui::UvSeams2d>>()
            .single(world).unwrap();
        if node.display == Display::Flex {
            let map = world.resource::<Assets<Image>>().get(&image.image).unwrap();
            let data = map.data.as_ref().unwrap();
            let size = map.size().x;
            let x = size / 2;
            let border_y = (0.2 * (size - 1) as f32).round() as u32;
            assert!(data[((border_y * size + x) * 4 + 3) as usize] > 127);
            assert_eq!(data[((x * size + x) * 4 + 3) as usize], 0);
            let wire = world.query_filtered::<&Node, With<ui::UvWire>>().single(world).unwrap();
            assert_eq!(wire.display, Display::None);
            break;
        }
        assert!(start.elapsed().as_secs() < 20, "2D seams were not prepared");
    }
    scroll_to_paint_control(&mut app, toggle_3d);
    paint_click(&mut app, window, toggle_3d);
    warm(&mut app, 3);
    {
        let world = app.world_mut();
        assert_eq!(world.query_filtered::<Entity, With<uv_seams::Overlay>>().iter(world).count(), 0);
        let layer = world.query_filtered::<&Node, With<ui::UvSeams2d>>().single(world).unwrap();
        assert_eq!(layer.display, Display::Flex);
    }
    std::fs::remove_dir_all(output).unwrap();
}

#[test]
#[ignore = "requires GPU; checks rendered UV wireframes on two UDIM tiles and during viewport navigation"]
fn gpu_paint_uv_wire_overlay_tracks_udim_and_navigation() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    let (mut app, window, target) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    let start = std::time::Instant::now();
    while !app.world().resource::<editing::State>().documents.contains_key(&0) {
        frame(&mut app);
        assert!(start.elapsed().as_secs() < 20);
    }
    {
        let mut editing = app.world_mut().resource_mut::<editing::State>();
        let doc = editing.documents.get_mut(&0).unwrap();
        let object = &mut doc.objects[0];
        let mut mesh = Geometry::default();
        for (face, (offset, min, max)) in [
            (Vec2::ZERO, Vec2::new(0.18, 0.20), Vec2::new(0.63, 0.43)),
            (Vec2::new(2., -1.), Vec2::new(0.27, 0.62), Vec2::new(0.82, 0.88)),
        ].into_iter().enumerate() {
            mesh.positions.extend([Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y]
                .map(|point| point + Vec3::Z * face as f32));
            mesh.faces.push(Face {
                corners: [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
                    .into_iter().enumerate().map(|(i, uv)| {
                        let mut corner = Corner::new(face * 4 + i);
                        corner.uv = offset + uv;
                        corner
                    }).collect(),
                material: 0,
                smooth: false,
            });
        }
        object.geometry = mesh.into();
        crate::editor::model_document::udim::import(object, true);
        assert_eq!(object.materials[0].udim().unwrap().number(), 1001);
        assert_eq!(object.materials[1].udim().unwrap().number(), 1013);
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::View(View::Two));
    paint_message(&mut app, PaintAction::Panel(Panel::Preview));
    paint_message(&mut app, PaintAction::Channel(1));
    let toggle = PaintAction::PreviewMaterial(PreviewEdit::UvWire);
    scroll_to_paint_control(&mut app, toggle);
    paint_click(&mut app, window, toggle);
    paint_message(&mut app, PaintAction::PreviewMaterial(PreviewEdit::Value {
        model: 0, parameter: PreviewParameter::UvWireOpacity, value: 80.,
    }));

    fn rect(world: &World, entity: Entity) -> Rect {
        let node = world.get::<ComputedNode>(entity).unwrap();
        let pose = world.get::<UiGlobalTransform>(entity).unwrap();
        Rect::from_center_size(
            pose.affine().translation * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        )
    }

    let first_top = Vec2::new(0.4, 0.2);
    let second_top = Vec2::new(0.55, 0.62);
    let wire_ready = |app: &mut App, present: Vec2, absent: Vec2| {
        let start = std::time::Instant::now();
        loop {
            frame(app);
            let world = app.world_mut();
            let entity = world.query_filtered::<Entity, With<ui::UvWire>>().single(world).unwrap();
            let wire = world.get::<ImageNode>(entity).unwrap();
            let map = world.resource::<Assets<Image>>().get(&wire.image).unwrap();
            let opaque_at = |uv: Vec2| {
                let p = (uv * (map.size().as_vec2() - Vec2::ONE)).round().as_uvec2();
                let i = ((p.y * map.size().x + p.x) * 4 + 3) as usize;
                map.data.as_ref().and_then(|data| data.get(i)).is_some_and(|alpha| *alpha > 127)
            };
            if world.get::<Node>(entity).unwrap().display == Display::Flex && opaque_at(present) {
                assert!(!opaque_at(absent));
                assert_eq!(wire.color, Color::WHITE.with_alpha(0.8));
                let parent = world.get::<ChildOf>(entity).unwrap().parent();
                let clip = world.get::<bevy::ui::CalculatedClip>(entity).unwrap();
                assert_eq!(clip, world.get::<bevy::ui::CalculatedClip>(parent).unwrap());
                let canvas = rect(world, parent);
                let wire = rect(world, entity);
                assert!(wire.min.distance(canvas.min) < 1.);
                assert!(wire.max.distance(canvas.max) < 1.);
                return canvas;
            }
            assert!(start.elapsed().as_secs() < 20, "UV wireframe texture was not ready");
        }
    };
    let first = wire_ready(&mut app, first_top, second_top);
    let pixel = (first.min + first_top * first.size()).round().as_uvec2();
    let output = std::env::temp_dir().join(format!("bevy-uv-wire-{}", bevy::asset::uuid::Uuid::new_v4()));
    std::fs::create_dir(&output).unwrap();
    let has_gold = |image: &image::RgbaImage| {
        (pixel.y.saturating_sub(2)..=pixel.y + 2).any(|y| {
            let p = image.get_pixel(pixel.x, y);
            p[0].saturating_sub(p[2]) > 25
        })
    };
    capture_when(&mut app, &target, &output.join("uv-wire.png"), has_gold);
    paint_click(&mut app, window, toggle);
    capture_when(&mut app, &target, &output.join("uv-wire-hidden.png"), |image| !has_gold(image));
    std::fs::remove_dir_all(output).unwrap();
    paint_click(&mut app, window, toggle);
    paint_message(&mut app, PaintAction::Set(1));
    let second = wire_ready(&mut app, second_top, first_top);

    move_pointer(&mut app, window, second.center());
    app.world_mut().write_message(bevy::input::mouse::MouseWheel {
        window, unit: bevy::input::mouse::MouseScrollUnit::Line, x: 0., y: 5.,
        phase: bevy::input::touch::TouchPhase::Moved,
    });
    let zoomed = wire_ready(&mut app, second_top, first_top);
    assert!(zoomed.width() > second.width() * 1.5);
    let invert = PaintAction::PreviewMaterial(PreviewEdit::InvertScrollZoom);
    scroll_to_paint_control(&mut app, invert);
    paint_click(&mut app, window, invert);
    paint_message(&mut app, PaintAction::PreviewMaterial(PreviewEdit::Value {
        model: 0, parameter: PreviewParameter::ZoomSpeed, value: 2.,
    }));
    move_pointer(&mut app, window, second.center());
    app.world_mut().write_message(bevy::input::mouse::MouseWheel {
        window, unit: bevy::input::mouse::MouseScrollUnit::Line, x: 0., y: 2.5,
        phase: bevy::input::touch::TouchPhase::Moved,
    });
    let unzoomed = wire_ready(&mut app, second_top, first_top);
    assert!((unzoomed.width() - second.width()).abs() < 1.5);
    paint_message(&mut app, PaintAction::PreviewMaterial(PreviewEdit::ResetNavigation));
    assert_eq!(app.world().resource::<PaintState>().navigation_settings(), navigation::Settings::default());
    move_pointer(&mut app, window, second.center());
    app.world_mut().write_message(bevy::input::mouse::MouseWheel {
        window, unit: bevy::input::mouse::MouseScrollUnit::Line, x: 0., y: 5.,
        phase: bevy::input::touch::TouchPhase::Moved,
    });
    let zoomed = wire_ready(&mut app, second_top, first_top);
    let press = |app: &mut App, state| {
        app.world_mut().write_message(bevy::input::mouse::MouseButtonInput {
            window, button: MouseButton::Middle, state,
        });
        frame(app);
    };
    let delta = Vec2::new(23., 15.);
    press(&mut app, ButtonState::Pressed);
    move_pointer(&mut app, window, zoomed.center() + delta);
    press(&mut app, ButtonState::Released);
    let panned = wire_ready(&mut app, second_top, first_top);
    assert!(panned.center().distance(zoomed.center() + delta) < 2.);
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
    let alpha = |app: &App, slot: usize, uv: Vec2| {
        let doc = &app.world().resource::<editing::State>().documents[&0];
        let Some(surface) = doc.objects[0].materials[slot].paint.as_deref() else { return 0 };
        let pixel = (uv * surface.size as f32).as_uvec2();
        let index = (pixel.y * surface.size + pixel.x) as usize * 4 + 3;
        surface.stack.layers[0].channels[0].as_ref().map_or(0, |pixels| pixels[index])
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
    paint_click(&mut app, window, PaintAction::Set(0));
    paint_message(&mut app, PaintAction::Panel(Panel::Layers));
    {
        let world = app.world_mut();
        let mut controls = world.query::<(&Control, &crate::editor::ui::control_hints::Help)>();
        assert!(controls.iter(world).any(|(control, hint)| {
            matches!(control.action, Action::ModelView(ModelViewAction::Paint(PaintAction::Undo)))
                && hint.0.contains("Мазок кисти") && hint.0.contains("1013")
        }));
    }
    for (action, set, at, wanted_alpha) in [
        (PaintAction::Undo, 1, Vec2::new(0.7, 0.5), 0),
        (PaintAction::Undo, 0, Vec2::new(0.3, 0.5), 0),
        (PaintAction::Redo, 0, Vec2::new(0.3, 0.5), 255),
        (PaintAction::Redo, 1, Vec2::new(0.7, 0.5), 255),
    ] {
        paint_click(&mut app, window, action);
        assert_eq!(app.world().resource::<PaintState>().sessions[&0].set, set);
        assert_eq!(alpha(&app, set, at), wanted_alpha);
    }
    paint_message(&mut app, PaintAction::Panel(Panel::Sets));
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
    let padding = PaintAction::ExportPadding(export::Padding::Transparent);
    scroll_to_paint_control(&mut app, padding);
    paint_click(&mut app, window, padding);
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
    scroll_to_paint_control(&mut app, PaintAction::NewExportTemplate);
    paint_click(&mut app, window, PaintAction::NewExportTemplate);
    let export::Template::Custom(draft) = app.world().resource::<export::prefs::Store>().settings().template else {
        panic!("Новый шаблон не выбран");
    };
    assert_ne!(draft, recipe);
    assert!(app.world().resource::<export::prefs::Store>().recipe(draft).unwrap().maps.is_empty());
    for edit in [Edit::Add, Edit::Remove] {
        let action = PaintAction::EditExportTemplate { recipe: draft, output: 0, channel: 0, edit };
        scroll_to_paint_control(&mut app, action);
        paint_click(&mut app, window, action);
    }
    assert!(app.world().resource::<export::prefs::Store>().recipe(draft).unwrap().maps.is_empty());
    let delete_draft = PaintAction::DeleteExportTemplate(draft);
    scroll_to_paint_control(&mut app, delete_draft);
    paint_click(&mut app, window, delete_draft);
    let store = app.world().resource::<export::prefs::Store>();
    assert!(store.recipe(draft).is_none());
    assert_eq!(store.settings().template, export::Template::Custom(recipe));
    for action in [PaintAction::ExportFileMenu, PaintAction::ExportPreviewScope(true)] {
        scroll_to_paint_control(&mut app, action);
        paint_click(&mut app, window, action);
    }
    scroll_to_paint_control(&mut app, PaintAction::ApplyExportMeshName);
    {
        let world = app.world_mut();
        let mut fields = world.query_filtered::<&mut bevy::text::EditableText, With<export::MeshExportName>>();
        fields.single_mut(world).unwrap().editor_mut().set_text("Hero Model");
    }
    paint_click(&mut app, window, PaintAction::ApplyExportMeshName);
    {
        let world = app.world_mut();
        let mut labels = world.query::<&Text>();
        for tile in [1001, 1013] {
            assert!(labels.iter(world).any(|label|
                label.0.starts_with(&format!("Hero_Model_Example_{tile}.")) && label.0.contains(" · RGB · ")));
        }
    }
    for action in [PaintAction::ExportPreviewScope(false), PaintAction::ExportTemplateMenu] {
        scroll_to_paint_control(&mut app, action);
        paint_click(&mut app, window, action);
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
        PaintAction::ExportSize(Some(2048)),
        PaintAction::ExportPreviewScope(true),
    ] {
        scroll_to_paint_control(&mut app, action);
        paint_click(&mut app, window, action);
    }
    {
        let world = app.world_mut();
        let mut labels = world.query::<&Text>();
        for (tile, source_size) in [(1001, 512), (1013, 1024)] {
            let info = format!("_ORM.{tile}.tif · RGB · 2048 × 2048 · 16 бит · увеличение с {source_size} × {source_size}");
            assert!(labels.iter(world).any(|label| label.0.ends_with(&info)), "{info}");
        }
    }
    for action in [PaintAction::ExportPreviewScope(false), PaintAction::ExportSize(Some(512))] {
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
    let preview = {
        let world = app.world_mut();
        let mut labels = world.query::<&Text>();
        labels.iter(world).find_map(|label| {
            let (name, info) = label.0.split_once(" · ")?;
            (name.contains("_ORM.") && info == "RGB · 512 × 512 · 16 бит").then(|| name.to_owned())
        }).expect("The format panel previews the selected set's 16-bit ORM filename")
    };
    assert!(tiff.path.join(&preview).is_file());
    for tile in [1001, 1013] {
        let orm = std::fs::read_dir(&tiff.path).unwrap().map(|entry| entry.unwrap().path())
            .find(|path| path.to_string_lossy().ends_with(&format!("_ORM.{tile}.tif"))).unwrap();
        let orm = image::open(orm).unwrap();
        assert_eq!((orm.width(), orm.height()), (512, 512));
        assert_eq!(orm.color(), image::ColorType::Rgb16);
    }
    let destination = directory.join("Новые текстуры").join("Модель");
    let set_destination_field = |app: &mut App, value: &str| {
        let world = app.world_mut();
        let mut fields = world.query_filtered::<&mut bevy::text::EditableText, With<export::DestinationField>>();
        fields.single_mut(world).unwrap().editor_mut().set_text(value);
    };
    set_destination_field(&mut app, "relative");
    paint_message(&mut app, PaintAction::ApplyExportDestination);
    assert!(app.world().resource::<Editor>().status.contains("полный путь"));
    assert!(app.world().resource::<export::prefs::Store>().destination().is_none());

    let file_in_place_of_folder = tiff.path.join(&preview);
    set_destination_field(&mut app, &file_in_place_of_folder.to_string_lossy());
    paint_message(&mut app, PaintAction::ExportMaps(false));
    let start = std::time::Instant::now();
    loop {
        frame(&mut app);
        if matches!(
            app.world().resource::<export::Activity>().state,
            export::ActivityState::Finished { failed: true, folder: None, .. }
        ) {
            break;
        }
        assert!(start.elapsed().as_secs() < 10, "The invalid destination did not report a failure");
    }
    warm(&mut app, 2);
    {
        let world = app.world_mut();
        let mut labels = world.query::<&Text>();
        assert!(labels.iter(world).any(|label| label.0.starts_with("Не удалось завершить операцию")));
    }

    scroll_to_paint_control(&mut app, PaintAction::ApplyExportDestination);
    set_destination_field(&mut app, &format!("\"{}\"", destination.display()));
    paint_click(&mut app, window, PaintAction::ApplyExportDestination);
    assert_eq!(app.world().resource::<export::prefs::Store>().destination(), Some(destination.as_path()));
    assert!(!destination.exists());
    paint_message(&mut app, PaintAction::ExportMaps(false));
    let exported = destination.join("Hero_Model_textures").join(preview);
    let start = std::time::Instant::now();
    loop {
        frame(&mut app);
        let status = &app.world().resource::<Editor>().status;
        if status.starts_with("Экспортировано карт:") {
            assert!(exported.is_file());
            break;
        }
        assert!(start.elapsed().as_secs() < 30, "{status}");
    }
    warm(&mut app, 2);
    {
        let world = app.world_mut();
        let status = world.resource::<Editor>().status.clone();
        let mut labels = world.query::<&Text>();
        assert!(labels.iter(world).any(|label| label.0 == status));
        let mut controls = world.query::<(&Control, Option<&bevy::ui::InteractionDisabled>)>();
        assert!(controls.iter(world).any(|(control, disabled)| disabled.is_none()
            && matches!(control.action, Action::ModelView(ModelViewAction::Paint(PaintAction::RevealExportResult)))));
    }
    set_destination_field(&mut app, "");
    paint_message(&mut app, PaintAction::ApplyExportDestination);
    assert!(app.world().resource::<export::prefs::Store>().destination().is_none());

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
    scroll_to_paint_control(&mut app, PaintAction::BakeInspectCage);
    paint_click(&mut app, window, PaintAction::BakeInspectCage);
    assert!(app.world().resource::<PaintState>().sessions[&0].bake_inspect_cage);
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
    scroll_to_paint_control(&mut app, PaintAction::BakeInspectCage);
    paint_click(&mut app, window, PaintAction::BakeInspectCage);
    {
        let world = app.world_mut();
        assert_eq!(world.query_filtered::<Entity, With<bake::cage_preview::Shell>>().iter(world).count(), 0);
    }
    scroll_to_paint_control(&mut app, PaintAction::BakeStart);
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
    scroll_to_paint_control(&mut app, PaintAction::BakePaintCage);
    paint_click(&mut app, window, PaintAction::BakePaintCage);
    assert!(app.world().resource::<PaintState>().sessions[&0].bake_paint_cage);
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
    let click_bake_control = |app: &mut App, action| {
        scroll_to_paint_control(app, action);
        paint_click(app, window, action);
    };
    click_bake_control(&mut app, PaintAction::BakeQuality(bake::Quality::Standard));
    {
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        assert_eq!(world.resource::<PaintState>().sessions[&0].bake_projection.quality, bake::Quality::Standard);
        assert!(world.resource::<bake::State>().maps(0, doc, (object, 0), 512).is_none());
    }
    click_bake_control(&mut app, PaintAction::BakeMapOptions);
    click_bake_control(&mut app, PaintAction::BakeSelectAllMaps(false));
    click_bake_control(&mut app, PaintAction::BakeStart);
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
    click_bake_control(&mut app, PaintAction::BakeClearDiskCache);
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
    click_bake_control(&mut app, PaintAction::BakeStart);
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
    paint_message(&mut app, PaintAction::Panel(Panel::Sets));
    let resize = PaintAction::Resolution(1024);
    assert_eq!(surface(&app, 0).size, 512);
    assert!(app.world().resource::<bake::State>().maps(0, doc(&app), (object, 0), 512).is_some());
    scroll_to_paint_control(&mut app, resize);
    paint_click(&mut app, window, resize);
    assert_eq!(surface(&app, 0).size, 1024);
    {
        let world = app.world();
        let resized = world.resource::<bake::State>().maps(0, doc(&app), (object, 0), 1024)
            .expect("Changing the paint resolution must keep existing bakes");
        assert_eq!(resized.size, 1024);
        assert!(resized.has_map(bake::BakeMap::WorldNormal));
        assert!(resized.has_map(bake::BakeMap::Custom(custom_id)));
        assert_eq!(resized.rgba(bake::BakeMap::Custom(custom_id)).len(), 1024 * 1024 * 4);
    }
    std::fs::remove_file(cage).unwrap();
    std::fs::remove_file(source).unwrap();
    for results in [
        report.path, rerun.path, godot.path, tiff.path, baked.path, packed.path, high.path, minimum.path,
        directory.join("bake-cache"), destination.join("Hero_Model_textures"),
    ] {
        for entry in std::fs::read_dir(&results).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(results).unwrap();
    }
    std::fs::remove_dir(&destination).unwrap();
    std::fs::remove_dir(directory.join("Новые текстуры")).unwrap();
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
    let wireframe = app.world().resource::<editing::State>().documents[&0].paint_uv.show_wire_3d;
    for expected in [!wireframe, wireframe] {
        press(&mut app, KeyCode::KeyW, Key::Character("w".into()));
        assert_eq!(app.world().resource::<editing::State>().documents[&0].paint_uv.show_wire_3d, expected);
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
    assert_eq!(app.world().resource::<editing::State>().documents[&0].paint_uv.show_wire_3d, wireframe);
    right(&mut app, ButtonState::Released);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].undo.len(), undo);
}

#[test]
#[ignore = "requires GPU; exercises brush adjustment with real viewport input in 3D and UV"]
fn gpu_paint_brush_adjusts_in_3d_and_uv_without_moving_camera() {
    use bevy::{input::{mouse::MouseButtonInput, touch::{TouchInput, TouchPhase}}, window::WindowEvent};
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
    let uv_center = center::<engine::TextureCanvas>(&mut app);
    move_pointer(&mut app, window, Vec2::splat(8.));
    app.world_mut().resource_mut::<PaintState>().sessions.get_mut(&0).unwrap()
        .set_value(Parameter::Size, 790.);
    app.world_mut().write_message(WindowEvent::TouchInput(TouchInput {
        window, id: 1, phase: TouchPhase::Moved, position: uv_center, force: None,
    }));
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
        doc.paint_preview = crate::editor::model_document::PaintPreview {
            height_strength: Some(0.),
            emission_strength: 3.25,
            use_baked_normal: true,
            ao_strength: 0.5,
        };
        doc.auto_bake_pending = false;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    app.world_mut().resource_scope(|world, mut maps: Mut<bake::State>| {
        let doc = &world.resource::<editing::State>().documents[&0];
        maps.insert_test_preview(doc, (doc.objects[0].id, 0), 512);
    });
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
    let start = std::time::Instant::now();
    let baked_normal = loop {
        frame(&mut app);
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        let materials = world.resource::<Assets<StandardMaterial>>();
        let runtime = world.resource::<engine::Runtime>();
        if let Some(material) = runtime.working_material(0, doc.objects[0].id, 0)
            .and_then(|handle| materials.get(handle))
            && let Some(ao) = &material.occlusion_texture
        {
            let images = world.resource::<Assets<Image>>();
            assert_eq!(material.occlusion_channel, bevy::mesh::UvChannel::Uv0);
            assert_eq!(images.get(ao).unwrap().data.as_ref().unwrap()[0], 159);
            assert!(!doc.paint_pending && doc.objects[0].materials[0].paint.is_none());
            break material.normal_map_texture.clone().unwrap();
        }
        assert!(start.elapsed().as_secs() < 15, "Baked material preview was not prepared");
    };
    assert_eq!(read(&mut app, baked_normal, 0), [204, 128, 229, 255]);
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
    let anchor_error = (zoomed.min + anchor_uv * zoomed.size()).distance(anchor);
    // Pixel snapping affects both rectangles. The old rounding is magnified by the zoom.
    let scale = app.world().get::<Window>(window).unwrap().scale_factor();
    let snap_error = (1. + zoomed.width() / fitted.width()) * std::f32::consts::FRAC_1_SQRT_2 / scale;
    assert!(anchor_error <= snap_error, "UV zoom shifted its anchor by {anchor_error:.2}px");
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
    {
        let world = app.world();
        let runtime = world.resource::<engine::Runtime>();
        let session = &world.resource::<PaintState>().sessions[&0];
        let queue = world.resource::<engine::gpu::Queue>();
        let bytes = runtime.layer_gpu_bytes(0, session, queue);
        assert_eq!(bytes.first().copied(), Some(8 * 512 * 512 * 4));
        assert!(runtime.paint_gpu_bytes(0, queue, world.resource::<Assets<Image>>()) > bytes[0]);
    }
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
        assert_eq!(material.emissive, LinearRgba::new(3.25, 3.25, 3.25, 1.));
        [
            material.base_color_texture.clone().unwrap(),
            material.metallic_roughness_texture.clone().unwrap(),
            material.normal_map_texture.clone().unwrap(),
        ]
    };
    assert_eq!(read(&mut app, material_maps[0].clone(), center), [120, 50, 220, 89]);
    assert_eq!(read(&mut app, material_maps[1].clone(), center), [255, 102, 153, 255]);
    let height = current(&app).unwrap().composite(3);
    let edge = ((256 * 512 + 1)..(257 * 512 - 1))
        .find(|&i| height[(i - 1) * 4] != height[(i + 1) * 4])
        .expect("stroke must have a height slope");
    assert_eq!(read(&mut app, material_maps[2].clone(), edge * 4), [204, 128, 229, 255]);
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
        viewer.field_of_view = Some(0.64);
        viewer.orthographic = false;
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

    paint_message(&mut app, PaintAction::Panel(Panel::Preview));
    let edit = |app: &mut App, parameter: PreviewParameter, text: &str| {
        let entity = {
            let world = app.world_mut();
            let label = format!("Рисование: {}", parameter.label());
            world.query::<(Entity, &AccessibleLabel, &bevy::text::EditableText)>().iter(world)
                .find(|(_, name, _)| name.0 == label).unwrap().0
        };
        app.world_mut().resource_mut::<bevy::input_focus::InputFocus>()
            .set(entity, bevy::input_focus::FocusCause::Auto);
        frame(app);
        app.world_mut().get_mut::<bevy::text::EditableText>(entity).unwrap().editor_mut().set_text(text);
        fill_modifier(app, window, KeyCode::Enter, Key::Enter, ButtonState::Pressed);
        fill_modifier(app, window, KeyCode::Enter, Key::Enter, ButtonState::Released);
        paint_ready(app);
    };
    edit(&mut app, PreviewParameter::Height, "1,50");
    edit(&mut app, PreviewParameter::Emission, "4,75");
    edit(&mut app, PreviewParameter::Occlusion, "25.50");
    let normal_toggle = PaintAction::PreviewMaterial(PreviewEdit::BakedNormal);
    scroll_to_paint_control(&mut app, normal_toggle);
    paint_click(&mut app, window, normal_toggle);
    paint_ready(&mut app);
    let normal = {
        let world = app.world();
        let doc = &world.resource::<editing::State>().documents[&0];
        assert_eq!(doc.paint_preview.height_strength, Some(1.5));
        assert_eq!(doc.paint_preview.emission_strength, 4.75);
        assert!((doc.paint_preview.ao_strength - 0.255).abs() < 1e-6);
        assert!(!doc.paint_preview.use_baked_normal);
        assert_eq!(doc.objects[0].materials[0].paint.as_ref().unwrap().stack, expected_stack);
        let runtime = world.resource::<engine::Runtime>();
        let handle = runtime.working_material(0, doc.objects[0].id, 0).unwrap();
        let materials = world.resource::<Assets<StandardMaterial>>();
        let material = materials.get(handle).unwrap();
        assert_eq!(material.emissive, LinearRgba::new(4.75, 4.75, 4.75, 1.));
        let ao = world.resource::<Assets<Image>>().get(material.occlusion_texture.as_ref().unwrap()).unwrap();
        assert_eq!(ao.data.as_ref().unwrap()[0], 206);
        material.normal_map_texture.clone().unwrap()
    };
    assert_eq!(read(&mut app, normal, 0), [128, 128, 255, 255]);

    paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
    for channel in [0, 1, 2, 3, 4, 6, 7] {
        paint_message(&mut app, PaintAction::BrushChannel(channel));
    }
    paint_message(&mut app, PaintAction::MultiChannelBrush);
    paint_message(&mut app, PaintAction::Preset(0));
    let history = undo_len(&app);
    stroke(&mut app, point);
    assert_eq!(undo_len(&app), history + 1);
    let surface = current(&app).unwrap();
    let painted = &surface.stack.layers[surface.stack.selected];
    let preset = &MATERIALS[0];
    let gray = |value: f32| [(value * 100. * 2.55).round() as u8; 3];
    for (channel, expected) in [
        (0, preset.rgb()),
        (1, gray(preset.roughness)),
        (2, gray(preset.metallic)),
        (5, [51, 153, 255]),
    ] {
        let pixels = painted.channels[channel].as_ref().unwrap();
        assert_eq!(&pixels[side..side + 3], &expected);
        assert_eq!(pixels[side + 3], 255);
    }
    assert!([3, 4, 6, 7].into_iter().all(|channel| painted.channels[channel].is_none()));

    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        let world = app.world_mut();
        let mut labels = world.query::<(&AccessibleLabel, &Text)>();
        if labels.iter(world).any(|(label, text)| {
            label.0 == "Статистика рисования"
                && text.0.contains("GPU рисования")
                && text.0.contains("История")
                && text.0.contains("512²")
                && text.0.contains("2 треуг.")
        }) {
            break;
        }
        assert!(started.elapsed().as_secs() < 5, "No paint workspace diagnostics");
    }
    let world = app.world();
    let session = &world.resource::<PaintState>().sessions[&0];
    assert!(session.history_cache.bytes() > 0);
    assert!(world.resource::<engine::Runtime>().paint_gpu_bytes(
        0, world.resource::<engine::gpu::Queue>(), world.resource::<Assets<Image>>(),
    ) > 0);

    #[cfg(feature = "mcp")]
    {
        use bevy::ecs::system::RunSystemOnce;
        use serde_json::json;

        paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
        let layer = app.world().resource::<PaintState>().sessions[&0].stack.selected;
        let workspace = json!({"model_id": 0, "set": 0});
        let stroke_request = |app: &mut App, target: serde_json::Value, position: [f32; 2], mut brush: serde_json::Value| {
            brush["size_px"] = json!(28.0);
            brush["hardness_percent"] = json!(100.0);
            let args: mcp_state::StrokeArgs = serde_json::from_value(json!({
                "workspace": workspace,
                "target": target,
                "samples": [{"position": position, "pressure": 1.0}],
                "brush": brush,
            })).unwrap();
            app.world_mut().run_system_once(move |mut input: mcp_state::McpPaintInput,
                editor: Res<Editor>, mut editing: ResMut<editing::State>, mut paint: ResMut<PaintState>| {
                mcp_state::stroke(&mut input, &editor, &mut editing, &mut paint, args.clone())
            }).unwrap()
        };
        let uv_target = json!({"layer": layer, "space": "uv"});
        let viewport_target = json!({"layer": layer, "space": "viewport"});
        let before_mcp = undo_len(&app);
        assert!(stroke_request(&mut app, uv_target.clone(), [-0.1, 0.5], json!({"channels": {"base_color": [1, 2, 3]}})).is_err());
        let screen_position = ((point - rect.min) / rect.size()).to_array();
        let queued = stroke_request(&mut app, viewport_target.clone(), screen_position, json!({"channels": {"base_color": [17, 150, 240]}})).unwrap();
        assert_eq!(queued["paint_busy"], true);
        paint_ready(&mut app);
        assert_eq!(undo_len(&app), before_mcp + 1);
        let surface = current(&app).unwrap();
        let pixels = surface.stack.layers[layer].channels[0].as_ref().unwrap();
        assert_eq!(&pixels[side..side + 4], &[17, 150, 240, 255]);

        stroke_request(&mut app, uv_target.clone(), [0.5, 0.5], json!({"channels": {"base_color": [210, 60, 30]}})).unwrap();
        paint_ready(&mut app);
        let surface = current(&app).unwrap();
        let pixels = surface.stack.layers[layer].channels[0].as_ref().unwrap();
        assert_eq!(&pixels[center..center + 4], &[210, 60, 30, 255]);
        assert_eq!(&pixels[side..side + 4], &[17, 150, 240, 255]);

        let args: mcp_state::FillArgs = serde_json::from_value(json!({
            "workspace": workspace,
            "target": {"layer": layer, "space": "uv"},
            "position": [0.5, 0.5], "mode": "polygon",
            "brush": {"channels": {"roughness": 77}},
        })).unwrap();
        app.world_mut().run_system_once(move |mut input: mcp_state::McpPaintInput,
            editor: Res<Editor>, mut editing: ResMut<editing::State>, mut paint: ResMut<PaintState>| {
            mcp_state::fill(&mut input, &editor, &mut editing, &mut paint, args.clone())
        }).unwrap().unwrap();
        paint_ready(&mut app);
        assert_eq!(undo_len(&app), before_mcp + 3);
        let surface = current(&app).unwrap();
        let pixels = surface.stack.layers[layer].channels[1].as_ref().unwrap();
        assert_eq!(&pixels[center..center + 4], &[77, 77, 77, 255]);
        assert_eq!(&pixels[side..side + 4], &[77, 77, 77, 255]);
        paint_message(&mut app, PaintAction::Undo);
        assert!(current(&app).unwrap().stack.layers[layer].channels[1].is_none());

        {
            let mut paint = app.world_mut().resource_mut::<PaintState>();
            let session = paint.sessions.get_mut(&0).unwrap();
            session.multi_channel_brush = false;
            session.channel = 4;
        }
        stroke_request(&mut app, uv_target.clone(), [0.5, 0.5], json!({"opacity_percent": 25, "stroke_opacity_percent": 50})).unwrap();
        paint_ready(&mut app);
        let surface = current(&app).unwrap();
        let pixels = surface.stack.layers[layer].channels[4].as_ref().unwrap();
        assert_eq!(&pixels[center..center + 4], &[64, 64, 64, 128]);
        paint_message(&mut app, PaintAction::Undo);
        stroke_request(&mut app, uv_target, [0.5, 0.5], json!({"opacity_percent": 25})).unwrap();
        paint_ready(&mut app);
        let surface = current(&app).unwrap();
        let pixels = surface.stack.layers[layer].channels[4].as_ref().unwrap();
        assert_eq!(&pixels[center..center + 4], &[64, 64, 64, 255]);

        paint_message(&mut app, PaintAction::View(View::Two));
        let camera_before = app.world().resource::<Viewer>().camera();
        let history_before = undo_len(&app);
        let mut target = viewport_target;
        target["view"] = json!({"width": 720, "height": 480,
            "camera": {"center": [-0.5, 0.0, 0.0], "distance": 2.0,
                "yaw_degrees": 0.0, "pitch_degrees": 0.0, "orthographic": true}});
        let brush = json!({"channels": {"base_color": [2, 180, 44]}});
        let mut invalid = target.clone();
        invalid["view"]["width"] = json!(0);
        assert!(stroke_request(&mut app, invalid, [0.5, 0.5], brush.clone()).is_err());
        assert_eq!(undo_len(&app), history_before);
        stroke_request(&mut app, target.clone(), [0.5, 0.5], brush).unwrap();
        paint_ready(&mut app);
        assert_eq!(undo_len(&app), history_before + 1);
        let surface = current(&app).unwrap();
        let pixels = surface.stack.layers[layer].channels[0].as_ref().unwrap();
        let offset_pixel = (256 * 512 + 128) * 4;
        assert_eq!(&pixels[offset_pixel..offset_pixel + 4], &[2, 180, 44, 255]);
        assert_eq!(&pixels[center..center + 4], &[210, 60, 30, 255]);

        let args: mcp_state::FillArgs = serde_json::from_value(json!({
            "workspace": workspace, "target": target, "position": [0.5, 0.5],
            "mode": "polygon", "brush": {"channels": {"roughness": 61}},
        })).unwrap();
        app.world_mut().run_system_once(move |mut input: mcp_state::McpPaintInput,
            editor: Res<Editor>, mut editing: ResMut<editing::State>, mut paint: ResMut<PaintState>| {
            mcp_state::fill(&mut input, &editor, &mut editing, &mut paint, args.clone())
        }).unwrap().unwrap();
        paint_ready(&mut app);
        let surface = current(&app).unwrap();
        let pixels = surface.stack.layers[layer].channels[1].as_ref().unwrap();
        assert_eq!(&pixels[center..center + 4], &[61, 61, 61, 255]);

        let list_args: mcp_state::SurfacePartsArgs = serde_json::from_value(json!({
            "workspace": workspace, "limit": 1,
        })).unwrap();
        let (reply, result) = std::sync::mpsc::channel();
        app.world_mut().run_system_once(move |editor: Res<Editor>, editing: Res<editing::State>, paint: Res<PaintState>| {
            let reply = reply.clone();
            mcp_state::list_surface_parts(&editor, &editing, &paint, list_args.clone(),
                Box::new(move |result| { reply.send(result).unwrap(); }));
        }).unwrap();
        let list = result.recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!(list["total"], 1);
        assert_eq!(list["parts"][0]["index"], 0);
        assert_eq!(list["parts"][0]["triangles_in_active_set"], 2);

        let selection = json!({
            "workspace": list["workspace"], "topology": list["topology"], "layer": layer,
            "source": {"kind": "viewport", "position": [0.5, 0.5], "view": target["view"]},
        });
        let select = |app: &mut App, args| {
            let args: mcp_state::SurfaceArgs = serde_json::from_value(args).unwrap();
            app.world_mut().run_system_once(move |mut input: mcp_state::McpPaintInput,
                editor: Res<Editor>, mut editing: ResMut<editing::State>, mut paint: ResMut<PaintState>| {
                mcp_state::select_surface(&mut input, &editor, &mut editing, &mut paint, args.clone())
            }).unwrap()
        };
        let before_selection = undo_len(&app);
        let mut stale = selection.clone();
        let revision = stale["topology"]["geometry_revision"].as_u64().unwrap();
        stale["topology"]["geometry_revision"] = json!(revision + 1);
        assert!(select(&mut app, stale).is_err());
        assert_eq!(undo_len(&app), before_selection);
        select(&mut app, selection).unwrap();
        paint_ready(&mut app);
        assert_eq!(undo_len(&app), before_selection + 1);
        assert!(current(&app).unwrap().stack.layers[layer].mask.is_some());
        assert_eq!(app.world().resource::<Viewer>().camera(), camera_before);
        assert_eq!(app.world().resource::<PaintState>().sessions[&0].view, View::Two);
    }

    paint_message(&mut app, PaintAction::View(View::Two));
    paint_message(&mut app, PaintAction::Tool(PaintTool::Brush));
    paint_message(&mut app, PaintAction::Add(LayerKind::Paint));
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.multi_channel_brush = false;
        session.channel = 4;
        session.set_value(Parameter::BrushAlpha, 25.);
        session.set_value(Parameter::Opacity, 50.);
    }
    let point = uv_rect::<engine::TextureCanvas>(&mut app).center();
    let opacity_pixel = |app: &App| {
        let surface = current(app).unwrap();
        let layer = &surface.stack.layers[surface.stack.selected];
        layer.channels[4].as_ref().unwrap()[center..center + 4].to_vec()
    };
    stroke(&mut app, point);
    assert_eq!(opacity_pixel(&app), [64, 64, 64, 128]);
    paint_message(&mut app, PaintAction::Tool(PaintTool::Eraser));
    stroke(&mut app, point);
    assert_eq!(opacity_pixel(&app), [64, 64, 64, 64]);
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(opacity_pixel(&app), [64, 64, 64, 128]);
}

#[test]
#[ignore = "requires GPU; verifies pen pressure and undo through the UV viewport"]
fn gpu_pen_uses_pressure_in_uv_and_undoes_per_contact() {
    use bevy::{
        input::touch::{ForceTouch, TouchInput, TouchPhase},
        window::WindowEvent,
    };
    use crate::editor::model_mesh::{Corner, Face, Geometry};

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
        let corners = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y].into_iter().enumerate()
            .map(|(vertex, uv)| {
                let mut corner = Corner::new(vertex);
                corner.uv = uv;
                corner
            }).collect();
        doc.objects[0].geometry = Geometry {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y],
            faces: vec![Face { corners, material: 0, smooth: false }],
            ..default()
        }.into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.paint_default_size = 512;
        doc.auto_bake_pending = false;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 6);
    paint_ready(&mut app);
    paint_message(&mut app, PaintAction::View(View::Two));
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.size = 90.;
        session.hardness = 100.;
        session.pressure_flow = true;
        session.pressure_size = true;
    }
    let uv = {
        let world = app.world_mut();
        let (node, transform) = world.query_filtered::<
            (&ComputedNode, &UiGlobalTransform), With<engine::TextureCanvas>,
        >().single(world).unwrap();
        engine::node_rect(node, transform).unwrap()
    };
    let alpha = |app: &App, x: usize, y: usize| {
        let doc = &app.world().resource::<editing::State>().documents[&0];
        let surface = doc.objects[0].materials[0].paint.as_ref().unwrap();
        assert_eq!(surface.size, 512);
        let pixels = surface.stack.layers[surface.stack.selected].channels[0].as_ref().unwrap();
        pixels[(y * surface.size as usize + x) * 4 + 3]
    };
    for (id, y, force) in [(41, 170, 0.25), (42, 342, 1.)] {
        let position = uv.min + uv.size() * Vec2::new(0.5, (y as f32 + 0.5) / 512.);
        for phase in [TouchPhase::Started, TouchPhase::Ended] {
            app.world_mut().write_message(WindowEvent::TouchInput(TouchInput {
                phase, position, window, id,
                force: (phase == TouchPhase::Started).then_some(ForceTouch::Normalized(force)),
            }));
            frame(&mut app);
        }
        paint_ready(&mut app);
    }
    let wide_x = 256 + (32. * 512. / uv.width()).round() as usize;
    assert!(alpha(&app, 256, 170).abs_diff(64) <= 1);
    assert_eq!(alpha(&app, 256, 342), 255);
    assert_eq!(alpha(&app, wide_x, 170), 0, "light pressure shrinks the stamp");
    assert_eq!(alpha(&app, wide_x, 342), 255, "full pressure keeps the diameter");
    paint_message(&mut app, PaintAction::Undo);
    assert!(alpha(&app, 256, 170).abs_diff(64) <= 1);
    assert_eq!(alpha(&app, 256, 342), 0, "undo removes exactly one pen contact");
}

#[test]
#[ignore = "requires GPU; verifies all PBR channels, occlusion, opacity, erasing, fill and orthographic painting"]
fn gpu_model_paint_channels_erase_and_occlusion() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    use bevy::input::mouse::MouseButtonInput;
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
            s.opacity = if channel == 4 { 50. } else { 100. };
            s.brush_alpha = 25.;
            s.brush_height = 75.;
            s.brush_emission = [80., 15., 25.];
            s.brush_specular = 100.;
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
            0 => [164, 61, 68, 255],
            1 => [204, 204, 204, 255],
            2 => [153, 153, 153, 255],
            3 => [191, 191, 191, 255],
            4 => [64, 64, 64, 128],
            5 => [204, 38, 64, 255],
            6 => [128, 128, 255, 255],
            _ => [255; 4],
        };
        assert_eq!(&p[front..front + 4], &expected, "GPU channel {channel}");
        assert_eq!(p[back + 3], 0, "GPU stroke must not paint the rear plane");
    }
    paint_message(&mut app, PaintAction::Channel(4));
    paint_click(&mut app, window, PaintAction::Tool(PaintTool::Eraser));
    app.world_mut().resource_mut::<PaintState>().sessions.get_mut(&0).unwrap().opacity = 50.;
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    let alpha = |app: &App| {
        let doc = &app.world().resource::<editing::State>().documents[&0];
        let surface = doc.objects[0].materials[0].paint.as_ref().unwrap();
        let side = surface.size as usize;
        let index = (side / 2 * side + side / 4) * 4;
        surface.stack.layers[0].channels[4].as_ref().unwrap()[index..index + 4].to_vec()
    };
    assert_eq!(alpha(&app), [64, 64, 64, 64]);
    paint_click(&mut app, window, PaintAction::Undo);
    assert_eq!(alpha(&app), [64, 64, 64, 128]);
    app.world_mut().resource_mut::<PaintState>().sessions.get_mut(&0).unwrap().opacity = 100.;
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
    scroll_to_paint_control(&mut app, PaintAction::Color(4));
    paint_click(&mut app, window, PaintAction::Color(4));
    paint_click(&mut app, window, PaintAction::Tool(PaintTool::Fill));
    scroll_to_paint_control(&mut app, PaintAction::FillMode(FillMode::Triangle));
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
    scroll_to_paint_control(&mut app, PaintAction::Color(3));
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
    let texture_canvas = |app: &mut App| {
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
    let uv = texture_canvas(&mut app);
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

    paint_message(&mut app, PaintAction::BrushTip(None));
    let brush_alpha = |app: &App, x: usize, y: usize| {
        let p = app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
            .paint
            .as_ref()
            .unwrap();
        let pixels = p.stack.layers[p.stack.selected].channels[0]
            .as_ref()
            .unwrap_or_else(|| {
                let session = &app.world().resource::<PaintState>().sessions[&0];
                panic!(
                    "missing color data for texel ({x}, {y}) on layer {} ({}), view {:?}, tool {:?}, symmetry {:?}",
                    p.stack.selected, p.stack.layers[p.stack.selected].name,
                    session.view, session.tool, session.symmetry,
                )
            });
        pixels[(y * p.size as usize + x) * 4 + 3]
    };
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
    let uv = texture_canvas(&mut app);
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
    // Winit emits aggregate cursor motion alongside the typed event.
    app.world_mut().write_message(bevy::window::WindowEvent::CursorMoved(bevy::window::CursorMoved {
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

    let model_view = labelled(&mut app, "3D-вид модели");
    let viewer = app.world().resource::<Viewer>();
    let path_ends = [-0.55, 0.55].map(|x| {
        editing::input::project(viewer, model_view, Vec3::new(x, 0., 0.)).unwrap()
    });
    for point in path_ends {
        move_pointer(&mut app, window, point);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
    }
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].path_count, 2);
    let apply = paint_button_rect(&mut app, PaintAction::PathApply);
    assert!(apply.max.y <= model_view.min.y, "the path toolbar is above the viewport");
    app.world_mut().resource_mut::<PaintState>().sessions.get_mut(&0).unwrap().opacity = 50.;
    paint_click(&mut app, window, PaintAction::PathApply);
    assert_eq!(brush_alpha(&app, 256, 512), 128, "3D control points apply on the model");
    paint_click(&mut app, window, PaintAction::PathApply);
    assert_eq!(brush_alpha(&app, 256, 512), 192, "reapplying uses the same path as a new stroke");
    paint_message(&mut app, PaintAction::Undo);
    assert_eq!(brush_alpha(&app, 256, 512), 128);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].path_count, 2);
    move_pointer(&mut app, window, path_ends[1]);
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(MouseButtonInput {
            window,
            button: MouseButton::Right,
            state,
        });
        frame(&mut app);
    }
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].path_count, 1);
    paint_click(&mut app, window, PaintAction::PathClear);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].path_count, 0);
    assert_eq!(brush_alpha(&app, 256, 512), 128);
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
    let viewer_ui = app.world().resource::<ViewerUi>();
    let overlay = viewer_ui.overlay;
    let stable_labels: Vec<_> = stable_labels
        .into_iter()
        .chain([viewer_ui.status, viewer_ui.hint])
        .collect();
    let camera = app
        .world_mut()
        .query_filtered::<Entity, With<ViewCamera>>()
        .single(app.world())
        .unwrap();
    let label_ticks = |app: &App| {
        stable_labels
            .iter()
            .map(|&entity| {
                app.world()
                    .entity(entity)
                    .get_ref::<Text>()
                    .unwrap()
                    .last_changed()
            })
            .collect::<Vec<_>>()
    };
    let layout_tick = |app: &App| {
        app.world()
            .entity(overlay)
            .get_ref::<Node>()
            .unwrap()
            .last_changed()
    };
    let camera_ticks = |app: &App| {
        let camera = app.world().entity(camera);
        (
            camera.get_ref::<Transform>().unwrap().last_changed(),
            camera.get_ref::<Projection>().unwrap().last_changed(),
        )
    };
    let ticks = (label_ticks(&app), layout_tick(&app), camera_ticks(&app));
    warm(&mut app, 3);
    assert_eq!(
        ticks,
        (label_ticks(&app), layout_tick(&app), camera_ticks(&app)),
        "idle paint workspace must keep viewer layout, labels and camera stable"
    );
    assert_eq!(
        app.world().resource::<editing::State>().mode(Some(0)),
        Mode::Paint
    );
    assert!(!app.world().resource::<editing::State>().available(
        editing::EditAction::Operate(crate::editor::model_document::Operation::Extrude),
        Some(0)
    ));
    let side = {
        let world = app.world_mut();
        world.query::<(&sidebar::PanelBody, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find_map(|(panel, node, transform)| {
                (panel.0 == Panel::Layers).then(|| engine::node_rect(node, transform)).flatten()
            })
            .unwrap()
    };
    let shelf = labelled(&mut app, "Рисование: библиотека ресурсов");
    let uv = labelled(&mut app, "Рисование: 2D-вид UV");
    let view = labelled(&mut app, "3D-вид модели");
    assert!(
        side.width() > 200. && shelf.height() > 150. && uv.width() > 200. && view.width() > 200.
    );
    assert!(view.max.x <= uv.min.x + 2. && uv.max.x <= side.min.x + 2.);
    assert!(view.max.y <= shelf.min.y + 2. && uv.max.y <= shelf.min.y + 2.);
    let point = view.center();
    let (expected_focus, before_focus) = {
        let world = app.world();
        let viewer = world.resource::<Viewer>();
        let doc = &world.resource::<editing::State>().documents[&0];
        let (origin, direction) = editing::input::ray(viewer, view, point);
        let (_, _, depth) = editing::input::Bvh::build(doc).cast(origin, direction, None).unwrap();
        (origin + direction * depth, viewer.center)
    };
    assert!(before_focus.distance(expected_focus) > 0.01);
    move_pointer(&mut app, window, point);
    fill_modifier(&mut app, window, KeyCode::AltLeft, Key::Alt, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(app.world().resource::<Viewer>().center, before_focus);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    let started = std::time::Instant::now();
    while app.world().resource::<Viewer>().center.distance(expected_focus) > 0.001 {
        frame(&mut app);
        assert!(started.elapsed().as_secs() < 5, "double-click did not focus the mesh surface");
    }
    fill_modifier(&mut app, window, KeyCode::AltLeft, Key::Alt, ButtonState::Released);
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
    scroll_to_paint_control(&mut app, PaintAction::Select(1));

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
    {
        let world = app.world_mut();
        let mut rows = world.query::<(&AccessibleLabel, &crate::editor::ui::control_hints::Help)>();
        assert!(rows.iter(world).any(|(label, hint)| {
            label.0.contains("Слоёв 1") && hint.0.starts_with("Буферы слоя на GPU")
        }));
    }
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

    scroll_to_paint_control(&mut app, PaintAction::Select(0));
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
    scroll_to_paint_control(&mut app, PaintAction::Undo);
    paint_click(&mut app, window, PaintAction::Undo);
    let session = &app.world().resource::<PaintState>().sessions[&0];
    assert_eq!(session.stack.selected, 1);
    assert!(session.stack.layers[0].mask.is_none());

    paint_click(&mut app, window, PaintAction::Category(Category::Materials));
    set_shelf_query(&mut app, "Gold");
    scroll_to_paint_control(&mut app, PaintAction::Add(LayerKind::Folder));
    paint_click(&mut app, window, PaintAction::Add(LayerKind::Folder));
    let folder = app.world().resource::<PaintState>().sessions[&0].stack.selected;
    scroll_to_paint_control(&mut app, PaintAction::Select(folder));
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
    scroll_to_paint_control(&mut app, PaintAction::Undo);
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
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().to_path_buf();
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
        while app.world().resource::<PaintState>().library.entries().count() < count {
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
        let material = resource.library.entries().find(|(_, entry)| matches!(entry.kind, presets::Kind::Material { .. })).unwrap().0;
        let mask = resource.library.entries().find(|(_, entry)| matches!(entry.kind, presets::Kind::Mask(_))).unwrap().0;
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
    let imported = app.world().resource::<PaintState>().library.entries()
        .find(|(_, entry)| entry.name == "MP Patinated brass").unwrap().0;
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
    let before = app.world().resource::<PaintState>().sessions[&0].stack.clone();
    paint_click(&mut app, window, PaintAction::SavedMaterialMenu(Some(imported)));
    assert!(paint_button_rect(&mut app, PaintAction::ShowSavedMaterial(imported)).width() > 0.);
    paint_click(&mut app, window, PaintAction::ShareSavedMaterial(imported));
    let start = std::time::Instant::now();
    while app.world().resource::<PaintState>().community_submission.is_some() {
        frame(&mut app);
        assert!(start.elapsed().as_secs() < 60, "Community package did not finish");
    }
    let package = std::fs::read_dir(directory.join("online/community/submissions")).unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "zip"))
        .unwrap_or_else(|| panic!("{}", app.world().resource::<Editor>().status));
    let (reopened, _) = presets::import_material(&source_dir, &package).unwrap();
    let layers = presets::load_material(&reopened.path, 32).unwrap();
    assert_eq!(layers.len(), 3);
    assert_eq!(layers[0].name, "Brass coating");
    assert_eq!(layers[0].mask.as_ref().unwrap().filters.len(), 1);

    let deleted = app.world().resource::<PaintState>().library.entry(material).unwrap().path.clone();
    paint_click(&mut app, window, PaintAction::SavedMaterialMenu(Some(material)));
    paint_click(&mut app, window, PaintAction::DeleteSavedMaterial(material));
    let start = std::time::Instant::now();
    while app.world().resource::<PaintState>().library.entries().any(|(index, _)| index == material) {
        frame(&mut app);
        assert!(start.elapsed().as_secs() < 30, "{}", app.world().resource::<Editor>().status);
    }
    assert!(!deleted.exists());
    let state = app.world().resource::<PaintState>();
    assert_eq!(state.library.entry(imported).unwrap().name, "MP Patinated brass");
    assert!(state.library.entry(mask).is_some());
    assert!(crate::editor::model_paint_data::same_stack(&state.sessions[&0].stack, &before));
    paint_click(&mut app, window, PaintAction::Undo);
    assert_eq!(app.world().resource::<PaintState>().sessions[&0].stack.layers.len(), 3);
}
