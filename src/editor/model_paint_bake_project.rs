//! Projects high-poly source geometry onto the low-poly UV raster.
use super::super::{IdSource, ProjectionFormat, material_id_color};
use super::{Progress, ProjectionSettings, triangle_id_color};
use crate::editor::model_document::{Object, glb::{read_id_colors, read_source_parts}};
use crate::editor::model_mesh::Face;
use crate::editor::ui::model_viewer::editing::input::{Bvh, PickTriangle};
use bevy::{gltf::gltf, prelude::*};
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    path::Path,
    sync::{Arc, Mutex},
};

#[path = "model_paint_bake_cage.rs"]
mod cage;
#[path = "model_paint_bake_fit.rs"]
mod fit;
#[path = "model_paint_bake_high_relief.rs"]
mod high_relief;
pub(super) use high_relief::HighRelief;
#[path = "model_paint_bake_source.rs"]
mod source;
pub(super) use cage::Cage;
pub(super) use cage::{CageRay, CageTriangle};
pub(in crate::editor::ui::model_viewer::paint::bake) use fit::FitReport;

struct SourceTriangle {
    points: [Vec3; 3],
    normals: [Vec3; 3],
    part: usize,
    material: Option<u32>,
    color: Option<[u8; 3]>,
    usage: SourceUse,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceUse {
    Omit,
    Project(u32),
    Occluder,
}
impl SourceUse {
    fn projection_group(self) -> u32 {
        match self {
            Self::Project(group) => group,
            _ => 0,
        }
    }

    fn projectable(self) -> bool {
        self.projection_group() != 0
    }
}

impl From<bool> for SourceUse {
    fn from(included: bool) -> Self {
        if included { Self::Project(1) } else { Self::Omit }
    }
}

impl SourceTriangle {
    fn new(points: [Vec3; 3], normals: [Vec3; 3], part: usize) -> Option<Self> {
        let [a, b, c] = points;
        let geo = (b - a).cross(c - a).try_normalize()?;
        Some(Self {
            points,
            normals: normals.map(|normal| normal.try_normalize().unwrap_or(geo)),
            part,
            material: None,
            color: None,
            usage: SourceUse::Project(1),
        })
    }

    fn with_usage(mut self, usage: SourceUse) -> Self {
        self.usage = usage;
        self
    }

    fn with_id(mut self, material: Option<u32>, color: Option<[u8; 3]>) -> Self {
        self.material = material;
        self.color = color;
        self
    }
}

struct SourceGeometry {
    bvh: Bvh,
    triangles: Vec<SourceTriangle>,
    material_colors: HashMap<Option<u32>, [u8; 3]>,
    relief: Mutex<high_relief::Cache>,
    groups: HashMap<String, u32>,
    grouped: bool,
}

#[derive(PartialEq, Eq)]
struct SourceKey {
    hash: blake3::Hash,
    match_names: Vec<String>,
}

const CACHED_SOURCES: usize = 4;
const CACHED_TRIANGLES: usize = 1_000_000;
// The GPU stores the group in the high 30 bits of the query flags.
const UNMATCHED_SOURCE_GROUP: u32 = u32::MAX >> 2;
static SOURCES: Mutex<VecDeque<(SourceKey, Arc<SourceGeometry>)>> = Mutex::new(VecDeque::new());

pub(super) struct Projection {
    source: Arc<SourceGeometry>,
    front: f32,
    rear: f32,
    fill_misses: bool,
    prefer_front_faces: bool,
    id_source: IdSource,
}

pub(super) struct Segment {
    pub origin: Vec3,
    pub outward: Vec3,
    pub front: f32,
    pub rear: f32,
    pub range: f32,
    pub group: u32,
}

pub(super) struct Query {
    pub segment: Segment,
    pub point: Vec3,
    pub normal: Vec3,
    pub fill: bool,
    pub prefer_front_faces: bool,
}

#[derive(Clone, Copy)]
pub(super) struct Hit {
    pub point: Vec3,
    pub triangle: usize,
    pub weights: Vec3,
    pub normal: Vec3,
    pub geometric: Vec3,
    pub height: f32,
    pub ray_hit: bool,
}

impl Projection {
    pub(super) fn paint_cage(
        &self,
        base: Option<&Cage>,
        directions: &[Vec3],
        overrides: &[f32],
        span: f32,
        progress: &Progress,
    ) -> Result<Option<Cage>, String> {
        Cage::apply_paint(base, directions, overrides, [self.front, self.rear], span, progress)
    }
    #[cfg(test)]
    pub(super) fn load(
        path: &Path,
        name: &str,
        scale: f32,
        settings: &ProjectionSettings,
        progress: &Progress,
    ) -> Result<Self, String> {
        let names = settings.match_names.then(|| base_name(name)).into_iter().collect();
        Self::load_filtered(path, name, names, scale, settings, progress)
    }

    pub(super) fn load_for_object(
        path: &Path, object: &Object, scale: f32, settings: &ProjectionSettings, progress: &Progress,
    ) -> Result<Self, String> {
        let name = object.bake_source_name();
        let names = if settings.match_names { part_names(object, progress)? } else { Vec::new() };
        Self::load_filtered(path, name, names, scale, settings, progress)
    }

    fn load_filtered(
        path: &Path, name: &str, match_names: Vec<String>, scale: f32,
        settings: &ProjectionSettings, progress: &Progress,
    ) -> Result<Self, String> {
        if match_names.len() >= UNMATCHED_SOURCE_GROUP as usize {
            return Err("Слишком много именованных частей сетки".into());
        }
        #[cfg(test)]
        let key = if progress.fresh_high_source {
            None
        } else {
            super::super::disk::source_hash(path, progress).map(|hash| SourceKey {
                hash,
                match_names: match_names.clone(),
            })
        };
        #[cfg(not(test))]
        let key = super::super::disk::source_hash(path, progress).map(|hash| SourceKey {
            hash,
            match_names: match_names.clone(),
        });
        progress.check()?;
        if let Some(key) = &key {
            let mut sources = SOURCES.lock().unwrap();
            if let Some(entry) = sources
                .iter()
                .position(|(cached, _)| cached == key)
                .and_then(|index| sources.remove(index))
            {
                let projection = Self::with_source(entry.1.clone(), scale, settings);
                sources.push_back(entry);
                #[cfg(test)]
                progress
                    .high_source_hit
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                return Ok(projection);
            }
        }
        let mut groups: HashMap<_, _> = match_names.into_iter().enumerate()
            .map(|(index, name)| (name, index as u32 + 1)).collect();
        let grouped = groups.len() > 1;
        let mut triangles = read_triangles(
            path,
            |name| {
                if groups.is_empty() {
                    SourceUse::Project(1)
                } else {
                    groups.get(&base_name(name)).copied().map_or(SourceUse::Occluder, SourceUse::Project)
                }
            },
            progress,
        )?;
        let matched: BTreeSet<_> = triangles.iter().map(|triangle| triangle.usage.projection_group()).collect();
        let expected_count = groups.len();
        groups.retain(|_, group| matched.contains(group));
        if grouped && groups.len() < expected_count {
            // A low-poly part without a name match can project onto the whole source.
            for triangle in &mut triangles {
                if !triangle.usage.projectable() {
                    triangle.usage = SourceUse::Project(UNMATCHED_SOURCE_GROUP);
                }
            }
        }
        if !triangles.iter().any(|triangle| triangle.usage.projectable()) {
            return Err(if settings.match_names {
                format!("В high-poly нет треугольников для «{name}» · проверьте имена объектов")
            } else {
                "В high-poly нет треугольников".into()
            });
        }
        progress.check()?;
        let projection = Self::new_grouped(triangles, groups, grouped, scale, settings);
        if let Some(key) = key {
            let current = super::super::disk::source_hash(path, progress);
            progress.check()?;
            if current != Some(key.hash) {
                return Err("Файл high-poly изменился во время чтения".into());
            }
            let incoming = projection.source.triangles.len();
            if incoming <= CACHED_TRIANGLES {
                let mut sources = SOURCES.lock().unwrap();
                if let Some(existing) = sources
                    .iter()
                    .position(|(cached, _)| cached == &key)
                    .and_then(|index| sources.remove(index))
                {
                    let projection = Self::with_source(existing.1.clone(), scale, settings);
                    sources.push_back(existing);
                    return Ok(projection);
                }
                while sources.len() >= CACHED_SOURCES
                    || incoming
                        + sources
                            .iter()
                            .map(|(_, source)| source.triangles.len())
                            .sum::<usize>()
                        > CACHED_TRIANGLES
                {
                    sources.pop_front();
                }
                sources.push_back((key, projection.source.clone()));
            }
        }
        Ok(projection)
    }

    #[cfg(test)]
    fn new(triangles: Vec<SourceTriangle>, scale: f32, settings: &ProjectionSettings) -> Self {
        Self::new_grouped(triangles, HashMap::new(), false, scale, settings)
    }

    fn new_grouped(
        triangles: Vec<SourceTriangle>, groups: HashMap<String, u32>, grouped: bool,
        scale: f32, settings: &ProjectionSettings,
    ) -> Self {
        let materials: BTreeSet<_> = triangles.iter().map(|triangle| triangle.material).collect();
        let material_colors = if materials.len() > 1 {
            materials.into_iter().enumerate()
                .map(|(index, material)| (material, material_id_color(index as u32)))
                .collect()
        } else {
            HashMap::new()
        };
        let bvh = Bvh::from_triangles(
            triangles
                .iter()
                .enumerate()
                .map(|(i, t)| PickTriangle {
                    object: t.part,
                    face: i,
                    points: t.points,
                })
                .collect(),
        );
        Self::with_source(
            Arc::new(SourceGeometry {
                bvh,
                triangles,
                material_colors,
                relief: Mutex::default(),
                groups,
                grouped,
            }),
            scale,
            settings,
        )
    }

    fn with_source(source: Arc<SourceGeometry>, scale: f32, settings: &ProjectionSettings) -> Self {
        Self {
            source,
            front: scale * f32::from(settings.front) / 100.,
            rear: scale * f32::from(settings.rear) / 100.,
            fill_misses: settings.fill_misses,
            prefer_front_faces: settings.prefer_front_faces,
            id_source: settings.id_source,
        }
    }

    pub(super) fn scene(&self) -> &Bvh {
        &self.source.bvh
    }

    pub(super) fn id_color(&self, triangle: usize) -> Option<[u8; 3]> {
        match self.id_source {
            IdSource::MeshParts => None,
            IdSource::Materials => self.source.material_colors.get(&self.source.triangles[triangle].material).copied(),
            IdSource::VertexColors => self.source.triangles[triangle].color,
        }
    }

    pub(super) fn surface_normal(&self, triangle: usize, point: Vec3) -> Vec3 {
        self.hit(triangle, point, 0., true).normal
    }

    pub(super) fn corner_normals(&self, triangle: usize) -> [Vec3; 3] {
        self.source.triangles[triangle].normals
    }

    pub(super) fn face_groups(&self, object: &Object) -> Vec<u32> {
        if !self.source.grouped {
            return Vec::new();
        }
        let mut by_name = HashMap::new();
        object.geometry.faces.iter().map(|face| {
            let name = part_name(object, face);
            *by_name.entry(name).or_insert_with(|| {
                self.source.groups.get(&base_name(name)).copied().unwrap_or(0)
            })
        }).collect()
    }

    pub(super) fn projection_group(&self, triangle: &PickTriangle) -> u32 {
        self.source.triangles[triangle.face].usage.projection_group()
    }

    pub(super) fn vertex_groups(&self, object: &Object) -> Vec<Vec<u32>> {
        if !self.source.grouped {
            return Vec::new();
        }
        let mut groups = vec![Vec::new(); object.geometry.positions.len()];
        for (face, group) in object.geometry.faces.iter().zip(self.face_groups(object)) {
            for corner in &face.corners {
                let entry = &mut groups[corner.vertex];
                if !entry.contains(&group) {
                    entry.push(group);
                }
            }
        }
        groups
    }

    fn can_project(&self, triangle: &PickTriangle, group: u32) -> bool {
        let source = self.projection_group(triangle);
        source != 0 && (group == 0 || group == source)
    }

    pub(super) fn query(
        &self,
        point: Vec3,
        direction: Vec3,
        normal: Vec3,
        cage: Option<CageRay>,
        fill: bool,
        group: u32,
    ) -> Query {
        Query {
            segment: self.segment(point, direction, cage, group),
            point,
            normal,
            fill: fill && self.fill_misses,
            prefer_front_faces: self.prefer_front_faces,
        }
    }

    pub(super) fn segment(&self, point: Vec3, direction: Vec3, cage: Option<CageRay>, group: u32) -> Segment {
        let CageRay {
            offset,
            reach,
            scale,
            max_scale,
        } = cage.unwrap_or(CageRay {
            offset: direction * self.front,
            reach: self.front,
            scale: 1.,
            max_scale: 1.,
        });
        Segment {
            origin: point + offset,
            outward: offset.try_normalize().unwrap_or(direction),
            front: offset.length(),
            rear: self.rear * scale,
            range: reach.max(self.rear * max_scale),
            group,
        }
    }

    pub(super) fn cast(&self, segment: &Segment) -> Option<(usize, f32)> {
        let Segment {
            origin,
            outward,
            front,
            rear,
            group,
            ..
        } = *segment;
        let nearest = |filter_backfaces: bool| {
            self.source
                .bvh
                .cast_triangle_up_to(origin, -outward, front + rear, |triangle| {
                    let [a, b, c] = triangle.points;
                    self.can_project(triangle, group)
                        && (!filter_backfaces || (b - a).cross(c - a).dot(outward) > 0.)
                })
        };
        if self.prefer_front_faces {
            nearest(true).or_else(|| nearest(false))
        } else {
            nearest(false)
        }
    }

    pub(super) fn closest_crossing(
        &self,
        position: Vec3,
        direction: Vec3,
        reach: f32,
        bias: f32,
        group: u32,
    ) -> Option<(usize, f32)> {
        let crossing = |sign: f32| {
            let ray = direction * sign;
            self.source
                .bvh
                .cast_triangle_up_to(position - ray * bias, ray, reach + bias, |triangle| {
                    self.can_project(triangle, group)
                })
                .map(|(triangle, distance)| (triangle, (distance - bias) * sign))
        };
        [crossing(1.), crossing(-1.)]
            .into_iter()
            .flatten()
            .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
    }

    pub(super) fn sample(
        &self,
        point: Vec3,
        direction: Vec3,
        low_normal: Vec3,
        cage: Option<CageRay>,
        group: u32,
    ) -> Option<Hit> {
        self.direct(point, direction, cage, group)
            .or_else(|| self.fill(point, direction, low_normal, cage, group))
    }

    pub(super) fn direct(
        &self,
        point: Vec3,
        direction: Vec3,
        cage: Option<CageRay>,
        group: u32,
    ) -> Option<Hit> {
        let segment = self.segment(point, direction, cage, group);
        let (face, distance) = self.cast(&segment)?;
        Some(self.hit(
            face,
            segment.origin - segment.outward * distance,
            (segment.front - distance) / segment.range,
            true,
        ))
    }

    pub(super) fn fill(
        &self,
        point: Vec3,
        direction: Vec3,
        normal: Vec3,
        cage: Option<CageRay>,
        group: u32,
    ) -> Option<Hit> {
        if !self.fill_misses {
            return None;
        }
        let range = self.segment(point, direction, cage, group).range;
        let (face, hit) = self
            .source
            .bvh
            .nearest_surface_where(point, range * 4., |triangle| self.can_project(triangle, group))?;
        Some(self.hit(face, hit, (hit - point).dot(normal) / range, false))
    }

    pub(super) fn hit(&self, face: usize, hit: Vec3, height: f32, ray_hit: bool) -> Hit {
        let triangle = &self.source.triangles[face];
        let [a, b, c] = triangle.points;
        let cross = (b - a).cross(c - a);
        let v = (hit - a).cross(c - a).dot(cross) / cross.length_squared();
        let w = (b - a).cross(hit - a).dot(cross) / cross.length_squared();
        let [na, nb, nc] = triangle.normals;
        let normal = (na * (1. - v - w) + nb * v + nc * w)
            .try_normalize()
            .unwrap_or_else(|| cross.normalize());
        let geometric = cross.normalize() * if cross.dot(normal) < 0. { -1. } else { 1. };
        Hit {
            point: hit,
            triangle: face,
            weights: Vec3::new(1. - v - w, v, w),
            normal,
            geometric,
            height,
            ray_hit,
        }
    }
}

fn read_triangles(
    path: &Path,
    usage: impl Fn(&str) -> SourceUse,
    progress: &Progress,
) -> Result<Vec<SourceTriangle>, String> {
    match ProjectionFormat::of(path) {
        Some(ProjectionFormat::Glb) => glb_triangles(path, usage, progress),
        Some(ProjectionFormat::Obj) => {
            source::mesh_triangles(path, ufbx::FileFormat::Obj, usage, progress)
        }
        Some(ProjectionFormat::Fbx) => {
            source::mesh_triangles(path, ufbx::FileFormat::Fbx, usage, progress)
        }
        None => Err("Выберите GLB, OBJ или FBX".into()),
    }
}

fn glb_triangles(
    path: &Path,
    usage: impl Fn(&str) -> SourceUse,
    progress: &Progress,
) -> Result<Vec<SourceTriangle>, String> {
    let source = gltf::Gltf::open(path).map_err(|error| error.to_string())?;
    let blob = source
        .blob
        .as_deref()
        .ok_or("Выберите GLB со встроенной геометрией")?;
    let scene = source
        .default_scene()
        .or_else(|| source.scenes().next())
        .ok_or("В high-poly нет сцены")?;
    let mut triangles = Vec::new();
    let mut nodes: Vec<_> = scene.nodes().map(|node| (node, Mat4::IDENTITY)).collect();
    let mut next_part = source.nodes().count();
    while let Some((node, parent)) = nodes.pop() {
        progress.check()?;
        let transform = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        nodes.extend(node.children().map(|child| (child, transform)));
        let Some(mesh) = node.mesh() else { continue };
        let node_usage = usage(node.name().or_else(|| mesh.name()).unwrap_or_default());
        let mut named_parts = HashMap::new();
        let matrix = Mat3::from_mat4(transform);
        let determinant = matrix.determinant();
        if !determinant.is_finite() || determinant == 0. {
            continue;
        }
        let normal_matrix = matrix.inverse().transpose();
        let editor_topology = mesh.extras().as_ref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw.get()).ok())
            .is_some_and(|extras| extras["bevy_editor_topology"]["version"].as_u64() == Some(1));
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = primitive.reader(|buffer| {
                matches!(buffer.source(), gltf::buffer::Source::Bin).then_some(blob)
            });
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let points: Vec<_> = positions
                .map(|p| transform.transform_point3(p.into()))
                .collect();
            let normals: Vec<_> = reader
                .read_normals()
                .into_iter()
                .flatten()
                .map(|n| normal_matrix * Vec3::from(n))
                .collect();
            let id_colors = read_id_colors(&source.document, &primitive, blob, points.len())?;
            let source_parts = read_source_parts(&source.document, &primitive, blob, points.len())?;
            if source_parts.is_none() && node_usage == SourceUse::Omit {
                continue;
            }
            let visible_colors = if id_colors.is_none() {
                reader.read_colors(0).map(|colors| colors.into_rgba_f32().collect::<Vec<_>>())
                    .filter(|colors| !editor_topology || colors.iter().any(|rgba| *rgba != [1.; 4]))
            } else {
                None
            };
            let material = primitive.material().index().map(|index| index as u32);
            let indices: Box<dyn Iterator<Item = u32>> = match reader.read_indices() {
                Some(indices) => Box::new(indices.into_u32()),
                None => Box::new(0..points.len() as u32),
            };
            let mut indices = indices;
            while let (Some(a), Some(mut b), Some(mut c)) =
                (indices.next(), indices.next(), indices.next())
            {
                if triangles.len() % 4096 == 0 {
                    progress.check()?;
                }
                if determinant < 0. {
                    std::mem::swap(&mut b, &mut c);
                }
                let ids = [a as usize, b as usize, c as usize];
                let [Some(a), Some(b), Some(c)] = ids.map(|i| points.get(i).copied()) else {
                    return Err("В high-poly есть недействительный индекс вершины".into());
                };
                let (part, usage) = if let Some(names) = &source_parts
                    && let Some(name) = names[ids[0]].as_ref()
                    && !name.as_str().is_empty()
                    && ids[1..].iter().all(|&i| names[i].as_ref() == Some(name))
                {
                    *named_parts.entry(name.name.clone()).or_insert_with(|| {
                        let part = next_part;
                        next_part += 1;
                        (part, usage(name.as_str()))
                    })
                } else {
                    (node.index(), node_usage)
                };
                if usage == SourceUse::Omit {
                    continue;
                }
                let color = triangle_id_color(ids.map(|i| {
                    id_colors.as_ref().and_then(|colors| colors[i]).or_else(|| {
                        visible_colors.as_ref().and_then(|colors| colors.get(i))
                            .map(|rgba| Vec4::from_array(*rgba).truncate())
                    })
                }));
                triangles.extend(
                    SourceTriangle::new(
                        [a, b, c],
                        ids.map(|i| normals.get(i).copied().unwrap_or_default()),
                        part,
                    )
                    .map(|triangle| triangle.with_usage(usage).with_id(material, color)),
                );
            }
        }
    }
    Ok(triangles)
}

fn part_name<'a>(object: &'a Object, face: &'a Face) -> &'a str {
    face.source_part().unwrap_or(&object.name)
}

fn part_names(object: &Object, progress: &Progress) -> Result<Vec<String>, String> {
    let mut names = BTreeSet::new();
    for face in &object.geometry.faces {
        progress.check()?;
        names.insert(part_name(object, face));
    }
    let names: BTreeSet<_> = names.into_iter().map(base_name).collect();
    Ok(names.into_iter().collect())
}

fn base_name(name: &str) -> String {
    let mut name = name.trim().to_lowercase();
    if let Some((base, number)) = name.rsplit_once('.')
        && !number.is_empty()
        && number.bytes().all(|c| c.is_ascii_digit())
    {
        name.truncate(base.len());
    }
    ["lowpoly", "highpoly", "low", "high", "lp", "hp", "lo", "hi"]
        .into_iter()
        .find_map(|suffix| {
            let stem = name
                .strip_suffix(suffix)?
                .strip_suffix(['_', '-', ' ', '.'])?;
            (!stem.is_empty()).then(|| stem.to_owned())
        })
        .unwrap_or(name)
}

pub(super) fn tangent_axes(points: [Vec3; 3], uv: [Vec2; 3]) -> Option<(Vec3, Vec3)> {
    let [a, b, c] = points;
    let u = uv[1] - uv[0];
    let v = uv[2] - uv[0];
    let determinant = u.perp_dot(v);
    (determinant != 0.).then(|| {
        let sign = determinant.signum();
        (
            ((b - a) * v.y - (c - a) * u.y) * sign,
            ((c - a) * u.x - (b - a) * v.x) * sign,
        )
    })
}

pub(super) fn tangent_normal(normal: Vec3, target: Vec3, axes: (Vec3, Vec3)) -> Vec3 {
    let (tangent, bitangent) = axes;
    let tangent = (tangent - target * tangent.dot(target))
        .try_normalize()
        .unwrap_or_else(|| target.any_orthonormal_vector());
    let side = target.cross(tangent);
    let bitangent = side * side.dot(bitangent).signum();
    Vec3::new(
        normal.dot(tangent),
        normal.dot(bitangent),
        normal.dot(target),
    )
}
