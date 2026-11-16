//! A small standalone Bevy host for authored levels. Games can reuse the plugin
//! and add their own gameplay systems instead of depending on the editor UI.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

use bevy::{
    camera::ScalingMode,
    gltf::{Gltf, GltfLoaderSettings},
    input::mouse::AccumulatedMouseMotion,
    light::{DirectionalLightShadowMap, PointLightShadowMap, RectLight},
    prelude::*,
    transform::helper::TransformHelper,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
    world_serialization::WorldInstanceReady,
};
use serde::{Deserialize, Serialize};

use crate::{
    LevelEdits, SceneLighting,
    data::{CameraDefinition, LensKind},
};

/// An immutable copy of an open level, including edits not yet saved to disk.
#[derive(Clone, Resource, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameLevel {
    pub title: String,
    pub asset: String,
    pub groups: Vec<String>,
    pub excluded_nodes: Vec<String>,
    pub cameras: Vec<CameraDefinition>,
    pub camera: String,
    pub edits: LevelEdits,
    pub environment: SceneLighting,
}

impl GameLevel {
    /// Read a native `.scene` file without loading an editor project manifest.
    pub fn read(root: impl AsRef<Path>, file: &str) -> io::Result<Self> {
        let root = root.as_ref();
        let edits = LevelEdits::read(root, file)?;
        let settings = edits
            .scene
            .as_ref()
            .ok_or_else(|| io::Error::other("Native scene metadata is missing"))?;
        let level = Self {
            title: settings.label.clone(),
            asset: String::new(),
            groups: vec![],
            excluded_nodes: vec![],
            camera: settings
                .cameras
                .first()
                .map(|camera| camera.id.clone())
                .unwrap_or_default(),
            cameras: settings.cameras.clone(),
            environment: edits.lighting.clone().unwrap_or_default(),
            edits,
        };
        level.validate(root)?;
        Ok(level)
    }

    pub fn validate(&self, root: &Path) -> io::Result<()> {
        self.edits.validate(root)?;
        self.environment.validate()?;
        if !self.asset.is_empty() {
            if !crate::data::relative_path(&self.asset, "glb") {
                return Err(io::Error::other("Invalid level asset"));
            }
            let asset = crate::asset_paths::resolve_asset_path(root, &self.asset)?;
            if !root.join(asset).is_file() {
                return Err(io::Error::other("Level asset does not exist"));
            }
        }
        let mut ids = std::collections::HashSet::new();
        if self.cameras.is_empty()
            || self.cameras.iter().any(|camera| {
                !ids.insert(&camera.id)
                    || !camera.placement.pose.valid()
                    || !camera.projection.valid_lens(camera.placement.lens)
            })
            || !self.cameras.iter().any(|camera| camera.id == self.camera)
            || self.edits.cameras.iter().any(|(id, placement)| {
                self.cameras
                    .iter()
                    .find(|camera| camera.id == *id)
                    .is_none_or(|camera| !camera.projection.valid_lens(placement.lens))
            })
        {
            return Err(io::Error::other("Invalid game camera"));
        }
        Ok(())
    }
}

/// Portable launch data shared by the editor, CLI and a custom game host.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameLaunch {
    pub asset_root: PathBuf,
    pub level: GameLevel,
}

impl GameLaunch {
    pub fn read(path: impl AsRef<Path>) -> io::Result<Self> {
        let launch: Self = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        launch.level.validate(&launch.asset_root)?;
        Ok(launch)
    }
}

#[derive(Component)]
pub struct GameCamera;

/// Stable placement ID for attaching gameplay to an authored model instance.
#[derive(Component)]
pub struct GameModel(pub u32);

#[derive(Resource, Default)]
pub struct GameLevelStatus {
    pub ready: bool,
    pub error: Option<String>,
    pending: Option<Handle<Gltf>>,
    models: Vec<Handle<WorldAsset>>,
    source_ready: bool,
    pending_instances: usize,
}

/// Adds the level's models, authored camera, lighting and imported node edits.
/// Add after DefaultPlugins and AssetRedirectPlugin. It adds no editor UI,
/// camera controls, collision or game rules.
pub struct GameLevelPlugin(pub GameLevel);

impl Plugin for GameLevelPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::HdriPlugin>() {
            app.add_plugins(crate::HdriPlugin);
        }
        app.insert_resource(self.0.clone())
            .init_resource::<GameLevelStatus>()
            .add_systems(Startup, setup_level)
            .add_systems(Update, load_level);
    }
}

/// The default game foundation: native window, authored level and a fly camera.
/// RMB + mouse looks around; WASD/QE moves; Shift accelerates; Esc closes.
pub fn create_game_app(root: impl AsRef<Path>, level: GameLevel) -> io::Result<App> {
    let root = root.as_ref().canonicalize()?;
    level.validate(&root)?;
    let mut app = App::new();
    app.add_plugins(crate::AssetRedirectPlugin::new(&root)?)
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: root.to_string_lossy().into_owned(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: format!("{} · Bevy", level.title),
                        resolution: (1280, 720).into(),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .insert_resource(ClearColor(Color::srgb(0.035, 0.04, 0.055)))
        .add_plugins(GameLevelPlugin(level))
        .add_systems(Update, fly_camera);
    Ok(app)
}

fn projection(camera: &CameraDefinition, lens: f32) -> Projection {
    match camera.projection {
        LensKind::Perspective => Projection::Perspective(PerspectiveProjection {
            fov: lens.to_radians(),
            near: 0.05,
            far: 10_000.0,
            ..default()
        }),
        LensKind::Orthographic => Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: lens,
            },
            near: 0.01,
            far: 10_000.0,
            ..OrthographicProjection::default_3d()
        }),
    }
}

fn setup_level(
    mut commands: Commands,
    level: Res<GameLevel>,
    server: Res<AssetServer>,
    mut status: ResMut<GameLevelStatus>,
) {
    let Some(camera) = level
        .cameras
        .iter()
        .find(|camera| camera.id == level.camera)
    else {
        status.error = Some("Уровень не содержит игровой камеры".into());
        return;
    };
    let placement = level
        .edits
        .cameras
        .get(&camera.id)
        .unwrap_or(&camera.placement);
    let environment = level.edits.lighting.as_ref().unwrap_or(&level.environment);
    commands.insert_resource(DirectionalLightShadowMap {
        size: environment.shadow_resolution as usize,
    });
    commands.insert_resource(PointLightShadowMap {
        size: environment.shadow_resolution as usize,
    });
    let mut entity = commands.spawn((
        GameCamera,
        Camera3d::default(),
        projection(camera, placement.lens),
        placement.pose.transform(),
    ));
    super::apply_camera_lighting(&mut entity, environment);
    let spatial = level
        .edits
        .hierarchy
        .as_ref()
        .map(|h| h.spawn(&mut commands))
        .unwrap_or_default();
    for model in &level.edits.models {
        status.pending_instances += 1;
        let handle = super::load_model(&server, model);
        status.models.push(handle.clone());
        let mut instance = commands.spawn((
            GameModel(model.id),
            Name::new(
                level
                    .edits
                    .model_names
                    .get(&model.id)
                    .cloned()
                    .unwrap_or_else(|| model.asset.clone()),
            ),
            WorldAssetRoot(handle),
            model.pose.transform(),
        ));
        if let Some(parent) = spatial.get(&model.id) {
            instance.insert((Transform::IDENTITY, ChildOf(*parent)));
        }
        instance.observe(prepare_instance);
    }
    for light in &level.edits.lights {
        let mut light = light.clone();
        light.enabled &= environment.scene_lights;
        light.shadows &= environment.shadows;
        let entity = super::spawn_light(&mut commands, &light);
        if let Some(parent) = spatial.get(&light.id) {
            commands
                .entity(entity)
                .insert((Transform::IDENTITY, ChildOf(*parent)));
        }
    }
    status.source_ready = level.asset.is_empty();
    if !level.asset.is_empty() {
        status.pending = Some(
            server
                .load_builder()
                .with_settings(|settings: &mut GltfLoaderSettings| {
                    settings.load_cameras = false;
                    settings.load_lights = true;
                })
                .load(level.asset.clone()),
        );
    }
}

fn load_level(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    mut status: ResMut<GameLevelStatus>,
) {
    if status.error.is_some() || status.ready {
        return;
    }
    for handle in &status.models {
        if let Some(error) = load_error(&server, handle) {
            status.error = Some(error);
            return;
        }
    }
    if let Some(handle) = &status.pending {
        if let Some(error) = load_error(&server, handle) {
            status.error = Some(error);
            return;
        }
        if server.is_loaded_with_dependencies(handle.id())
            && let Some(gltf) = gltfs.get(handle)
        {
            if let Some(scene) = gltf.default_scene.as_ref().or_else(|| gltf.scenes.first()) {
                commands
                    .spawn(WorldAssetRoot(scene.clone()))
                    .observe(prepare_source);
            } else {
                status.error = Some("GLB не содержит сцены".into());
            }
            status.pending = None;
        }
    }
    status.ready = status.source_ready
        && status.pending_instances == 0
        && status
            .models
            .iter()
            .all(|handle| server.is_loaded_with_dependencies(handle.id()));
    if status.ready {
        info!("Bevy game level ready");
    }
}

fn load_error<A: Asset>(server: &AssetServer, handle: &Handle<A>) -> Option<String> {
    use bevy::asset::{LoadState, RecursiveDependencyLoadState};
    match server.get_load_states(handle.id()) {
        Some((LoadState::Failed(error), _, _))
        | Some((_, _, RecursiveDependencyLoadState::Failed(error))) => Some(error.to_string()),
        _ => None,
    }
}

fn prepare_source(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    level: Res<GameLevel>,
    mut status: ResMut<GameLevelStatus>,
    children: Query<&Children>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    transforms: TransformHelper,
    lights: Query<(
        Option<&PointLight>,
        Option<&SpotLight>,
        Option<&DirectionalLight>,
        Option<&RectLight>,
    )>,
    cameras: Query<&Camera>,
) {
    prepare_entities(
        ready.entity,
        &mut commands,
        &level,
        &children,
        &lights,
        &cameras,
    );
    for (entity, name) in super::imported_nodes(
        ready.entity,
        &level.groups,
        &level.excluded_nodes,
        &children,
        &names,
    ) {
        if level.edits.deleted_nodes.contains(name) {
            commands.entity(entity).despawn();
        } else if let Some(edit) = level.edits.nodes.get(name)
            && let Ok(parent) = parents.get(entity)
            && let Ok(parent) = transforms.compute_global_transform(parent.parent())
        {
            commands
                .entity(entity)
                .insert(GlobalTransform::from(edit.pose.transform()).reparented_to(&parent));
        }
    }
    status.source_ready = true;
}

fn prepare_instance(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    level: Res<GameLevel>,
    children: Query<&Children>,
    lights: Query<(
        Option<&PointLight>,
        Option<&SpotLight>,
        Option<&DirectionalLight>,
        Option<&RectLight>,
    )>,
    cameras: Query<&Camera>,
    mut status: ResMut<GameLevelStatus>,
) {
    prepare_entities(
        ready.entity,
        &mut commands,
        &level,
        &children,
        &lights,
        &cameras,
    );
    status.pending_instances = status.pending_instances.saturating_sub(1);
}

fn prepare_entities(
    root: Entity,
    commands: &mut Commands,
    level: &GameLevel,
    children: &Query<&Children>,
    lights: &Query<(
        Option<&PointLight>,
        Option<&SpotLight>,
        Option<&DirectionalLight>,
        Option<&RectLight>,
    )>,
    cameras: &Query<&Camera>,
) {
    let settings = level.edits.lighting.as_ref().unwrap_or(&level.environment);
    for entity in children.iter_descendants(root) {
        if let Ok(camera) = cameras.get(entity) {
            commands.entity(entity).insert(Camera {
                is_active: false,
                ..camera.clone()
            });
        }
        if let Ok((point, spot, directional, rect)) = lights.get(entity) {
            super::apply_imported_light(
                &mut commands.entity(entity),
                settings,
                point,
                spot,
                directional,
                rect,
            );
        }
    }
}

fn fly_camera(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    time: Res<Time>,
    mut window: Single<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut camera: Single<&mut Transform, With<GameCamera>>,
    status: Res<GameLevelStatus>,
    mut exit: MessageWriter<AppExit>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
    if let Some(error) = &status.error {
        error!("Не удалось загрузить уровень: {error}");
        exit.write(AppExit::error());
    }
    let looking = window.0.focused && mouse.pressed(MouseButton::Right);
    window.1.visible = !looking;
    window.1.grab_mode = if looking {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
    if !looking || !status.ready {
        return;
    }
    let (mut yaw, mut pitch, _) = camera.rotation.to_euler(EulerRot::YXZ);
    yaw -= motion.delta.x * 0.003;
    pitch = (pitch - motion.delta.y * 0.003).clamp(-1.55, 1.55);
    camera.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
    let axis = |positive, negative| {
        i32::from(keys.pressed(positive)) as f32 - i32::from(keys.pressed(negative)) as f32
    };
    let input = Vec3::new(
        axis(KeyCode::KeyD, KeyCode::KeyA),
        axis(KeyCode::KeyE, KeyCode::KeyQ),
        axis(KeyCode::KeyS, KeyCode::KeyW),
    );
    let movement = camera.rotation * input.normalize_or_zero();
    let speed = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        15.0
    } else {
        4.0
    };
    camera.translation += movement * speed * time.delta_secs().min(0.1);
}

#[cfg(test)]
mod tests;
