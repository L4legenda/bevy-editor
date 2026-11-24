//! Projects high-poly source geometry onto the low-poly UV raster.
use super::super::ProjectionFormat;
use super::{Progress, ProjectionSettings};
use crate::editor::ui::model_viewer::editing::input::{Bvh, PickTriangle};
use bevy::{gltf::gltf, prelude::*};
use std::{
    collections::VecDeque,
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
    usage: SourceUse,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceUse {
    Omit,
    Project,
    Occluder,
}
impl From<bool> for SourceUse {
    fn from(included: bool) -> Self {
        if included { Self::Project } else { Self::Omit }
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
            usage: SourceUse::Project,
        })
    }

    fn with_usage(mut self, usage: SourceUse) -> Self {
        self.usage = usage;
        self
    }
}

struct SourceGeometry {
    bvh: Bvh,
    triangles: Vec<SourceTriangle>,
    relief: Mutex<high_relief::Cache>,
}

#[derive(PartialEq, Eq)]
struct SourceKey {
    hash: blake3::Hash,
    match_name: Option<String>,
}

const CACHED_SOURCES: usize = 4;
const CACHED_TRIANGLES: usize = 1_000_000;
static SOURCES: Mutex<VecDeque<(SourceKey, Arc<SourceGeometry>)>> = Mutex::new(VecDeque::new());

pub(super) struct Projection {
    source: Arc<SourceGeometry>,
    front: f32,
    rear: f32,
    fill_misses: bool,
}

pub(super) struct Segment {
    pub origin: Vec3,
    pub outward: Vec3,
    pub front: f32,
    pub rear: f32,
    pub range: f32,
}

pub(super) struct Query {
    pub segment: Segment,
    pub point: Vec3,
    pub normal: Vec3,
    pub fill: bool,
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
        if directions.len() != overrides.len() || overrides.iter().all(|&value| value == 1.) {
            return Ok(None);
        }
        let limit = (span / self.front.max(self.rear)).min(8.);
        Cage::painted(base, directions, overrides, self.front, limit, progress).map(Some)
    }
    pub(super) fn load(
        path: &Path,
        name: &str,
        scale: f32,
        settings: &ProjectionSettings,
        progress: &Progress,
    ) -> Result<Self, String> {
        let match_name = settings.match_names.then(|| base_name(name));
        #[cfg(test)]
        let key = if progress.fresh_high_source {
            None
        } else {
            super::super::disk::source_hash(path, progress).map(|hash| SourceKey {
                hash,
                match_name: match_name.clone(),
            })
        };
        #[cfg(not(test))]
        let key = super::super::disk::source_hash(path, progress).map(|hash| SourceKey {
            hash,
            match_name: match_name.clone(),
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
        let triangles = read_triangles(
            path,
            |name| {
                if match_name
                    .as_deref()
                    .is_none_or(|expected| base_name(name) == expected)
                {
                    SourceUse::Project
                } else {
                    SourceUse::Occluder
                }
            },
            progress,
        )?;
        if !triangles
            .iter()
            .any(|triangle| triangle.usage == SourceUse::Project)
        {
            return Err(if settings.match_names {
                format!("В high-poly нет треугольников для «{name}» · проверьте имена объектов")
            } else {
                "В high-poly нет треугольников".into()
            });
        }
        progress.check()?;
        let projection = Self::new(triangles, scale, settings);
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

    fn new(triangles: Vec<SourceTriangle>, scale: f32, settings: &ProjectionSettings) -> Self {
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
                relief: Mutex::default(),
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
        }
    }

    pub(super) fn scene(&self) -> &Bvh {
        &self.source.bvh
    }

    pub(super) fn surface_normal(&self, triangle: usize, point: Vec3) -> Vec3 {
        self.hit(triangle, point, 0., true).normal
    }

    pub(super) fn corner_normals(&self, triangle: usize) -> [Vec3; 3] {
        self.source.triangles[triangle].normals
    }

    fn can_project(&self, triangle: &PickTriangle) -> bool {
        self.source.triangles[triangle.face].usage == SourceUse::Project
    }

    pub(super) fn accepts_projection(&self, triangle: &PickTriangle) -> bool {
        self.can_project(triangle)
    }

    pub(super) fn query(
        &self,
        point: Vec3,
        direction: Vec3,
        normal: Vec3,
        cage: Option<CageRay>,
        fill: bool,
    ) -> Query {
        Query {
            segment: self.segment(point, direction, cage),
            point,
            normal,
            fill: fill && self.fill_misses,
        }
    }

    pub(super) fn segment(&self, point: Vec3, direction: Vec3, cage: Option<CageRay>) -> Segment {
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
        }
    }

    pub(super) fn cast(&self, segment: &Segment) -> Option<(usize, f32)> {
        let Segment {
            origin,
            outward,
            front,
            rear,
            ..
        } = *segment;
        let nearest = |filter_backfaces: bool| {
            self.source
                .bvh
                .cast_triangle_up_to(origin, -outward, front + rear, |triangle| {
                    let [a, b, c] = triangle.points;
                    self.can_project(triangle)
                        && (!filter_backfaces || (b - a).cross(c - a).dot(outward) > 0.)
                })
        };
        nearest(true).or_else(|| nearest(false))
    }

    pub(super) fn closest_crossing(
        &self,
        position: Vec3,
        direction: Vec3,
        reach: f32,
        bias: f32,
    ) -> Option<(usize, f32)> {
        let crossing = |sign: f32| {
            let ray = direction * sign;
            self.source
                .bvh
                .cast_triangle_up_to(position - ray * bias, ray, reach + bias, |triangle| {
                    self.can_project(triangle)
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
    ) -> Option<Hit> {
        self.direct(point, direction, cage)
            .or_else(|| self.fill(point, direction, low_normal, cage))
    }

    pub(super) fn direct(
        &self,
        point: Vec3,
        direction: Vec3,
        cage: Option<CageRay>,
    ) -> Option<Hit> {
        let segment = self.segment(point, direction, cage);
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
    ) -> Option<Hit> {
        if !self.fill_misses {
            return None;
        }
        let range = self.segment(point, direction, cage).range;
        let (face, hit) = self
            .source
            .bvh
            .nearest_surface_where(point, range * 4., |triangle| self.can_project(triangle))?;
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
    while let Some((node, parent)) = nodes.pop() {
        progress.check()?;
        let transform = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        nodes.extend(node.children().map(|child| (child, transform)));
        let Some(mesh) = node.mesh() else { continue };
        let usage = usage(node.name().or_else(|| mesh.name()).unwrap_or_default());
        if usage == SourceUse::Omit {
            continue;
        }
        let matrix = Mat3::from_mat4(transform);
        let determinant = matrix.determinant();
        if !determinant.is_finite() || determinant == 0. {
            continue;
        }
        let normal_matrix = matrix.inverse().transpose();
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
                triangles.extend(
                    SourceTriangle::new(
                        [a, b, c],
                        ids.map(|i| normals.get(i).copied().unwrap_or_default()),
                        node.index(),
                    )
                    .map(|triangle| triangle.with_usage(usage)),
                );
            }
        }
    }
    Ok(triangles)
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
