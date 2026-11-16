//! An isolated render target driven by the creation dialog's valid parameters.
#[path = "browser_primitive_animation.rs"]
mod animation;

use super::{ContextUi, Geometry, hit};
use crate::editor::{
    model_primitives::{Parameters, Primitive},
    viewport_grid,
};
use animation::Transition;
use bevy::{
    camera::{Exposure, Hdr, RenderTarget, primitives::MeshAabb, visibility::RenderLayers},
    input::mouse::{AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
    render::{
        render_resource::{Extent3d, TextureFormat},
        view::Tonemapping,
    },
    text::EditableText,
    window::PrimaryWindow,
};

const LAYER: usize = 10;
const FOV: f32 = 45.0_f32.to_radians();

#[derive(Component)]
pub(super) struct PreviewCamera;
#[derive(Component)]
pub(super) struct PreviewModel;

struct Stage {
    root: Entity,
    model: Entity,
    grid: Entity,
    camera: Entity,
    mesh: Handle<Mesh>,
    image: Handle<Image>,
}

#[derive(Resource, Default)]
struct Preview {
    stage: Option<Stage>,
    last: Option<(Primitive, Parameters)>,
    transition: Option<Transition>,
    center: Vec3,
    size: Vec3,
    radius: f32,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    drag: Option<Vec2>,
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<Preview>()
        .add_systems(Update, update.after(super::update).after(super::refresh));
}

fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    grids: &mut Assets<viewport_grid::GridMaterial>,
    images: &mut Assets<Image>,
    mesh: Mesh,
) -> Stage {
    let root = commands
        .spawn((
            Name::new("Primitive creation preview"),
            Transform::IDENTITY,
            Visibility::Inherited,
        ))
        .id();
    let mesh = meshes.add(mesh);
    let model = commands
        .spawn((
            PreviewModel,
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.62, 0.65, 0.70),
                perceptual_roughness: 0.72,
                double_sided: true,
                cull_mode: None,
                ..default()
            })),
            Transform::IDENTITY,
            RenderLayers::layer(LAYER),
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .id();
    let grid = viewport_grid::spawn(commands, meshes, grids, LAYER);
    commands.entity(grid).insert((
        Transform::from_xyz(0.0, -0.01, 0.0),
        Visibility::Inherited,
        ChildOf(root),
    ));
    let image = images.add(Image::new_target_texture(
        640,
        512,
        TextureFormat::Rgba8Unorm,
        Some(TextureFormat::Rgba8UnormSrgb),
    ));
    let camera = commands
        .spawn((
            PreviewCamera,
            Camera3d::default(),
            Hdr,
            Camera {
                order: -3,
                clear_color: ClearColorConfig::Custom(Color::linear_rgb(0.025, 0.029, 0.037)),
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
            RenderLayers::layer(LAYER),
            Projection::Perspective(PerspectiveProjection {
                fov: FOV,
                ..default()
            }),
            Transform::from_xyz(3.0, 2.5, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
            AmbientLight {
                brightness: 120.0,
                ..default()
            },
            Exposure::BLENDER,
            Tonemapping::AcesFitted,
            Msaa::Off,
            bevy::anti_alias::fxaa::Fxaa::default(),
            ChildOf(root),
        ))
        .id();
    for (position, illuminance) in [
        (Vec3::new(-3.0, 5.0, 4.0), 6000.0),
        (Vec3::new(4.0, 1.0, -3.0), 2500.0),
    ] {
        commands.spawn((
            DirectionalLight {
                illuminance,
                ..default()
            },
            Transform::from_translation(position).looking_at(Vec3::ZERO, Vec3::Y),
            RenderLayers::layer(LAYER),
            ChildOf(root),
        ));
    }
    Stage {
        root,
        model,
        grid,
        camera,
        mesh,
        image,
    }
}

fn update(
    mut commands: Commands,
    ui: Res<ContextUi>,
    mut preview: ResMut<Preview>,
    (mut meshes, mut materials, mut grids, mut images): (
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
        ResMut<Assets<viewport_grid::GridMaterial>>,
        ResMut<Assets<Image>>,
    ),
    inputs: Query<&EditableText>,
    geometry: Geometry,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cameras: Query<(&mut Camera, &mut Transform, &mut Projection), With<PreviewCamera>>,
    mut labels: Query<&mut Text>,
    mut image_nodes: Query<&mut ImageNode>,
    mut scrolls: Query<&mut ScrollPosition>,
    time: Res<Time>,
) {
    let Some(kind) = ui.phase.primitive() else {
        if let Some(stage) = &preview.stage {
            if let Ok((mut camera, _, _)) = cameras.get_mut(stage.camera) {
                camera.is_active = false;
            }
            if preview.last.is_some() {
                commands.entity(stage.root).insert(Visibility::Hidden);
            }
        }
        preview.last = None;
        preview.transition = None;
        preview.drag = None;
        return;
    };
    if preview.last.is_none() {
        preview.yaw = 0.65;
        preview.pitch = -0.45;
        preview.zoom = 1.0;
        preview.drag = None;
    }
    let requested = ui.parameters.read(kind, &inputs);
    let valid = requested.is_ok();
    if let Ok(parameters) = requested {
        if let Some(transition) = &mut preview.transition
            && transition.kind == kind
        {
            transition.retarget(parameters);
        } else {
            preview.transition = Some(Transition::new(kind, parameters));
        }
    }
    // Continue toward the last valid request while a field is being edited.
    if let Some(transition) = &mut preview.transition {
        let parameters = transition.advance(time.delta_secs());
        if preview.last != Some((kind, parameters))
            && let Ok(mesh) = kind.mesh(&parameters)
            && let Some(bounds) = mesh.get_aabb()
        {
            preview.center = bounds.center.into();
            preview.radius = bounds.half_extents.length().max(0.0001);
            preview.size = Vec3::from(bounds.half_extents) * 2.0;
            if let Some(stage) = &preview.stage {
                let _ = meshes.insert(stage.mesh.id(), mesh);
                // The cached bounds must follow the replacement mesh too.
                commands
                    .entity(stage.model)
                    .remove::<bevy::camera::primitives::Aabb>();
                commands.entity(stage.root).insert(Visibility::Inherited);
            } else {
                let stage = spawn(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    &mut grids,
                    &mut images,
                    mesh,
                );
                if let Ok(mut node) = image_nodes.get_mut(ui.parameters.image) {
                    node.image = stage.image.clone();
                }
                preview.stage = Some(stage);
            }
            if let Some(stage) = &preview.stage {
                commands.entity(stage.grid).insert(Transform::from_xyz(
                    0.0,
                    bounds.center.y - bounds.half_extents.y - preview.radius * 0.005,
                    0.0,
                ));
            }
            preview.last = Some((kind, parameters));
        }
    }
    if let Ok(mut status) = labels.get_mut(ui.parameters.status) {
        let size = preview.size;
        status.set_if_neq(Text(if valid {
            format!("{:.3} × {:.3} × {:.3} м", size.x, size.y, size.z)
        } else {
            "Проверьте параметры · показан последний корректный вариант".into()
        }));
    }
    let cursor = window.cursor_position();
    let hovered = window.focused
        && cursor.is_some_and(|cursor| {
            hit(
                &geometry,
                ui.parameters.preview,
                cursor * window.scale_factor(),
            )
        });
    if hovered && mouse.just_pressed(MouseButton::Left) {
        preview.drag = cursor;
    }
    if !window.focused || !mouse.pressed(MouseButton::Left) {
        preview.drag = None;
    }
    if let (Some(previous), Some(cursor)) = (preview.drag, cursor) {
        let delta = cursor - previous;
        preview.yaw -= delta.x * 0.008;
        preview.pitch = (preview.pitch - delta.y * 0.008).clamp(-1.5, 1.5);
        preview.drag = Some(cursor);
    }
    if hovered && scroll.delta.y != 0.0 {
        let factor = if scroll.unit == MouseScrollUnit::Line {
            0.13
        } else {
            0.002
        };
        preview.zoom = (preview.zoom * (-scroll.delta.y * factor).exp()).clamp(0.45, 4.0);
    } else if scroll.delta.y != 0.0
        && window.focused
        && cursor.is_some_and(|cursor| hit(&geometry, ui.dialog, cursor * window.scale_factor()))
        && let Ok((_, node, _, _)) = geometry.get(ui.dialog)
        && let Ok(mut offset) = scrolls.get_mut(ui.dialog)
    {
        let amount = if scroll.unit == MouseScrollUnit::Line {
            33.0
        } else {
            1.0
        };
        let max = ((node.content_size().y - node.size().y) * node.inverse_scale_factor()).max(0.0);
        offset.y = (offset.y - scroll.delta.y * amount).clamp(0.0, max);
    }
    let Some(stage) = &preview.stage else {
        return;
    };
    let size = geometry
        .get(ui.parameters.preview)
        .ok()
        .map(|(_, node, _, _)| node.size() - Vec2::splat(2.0))
        .unwrap_or(Vec2::new(640.0, 512.0))
        .max(Vec2::splat(1.0));
    let extent = Extent3d {
        width: size.x.ceil().min(2048.0) as u32,
        height: size.y.ceil().min(2048.0) as u32,
        depth_or_array_layers: 1,
    };
    if images
        .get(&stage.image)
        .is_some_and(|image| image.texture_descriptor.size != extent)
        && let Some(mut image) = images.get_mut(&stage.image)
    {
        image.resize(extent);
    }
    // Fit either dimension, preserve the orbit and user's zoom while editing.
    let half_angle = ((FOV * 0.5).tan() * (size.x / size.y).min(1.0)).atan();
    let distance = preview.radius / half_angle.sin() * 1.15 * preview.zoom;
    if let Ok((mut camera, mut pose, mut projection)) = cameras.get_mut(stage.camera) {
        camera.is_active = true;
        let rotation = Quat::from_euler(EulerRot::YXZ, preview.yaw, preview.pitch, 0.0);
        *pose = Transform::from_translation(preview.center + rotation * Vec3::Z * distance)
            .looking_at(preview.center, Vec3::Y);
        if let Projection::Perspective(projection) = &mut *projection {
            projection.near = (preview.radius * 0.001).clamp(0.0000001, 1.0);
            projection.far = (distance + preview.radius * 4.0).max(100.0);
        }
    }
}
