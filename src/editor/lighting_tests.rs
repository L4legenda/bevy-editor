use super::*;
use crate::editor::{Action, RoomEdits, handle_actions, viewport};

fn app() -> App {
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let mut editor = Editor::new(project, RoomEdits::default());
    editor.ready = true;
    editor.reload = false;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(editor)
        .init_resource::<Scene>()
        .init_resource::<viewport::PointerState>()
        .add_message::<Action>()
        .add_systems(Update, (handle_actions, sync).chain());
    app
}

fn action(app: &mut App, action: LightingAction) {
    app.world_mut().write_message(Action::Lighting(action));
    app.update();
}

fn history(app: &mut App, action: Action) {
    app.world_mut().write_message(action);
    app.update();
}

#[test]
fn cloudy_is_automatic_without_overwriting_saved_off_or_dirtying_the_scene() {
    let mut app = app();
    let camera = app
        .world_mut()
        .spawn((EditorCamera, Camera3d::default()))
        .id();
    app.update();
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.environment().hdri, Some(EnvironmentPreset::Cloudy));
    assert!(editor.environment().hdri_background);
    assert!(editor.edits.lighting.is_none());
    assert!(!editor.dirty());
    assert!(editor.undo.is_empty());
    assert!(
        app.world()
            .get::<crate::hdri::CameraEnvironment>(camera)
            .is_some()
    );

    action(&mut app, LightingAction::SetEnvironment(None));
    assert_eq!(app.world().resource::<Editor>().environment().hdri, None);
    assert!(
        app.world()
            .get::<crate::hdri::CameraEnvironment>(camera)
            .is_none()
    );
    // A saved explicit "off" remains off on subsequent synchronization frames.
    app.update();
    assert!(
        app.world()
            .get::<crate::hdri::CameraEnvironment>(camera)
            .is_none()
    );
    history(&mut app, Action::Undo);
    assert_eq!(
        app.world().resource::<Editor>().environment().hdri,
        Some(EnvironmentPreset::Cloudy)
    );
    assert!(
        app.world()
            .get::<crate::hdri::CameraEnvironment>(camera)
            .is_some()
    );
    assert!(!app.world().resource::<Editor>().dirty());

    action(
        &mut app,
        LightingAction::SetEnvironment(Some(EnvironmentPreset::Forest)),
    );
    app.update();
    assert_eq!(
        app.world().resource::<Editor>().environment().hdri,
        Some(EnvironmentPreset::Forest)
    );
}

#[test]
fn hdri_selection_rotation_brightness_and_off_are_undoable() {
    let mut app = app();
    history(&mut app, Action::Select(Selection::Environment));
    action(
        &mut app,
        LightingAction::SetEnvironment(Some(EnvironmentPreset::SmallStudio)),
    );
    action(
        &mut app,
        LightingAction::SetNumber(Field::Environment(EnvironmentField::HdriRotation), 90.0),
    );
    action(
        &mut app,
        LightingAction::SetNumber(Field::Environment(EnvironmentField::HdriBrightness), 2500.0),
    );
    let settings = app.world().resource::<Editor>().environment();
    assert_eq!(settings.hdri, Some(EnvironmentPreset::SmallStudio));
    assert_eq!(settings.hdri_rotation, 90.0);
    assert_eq!(settings.hdri_brightness, 2500.0);
    assert!(settings.hdri_background);
    action(&mut app, LightingAction::ToggleHdriBackground);
    assert!(
        !app.world()
            .resource::<Editor>()
            .environment()
            .hdri_background
    );
    history(&mut app, Action::Undo);
    assert!(
        app.world()
            .resource::<Editor>()
            .environment()
            .hdri_background
    );
    action(
        &mut app,
        LightingAction::SetEnvironment(Some(EnvironmentPreset::Forest)),
    );
    history(&mut app, Action::Undo);
    assert_eq!(
        app.world().resource::<Editor>().environment().hdri,
        Some(EnvironmentPreset::SmallStudio)
    );
    action(&mut app, LightingAction::SetEnvironment(None));
    assert_eq!(app.world().resource::<Editor>().environment().hdri, None);
    history(&mut app, Action::Undo);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .environment()
            .hdri_brightness,
        2500.0
    );
    history(&mut app, Action::Redo);
    assert_eq!(app.world().resource::<Editor>().environment().hdri, None);
    action(&mut app, LightingAction::ResetEnvironment);
    assert!(app.world().resource::<Editor>().edits.lighting.is_none());
}

#[test]
fn environment_override_and_reset_restore_host_reflections() {
    use bevy::light::{EnvironmentMapLight, Skybox};
    let mut app = app();
    let original = EnvironmentMapLight {
        intensity: 42.0,
        rotation: Quat::from_rotation_y(0.4),
        ..default()
    };
    let skybox = Skybox {
        image: Some(Handle::default()),
        brightness: 42.0,
        rotation: original.rotation,
    };
    let camera = app
        .world_mut()
        .spawn((
            EditorCamera,
            Camera3d::default(),
            original.clone(),
            skybox.clone(),
        ))
        .id();
    app.update();
    history(&mut app, Action::Select(Selection::Environment));
    action(&mut app, LightingAction::SetEnvironment(None));
    assert!(app.world().get::<EnvironmentMapLight>(camera).is_none());
    assert!(app.world().get::<Skybox>(camera).is_none());
    action(
        &mut app,
        LightingAction::SetEnvironment(Some(EnvironmentPreset::Cloudy)),
    );
    assert!(
        app.world()
            .get::<crate::hdri::CameraEnvironment>(camera)
            .is_some()
    );
    action(&mut app, LightingAction::ResetEnvironment);
    let restored = app.world().get::<EnvironmentMapLight>(camera).unwrap();
    assert_eq!(restored.intensity, original.intensity);
    assert_eq!(restored.rotation, original.rotation);
    let restored_skybox = app.world().get::<Skybox>(camera).unwrap();
    assert_eq!(restored_skybox.image, skybox.image);
    assert_eq!(restored_skybox.brightness, skybox.brightness);
    assert_eq!(restored_skybox.rotation, skybox.rotation);
    assert!(
        app.world()
            .get::<crate::hdri::CameraEnvironment>(camera)
            .is_none()
    );
}

#[test]
fn environment_selection_edits_and_history_do_not_require_a_transform() {
    let mut app = app();
    history(&mut app, Action::Select(Selection::Environment));
    assert_eq!(
        app.world().resource::<Editor>().selection,
        Some(Selection::Environment)
    );
    action(
        &mut app,
        LightingAction::SetNumber(Field::Environment(EnvironmentField::Exposure), 14.0),
    );
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .edits
            .lighting
            .as_ref()
            .unwrap()
            .exposure_ev100,
        14.0
    );
    assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
    history(&mut app, Action::Undo);
    assert!(app.world().resource::<Editor>().edits.lighting.is_none());
    assert_eq!(
        app.world().resource::<Editor>().selection,
        Some(Selection::Environment)
    );
    history(&mut app, Action::Redo);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .edits
            .lighting
            .as_ref()
            .unwrap()
            .exposure_ev100,
        14.0
    );
}

#[test]
fn a_light_and_a_complete_rig_each_have_one_undo_step() {
    let mut app = app();
    action(&mut app, LightingAction::Add(LightPreset::Bulb));
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.edits.lights.len(), 1);
    assert_eq!(editor.undo.len(), 1);
    assert!(editor.dirty());
    let light = editor.edits.lights[0].clone();
    assert_eq!(editor.selection, Some(Selection::Light(light.id)));
    action(
        &mut app,
        LightingAction::ApplyPreset(LightingPreset::Studio),
    );
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.undo.len(), 2);
    assert!(!editor.edits.lighting.as_ref().unwrap().scene_lights);
    assert!(editor.edits.lights.len() > 1);
    history(&mut app, Action::Undo);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.edits.lights, [light.clone()]);
    assert_eq!(editor.edits.lighting.as_ref().unwrap().exposure_ev100, 5.0);
    history(&mut app, Action::Undo);
    assert!(app.world().resource::<Editor>().edits.lights.is_empty());
    assert!(!app.world().resource::<Editor>().dirty());
    history(&mut app, Action::Redo);
    assert_eq!(app.world().resource::<Editor>().edits.lights, [light]);
}

#[test]
fn editing_legacy_lights_initializes_exposure_but_preserves_authored_environments() {
    let mut app = app();
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.edits.lights.push(LightPreset::Bulb.create(1));
        editor.selection = Some(Selection::Light(1));
    }
    action(
        &mut app,
        LightingAction::SetNumber(Field::Light(LightField::Intensity), 1200.0),
    );
    let settings = app.world().resource::<Editor>().environment().clone();
    assert_eq!(settings.exposure_ev100, 5.0);
    assert!(!settings.scene_lights);
    action(&mut app, LightingAction::Add(LightPreset::Sun));
    assert_eq!(*app.world().resource::<Editor>().environment(), settings);
    history(&mut app, Action::Undo);
    history(&mut app, Action::Undo);
    assert!(app.world().resource::<Editor>().edits.lighting.is_none());
    assert_eq!(
        app.world().resource::<Editor>().edits.lights[0].intensity,
        800.0
    );
}

#[test]
fn invalid_numeric_edits_do_not_partially_mutate_a_light_or_its_history() {
    let mut app = app();
    action(&mut app, LightingAction::Add(LightPreset::Spot));
    let editor = app.world().resource::<Editor>();
    let light = editor.edits.lights[0].clone();
    let undo = editor.undo.len();
    for (field, value) in [
        (LightField::Intensity, f32::NAN),
        (LightField::Intensity, -1.0),
        (LightField::InnerAngle, light.outer_angle + 1.0),
        (LightField::ShadowNear, light.range),
    ] {
        action(
            &mut app,
            LightingAction::SetNumber(Field::Light(field), value),
        );
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.edits.lights[0], light);
        assert_eq!(editor.undo.len(), undo);
    }
    action(&mut app, LightingAction::SetKind(LightKind::Rect));
    let editor = app.world().resource::<Editor>();
    assert!(
        editor.lighting_field_allowed(Selection::Light(light.id), Field::Light(LightField::Width))
    );
    assert!(
        !editor
            .lighting_field_allowed(Selection::Light(light.id), Field::Light(LightField::Radius))
    );
    assert!(!editor.edits.lights[0].shadows);
    assert!(!editor.edits.lights[0].contact_shadows);
}

#[test]
fn gizmo_visibility_never_changes_saved_scene_lighting() {
    let mut app = app();
    let before = app.world().resource::<Editor>().show_lights;
    action(&mut app, LightingAction::ToggleGizmos);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.show_lights, !before);
    assert!(editor.undo.is_empty());
    assert!(!editor.dirty());
}

#[test]
fn runtime_restores_imported_lights_and_camera_defaults_after_undo() {
    let mut app = app();
    app.insert_resource(PointLightShadowMap { size: 256 });
    app.insert_resource(DirectionalLightShadowMap { size: 1024 });
    let root = app.world_mut().spawn(Transform::IDENTITY).id();
    app.world_mut().resource_mut::<Scene>().root = Some(root);
    let source = app
        .world_mut()
        .spawn((
            PointLight {
                intensity: 1234.0,
                shadow_maps_enabled: true,
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    let rect = app
        .world_mut()
        .spawn((
            RectLight {
                intensity: 2345.0,
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    let camera = app
        .world_mut()
        .spawn((
            EditorCamera,
            Camera3d::default(),
            AmbientLight {
                brightness: 23.0,
                ..default()
            },
            Exposure { ev100: 5.65 },
            Tonemapping::TonyMcMapface,
        ))
        .id();
    let mask = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            RenderLayers::layer(4),
            Exposure { ev100: 12.0 },
        ))
        .id();
    app.update();
    let defaults = &app.world().resource::<Editor>().lighting_defaults;
    assert_eq!(defaults.ambient_brightness, 23.0);
    assert_eq!(defaults.exposure_ev100, 5.65);
    assert_eq!(defaults.tonemapping, ToneMap::TonyMcMapface);
    action(&mut app, LightingAction::Add(LightPreset::Bulb));
    assert_eq!(
        app.world().get::<PointLight>(source).unwrap().intensity,
        0.0
    );
    action(&mut app, LightingAction::ToggleImportedLights);
    assert_eq!(
        app.world().get::<PointLight>(source).unwrap().intensity,
        1234.0
    );
    assert_eq!(
        app.world().get::<RectLight>(rect).unwrap().intensity,
        2345.0
    );
    action(&mut app, LightingAction::ToggleGlobalShadows);
    assert!(
        !app.world()
            .get::<PointLight>(source)
            .unwrap()
            .shadow_maps_enabled
    );
    assert_eq!(app.world().get::<Exposure>(mask).unwrap().ev100, 12.0);
    history(&mut app, Action::Undo);
    history(&mut app, Action::Undo);
    assert!(app.world().resource::<Editor>().edits.lighting.is_some());
    assert_eq!(
        app.world().get::<PointLight>(source).unwrap().intensity,
        0.0
    );
    history(&mut app, Action::Undo);
    assert!(app.world().resource::<Editor>().edits.lighting.is_none());
    assert_eq!(
        app.world().get::<PointLight>(source).unwrap().intensity,
        1234.0
    );
    assert_eq!(
        app.world().get::<RectLight>(rect).unwrap().intensity,
        2345.0
    );
    assert_eq!(app.world().get::<Exposure>(camera).unwrap().ev100, 5.65);
    assert_eq!(
        app.world().get::<Tonemapping>(camera),
        Some(&Tonemapping::TonyMcMapface)
    );
    assert!(app.world().get::<bevy::camera::Hdr>(camera).is_some());
    assert_eq!(
        app.world_mut()
            .query::<&LightEntity>()
            .iter(app.world())
            .count(),
        0
    );
    assert!(app.world().get::<ContactShadows>(camera).is_none());
    assert!(app.world().get::<DepthPrepass>(camera).is_none());
    assert_eq!(app.world().resource::<PointLightShadowMap>().size, 256);
    assert_eq!(
        app.world().resource::<DirectionalLightShadowMap>().size,
        1024
    );
}

#[test]
fn closing_the_workspace_despawns_custom_lights_without_removing_source_lights() {
    let mut app = app();
    let source = app.world_mut().spawn(PointLight::default()).id();
    action(&mut app, LightingAction::Add(LightPreset::Softbox));
    let custom: Vec<_> = app
        .world_mut()
        .query::<(Entity, &LightEntity)>()
        .iter(app.world())
        .map(|(entity, light)| {
            assert_eq!(light.0, 1);
            entity
        })
        .collect();
    assert_eq!(custom.len(), 1);
    assert!(app.world().get::<RectLight>(custom[0]).is_some());
    assert_eq!(
        app.world().get::<RenderLayers>(custom[0]),
        Some(&RenderLayers::layer(0))
    );
    app.world_mut().resource_mut::<Editor>().open_tabs.clear();
    app.update();
    assert!(!app.world().entities().contains(custom[0]));
    assert!(app.world().entities().contains(source));
}
