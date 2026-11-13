//! Shared, asynchronously loaded HDRI maps for scene cameras and previews.
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use bevy::{
    asset::{AssetApp, LoadState, io::embedded::EmbeddedAssetRegistry},
    image::{ImageLoaderSettings, ImageSampler},
    light::{EnvironmentMapLight, Skybox},
    prelude::*,
};

use crate::{EnvironmentPreset, SceneLighting};

mod loader;
#[cfg(feature = "editor")]
pub(crate) use loader::validate_panorama;
use loader::{PanoramaLoader, PanoramaMaps};

/// Registers the bundled CC0 environments. Add after Bevy's `DefaultPlugins`
/// when using `apply_camera_lighting` in a game. The editor adds it automatically.
pub struct HdriPlugin;

impl Plugin for HdriPlugin {
    fn build(&self, app: &mut App) {
        let registry = app.world_mut().resource_mut::<EmbeddedAssetRegistry>();
        macro_rules! embed {
            ($name:literal) => {
                registry.insert_asset(
                    PathBuf::new(),
                    Path::new(concat!("bevy_editor/hdri/", $name, ".diffuse.ktx2")),
                    include_bytes!(concat!("../assets/hdri/", $name, ".diffuse.ktx2")),
                );
                registry.insert_asset(
                    PathBuf::new(),
                    Path::new(concat!("bevy_editor/hdri/", $name, ".specular.ktx2")),
                    include_bytes!(concat!("../assets/hdri/", $name, ".specular.ktx2")),
                );
            };
        }
        embed!("studio_small_09");
        embed!("brown_photostudio_02");
        embed!("christmas_photo_studio_07");
        embed!("empty_warehouse_01");
        embed!("lebombo");
        embed!("kloofendal_48d_partly_cloudy_puresky");
        embed!("venice_sunset");
        embed!("industrial_sunset_02_puresky");
        embed!("phalzer_forest_01");
        embed!("moonless_golf");
        app.init_asset::<PanoramaMaps>()
            .init_asset_loader::<PanoramaLoader>()
            .init_resource::<EnvironmentImages>()
            .init_resource::<HdriLoadStatus>()
            .add_systems(PostUpdate, synchronize);
    }
}

#[derive(Clone)]
enum EnvironmentSource {
    Builtin(EnvironmentPreset),
    Downloaded(String),
}

#[derive(Component, Clone)]
pub(crate) struct CameraEnvironment {
    source: EnvironmentSource,
    brightness: f32,
    rotation: f32,
    background: bool,
}

/// Optional preview backdrop, independent of the lighting and reflection maps.
#[derive(Component, Clone, Copy)]
pub(crate) struct EnvironmentBackdrop {
    pub brightness: f32,
    pub blur: f32,
}

#[derive(Resource, Default)]
struct EnvironmentImages {
    builtin: HashMap<EnvironmentPreset, (Handle<Image>, Handle<Image>)>,
    downloaded: HashMap<String, Handle<PanoramaMaps>>,
    backdrops: HashMap<Entity, (AssetId<Image>, u32, Handle<Image>)>,
}

#[derive(Resource, Default)]
pub(crate) struct HdriLoadStatus(pub HashMap<String, Result<bool, String>>);

pub(crate) fn apply_camera_environment(entity: &mut EntityCommands, settings: &SceneLighting) {
    let source = settings
        .hdri_asset
        .as_ref()
        .map(|asset| EnvironmentSource::Downloaded(asset.asset.clone()))
        .or_else(|| settings.hdri.map(EnvironmentSource::Builtin));
    if let Some(source) = source {
        entity.insert(CameraEnvironment {
            source,
            brightness: settings.hdri_brightness,
            rotation: settings.hdri_rotation,
            background: settings.hdri_background,
        });
        if !settings.hdri_background {
            entity.remove::<Skybox>();
        }
    } else {
        entity.remove::<(CameraEnvironment, EnvironmentMapLight, Skybox)>();
    }
}

fn synchronize(
    mut commands: Commands,
    cameras: Query<(
        Entity,
        &CameraEnvironment,
        Option<&EnvironmentMapLight>,
        Option<&Skybox>,
        Option<&EnvironmentBackdrop>,
    )>,
    server: Res<AssetServer>,
    panoramas: Res<Assets<PanoramaMaps>>,
    mut cache: ResMut<EnvironmentImages>,
    mut status: ResMut<HdriLoadStatus>,
    mut images: ResMut<Assets<Image>>,
) {
    let active: HashSet<_> = cameras
        .iter()
        .filter_map(|(_, settings, _, _, _)| match &settings.source {
            EnvironmentSource::Downloaded(path) => Some(path.as_str()),
            _ => None,
        })
        .collect();
    cache
        .downloaded
        .retain(|path, _| active.contains(path.as_str()));
    status.0.retain(|path, _| active.contains(path.as_str()));
    cache
        .backdrops
        .retain(|entity, _| cameras.contains(*entity));
    for (entity, settings, current, current_sky, backdrop) in &cameras {
        let (diffuse, specular, skybox) = match &settings.source {
            EnvironmentSource::Builtin(preset) => {
                let (diffuse, specular) = cache.builtin.entry(*preset).or_insert_with(|| {
                    let load = |kind| {
                        server
                            .load_builder()
                            .with_settings(|settings: &mut ImageLoaderSettings| {
                                settings.is_srgb = false;
                                settings.sampler = ImageSampler::linear();
                            })
                            .load(format!(
                                "embedded://bevy_editor/hdri/{}.{kind}.ktx2",
                                preset.asset_name()
                            ))
                    };
                    (load("diffuse"), load("specular"))
                });
                (diffuse.clone(), specular.clone(), specular.clone())
            }
            EnvironmentSource::Downloaded(path) => {
                let handle = cache
                    .downloaded
                    .entry(path.clone())
                    .or_insert_with(|| server.load(path.clone()));
                let Some(maps) = panoramas.get(handle.id()) else {
                    let state = match server.get_load_state(handle.id()) {
                        Some(LoadState::Failed(error)) => Err(error.to_string()),
                        _ => Ok(false),
                    };
                    status.0.insert(path.clone(), state);
                    continue;
                };
                status.0.insert(path.clone(), Ok(true));
                (
                    maps.diffuse.clone(),
                    maps.specular.clone(),
                    maps.skybox.clone(),
                )
            }
        };
        let skybox = if let Some(backdrop) =
            backdrop.filter(|backdrop| settings.background && backdrop.blur > 0.0)
        {
            if let Some(image) = images.get(&specular) {
                let level = (backdrop.blur.clamp(0.0, 1.0)
                    * image.texture_descriptor.mip_level_count.saturating_sub(1) as f32)
                    .round() as u32;
                if cache
                    .backdrops
                    .get(&entity)
                    .is_none_or(|(source, old_level, _)| {
                        *source != specular.id() || *old_level != level
                    })
                {
                    let mut blurred = image.clone();
                    let view = blurred.texture_view_descriptor.get_or_insert_default();
                    view.base_mip_level = level;
                    view.mip_level_count = Some(1);
                    let handle = images.add(blurred);
                    cache
                        .backdrops
                        .insert(entity, (specular.id(), level, handle));
                }
                cache.backdrops[&entity].2.clone()
            } else {
                skybox
            }
        } else {
            cache.backdrops.remove(&entity);
            skybox
        };
        let sky_brightness =
            settings.brightness * backdrop.map_or(1.0, |backdrop| backdrop.brightness);
        let rotation = Quat::from_rotation_y(settings.rotation.to_radians());
        let mut camera = commands.entity(entity);
        if current.is_none_or(|old| {
            old.diffuse_map != diffuse
                || old.specular_map != specular
                || old.intensity != settings.brightness
                || old.rotation != rotation
        }) {
            camera.insert(EnvironmentMapLight {
                diffuse_map: diffuse,
                specular_map: specular,
                intensity: settings.brightness,
                rotation,
                ..default()
            });
        }
        if settings.background {
            if current_sky.is_none_or(|old| {
                old.image.as_ref() != Some(&skybox)
                    || old.brightness != sky_brightness
                    || old.rotation != rotation
            }) {
                camera.insert(Skybox {
                    image: Some(skybox),
                    brightness: sky_brightness,
                    rotation,
                });
            }
        } else {
            camera.remove::<Skybox>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        asset::{AssetPlugin, RenderAssetUsages},
        image::{CompressedImageFormats, ImagePlugin, ImageType},
        render::render_resource::{TextureFormat, TextureViewDimension},
    };

    #[test]
    fn preview_backdrop_uses_a_blurred_view_without_changing_lighting_maps() {
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureViewDescriptor};
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            HdriPlugin,
        ));
        app.finish();
        app.cleanup();
        let mut image = Image::new_uninit(
            Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 6,
            },
            TextureDimension::D2,
            TextureFormat::Rgba16Float,
            RenderAssetUsages::all(),
        );
        image.texture_descriptor.mip_level_count = 4;
        image.data = Some(vec![0; (64 + 16 + 4 + 1) * 6 * 8]);
        image.texture_view_descriptor = Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::Cube),
            ..default()
        });
        let source = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        app.world_mut()
            .resource_mut::<EnvironmentImages>()
            .builtin
            .insert(
                EnvironmentPreset::SmallStudio,
                (source.clone(), source.clone()),
            );
        let camera = app
            .world_mut()
            .spawn((
                CameraEnvironment {
                    source: EnvironmentSource::Builtin(EnvironmentPreset::SmallStudio),
                    brightness: 1000.0,
                    rotation: 30.0,
                    background: true,
                },
                EnvironmentBackdrop {
                    brightness: 0.55,
                    blur: 0.6,
                },
            ))
            .id();
        app.update();
        let lighting = app.world().get::<EnvironmentMapLight>(camera).unwrap();
        assert_eq!(lighting.specular_map, source);
        assert_eq!(lighting.intensity, 1000.0);
        let sky = app.world().get::<Skybox>(camera).unwrap();
        assert_eq!(sky.brightness, 550.0);
        let images = app.world().resource::<Assets<Image>>();
        let original = images
            .get(&source)
            .unwrap()
            .texture_view_descriptor
            .as_ref()
            .unwrap();
        assert_eq!(original.base_mip_level, 0);
        let blurred = images
            .get(sky.image.as_ref().unwrap())
            .unwrap()
            .texture_view_descriptor
            .as_ref()
            .unwrap();
        assert_eq!(blurred.base_mip_level, 2);
        assert_eq!(blurred.mip_level_count, Some(1));
        app.world_mut()
            .get_mut::<CameraEnvironment>(camera)
            .unwrap()
            .background = false;
        app.update();
        assert!(app.world().get::<Skybox>(camera).is_none());
        assert_eq!(
            app.world()
                .get::<EnvironmentMapLight>(camera)
                .unwrap()
                .specular_map,
            source
        );
        assert!(
            !app.world()
                .resource::<EnvironmentImages>()
                .backdrops
                .contains_key(&camera)
        );
    }

    #[test]
    fn all_bundled_environments_decode_to_hdr_cubemaps_with_roughness_mips() {
        for preset in EnvironmentPreset::ALL {
            for (kind, resolution, levels) in [("diffuse", 32, 1), ("specular", 256, 9)] {
                let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets/hdri")
                    .join(format!("{}.{kind}.ktx2", preset.asset_name()));
                let bytes = std::fs::read(&path).unwrap();
                let image = Image::from_buffer(
                    &bytes,
                    ImageType::Extension("ktx2"),
                    CompressedImageFormats::NONE,
                    false,
                    ImageSampler::linear(),
                    RenderAssetUsages::all(),
                )
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba16Float);
                assert_eq!(image.texture_descriptor.size.width, resolution);
                assert_eq!(image.texture_descriptor.size.height, resolution);
                assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 6);
                assert_eq!(image.texture_descriptor.mip_level_count, levels);
                assert_eq!(
                    image.texture_view_descriptor.as_ref().unwrap().dimension,
                    Some(TextureViewDimension::Cube)
                );
                let data = image.data.unwrap();
                assert!(
                    data.chunks_exact(2)
                        .all(|v| u16::from_le_bytes([v[0], v[1]]) & 0x7c00 != 0x7c00),
                    "HDRI contains a nonfinite texel: {}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn cameras_share_map_handles_but_keep_independent_rotation_and_brightness() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            HdriPlugin,
        ));
        app.finish();
        app.cleanup();
        let first = app
            .world_mut()
            .spawn(CameraEnvironment {
                source: EnvironmentSource::Builtin(EnvironmentPreset::SmallStudio),
                brightness: 1000.0,
                rotation: 90.0,
                background: true,
            })
            .id();
        let second = app
            .world_mut()
            .spawn(CameraEnvironment {
                source: EnvironmentSource::Builtin(EnvironmentPreset::SmallStudio),
                brightness: 250.0,
                rotation: -30.0,
                background: false,
            })
            .id();
        app.update();
        let a = app.world().get::<EnvironmentMapLight>(first).unwrap();
        let b = app.world().get::<EnvironmentMapLight>(second).unwrap();
        assert_eq!(a.diffuse_map, b.diffuse_map);
        assert_eq!(a.specular_map, b.specular_map);
        assert_eq!(a.intensity, 1000.0);
        assert_eq!(b.intensity, 250.0);
        assert!(
            a.rotation
                .abs_diff_eq(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), 1e-6)
        );
        assert!(!a.rotation.abs_diff_eq(b.rotation, 0.1));
        let skybox = app.world().get::<Skybox>(first).unwrap();
        assert_eq!(skybox.image.as_ref(), Some(&a.specular_map));
        assert_eq!(skybox.brightness, a.intensity);
        assert_eq!(skybox.rotation, a.rotation);
        assert!(app.world().get::<Skybox>(second).is_none());
        let original_specular = a.specular_map.clone();
        app.world_mut()
            .get_mut::<CameraEnvironment>(first)
            .unwrap()
            .background = false;
        app.update();
        assert!(app.world().get::<Skybox>(first).is_none());
        assert_eq!(
            app.world()
                .get::<EnvironmentMapLight>(first)
                .unwrap()
                .specular_map,
            original_specular
        );
    }
}
