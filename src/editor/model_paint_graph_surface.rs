//! World-space positions, normals and baked channels for material nodes in UV space.
use super::{
    Graph,
    bake::{BakeMap, Maps},
    library, mesh_maps, texture_fill,
};
use crate::editor::model_document::{Document, MeshData};
use bevy::{
    prelude::{Transform, Vec2, Vec3},
    render::{
        render_resource::TextureFormat,
        renderer::{RenderDevice, RenderQueue},
    },
};
use half::f16;
use material_painter_graph::{Tex, runtime::Surface};

#[derive(Clone)]
struct Shape {
    id: usize,
    mesh: MeshData,
    transform: Transform,
}

pub(in crate::editor::ui::model_viewer::paint) struct Source {
    shapes: Vec<Shape>,
    object: usize,
    slot: usize,
    offset: Vec2,
}

impl Source {
    pub fn capture(doc: &Document, (object, slot): (usize, usize)) -> Option<Self> {
        let target = doc
            .objects
            .iter()
            .find(|item| item.id == object && !item.hidden)?;
        target.materials.get(slot)?;
        let shapes = doc
            .objects
            .iter()
            .filter(|item| !item.hidden)
            .map(|item| Shape {
                id: item.id,
                mesh: item.geometry.clone(),
                transform: item.transform,
            })
            .collect();
        Some(Self {
            shapes,
            object,
            slot,
            offset: target.uv_offset(slot),
        })
    }

    pub fn matches(&self, doc: &Document, (object, slot): (usize, usize)) -> bool {
        if (self.object, self.slot) != (object, slot) {
            return false;
        }
        let mut current = doc.objects.iter().filter(|item| !item.hidden);
        self.shapes.iter().all(|shape| {
            current.next().is_some_and(|item| {
                item.id == shape.id
                    && item.transform == shape.transform
                    && item.geometry.shares_storage_with(&shape.mesh)
                    && (item.id != object
                        || (item.materials.get(slot).is_some()
                            && item.uv_offset(slot) == self.offset))
            })
        }) && current.next().is_none()
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for shape in &self.shapes {
            for corner in shape.mesh.faces.iter().flat_map(|face| &face.corners) {
                let p = shape
                    .transform
                    .transform_point(shape.mesh.positions[corner.vertex]);
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        if lo.is_finite() && hi.is_finite() {
            (lo, hi - lo)
        } else {
            (Vec3::ZERO, Vec3::ZERO)
        }
    }
}

pub(in crate::editor::ui::model_viewer::paint) fn used(graph: &Graph) -> bool {
    let lib = library();
    graph
        .nodes
        .iter()
        .any(|node| lib.find(&node.def).is_some_and(|def| def.surface))
}

type Channel = Option<(BakeMap, usize)>;
const PACK: [[Channel; 4]; 4] = [
    [
        Some((BakeMap::Occlusion, 0)),
        Some((BakeMap::Curvature, 0)),
        Some((BakeMap::Cavity, 0)),
        Some((BakeMap::Thickness, 0)),
    ],
    [
        Some((BakeMap::Sky, 0)),
        Some((BakeMap::EdgeDistance, 0)),
        Some((BakeMap::CreaseDistance, 0)),
        Some((BakeMap::PartRandom, 0)),
    ],
    [
        Some((BakeMap::BentNormal, 0)),
        Some((BakeMap::BentNormal, 1)),
        Some((BakeMap::BentNormal, 2)),
        Some((BakeMap::PartGradient, 0)),
    ],
    [
        Some((BakeMap::PartId, 0)),
        Some((BakeMap::PartId, 1)),
        Some((BakeMap::PartId, 2)),
        None,
    ],
];
const NEUTRAL: [[u8; 4]; 4] = [
    [255, 128, 255, 255],
    [255, 255, 255, 128],
    [128, 255, 128, 128],
    [128, 128, 128, 255],
];

pub(in crate::editor::ui::model_viewer::paint) struct Textures {
    position: Tex,
    normal: Tex,
    maps: [Tex; 4],
    min: Vec3,
    extent: Vec3,
    edge_max: f32,
}

impl Textures {
    pub fn new(
        source: &Source,
        baked: Option<&Maps>,
        size: u32,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) -> Self {
        let shape = source
            .shapes
            .iter()
            .find(|shape| shape.id == source.object)
            .unwrap();
        let world: Vec<_> = shape
            .mesh
            .positions
            .iter()
            .map(|&p| shape.transform.transform_point(p))
            .collect();
        let (min, extent) = source.bounds();
        let count = size as usize * size as usize;
        let mut position = vec![[0u16; 4]; count];
        let mut normal = vec![[128u8, 128, 255, 0]; count];
        let mut coverage = vec![0u8; count];
        texture_fill::raster(
            &shape.mesh,
            source.slot,
            size,
            source.offset,
            shape.transform,
            &world,
            &mut coverage,
            |i, _, p, n, _, _| {
                position[i] = p
                    .extend(1.)
                    .to_array()
                    .map(|v| f16::from_f32(v.clamp(-65504., 65504.)).to_bits());
                normal[i] = (n * 0.5 + Vec3::splat(0.5))
                    .extend(1.)
                    .to_array()
                    .map(|v| (v.clamp(0., 1.) * 255.).round() as u8);
            },
        );
        pad(size as usize, &mut coverage, &mut position, &mut normal);
        let position_bytes: Vec<u8> = position
            .into_iter()
            .flatten()
            .flat_map(u16::to_le_bytes)
            .collect();
        let position = mesh_maps::upload(
            device,
            queue,
            "Graph world positions",
            size,
            TextureFormat::Rgba16Float,
            &position_bytes,
        );
        let normal = mesh_maps::upload(
            device,
            queue,
            "Graph world normals",
            size,
            TextureFormat::Rgba8Unorm,
            normal.as_flattened(),
        );
        let maps = std::array::from_fn(|index| {
            let samples = PACK[index].map(|component| {
                let (map, channel) = component?;
                baked?.graph_component(map, channel)
            });
            let edge = baked
                .filter(|_| samples.iter().any(Option::is_some))
                .map_or(1, |maps| size.min(maps.size));
            let source_size = baked.map_or(1, |maps| maps.size);
            let mut pixels = Vec::with_capacity(edge as usize * edge as usize * 4);
            for y in 0..edge {
                for x in 0..edge {
                    let coord = |p| (2 * p + 1) * source_size / (2 * edge);
                    let pixel = (coord(y) * source_size + coord(x)) as usize;
                    pixels.extend((0..4).map(|channel| {
                        samples[channel]
                            .as_ref()
                            .map_or(NEUTRAL[index][channel], |sample| sample(pixel))
                    }));
                }
            }
            mesh_maps::upload(
                device,
                queue,
                "Graph surface channels",
                edge,
                TextureFormat::Rgba8Unorm,
                &pixels,
            )
        });
        Self {
            position,
            normal,
            maps,
            min,
            extent,
            edge_max: baked.map_or((extent.length() * 0.2).max(0.001), |maps| maps.edge_reach()),
        }
    }

    pub fn inputs(&self) -> Surface<'_> {
        Surface {
            pos: &self.position,
            nrm: &self.normal,
            maps: [&self.maps[0], &self.maps[1], &self.maps[2], &self.maps[3]],
            bmin: self.min.to_array(),
            bsize: self.extent.to_array(),
            edge_max: self.edge_max,
        }
    }
}

fn pad(size: usize, coverage: &mut [u8], positions: &mut [[u16; 4]], normals: &mut [[u8; 4]]) {
    for ring in 1..=4 {
        for i in 0..coverage.len() {
            if coverage[i] != 0 {
                continue;
            }
            let (x, y) = (i % size, i / size);
            let neighbors = [
                (x > 0).then(|| i - 1),
                (x + 1 < size).then(|| i + 1),
                (y > 0).then(|| i - size),
                (y + 1 < size).then(|| i + size),
            ];
            if let Some(source) = neighbors
                .into_iter()
                .flatten()
                .find(|&n| coverage[n] == ring)
            {
                coverage[i] = ring + 1;
                positions[i] = positions[source];
                positions[i][3] = f16::from_f32(0.75).to_bits();
                normals[i] = normals[source];
            }
        }
    }
}
