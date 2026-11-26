//! Camera-dependent stroke geometry is prepared lazily on the paint worker.
use super::*;
pub(in crate::editor::ui::model_viewer::paint) use crate::editor::model_mesh::uv_weights as weights;
#[cfg(test)]
use std::sync::OnceLock;
#[cfg(test)]
const CELL: f32 = 64.;
pub(super) type GeometryVersion = (u64, u64);
pub(super) struct Mapping {
    pub objects: Vec<Object>,
    pub slot: usize,
    pub revision:u64,
    pub visibility: sets::SetVisibility,
    pub object: usize,
    pub camera: CameraMap,
    pub uv: bool,
    #[cfg(test)]
    geometry: OnceLock<Projection>,
}
#[cfg(test)]
pub(super) struct Projection {
    pub triangles: Vec<Triangle>,
    pub bvh: editing::input::Bvh,
    bins: Vec<Vec<usize>>,
    columns: usize,
    rows: usize,
    rect: Rect,
}
#[cfg(test)]
pub(super) struct Triangle {
    pub points: [Vec3; 3],
    pub uv: [Vec2; 3],
    pub face: usize,
    pub screen: Option<[Vec2; 3]>,
}
#[derive(Clone, Copy, PartialEq)]
pub(super) struct CameraMap {
    pub pose: Transform,
    pub inverse: Mat4,
    pub rect: Rect,
    pub clip: Rect,
    pub ortho: bool,
    pub fov: f32,
    pub height: f32,
    pub depth: f32,
    pub epsilon: f32,
}
impl CameraMap {
    pub fn project(self, p: Vec3) -> Option<Vec2> {
        let p = self.inverse.transform_point3(p);
        if !self.ortho && p.z >= -1.0e-6 {
            return None;
        }
        let h = if self.ortho {
            self.height
        } else {
            2. * -p.z * (self.fov * 0.5).tan()
        };
        Some(
            self.rect.center()
                + Vec2::new(
                    p.x / (h * 0.5 * self.rect.width() / self.rect.height()),
                    -p.y / (h * 0.5),
                ) * self.rect.size()
                    * 0.5,
        )
    }
    pub fn ray(self, p: Vec2) -> (Vec3, Vec3) {
        let p = (p - self.rect.center()) / self.rect.size() * 2.;
        let aspect = self.rect.width() / self.rect.height();
        if self.ortho {
            (
                self.pose.translation
                    + self.pose.rotation
                        * Vec3::new(
                            p.x * self.height * 0.5 * aspect,
                            -p.y * self.height * 0.5,
                            0.,
                        )
                    - *self.pose.forward() * self.depth,
                *self.pose.forward(),
            )
        } else {
            (
                self.pose.translation,
                (self.pose.rotation
                    * Vec3::new(
                        p.x * aspect * (self.fov * 0.5).tan(),
                        -p.y * (self.fov * 0.5).tan(),
                        -1.,
                    ))
                .normalize(),
            )
        }
    }
}
#[cfg(test)] pub(super) fn mix3(t: [Vec3; 3], b: Vec3) -> Vec3 {
    t[0] * b.x + t[1] * b.y + t[2] * b.z
}
#[cfg(test)]
pub(super) fn mapping(
    doc: &Document,
    object: &Object,
    slot: usize,
    viewer: &Viewer,
    rect: Rect,
    uv: bool,
    visibility: &sets::SetVisibility,
) -> Mapping {
    Mapping::for_camera(doc, object, slot, camera_map(viewer, rect), uv, visibility)
}
pub(super) fn camera_map(viewer: &Viewer, rect: Rect) -> CameraMap {
    model_camera_map(viewer.camera(), viewer.radius, rect)
}

pub(super) fn model_camera_map(camera: ModelCamera, radius: f32, rect: Rect) -> CameraMap {
    let pose = model_pose(camera);
    CameraMap {
        pose,
        inverse: pose.to_matrix().inverse(),
        rect,
        clip: rect,
        ortho: camera.orthographic,
        fov: camera.fov,
        height: 2. * camera.distance.max(0.000001) * (camera.fov * 0.5).tan(),
        depth: model_orthographic_depth(camera, radius),
        epsilon: radius.max(0.001) * 0.0005,
    }
}
pub(in crate::editor::ui::model_viewer::paint) fn has_uv(object: &Object, slot: usize) -> bool {
    object
        .geometry
        .faces
        .iter()
        .filter(|f| f.material == slot)
        .any(|f| {
            object.geometry.triangles(f).into_iter().any(|t| {
                let p = t.map(|i| f.corners[i].uv);
                (p[1] - p[0]).perp_dot(p[2] - p[0]).abs() > 1.0e-8
            })
        })
}
impl Mapping {
    pub fn for_camera(doc: &Document, object: &Object, slot: usize, camera: CameraMap, uv: bool, visibility: &sets::SetVisibility) -> Self {
        Self {
            revision: doc.revision,
            visibility: visibility.clone(),
            objects: doc.objects.clone(),
            slot,
            object: object.id,
            uv,
            #[cfg(test)]
            geometry: OnceLock::new(),
            camera,
        }
    }

    pub fn version(&self) -> GeometryVersion {
        (self.revision, self.visibility.revision)
    }

    #[cfg(test)]
    pub fn geometry(&self) -> &Projection {
        self.geometry.get_or_init(|| {
            let mut triangles = Vec::new();
            if let Some(object) = self.objects.iter().find(|o| o.id == self.object) {
                let offset = object.uv_offset(self.slot);
                for (face, f) in object
                    .geometry
                    .faces
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.material == self.slot && self.visibility.shown(object.id, f.material))
                {
                    for indices in object.geometry.triangles(f) {
                        let points = indices.map(|i| {
                            object
                                .transform
                                .transform_point(object.geometry.positions[f.corners[i].vertex])
                        });
                        let uv = indices.map(|i| f.corners[i].uv - offset);
                        let screen = if self.uv {
                            Some(uv.map(|p| self.camera.rect.min + p * self.camera.rect.size()))
                        } else {
                            let p = points.map(|p| self.camera.project(p));
                            if let [Some(a), Some(b), Some(c)] = p {
                                Some([a, b, c])
                            } else {
                                None
                            }
                        };
                        triangles.push(Triangle {
                            points,
                            uv,
                            face,
                            screen,
                        });
                    }
                }
            }
            let columns = (self.camera.rect.width() / CELL).ceil().max(1.) as usize;
            let rows = (self.camera.rect.height() / CELL).ceil().max(1.) as usize;
            let mut out = Projection {
                triangles,
                bvh: editing::input::Bvh::from_objects_where(&self.objects, |id, slot| self.visibility.shown(id, slot)),
                bins: vec![Vec::new(); columns * rows],
                columns,
                rows,
                rect: self.camera.rect,
            };
            for i in 0..out.triangles.len() {
                if let Some(p) = out.triangles[i].screen {
                    let min = p[0].min(p[1]).min(p[2]);
                    let max = p[0].max(p[1]).max(p[2]);
                    if max.x < out.rect.min.x
                        || max.y < out.rect.min.y
                        || min.x > out.rect.max.x
                        || min.y > out.rect.max.y
                    {
                        continue;
                    }
                    let (lo, hi) = out.cells(Rect::from_corners(min, max));
                    for y in lo.y..=hi.y {
                        for x in lo.x..=hi.x {
                            out.bins[y as usize * columns + x as usize].push(i);
                        }
                    }
                }
            }
            out
        })
    }
}
#[cfg(test)]
impl Projection {
    fn cells(&self, r: Rect) -> (UVec2, UVec2) {
        let limit = Vec2::new((self.columns - 1) as f32, (self.rows - 1) as f32);
        (
            ((r.min - self.rect.min) / CELL)
                .floor()
                .clamp(Vec2::ZERO, limit)
                .as_uvec2(),
            ((r.max - self.rect.min) / CELL)
                .floor()
                .clamp(Vec2::ZERO, limit)
                .as_uvec2(),
        )
    }
    pub fn candidates(&self, r: Rect) -> Vec<usize> {
        let (lo, hi) = self.cells(r);
        let mut out = Vec::new();
        for y in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                out.extend_from_slice(&self.bins[y as usize * self.columns + x as usize]);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}
