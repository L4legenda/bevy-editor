//! Imports FBX meshes into the same Bevy WorldAsset used by placed GLB models.
use std::{
    collections::{HashMap, HashSet},
    io::{self, Cursor},
    path::{Component, Path, PathBuf},
};

use bevy::{
    asset::{AssetLoader, AssetPath, LoadContext, RenderAssetUsages, io::Reader},
    camera::primitives::{Aabb, MeshAabb},
    image::{ImageAddressMode, ImageSampler},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    reflect::TypePath,
    tasks::futures_lite::io::AsyncReadExt,
};

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "fbx_tests.rs"]
mod tests;

const MAX_FBX_BYTES: usize = 512 * 1024 * 1024;
const MAX_TRIANGLES: usize = 4_000_000;

/// Registers the FBX mesh loader. EditorPlugin and GameLevelPlugin add it
/// automatically. Custom apps can add it after Bevy's 3D and asset plugins.
pub struct FbxPlugin;

impl Plugin for FbxPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset_loader::<FbxLoader>();
    }
}

#[derive(Default, TypePath)]
struct FbxLoader;

impl AssetLoader for FbxLoader {
    type Asset = WorldAsset;
    type Settings = ();
    type Error = io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> io::Result<WorldAsset> {
        let mut bytes = Vec::new();
        reader
            .take((MAX_FBX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await?;
        let scene = parse(&bytes, false)?;
        drop(bytes);
        if scene.meshes.is_empty() {
            return Err(invalid("В FBX нет полигональных мешей"));
        }
        let triangles = (&scene.meshes).into_iter().try_fold(0usize, |sum, mesh| {
            sum.checked_add(mesh.num_triangles)
                .filter(|total| *total <= MAX_TRIANGLES)
                .ok_or_else(|| invalid("Слишком много треугольников в FBX (максимум 4 000 000)"))
        })?;
        if triangles == 0 {
            return Err(invalid("В FBX нет полигональных поверхностей"));
        }
        let textures = load_textures(&scene, context).await;
        build_world(&scene, &textures, context)
    }

    fn extensions(&self) -> &[&str] {
        // AssetServer matches registered extensions case-sensitively, including on reload.
        &["fbx", "FBX", "Fbx", "fBx", "fbX", "FBx", "FbX", "fBX"]
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn parse(bytes: &[u8], ignore_geometry: bool) -> io::Result<ufbx::SceneRoot> {
    if bytes.len() > MAX_FBX_BYTES {
        return Err(invalid("Размер FBX превышает 512 МиБ"));
    }
    ufbx::load_memory(
        bytes,
        ufbx::LoadOpts {
            file_format: ufbx::FileFormat::Fbx,
            temp_allocator: ufbx::AllocatorOpts {
                memory_limit: MAX_FBX_BYTES,
                ..default()
            },
            result_allocator: ufbx::AllocatorOpts {
                memory_limit: MAX_FBX_BYTES,
                ..default()
            },
            node_depth_limit: 256,
            ignore_geometry,
            ignore_animation: true,
            load_external_files: false,
            use_blender_pbr_material: true,
            generate_missing_normals: true,
            normalize_normals: true,
            geometry_transform_handling: ufbx::GeometryTransformHandling::HelperNodes,
            inherit_mode_handling: ufbx::InheritModeHandling::HelperNodes,
            space_conversion: ufbx::SpaceConversion::AdjustTransforms,
            target_axes: ufbx::axes_right_handed_y_up(),
            target_unit_meters: 1.0,
            ..default()
        },
    )
    .map_err(|error| invalid(format!("Не удалось прочитать FBX: {}", error.description)))
}

fn vector(value: ufbx::Vec3) -> Vec3 {
    Vec3::new(value.x as f32, value.y as f32, value.z as f32)
}

fn transform(value: ufbx::Transform) -> io::Result<Transform> {
    let translation = vector(value.translation);
    let scale = vector(value.scale);
    let r = value.rotation;
    let rotation = Quat::from_xyzw(r.x as f32, r.y as f32, r.z as f32, r.w as f32);
    if !translation.is_finite()
        || !scale.is_finite()
        || !rotation.is_finite()
        || rotation.length_squared() < 1e-10
    {
        return Err(invalid("Некорректная трансформация объекта FBX"));
    }
    Ok(Transform {
        translation,
        rotation: rotation.normalize(),
        scale,
    })
}

struct Part {
    slot: usize,
    mesh: Handle<Mesh>,
    bounds: Aabb,
}

fn build_world(
    scene: &ufbx::Scene,
    textures: &HashMap<TextureKey, LoadedTexture>,
    context: &mut LoadContext<'_>,
) -> io::Result<WorldAsset> {
    let default_material = label(
        context,
        "DefaultMaterial".into(),
        StandardMaterial {
            base_color: Color::srgb(0.78, 0.78, 0.78),
            ..default()
        },
    );
    let materials: HashMap<_, _> = (&scene.materials)
        .into_iter()
        .map(|material| {
            let id = material.element.typed_id;
            (
                id,
                label(
                    context,
                    format!("Material{id}"),
                    standard_material(material, textures),
                ),
            )
        })
        .collect();
    let mut parts = HashMap::new();
    let has_normal_maps = textures.keys().any(|key| !key.srgb);
    for mesh in &scene.meshes {
        let mut triangles = Vec::new();
        let mut chunks = Vec::new();
        let id = mesh.element.typed_id;
        for part in &mesh.material_parts {
            if let Some(mut data) = mesh_part(mesh, part, &mut triangles)? {
                if mesh.vertex_uv.exists && has_normal_maps {
                    // MikkTSpace uses the converted UVs, including the V-axis flip.
                    let _ = data.generate_tangents();
                }
                let bounds = data
                    .get_aabb()
                    .ok_or_else(|| invalid("Меш FBX не имеет границ"))?;
                chunks.push(Part {
                    slot: part.index as usize,
                    mesh: label(context, format!("Mesh{id}/Part{}", part.index), data),
                    bounds,
                });
            }
        }
        parts.insert(id, chunks);
    }

    let mut world = World::new();
    let mut nodes = HashMap::new();
    let mut mesh_count = 0;
    for node in &scene.nodes {
        let name = &*node.element.name;
        let id = world
            .spawn((
                Name::new(if name.is_empty() { "FBX" } else { name }.to_owned()),
                transform(node.local_transform)?,
                if node.visible || node.is_root {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
            ))
            .id();
        nodes.insert(node.element.typed_id, id);
        if let Some(mesh) = &node.mesh
            && let Some(chunks) = parts.get(&mesh.element.typed_id)
        {
            for part in chunks {
                // FBX material slots are local to the node instance.
                let material = node
                    .materials
                    .get(part.slot)
                    .or_else(|| mesh.materials.get(part.slot))
                    .and_then(|material| materials.get(&material.element.typed_id))
                    .unwrap_or(&default_material);
                world.spawn((
                    Mesh3d(part.mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    part.bounds,
                    Transform::IDENTITY,
                    ChildOf(id),
                ));
                mesh_count += 1;
            }
        }
    }
    if mesh_count == 0 {
        return Err(invalid("В FBX нет полигональных поверхностей в узлах"));
    }
    for node in &scene.nodes {
        if let Some(parent) = &node.parent {
            world
                .entity_mut(nodes[&node.element.typed_id])
                .insert(ChildOf(nodes[&parent.element.typed_id]));
        }
    }
    Ok(WorldAsset::new(world))
}

// WorldAsset does not reflect handles inside its World for dependency tracking.
fn label<A: Asset>(context: &mut LoadContext<'_>, name: String, asset: A) -> Handle<A> {
    context.add_labeled_asset(name.clone(), asset);
    context.get_label_handle(name)
}

fn mesh_part(
    mesh: &ufbx::Mesh,
    part: &ufbx::MeshPart,
    triangles: &mut Vec<u32>,
) -> io::Result<Option<Mesh>> {
    let mut vertices = HashMap::<[u32; 4], u32>::new();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::with_capacity(part.num_triangles * 3);
    for &face_index in &*part.face_indices {
        let face_index = face_index as usize;
        let Some(&face) = mesh.faces.get(face_index) else {
            return Err(invalid("Некорректный индекс грани в FBX"));
        };
        if face.num_indices < 3 || mesh.face_hole.get(face_index) == Some(&true) {
            continue;
        }
        let count = ufbx::triangulate_face_vec(triangles, mesh, face) as usize * 3;
        for &corner in &triangles[..count] {
            let corner = corner as usize;
            let key = [
                &mesh.vertex_position.indices,
                &mesh.vertex_normal.indices,
                &mesh.vertex_uv.indices,
                &mesh.vertex_color.indices,
            ]
            .map(|indices| indices.get(corner).copied().unwrap_or(u32::MAX));
            let index = if let Some(&index) = vertices.get(&key) {
                index
            } else {
                let position = mesh
                    .vertex_position
                    .values
                    .get(key[0] as usize)
                    .map(|&position| vector(position))
                    .ok_or_else(|| invalid("Некорректный индекс вершины в FBX"))?;
                let normal = mesh
                    .vertex_normal
                    .values
                    .get(key[1] as usize)
                    .map_or(Vec3::Y, |&normal| vector(normal));
                if !position.is_finite() || !normal.is_finite() {
                    return Err(invalid("Нечисловые координаты или нормали в FBX"));
                }
                let index = positions.len() as u32;
                positions.push(position.to_array());
                normals.push(normal.try_normalize().unwrap_or(Vec3::Y).to_array());
                if mesh.vertex_uv.exists {
                    let uv = mesh
                        .vertex_uv
                        .values
                        .get(key[2] as usize)
                        .map_or(Vec2::ZERO, |uv| Vec2::new(uv.x as f32, 1.0 - uv.y as f32));
                    uvs.push(if uv.is_finite() {
                        uv.to_array()
                    } else {
                        [0.0; 2]
                    });
                }
                if mesh.vertex_color.exists {
                    let color = mesh
                        .vertex_color
                        .values
                        .get(key[3] as usize)
                        .map_or([1.0; 4], |c| [c.x, c.y, c.z, c.w].map(|v| channel(v, 1.0)));
                    colors.push(color);
                }
                vertices.insert(key, index);
                index
            };
            indices.push(index);
        }
    }
    if indices.is_empty() {
        return Ok(None);
    }
    let mut data = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices));
    if !uvs.is_empty() {
        data.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    }
    if !colors.is_empty() {
        data.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    Ok(Some(data))
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct TextureKey {
    id: u32,
    srgb: bool,
}

struct LoadedTexture {
    handle: Handle<Image>,
    translucent: bool,
}

fn material_textures(material: &ufbx::Material) -> [Option<TextureKey>; 3] {
    let pbr = &material.pbr;
    [
        texture_key(&pbr.base_color, true),
        texture_key(&pbr.normal_map, false),
        texture_key(&pbr.emission_color, true),
    ]
}

fn texture_key(map: &ufbx::MaterialMap, srgb: bool) -> Option<TextureKey> {
    if map.feature_disabled || !map.texture_enabled {
        return None;
    }
    let texture = map.texture.as_ref()?.file_textures.first()?;
    texture.has_file.then_some(TextureKey {
        id: texture.element.typed_id,
        srgb,
    })
}

async fn load_textures(
    scene: &ufbx::SceneRoot,
    context: &mut LoadContext<'_>,
) -> HashMap<TextureKey, LoadedTexture> {
    let used: HashSet<_> = (&scene.materials)
        .into_iter()
        .flat_map(material_textures)
        .flatten()
        .collect();
    let mut result = HashMap::new();
    let source = context.path().clone();
    for key in used {
        let (mut decoded, path, wrap_u, wrap_v) = {
            let texture = &scene.textures[key.id as usize];
            let file = &scene.texture_files[texture.file_index as usize];
            let decoded = (!file.content.is_empty())
                .then(|| decode_image(&file.content, &file.relative_filename))
                .flatten();
            (
                decoded,
                texture_location(source.path(), file),
                texture.wrap_u,
                texture.wrap_v,
            )
        };
        if decoded.is_none()
            && let Some(path) = &path
            && let Ok(bytes) = context
                .read_asset_bytes(
                    AssetPath::from_path_buf(path.clone()).with_source(source.source().clone()),
                )
                .await
        {
            decoded = decode_image(&bytes, &path.to_string_lossy());
        }
        let Some(pixels) = decoded else {
            if let Some(path) = path {
                warn!("FBX {source}: текстура {} недоступна", path.display());
            }
            continue;
        };
        let translucent = key.srgb && pixels.pixels().any(|pixel| pixel[3] < 255);
        let mut image =
            Image::from_dynamic(image::DynamicImage::ImageRgba8(pixels), key.srgb, default());
        image.sampler = ImageSampler::linear();
        let sampler = image.sampler.get_or_init_descriptor();
        let wrap = |mode| match mode {
            ufbx::WrapMode::Repeat => ImageAddressMode::Repeat,
            ufbx::WrapMode::Clamp => ImageAddressMode::ClampToEdge,
        };
        sampler.address_mode_u = wrap(wrap_u);
        sampler.address_mode_v = wrap(wrap_v);
        let handle = context.add_labeled_asset(format!("Texture{}/{}", key.id, key.srgb), image);
        result.insert(
            key,
            LoadedTexture {
                handle,
                translucent,
            },
        );
    }
    result
}

fn decode_image(data: &[u8], name: &str) -> Option<image::RgbaImage> {
    let format = image::guess_format(data)
        .ok()
        .or_else(|| image::ImageFormat::from_path(name).ok())?;
    let mut reader = image::ImageReader::with_format(Cursor::new(data), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    Some(reader.decode().ok()?.into_rgba8())
}

fn texture_location(source: &Path, file: &ufbx::TextureFile) -> Option<PathBuf> {
    texture_path(source, &file.relative_filename).or_else(|| texture_path(source, &file.filename))
}

/// Resolve external texture references at the FBX's source and destination.
#[cfg(feature = "editor")]
pub(crate) fn external_texture_paths(
    root: &Path,
    source: &Path,
    destination: &Path,
) -> io::Result<Vec<(PathBuf, Option<PathBuf>)>> {
    use std::io::Read;

    let mut bytes = Vec::new();
    std::fs::File::open(root.join(source))?
        .take((MAX_FBX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let scene = parse(&bytes, true)?;
    Ok(scene
        .texture_files
        .iter()
        .filter(|file| file.content.is_empty())
        .filter_map(|file| {
            Some((
                texture_location(source, file)?,
                texture_location(destination, file),
            ))
        })
        .collect())
}

fn texture_path(source: &Path, reference: &str) -> Option<PathBuf> {
    if reference.is_empty() || reference.contains([':', '#', '?', '\0']) {
        return None;
    }
    let reference = reference.replace('\\', "/");
    let mut path = source.parent()?.to_path_buf();
    for component in Path::new(&reference).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            Component::ParentDir if path.pop() => {}
            _ => return None,
        }
    }
    path.file_name()?;
    Some(path)
}

fn channel(value: f64, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0) as f32
    } else {
        fallback
    }
}

fn scalar(map: &ufbx::MaterialMap, fallback: f32) -> f32 {
    if map.has_value {
        channel(map.value_vec4.x, fallback)
    } else {
        fallback
    }
}

fn rgb(map: &ufbx::MaterialMap, fallback: f32) -> [f32; 3] {
    if map.has_value {
        let v = map.value_vec4;
        [v.x, v.y, v.z].map(|value| channel(value, fallback))
    } else {
        [fallback; 3]
    }
}

fn standard_material(
    material: &ufbx::Material,
    textures: &HashMap<TextureKey, LoadedTexture>,
) -> StandardMaterial {
    let [base, normal, emission] =
        material_textures(material).map(|key| key.and_then(|key| textures.get(&key)));
    let pbr = &material.pbr;
    let factor = scalar(&pbr.base_factor, 1.0);
    let [r, g, b] = rgb(&pbr.base_color, if base.is_some() { 1.0 } else { 0.65 });
    let [er, eg, eb] = rgb(&pbr.emission_color, f32::from(emission.is_some()));
    let energy = scalar(&pbr.emission_factor, 1.0);
    let alpha = scalar(&pbr.opacity, 1.0);
    StandardMaterial {
        base_color: Color::linear_rgba(r * factor, g * factor, b * factor, alpha),
        base_color_texture: base.map(|texture| texture.handle.clone()),
        normal_map_texture: normal.map(|texture| texture.handle.clone()),
        emissive: LinearRgba::rgb(er * energy, eg * energy, eb * energy),
        emissive_texture: emission.map(|texture| texture.handle.clone()),
        metallic: scalar(&pbr.metalness, 0.0),
        perceptual_roughness: scalar(&pbr.roughness, 0.65),
        alpha_mode: if alpha < 0.999 || base.is_some_and(|texture| texture.translucent) {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        },
        double_sided: material.features.double_sided.enabled,
        cull_mode: if material.features.double_sided.enabled {
            None
        } else {
            StandardMaterial::default().cull_mode
        },
        unlit: material.features.unlit.enabled,
        ..default()
    }
}
