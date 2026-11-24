//! Imports FBX meshes into the same Bevy WorldAsset used by placed GLB models.
use std::{
    collections::{HashMap, HashSet},
    io::{self, Cursor},
    path::{Component, Path, PathBuf},
};

use bevy::{
    asset::{AssetLoader, AssetPath, LoadContext, RenderAssetUsages, io::Reader},
    camera::primitives::{Aabb, MeshAabb},
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    reflect::TypePath,
    tasks::futures_lite::io::AsyncReadExt,
};
use serde::Deserialize;

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
        let profiles = import_profiles(context).await?;
        let clinic_materials = clinic_room_materials(context).await?;
        let textures = if profiles.is_empty() && clinic_materials.is_none() {
            load_textures(&scene, context).await
        } else {
            HashMap::new()
        };
        build_world(
            &scene,
            &textures,
            &profiles,
            clinic_materials.as_ref(),
            context,
        )
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
    parse_mesh(bytes, ufbx::FileFormat::Fbx, ignore_geometry)
}

#[cfg(feature = "editor")]
pub(crate) fn load_mesh_file(path: &Path, format: ufbx::FileFormat) -> io::Result<ufbx::SceneRoot> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((MAX_FBX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    parse_mesh(&bytes, format, false)
}

#[cfg(feature = "editor")]
pub(crate) fn load_obj_with_materials(
    path: &Path,
) -> io::Result<(ufbx::SceneRoot, Option<PathBuf>)> {
    use std::io::Read;
    let folder = path.parent().ok_or_else(|| invalid("Нет папки OBJ"))?;
    let name = path.file_name().and_then(|name| name.to_str())
        .ok_or_else(|| invalid("Недопустимое имя OBJ"))?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((MAX_FBX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FBX_BYTES {
        return Err(invalid("Размер OBJ превышает 512 МиБ"));
    }
    let mut material_folder = None;
    let mut open_mtl = |reference: &str, info: &ufbx::OpenFileInfo| {
        let path = folder.join(reference).canonicalize().ok()?;
        if info.type_ != ufbx::OpenFileType::ObjMtl || !path.starts_with(folder) {
            return None;
        }
        let file = std::fs::File::open(&path).ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
            return None;
        }
        material_folder = path.parent().map(Path::to_owned);
        Some(ufbx::Stream::File(file))
    };
    let opts = ufbx::LoadOpts {
        filename: name.into(),
        load_external_files: true,
        ignore_missing_external_files: true,
        open_file_cb: ufbx::OpenFileCb::Mut(&mut open_mtl),
        ..mesh_options(ufbx::FileFormat::Obj, false)
    };
    let scene = ufbx::load_memory(&bytes, opts)
        .map_err(|error| invalid(format!("Не удалось прочитать OBJ: {}", error.description)))?;
    Ok((scene, material_folder))
}

fn parse_mesh(
    bytes: &[u8],
    format: ufbx::FileFormat,
    ignore_geometry: bool,
) -> io::Result<ufbx::SceneRoot> {
    let label = if format == ufbx::FileFormat::Fbx { "FBX" } else { "OBJ" };
    if bytes.len() > MAX_FBX_BYTES {
        return Err(invalid(format!("Размер {label} превышает 512 МиБ")));
    }
    ufbx::load_memory(bytes, mesh_options(format, ignore_geometry))
        .map_err(|error| invalid(format!("Не удалось прочитать {label}: {}", error.description)))
}

fn mesh_options<'a>(format: ufbx::FileFormat, ignore_geometry: bool) -> ufbx::LoadOpts<'a> {
    ufbx::LoadOpts {
            file_format: format,
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
    }
}

pub(crate) fn geometry_matrix(node: &ufbx::Node) -> Mat4 {
    let m = node.geometry_to_world;
    Mat4::from_cols(
        Vec4::new(m.m00 as f32, m.m10 as f32, m.m20 as f32, 0.0),
        Vec4::new(m.m01 as f32, m.m11 as f32, m.m21 as f32, 0.0),
        Vec4::new(m.m02 as f32, m.m12 as f32, m.m22 as f32, 0.0),
        Vec4::new(m.m03 as f32, m.m13 as f32, m.m23 as f32, 1.0),
    )
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportProfile {
    translation: [f32; 3],
    rotation: [f32; 4],
    scale: [f32; 3],
    nodes: Vec<u32>,
    #[serde(default)]
    adjustments: HashMap<u32, ImportTransform>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportTransform {
    translation: [f32; 3],
    rotation: [f32; 4],
    scale: [f32; 3],
}

impl ImportTransform {
    fn transform(&self) -> Transform {
        Transform {
            translation: Vec3::from_array(self.translation),
            rotation: Quat::from_array(self.rotation).normalize(),
            scale: Vec3::from_array(self.scale),
        }
    }
}

impl ImportProfile {
    fn transform(&self) -> Transform {
        Transform {
            translation: Vec3::from_array(self.translation),
            rotation: Quat::from_array(self.rotation).normalize(),
            scale: Vec3::from_array(self.scale),
        }
    }
}

async fn import_profiles(context: &mut LoadContext<'_>) -> io::Result<Vec<ImportProfile>> {
    let source = context.path().clone();
    let path = source.path();
    if !path.starts_with("models/hospital/Meshes") {
        return Ok(Vec::new());
    }
    let file = path
        .file_name()
        .ok_or_else(|| invalid("Missing hospital FBX file name"))?;
    let mut profile = PathBuf::from("models/hospital/fbx-profiles").join(file);
    profile.set_extension("json");
    let bytes = context
        .read_asset_bytes(AssetPath::from_path_buf(profile).with_source(source.source().clone()))
        .await
        .map_err(|error| {
            invalid(format!(
                "Missing hospital FBX import profile for {source}: {error}"
            ))
        })?;
    let profiles: Vec<ImportProfile> = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if profiles.is_empty() {
        return Err(invalid(format!(
            "Empty hospital FBX import profile for {source}"
        )));
    }
    Ok(profiles)
}

const CLINIC_ROOM_MATERIALS: &str = "models/clinic_rooms/materials.json";

#[derive(Deserialize)]
struct ClinicRoomMaterial {
    base_color: [f32; 4],
    metallic: f32,
    roughness: f32,
    emissive: [f32; 3],
    unlit: bool,
    blend: bool,
    double_sided: bool,
    base_color_texture: Option<String>,
    metallic_roughness_texture: Option<String>,
    emissive_texture: Option<String>,
}

impl ClinicRoomMaterial {
    fn to_material(&self, context: &mut LoadContext<'_>) -> StandardMaterial {
        let [r, g, b, a] = self.base_color;
        let [er, eg, eb] = self.emissive;
        StandardMaterial {
            base_color: Color::linear_rgba(r, g, b, a),
            base_color_texture: clinic_room_texture(
                context,
                self.base_color_texture.as_deref(),
                true,
            ),
            metallic: self.metallic,
            perceptual_roughness: self.roughness,
            metallic_roughness_texture: clinic_room_texture(
                context,
                self.metallic_roughness_texture.as_deref(),
                false,
            ),
            emissive: LinearRgba::rgb(er, eg, eb),
            emissive_texture: clinic_room_texture(context, self.emissive_texture.as_deref(), true),
            unlit: self.unlit,
            alpha_mode: if self.blend {
                AlphaMode::Blend
            } else {
                AlphaMode::Opaque
            },
            double_sided: self.double_sided,
            cull_mode: if self.double_sided {
                None
            } else {
                StandardMaterial::default().cull_mode
            },
            ..default()
        }
    }
}

fn clinic_room_texture(
    context: &mut LoadContext<'_>,
    path: Option<&str>,
    srgb: bool,
) -> Option<Handle<Image>> {
    let source = context.path().clone_owned();
    let path = AssetPath::from_path_buf(PathBuf::from(path?)).with_source(source.source().clone());
    Some(
        context
            .load_builder()
            .with_settings(move |settings: &mut ImageLoaderSettings| {
                settings.is_srgb = srgb;
                let mut sampler = ImageSamplerDescriptor::linear();
                sampler.address_mode_u = ImageAddressMode::Repeat;
                sampler.address_mode_v = ImageAddressMode::Repeat;
                settings.sampler = ImageSampler::Descriptor(sampler);
            })
            .load(path),
    )
}

async fn clinic_room_materials(
    context: &mut LoadContext<'_>,
) -> io::Result<Option<HashMap<String, ClinicRoomMaterial>>> {
    let source = context.path().clone();
    if !source.path().starts_with("models/clinic_rooms") {
        return Ok(None);
    }
    // Blender FBX cannot round-trip the original packed surface maps, unlit
    // signs or emissive strength. The palette is shared by the exported props.
    let path = AssetPath::from_path_buf(PathBuf::from(CLINIC_ROOM_MATERIALS))
        .with_source(source.source().clone());
    let bytes = context
        .read_asset_bytes(path)
        .await
        .map_err(io::Error::other)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(io::Error::other)
}

struct Part {
    slot: usize,
    mesh: Handle<Mesh>,
    bounds: Aabb,
}

fn build_world(
    scene: &ufbx::Scene,
    textures: &HashMap<TextureKey, LoadedTexture>,
    profiles: &[ImportProfile],
    clinic_materials: Option<&HashMap<String, ClinicRoomMaterial>>,
    context: &mut LoadContext<'_>,
) -> io::Result<WorldAsset> {
    let mut parts = HashMap::new();
    let required_meshes = (!profiles.is_empty()).then(|| {
        scene
            .nodes
            .iter()
            .filter(|node| {
                profiles
                    .iter()
                    .any(|p| p.nodes.contains(&node.element.typed_id))
            })
            .filter_map(|node| node.mesh.as_ref().map(|mesh| mesh.element.typed_id))
            .collect::<HashSet<_>>()
    });
    let has_normal_maps = textures.keys().any(|key| !key.srgb);
    for mesh in &scene.meshes {
        let mut triangles = Vec::new();
        let mut chunks = Vec::new();
        let id = mesh.element.typed_id;
        if required_meshes
            .as_ref()
            .is_some_and(|ids| !ids.contains(&id))
        {
            continue;
        }
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

    // Tracking a label from an unused lower LOD can keep the main world loading
    // after the asset server drops that material and its texture dependencies.
    let needed: HashSet<_> = scene
        .nodes
        .iter()
        .filter(|node| {
            profiles.is_empty()
                || profiles
                    .iter()
                    .any(|p| p.nodes.contains(&node.element.typed_id))
        })
        .filter_map(|node| {
            let mesh = node.mesh.as_ref()?;
            Some(
                parts
                    .get(&mesh.element.typed_id)?
                    .iter()
                    .map(move |part| material_id(node, mesh, part.slot)),
            )
        })
        .flatten()
        .collect();
    let mut materials = HashMap::new();
    for material in &scene.materials {
        let typed_id = material.element.typed_id;
        let id = Some(typed_id);
        if !needed.contains(&id) {
            continue;
        }
        let name = &*material.element.name;
        let mut surface = match clinic_materials {
            Some(palette) => palette
                .get(name)
                .ok_or_else(|| invalid(format!("Missing clinic room material {name}")))?
                .to_material(context),
            None => standard_material(material, textures),
        };
        if !profiles.is_empty() {
            hospital_material(&mut surface, name, context);
        }
        materials.insert(id, label(context, format!("Material{typed_id}"), surface));
    }
    if needed.contains(&None) {
        let fallback = StandardMaterial {
            base_color: Color::srgb(0.78, 0.78, 0.78),
            ..default()
        };
        materials.insert(None, label(context, "DefaultMaterial".into(), fallback));
    }

    for (index, profile) in profiles.iter().enumerate().skip(1) {
        let asset = world_asset(scene, &parts, &materials, Some(profile))?;
        context.add_labeled_asset(format!("Scene{index}"), asset);
    }
    world_asset(scene, &parts, &materials, profiles.first())
}

fn material_id(node: &ufbx::Node, mesh: &ufbx::Mesh, slot: usize) -> Option<u32> {
    node.materials
        .get(slot)
        .or_else(|| mesh.materials.get(slot))
        .map(|material| material.element.typed_id)
}

fn world_asset(
    scene: &ufbx::Scene,
    parts: &HashMap<u32, Vec<Part>>,
    materials: &HashMap<Option<u32>, Handle<StandardMaterial>>,
    profile: Option<&ImportProfile>,
) -> io::Result<WorldAsset> {
    let mut world = World::new();
    let import_root = profile.map(|profile| {
        world
            .spawn((profile.transform(), Visibility::Inherited))
            .id()
    });
    let mut nodes = HashMap::new();
    let mut mesh_count = 0;
    for node in &scene.nodes {
        let name = &*node.element.name;
        let id = world
            .spawn((
                Name::new(if name.is_empty() { "FBX" } else { name }.to_owned()),
                transform(node.local_transform)?,
                if profile.is_some() || node.visible || node.is_root {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
            ))
            .id();
        nodes.insert(node.element.typed_id, id);
        if profile.is_none_or(|profile| profile.nodes.contains(&node.element.typed_id))
            && let Some(mesh) = &node.mesh
            && let Some(chunks) = parts.get(&mesh.element.typed_id)
        {
            let parent = if let Some(adjustment) =
                profile.and_then(|p| p.adjustments.get(&node.element.typed_id))
            {
                // The original assemblies store several articulated parts in a showroom pose.
                // Put the fitted placement around the original node-to-world matrix.
                let correction = world
                    .spawn((adjustment.transform(), Visibility::Inherited))
                    .id();
                let geometry = geometry_matrix(node);
                world
                    .spawn((
                        Transform::from_matrix(geometry),
                        Visibility::Inherited,
                        ChildOf(correction),
                    ))
                    .id()
            } else {
                id
            };
            for part in chunks {
                // FBX material slots are local to the node instance.
                let material = materials
                    .get(&material_id(node, mesh, part.slot))
                    .ok_or_else(|| invalid("Missing FBX material for a selected mesh part"))?;
                world.spawn((
                    Mesh3d(part.mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    part.bounds,
                    Transform::IDENTITY,
                    ChildOf(parent),
                ));
                mesh_count += 1;
            }
        }
    }
    if mesh_count == 0 {
        return Err(invalid("В FBX нет полигональных поверхностей в узлах"));
    }
    for node in &scene.nodes {
        if node.parent.is_none()
            && let Some(root) = import_root
        {
            world
                .entity_mut(nodes[&node.element.typed_id])
                .insert(ChildOf(root));
        }
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
    texture_path(source, &file.relative_filename)
        .or_else(|| texture_path(source, &file.filename))
        .or_else(|| {
            // FBX exporters can store an old absolute path even when the texture travels with the model.
            let name = file
                .relative_filename
                .rsplit(['/', '\\'])
                .next()
                .filter(|name| !name.is_empty())
                .or_else(|| file.filename.rsplit(['/', '\\']).next())?;
            texture_path(source, name)
        })
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

pub(crate) fn scalar(map: &ufbx::MaterialMap, fallback: f32) -> f32 {
    if map.has_value {
        channel(map.value_vec4.x, fallback)
    } else {
        fallback
    }
}

pub(crate) fn rgb(map: &ufbx::MaterialMap, fallback: f32) -> [f32; 3] {
    if map.has_value {
        let v = map.value_vec4;
        [v.x, v.y, v.z].map(|value| channel(value, fallback))
    } else {
        [fallback; 3]
    }
}

fn hospital_material(material: &mut StandardMaterial, name: &str, context: &mut LoadContext<'_>) {
    const ATLAS: &str = "textures/hospital/Shaders/Textures/Atlas_01_Color.png";
    const CEILING: &str = "textures/hospital/Shaders/Textures/Ceiling_Tile_Color.png";
    const CEILING_EMISSION: &str = "textures/hospital/Shaders/Textures/Ceiling_Tile_Emission.png";
    let name = name.to_ascii_lowercase();
    if name.starts_with("lightbeam") {
        material.base_color = Color::srgba(0.92, 0.94, 1.0, 0.045);
        material.alpha_mode = AlphaMode::Add;
        material.cull_mode = None;
        material.double_sided = true;
        material.unlit = true;
        return;
    }
    let texture = if name.starts_with("atlas") {
        material.metallic_roughness_texture =
            Some(context.load("textures/hospital/pbr/atlas_metallic_roughness.png"));
        material.metallic = 1.0;
        material.perceptual_roughness = 1.0;
        Some(ATLAS)
    } else if name == "ceiling_tile" || name == "ceiling_lamp" {
        if name == "ceiling_lamp" {
            material.emissive_texture = Some(context.load(CEILING_EMISSION));
            material.emissive = LinearRgba::rgb(1.8, 1.75, 1.65);
        }
        Some(CEILING)
    } else if name == "painting" || name.starts_with("painting_") {
        let asset = context.path().path().file_stem().and_then(|s| s.to_str());
        Some(match asset {
            Some("Painting_2") => "textures/hospital/Shaders/Textures/Painting_2.png",
            Some("Wall_Painting") => "textures/hospital/Shaders/Textures/Wall_Painting.png",
            _ => "textures/hospital/Shaders/Textures/Painting_1.png",
        })
    } else {
        if name == "light" || name.starts_with("lamp") {
            material.emissive = LinearRgba::rgb(1.8, 1.65, 1.35);
        }
        None
    };
    if let Some(texture) = texture {
        material.base_color = Color::WHITE;
        material.base_color_texture = Some(context.load(texture));
        material.alpha_mode = AlphaMode::Opaque;
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
