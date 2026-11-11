use super::*;
use crate::LevelEdits;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "bevy-editor-lighting-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn legacy_documents_preserve_host_lighting_and_stay_byte_equivalent_when_empty() {
    let edits: LevelEdits = serde_json::from_str("{}").unwrap();
    assert!(edits.lights.is_empty());
    assert!(edits.lighting.is_none());
    assert_eq!(serde_json::to_string(&edits).unwrap(), "{}");
    let legacy: LevelEdits =
        serde_json::from_str(r#"{"nodes":{},"cameras":{},"models":[]}"#).unwrap();
    assert_eq!(legacy, edits);
}

#[test]
fn authored_lights_and_environment_roundtrip_with_typed_camera_keys() {
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    enum Camera {
        Main,
    }
    let directory = Directory::new();
    let (mut settings, lights) = LightingPreset::Studio.create(10);
    settings.fog = Some(FogSettings::default());
    let edits = LevelEdits::<Camera> {
        lights,
        lighting: Some(settings),
        ..Default::default()
    };
    edits.write(&directory.0, "levels/studio.json").unwrap();
    assert_eq!(
        LevelEdits::<Camera>::read(&directory.0, "levels/studio.json").unwrap(),
        edits
    );
    let editor: LevelEdits<String> = LevelEdits::read(&directory.0, "levels/studio.json").unwrap();
    assert_eq!(editor.lights, edits.lights);
    assert_eq!(editor.lighting, edits.lighting);
}

#[test]
fn invalid_lighting_and_duplicate_ids_do_not_replace_the_last_saved_document() {
    let directory = Directory::new();
    let mut edits = LevelEdits::<String> {
        lights: vec![LightPreset::Bulb.create(7)],
        ..Default::default()
    };
    edits.write(&directory.0, "scene.json").unwrap();
    let original = fs::read(directory.0.join("scene.json")).unwrap();
    edits.lights.push(LightPreset::Sun.create(7));
    assert!(edits.write(&directory.0, "scene.json").is_err());
    edits.lights.pop();
    edits.lights[0].intensity = f32::NAN;
    assert!(edits.write(&directory.0, "scene.json").is_err());
    edits.lights[0].intensity = 800.0;
    edits.lighting = Some(SceneLighting {
        shadow_resolution: 300,
        ..Default::default()
    });
    assert!(edits.write(&directory.0, "scene.json").is_err());
    assert_eq!(fs::read(directory.0.join("scene.json")).unwrap(), original);
}

#[test]
fn every_preset_is_valid_and_scene_presets_assign_distinct_ids_even_at_wraparound() {
    for preset in LightPreset::ALL {
        let light = preset.create(42);
        light.validate().unwrap();
        assert_eq!(light.id, 42);
        assert!(!light.name.is_empty());
    }
    assert_eq!(LightPreset::Softbox.create(1).kind, LightKind::Rect);
    assert!(!LightPreset::Softbox.create(1).shadows);
    for preset in LightingPreset::ALL {
        let (settings, lights) = preset.create(u32::MAX);
        settings.validate().unwrap();
        let mut ids = std::collections::HashSet::new();
        for light in lights {
            light.validate().unwrap();
            assert!(ids.insert(light.id));
        }
    }
}

#[test]
fn malformed_and_nonfinite_light_values_are_rejected_with_kind_specific_constraints() {
    let changes: &[fn(&mut SceneLight)] = &[
        |light| light.name.clear(),
        |light| light.name = "x".repeat(129),
        |light| light.name = "bad\nname".into(),
        |light| light.pose.rotation[0] = f32::NAN,
        |light| light.color[1] = 1.1,
        |light| light.temperature = Some(f32::INFINITY),
        |light| light.intensity = -1.0,
        |light| light.range = 0.0,
        |light| light.radius = light.range + 1.0,
        |light| light.width = 0.0,
        |light| light.height = f32::NAN,
        |light| {
            light.kind = LightKind::Spot;
            light.inner_angle = 60.0;
            light.outer_angle = 30.0;
        },
        |light| {
            light.kind = LightKind::Directional;
            light.shadow_near = light.shadow_distance;
        },
    ];
    for change in changes {
        let mut light = SceneLight::default();
        change(&mut light);
        assert!(
            light.validate().is_err(),
            "invalid light accepted: {light:?}"
        );
    }
    let rect = SceneLight {
        kind: LightKind::Rect,
        range: 0.01,
        inner_angle: 60.0,
        outer_angle: 30.0,
        ..Default::default()
    };
    rect.validate().unwrap();
    let point = SceneLight {
        inner_angle: 60.0,
        outer_angle: 30.0,
        ..Default::default()
    };
    point.validate().unwrap();
}

#[test]
fn environment_fog_ranges_and_temperature_tints_are_finite_and_predictable() {
    for size in [0, 127, 300, 16384] {
        assert!(
            SceneLighting {
                shadow_resolution: size,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        SceneLighting {
            exposure_ev100: f32::NAN,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        FogSettings {
            start: 20.0,
            end: 10.0,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        FogSettings {
            end: f32::INFINITY,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    let warm = temperature_rgb(1850.0);
    let cold = temperature_rgb(10000.0);
    assert!(warm[0] > warm[2]);
    assert!(cold[2] > cold[0]);
    assert_eq!(temperature_rgb(f32::NAN), [1.0; 3]);
    let mut light = SceneLight {
        color: [0.1, 0.5, 0.8],
        ..Default::default()
    };
    assert_eq!(light.resolved_rgb(), light.color);
    light.temperature = Some(2700.0);
    assert!(
        light
            .resolved_rgb()
            .into_iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
    );
    assert!(serde_json::from_str::<SceneLight>(r#"{"kind":"unknown"}"#).is_err());
    assert!(serde_json::from_str::<SceneLighting>(r#"{"unexpected":true}"#).is_err());
}

#[cfg(feature = "lighting")]
mod rendering {
    use super::*;
    use bevy::{
        camera::{Exposure, Hdr, visibility::RenderLayers},
        ecs::world::CommandQueue,
        light::{CascadeShadowConfig, RectLight},
        pbr::{ContactShadows, DistanceFog},
        prelude::*,
        render::view::Tonemapping,
    };

    fn edit(world: &mut World, entity: Entity, apply: impl FnOnce(&mut EntityCommands)) {
        let mut queue = CommandQueue::default();
        {
            let mut commands = Commands::new(&mut queue, world);
            apply(&mut commands.entity(entity));
        }
        queue.apply(world);
    }

    #[test]
    fn light_kinds_replace_components_and_preserve_layers_and_physical_units() {
        let mut world = World::new();
        let entity = world.spawn(RenderLayers::layer(6)).id();
        let point = LightPreset::Bulb.create(1);
        edit(&mut world, entity, |entity| apply_light(entity, &point));
        assert_eq!(world.get::<PointLight>(entity).unwrap().intensity, 800.0);
        let rect = LightPreset::Softbox.create(1);
        edit(&mut world, entity, |entity| apply_light(entity, &rect));
        assert!(world.get::<PointLight>(entity).is_none());
        assert_eq!(world.get::<RectLight>(entity).unwrap().width, rect.width);
        let sun = LightPreset::Sun.create(1);
        edit(&mut world, entity, |entity| apply_light(entity, &sun));
        assert!(world.get::<RectLight>(entity).is_none());
        assert_eq!(
            world.get::<DirectionalLight>(entity).unwrap().illuminance,
            100000.0
        );
        assert!(world.get::<CascadeShadowConfig>(entity).is_some());
        let spot = LightPreset::Spot.create(1);
        edit(&mut world, entity, |entity| apply_light(entity, &spot));
        assert!(world.get::<DirectionalLight>(entity).is_none());
        assert!(world.get::<CascadeShadowConfig>(entity).is_none());
        assert!(
            (world.get::<SpotLight>(entity).unwrap().outer_angle - spot.outer_angle.to_radians())
                .abs()
                < 1e-6
        );
        assert_eq!(
            *world.get::<RenderLayers>(entity).unwrap(),
            RenderLayers::layer(6)
        );
        let off = SceneLight {
            enabled: false,
            ..point
        };
        edit(&mut world, entity, |entity| apply_light(entity, &off));
        assert_eq!(
            *world.get::<Visibility>(entity).unwrap(),
            Visibility::Hidden
        );
    }

    #[test]
    fn camera_settings_and_imported_light_switches_restore_from_caller_snapshots() {
        let mut world = World::new();
        let camera = world
            .spawn((Camera3d::default(), RenderLayers::layer(7)))
            .id();
        let mut settings = SceneLighting {
            fog: Some(FogSettings::default()),
            ..Default::default()
        };
        edit(&mut world, camera, |entity| {
            apply_camera_lighting(entity, &settings)
        });
        assert!(world.get::<Hdr>(camera).is_some());
        assert!(world.get::<ContactShadows>(camera).is_some());
        assert!(world.get::<DistanceFog>(camera).is_some());
        assert_eq!(
            world.get::<Exposure>(camera).unwrap().ev100,
            settings.exposure_ev100
        );
        settings.fog = None;
        settings.scene_lights = false;
        settings.shadows = false;
        edit(&mut world, camera, |entity| {
            apply_camera_lighting(entity, &settings)
        });
        assert!(world.get::<DistanceFog>(camera).is_none());
        assert!(world.get::<ContactShadows>(camera).is_none());
        assert_eq!(
            *world.get::<RenderLayers>(camera).unwrap(),
            RenderLayers::layer(7)
        );
        let original = PointLight {
            intensity: 1234.0,
            shadow_maps_enabled: true,
            contact_shadows_enabled: true,
            ..default()
        };
        let light = world.spawn_empty().id();
        edit(&mut world, light, |entity| {
            apply_imported_light(entity, &settings, Some(&original), None, None, None)
        });
        let disabled = world.get::<PointLight>(light).unwrap();
        assert_eq!(disabled.intensity, 0.0);
        assert!(!disabled.shadow_maps_enabled);
        assert!(!disabled.contact_shadows_enabled);
        edit(&mut world, light, |entity| {
            apply_imported_light(
                entity,
                &SceneLighting::default(),
                Some(&original),
                None,
                None,
                None,
            )
        });
        assert_eq!(world.get::<PointLight>(light).unwrap().intensity, 1234.0);
        assert!(world.get::<PointLight>(light).unwrap().shadow_maps_enabled);
        assert_eq!(original.intensity, 1234.0);
        for tone in ToneMap::ALL {
            assert_eq!(ToneMap::from(Tonemapping::from(tone)), tone);
        }
    }
}
