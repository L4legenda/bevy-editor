//! Editor mutations and live synchronization for persistent scene lighting.
use std::collections::{HashMap, HashSet};

use bevy::{
    camera::{Exposure, visibility::RenderLayers},
    core_pipeline::prepass::DepthPrepass,
    ecs::world::CommandQueue,
    light::{DirectionalLightShadowMap, GlobalAmbientLight, PointLightShadowMap, RectLight},
    pbr::{ContactShadows, DistanceFog, FogFalloff},
    prelude::*,
    render::view::Tonemapping,
};

use super::{Editor, Field, Scene, Selection, ui, viewport::EditorCamera};
use crate::lighting::{
    FogSettings, LightKind, LightPreset, LightingPreset, SceneLight, SceneLighting, ToneMap,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LightField {
    Intensity,
    Range,
    Radius,
    Width,
    Height,
    InnerAngle,
    OuterAngle,
    Temperature,
    ShadowDepthBias,
    ShadowNormalBias,
    ShadowNear,
    ShadowDistance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EnvironmentField {
    AmbientBrightness,
    Exposure,
    FogStart,
    FogEnd,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum LightingAction {
    Add(LightPreset),
    ApplyPreset(LightingPreset),
    SetKind(LightKind),
    ToggleEnabled,
    ToggleShadows,
    ToggleContactShadows,
    ToggleTemperature,
    SetColor([u8; 3]),
    SetNumber(Field, f32),
    ToggleImportedLights,
    ToggleFog,
    ToggleGlobalShadows,
    SetShadowResolution(u32),
    CycleToneMapping,
    ResetEnvironment,
    AimAtView,
    PlaceAtView,
    ToggleGizmos,
}

fn fog_default() -> FogSettings {
    FogSettings {
        color: [0.55, 0.60, 0.70],
        start: 10.0,
        end: 60.0,
    }
}

fn light_number(light: &SceneLight, field: LightField) -> f32 {
    match field {
        LightField::Intensity => light.intensity,
        LightField::Range => light.range,
        LightField::Radius => light.radius,
        LightField::Width => light.width,
        LightField::Height => light.height,
        LightField::InnerAngle => light.inner_angle,
        LightField::OuterAngle => light.outer_angle,
        LightField::Temperature => light.temperature.unwrap_or(6500.0),
        LightField::ShadowDepthBias => light.shadow_depth_bias,
        LightField::ShadowNormalBias => light.shadow_normal_bias,
        LightField::ShadowNear => light.shadow_near,
        LightField::ShadowDistance => light.shadow_distance,
    }
}

fn set_light_number(light: &mut SceneLight, field: LightField, value: f32) {
    match field {
        LightField::Intensity => light.intensity = value,
        LightField::Range => light.range = value,
        LightField::Radius => light.radius = value,
        LightField::Width => light.width = value,
        LightField::Height => light.height = value,
        LightField::InnerAngle => light.inner_angle = value,
        LightField::OuterAngle => light.outer_angle = value,
        LightField::Temperature => light.temperature = Some(value),
        LightField::ShadowDepthBias => light.shadow_depth_bias = value,
        LightField::ShadowNormalBias => light.shadow_normal_bias = value,
        LightField::ShadowNear => light.shadow_near = value,
        LightField::ShadowDistance => light.shadow_distance = value,
    }
}

fn environment_number(environment: &SceneLighting, field: EnvironmentField) -> f32 {
    let fog = environment.fog.clone().unwrap_or_else(fog_default);
    match field {
        EnvironmentField::AmbientBrightness => environment.ambient_brightness,
        EnvironmentField::Exposure => environment.exposure_ev100,
        EnvironmentField::FogStart => fog.start,
        EnvironmentField::FogEnd => fog.end,
    }
}

fn set_environment_number(environment: &mut SceneLighting, field: EnvironmentField, value: f32) {
    match field {
        EnvironmentField::AmbientBrightness => environment.ambient_brightness = value,
        EnvironmentField::Exposure => environment.exposure_ev100 = value,
        _ => {
            let fog = environment.fog.get_or_insert_with(fog_default);
            match field {
                EnvironmentField::FogStart => fog.start = value,
                EnvironmentField::FogEnd => fog.end = value,
                _ => {}
            }
        }
    }
}

impl Editor {
    pub(super) fn light(&self, id: u32) -> Option<&SceneLight> {
        self.edits.lights.iter().find(|light| light.id == id)
    }

    pub(super) fn environment(&self) -> &SceneLighting {
        self.edits
            .lighting
            .as_ref()
            .unwrap_or(&self.lighting_defaults)
    }

    pub(super) fn lighting_value(&self, target: Selection, field: Field) -> Option<f32> {
        match (target, field) {
            (Selection::Light(id), Field::Light(field)) => {
                self.light(id).map(|light| light_number(light, field))
            }
            (Selection::Environment, Field::Environment(field)) => {
                Some(environment_number(self.environment(), field))
            }
            _ => None,
        }
    }

    pub(super) fn lighting_field_allowed(&self, target: Selection, field: Field) -> bool {
        if !self.has_scene() {
            return false;
        }
        match (target, field) {
            (Selection::Light(id), Field::Light(field)) => {
                self.light(id).is_some_and(|light| match field {
                    LightField::Range => light.kind != LightKind::Directional,
                    LightField::Radius => matches!(light.kind, LightKind::Point | LightKind::Spot),
                    LightField::Width | LightField::Height => light.kind == LightKind::Rect,
                    LightField::InnerAngle | LightField::OuterAngle => {
                        light.kind == LightKind::Spot
                    }
                    LightField::Temperature => light.temperature.is_some(),
                    LightField::ShadowNear => {
                        matches!(light.kind, LightKind::Point | LightKind::Spot)
                    }
                    LightField::ShadowDistance => light.kind == LightKind::Directional,
                    LightField::ShadowDepthBias | LightField::ShadowNormalBias => {
                        light.kind != LightKind::Rect
                    }
                    _ => true,
                })
            }
            (Selection::Environment, Field::Environment(field)) => {
                !matches!(field, EnvironmentField::FogStart | EnvironmentField::FogEnd)
                    || self.environment().fog.is_some()
            }
            _ => false,
        }
    }

    pub(super) fn validate_lighting_field(
        &self,
        field: Field,
        value: f32,
    ) -> Result<(), &'static str> {
        if !value.is_finite() {
            return Err("Введите конечное число");
        }
        let target = self
            .selection
            .ok_or("Выберите источник света или окружение")?;
        if !self.lighting_field_allowed(target, field) {
            return Err("Параметр недоступен для этого типа света");
        }
        match (target, field) {
            (Selection::Light(id), Field::Light(field)) => {
                let mut light = self
                    .light(id)
                    .ok_or("Источник света больше не существует")?
                    .clone();
                set_light_number(&mut light, field, value);
                light.validate().map_err(|_| match field {
                    LightField::Temperature => "Температура: от 1000 до 40000 K",
                    LightField::InnerAngle | LightField::OuterAngle => {
                        "Угол: 0–89°, внутренний не больше внешнего"
                    }
                    LightField::Range | LightField::ShadowNear => {
                        "Дальность должна быть больше ближней границы тени"
                    }
                    _ => "Значение света вне допустимого диапазона",
                })
            }
            (Selection::Environment, Field::Environment(field)) => {
                let mut environment = self.environment().clone();
                set_environment_number(&mut environment, field, value);
                environment.validate().map_err(|_| match field {
                    EnvironmentField::FogStart | EnvironmentField::FogEnd => {
                        "Конец тумана должен быть дальше начала"
                    }
                    _ => "Значение окружения вне допустимого диапазона",
                })
            }
            _ => Err("Параметр освещения не найден"),
        }
    }

    pub(super) fn set_lighting_field(&mut self, field: Field, value: f32) {
        if self.validate_lighting_field(field, value).is_err() {
            return;
        }
        match (self.selection, field) {
            (Some(Selection::Light(id)), Field::Light(field)) => {
                if let Some(light) = self.edits.lights.iter_mut().find(|light| light.id == id) {
                    set_light_number(light, field, value);
                }
            }
            (Some(Selection::Environment), Field::Environment(field)) => {
                let mut environment = self.environment().clone();
                set_environment_number(&mut environment, field, value);
                self.edits.lighting = Some(environment);
            }
            _ => {}
        }
    }
}

/// Root action dispatch owns the history snapshot, so each complete preset or
/// toggle produces one undo entry. Gizmo visibility is a UI preference only.
pub(super) fn handle(editor: &mut Editor, action: LightingAction) -> bool {
    if matches!(action, LightingAction::ToggleGizmos) {
        editor.show_lights = !editor.show_lights;
        return false;
    }
    if !editor.has_scene() || !editor.ready || editor.pending.is_some() {
        return false;
    }
    let before = editor.edits.clone();
    match action {
        LightingAction::Add(preset) => {
            let Some(id) = editor
                .edits
                .lights
                .iter()
                .map(|light| light.id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
            else {
                return false;
            };
            let mut light = preset.create(id);
            light.pose.position = (Vec3::from(light.pose.position) + editor.pivot).to_array();
            editor.edits.lights.push(light);
            editor.select(Some(Selection::Light(id)));
        }
        LightingAction::ApplyPreset(preset) => {
            let (mut settings, mut lights) = preset.create(1);
            settings.scene_lights = false;
            for light in &mut lights {
                light.pose.position = (Vec3::from(light.pose.position) + editor.pivot).to_array();
            }
            let selection = lights
                .first()
                .map(|light| Selection::Light(light.id))
                .unwrap_or(Selection::Environment);
            editor.edits.lights = lights;
            editor.edits.lighting = Some(settings);
            editor.select(Some(selection));
        }
        LightingAction::SetNumber(field, value) => editor.set_field(field, value),
        LightingAction::SetColor(color) if editor.selection == Some(Selection::Environment) => {
            let mut settings = editor.environment().clone();
            settings.ambient_color = color.map(|channel| channel as f32 / 255.0);
            editor.edits.lighting = Some(settings);
        }
        LightingAction::ResetEnvironment => editor.edits.lighting = None,
        LightingAction::ToggleImportedLights
        | LightingAction::ToggleFog
        | LightingAction::ToggleGlobalShadows
        | LightingAction::SetShadowResolution(_)
        | LightingAction::CycleToneMapping => {
            let mut settings = editor.environment().clone();
            match action {
                LightingAction::ToggleImportedLights => {
                    settings.scene_lights = !settings.scene_lights
                }
                LightingAction::ToggleGlobalShadows => settings.shadows = !settings.shadows,
                LightingAction::ToggleFog => {
                    settings.fog = if settings.fog.is_some() {
                        None
                    } else {
                        Some(fog_default())
                    }
                }
                LightingAction::SetShadowResolution(resolution) => {
                    settings.shadow_resolution = resolution
                }
                LightingAction::CycleToneMapping => {
                    let index = ToneMap::ALL
                        .iter()
                        .position(|tone| *tone == settings.tonemapping)
                        .unwrap_or(0);
                    settings.tonemapping = ToneMap::ALL[(index + 1) % ToneMap::ALL.len()];
                }
                _ => {}
            }
            if settings.validate().is_ok() {
                editor.edits.lighting = Some(settings);
            }
        }
        _ => {
            let Some(Selection::Light(id)) = editor.selection else {
                return false;
            };
            let view = editor.view;
            let Some(light) = editor.edits.lights.iter_mut().find(|light| light.id == id) else {
                return false;
            };
            match action {
                LightingAction::SetKind(kind) => {
                    light.kind = kind;
                    if matches!(kind, LightKind::Point | LightKind::Spot) {
                        light.radius = light.radius.min(light.range);
                        light.shadow_near = light.shadow_near.min(light.range * 0.5).max(0.001);
                    }
                    if kind == LightKind::Spot {
                        light.inner_angle = light.inner_angle.min(light.outer_angle);
                    }
                    if kind == LightKind::Directional {
                        light.shadow_distance = light.shadow_distance.max(light.shadow_near + 1.0);
                    }
                    if kind == LightKind::Rect {
                        light.shadows = false;
                        light.contact_shadows = false;
                    }
                }
                LightingAction::ToggleEnabled => light.enabled = !light.enabled,
                LightingAction::ToggleShadows if light.kind != LightKind::Rect => {
                    light.shadows = !light.shadows
                }
                LightingAction::ToggleContactShadows if light.kind != LightKind::Rect => {
                    light.contact_shadows = !light.contact_shadows
                }
                LightingAction::ToggleTemperature => {
                    light.temperature = if light.temperature.is_some() {
                        None
                    } else {
                        light.color = [1.0; 3];
                        Some(6500.0)
                    }
                }
                LightingAction::SetColor(color) => {
                    light.color = color.map(|channel| channel as f32 / 255.0);
                    light.temperature = None;
                }
                LightingAction::AimAtView => light.pose.rotation = view.rotation.to_array(),
                LightingAction::PlaceAtView => {
                    light.pose.position = view.translation.to_array();
                    light.pose.rotation = view.rotation.to_array();
                }
                _ => {}
            }
        }
    }
    editor.edits != before
}

#[derive(Component)]
pub(super) struct LightEntity(pub u32);

#[derive(Clone)]
struct CameraBaseline {
    ambient: Option<AmbientLight>,
    exposure: Option<Exposure>,
    tone: Option<Tonemapping>,
    fog: Option<DistanceFog>,
    contact: Option<ContactShadows>,
    hdr: bool,
    depth: bool,
}

impl CameraBaseline {
    fn capture(world: &World, entity: Entity) -> Self {
        Self {
            ambient: world.get::<AmbientLight>(entity).cloned(),
            exposure: world.get::<Exposure>(entity).copied(),
            tone: world.get::<Tonemapping>(entity).cloned(),
            fog: world.get::<DistanceFog>(entity).cloned(),
            contact: world.get::<ContactShadows>(entity).copied(),
            hdr: world.get::<bevy::camera::Hdr>(entity).is_some(),
            depth: world.get::<DepthPrepass>(entity).is_some(),
        }
    }

    fn settings(&self, world: &World, resolution: u32) -> SceneLighting {
        let mut settings = SceneLighting::default();
        let ambient = self.ambient.clone().unwrap_or_else(|| {
            world.get_resource::<GlobalAmbientLight>().map_or_else(
                AmbientLight::default,
                |ambient| AmbientLight {
                    color: ambient.color,
                    brightness: ambient.brightness,
                    affects_lightmapped_meshes: ambient.affects_lightmapped_meshes,
                },
            )
        });
        settings.ambient_color = rgb(ambient.color);
        settings.ambient_brightness = ambient.brightness.clamp(0.0, 100_000.0);
        settings.exposure_ev100 = self.exposure.unwrap_or_default().ev100.clamp(-10.0, 24.0);
        settings.tonemapping = self.tone.clone().unwrap_or_default().into();
        settings.shadow_resolution = resolution;
        settings.fog = self.fog.as_ref().and_then(|fog| match fog.falloff {
            FogFalloff::Linear { start, end } => Some(FogSettings {
                color: rgb(fog.color),
                start,
                end,
            }),
            _ => None,
        });
        settings
    }

    fn restore(&self, world: &mut World, entity: Entity) {
        restore_component(world, entity, &self.ambient);
        restore_component(world, entity, &self.exposure);
        restore_component(world, entity, &self.tone);
        restore_component(world, entity, &self.fog);
        restore_component(world, entity, &self.contact);
        if let Ok(mut entity) = world.get_entity_mut(entity) {
            if self.hdr {
                entity.insert(bevy::camera::Hdr);
            } else {
                entity.remove::<bevy::camera::Hdr>();
            }
            if self.depth {
                entity.insert(DepthPrepass);
            } else {
                entity.remove::<DepthPrepass>();
            }
        }
    }
}

fn rgb(color: Color) -> [f32; 3] {
    let color = color.to_srgba();
    [
        color.red.clamp(0.0, 1.0),
        color.green.clamp(0.0, 1.0),
        color.blue.clamp(0.0, 1.0),
    ]
}

fn restore_component<T: Component + Clone>(world: &mut World, entity: Entity, value: &Option<T>) {
    if let Ok(mut entity) = world.get_entity_mut(entity) {
        if let Some(value) = value {
            entity.insert(value.clone());
        } else {
            entity.remove::<T>();
        }
    }
}

#[derive(Clone)]
enum SourceLight {
    Point(PointLight),
    Spot(SpotLight),
    Directional(DirectionalLight),
    Rect(RectLight),
}

impl SourceLight {
    fn capture(world: &World, entity: Entity) -> Option<Self> {
        if let Some(light) = world.get::<PointLight>(entity) {
            Some(Self::Point(light.clone()))
        } else if let Some(light) = world.get::<SpotLight>(entity) {
            Some(Self::Spot(light.clone()))
        } else if let Some(light) = world.get::<DirectionalLight>(entity) {
            Some(Self::Directional(light.clone()))
        } else {
            world.get::<RectLight>(entity).copied().map(Self::Rect)
        }
    }

    fn apply(&self, world: &mut World, entity: Entity, environment: Option<&SceneLighting>) {
        if !world.entities().contains(entity) {
            return;
        }
        if let Some(environment) = environment {
            let (point, spot, directional, rect) = match self {
                Self::Point(light) => (Some(light), None, None, None),
                Self::Spot(light) => (None, Some(light), None, None),
                Self::Directional(light) => (None, None, Some(light), None),
                Self::Rect(light) => (None, None, None, Some(light)),
            };
            let mut queue = CommandQueue::default();
            crate::lighting::apply_imported_light(
                &mut Commands::new(&mut queue, world).entity(entity),
                environment,
                point,
                spot,
                directional,
                rect,
            );
            queue.apply(world);
            return;
        }
        let Ok(mut entity) = world.get_entity_mut(entity) else {
            return;
        };
        match self {
            Self::Point(original) => {
                entity.insert(original.clone());
            }
            Self::Spot(original) => {
                entity.insert(original.clone());
            }
            Self::Directional(original) => {
                entity.insert(original.clone());
            }
            Self::Rect(original) => {
                entity.insert(*original);
            }
        }
    }

    fn contact(&self) -> bool {
        match self {
            Self::Point(light) => light.contact_shadows_enabled,
            Self::Spot(light) => light.contact_shadows_enabled,
            Self::Directional(light) => light.contact_shadows_enabled,
            Self::Rect(_) => false,
        }
    }
}

#[derive(Resource, Default)]
struct Runtime {
    room: Option<usize>,
    custom: HashMap<u32, (Entity, SceneLight)>,
    imported: HashMap<Entity, SourceLight>,
    cameras: HashMap<Entity, CameraBaseline>,
    environment: Option<SceneLighting>,
    contact: bool,
    shadow_defaults: Option<(u32, u32)>,
}

fn belongs_to(world: &World, mut entity: Entity, roots: &HashSet<Entity>) -> bool {
    loop {
        if roots.contains(&entity) {
            return true;
        }
        let Some(parent) = world.get::<ChildOf>(entity) else {
            return false;
        };
        entity = parent.parent();
    }
}

fn restore_maps(world: &mut World, original: (u32, u32)) {
    if let Some(mut map) = world.get_resource_mut::<PointLightShadowMap>() {
        map.size = original.0 as usize;
    }
    if let Some(mut map) = world.get_resource_mut::<DirectionalLightShadowMap>() {
        map.size = original.1 as usize;
    }
}

fn clear_room(world: &mut World, runtime: &mut Runtime) {
    for (_, (entity, _)) in runtime.custom.drain() {
        let _ = world.despawn(entity);
    }
    for (entity, original) in runtime.imported.drain() {
        original.apply(world, entity, None);
    }
    for (entity, original) in runtime.cameras.drain() {
        original.restore(world, entity);
    }
    if let Some(original) = runtime.shadow_defaults {
        restore_maps(world, original);
    }
    runtime.environment = None;
    runtime.contact = false;
}

/// Runs in Update after camera creation and before transform propagation.
/// It never changes source meshes, thumbnail lights, mask views or saved DTOs.
pub(super) fn sync(world: &mut World) {
    let Some(editor) = world.get_resource::<Editor>() else {
        return;
    };
    let room = editor.has_scene().then_some(editor.room);
    let lights = if room.is_some() {
        editor.edits.lights.clone()
    } else {
        Vec::new()
    };
    let environment = room.and(editor.edits.lighting.clone());
    let mut runtime = world.remove_resource::<Runtime>().unwrap_or_default();
    let shadow_defaults = *runtime.shadow_defaults.get_or_insert_with(|| {
        (
            world
                .get_resource::<PointLightShadowMap>()
                .map_or(512, |map| map.size as u32),
            world
                .get_resource::<DirectionalLightShadowMap>()
                .map_or(2048, |map| map.size as u32),
        )
    });
    let changed_room = runtime.room != room;
    if changed_room {
        clear_room(world, &mut runtime);
        runtime.room = room;
    }
    if room.is_none() {
        world.insert_resource(runtime);
        return;
    }

    let environment_changed = runtime.environment != environment || changed_room;
    let cameras: Vec<_> = world
        .query_filtered::<Entity, Or<(With<EditorCamera>, With<ui::PreviewCamera>)>>()
        .iter(world)
        .collect();
    runtime
        .cameras
        .retain(|entity, _| world.entities().contains(*entity));
    let mut new_cameras = HashSet::new();
    for entity in cameras {
        if !runtime.cameras.contains_key(&entity) {
            let baseline = CameraBaseline::capture(world, entity);
            if world.get::<EditorCamera>(entity).is_some() {
                let defaults = baseline.settings(world, shadow_defaults.0);
                world.resource_mut::<Editor>().lighting_defaults = defaults;
            }
            runtime.cameras.insert(entity, baseline);
            new_cameras.insert(entity);
        }
    }

    let roots: HashSet<_> = world
        .get_resource::<Scene>()
        .map(|scene| {
            scene
                .root
                .into_iter()
                .chain(scene.models.values().map(|(entity, _)| *entity))
                .collect()
        })
        .unwrap_or_default();
    let sources: Vec<_> = world
        .query_filtered::<Entity, (
            Or<(
                With<PointLight>,
                With<SpotLight>,
                With<DirectionalLight>,
                With<RectLight>,
            )>,
            Without<LightEntity>,
        )>()
        .iter(world)
        .filter(|entity| belongs_to(world, *entity, &roots))
        .collect();
    runtime
        .imported
        .retain(|entity, _| world.entities().contains(*entity));
    for entity in sources {
        if let Some(original) = SourceLight::capture(world, entity)
            && !runtime.imported.contains_key(&entity)
        {
            if environment.is_some() {
                original.apply(world, entity, environment.as_ref());
            }
            runtime.imported.insert(entity, original);
        }
    }
    if environment_changed {
        for (entity, original) in &runtime.imported {
            original.apply(world, *entity, environment.as_ref());
        }
        if let Some(environment) = &environment {
            world.insert_resource(PointLightShadowMap {
                size: environment.shadow_resolution as usize,
            });
            world.insert_resource(DirectionalLightShadowMap {
                size: environment.shadow_resolution as usize,
            });
        } else {
            restore_maps(world, shadow_defaults);
        }
    }
    let ids: HashSet<_> = lights.iter().map(|light| light.id).collect();
    let stale: Vec<_> = runtime
        .custom
        .keys()
        .copied()
        .filter(|id| !ids.contains(id))
        .collect();
    for id in stale {
        if let Some((entity, _)) = runtime.custom.remove(&id) {
            let _ = world.despawn(entity);
        }
    }
    let mut queue = CommandQueue::default();
    {
        let mut commands = Commands::new(&mut queue, world);
        for light in &lights {
            let mut effective = light.clone();
            if environment
                .as_ref()
                .is_some_and(|environment| !environment.shadows)
            {
                effective.shadows = false;
                effective.contact_shadows = false;
            }
            if let Some((entity, previous)) = runtime.custom.get_mut(&light.id)
                && world
                    .get::<LightEntity>(*entity)
                    .is_some_and(|marker| marker.0 == light.id)
            {
                if *previous != *light || environment_changed {
                    crate::lighting::apply_light(&mut commands.entity(*entity), &effective);
                    *previous = light.clone();
                }
            } else {
                let entity = crate::lighting::spawn_light(&mut commands, &effective);
                commands
                    .entity(entity)
                    .insert((LightEntity(light.id), RenderLayers::layer(0)));
                runtime.custom.insert(light.id, (entity, light.clone()));
            }
        }
    }
    queue.apply(world);
    let contact = environment
        .as_ref()
        .is_none_or(|environment| environment.shadows)
        && (lights
            .iter()
            .any(|light| light.enabled && light.contact_shadows && light.kind != LightKind::Rect)
            || (environment
                .as_ref()
                .is_some_and(|environment| environment.scene_lights)
                && runtime.imported.values().any(SourceLight::contact)));
    let camera_changed = environment_changed || runtime.contact != contact;
    for (entity, baseline) in &runtime.cameras {
        if !camera_changed && !new_cameras.contains(entity) {
            continue;
        }
        if let Some(environment) = &environment {
            let mut queue = CommandQueue::default();
            crate::lighting::apply_camera_lighting(
                &mut Commands::new(&mut queue, world).entity(*entity),
                environment,
            );
            queue.apply(world);
        } else {
            baseline.restore(world, *entity);
        }
        let shadows = environment
            .as_ref()
            .is_none_or(|environment| environment.shadows);
        if shadows && (contact || baseline.contact.is_some()) {
            world
                .entity_mut(*entity)
                .insert(baseline.contact.unwrap_or_default());
        } else {
            world.entity_mut(*entity).remove::<ContactShadows>();
            if !baseline.depth {
                world.entity_mut(*entity).remove::<DepthPrepass>();
            }
        }
    }
    runtime.environment = environment;
    runtime.contact = contact;
    world.insert_resource(runtime);
}

#[cfg(test)]
#[path = "lighting_tests.rs"]
mod tests;
