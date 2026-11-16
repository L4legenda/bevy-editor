use super::*;
use edit::layers::{LayerAction as L, PanelSection};

fn editing() -> (Fixture, App) {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    let id = file(&app, "d-small.png");
    app.world_mut().write_message(Action::ViewImage(id));
    wait_for_image(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    wait_for_edit(&mut app);
    (fixture, app)
}
fn layer(app: &mut App, action: L) {
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Layers(action)));
    app.update();
}
fn field(app: &App, key: &str) -> usize {
    app.world()
        .resource::<ImageViewer>()
        .edit
        .layers
        .form
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .position(|f| f.key == key || f.key.rsplit('/').next() == Some(key))
        .unwrap()
}
fn effects(app: &App) -> usize {
    let v = app.world().resource::<ImageViewer>();
    v.edit
        .document(v.tab)
        .unwrap()
        .active_layer()
        .unwrap()
        .effects
        .items
        .len()
}

#[test]
fn layer_ux_quick_actions_create_copy_group_and_mask_without_parameter_forms() {
    let (_fixture, mut app) = editing();
    layer(&mut app, L::Quick("layer.duplicate"));
    let v = app.world().resource::<ImageViewer>();
    assert_eq!(v.edit.document(v.tab).unwrap().document.layer_count(), 2);
    assert!(
        v.edit
            .document(v.tab)
            .unwrap()
            .active_layer()
            .unwrap()
            .name
            .ends_with(" — копия")
    );
    assert!(v.edit.layers.form.is_none());
    layer(&mut app, L::Quick("layer.layerMask.revealAll"));
    let v = app.world().resource::<ImageViewer>();
    assert!(
        v.edit
            .document(v.tab)
            .unwrap()
            .active_layer()
            .unwrap()
            .mask
            .is_some()
    );
    assert!(!v.edit.available(
        edit::EditAction::Layers(L::Quick("layer.layerMask.revealAll")),
        v.tab
    ));
    layer(&mut app, L::Quick("layer.layerMask.enabled"));
    let v = app.world().resource::<ImageViewer>();
    assert!(
        !v.edit
            .document(v.tab)
            .unwrap()
            .active_layer()
            .unwrap()
            .mask
            .as_ref()
            .unwrap()
            .enabled
    );
    layer(&mut app, L::Group);
    let v = app.world().resource::<ImageViewer>();
    assert!(
        v.edit
            .document(v.tab)
            .unwrap()
            .active_layer()
            .unwrap()
            .is_group()
    );
    assert!(v.edit.layers.form.is_none());
    assert!(v.edit.error.is_none(), "{:?}", v.edit.error);
}

#[test]
fn layer_ux_effect_preview_is_reversible_and_apply_is_one_undo_step() {
    let (_fixture, mut app) = editing();
    let v = app.world().resource::<ImageViewer>();
    let original = v.edit.document(v.tab).unwrap().composite().pixels;
    let original_revision = v.edit.document(v.tab).unwrap().revision;
    layer(&mut app, L::Command("layer.layerStyle.stroke"));
    assert_eq!(effects(&app), 1);
    let size = field(&app, "size");
    for value in [2.0, 4.0, 7.0] {
        layer(&mut app, L::FieldNumber(size, value));
    }
    layer(&mut app, L::Cancel);
    let v = app.world().resource::<ImageViewer>();
    let doc = v.edit.document(v.tab).unwrap();
    assert_eq!(doc.revision, original_revision);
    assert_eq!(doc.composite().pixels, original);
    assert_eq!(effects(&app), 0);
    layer(&mut app, L::Command("layer.layerStyle.stroke"));
    let size = field(&app, "size");
    layer(&mut app, L::FieldNumber(size, 4.0));
    layer(&mut app, L::Apply);
    assert_eq!(effects(&app), 1);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Undo));
    app.update();
    assert_eq!(effects(&app), 0);
    let v = app.world().resource::<ImageViewer>();
    assert_eq!(v.edit.document(v.tab).unwrap().revision, original_revision);
}

#[test]
fn layer_ux_individual_effect_changes_preserve_other_effects_and_preview_toggle() {
    let (_fixture, mut app) = editing();
    for cmd in ["layer.layerStyle.dropShadow", "layer.layerStyle.stroke"] {
        layer(&mut app, L::Command(cmd));
        layer(&mut app, L::Apply);
    }
    assert_eq!(effects(&app), 2);
    let v = app.world().resource::<ImageViewer>();
    let index = v.edit.document(v.tab).unwrap().active;
    let before = v
        .edit
        .document(v.tab)
        .unwrap()
        .active_layer()
        .unwrap()
        .effects
        .items
        .clone();
    layer(&mut app, L::EffectEdit(index, 1));
    let size = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .layers
        .form
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .position(|f| f.key == "/effects/1/params/size")
        .unwrap();
    layer(&mut app, L::Preview);
    layer(&mut app, L::FieldNumber(size, 10.0));
    let v = app.world().resource::<ImageViewer>();
    assert_eq!(
        v.edit
            .document(v.tab)
            .unwrap()
            .active_layer()
            .unwrap()
            .effects
            .items,
        before
    );
    layer(&mut app, L::Preview);
    layer(&mut app, L::Apply);
    let v = app.world().resource::<ImageViewer>();
    let after = &v
        .edit
        .document(v.tab)
        .unwrap()
        .active_layer()
        .unwrap()
        .effects
        .items;
    assert_eq!(after[0], before[0]);
    assert_ne!(after[1], before[1]);
    layer(&mut app, L::EffectToggle(index, 0));
    let v = app.world().resource::<ImageViewer>();
    assert!(
        !v.edit
            .document(v.tab)
            .unwrap()
            .active_layer()
            .unwrap()
            .effects
            .items[0]
            .enabled()
    );
    layer(&mut app, L::EffectRemove(index, 1));
    assert_eq!(effects(&app), 1);
}

#[test]
#[ignore = "requires GPU: verifies beginner layer UI, real slider input, tooltip, preview cancellation and single-step undo"]
fn layer_ux_gpu_panel_properties_preview_and_help_are_usable_without_parameter_typing() {
    use crate::editor::selection_outline::render_tests::{capture_when, frame};
    use crate::editor::tests::lighting_ui_render::{mouse, move_pointer};
    use bevy::input::ButtonState;
    let fixture = Fixture::new();
    image::RgbaImage::from_fn(480, 320, |x, y| {
        if (100..340).contains(&x) && (65..245).contains(&y) {
            image::Rgba([55, 125, 225, 255])
        } else {
            image::Rgba([0, 0, 0, 0])
        }
    })
    .save(fixture.0.join("layer-ux.png"))
    .unwrap();
    let (mut app, window, target) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.0.clone();
            editor.project.asset_root = fixture.0.clone();
            editor.drawer = false;
            browser.refresh(&mut editor.project);
        });
    let id = file(&app, "layer-ux.png");
    app.world_mut().write_message(Action::ViewImage(id));
    for _ in 0..40 {
        frame(&mut app);
    }
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    for _ in 0..120 {
        frame(&mut app);
        if app
            .world()
            .resource::<ImageViewer>()
            .edit
            .editing(app.world().resource::<Editor>().viewing_image)
        {
            break;
        }
    }
    for _ in 0..12 {
        frame(&mut app);
    }
    let id = app.world().resource::<Editor>().viewing_image.unwrap();
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/layer-ux-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(&mut app, &target, &output.join("00-layers.png"), |_| true);
    let click = |app: &mut App, wanted: fn(L) -> bool| {
        click_action(
            app,
            window,
            |action| matches!(action,Action::ImageEdit(edit::EditAction::Layers(l)) if wanted(l)),
        );
        for _ in 0..4 {
            frame(app);
        }
    };
    click(&mut app, |a| matches!(a, L::Quick("layer.duplicate")));
    assert_eq!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .document
            .layer_count(),
        2
    );
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .layers
            .form
            .is_none()
    );
    click(&mut app, |a| {
        matches!(a, L::PanelSection(PanelSection::Settings))
    });
    capture_when(&mut app, &target, &output.join("01-properties.png"), |_| {
        true
    });
    click(&mut app, |a| matches!(a, L::Category("Эффекты")));
    capture_when(
        &mut app,
        &target,
        &output.join("02-effects-menu.png"),
        |_| true,
    );
    click(&mut app, |a| {
        matches!(a, L::Command("layer.layerStyle.dropShadow"))
    });
    assert_eq!(effects(&app), 1);
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .layers
            .preview
            .is_some()
    );
    let (slider, center, width) = {
        let world = app.world_mut();
        world.query_filtered::<(Entity,&AccessibleLabel,&ComputedNode,&UiGlobalTransform),With<bevy::ui_widgets::Slider>>()
            .iter(world).find_map(|(e,label,n,t)|(label.0=="Смещение, px"&&!n.is_empty()).then_some((e,t.affine().translation,n.size().x))).unwrap()
    };
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, center + Vec2::new(-width * 0.3, 0.0));
    mouse(&mut app, window, ButtonState::Released);
    for _ in 0..5 {
        frame(&mut app);
    }
    let distance = field(&app, "distance");
    let value = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .layers
        .form
        .as_ref()
        .unwrap()
        .fields[distance]
        .text
        .parse::<f64>()
        .unwrap();
    assert!(value > 5.0 && value < 100.0, "{value}");
    assert!(
        app.world().get::<ComputedNode>(slider).is_some(),
        "Dragging preserves the live slider entity"
    );
    click_action(&mut app, window, |a| {
        matches!(
            a,
            Action::ImageEdit(edit::EditAction::ColorPicker(
                edit::ColorTarget::LayerField(_)
            ))
        )
    });
    for _ in 0..3 {
        frame(&mut app);
    }
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .color_picker
            .is_some()
    );
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut()
            .write_message(bevy::input::keyboard::KeyboardInput {
                window,
                key_code: KeyCode::Escape,
                logical_key: bevy::input::keyboard::Key::Escape,
                state,
                text: None,
                repeat: false,
            });
        frame(&mut app);
    }
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .color_picker
            .is_none()
    );
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .layers
            .form
            .is_some(),
        "Esc closes the colour picker without discarding its parent property dialog"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("03-shadow-preview.png"),
        |_| true,
    );
    move_pointer(&mut app, window, center);
    for _ in 0..55 {
        frame(&mut app);
    }
    capture_when(
        &mut app,
        &target,
        &output.join("04-property-tooltip.png"),
        |_| true,
    );
    {
        let world = app.world_mut();
        assert!(
            world
                .query::<(&Text, &ComputedNode)>()
                .iter(world)
                .any(|(t, n)| t.0.starts_with("Как далеко тень") && !n.is_empty()),
            "Explaining tooltip must appear above the property sheet"
        );
    }
    click(&mut app, |a| matches!(a, L::Cancel));
    assert_eq!(effects(&app), 0);
    click(&mut app, |a| matches!(a, L::Category("Эффекты")));
    click(&mut app, |a| {
        matches!(a, L::Command("layer.layerStyle.stroke"))
    });
    click(&mut app, |a| matches!(a, L::Apply));
    assert_eq!(effects(&app), 1);
    capture_when(
        &mut app,
        &target,
        &output.join("05-effect-in-properties.png"),
        |_| true,
    );
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Undo));
    for _ in 0..5 {
        frame(&mut app);
    }
    assert_eq!(effects(&app), 0);
    click(&mut app, |a| {
        matches!(a, L::PanelSection(PanelSection::Layers))
    });
    capture_when(
        &mut app,
        &target,
        &output.join("06-layer-stack.png"),
        |_| true,
    );
    assert!(
        app.world().resource::<ImageViewer>().edit.error.is_none(),
        "{:?}",
        app.world().resource::<ImageViewer>().edit.error
    );
}
