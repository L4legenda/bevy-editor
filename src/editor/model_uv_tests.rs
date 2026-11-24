use super::*;
use std::path::PathBuf;

#[test]
fn udim_import_preference_persists_and_defaults_to_enabled() {
    let dir = std::env::temp_dir().join(format!("bevy-model-import-{}", bevy::asset::uuid::Uuid::new_v4()));
    let file = dir.join("model-import.json");
    let mut prefs = ImportPreferences::open(Some(file.clone()));
    assert!(prefs.udim_tiles);
    prefs.toggle_udim().unwrap();
    assert!(!prefs.udim_tiles);
    let mut reopened = ImportPreferences::open(Some(file.clone()));
    assert!(!reopened.udim_tiles);
    assert_eq!(reopened.auto_unwrap, AutoUnwrap::Missing);
    reopened.update(|prefs| {
        prefs.auto_unwrap = AutoUnwrap::All;
        prefs.texture_size = 2048;
        prefs.bake_on_load = true;
        prefs.bake_defaults.quality = model_bake_settings::Quality::High;
        prefs.bake_defaults.match_names = false;
    }).unwrap();
    assert_eq!(ImportPreferences::open(Some(file.clone())).auto_unwrap, AutoUnwrap::All);
    reopened.toggle_udim().unwrap();
    let reopened = ImportPreferences::open(Some(file.clone()));
    assert!(reopened.udim_tiles);
    assert_eq!(reopened.auto_unwrap, AutoUnwrap::All);
    assert_eq!(reopened.texture_size, 2048);
    assert!(reopened.bake_on_load);
    assert_eq!(reopened.bake_defaults, model_bake_settings::Defaults {
        quality: model_bake_settings::Quality::High,
        match_names: false,
    });

    std::fs::write(&file, "{}").unwrap();
    let legacy = ImportPreferences::open(Some(file.clone()));
    assert!(legacy.udim_tiles);
    assert_eq!(legacy.bake_defaults, model_bake_settings::Defaults::default());
    let mut invalid = ImportPreferences { path: Some(file.join("invalid")), ..default() };
    assert!(invalid.toggle_udim().is_err());
    assert!(invalid.udim_tiles);
    std::fs::remove_dir_all(dir).unwrap();
}

fn object() -> Object {
    let mut geometry = Geometry::default();
    document::from_mesh(&Mesh::from(Cuboid::from_size(Vec3::ONE)), &mut geometry, 0).unwrap();
    Object {
        id: 0,
        source_node: None,
        name: "Cube".into(),
        geometry: geometry.into(),
        transform: Transform::IDENTITY,
        materials: vec![],
        hidden: false,
    }
}
fn document_fixture() -> Document {
    let fixture_object = object();
    let bytes = document::glb::Glb::new_asset(&[fixture_object]).unwrap();
    let dir = std::env::temp_dir().join(format!("bevy-uv-{}", bevy::asset::uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cube.glb");
    std::fs::write(&path, bytes).unwrap();
    Document::new(document::glb::Glb::load(&path).unwrap(), vec![object()])
}
fn output(input: &topology::Input) -> backend::Output {
    backend::Output {
        suggestion: vec![],
        uv: input
            .corners
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let x = (i % 4) as f32 * 0.23;
                let y = (i / 4) as f32 * 0.23;
                [[x, y], [x + 0.2, y], [x, y + 0.2]]
            })
            .collect(),
        charts: 12,
        utilization: 0.5,
    }
}
#[test]
fn uv_packs_udim_sets_independently_and_restores_their_offsets() {
    use crate::editor::model_document::{MaterialSlot, udim};
    use crate::editor::model_mesh::{Corner, Face};
    let mut object = object();
    let local = [
        Vec2::splat(0.125),
        Vec2::new(0.875, 0.125),
        Vec2::new(0.125, 0.875),
    ];
    let offsets = [Vec2::ZERO, Vec2::new(2., -1.)];
    let mut mesh = Geometry::default();
    for (slot, offset) in offsets.into_iter().enumerate() {
        mesh.positions
            .extend(local.map(|point| point.extend(slot as f32)));
        mesh.faces.push(Face {
            material: 0,
            smooth: false,
            corners: local
                .into_iter()
                .enumerate()
                .map(|(i, uv)| {
                    let mut corner = Corner::new(slot * 3 + i);
                    corner.uv = uv + offset;
                    corner
                })
                .collect(),
        });
    }
    object.geometry = mesh.into();
    object.materials = vec![MaterialSlot {
        bake_fixes: Default::default(),
        authored: None,
        paint: None,
        material: default(),
        source: serde_json::json!({}),
    }];
    udim::import(&mut object, true);
    let mut input = topology::input(&object.geometry, 1.25, 512, 4).unwrap();
    input.request.existing_uv = Some(vec![local.map(|uv| uv.to_array()); 2]);
    let result = jobs::run_layout(
        input.request,
        vec![Some((0, 1001)), Some((1, 1013))],
        Arc::new(backend::Progress::default()),
    )
    .unwrap();
    assert_eq!(
        result.uv[0], result.uv[1],
        "each set receives its own full tile"
    );
    let geometry = apply_layout(&object, &input.corners, &result).unwrap();
    assert!(object.preserves_udim_assignment(&geometry));
    for (slot, face) in geometry.faces.iter().enumerate() {
        for (corner, uv) in face.corners.iter().zip(result.uv[slot]) {
            assert!(
                corner
                    .uv
                    .abs_diff_eq(Vec2::from_array(uv) + offsets[slot], 0.00001)
            );
        }
    }
    let mut invalid = geometry.clone();
    invalid.faces[1].corners[0].uv += Vec2::X;
    assert!(!object.preserves_udim_assignment(&invalid));
    let atlas = topology::Atlas::with_offsets(&geometry, |slot| object.uv_offset(slot));
    assert_eq!(atlas.outside, 0);
}
#[test]
fn uv_moves_islands_between_udim_tiles_with_paint_and_history() {
    use crate::editor::model_document::MaterialSlot;
    use crate::editor::model_mesh::{Corner, Face};
    use crate::editor::model_paint_data::{LayerKind, Stack, Surface, mask::Mask, solid};
    let mut doc = document_fixture();
    let mut object = object();
    let mut geometry = Geometry::default();
    let local = |x| {
        [
            Vec2::new(x, 0.25),
            Vec2::new(x + 0.125, 0.25),
            Vec2::new(x, 0.5),
        ]
    };
    for (face, (slot, offset, x)) in [
        (0, Vec2::ZERO, 0.125),
        (0, Vec2::ZERO, 0.625),
        (1, Vec2::X, 0.125),
    ]
    .into_iter()
    .enumerate()
    {
        geometry
            .positions
            .extend(local(x).map(|uv| uv.extend(face as f32)));
        geometry.faces.push(Face {
            material: slot,
            smooth: false,
            corners: local(x)
                .into_iter()
                .enumerate()
                .map(|(i, uv)| {
                    let mut corner = Corner::new(face * 3 + i);
                    corner.uv = uv + offset;
                    corner.uv1 = uv;
                    corner
                })
                .collect(),
        });
    }
    let paint = |color| {
        let mut stack = Stack::default();
        assert_eq!(stack.layers[0].kind, LayerKind::Paint);
        stack.layers[0].channels[0] = Some(solid(16, color));
        stack.layers[0].mask = Some(Arc::new(Mask::new(solid(16, [120; 4]), vec![])));
        Arc::new(Surface {
            size: 16,
            stack,
            material: serde_json::Value::Null,
            baked: None,
            base: std::array::from_fn(|_| solid(16, [255; 4])),
        })
    };
    let saved_paint = paint([34, 98, 170, 255]);
    object.geometry = geometry.into();
    object.materials = [saved_paint.clone(), paint([170, 18, 43, 255])]
        .into_iter()
        .enumerate()
        .map(|(index, paint)| MaterialSlot {
            bake_fixes: Default::default(),
            material: default(),
            source: serde_json::json!({"material":0, "extras":{"bevy_editor_udim":1001 + index}}),
            paint: Some(paint),
            authored: None,
        })
        .collect();
    doc.objects = vec![object.clone()];
    doc.mode = Mode::Uv;
    let mut editing = State::default();
    editing.documents.insert(0, doc);
    let mut uv = UvState::default();
    let drag = |doc: &Document, face: usize, start: Vec2| {
        let active = doc.active().unwrap();
        let atlas = topology::Atlas::with_offsets(&active.geometry, |slot| active.uv_offset(slot));
        let selected = BTreeSet::from([atlas.islands[face]]);
        Drag {
            model: 0,
            source: doc.source.path.clone(),
            start,
            snapshot: doc.snapshot(),
            atlas,
            selected,
            changed: false,
            tile_delta: None,
        }
    };
    uv.drag = Some(drag(&editing.documents[&0], 0, Vec2::new(0.15, 0.3)));
    input::finish_drag_for_test(&mut uv, &mut editing, Some(Vec2::new(1.75, 0.6)));
    assert!(uv.error.is_empty(), "{}", uv.error);
    let doc = editing.documents.get_mut(&0).unwrap();
    let moved = doc.active().unwrap();
    assert_eq!(moved.materials.len(), 3);
    assert_eq!(moved.geometry.faces[0].material, 2);
    assert_eq!(moved.materials[2].udim().unwrap().number(), 1002);
    assert!(Arc::ptr_eq(
        moved.materials[2].paint.as_ref().unwrap(),
        &saved_paint
    ));
    assert_eq!(&moved.geometry.faces[1..], &object.geometry.faces[1..]);
    for (a, b) in moved.geometry.faces[0]
        .corners
        .iter()
        .zip(&object.geometry.faces[0].corners)
    {
        assert_eq!(a.uv, b.uv + Vec2::X);
        assert_eq!(a.uv1, b.uv1);
    }
    assert!(moved.preserves_udim_assignment(&moved.geometry));
    let path = doc.source.path.with_file_name("island.glb");
    std::fs::write(
        &path,
        document::glb::Glb::new_asset(&[moved.clone()]).unwrap(),
    )
    .unwrap();
    let glb = document::glb::Glb::load(&path).unwrap();
    let primitive = &glb.json["meshes"][0]["primitives"][2];
    assert_eq!(primitive["extras"]["bevy_editor_udim"], 1002);
    let reopened = glb.paint_surface(primitive).unwrap().unwrap();
    assert_eq!(
        reopened.stack.layers[0].channels[0],
        saved_paint.stack.layers[0].channels[0]
    );
    assert_eq!(
        reopened.stack.layers[0].mask.as_ref().unwrap().pixels,
        saved_paint.stack.layers[0].mask.as_ref().unwrap().pixels
    );
    assert_eq!(doc.undo.len(), 1);
    doc.undo();
    assert_eq!(doc.active().unwrap(), &object);
    doc.redo();
    assert_eq!(doc.active().unwrap().materials.len(), 3);

    uv.drag = Some(drag(doc, 0, Vec2::new(1.15, 0.3)));
    input::finish_drag_for_test(&mut uv, &mut editing, Some(Vec2::new(2.15, 0.3)));
    let doc = &editing.documents[&0];
    assert_eq!(
        doc.active().unwrap().materials.len(),
        3,
        "moving a whole set reuses its slot"
    );
    assert_eq!(
        doc.active().unwrap().materials[2].udim().unwrap().number(),
        1003
    );
    let before_cancel = doc.active().unwrap().clone();
    let mut invalid = drag(doc, 2, Vec2::new(1.15, 0.3));
    invalid.selected.insert(invalid.atlas.islands[0]);
    invalid.changed = true;
    let doc = editing.documents.get_mut(&0).unwrap();
    doc.active_mut().unwrap().geometry.faces[2].corners[0].uv.x += 0.01;
    uv.drag = Some(invalid);
    input::finish_drag_for_test(&mut uv, &mut editing, Some(Vec2::new(9.15, 0.3)));
    assert!(uv.error.contains("выходит за пределы UDIM"), "{}", uv.error);
    let doc = &editing.documents[&0];
    assert_eq!(doc.undo.len(), 2);
    assert_eq!(doc.active().unwrap(), &before_cancel);

    // Move into an unused region of the same material on 1002.
    let mut compatible = object.clone();
    for corner in &mut compatible.geometry.faces[2].corners {
        corner.uv.x += 0.5;
    }
    let destination = Arc::make_mut(compatible.materials[1].paint.as_mut().unwrap());
    destination.stack.layers[0].mask = Some(Arc::new(Mask::new(solid(16, [210; 4]), vec![])));
    destination.stack.layers[0].channels[1] = Some(solid(16, [22, 22, 22, 255]));
    let original_destination = compatible.materials[1].paint.as_ref().unwrap().clone();
    let mut doc = document_fixture();
    doc.mode = Mode::Uv;
    doc.objects = vec![compatible.clone()];
    uv.drag = Some(drag(&doc, 0, Vec2::new(0.15, 0.3)));
    editing.documents.insert(0, doc);
    input::finish_drag_for_test(&mut uv, &mut editing, Some(Vec2::new(1.15, 0.3)));
    let doc = editing.documents.get_mut(&0).unwrap();
    let merged = doc.active().unwrap();
    assert_eq!(doc.undo.len(), 1);
    assert_eq!(merged.materials.len(), 2);
    assert_eq!(merged.geometry.faces[0].material, 1);
    assert_eq!(merged.geometry.faces[1].material, 0);
    assert_eq!(merged.geometry.faces[2].material, 1);
    assert!(Arc::ptr_eq(merged.materials[0].paint.as_ref().unwrap(), &saved_paint));
    let surface = merged.materials[1].paint.as_ref().unwrap();
    let left = (4 * 16 + 2) * 4;
    let right = (4 * 16 + 10) * 4;
    let layer = &surface.stack.layers[0];
    assert_eq!(&layer.channels[0].as_ref().unwrap()[left..left + 4], &[34, 98, 170, 255]);
    assert_eq!(&layer.channels[0].as_ref().unwrap()[right..right + 4], &[170, 18, 43, 255]);
    let mask = &layer.mask.as_ref().unwrap().pixels;
    assert_eq!([mask[left], mask[right]], [120, 210]);
    let roughness = layer.channels[1].as_ref().unwrap();
    assert_eq!([roughness[left + 3], roughness[right + 3]], [0, 255]);
    for (offset, expected) in [(left, &saved_paint), (right, &original_destination)] {
        for channel in [0, 1] {
            assert_eq!(
                surface.composite(channel)[offset..offset + 4],
                expected.composite(channel)[offset..offset + 4],
            );
        }
    }
    std::fs::write(&path, document::glb::Glb::new_asset(&[merged.clone()]).unwrap()).unwrap();
    let glb = document::glb::Glb::load(&path).unwrap();
    let primitive = &glb.json["meshes"][0]["primitives"][1];
    assert_eq!(primitive["extras"]["bevy_editor_udim"], 1002);
    assert_eq!(*glb.paint_surface(primitive).unwrap().unwrap(), **surface);
    doc.undo();
    assert_eq!(doc.active().unwrap(), &compatible);
    doc.redo();
    assert_eq!(doc.active().unwrap().geometry.faces[0].material, 1);

    // Moving the whole source into unused space removes its empty slot and remaps faces.
    let mut whole = compatible.clone();
    whole.geometry.faces.remove(1);
    let whole = document::udim::relocate(&whole, &[0, 1], &BTreeSet::from([0]), Vec2::X).unwrap();
    assert_eq!(whole.materials.len(), 1);
    assert!(whole.geometry.faces.iter().all(|face| face.material == 0));
    assert_eq!(whole.udim_tiles().next().unwrap().number(), 1002);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn uv_job_is_one_undoable_transaction_and_rejects_stale_geometry() {
    let mut state = State::default();
    state.documents.insert(0, document_fixture());
    let original = state.documents[&0].active().unwrap().geometry.clone();
    let input = topology::input(&original, 1.25, 2048, 8).unwrap();
    apply_job(&mut state, 0, 0, &original, &input.corners, &output(&input)).unwrap();
    let after = state.documents[&0].active().unwrap().geometry.clone();
    assert_eq!(original.positions, after.positions);
    assert_eq!(state.documents[&0].undo.len(), 1);
    assert!(apply_job(&mut state, 0, 0, &original, &input.corners, &output(&input)).is_err());
    assert_eq!(state.documents[&0].undo.len(), 1);
    state.documents.get_mut(&0).unwrap().undo();
    assert_eq!(state.documents[&0].active().unwrap().geometry, original);
    state.documents.get_mut(&0).unwrap().redo();
    assert_eq!(state.documents[&0].active().unwrap().geometry, after);
}
#[test]
fn uv_invalid_results_do_not_change_materials_geometry_or_history() {
    let mut state = State::default();
    state.documents.insert(0, document_fixture());
    let original = state.documents[&0].objects.clone();
    let input = topology::input(&original[0].geometry, 1.25, 2048, 8).unwrap();
    let mut bad = output(&input);
    bad.uv[0][0][0] = f32::NAN;
    assert!(
        apply_job(
            &mut state,
            0,
            0,
            &original[0].geometry,
            &input.corners,
            &bad
        )
        .is_err()
    );
    assert_eq!(state.documents[&0].objects, original);
    assert!(state.documents[&0].undo.is_empty());
}
#[test]
fn uv_before_after_is_view_only_and_does_not_overwrite_newer_edits() {
    let mut state = State::default();
    state.documents.insert(0, document_fixture());
    let before = state.documents[&0].active().unwrap().geometry.clone();
    let input = topology::input(&before, 1.25, 2048, 8).unwrap();
    let (_, after) = apply_job(&mut state, 0, 0, &before, &input.corners, &output(&input)).unwrap();
    let mut uv = UvState::default();
    uv.before = Some(Comparison {
        index: 0,
        object: 0,
        before: before.clone(),
        after: after.clone(),
    });
    uv.comparing = true;
    assert_eq!(
        uv.display_geometry(0, state.documents[&0].active().unwrap()),
        before
    );
    assert_eq!(state.documents[&0].active().unwrap().geometry, after);
    assert_eq!(
        uv.display_geometry(1, state.documents[&0].active().unwrap()),
        after
    );
    state
        .documents
        .get_mut(&0)
        .unwrap()
        .active_mut()
        .unwrap()
        .geometry
        .positions[0]
        .x += 0.1;
    assert!(!uv.comparison_available(Some(0), &state));
}

#[test]
fn uv_partuv_real_worker_preserves_cube_corners() {
    let mesh = object().geometry;
    let input = topology::input(&mesh, 1.25, 1024, 4).unwrap();
    let result = backend::run(
        input.request.clone(),
        Arc::new(backend::Progress::default()),
    )
    .unwrap();
    assert_eq!(result.uv.len(), input.corners.len());
    assert!(result.charts > 0);
    let mapped = topology::apply(&mesh, &input.corners, &result).unwrap();
    assert_eq!(mapped.positions, mesh.positions);
    assert!(topology::Atlas::new(&mapped).area > 0.1);
}

#[test]
fn model_uv_manual_seam_separates_coplanar_faces_in_the_native_unwrap() {
    use crate::editor::model_mesh::{Corner, Edge, Face};

    let mut mesh = Geometry {
        positions: vec![Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y],
        faces: [[0, 1, 2], [0, 2, 3]]
            .map(|vertices| Face {
                corners: vertices.into_iter().map(Corner::new).collect(),
                material: 0,
                smooth: false,
            })
            .to_vec(),
        ..default()
    };
    mesh.uv_seams.insert(Edge::new(0, 2));
    let mut input = topology::input(&mesh, 1.25, 1024, 4).unwrap();
    input.cut_manual_seams(&mesh.uv_seams);
    input.request.options.guide_seams = true;
    assert_eq!(input.request.positions.len(), 6);
    let result = backend::run(
        input.request.clone(),
        Arc::new(backend::Progress::default()),
    )
    .unwrap();
    let unwrapped = topology::apply(&mesh, &input.corners, &result).unwrap();
    assert_eq!(unwrapped.positions, mesh.positions);
    assert_eq!(unwrapped.uv_seams, mesh.uv_seams);
    let atlas = topology::Atlas::new(&unwrapped);
    assert_eq!(atlas.count, 2);
    assert!(atlas.seams.contains(&Edge::new(0, 2)));
}

#[test]
#[ignore = "requires GPU; tests bundled native UV, split view, background job, selection, undo and GLB save"]
fn gpu_uv_workspace_unwraps_previews_edits_and_saves_without_scene_changes() {
    use crate::editor::{
        Editor,
        selection_outline::render_tests::{capture_when, frame},
        tests::lighting_ui_render::{fixture_project, mouse, move_pointer, scroll_to_entity},
    };
    use bevy::input::ButtonState;
    fn warm(app: &mut App, n: usize) {
        for _ in 0..n {
            frame(app);
        }
    }
    fn click(app: &mut App, window: Entity, test: impl Fn(Action) -> bool) {
        let control = {
            let world = app.world_mut();
            world
                .query::<(Entity, &Control, &ComputedNode)>()
                .iter(world)
                .find_map(|(e, c, n)| (!n.is_empty() && test(c.action)).then_some(e))
                .expect("UV control must be visible")
        };
        scroll_to_entity(app, control);
        let world = app.world();
        let point = world
            .get::<UiGlobalTransform>(control)
            .unwrap()
            .affine()
            .translation
            * world
                .get::<ComputedNode>(control)
                .unwrap()
                .inverse_scale_factor();
        move_pointer(app, window, point);
        mouse(app, window, ButtonState::Pressed);
        mouse(app, window, ButtonState::Released);
        warm(app, 3);
    }
    let directory =
        std::env::temp_dir().join(format!("bevy-uv-gpu-{}", bevy::asset::uuid::Uuid::new_v4()));
    let mut project = crate::EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    fn copy(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let e = entry.unwrap();
            let dest = to.join(e.file_name());
            if e.path().is_dir() {
                copy(&e.path(), &dest);
            } else {
                std::fs::copy(e.path(), dest).unwrap();
            }
        }
    }
    copy(&project.asset_root, &directory);
    project.asset_root = directory.clone();
    if let Some(character) = std::env::var_os("BEVY_EDITOR_UV_QA_MODEL") {
        std::fs::copy(character, directory.join("models/block.glb")).unwrap();
    }
    let file = directory.join("models/block.glb");
    let bytes = std::fs::read(&file).unwrap();
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/uv-qa");
    std::fs::create_dir_all(&out).unwrap();
    let (mut app, window, target) = fixture_project(project);
    let scene = app.world().resource::<Editor>().edits.clone();
    let history = app.world().resource::<Editor>().undo.len();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..300 {
        frame(&mut app);
        if app.world().resource::<State>().documents.contains_key(&0) {
            break;
        }
    }
    assert!(app.world().resource::<State>().documents.contains_key(&0));
    click(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(EditAction::Menu))
        )
    });
    click(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(EditAction::Mode(Mode::Uv)))
        )
    });
    warm(&mut app, 35);
    assert_eq!(app.world().resource::<State>().mode(Some(0)), Mode::Uv);
    let original = app.world().resource::<State>().documents[&0]
        .objects
        .clone();
    if std::env::var_os("BEVY_EDITOR_UV_QA_LAYOUT_ONLY").is_none() {
        let request = topology::input(&original[0].geometry, 1.25, 2048, 8)
            .unwrap()
            .request;
        serde_json::to_writer(
            std::fs::File::create(out.join("native-character-request.json")).unwrap(),
            &request,
        )
        .unwrap();
    }
    capture_when(&mut app, &target, &out.join("01-uv-before.png"), |_| true);
    if std::env::var_os("BEVY_EDITOR_UV_QA_LAYOUT_ONLY").is_some() {
        capture_when(
            &mut app,
            &target,
            &out.join("inspector-grouped.png"),
            |_| true,
        );
        let heading = {
            let world = app.world_mut();
            world
                .query::<(&Text, &ComputedNode, &UiGlobalTransform)>()
                .iter(world)
                .find_map(|(t, n, p)| {
                    (t.0 == "Развёртка" && !n.is_empty())
                        .then_some(p.affine().translation * n.inverse_scale_factor())
                })
                .unwrap()
        };
        move_pointer(&mut app, window, heading);
        warm(&mut app, 45);
        assert!(
            app.world_mut()
                .query::<(&Text, &ComputedNode)>()
                .iter(app.world())
                .any(|(t, n)| t.0.starts_with("PartUV Geometric автоматически") && !n.is_empty()),
            "Text headings must show real hover hints"
        );
        capture_when(
            &mut app,
            &target,
            &out.join("inspector-text-hint.png"),
            |_| true,
        );
        click(&mut app, window, |a| {
            matches!(
                a,
                Action::ModelView(ModelViewAction::Uv(UvAction::Quality(0)))
            )
        });
        assert_eq!(app.world().resource::<UvState>().quality, 0);
        click(&mut app, window, |a| {
            matches!(
                a,
                Action::ModelView(ModelViewAction::Uv(UvAction::Resolution))
            )
        });
        assert_eq!(app.world().resource::<UvState>().resolution, 4096);
        click(&mut app, window, |a| {
            matches!(a, Action::ModelView(ModelViewAction::Uv(UvAction::Padding)))
        });
        assert_eq!(app.world().resource::<UvState>().padding, 16);
        use super::super::super::shading::{Mode as ShadingMode, ShadingAction};
        app.world_mut().write_message(Action::ModelView(ModelViewAction::Shading(
            ShadingAction::Set(ShadingMode::Rendered),
        )));
        warm(&mut app, 5);
        click(&mut app, window, |a| {
            matches!(
                a,
                Action::ModelView(ModelViewAction::Environment(
                    super::super::super::environment::EnvironmentAction::TogglePanel
                ))
            )
        });
        click(&mut app, window, |a| {
            matches!(a, Action::ModelView(ModelViewAction::Shading(ShadingAction::PreviewLighting)))
        });
        warm(&mut app, 5);
        assert!(
            app.world_mut()
                .query::<(&AccessibleLabel, &ComputedNode)>()
                .iter(app.world())
                .any(|(label, n)| label.0 == "Просмотр: Яркость окружения" && !n.is_empty())
        );
        capture_when(
            &mut app,
            &target,
            &out.join("inspector-environment-expanded.png"),
            |_| true,
        );
        assert_eq!(
            app.world().resource::<State>().documents[&0].objects,
            original
        );
        return;
    }
    fn uv_click(app: &mut App, window: Entity, action: UvAction) {
        click(
            app,
            window,
            |a| matches!(a, Action::ModelView(ModelViewAction::Uv(value)) if value == action),
        );
    }
    assert!(app.world().resource::<UvState>().detect_udim_on_import());
    uv_click(&mut app, window, UvAction::DetectUdim);
    assert!(!app.world().resource::<UvState>().detect_udim_on_import());
    assert_eq!(app.world().resource::<State>().documents[&0].objects, original);
    assert!(!app.world().resource::<State>().documents[&0].dirty());
    uv_click(&mut app, window, UvAction::DetectUdim);
    assert!(app.world().resource::<UvState>().detect_udim_on_import());

    fn wait_for_proposal(app: &mut App) {
        let start = std::time::Instant::now();
        loop {
            frame(app);
            let uv = app.world().resource::<UvState>();
            assert!(uv.error.is_empty(), "{}", uv.error);
            if uv.proposal.is_some() {
                break;
            }
            assert!(
                start.elapsed().as_secs() < 60,
                "Missing proposal: {}",
                uv.status
            );
        }
        warm(app, 3);
    }
    // Both preview decisions must leave the on-disk asset untouched. Accept gets one undo step.
    for apply in [false, true] {
        uv_click(&mut app, window, UvAction::Preview);
        wait_for_proposal(&mut app);
        let uv = app.world().resource::<UvState>();
        assert!(matches!(
            uv.proposal.as_ref().unwrap().result,
            Proposed::Layout(_)
        ));
        assert_eq!(
            app.world().resource::<State>().documents[&0].objects,
            original
        );
        assert!(
            app.world().resource::<State>().documents[&0]
                .undo
                .is_empty()
        );
        if apply {
            uv_click(&mut app, window, UvAction::Accept);
            assert_eq!(app.world().resource::<State>().documents[&0].undo.len(), 1);
            app.world_mut()
                .write_message(Action::ModelView(ModelViewAction::Edit(EditAction::Undo)));
            warm(&mut app, 4);
        } else {
            uv_click(&mut app, window, UvAction::Compare);
            assert!(app.world().resource::<UvState>().comparing);
            uv_click(&mut app, window, UvAction::Dismiss);
        }
        assert!(app.world().resource::<UvState>().proposal.is_none());
        assert_eq!(
            app.world().resource::<State>().documents[&0].objects,
            original
        );
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
    }
    let model_point = {
        let world = app.world_mut();
        let (n, t) = world
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<Canvas>>()
            .single(world)
            .unwrap();
        let bounds = Rect::from_center_size(
            t.affine().translation * n.inverse_scale_factor(),
            n.size() * n.inverse_scale_factor(),
        );
        let doc = &world.resource::<State>().documents[&0];
        let bvh = input::Bvh::build(doc);
        let viewer = world.resource::<Viewer>();
        (0..101)
            .flat_map(|x| (0..101).map(move |y| (x, y)))
            .map(|(x, y)| bounds.min + bounds.size() * Vec2::new(x as f32 / 100., y as f32 / 100.))
            .filter(|&point| {
                let (origin, dir) = super::super::input::ray(viewer, bounds, point);
                bvh.cast(origin, dir, None)
                    .is_some_and(|(object, _, _)| Some(object) == doc.active)
            })
            .min_by_key(|p| p.distance_squared(bounds.center()) as i32)
            .expect("Pickable model surface")
    };
    let stroke = |app: &mut App| {
        move_pointer(app, window, model_point);
        mouse(app, window, ButtonState::Pressed);
        mouse(app, window, ButtonState::Released);
        warm(app, 3);
    };
    uv_click(&mut app, window, UvAction::Brush);
    assert!(app.world().resource::<UvState>().tool == Tool::Brush);
    stroke(&mut app);
    assert!(
        app.world()
            .resource::<UvState>()
            .masks
            .values()
            .any(|m| !m.is_empty())
    );
    uv_click(&mut app, window, UvAction::ClearMask);
    assert!(app.world().resource::<UvState>().masks.is_empty());
    uv_click(&mut app, window, UvAction::Suggest);
    stroke(&mut app);
    wait_for_proposal(&mut app);
    assert!(
        matches!(&app.world().resource::<UvState>().proposal.as_ref().unwrap().result, Proposed::Island(faces) if !faces.is_empty())
    );
    capture_when(
        &mut app,
        &target,
        &out.join("06-dawand-proposal.png"),
        |_| true,
    );
    uv_click(&mut app, window, UvAction::Accept);
    assert!(
        app.world()
            .resource::<UvState>()
            .masks
            .values()
            .any(|m| !m.is_empty())
    );
    assert!(
        app.world().resource::<State>().documents[&0]
            .undo
            .is_empty()
    );
    uv_click(&mut app, window, UvAction::ClearMask);
    uv_click(&mut app, window, UvAction::Heatmap);
    let uv = app.world().resource::<UvState>();
    assert!(uv.heatmap && uv.atlas.angular_p95.is_some());
    uv_click(&mut app, window, UvAction::Heatmap);
    click(&mut app, window, |a| {
        matches!(a, Action::ModelView(ModelViewAction::Uv(UvAction::Unwrap)))
    });
    let started = std::time::Instant::now();
    let mut saw_job = false;
    while started.elapsed() < std::time::Duration::from_secs(180) {
        frame(&mut app);
        let uv = app.world().resource::<UvState>();
        saw_job |= uv.job.is_some();
        if uv.job.is_none() && saw_job {
            assert!(uv.error.is_empty(), "{}", uv.error);
            break;
        }
    }
    assert!(saw_job);
    assert!(app.world().resource::<UvState>().job.is_none());
    warm(&mut app, 12);
    let after = app.world().resource::<State>().documents[&0]
        .objects
        .clone();
    assert_ne!(original, after);
    assert_eq!(original[0].geometry.positions, after[0].geometry.positions);
    assert_eq!(
        std::fs::read(&file).unwrap(),
        bytes,
        "Auto unwrap does not overwrite the file"
    );
    assert_eq!(app.world().resource::<State>().documents[&0].undo.len(), 1);
    assert!(app.world().resource::<UvState>().atlas.area > 0.1);
    assert!(app.world().resource::<UvState>().atlas.area <= 1.0001);
    capture_when(&mut app, &target, &out.join("02-uv-unwrapped.png"), |_| {
        true
    });
    uv_click(&mut app, window, UvAction::Heatmap);
    assert!(
        app.world()
            .resource::<UvState>()
            .atlas
            .angular_p95
            .is_some_and(f32::is_finite)
    );
    capture_when(
        &mut app,
        &target,
        &out.join("07-angular-distortion.png"),
        |_| true,
    );
    uv_click(&mut app, window, UvAction::Heatmap);
    {
        let assets = app.world().resource::<view::AssetsState>();
        assert!(assets.flat.is_some(), "Atlas uses its own unlit material");
        assert!(
            assets.checker.is_some(),
            "Checker uses an actual texture material"
        );
    }
    click(&mut app, window, |a| {
        matches!(a, Action::ModelView(ModelViewAction::Uv(UvAction::Compare)))
    });
    assert!(app.world().resource::<UvState>().comparing);
    assert_eq!(app.world().resource::<State>().documents[&0].objects, after);
    capture_when(
        &mut app,
        &target,
        &out.join("03-uv-before-comparison.png"),
        |_| true,
    );
    click(&mut app, window, |a| {
        matches!(a, Action::ModelView(ModelViewAction::Uv(UvAction::Compare)))
    });
    click(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Uv(UvAction::SelectAll))
        )
    });
    click(&mut app, window, |a| {
        matches!(a, Action::ModelView(ModelViewAction::Uv(UvAction::Rotate)))
    });
    assert_ne!(app.world().resource::<State>().documents[&0].objects, after);
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Edit(EditAction::Undo)));
    warm(&mut app, 4);
    assert_eq!(app.world().resource::<State>().documents[&0].objects, after);
    click(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Uv(UvAction::Deselect))
        )
    });
    // Drag an actual island with pointer events, then undo the entire gesture.
    let point = {
        let uv = app.world().resource::<UvState>();
        let object = &after[0];
        let tri = object
            .geometry
            .faces
            .iter()
            .flat_map(|face| {
                object
                    .geometry
                    .triangles(face)
                    .into_iter()
                    .map(|ids| ids.map(|i| face.corners[i].uv))
            })
            .max_by(|a, b| {
                (a[1] - a[0])
                    .perp_dot(a[2] - a[0])
                    .abs()
                    .total_cmp(&(b[1] - b[0]).perp_dot(b[2] - b[0]).abs())
            })
            .unwrap();
        let center = (tri[0] + tri[1] + tri[2]) / 3.;
        let canvas = app.world().resource::<ui::UvUi>().canvas;
        let computed = app.world().get::<ComputedNode>(canvas).unwrap();
        let transform = app.world().get::<UiGlobalTransform>(canvas).unwrap();
        let size = computed.size() * computed.inverse_scale_factor();
        let origin = transform.affine().translation * computed.inverse_scale_factor();
        let height = 1.18_f32.max(1.18 / (size.x / size.y)) / uv.zoom;
        origin + (center - uv.center) * Vec2::new(1., -1.) * size.y / height
    };
    move_pointer(&mut app, window, point);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, point + Vec2::new(13., -7.));
    warm(&mut app, 2);
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 3);
    assert_ne!(app.world().resource::<State>().documents[&0].objects, after);
    capture_when(
        &mut app,
        &target,
        &out.join("04-uv-island-selected.png"),
        |_| true,
    );
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Edit(EditAction::Undo)));
    warm(&mut app, 4);
    assert_eq!(app.world().resource::<State>().documents[&0].objects, after);
    // Cancellation must leave both mesh data and history untouched.
    let undo_count = app.world().resource::<State>().documents[&0].undo.len();
    click(&mut app, window, |a| {
        matches!(a, Action::ModelView(ModelViewAction::Uv(UvAction::Unwrap)))
    });
    assert!(app.world().resource::<UvState>().job.is_some());
    click(&mut app, window, |a| {
        matches!(a, Action::ModelView(ModelViewAction::Uv(UvAction::Cancel)))
    });
    assert!(app.world().resource::<UvState>().job.is_none());
    assert_eq!(app.world().resource::<State>().documents[&0].objects, after);
    assert_eq!(
        app.world().resource::<State>().documents[&0].undo.len(),
        undo_count
    );
    click(&mut app, window, |a| {
        matches!(a, Action::ModelView(ModelViewAction::Uv(UvAction::Pack)))
    });
    let started = std::time::Instant::now();
    while app.world().resource::<UvState>().job.is_some() && started.elapsed().as_secs() < 90 {
        frame(&mut app);
    }
    assert!(
        app.world().resource::<UvState>().error.is_empty(),
        "{}",
        app.world().resource::<UvState>().error
    );
    assert!(app.world().resource::<UvState>().job.is_none());
    assert!(
        app.world().resource::<UvState>().atlas.area <= 1.0001,
        "Packing must never stretch sliver corners into overlapping triangles"
    );
    assert_eq!(
        app.world().resource::<State>().documents[&0].objects[0]
            .geometry
            .positions,
        original[0].geometry.positions
    );
    move_pointer(&mut app, window, Vec2::new(440., 270.));
    warm(&mut app, 8);
    click(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(EditAction::Save))
        )
    });
    for _ in 0..300 {
        frame(&mut app);
        if app.world().resource::<State>().saving.is_none() {
            break;
        }
    }
    assert!(!app.world().resource::<State>().documents[&0].dirty());
    assert_ne!(std::fs::read(&file).unwrap(), bytes);
    assert_eq!(app.world().resource::<Editor>().edits, scene);
    assert_eq!(app.world().resource::<Editor>().undo.len(), history);
    capture_when(&mut app, &target, &out.join("05-uv-saved.png"), |_| true);
    std::fs::write(
        out.join("tested-project.txt"),
        directory.to_string_lossy().as_bytes(),
    )
    .unwrap();
}
