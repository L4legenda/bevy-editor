//! Serializable scene lighting. DTOs and validation do not require a renderer.
//! Intensities are lumens for local lights and lux for directional lights.
use std::io;

use serde::{Deserialize, Serialize};

use crate::Pose;

#[cfg(test)]
#[path = "lighting_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    #[default]
    Point,
    Spot,
    Directional,
    Rect,
}

impl LightKind {
    pub const ALL: [Self; 4] = [Self::Point, Self::Spot, Self::Directional, Self::Rect];

    pub fn label(self) -> &'static str {
        match self {
            Self::Point => "Точечный свет",
            Self::Spot => "Прожектор",
            Self::Directional => "Солнце",
            Self::Rect => "Световая панель",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SceneLight {
    pub id: u32,
    pub name: String,
    pub kind: LightKind,
    pub enabled: bool,
    pub pose: Pose,
    /// Artist tint in sRGB, multiplied by the optional temperature tint.
    pub color: [f32; 3],
    pub temperature: Option<f32>,
    pub intensity: f32,
    pub range: f32,
    pub radius: f32,
    pub width: f32,
    pub height: f32,
    /// Spot-cone half angles, in degrees.
    pub inner_angle: f32,
    pub outer_angle: f32,
    pub shadows: bool,
    pub contact_shadows: bool,
    pub shadow_depth_bias: f32,
    pub shadow_normal_bias: f32,
    pub shadow_near: f32,
    /// Directional cascade coverage, in world metres.
    pub shadow_distance: f32,
}

impl Default for SceneLight {
    fn default() -> Self {
        Self {
            id: 0,
            name: LightKind::Point.label().into(),
            kind: LightKind::Point,
            enabled: true,
            pose: pose([0.0, 2.0, 0.0], None),
            color: [1.0; 3],
            temperature: None,
            intensity: 800.0,
            range: 20.0,
            radius: 0.1,
            width: 1.0,
            height: 1.0,
            inner_angle: 20.0,
            outer_angle: 35.0,
            shadows: true,
            contact_shadows: true,
            shadow_depth_bias: 0.08,
            shadow_normal_bias: 0.6,
            shadow_near: 0.1,
            shadow_distance: 100.0,
        }
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn bounded(value: f32, minimum: f32, maximum: f32) -> bool {
    value.is_finite() && (minimum..=maximum).contains(&value)
}

fn rgb(color: [f32; 3]) -> bool {
    color.into_iter().all(|channel| bounded(channel, 0.0, 1.0))
}

impl SceneLight {
    pub fn validate(&self) -> io::Result<()> {
        if self.name.trim().is_empty()
            || self.name.chars().count() > 128
            || self.name.chars().any(char::is_control)
        {
            return Err(invalid("Light names must contain 1–128 visible characters"));
        }
        if !self.pose.valid()
            || !rgb(self.color)
            || self
                .temperature
                .is_some_and(|value| !bounded(value, 1000.0, 40000.0))
            || !bounded(self.intensity, 0.0, 1.0e9)
            || !bounded(self.range, 0.01, 10000.0)
            || !bounded(self.radius, 0.0, 1000.0)
            || (matches!(self.kind, LightKind::Point | LightKind::Spot) && self.radius > self.range)
            || !bounded(self.width, 0.01, 100.0)
            || !bounded(self.height, 0.01, 100.0)
            || !bounded(self.inner_angle, 0.0, 89.9)
            || !bounded(self.outer_angle, 0.1, 89.9)
            || (self.kind == LightKind::Spot && self.inner_angle > self.outer_angle)
            || !bounded(self.shadow_depth_bias, 0.0, 10.0)
            || !bounded(self.shadow_normal_bias, 0.0, 100.0)
            || !bounded(self.shadow_near, 0.001, 1000.0)
            || !bounded(self.shadow_distance, 0.01, 10000.0)
            || (self.kind == LightKind::Directional && self.shadow_near >= self.shadow_distance)
            || (matches!(self.kind, LightKind::Point | LightKind::Spot)
                && self.shadow_near >= self.range)
        {
            return Err(invalid(format!(
                "Invalid lighting parameters for light #{}",
                self.id
            )));
        }
        Ok(())
    }

    /// Combined artist and temperature tint, returned in sRGB for UI swatches
    /// and light components. Multiplication happens in linear light.
    pub fn resolved_rgb(&self) -> [f32; 3] {
        let Some(temperature) = self.temperature else {
            return self.color.map(|channel| {
                if channel.is_finite() {
                    channel.clamp(0.0, 1.0)
                } else {
                    1.0
                }
            });
        };
        let tint = temperature_rgb(temperature);
        std::array::from_fn(|index| {
            let base = if self.color[index].is_finite() {
                self.color[index].clamp(0.0, 1.0)
            } else {
                1.0
            };
            let linear = |value: f32| {
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            let value = linear(base) * linear(tint[index]);
            if value <= 0.0031308 {
                value * 12.92
            } else {
                1.055 * value.powf(1.0 / 2.4) - 0.055
            }
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToneMap {
    None,
    Linear,
    Reinhard,
    ReinhardLuminance,
    AcesFitted,
    #[default]
    AgX,
    SomewhatBoringDisplayTransform,
    TonyMcMapface,
    BlenderFilmic,
    KhronosPbrNeutral,
}

impl ToneMap {
    pub const ALL: [Self; 10] = [
        Self::None,
        Self::Linear,
        Self::Reinhard,
        Self::ReinhardLuminance,
        Self::AcesFitted,
        Self::AgX,
        Self::SomewhatBoringDisplayTransform,
        Self::TonyMcMapface,
        Self::BlenderFilmic,
        Self::KhronosPbrNeutral,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Без обработки",
            Self::Linear => "Линейный",
            Self::Reinhard => "Reinhard",
            Self::ReinhardLuminance => "Reinhard Luminance",
            Self::AcesFitted => "ACES",
            Self::AgX => "AgX",
            Self::SomewhatBoringDisplayTransform => "Somewhat Boring",
            Self::TonyMcMapface => "Tony McMapface",
            Self::BlenderFilmic => "Blender Filmic",
            Self::KhronosPbrNeutral => "Khronos PBR Neutral",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FogSettings {
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
}

impl Default for FogSettings {
    fn default() -> Self {
        Self {
            color: [0.5, 0.6, 0.7],
            start: 10.0,
            end: 100.0,
        }
    }
}

impl FogSettings {
    pub fn validate(&self) -> io::Result<()> {
        if !rgb(self.color)
            || !bounded(self.start, 0.0, 100000.0)
            || !bounded(self.end, 0.01, 1.0e6)
            || self.end <= self.start
        {
            return Err(invalid("Fog requires a valid color and finite start < end"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SceneLighting {
    pub ambient_color: [f32; 3],
    pub ambient_brightness: f32,
    pub exposure_ev100: f32,
    /// Preserve or suppress host/imported lights; custom lights are separate.
    pub scene_lights: bool,
    pub shadows: bool,
    pub shadow_resolution: u32,
    pub tonemapping: ToneMap,
    pub fog: Option<FogSettings>,
}

impl Default for SceneLighting {
    fn default() -> Self {
        Self {
            ambient_color: [1.0; 3],
            ambient_brightness: 80.0,
            exposure_ev100: 9.7,
            scene_lights: true,
            shadows: true,
            shadow_resolution: 1024,
            tonemapping: ToneMap::AgX,
            fog: None,
        }
    }
}

impl SceneLighting {
    pub fn validate(&self) -> io::Result<()> {
        if !rgb(self.ambient_color)
            || !bounded(self.ambient_brightness, 0.0, 100000.0)
            || !bounded(self.exposure_ev100, -10.0, 24.0)
            || !(128..=8192).contains(&self.shadow_resolution)
            || !self.shadow_resolution.is_power_of_two()
        {
            return Err(invalid("Invalid scene lighting or shadow-map resolution"));
        }
        if let Some(fog) = &self.fog {
            fog.validate()?;
        }
        Ok(())
    }
}

/// Approximate display-white tint for 1000–40000 K; values are sRGB.
/// Invalid inputs produce neutral white rather than propagating NaNs.
pub fn temperature_rgb(kelvin: f32) -> [f32; 3] {
    if !kelvin.is_finite() {
        return [1.0; 3];
    }
    let temperature = kelvin.clamp(1000.0, 40000.0) / 100.0;
    let red = if temperature <= 66.0 {
        255.0
    } else {
        329.69873 * (temperature - 60.0).powf(-0.13320476)
    };
    let green = if temperature <= 66.0 {
        99.4708 * temperature.ln() - 161.11957
    } else {
        288.12216 * (temperature - 60.0).powf(-0.07551485)
    };
    let blue = if temperature >= 66.0 {
        255.0
    } else if temperature <= 19.0 {
        0.0
    } else {
        138.51773 * (temperature - 10.0).ln() - 305.0448
    };
    [red, green, blue].map(|channel| (channel / 255.0).clamp(0.0, 1.0))
}

fn pose(position: [f32; 3], target: Option<[f32; 3]>) -> Pose {
    use bevy::prelude::{Transform, Vec3};
    let mut pose = Transform::from_translation(Vec3::from(position));
    if let Some(target) = target {
        pose = pose.looking_at(Vec3::from(target), Vec3::Y);
    }
    pose.into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightPreset {
    Bulb,
    Candle,
    Softbox,
    Neon,
    Spot,
    Sun,
    Moon,
}

impl LightPreset {
    pub const ALL: [Self; 7] = [
        Self::Bulb,
        Self::Candle,
        Self::Softbox,
        Self::Neon,
        Self::Spot,
        Self::Sun,
        Self::Moon,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Bulb => "Лампочка",
            Self::Candle => "Свеча",
            Self::Softbox => "Софтбокс",
            Self::Neon => "Неон",
            Self::Spot => "Прожектор",
            Self::Sun => "Солнце",
            Self::Moon => "Луна",
        }
    }

    pub fn create(self, id: u32) -> SceneLight {
        let mut light = SceneLight {
            id,
            name: self.label().into(),
            ..Default::default()
        };
        match self {
            Self::Bulb => {
                light.temperature = Some(2700.0);
                light.range = 12.0;
                light.radius = 0.05;
            }
            Self::Candle => {
                light.temperature = Some(1850.0);
                light.intensity = 12.6;
                light.range = 4.0;
                light.radius = 0.02;
                light.shadow_near = 0.02;
                light.pose = pose([0.0, 1.0, 0.0], None);
            }
            Self::Softbox => {
                light.kind = LightKind::Rect;
                light.temperature = Some(5600.0);
                light.intensity = 4000.0;
                light.width = 2.0;
                light.height = 1.5;
                light.range = 15.0;
                light.shadows = false;
                light.contact_shadows = false;
                light.pose = pose([3.0, 4.0, 3.0], Some([0.0; 3]));
            }
            Self::Neon => {
                light.kind = LightKind::Rect;
                light.color = [0.35, 0.65, 1.0];
                light.intensity = 1200.0;
                light.width = 2.0;
                light.height = 0.08;
                light.range = 8.0;
                light.shadows = false;
                light.contact_shadows = false;
                light.pose = pose([0.0, 2.0, 2.0], Some([0.0, 1.0, 0.0]));
            }
            Self::Spot => {
                light.kind = LightKind::Spot;
                light.temperature = Some(3200.0);
                light.intensity = 2000.0;
                light.inner_angle = 20.0;
                light.outer_angle = 32.0;
                light.range = 18.0;
                light.shadow_depth_bias = 0.02;
                light.shadow_normal_bias = 1.8;
                light.pose = pose([2.0, 4.0, 2.0], Some([0.0; 3]));
            }
            Self::Sun | Self::Moon => {
                light.kind = LightKind::Directional;
                light.range = 100.0;
                light.intensity = if self == Self::Sun { 100000.0 } else { 0.25 };
                light.temperature = Some(if self == Self::Sun { 6500.0 } else { 9000.0 });
                light.shadow_depth_bias = 0.02;
                light.shadow_normal_bias = 1.8;
                light.shadow_distance = 120.0;
                light.pose = pose([8.0, 12.0, 6.0], Some([0.0; 3]));
            }
        }
        light
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightingPreset {
    Daylight,
    WarmInterior,
    Night,
    Studio,
}

impl LightingPreset {
    pub const ALL: [Self; 4] = [
        Self::Daylight,
        Self::WarmInterior,
        Self::Night,
        Self::Studio,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Daylight => "Дневной свет",
            Self::WarmInterior => "Тёплый интерьер",
            Self::Night => "Ночь",
            Self::Studio => "Студия",
        }
    }

    pub fn settings(self) -> SceneLighting {
        let mut settings = SceneLighting {
            scene_lights: false,
            ..Default::default()
        };
        match self {
            Self::Daylight => {
                settings.ambient_color = [0.8, 0.9, 1.0];
                settings.ambient_brightness = 2000.0;
                settings.exposure_ev100 = 15.0;
                settings.shadow_resolution = 2048;
            }
            Self::WarmInterior => {
                settings.ambient_color = [1.0, 0.84, 0.65];
                settings.ambient_brightness = 20.0;
                settings.exposure_ev100 = 7.0;
            }
            Self::Night => {
                settings.ambient_color = [0.4, 0.55, 1.0];
                settings.ambient_brightness = 0.05;
                settings.exposure_ev100 = -1.0;
            }
            Self::Studio => {
                settings.ambient_brightness = 100.0;
                settings.exposure_ev100 = 9.7;
                settings.shadow_resolution = 2048;
            }
        }
        settings
    }

    pub fn lights(self, first_id: u32) -> Vec<SceneLight> {
        let mut lights = match self {
            Self::Daylight => vec![LightPreset::Sun.create(first_id)],
            Self::WarmInterior => {
                let mut fill = LightPreset::Softbox.create(0);
                fill.intensity = 800.0;
                vec![LightPreset::Bulb.create(first_id), fill]
            }
            Self::Night => vec![
                LightPreset::Moon.create(first_id),
                LightPreset::Candle.create(0),
            ],
            Self::Studio => {
                let mut key = LightPreset::Softbox.create(first_id);
                key.intensity = 10000.0;
                let mut fill = LightPreset::Softbox.create(0);
                fill.name = "Заполняющий свет".into();
                fill.intensity = 4000.0;
                fill.pose = pose([-3.0, 3.0, 2.0], Some([0.0; 3]));
                let mut rim = LightPreset::Spot.create(0);
                rim.name = "Контровой свет".into();
                rim.intensity = 3000.0;
                rim.pose = pose([0.0, 4.0, -3.0], Some([0.0, 1.0, 0.0]));
                vec![key, fill, rim]
            }
        };
        for (offset, light) in lights.iter_mut().enumerate() {
            light.id = first_id.wrapping_add(offset as u32);
        }
        lights
    }

    pub fn create(self, first_id: u32) -> (SceneLighting, Vec<SceneLight>) {
        (self.settings(), self.lights(first_id))
    }
}

#[cfg(feature = "lighting")]
mod rendering {
    use super::*;
    use bevy::{
        camera::{Exposure, Hdr},
        light::{CascadeShadowConfig, CascadeShadowConfigBuilder, RectLight},
        pbr::{ContactShadows, DistanceFog, FogFalloff},
        prelude::*,
        render::view::Tonemapping,
    };

    impl From<ToneMap> for Tonemapping {
        fn from(value: ToneMap) -> Self {
            match value {
                ToneMap::None => Self::None,
                ToneMap::Linear => Self::Linear,
                ToneMap::Reinhard => Self::Reinhard,
                ToneMap::ReinhardLuminance => Self::ReinhardLuminance,
                ToneMap::AcesFitted => Self::AcesFitted,
                ToneMap::AgX => Self::AgX,
                ToneMap::SomewhatBoringDisplayTransform => Self::SomewhatBoringDisplayTransform,
                ToneMap::TonyMcMapface => Self::TonyMcMapface,
                ToneMap::BlenderFilmic => Self::BlenderFilmic,
                ToneMap::KhronosPbrNeutral => Self::KhronosPbrNeutral,
            }
        }
    }

    impl From<Tonemapping> for ToneMap {
        fn from(value: Tonemapping) -> Self {
            match value {
                Tonemapping::None => Self::None,
                Tonemapping::Linear => Self::Linear,
                Tonemapping::Reinhard => Self::Reinhard,
                Tonemapping::ReinhardLuminance => Self::ReinhardLuminance,
                Tonemapping::AcesFitted => Self::AcesFitted,
                Tonemapping::AgX => Self::AgX,
                Tonemapping::SomewhatBoringDisplayTransform => Self::SomewhatBoringDisplayTransform,
                Tonemapping::TonyMcMapface => Self::TonyMcMapface,
                Tonemapping::BlenderFilmic => Self::BlenderFilmic,
                Tonemapping::KhronosPbrNeutral => Self::KhronosPbrNeutral,
            }
        }
    }

    /// Spawn a custom light. Callers choose parent entities and render layers.
    pub fn spawn_light(commands: &mut Commands, light: &SceneLight) -> Entity {
        let mut entity = commands.spawn_empty();
        let id = entity.id();
        apply_light(&mut entity, light);
        id
    }

    /// Replace the previous light kind, preserving host-owned render layers.
    /// Invalid DTOs are disabled instead of sending NaNs into the renderer.
    pub fn apply_light(entity: &mut EntityCommands, light: &SceneLight) {
        entity.remove::<(
            PointLight,
            SpotLight,
            DirectionalLight,
            RectLight,
            CascadeShadowConfig,
        )>();
        if light.validate().is_err() {
            entity.insert(Visibility::Hidden);
            return;
        }
        let [red, green, blue] = light.resolved_rgb();
        let color = Color::srgb(red, green, blue);
        entity.insert((
            Name::new(light.name.clone()),
            light.pose.transform(),
            if light.enabled {
                Visibility::Visible
            } else {
                Visibility::Hidden
            },
        ));
        match light.kind {
            LightKind::Point => {
                entity.insert(PointLight {
                    color,
                    intensity: light.intensity,
                    range: light.range,
                    radius: light.radius,
                    shadow_maps_enabled: light.shadows,
                    contact_shadows_enabled: light.contact_shadows,
                    shadow_depth_bias: light.shadow_depth_bias,
                    shadow_normal_bias: light.shadow_normal_bias,
                    shadow_map_near_z: light.shadow_near,
                    ..default()
                });
            }
            LightKind::Spot => {
                entity.insert(SpotLight {
                    color,
                    intensity: light.intensity,
                    range: light.range,
                    radius: light.radius,
                    inner_angle: light.inner_angle.to_radians(),
                    outer_angle: light.outer_angle.to_radians(),
                    shadow_maps_enabled: light.shadows,
                    contact_shadows_enabled: light.contact_shadows,
                    shadow_depth_bias: light.shadow_depth_bias,
                    shadow_normal_bias: light.shadow_normal_bias,
                    shadow_map_near_z: light.shadow_near,
                    ..default()
                });
            }
            LightKind::Directional => {
                let first = light.shadow_near + (light.shadow_distance - light.shadow_near) * 0.1;
                let cascades = if first > light.shadow_near && first < light.shadow_distance {
                    4
                } else {
                    1
                };
                entity.insert((
                    DirectionalLight {
                        color,
                        illuminance: light.intensity,
                        shadow_maps_enabled: light.shadows,
                        contact_shadows_enabled: light.contact_shadows,
                        shadow_depth_bias: light.shadow_depth_bias,
                        shadow_normal_bias: light.shadow_normal_bias,
                        ..default()
                    },
                    CascadeShadowConfigBuilder {
                        num_cascades: cascades,
                        minimum_distance: light.shadow_near,
                        maximum_distance: light.shadow_distance,
                        first_cascade_far_bound: if cascades == 1 {
                            light.shadow_distance
                        } else {
                            first
                        },
                        ..default()
                    }
                    .build(),
                ));
            }
            LightKind::Rect => {
                entity.insert(RectLight {
                    color,
                    intensity: light.intensity,
                    range: light.range,
                    width: light.width,
                    height: light.height,
                });
            }
        }
    }

    /// Apply one environment override. `LevelEdits::lighting == None` means
    /// callers leave their original camera components untouched.
    pub fn apply_camera_lighting(entity: &mut EntityCommands, settings: &SceneLighting) {
        if settings.validate().is_err() {
            return;
        }
        let [red, green, blue] = settings.ambient_color;
        entity.insert((
            AmbientLight {
                color: Color::srgb(red, green, blue),
                brightness: settings.ambient_brightness,
                ..default()
            },
            Exposure {
                ev100: settings.exposure_ev100,
            },
            Tonemapping::from(settings.tonemapping),
            Hdr,
        ));
        if let Some(fog) = &settings.fog {
            let [red, green, blue] = fog.color;
            entity.insert(DistanceFog {
                color: Color::srgb(red, green, blue),
                directional_light_color: Color::NONE,
                falloff: FogFalloff::Linear {
                    start: fog.start,
                    end: fog.end,
                },
                ..default()
            });
        } else {
            entity.remove::<DistanceFog>();
        }
        if settings.shadows {
            entity.insert(ContactShadows::default());
        } else {
            entity.remove::<ContactShadows>();
        }
    }

    /// Apply environment switches to snapshots of imported/host-owned lights.
    /// Callers keep the originals when later restoring or undoing the override.
    pub fn apply_imported_light(
        entity: &mut EntityCommands,
        settings: &SceneLighting,
        point: Option<&PointLight>,
        spot: Option<&SpotLight>,
        directional: Option<&DirectionalLight>,
        rect: Option<&RectLight>,
    ) {
        if settings.validate().is_err() {
            return;
        }
        let shadows = settings.scene_lights && settings.shadows;
        if let Some(original) = point {
            let mut light = original.clone();
            if !settings.scene_lights {
                light.intensity = 0.0;
            }
            light.shadow_maps_enabled &= shadows;
            light.contact_shadows_enabled &= shadows;
            entity.insert(light);
        }
        if let Some(original) = spot {
            let mut light = original.clone();
            if !settings.scene_lights {
                light.intensity = 0.0;
            }
            light.shadow_maps_enabled &= shadows;
            light.contact_shadows_enabled &= shadows;
            entity.insert(light);
        }
        if let Some(original) = directional {
            let mut light = original.clone();
            if !settings.scene_lights {
                light.illuminance = 0.0;
            }
            light.shadow_maps_enabled &= shadows;
            light.contact_shadows_enabled &= shadows;
            entity.insert(light);
        }
        if let Some(original) = rect {
            let mut light = *original;
            if !settings.scene_lights {
                light.intensity = 0.0;
            }
            entity.insert(light);
        }
    }
}

#[cfg(feature = "lighting")]
pub use rendering::{apply_camera_lighting, apply_imported_light, apply_light, spawn_light};
