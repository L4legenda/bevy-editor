//! Import static OBJ and FBX meshes into the editable GLB format.
use super::{
    model_document::{MaterialSlot, Object, glb::Glb},
    model_mesh::{Corner, Face, Geometry},
    model_paint_data::texture_fill::{Bitmap, encode_png},
    model_uv_backend as uv_backend, model_uv_mesh as uv_mesh,
};
use crate::runtime::fbx::{geometry_matrix, load_mesh_file, load_obj_with_materials, rgb, scalar};
use bevy::prelude::*;
use bevy_scene_format::{Revision, atomic_write};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_VERTICES: usize = 250_000;
const MAX_CORNERS: usize = 2_000_000;
const MAX_TEXTURE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AutoUnwrap {
    Off,
    #[default]
    Missing,
    All,
}

impl AutoUnwrap {
    pub fn next(self) -> Self {
        match self {
            Self::Missing => Self::All,
            Self::All => Self::Off,
            Self::Off => Self::Missing,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Missing => "Если нет UV",
            Self::All => "Всегда",
            Self::Off => "Выключена",
        }
    }
}

pub(super) struct ImportedMesh {
    pub name: PathBuf,
    pub warnings: Vec<String>,
    pub unwrapped_sets: usize,
}

pub(super) fn convert(
    path: &Path,
    target: &Path,
    name: &str,
    auto_unwrap: AutoUnwrap,
) -> Result<ImportedMesh, String> {
    let (objects, images, unwrapped_sets) = read(path, name, auto_unwrap, true)?;
    let bytes = Glb::new_asset_with_pngs(&objects, &images.pngs)?;
    if bytes.len() > 512 * 1024 * 1024 {
        return Err("Преобразованный GLB превышает 512 МиБ".into());
    }
    for suffix in 1..10_000 {
        let file = if suffix == 1 {
            format!("{name}.glb")
        } else {
            format!("{name} ({suffix}).glb")
        };
        let destination = target.join(&file);
        if std::fs::symlink_metadata(&destination).is_ok() {
            continue;
        }
        match atomic_write(&destination, &bytes, Revision::Missing) {
            Ok(_) => {
                return Ok(ImportedMesh {
                    name: PathBuf::from(file),
                    warnings: images.warnings,
                    unwrapped_sets,
                });
            }
            Err(_) if std::fs::symlink_metadata(&destination).is_ok() => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("Не удалось подобрать имя для импортированной модели".into())
}

pub(super) fn replacement(path: &Path, auto_unwrap: AutoUnwrap) -> Result<Vec<Object>, String> {
    if !path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("fbx") || extension.eq_ignore_ascii_case("obj")
    }) {
        return Err("Для замены сетки выберите OBJ или FBX".into());
    }
    let name = path.file_stem().and_then(|name| name.to_str()).unwrap_or("Модель");
    let (objects, images, _) = read(path, name, auto_unwrap, false)?;
    if let Some(error) = images.warnings.into_iter().next() {
        return Err(error);
    }
    Ok(objects)
}

fn read(
    path: &Path,
    name: &str,
    auto_unwrap: AutoUnwrap,
    include_textures: bool,
) -> Result<(Vec<Object>, Images, usize), String> {
    let is_fbx = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("fbx"));
    let format = if is_fbx { "FBX" } else { "OBJ" };
    let (scene, material_folder) = if is_fbx {
        load_mesh_file(path, ufbx::FileFormat::Fbx).map(|scene| (scene, None))
    } else {
        load_obj_with_materials(path)
    }
    .map_err(|error| error.to_string())?;
    let folder = path.parent().ok_or("Нет папки исходной модели")?;
    let folders = [material_folder.as_deref().unwrap_or(folder), folder];
    let mut images = Images::default();
    let mut objects = Vec::new();
    let mut vertex_count = 0;
    let mut corner_count = 0;
    let mut unwrapped_sets = 0;
    for node in &scene.nodes {
        let Some(mesh) = &node.mesh else { continue };
        if mesh.num_triangles == 0 {
            continue;
        }
        corner_count += mesh.num_indices;
        if corner_count > MAX_CORNERS {
            return Err(format!("Слишком много углов в {format}"));
        }
        let matrix = geometry_matrix(node);
        let normal_matrix = Mat3::from_mat4(matrix);
        let determinant = normal_matrix.determinant();
        if !matrix.is_finite() || !determinant.is_finite() || determinant.abs() <= 1e-12 {
            return Err(format!("Некорректное преобразование объекта {format}"));
        }
        let normal_matrix = normal_matrix.inverse().transpose();
        let mut geometry = Geometry::default();
        let mut vertices = HashMap::new();
        let mut material_indices = HashMap::new();
        let mut materials = Vec::new();
        let mut missing_uv = BTreeSet::new();
        for (index, face) in mesh.faces.iter().enumerate() {
            if face.num_indices < 3 {
                continue;
            }
            if face.num_indices > 4096 {
                return Err(format!("Грань {format} содержит больше 4096 вершин"));
            }
            let source_slot = mesh.face_material.get(index).copied().unwrap_or_default() as usize;
            let material = node
                .materials
                .get(source_slot)
                .or_else(|| mesh.materials.get(source_slot));
            let key = material.map(|material| material.element.typed_id);
            let slot = *material_indices.entry(key).or_insert_with(|| {
                let slot = materials.len();
                materials.push(images.material(
                    &scene,
                    folders,
                    material.map(|v| &**v),
                    mesh,
                    format,
                    include_textures,
                ));
                slot
            });
            let start = face.index_begin as usize;
            let mut corners: Vec<_> = (start..start + face.num_indices as usize)
                .map(|i| {
                    let vertex = *vertices.entry(mesh.vertex_indices[i]).or_insert_with(|| {
                        let index = geometry.positions.len();
                        geometry
                            .positions
                            .push(matrix.transform_point3(xyz(mesh.vertex_position[i])));
                        index
                    });
                    let mut corner = Corner::new(vertex);
                    if mesh.vertex_uv.exists
                        && let Some(uv) = mesh
                            .vertex_uv
                            .indices
                            .get(i)
                            .and_then(|&index| mesh.vertex_uv.values.get(index as usize))
                    {
                        corner.uv = Vec2::new(uv.x as f32, 1. - uv.y as f32);
                        corner.uv1 = corner.uv;
                    } else {
                        missing_uv.insert(slot);
                    }
                    if let Some(uv) = mesh.uv_sets.get(1).map(|set| &set.vertex_uv)
                        && let Some(uv) = uv
                            .indices
                            .get(i)
                            .and_then(|&index| uv.values.get(index as usize))
                    {
                        corner.uv1 = Vec2::new(uv.x as f32, 1. - uv.y as f32);
                    }
                    if mesh.vertex_normal.exists
                        && let Some(&normal) = mesh
                            .vertex_normal
                            .indices
                            .get(i)
                            .and_then(|&index| mesh.vertex_normal.values.get(index as usize))
                    {
                        corner.source_normal = (normal_matrix * xyz(normal)).try_normalize();
                    }
                    if mesh.vertex_color.exists
                        && let Some(color) = mesh
                            .vertex_color
                            .indices
                            .get(i)
                            .and_then(|&index| mesh.vertex_color.values.get(index as usize))
                    {
                        corner.color = Vec4::new(
                            color.x as f32,
                            color.y as f32,
                            color.z as f32,
                            color.w as f32,
                        );
                        corner.id_color = Some(corner.color.truncate());
                    }
                    corner
                })
                .collect();
            if determinant < 0. {
                corners.reverse();
            }
            geometry.faces.push(Face {
                corners,
                material: slot,
                smooth: mesh.face_smoothing.get(index).copied().unwrap_or(false),
            });
        }
        vertex_count += geometry.positions.len();
        if vertex_count > MAX_VERTICES {
            return Err(format!(
                "Для редактирования {format} поддерживается до 250 000 вершин"
            ));
        }
        geometry.validate()?;
        let object_name = [&*node.element.name, &*mesh.element.name, name]
            .into_iter()
            .find(|name| !name.is_empty())
            .unwrap_or(format);
        if mesh.vertex_uv.exists {
            for face in &geometry.faces {
                if !missing_uv.contains(&face.material)
                    && geometry.triangles(face).iter().any(|triangle| {
                        let [a, b, c] = triangle.map(|index| face.corners[index].uv);
                        (b - a).perp_dot(c - a).abs() <= f32::EPSILON * f32::EPSILON
                    })
                {
                    missing_uv.insert(face.material);
                }
            }
        }
        let unwrap = match auto_unwrap {
            AutoUnwrap::Off if !missing_uv.is_empty() => {
                return Err(format!(
                    "{format} «{object_name}» · отсутствуют UV. Включите авторазвёртку в настройках импорта"
                ));
            }
            AutoUnwrap::All => geometry.faces.iter().map(|face| face.material).collect(),
            _ => missing_uv,
        };
        if !unwrap.is_empty() {
            match unwrap_materials(&mut geometry, &unwrap, mesh.uv_sets.len() < 2) {
                Ok(()) => unwrapped_sets += unwrap.len(),
                Err(error) => {
                    let message = format!("{format} «{object_name}» · не удалось построить UV: {error}");
                    if auto_unwrap == AutoUnwrap::All {
                        return Err(message);
                    }
                    images.warnings.push(message);
                }
            }
        }
        objects.push(Object {
            id: objects.len(),
            source_node: None,
            name: object_name.to_owned(),
            geometry: geometry.into(),
            transform: Transform::IDENTITY,
            materials,
            hidden: false,
        });
    }
    if objects.is_empty() {
        return Err(format!("В {format} нет полигональных поверхностей"));
    }
    Ok((objects, images, unwrapped_sets))
}

fn unwrap_materials(
    geometry: &mut Geometry,
    materials: &BTreeSet<usize>,
    copy_to_uv1: bool,
) -> Result<(), String> {
    let mut groups = BTreeMap::<usize, Vec<&Face>>::new();
    for face in &geometry.faces {
        if materials.contains(&face.material) {
            groups.entry(face.material).or_default().push(face);
        }
    }
    let mut unwrapped = Vec::new();
    for faces in groups.into_values() {
        // Each material has its own texture space. Pack its charts independently.
        let mut part = Geometry::default();
        let mut local = HashMap::new();
        let mut original = Vec::new();
        for face in faces {
            let mut face = face.clone();
            for corner in &mut face.corners {
                corner.vertex = *local.entry(corner.vertex).or_insert_with(|| {
                    let id = part.positions.len();
                    part.positions.push(geometry.positions[corner.vertex]);
                    original.push(corner.vertex);
                    id
                });
            }
            part.faces.push(face);
        }
        let input = uv_mesh::input(&part, 1.25, 2048, 8)?;
        let output = uv_backend::run(input.request, Arc::new(uv_backend::Progress::default()))?;
        let result = uv_mesh::apply(&part, &input.corners, &output)?;
        unwrapped.extend(result.faces.into_iter().map(|mut face| {
            for corner in &mut face.corners {
                corner.vertex = original[corner.vertex];
                if copy_to_uv1 {
                    corner.uv1 = corner.uv;
                }
            }
            face
        }));
    }
    geometry
        .faces
        .retain(|face| !materials.contains(&face.material));
    geometry.faces.extend(unwrapped);
    Ok(())
}

fn xyz(v: ufbx::Vec3) -> Vec3 {
    Vec3::new(v.x as f32, v.y as f32, v.z as f32)
}

#[derive(Default)]
struct Images {
    pngs: Vec<Vec<u8>>,
    indices: HashMap<TextureSource, (usize, bool)>,
    used: usize,
    warnings: Vec<String>,
}

#[derive(Eq, Hash, PartialEq)]
enum TextureSource {
    File(PathBuf),
    Embedded(u32),
}

impl Images {
    fn material(
        &mut self,
        scene: &ufbx::Scene,
        folders: [&Path; 2],
        material: Option<&ufbx::Material>,
        mesh: &ufbx::Mesh,
        format: &str,
        include_textures: bool,
    ) -> MaterialSlot {
        let authored = material.map(|material| {
            let pbr = &material.pbr;
            let mut texture = |map| {
                if include_textures { self.texture(scene, folders, map, mesh, format) } else { None }
            };
            let base = texture(&pbr.base_color);
            let normal = texture(&pbr.normal_map);
            let emission = texture(&pbr.emission_color);
            let alpha = scalar(&pbr.opacity, 1.);
            let base_factor = scalar(&pbr.base_factor, 1.);
            let [r, g, b] = rgb(&pbr.base_color, if base.is_some() { 1. } else { 0.72 })
                .map(|value| value * base_factor);
            let mut definition = json!({
                "name": &*material.element.name,
                "doubleSided": true,
                "pbrMetallicRoughness": {
                    "baseColorFactor": [r, g, b, alpha],
                    "metallicFactor": scalar(&pbr.metalness, 0.),
                    "roughnessFactor": scalar(&pbr.roughness, 0.75),
                },
            });
            if let Some((binding, translucent)) = base {
                definition["pbrMetallicRoughness"]["baseColorTexture"] = binding;
                if translucent {
                    definition["alphaMode"] = json!("BLEND");
                }
            }
            if alpha < 1. {
                definition["alphaMode"] = json!("BLEND");
            }
            if let Some((binding, _)) = normal {
                definition["normalTexture"] = binding;
            }
            if let Some((binding, _)) = &emission {
                definition["emissiveTexture"] = binding.clone();
            }
            let emission_color = rgb(&pbr.emission_color, f32::from(u8::from(emission.is_some())));
            if emission_color != [0.; 3] {
                definition["emissiveFactor"] = json!(emission_color);
            }
            Arc::new(definition)
        });
        MaterialSlot {
            bake_fixes: Default::default(),
            material: default(),
            source: if include_textures { json!({"material":0}) } else { json!({}) },
            paint: None,
            authored,
        }
    }

    fn texture(
        &mut self,
        scene: &ufbx::Scene,
        folders: [&Path; 2],
        map: &ufbx::MaterialMap,
        mesh: &ufbx::Mesh,
        format: &str,
    ) -> Option<(serde_json::Value, bool)> {
        let material_texture = map.texture.as_ref()?;
        let texture = material_texture.file_textures.first()?;
        if !texture.has_file {
            return None;
        }
        let uv_set = if material_texture.uv_set.is_empty() {
            &*texture.uv_set
        } else {
            &*material_texture.uv_set
        };
        let tex_coord = mesh
            .uv_sets
            .get(1)
            .is_some_and(|set| !uv_set.is_empty() && &*set.name == uv_set);
        let embedded = scene
            .texture_files
            .get(texture.file_index as usize)
            .filter(|file| !file.content.is_empty());
        let result = (|| -> Result<(usize, bool), String> {
            let source = if embedded.is_some() {
                TextureSource::Embedded(texture.file_index)
            } else {
                let basename = texture
                    .relative_filename
                    .rsplit(['/', '\\'])
                    .next()
                    .filter(|name| !name.is_empty())
                    .or_else(|| texture.filename.rsplit(['/', '\\']).next())
                    .unwrap_or("");
                let fallback = if format == "FBX" { basename } else { "" };
                let references = [&*texture.filename, &*texture.relative_filename, fallback];
                let path = folders
                    .into_iter()
                    .flat_map(|folder| {
                        references
                            .iter()
                            .filter(|name| !name.is_empty())
                            .filter_map(move |name| {
                                folder.join(name.replace('\\', "/")).canonicalize().ok()
                            })
                    })
                    .find(|path| path.starts_with(folders[1]) && path.is_file())
                    .ok_or("файл отсутствует в папке модели")?;
                TextureSource::File(path)
            };
            if let Some(&index) = self.indices.get(&source) {
                return Ok(index);
            }
            let bitmap = match &source {
                TextureSource::File(path) => Bitmap::open(path)?,
                TextureSource::Embedded(_) => Bitmap::decode(
                    texture.relative_filename.to_string(),
                    &embedded.unwrap().content,
                )?,
            };
            let encoded = encode_png(bitmap.pixels())?;
            if self.used + encoded.len() > MAX_TEXTURE_BYTES {
                return Err("превышен общий лимит текстур в 128 МиБ".into());
            }
            let index = (self.pngs.len(), bitmap.has_alpha());
            self.indices.insert(source, index);
            self.used += encoded.len();
            self.pngs.push(encoded);
            Ok(index)
        })();
        match result {
            Ok((index, translucent)) => Some((
                json!({"index":index,"texCoord":u8::from(tex_coord)}),
                translucent,
            )),
            Err(error) => {
                let name = Path::new(&*texture.relative_filename)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                let warning = format!("Текстура {format} «{name}» · {error}");
                if !self.warnings.contains(&warning) {
                    self.warnings.push(warning);
                }
                None
            }
        }
    }
}
