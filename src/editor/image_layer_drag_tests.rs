use super::*;
use crate::editor::selection_outline::render_tests::{capture_when, frame};
use crate::editor::tests::lighting_ui_render::{mouse, move_pointer};
use bevy::input::{
    ButtonState,
    keyboard::{Key, KeyboardInput},
};
use edit::layers::{
    LayerAction as L,
    ui::drag_ui::{CopyExtent, DropRow, LayerDrag, LayerList, Place, Slide},
};

fn warm(app: &mut App, frames: usize) {
    for _ in 0..frames {
        frame(app);
    }
}
fn layer(app: &mut App, action: L) {
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Layers(action)));
    warm(app, 4);
}
fn document(app: &App) -> &crate::editor::photo_document::PhotoDocument {
    let v = app.world().resource::<ImageViewer>();
    v.edit.document(v.tab).unwrap()
}

#[test]
#[ignore = "requires GPU: verifies the layer flyout with real pointer events"]
fn gpu_layer_flyout_opens_beside_the_row_and_dismisses_without_painting() {
    let fixture = Fixture::new();
    let (mut app, window, target) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.0.clone();
            editor.project.asset_root = fixture.0.clone();
            editor.drawer = false;
            browser.refresh(&mut editor.project);
        });
    let file = file(&app, "b-transparent.png");
    app.world_mut().write_message(Action::ViewImage(file));
    wait_for_image(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    wait_for_edit(&mut app);
    layer(&mut app, L::Quick("layer.new.layer"));
    let right_click = |app: &mut App, index| {
        click_control(
            app,
            window,
            |action| matches!(action, Action::ImageEdit(edit::EditAction::SelectLayer(i)) if i == index),
            MouseButton::Right,
        );
        warm(app, 12);
    };
    right_click(&mut app, 0);
    assert_eq!(document(&app).active, 0);
    let layer_id = document(&app).layer(0).unwrap().id.0;
    let row = rect(&mut app, Some(layer_id));
    let menu = {
        let world = app.world_mut();
        let (node, transform) = world
            .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find_map(|(label, node, transform)| {
                (label.0 == "Меню слоя" && !node.is_empty()).then_some((node, transform))
            })
            .unwrap();
        Rect::from_center_size(
            transform.affine().translation * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        )
    };
    assert!(menu.max.x < row.min.x);
    assert!((menu.min.y..menu.max.y).contains(&row.center().y));
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/layer-flyout.png");
    capture_when(&mut app, &target, &output, |_| true);
    click_action(&mut app, window, |action| {
        matches!(
            action,
            Action::ImageEdit(edit::EditAction::Layers(L::Category("Эффекты")))
        )
    });
    assert!(visible(&mut app, "Меню слоя"));
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.layers.category,
        "Эффекты"
    );
    right_click(&mut app, 1);
    assert_eq!(document(&app).active, 1);
    assert!(visible(&mut app, "Меню слоя"));
    let revision = document(&app).revision;
    move_pointer(&mut app, window, Vec2::new(520.0, 450.0));
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    assert!(!app.world().resource::<ImageViewer>().edit.layers.menu);
    assert!(
        visible(&mut app, "Меню слоя"),
        "The closing menu should still be animating"
    );
    warm(&mut app, 14);
    assert!(!visible(&mut app, "Меню слоя"));
    assert_eq!(document(&app).revision, revision);
    right_click(&mut app, 1);
    escape(&mut app, window);
    assert!(!app.world().resource::<ImageViewer>().edit.layers.menu);
    assert!(visible(&mut app, "Меню слоя"));
    warm(&mut app, 14);
    assert!(!visible(&mut app, "Меню слоя"));

    let open_masks = |app: &mut App| {
        click_action(app, window, |action| {
            matches!(
                action,
                Action::ImageEdit(edit::EditAction::Layers(L::Category("Добавить маску")))
            )
        });
        warm(app, 18);
    };
    open_masks(&mut app);
    let element_bounds = |app: &mut App, name| {
        let world = app.world_mut();
        let (node, transform) = world
            .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find_map(|(label, node, transform)| {
                (label.0 == name && !node.is_empty()).then_some((node, transform))
            })
            .unwrap();
        let scale = node.inverse_scale_factor();
        Rect::from_center_size(transform.affine().translation * scale, node.size() * scale)
    };
    let masks = element_bounds(&mut app, "Меню масок");
    let button = element_bounds(&mut app, "Добавить маску");
    assert_eq!(masks.width(), menu.width());
    assert!(masks.max.y < button.min.y && button.min.y - masks.max.y < 12.0);
    assert!((masks.center().x - button.center().x).abs() < 2.0);
    assert!(visible(&mut app, "Маска из изображения…"));
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/mask-flyout.png");
    capture_when(&mut app, &target, &output, |_| true);
    move_pointer(&mut app, window, Vec2::new(520.0, 450.0));
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 14);
    assert!(!visible(&mut app, "Меню масок"));
    assert_eq!(document(&app).revision, revision);
    open_masks(&mut app);
    click_action(&mut app, window, |action| {
        matches!(
            action,
            Action::ImageEdit(edit::EditAction::Layers(L::Quick(
                "layer.layerMask.hideAll"
            )))
        )
    });
    warm(&mut app, 14);
    assert!(!visible(&mut app, "Меню масок"));
    assert_eq!(
        document(&app)
            .active_layer()
            .unwrap()
            .mask
            .as_ref()
            .unwrap()
            .value(1, 1),
        0.0
    );
}

#[test]
#[ignore = "requires GPU: verifies Alt-click, the clipping arrow and composited pixels"]
fn gpu_alt_click_toggles_clipping_on_the_clicked_layer() {
    let fixture = Fixture::new();
    let (mut app, window, _) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.0.clone();
            editor.project.asset_root = fixture.0.clone();
            editor.drawer = false;
            browser.refresh(&mut editor.project);
        });
    let file = file(&app, "b-transparent.png");
    app.world_mut().write_message(Action::ViewImage(file));
    wait_for_image(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    wait_for_edit(&mut app);
    layer(&mut app, L::Quick("layer.newFillLayer.solidColor"));
    let upper = document(&app).active_id().unwrap();
    let arrow = "Обтравочная маска по нижнему слою";
    let alpha =
        |app: &App, x: usize, y: usize| document(app).composite().pixels[(y * 300 + x) * 4 + 3];
    let alt_click = |app: &mut App, index| {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::AltLeft);
        click_action(
            app,
            window,
            |action| matches!(action, Action::ImageEdit(edit::EditAction::SelectLayer(i)) if i == index),
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::AltLeft);
    };
    assert_eq!(alpha(&app, 0, 0), 255);
    alt_click(&mut app, 0);
    assert!(
        !visible(&mut app, arrow),
        "The bottom layer has no clipping base"
    );
    alt_click(&mut app, 1);
    assert!(document(&app).document.layer(upper).unwrap().clipped);
    assert!(visible(&mut app, arrow));
    assert_eq!(alpha(&app, 0, 0), 0);
    assert_eq!(alpha(&app, 150, 100), 255);
    alt_click(&mut app, 1);
    assert!(!visible(&mut app, arrow));
    assert_eq!(alpha(&app, 0, 0), 255);
    undo(&mut app);
    assert!(visible(&mut app, arrow));
    assert_eq!(alpha(&app, 0, 0), 0);
}
fn order(app: &App) -> Vec<u64> {
    document(app)
        .document
        .layers
        .iter()
        .map(|l| l.id.0)
        .collect()
}
fn rect(app: &mut App, id: Option<u64>) -> Rect {
    let scale = {
        let world = app.world_mut();
        let mut q = world.query_filtered::<&Window, With<PrimaryWindow>>();
        q.single(world).unwrap().scale_factor()
    };
    let world = app.world_mut();
    let mut q = world.query::<(
        &ComputedNode,
        &UiGlobalTransform,
        Option<&DropRow>,
        Option<&LayerList>,
    )>();
    let (node, t, _, _) = q
        .iter(world)
        .find(|(n, _, r, list)| {
            !n.is_empty()
                && match id {
                    Some(id) => r.is_some_and(|r| r.id == id),
                    None => list.is_some(),
                }
        })
        .unwrap();
    Rect::from_center_size(t.affine().translation / scale, node.size() / scale)
}
fn point(rect: Rect, fraction: f32) -> Vec2 {
    Vec2::new(rect.min.x + 80.0, rect.min.y + rect.height() * fraction)
}
fn start(app: &mut App, window: Entity, id: u64) {
    let p = point(rect(app, Some(id)), 0.5);
    move_pointer(app, window, p);
    mouse(app, window, ButtonState::Pressed);
    assert!(
        app.world().resource::<LayerDrag>().capturing(),
        "The native row must capture a real pointer press"
    );
}
fn target(app: &App, id: u64, place: Place) {
    let drag = app.world().resource::<LayerDrag>();
    assert!(drag.active);
    let t = drag.target.as_ref().expect("Visible insertion target");
    assert_eq!((t.id, t.place), (id, place));
    assert!(!t.unchanged);
}
fn visible(app: &mut App, label: &str) -> bool {
    let world = app.world_mut();
    world
        .query::<(&AccessibleLabel, &ComputedNode)>()
        .iter(world)
        .any(|(l, n)| l.0 == label && !n.is_empty())
}
fn gap(app: &mut App) -> Rect {
    let world = app.world_mut();
    let (node, t) = world
        .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
        .iter(world)
        .find_map(|(l, n, t)| (l.0 == "Место для слоя" && !n.is_empty()).then_some((n, t)))
        .unwrap();
    Rect::from_center_size(
        t.affine().translation * node.inverse_scale_factor(),
        node.size() * node.inverse_scale_factor(),
    )
}
fn hidden(app: &mut App, id: u64) -> bool {
    let world = app.world_mut();
    world
        .query::<(&DropRow, &Visibility)>()
        .iter(world)
        .any(|(r, v)| r.id == id && *v == Visibility::Hidden)
}
fn release(app: &mut App, window: Entity) {
    mouse(app, window, ButtonState::Released);
    warm(app, 4);
    assert!(!app.world().resource::<LayerDrag>().capturing());
    assert!(!visible(app, "Перетаскиваемый слой"));
    assert!(!visible(app, "Место вставки слоя"));
    assert!(!visible(app, "Место для слоя"));
    {
        let world = app.world_mut();
        for (_, slide, visibility) in world.query::<(&DropRow, &Slide, &Visibility)>().iter(world) {
            assert_eq!(
                slide.offset, 0.0,
                "No row translation may leak past the drag"
            );
            assert_eq!(*visibility, Visibility::Inherited);
        }
    }
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .layers
            .form
            .is_none(),
        "A drop must not be interpreted as a double-click rename"
    );
}
fn undo(app: &mut App) {
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Undo));
    warm(app, 4);
}
fn escape(app: &mut App, window: Entity) {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code: KeyCode::Escape,
            logical_key: Key::Escape,
            state,
            text: None,
            repeat: false,
        });
        frame(app);
    }
}

#[test]
#[ignore = "requires GPU: real layer pointer capture, lifted row, insertion marker, groups, cancellation, undo, DPI and autoscroll"]
fn gpu_layer_drag_matches_preview_and_commits_once() {
    let fixture = Fixture::new();
    let (mut app, window, image_target) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.0.clone();
            editor.project.asset_root = fixture.0.clone();
            editor.drawer = false;
            browser.refresh(&mut editor.project);
        });
    let id = file(&app, "b-transparent.png");
    app.world_mut().write_message(Action::ViewImage(id));
    warm(&mut app, 30);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    for _ in 0..120 {
        frame(&mut app);
        if app.world().resource::<ImageViewer>().edit.editing(Some(id)) {
            break;
        }
    }
    warm(&mut app, 12);
    let a = document(&app).active_id().unwrap().0;
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::AddLayer));
    warm(&mut app, 4);
    let b = document(&app).active_id().unwrap().0;
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::AddLayer));
    warm(&mut app, 4);
    let c = document(&app).active_id().unwrap().0;
    layer(&mut app, L::Group);
    let g = document(&app).active_id().unwrap().0;
    assert_eq!(order(&app), [a, b, c, g]);
    let pixels = document(&app).composite().pixels;
    let scene = app.world().resource::<Editor>().edits.clone();
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/layer-drag-qa");
    std::fs::create_dir_all(&output).unwrap();

    // The marker and the engine must agree at both ordinary and fractional Windows DPI.
    for scale in [1.0, 1.5] {
        super::panel_sizing::resize_target(&mut app, window, &image_target, 1280.0, scale);
        let revision = document(&app).revision;
        let original_top = rect(&mut app, Some(g)).min.y;
        let height = rect(&mut app, Some(c)).height();
        start(&mut app, window, c);
        let p = point(rect(&mut app, Some(g)), 0.1);
        move_pointer(&mut app, window, p);
        target(&app, g, Place::Above);
        warm(&mut app, 18);
        target(&app, g, Place::Above);
        let preview_top = rect(&mut app, Some(g)).min.y;
        let vacant = gap(&mut app);
        assert!((vacant.height() - height).abs() < 1.0);
        assert!((vacant.min.y - original_top).abs() < 1.0);
        assert!(
            (preview_top - original_top - height - 2.0).abs() < 1.0,
            "Other rows must move aside by the lifted row's height"
        );
        assert!(hidden(&mut app, c));
        assert_eq!(
            order(&app),
            [a, b, c, g],
            "Pointer motion must never change the document"
        );
        assert_eq!(document(&app).revision, revision);
        assert!(visible(&mut app, "Перетаскиваемый слой"));
        assert!(visible(&mut app, "Место вставки слоя"));
        if scale == 1.0 {
            let first = capture_when(
                &mut app,
                &image_target,
                &output.join("01-reorder.png"),
                |_| true,
            );
            warm(&mut app, 40);
            target(&app, g, Place::Above);
            let second = capture_when(
                &mut app,
                &image_target,
                &output.join("07-moving-rim.png"),
                |_| true,
            );
            let mut changing = 0;
            for y in (vacant.min.y as u32 + 1)..(vacant.max.y as u32 - 1) {
                for x in (vacant.min.x as u32 + 1)..(vacant.max.x as u32 - 1) {
                    if first.get_pixel(x, y).0[..3]
                        .iter()
                        .zip(&second.get_pixel(x, y).0[..3])
                        .any(|(a, b)| a.abs_diff(*b) > 2)
                    {
                        changing += 1;
                    }
                }
            }
            assert!(
                changing > 20,
                "The shader must animate on a stationary drop slot, without moving its hit target"
            );
        }
        release(&mut app, window);
        assert!(visible(&mut app, "Подтверждение переноса слоя"));
        if scale == 1.0 {
            capture_when(
                &mut app,
                &image_target,
                &output.join("08-landing.png"),
                |_| true,
            );
        }
        warm(&mut app, 24);
        assert!(
            !visible(&mut app, "Подтверждение переноса слоя"),
            "Drop confirmation must expire without leaving an overlay"
        );
        assert_eq!(order(&app), [a, b, g, c]);
        assert!(
            (rect(&mut app, Some(c)).min.y - vacant.min.y).abs() < 1.0,
            "The dropped layer must occupy the previewed vacancy"
        );
        assert!(
            (rect(&mut app, Some(g)).min.y - preview_top).abs() < 1.0,
            "The surrounding rows must stay where the preview placed them"
        );
        // Revision IDs are monotonic even across Undo: one undo restoring the
        // exact old stack/revision is the meaningful single-transaction check.
        assert_ne!(document(&app).revision, revision);
        undo(&mut app);
        assert_eq!(order(&app), [a, b, c, g]);
        assert_eq!(document(&app).revision, revision);
    }
    super::panel_sizing::resize_target(&mut app, window, &image_target, 1280.0, 1.0);

    // Alt-copy keeps its source and still reserves enough scrollable room.
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::AltLeft);
    frame(&mut app);
    start(&mut app, window, a);
    let p = point(rect(&mut app, Some(g)), 0.1);
    move_pointer(&mut app, window, p);
    warm(&mut app, 18);
    target(&app, g, Place::Above);
    assert!(!hidden(&mut app, a));
    assert!(visible(&mut app, "Место для слоя"));
    {
        let world = app.world_mut();
        assert!(
            world
                .query_filtered::<&ComputedNode, With<CopyExtent>>()
                .iter(world)
                .any(|n| !n.is_empty())
        );
    }
    capture_when(
        &mut app,
        &image_target,
        &output.join("05-copy-gap.png"),
        |_| true,
    );
    release(&mut app, window);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::AltLeft);
    assert_eq!(document(&app).document.layer_count(), 5);
    undo(&mut app);
    assert_eq!(order(&app), [a, b, c, g]);

    layer(&mut app, L::Collapse(g));
    start(&mut app, window, c);
    let p = point(rect(&mut app, Some(g)), 0.5);
    move_pointer(&mut app, window, p);
    target(&app, g, Place::Into);
    assert!(visible(&mut app, "Перенос внутрь группы"));
    warm(&mut app, 45);
    assert!(
        matches!(
            &document(&app)
                .document
                .layer(photocraft_doc::LayerId(g))
                .unwrap()
                .content,
            photocraft_doc::LayerContent::Group(group) if group.expanded
        ),
        "Hovering a collapsed group must expand it without losing the captured drag"
    );
    target(&app, g, Place::Into);
    capture_when(
        &mut app,
        &image_target,
        &output.join("02-into-group.png"),
        |_| true,
    );
    release(&mut app, window);
    assert_eq!(
        document(&app)
            .document
            .path_of(photocraft_doc::LayerId(c))
            .unwrap()
            .len(),
        2
    );
    assert_eq!(order(&app), [a, b, g]);

    // The empty tail is an explicit root-level target for taking a child out of a group.
    start(&mut app, window, c);
    let r = rect(&mut app, Some(a));
    move_pointer(&mut app, window, Vec2::new(r.min.x + 80.0, r.max.y + 24.0));
    target(&app, a, Place::Below);
    capture_when(
        &mut app,
        &image_target,
        &output.join("03-out-of-group.png"),
        |_| true,
    );
    release(&mut app, window);
    assert_eq!(order(&app), [c, a, b, g]);
    undo(&mut app);
    undo(&mut app);
    assert_eq!(order(&app), [a, b, c, g]);

    // Escape and dropping over the canvas cancel without painting, renaming, or history.
    for cancel in [true, false] {
        let revision = document(&app).revision;
        start(&mut app, window, b);
        let p = point(rect(&mut app, Some(g)), 0.1);
        move_pointer(&mut app, window, p);
        target(&app, g, Place::Above);
        if cancel {
            escape(&mut app, window);
        } else {
            move_pointer(&mut app, window, Vec2::new(450.0, 450.0));
            assert!(app.world().resource::<LayerDrag>().target.is_none());
            let v = app.world().resource::<ImageViewer>();
            assert!(
                v.brush_cursor.is_none() && v.brush_cursor_position.is_none(),
                "A layer drag over the canvas must not show a brush cursor"
            );
        }
        release(&mut app, window);
        assert_eq!(order(&app), [a, b, c, g]);
        assert_eq!(document(&app).revision, revision);
        assert_eq!(document(&app).composite().pixels, pixels);
        assert_ne!(
            app.world().get::<bevy::window::CursorIcon>(window),
            Some(&bevy::window::CursorIcon::from(
                bevy::window::SystemCursorIcon::Grabbing
            )),
            "The drag cursor must be released after dropping or cancelling"
        );
    }

    // Multi-selection keeps its stacking order and has one undo step.
    let walk = document(&app).document.walk();
    let ai = walk.iter().position(|(_, _, l)| l.id.0 == a).unwrap();
    let bi = walk.iter().position(|(_, _, l)| l.id.0 == b).unwrap();
    layer(&mut app, L::Select(ai, false, false));
    layer(&mut app, L::Select(bi, true, false));
    start(&mut app, window, b);
    let p = point(rect(&mut app, Some(g)), 0.1);
    move_pointer(&mut app, window, p);
    target(&app, g, Place::Above);
    release(&mut app, window);
    assert_eq!(order(&app), [c, g, a, b]);
    undo(&mut app);
    assert_eq!(order(&app), [a, b, c, g]);

    // A subsequent ordinary click is not swallowed by the previous drag.
    click_action(
        &mut app,
        window,
        |action| matches!(action,Action::ImageEdit(edit::EditAction::SelectLayer(index)) if index==ai),
    );
    warm(&mut app, 4);
    assert_eq!(document(&app).active_id().unwrap().0, a);
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .layers
            .form
            .is_none()
    );

    // A captured ID survives row rebuilds while the list autoscrolls.
    for _ in 0..18 {
        app.world_mut()
            .write_message(Action::ImageEdit(edit::EditAction::AddLayer));
        warm(&mut app, 2);
    }
    {
        let world = app.world_mut();
        let mut q = world.query_filtered::<&mut ScrollPosition, With<LayerList>>();
        q.single_mut(world).unwrap().y = 0.0;
    }
    warm(&mut app, 4);
    let source = *order(&app).last().unwrap();
    let before = order(&app);
    start(&mut app, window, source);
    let viewport = rect(&mut app, None);
    move_pointer(
        &mut app,
        window,
        Vec2::new(viewport.min.x + 100.0, viewport.max.y - 5.0),
    );
    warm(&mut app, 40);
    {
        let world = app.world_mut();
        let mut q = world.query_filtered::<&ScrollPosition, With<LayerList>>();
        assert!(
            q.single(world).unwrap().y > 20.0,
            "Dragging at the lower edge must scroll the list"
        );
    }
    assert!(visible(&mut app, "Перетаскиваемый слой"));
    capture_when(
        &mut app,
        &image_target,
        &output.join("04-autoscroll.png"),
        |_| true,
    );
    escape(&mut app, window);
    release(&mut app, window);
    assert_eq!(order(&app), before);
    assert_eq!(app.world().resource::<Editor>().edits, scene);
    assert_eq!(document(&app).composite().pixels, pixels);
}
