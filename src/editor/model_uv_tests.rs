use super::*;
use std::path::PathBuf;

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
    let result = backend::run(input.request.clone(), Arc::new(backend::Progress::default())).unwrap();
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
        click(&mut app, window, |a| {
            matches!(
                a,
                Action::ModelView(ModelViewAction::Environment(
                    super::super::super::environment::EnvironmentAction::TogglePanel
                ))
            )
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
