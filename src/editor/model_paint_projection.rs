//! Camera-dependent stroke geometry is prepared lazily on the paint worker.
use super::*;
use std::sync::OnceLock;
const CELL: f32 = 64.;
pub(super) struct Mapping {
    pub objects: Vec<Object>,
    pub slot: usize,
    pub revision:u64,
    pub object: usize,
    pub camera: CameraMap,
    pub uv: bool,
    geometry: OnceLock<Projection>,
}
pub(super) struct Projection {
    pub triangles: Vec<Triangle>,
    pub bvh: editing::input::Bvh,
    bins: Vec<Vec<usize>>,
    columns: usize,
    rows: usize,
    rect: Rect,
}
pub(super) struct Triangle {
    pub points: [Vec3; 3],
    pub uv: [Vec2; 3],
    pub face: usize,
    pub screen: Option<[Vec2; 3]>,
}
#[derive(Clone, Copy)]
pub(super) struct CameraMap {
    pub pose: Transform,
    pub inverse: Mat4,
    pub rect: Rect,
    pub ortho: bool,
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
            2. * -p.z * (FOV * 0.5).tan()
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
                        p.x * aspect * (FOV * 0.5).tan(),
                        -p.y * (FOV * 0.5).tan(),
                        -1.,
                    ))
                .normalize(),
            )
        }
    }
}
#[cfg(test)] pub(super) fn weights(p: Vec2, t: [Vec2; 3]) -> Option<Vec3> {
    let a = t[1] - t[0];
    let b = t[2] - t[0];
    let d = a.perp_dot(b);
    if d.abs() < 1.0e-10 {
        return None;
    }
    let q = p - t[0];
    let u = q.perp_dot(b) / d;
    let v = a.perp_dot(q) / d;
    Some(Vec3::new(1. - u - v, u, v))
}
#[cfg(test)] pub(super) fn mix3(t: [Vec3; 3], b: Vec3) -> Vec3 {
    t[0] * b.x + t[1] * b.y + t[2] * b.z
}
pub(super) fn mapping(
    doc: &Document,
    object: &Object,
    slot: usize,
    viewer: &Viewer,
    rect: Rect,
    uv: bool,
) -> Mapping {
    let pose = view_pose(viewer);
    Mapping {
        revision:doc.revision,
        objects: doc.objects.clone(),
        slot,
        object: object.id,
        uv,
        geometry: OnceLock::new(),
        camera: CameraMap {
            pose,
            inverse: pose.to_matrix().inverse(),
            rect,
            ortho: viewer.orthographic,
            height: view_height(viewer, viewer.distance),
            depth: orthographic_depth(viewer),
            epsilon: viewer.radius.max(0.001) * 0.0005,
        },
    }
}
pub(super) fn has_uv(object: &Object, slot: usize) -> bool {
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
    pub fn geometry(&self) -> &Projection {
        self.geometry.get_or_init(|| {
            let mut triangles = Vec::new();
            if let Some(object) = self.objects.iter().find(|o| o.id == self.object) {
                for (face, f) in object
                    .geometry
                    .faces
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.material == self.slot)
                {
                    for indices in object.geometry.triangles(f) {
                        let points = indices.map(|i| {
                            object
                                .transform
                                .transform_point(object.geometry.positions[f.corners[i].vertex])
                        });
                        let uv = indices.map(|i| f.corners[i].uv);
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
                bvh: editing::input::Bvh::from_objects(&self.objects),
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
    #[cfg(test)] pub fn candidates(&self, r: Rect) -> Vec<usize> {
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
