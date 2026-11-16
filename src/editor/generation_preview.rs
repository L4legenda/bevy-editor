//! A render layer and camera owned only by the generation result preview.
use super::*;
use crate::editor::wireframe;
use bevy::{
    asset::RenderAssetUsages,
    camera::{ClearColorConfig, RenderTarget, visibility::RenderLayers},
    input::mouse::{AccumulatedMouseScroll, MouseScrollUnit},
    mesh::{Indices, PrimitiveTopology},
    render::render_resource::{Extent3d, TextureFormat},
};

const LAYER: usize = 17;
#[derive(Component)]
struct PreviewCamera;
#[derive(Component)]
struct PreviewWire;
struct Data {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
    edges: Vec<u32>,
    min: Vec3,
    max: Vec3,
}

#[derive(Resource)]
struct Preview {
    canvas: Entity,
    loading: Entity,
    loading_fill: Entity,
    image: Handle<Image>,
    mesh: Option<Handle<Mesh>>,
    object: Option<Entity>,
    wire_mesh: Option<Handle<Mesh>>,
    wire_object: Option<Entity>,
    camera: Option<Entity>,
    material: Handle<StandardMaterial>,
    wire_material: Handle<wireframe::WireMaterial>,
    requested: Option<PathBuf>,
    task: Option<Task<Result<Data, String>>>,
    ready: bool,
    center: Vec3,
    radius: f32,
    distance: f32,
    yaw: f32,
    pitch: f32,
    last: Option<Vec2>,
}

pub(super) fn register(app: &mut App) {
    app.add_systems(Startup, initialize.after(crate::editor::ui::setup))
        .add_systems(
            Update,
            update
                .after(super::update)
                .after(crate::editor::ui::refresh)
                .before(super::super::progress::animate),
        );
}
pub(super) fn setup(commands: &mut Commands, parent: Entity, _handle: &Handle<Font>) {
    let canvas = commands
        .spawn((
            ImageNode::default(),
            Node {
                display: Display::None,
                width: percent(100),
                min_height: px(80),
                flex_grow: 1.0,
                overflow: Overflow::clip(),
                border_radius: BorderRadius::all(px(8)),
                ..default()
            },
            AccessibleLabel("3D предпросмотр результата генерации".into()),
            ChildOf(parent),
        ))
        .id();
    commands.entity(parent).insert_children(1, &[canvas]);
    let loading = node(
        commands,
        parent,
        Node {
            display: Display::None,
            width: percent(100),
            min_height: px(80),
            flex_grow: 1.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..column(10.0)
        },
    );
    text(
        commands,
        loading,
        _handle,
        "Загрузка предпросмотра…",
        13.0,
        MUTED,
    );
    let (track, loading_fill) = super::super::progress::bar(commands, loading, 5.0);
    commands
        .entity(track)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.width = percent(65);
            node.max_width = px(360);
        });
    commands.entity(parent).insert_children(2, &[loading]);
    commands.insert_resource(Preview {
        canvas,
        loading,
        loading_fill,
        image: default(),
        mesh: None,
        object: None,
        wire_mesh: None,
        wire_object: None,
        camera: None,
        material: default(),
        wire_material: default(),
        requested: None,
        task: None,
        ready: false,
        center: Vec3::ZERO,
        radius: 1.0,
        distance: 3.2,
        yaw: 0.45,
        pitch: 0.2,
        last: None,
    });
}
fn initialize(
    mut commands: Commands,
    mut preview: ResMut<Preview>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut wire_materials: ResMut<Assets<wireframe::WireMaterial>>,
) {
    preview.image = images.add(Image::new_target_texture(
        640,
        640,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    preview.material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.72, 0.73, 0.76),
        perceptual_roughness: 0.7,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    preview.wire_material = wire_materials.add(wireframe::WireMaterial::default());
    let camera = commands
        .spawn((
            PreviewCamera,
            Camera3d::default(),
            bevy::camera::Hdr,
            Camera {
                order: -3,
                is_active: false,
                clear_color: ClearColorConfig::Custom(Color::srgb(0.065, 0.07, 0.085)),
                ..default()
            },
            RenderTarget::Image(preview.image.clone().into()),
            RenderLayers::layer(LAYER),
            Transform::from_xyz(1.5, 0.8, 3.0).looking_at(Vec3::ZERO, Vec3::Y),
            AmbientLight {
                brightness: 400.0,
                ..default()
            },
            bevy::camera::Exposure::BLENDER,
            bevy::render::view::Tonemapping::AcesFitted,
            Msaa::Off,
        ))
        .id();
    preview.camera = Some(camera);
    commands.spawn((
        DirectionalLight {
            illuminance: 6000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(3.0, 5.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
        RenderLayers::layer(LAYER),
    ));
}

fn decode(path: &std::path::Path) -> Result<Data, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 512 * 1024 * 1024 {
        return Err("Модель слишком велика для предпросмотра".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let gltf = bevy::gltf::gltf::Gltf::from_slice(&bytes).map_err(|e| e.to_string())?;
    let blob = gltf.blob.as_deref().ok_or("GLB не содержит геометрии")?;
    let mut data = Data {
        positions: Vec::new(),
        normals: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };
    fn visit(
        node: bevy::gltf::gltf::Node,
        world: Mat4,
        blob: &[u8],
        data: &mut Data,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 64 {
            return Err("Слишком глубокая иерархия модели".into());
        }
        let world = world * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            for primitive in mesh.primitives() {
                if primitive.mode() != bevy::gltf::gltf::mesh::Mode::Triangles {
                    continue;
                }
                let reader = primitive.reader(|buffer| (buffer.index() == 0).then_some(blob));
                let positions: Vec<_> = reader
                    .read_positions()
                    .ok_or("В модели нет вершин")?
                    .collect();
                if positions.len() + data.positions.len() > 10_000_000 {
                    return Err("Слишком много вершин для предпросмотра".into());
                }
                let base = data.positions.len() as u32;
                for p in &positions {
                    let p = world.transform_point3(Vec3::from_array(*p));
                    if !p.is_finite() {
                        return Err("Модель содержит некорректные вершины".into());
                    }
                    data.min = data.min.min(p);
                    data.max = data.max.max(p);
                    data.positions.push(p.to_array());
                }
                let normal_matrix = world.inverse().transpose();
                if let Some(normals) = reader.read_normals() {
                    data.normals.extend(normals.map(|n| {
                        normal_matrix
                            .transform_vector3(Vec3::from_array(n))
                            .normalize_or_zero()
                            .to_array()
                    }));
                } else {
                    data.normals.resize(data.positions.len(), [0.0; 3]);
                }
                let indices: Vec<u32> = reader
                    .read_indices()
                    .map(|i| i.into_u32().collect())
                    .unwrap_or_else(|| (0..positions.len() as u32).collect());
                if indices.len() % 3 != 0 || indices.iter().any(|i| *i as usize >= positions.len())
                {
                    return Err("В модели повреждены грани".into());
                }
                data.indices.extend(indices.into_iter().map(|i| i + base));
            }
        }
        for child in node.children() {
            visit(child, world, blob, data, depth + 1)?;
        }
        Ok(())
    }
    let scene = gltf
        .default_scene()
        .or_else(|| gltf.scenes().next())
        .ok_or("В GLB нет сцены")?;
    for node in scene.nodes() {
        visit(node, Mat4::IDENTITY, blob, &mut data, 0)?;
    }
    if data.positions.is_empty() || data.indices.is_empty() {
        return Err("В результате нет геометрии для предпросмотра".into());
    }
    finish_normals(&mut data);
    // Work once in the decoder task, never while orbiting or clicking the toggle.
    data.edges = wire_indices(&data.indices);
    Ok(data)
}

fn finish_normals(data: &mut Data) {
    let missing: Vec<_> = data
        .normals
        .iter_mut()
        .map(|normal| {
            if let Some(unit) = Vec3::from_array(*normal)
                .try_normalize()
                .filter(|n| n.is_finite())
            {
                *normal = unit.to_array();
                false
            } else {
                true
            }
        })
        .collect();
    if !missing.iter().any(|&missing| missing) {
        return;
    }

    // Normalize edges before computing corner angles. Testing their squared
    // lengths against an absolute epsilon discards dense, small-scale triangles.
    let mut generated = vec![Vec3::ZERO; data.positions.len()];
    for face in data.indices.chunks_exact(3) {
        let [a, b, c] = [face[0] as usize, face[1] as usize, face[2] as usize];
        let pa = Vec3::from_array(data.positions[a]);
        let pb = Vec3::from_array(data.positions[b]);
        let pc = Vec3::from_array(data.positions[c]);
        let (Some(ab), Some(ac), Some(bc)) = (
            (pb - pa).try_normalize(),
            (pc - pa).try_normalize(),
            (pc - pb).try_normalize(),
        ) else {
            continue;
        };
        let Some(normal) = ab.cross(ac).try_normalize() else {
            continue;
        };
        let angle = |u: Vec3, v: Vec3| u.cross(v).length().atan2(u.dot(v));
        generated[a] += normal * angle(ab, ac);
        generated[b] += normal * angle(-ab, bc);
        generated[c] += normal * angle(-ac, -bc);
    }
    for (i, &missing) in missing.iter().enumerate() {
        if missing {
            data.normals[i] = generated[i].try_normalize().unwrap_or(Vec3::Y).to_array();
        }
    }
}

fn wire_indices(indices: &[u32]) -> Vec<u32> {
    let mut edges = Vec::with_capacity(indices.len());
    for face in indices.chunks_exact(3) {
        for (a, b) in [(face[0], face[1]), (face[1], face[2]), (face[2], face[0])] {
            if a != b {
                edges.push([a.min(b), a.max(b)]);
            }
        }
    }
    edges.sort_unstable();
    edges.dedup();
    edges.into_iter().flatten().collect()
}

fn update(
    mut commands: Commands,
    editor: Res<Editor>,
    mut state: ResMut<State>,
    mut preview: ResMut<Preview>,
    mut events: MessageReader<Action>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut focus: ResMut<InputFocus>,
    mut canvas: Query<(&ComputedNode, &UiGlobalTransform, &mut Node, &mut ImageNode)>,
    mut cameras: Query<(&mut Camera, &mut Transform, &mut Projection), With<PreviewCamera>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut loading_nodes: Query<&mut Node, Without<ImageNode>>,
    mut wires: Query<&mut Visibility, With<PreviewWire>>,
) {
    let wanted = state.result.as_ref().map(|result| result.glb());
    if wanted != preview.requested {
        preview.requested = wanted.clone();
        preview.task = None;
        preview.ready = false;
        if let Some(path) = wanted {
            preview.task = Some(IoTaskPool::get().spawn(async move { decode(&path) }));
        }
    }
    if let Some(result) = preview
        .task
        .as_mut()
        .and_then(|task| block_on(poll_once(task)))
    {
        preview.task = None;
        match result {
            Ok(data) => {
                let wire = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::RENDER_WORLD)
                    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, data.positions.clone())
                    .with_inserted_indices(Indices::U32(data.edges));
                let mut mesh = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
                );
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, data.positions);
                mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, data.normals);
                mesh.insert_indices(Indices::U32(data.indices));
                if let Some(entity) = preview.object.take() {
                    commands.entity(entity).despawn();
                }
                if let Some(mesh) = preview.mesh.take() {
                    meshes.remove(mesh.id());
                }
                if let Some(entity) = preview.wire_object.take() {
                    commands.entity(entity).despawn();
                }
                if let Some(mesh) = preview.wire_mesh.take() {
                    meshes.remove(mesh.id());
                }
                let wire_mesh = meshes.add(wire);
                preview.wire_mesh = Some(wire_mesh.clone());
                preview.wire_object = Some(
                    commands
                        .spawn((
                            PreviewWire,
                            Mesh3d(wire_mesh),
                            MeshMaterial3d(preview.wire_material.clone()),
                            Transform::IDENTITY,
                            RenderLayers::layer(LAYER),
                            if editor.generation.wireframe {
                                Visibility::Visible
                            } else {
                                Visibility::Hidden
                            },
                            Pickable::IGNORE,
                            bevy::light::NotShadowCaster,
                            bevy::light::NotShadowReceiver,
                        ))
                        .id(),
                );
                let mesh = meshes.add(mesh);
                preview.mesh = Some(mesh.clone());
                preview.object = Some(
                    commands
                        .spawn((
                            Mesh3d(mesh),
                            MeshMaterial3d(preview.material.clone()),
                            Transform::IDENTITY,
                            RenderLayers::layer(LAYER),
                        ))
                        .id(),
                );
                preview.center = (data.min + data.max) * 0.5;
                preview.radius = ((data.max - data.min).length() * 0.5).max(0.001);
                preview.distance = preview.radius * 3.0;
                preview.yaw = 0.45;
                preview.pitch = 0.2;
                preview.ready = true;
            }
            Err(error) => state.error = format!("Не удалось показать модель: {error}"),
        }
    }
    for mut visibility in &mut wires {
        visibility.set_if_neq(if editor.generation.wireframe {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
    }
    for action in events.read() {
        if matches!(action, Action::Generation(GenerationAction::FitResult)) {
            preview.distance = preview.radius * 3.0;
            preview.yaw = 0.45;
            preview.pitch = 0.2;
        }
    }
    let visible = editor.generation.active && editor.generation.backend_ready && preview.ready;
    if let Ok(mut node) = loading_nodes.get_mut(preview.loading) {
        node.display = if preview.task.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    super::super::progress::set(
        &mut commands,
        preview.loading_fill,
        if preview.task.is_some() && editor.generation.active {
            super::super::progress::Value::Busy
        } else {
            super::super::progress::Value::Hidden
        },
    );
    let enabled = visible
        && !editor.generation.running
        && !editor.drawer
        && editor.pending.is_none()
        && window.focused;
    let Ok((computed, transform, mut node, mut picture)) = canvas.get_mut(preview.canvas) else {
        return;
    };
    node.display = if preview.ready {
        Display::Flex
    } else {
        Display::None
    };
    picture.image = preview.image.clone();
    let point = window.cursor_position().map(|p| p * window.scale_factor());
    let inside = point.is_some_and(|point| {
        Rect::from_center_size(transform.translation, computed.size()).contains(point)
    });
    if enabled && mouse.just_pressed(MouseButton::Left) && inside {
        preview.last = point;
        focus.clear();
    }
    if !enabled || !mouse.pressed(MouseButton::Left) {
        preview.last = None;
    }
    if let (Some(last), Some(point)) = (preview.last, point) {
        let delta = point - last;
        preview.yaw -= delta.x * 0.006;
        preview.pitch = (preview.pitch + delta.y * 0.006).clamp(-1.45, 1.45);
        preview.last = Some(point);
    }
    if enabled && inside {
        let amount = scroll.delta.y
            * if scroll.unit == MouseScrollUnit::Pixel {
                0.005
            } else {
                0.12
            };
        preview.distance = (preview.distance * (-amount).exp())
            .clamp(preview.radius * 0.15, preview.radius * 30.0);
    }
    if let Some(camera) = preview.camera
        && let Ok((mut camera, mut transform, mut projection)) = cameras.get_mut(camera)
    {
        camera.is_active = visible && !editor.generation.running;
        let size = computed
            .size()
            .max(Vec2::splat(16.0))
            .min(Vec2::splat(2048.0))
            .as_uvec2();
        if let Some(mut image) = images.get_mut(preview.image.id())
            && (image.width() != size.x || image.height() != size.y)
        {
            image.resize(Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            });
        }
        if let Projection::Perspective(perspective) = &mut *projection {
            perspective.aspect_ratio = size.x as f32 / size.y as f32;
            perspective.near = (preview.radius * 0.001).max(0.0001);
            perspective.far = preview.radius * 100.0;
        }
        let offset = Quat::from_rotation_y(preview.yaw)
            * Quat::from_rotation_x(-preview.pitch)
            * Vec3::Z
            * preview.distance;
        *transform = Transform::from_translation(preview.center + offset)
            .looking_at(preview.center, Vec3::Y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_wireframe_retains_triangle_diagonals_and_deduplicates_shared_edges() {
        // Two triangles forming a square: four sides plus their real diagonal.
        assert_eq!(
            wire_indices(&[0, 1, 2, 2, 1, 3]),
            [0, 1, 0, 2, 1, 2, 1, 3, 2, 3]
        );
        assert_eq!(wire_indices(&[1, 1, 2]), [1, 2]);
    }

    fn triangle(scale: f32, normals: Vec<[f32; 3]>) -> Data {
        Data {
            positions: vec![[0.0; 3], [scale, 0.0, 0.0], [0.0, scale, 0.0]],
            normals,
            indices: vec![0, 1, 2],
            edges: vec![],
            min: Vec3::ZERO,
            max: Vec3::splat(scale),
        }
    }

    #[test]
    fn generation_preview_normals_do_not_disappear_on_dense_small_triangles() {
        for scale in [0.000001, 0.005, 1.0, 10000.0] {
            let mut data = triangle(scale, vec![[0.0; 3]; 3]);
            finish_normals(&mut data);
            assert!(
                data.normals
                    .iter()
                    .all(|n| Vec3::from_array(*n).abs_diff_eq(Vec3::Z, 0.00001)),
                "Scale {scale}"
            );
        }
    }

    #[test]
    fn generation_preview_keeps_authored_normals_and_only_repairs_missing_values() {
        let mut data = triangle(
            0.005,
            vec![Vec3::Y.to_array(), [0.0; 3], (-Vec3::X).to_array()],
        );
        finish_normals(&mut data);
        assert_eq!(
            data.normals,
            [
                Vec3::Y.to_array(),
                Vec3::Z.to_array(),
                (-Vec3::X).to_array()
            ]
        );
    }

    #[test]
    fn generation_preview_degenerate_faces_do_not_poison_neighbor_normals() {
        let mut data = triangle(0.005, vec![[f32::NAN; 3]; 3]);
        data.indices.extend([0, 0, 1]);
        finish_normals(&mut data);
        assert_eq!(data.normals, [Vec3::Z.to_array(); 3]);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "generation_surface_diagnostic.rs"]
mod surface_diagnostic;
