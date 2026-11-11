use std::time::{Duration, Instant};

use bevy::{
    camera::{
        ClearColorConfig, Exposure, Hdr, RenderTarget, ScalingMode, primitives::Aabb,
        visibility::RenderLayers,
    },
    prelude::*,
    render::{
        MainWorld,
        render_resource::{PipelineCache, TextureFormat},
        view::Tonemapping,
    },
    world_serialization::WorldInstanceReady,
};

use super::load_error;

use super::{Editor, bounds_for};

const WIDTH: u32 = 224;
const HEIGHT: u32 = 112;
const LAYER: usize = 2;

#[derive(Resource)]
pub(super) struct Thumbnails {
    pub(super) images: Vec<Handle<Image>>,
    // Reuse these handles when a catalog model is placed in the room.
    assets: Vec<Handle<WorldAsset>>,
    next: usize,
    light: Option<Entity>,
    job: Option<Job>,
}

struct Job {
    root: Entity,
    started: Instant,
    camera: Option<Entity>,
    stable_frames: u8,
}

impl Thumbnails {
    pub(super) fn new(images: &mut Assets<Image>, count: usize) -> Self {
        Self {
            images: (0..count)
                .map(|_| {
                    images.add(Image::new_target_texture(
                        WIDTH,
                        HEIGHT,
                        TextureFormat::Rgba8Unorm,
                        Some(TextureFormat::Rgba8UnormSrgb),
                    ))
                })
                .collect(),
            assets: Vec::with_capacity(count),
            next: 0,
            light: None,
            job: None,
        }
    }
}

pub(super) fn update(
    mut commands: Commands,
    editor: Res<Editor>,
    mut previews: ResMut<Thumbnails>,
    assets: Res<AssetServer>,
    children: Query<&Children>,
    meshes: Query<(&Aabb, &GlobalTransform)>,
    transforms: Query<&GlobalTransform>,
) {
    let Some(mut job) = previews.job.take() else {
        if previews.next == editor.project.catalog.len() {
            if let Some(light) = previews.light.take() {
                commands.entity(light).despawn();
            }
        } else if editor.ready && editor.drawer {
            let model = assets.load(format!(
                "{}#Scene0",
                editor.project.catalog[previews.next].asset
            ));
            let root = commands
                .spawn((
                    WorldAssetRoot(model.clone()),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                ))
                .observe(isolate_model)
                .id();
            previews.assets.push(model);
            previews.job = Some(Job {
                root,
                started: Instant::now(),
                camera: None,
                stable_frames: 0,
            });
        }
        return;
    };

    if let Some(camera) = job.camera {
        if job.stable_frames >= 5 || job.started.elapsed() >= Duration::from_secs(5) {
            commands.entity(camera).despawn();
            commands.entity(job.root).despawn();
            previews.next += 1;
            return;
        }
    } else if let Some(bounds) = bounds_for(job.root, &children, &meshes, &transforms) {
        let direction = Vec3::from(editor.project.catalog[previews.next].preview_direction);
        let light_pose = Transform::from_xyz(3.5, 6.0, 4.0 * direction.z.signum())
            .looking_at(Vec3::ZERO, Vec3::Y);
        if let Some(light) = previews.light {
            commands.entity(light).insert(light_pose);
        } else {
            previews.light = Some(
                commands
                    .spawn((
                        DirectionalLight {
                            illuminance: 900.0,
                            ..default()
                        },
                        light_pose,
                        RenderLayers::layer(LAYER),
                    ))
                    .id(),
            );
        }
        commands.entity(job.root).insert(Visibility::Inherited);
        let center = Vec3::from(bounds.center);
        let extent = Vec3::from(bounds.half_size);
        let view = Transform::from_translation(
            center + direction.normalize() * (extent.length() * 3.0 + 0.6),
        )
        .looking_at(center, Vec3::Y);
        let half_width = extent.dot(view.right().abs());
        let half_height = extent.dot(view.up().abs());
        let vertical =
            (2.0 * half_height.max(half_width * HEIGHT as f32 / WIDTH as f32) * 1.2).max(0.01);
        job.camera = Some(
            commands
                .spawn((
                    Camera3d::default(),
                    Camera {
                        order: -2,
                        clear_color: ClearColorConfig::Custom(Color::srgb(0.16, 0.17, 0.20)),
                        ..default()
                    },
                    RenderTarget::Image(previews.images[previews.next].clone().into()),
                    RenderLayers::layer(LAYER),
                    Hdr,
                    Exposure::INDOOR,
                    Tonemapping::AgX,
                    AmbientLight {
                        brightness: 100.0,
                        ..default()
                    },
                    Projection::Orthographic(OrthographicProjection {
                        scaling_mode: ScalingMode::FixedVertical {
                            viewport_height: vertical,
                        },
                        near: 0.01,
                        far: 100.0,
                        ..OrthographicProjection::default_3d()
                    }),
                    view,
                ))
                .id(),
        );
        job.started = Instant::now();
    } else if let Some(error) = load_error(&assets, &previews.assets[previews.next]).or_else(|| {
        (job.started.elapsed() >= Duration::from_secs(15)).then(|| "no model bounds".into())
    }) {
        warn!(
            "Thumbnail for {}: {error}",
            editor.project.catalog[previews.next].asset
        );
        commands.entity(job.root).despawn();
        previews.next += 1;
        return;
    }
    previews.job = Some(job);
}

fn isolate_model(
    ready: On<WorldInstanceReady>,
    children: Query<&Children>,
    mut commands: Commands,
) {
    for entity in children.iter_descendants(ready.entity) {
        commands.entity(entity).insert(RenderLayers::layer(LAYER));
    }
}

pub(super) fn track_frame(mut world: ResMut<MainWorld>, cache: Res<PipelineCache>) {
    if let Some(mut previews) = world.get_resource_mut::<Thumbnails>()
        && let Some(job) = &mut previews.job
        && job.camera.is_some()
    {
        job.stable_frames = if cache.waiting_pipelines().next().is_none() {
            job.stable_frames.saturating_add(1)
        } else {
            0
        };
    }
}
