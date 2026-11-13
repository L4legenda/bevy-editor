use super::*;
use crate::editor::{Scene, handle_actions, viewport::PointerState};

fn fixture() -> (App, Entity, Entity) {
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let mut editor = Editor::new(project, crate::data::LevelEdits::default());
    editor.ready = true;
    editor.reload = false;
    editor.selection = Some(Selection::Environment);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(editor)
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<InputFocus>()
        .init_resource::<DrawerMotion>()
        .init_resource::<NumericInput>()
        .init_resource::<Scene>()
        .init_resource::<PointerState>()
        .add_message::<Action>()
        .add_message::<WindowCloseRequested>()
        .add_systems(Update, (input, handle_actions).chain());
    app.world_mut().spawn((
        Window {
            focused: true,
            ..default()
        },
        PrimaryWindow,
    ));
    let dummy = app.world_mut().spawn_empty().id();
    app.insert_resource(LightingUi {
        add: dummy,
        menu: dummy,
        menu_first: dummy,
        inspector: dummy,
        name: dummy,
        tooltip: dummy,
        tooltip_label: dummy,
        menu_open: false,
        advanced: false,
        pointer_consumed: false,
        controls: State::default(),
        name_draft: None,
        hint_target: None,
        hint_elapsed: 0.0,
    });
    let number = app
        .world_mut()
        .spawn(LightingSlider {
            binding: Binding::Number(Field::Environment(EnvironmentField::Exposure)),
            thumb: dummy,
            fill: None,
            readout: None,
        })
        .observe(queue_slider)
        .id();
    let plane = app
        .world_mut()
        .spawn(ColorPicker {
            swatch: Swatch::Ambient,
            hsv: Hsva::default(),
            last_rgb: [1.0; 3],
            context: None,
            marker: dummy,
            chip: dummy,
            code: dummy,
            image: Handle::default(),
            drawn_hue: 0.0,
        })
        .id();
    (app, number, plane)
}

fn value(app: &mut App, source: Entity, value: f32, is_final: bool) {
    app.world_mut().trigger(ValueChange {
        source,
        value,
        is_final,
    });
    app.update();
}

#[test]
fn hdri_slider_drags_are_live_and_each_produce_one_undo_step() {
    for field in [
        EnvironmentField::HdriRotation,
        EnvironmentField::HdriBrightness,
    ] {
        let (mut app, number, _) = fixture();
        app.world_mut()
            .write_message(Action::Lighting(LightingAction::SetEnvironment(Some(
                crate::EnvironmentPreset::SmallStudio,
            ))));
        app.update();
        app.world_mut()
            .get_mut::<LightingSlider>(number)
            .unwrap()
            .binding = Binding::Number(Field::Environment(field));
        let original = app.world().resource::<Editor>().edits.clone();
        let history = app.world().resource::<Editor>().undo.len();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        value(&mut app, number, 0.2, false);
        value(&mut app, number, 0.8, false);
        assert_ne!(app.world().resource::<Editor>().edits, original);
        assert_eq!(app.world().resource::<Editor>().undo.len(), history);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        assert_eq!(app.world().resource::<Editor>().undo.len(), history + 1);
        app.world_mut().write_message(Action::Undo);
        app.update();
        assert_eq!(app.world().resource::<Editor>().edits, original);
    }
}

#[test]
fn a_slider_drag_updates_live_and_has_one_undo_step() {
    let (mut app, number, _) = fixture();
    let original = app.world().resource::<Editor>().edits.clone();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    value(&mut app, number, 0.2, false);
    let editor = app.world().resource::<Editor>();
    assert!((editor.environment().exposure_ev100 + 3.2).abs() < 1e-4);
    assert!(editor.undo.is_empty());
    assert!(editor.dirty());
    value(&mut app, number, 0.7, false);
    value(&mut app, number, 0.5, false);
    assert!(app.world().resource::<Editor>().undo.is_empty());
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    app.update();
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.undo.len(), 1);
    assert!((editor.environment().exposure_ev100 - 7.0).abs() < 1e-4);
    app.world_mut().write_message(Action::Undo);
    app.update();
    assert_eq!(app.world().resource::<Editor>().edits, original);
    app.world_mut().write_message(Action::Redo);
    app.update();
    assert!(
        (app.world()
            .resource::<Editor>()
            .environment()
            .exposure_ev100
            - 7.0)
            .abs()
            < 1e-4
    );
}

#[test]
fn escape_restores_the_entire_gesture_and_ignores_its_late_release() {
    let (mut app, number, _) = fixture();
    let original = app.world().resource::<Editor>().edits.clone();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    value(&mut app, number, 0.9, false);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.update();
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.edits, original);
    assert!(editor.undo.is_empty());
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    value(&mut app, number, 0.3, false);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    value(&mut app, number, 0.3, true);
    assert_eq!(app.world().resource::<Editor>().edits, original);
    assert!(app.world().resource::<Editor>().undo.is_empty());
}

#[test]
fn switching_documents_finishes_the_drag_before_stashing_and_blocks_old_input() {
    let (mut app, number, _) = fixture();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    value(&mut app, number, 0.5, false);
    app.world_mut().write_message(Action::Open(1));
    app.update();
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.room, 1);
    let document = editor.documents.get(&0).unwrap();
    assert_eq!(document.undo.len(), 1);
    assert!((document.edits.lighting.as_ref().unwrap().exposure_ev100 - 7.0).abs() < 1e-4);
    let current = editor.edits.clone();
    value(&mut app, number, 0.9, false);
    assert_eq!(app.world().resource::<Editor>().edits, current);
}

#[test]
fn choosing_a_hue_on_white_keeps_it_until_saturation_changes() {
    let (mut app, _, plane) = fixture();
    app.world_mut().resource_mut::<Editor>().lighting_defaults.ambient_color = [1.0; 3];
    let dummy = app.world_mut().spawn_empty().id();
    let hue = app
        .world_mut()
        .spawn(LightingSlider {
            binding: Binding::Hue(plane),
            thumb: dummy,
            fill: None,
            readout: None,
        })
        .observe(queue_slider)
        .id();
    value(&mut app, hue, 1.0 / 3.0, true);
    assert!(!app.world().resource::<Editor>().dirty());
    assert!(app.world().resource::<Editor>().undo.is_empty());
    assert!((app.world().get::<ColorPicker>(plane).unwrap().hsv.hue - 120.0).abs() < 1e-4);
    app.world_mut()
        .resource_mut::<LightingUi>()
        .controls
        .queued
        .push(Queued {
            source: plane,
            context: Some((0, Selection::Environment)),
            change: Change::Plane(Vec2::new(0.8, -0.2)),
            final_value: true,
        });
    app.update();
    let editor = app.world().resource::<Editor>();
    let [red, green, blue] = editor.environment().ambient_color;
    assert!((green - 0.8).abs() < 1e-4 && red < 0.2 && blue < 0.2);
    assert_eq!(editor.undo.len(), 1);
    app.world_mut().write_message(Action::Undo);
    app.update();
    assert!(!app.world().resource::<Editor>().dirty());
}

#[test]
fn slider_limits_keep_coupled_lighting_values_valid_and_roundtrip() {
    let (mut app, _, _) = fixture();
    let editor = &mut *app.world_mut().resource_mut::<Editor>();
    for preset in [
        LightPreset::Bulb,
        LightPreset::Spot,
        LightPreset::Sun,
        LightPreset::Softbox,
    ] {
        let mut light = preset.create(1);
        light.temperature = Some(6500.0);
        editor.edits.lights = vec![light];
        editor.selection = Some(Selection::Light(1));
        for field in [
            LightField::Intensity,
            LightField::Range,
            LightField::Radius,
            LightField::Width,
            LightField::Height,
            LightField::InnerAngle,
            LightField::OuterAngle,
            LightField::Temperature,
            LightField::ShadowDepthBias,
            LightField::ShadowNormalBias,
            LightField::ShadowNear,
            LightField::ShadowDistance,
        ] {
            let field = Field::Light(field);
            if !editor.field_allowed(Selection::Light(1), field) {
                continue;
            }
            let range = limits(editor, field).unwrap();
            for t in [0.0, 0.25, 0.5, 1.0] {
                let value = range.value(t);
                assert!(
                    editor.validate_lighting_field(field, value).is_ok(),
                    "{preset:?} {field:?}: {value}"
                );
                assert!((range.position(value) - t).abs() < 1e-4);
            }
        }
    }
}
