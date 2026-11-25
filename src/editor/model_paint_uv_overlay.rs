//! Texture-local wireframe and UV seams for the paint workspace.
use crate::editor::{
    area_selection::clip_segment,
    model_document::MeshData,
    model_mesh::Edge,
    model_uv_mesh::{has_face_uv, seam_faces},
};
use bevy::{
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};
use image::{Rgba, RgbaImage};

const SIZE: u32 = 1024;
const WIRE: [u8; 3] = [225, 184, 119];
const SEAM: [u8; 3] = [255, 87, 62];

pub(super) struct Source {
    pub key: (usize, usize, usize),
    pub mesh: MeshData,
    pub offset: Vec2,
}

impl Source {
    fn matches(&self, other: &Self) -> bool {
        self.key == other.key
            && self.offset == other.offset
            && self.mesh.shares_storage_with(&other.mesh)
    }
}

#[derive(Default)]
pub(super) struct Overlay {
    source: Option<Source>,
    job: Option<Task<Raster>>,
    pub has_uv: Option<bool>,
    pub wire_ready: bool,
    pub seams_ready: bool,
}

pub(super) struct Raster {
    has_uv: bool,
    pub wire: Option<RgbaImage>,
    pub seams: Option<RgbaImage>,
}

impl Overlay {
    pub fn update(
        &mut self,
        source: Option<Source>,
        show_wire: bool,
        show_seams: bool,
    ) -> Option<Raster> {
        let Some(source) = source else {
            *self = Self::default();
            return None;
        };
        if !self.source.as_ref().is_some_and(|old| old.matches(&source)) {
            *self = Self {
                source: Some(source),
                ..Self::default()
            };
        }
        let wire = show_wire && !self.wire_ready;
        let seams = show_seams && !self.seams_ready;
        if self.job.is_none()
            && (self.has_uv.is_none() || (self.has_uv == Some(true) && (wire || seams)))
        {
            let source = self.source.as_ref().unwrap();
            let mesh = source.mesh.clone();
            let material = source.key.2;
            let offset = source.offset;
            self.job = Some(
                AsyncComputeTaskPool::get()
                    .spawn(async move { raster(&mesh, material, offset, wire, seams) }),
            );
        }
        let drawn = self.job.as_mut().and_then(|job| block_on(poll_once(job)))?;
        self.job = None;
        self.has_uv = Some(drawn.has_uv);
        self.wire_ready |= drawn.wire.is_some();
        self.seams_ready |= drawn.seams.is_some();
        Some(drawn)
    }
}

fn raster(mesh: &MeshData, material: usize, offset: Vec2, wire: bool, seams: bool) -> Raster {
    let mut faces = mesh
        .faces
        .iter()
        .filter(|face| face.material == material && has_face_uv(face))
        .peekable();
    let has_uv = faces.peek().is_some();
    let image = |[r, g, b]: [u8; 3]| RgbaImage::from_pixel(SIZE, SIZE, Rgba([r, g, b, 0]));
    let mut wire = (wire && has_uv).then(|| image(WIRE));
    let mut seams = (seams && has_uv).then(|| image(SEAM));
    if wire.is_some() || seams.is_some() {
        let seam_edges: std::collections::BTreeSet<_> = if seams.is_some() {
            seam_faces(mesh).map(|(edge, _)| edge).collect()
        } else {
            Default::default()
        };
        let tile = Rect::from_corners(Vec2::ZERO, Vec2::ONE);
        for face in faces {
            for (start, end) in face.corners.iter().zip(face.corners.iter().cycle().skip(1)) {
                // Texture UVs and image rows both increase downwards.
                if let Some([a, b]) = clip_segment(tile, start.uv - offset, end.uv - offset) {
                    let a = a * (SIZE - 1) as f32;
                    let b = b * (SIZE - 1) as f32;
                    if let Some(image) = &mut wire {
                        line(image, a, b);
                    }
                    if let Some(image) = &mut seams
                        && seam_edges.contains(&Edge::new(start.vertex, end.vertex))
                    {
                        line(image, a, b);
                    }
                }
            }
        }
    }
    Raster {
        has_uv,
        wire,
        seams,
    }
}

fn line(image: &mut RgbaImage, mut a: Vec2, mut b: Vec2) {
    let delta = (b - a).abs();
    let steep = delta.y > delta.x;
    if steep {
        a = Vec2::new(a.y, a.x);
        b = Vec2::new(b.y, b.x);
    }
    if a.x > b.x {
        std::mem::swap(&mut a, &mut b);
    }
    let span = b.x - a.x;
    for x in a.x.round() as i32..=b.x.round() as i32 {
        let t = if span > 0. {
            ((x as f32 - a.x) / span).clamp(0., 1.)
        } else {
            0.
        };
        let y = a.lerp(b, t).y;
        let low = y.floor() as i32;
        let upper = y.fract();
        for (row, coverage) in [(low, 1. - upper), (low + 1, upper)] {
            let (column, row) = if steep { (row, x) } else { (x, row) };
            if let Some(pixel) = image.get_pixel_mut_checked(column as u32, row as u32) {
                let alpha = (coverage * 255.).round() as u8;
                // Coincident edges keep the same opacity as a single edge.
                if alpha > pixel[3] {
                    pixel[3] = alpha;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_mesh::{Corner, Face, Geometry};

    #[test]
    fn seam_layer_distinguishes_continuity_splits_and_manually_marked_edges() {
        let face = |vertices: [usize; 3], offset: Vec2, material| Face {
            corners: vertices
                .into_iter()
                .map(|vertex| {
                    let mut corner = Corner::new(vertex);
                    corner.uv = [
                        Vec2::new(0.2, 0.2),
                        Vec2::new(0.8, 0.2),
                        Vec2::new(0.8, 0.8),
                        Vec2::new(0.2, 0.8),
                    ][vertex]
                        + offset;
                    corner
                })
                .collect(),
            material,
            smooth: false,
        };
        let mut mesh = MeshData::from(Geometry {
            faces: vec![
                face([0, 1, 2], Vec2::ZERO, 0),
                face([0, 2, 3], Vec2::ZERO, 0),
            ],
            ..default()
        });
        let center = SIZE / 2;
        let layers = raster(&mesh, 0, Vec2::ZERO, true, true);
        assert!(layers.wire.unwrap().get_pixel(center, center)[3] > 127);
        assert_eq!(layers.seams.unwrap().get_pixel(center, center)[3], 0);
        mesh.uv_seams.insert(Edge::new(0, 2));
        let marked = raster(&mesh, 0, Vec2::ZERO, false, true);
        assert!(marked.wire.is_none());
        assert!(marked.seams.unwrap().get_pixel(center, center)[3] > 127);

        mesh.uv_seams.clear();
        let tile = Vec2::new(2., -1.);
        mesh.faces[1] = face([0, 2, 3], tile, 1);
        for (slot, offset) in [(0, Vec2::ZERO), (1, tile)] {
            let split = raster(&mesh, slot, offset, false, true).seams.unwrap();
            assert!(split.get_pixel(center, center)[3] > 127);
        }
    }

    #[test]
    fn wire_respects_texture_rows_materials_and_udim_bounds() {
        let face = |material, uvs: [Vec2; 4]| Face {
            corners: uvs
                .into_iter()
                .enumerate()
                .map(|(vertex, uv)| {
                    let mut corner = Corner::new(vertex);
                    corner.uv = uv;
                    corner
                })
                .collect(),
            material,
            smooth: false,
        };
        let offset = Vec2::new(2., -1.);
        let rect = [
            Vec2::new(-0.25, 0.2),
            Vec2::new(0.6, 0.2),
            Vec2::new(0.6, 0.4),
            Vec2::new(-0.25, 0.4),
        ];
        let mesh = MeshData::from(Geometry {
            faces: vec![face(1, rect.map(|uv| uv + offset)), face(0, rect)],
            ..default()
        });
        let image = raster(&mesh, 1, offset, true, false).wire.unwrap();
        let top = (0.2 * (SIZE - 1) as f32).round() as u32;
        assert!(image.get_pixel(100, top)[3] > 127);
        assert_eq!(image.get_pixel(100, SIZE - 1 - top)[3], 0);
        assert_eq!(
            image.get_pixel(0, 300)[3],
            0,
            "clipping must not produce a false tile edge"
        );
        assert!(raster(&mesh, 1, offset, false, false).wire.is_none());
        let other_tile = raster(&mesh, 0, offset, true, false).wire;
        assert!(other_tile.unwrap().pixels().all(|pixel| pixel[3] == 0));
    }
}
