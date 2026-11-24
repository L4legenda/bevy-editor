//! Maps from the active texture set, packed for Material Painter's graph runtime.
use super::{Graph, Value, library};
use crate::editor::ui::model_viewer::paint::bake::{BakeMap, Maps};
use bevy::render::{
    render_resource::{TexelCopyBufferLayout, TextureFormat, TextureUsages},
    renderer::{RenderDevice, RenderQueue},
};
use material_painter_graph::{CUSTOM_MAP_VALUE, MESH_MAPS, Tex};
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

const BUILTIN: [BakeMap; MESH_MAPS.len()] = [
    BakeMap::Occlusion,
    BakeMap::Curvature,
    BakeMap::Cavity,
    BakeMap::Thickness,
    BakeMap::Sky,
    BakeMap::EdgeDistance,
    BakeMap::CreaseDistance,
    BakeMap::PartRandom,
    BakeMap::PartGradient,
    BakeMap::BentNormal,
    BakeMap::ProjectedNormal,
    BakeMap::ProjectedHeight,
    BakeMap::PartId,
];
const NEUTRAL: [u8; MESH_MAPS.len()] = [
    255, 128, 255, 255, 255, 255, 255, 128, 128, 128, 128, 128, 128,
];

pub(in crate::editor::ui::model_viewer::paint) fn kind(value: usize) -> Option<BakeMap> {
    if let Some(id) = value.checked_sub(CUSTOM_MAP_VALUE) {
        Some(BakeMap::Custom(u64::try_from(id).ok()?))
    } else {
        BUILTIN.get(value).copied()
    }
}

pub(in crate::editor::ui::model_viewer::paint) fn same<T>(
    a: Option<&Arc<T>>,
    b: Option<&Arc<T>>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

pub(in crate::editor::ui::model_viewer::paint) fn used(graph: &Graph) -> BTreeSet<usize> {
    used_with(graph, &library())
}

pub(in crate::editor::ui::model_viewer::paint) fn used_with(
    graph: &Graph,
    library: &super::Library,
) -> BTreeSet<usize> {
    graph
        .nodes
        .iter()
        .flat_map(|node| {
            library
                .find(&node.def)
                .into_iter()
                .flat_map(|def| &def.params)
                .filter_map(move |param| match library.value(node, &param.name) {
                    Some(Value::MeshMap(value)) => Some(value),
                    _ => None,
                })
        })
        .collect()
}

#[derive(Default)]
pub(in crate::editor::ui::model_viewer::paint) struct Textures {
    images: Vec<Tex>,
    channels: HashMap<usize, (usize, usize)>,
}

impl Textures {
    pub fn new(
        requested: &BTreeSet<usize>,
        maps: Option<&Maps>,
        resolution: u32,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) -> Self {
        let mut textures = Self::default();
        let (normal, unsigned): (Vec<_>, Vec<_>) = requested
            .iter()
            .copied()
            .partition(|&value| kind(value) == Some(BakeMap::ProjectedNormal));
        for values in unsigned.chunks(4).chain(normal.chunks(4)) {
            let signed = kind(values[0]) == Some(BakeMap::ProjectedNormal);
            let samples: Vec<_> = values
                .iter()
                .map(|&value| maps.and_then(|maps| maps.graph_plane(kind(value)?)))
                .collect();
            let size = maps
                .filter(|_| samples.iter().any(Option::is_some))
                .map_or(1, |maps| resolution.min(maps.size));
            let original_size = maps.map_or(1, |maps| maps.size);
            let mut bytes = Vec::with_capacity(size as usize * size as usize * 4);
            for y in 0..size {
                for x in 0..size {
                    let coord = |p| (2 * p + 1) * original_size / (2 * size);
                    let pixel = (coord(y) * original_size + coord(x)) as usize;
                    let mut packed = [0; 4];
                    for (channel, &value) in values.iter().enumerate() {
                        let sample = samples[channel].as_ref().map_or_else(
                            || NEUTRAL.get(value).copied().unwrap_or(0),
                            |sample| sample(pixel),
                        );
                        packed[channel] = if signed {
                            (i16::from(sample) - 128).max(-127) as i8 as u8
                        } else {
                            sample
                        };
                    }
                    bytes.extend_from_slice(&packed);
                }
            }
            let format = if signed {
                TextureFormat::Rgba8Snorm
            } else {
                TextureFormat::Rgba8Unorm
            };
            let texture = upload(
                device,
                queue,
                "Material graph baked maps",
                size,
                format,
                &bytes,
            );
            for (channel, &value) in values.iter().enumerate() {
                textures
                    .channels
                    .insert(value, (textures.images.len(), channel));
            }
            textures.images.push(texture);
        }
        textures
    }

    pub fn get(&self, value: usize) -> Option<(&Tex, usize)> {
        let &(texture, channel) = self.channels.get(&value)?;
        Some((&self.images[texture], channel))
    }
}

pub(super) fn upload(
    device: &RenderDevice,
    queue: &RenderQueue,
    label: &str,
    size: u32,
    format: TextureFormat,
    bytes: &[u8],
) -> Tex {
    let texture = Tex::new(
        device.wgpu_device(),
        label,
        size,
        size,
        format,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
    );
    queue.write_texture(
        texture.copy_info(),
        bytes,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes.len() as u32 / size),
            rows_per_image: Some(size),
        },
        texture.extent(),
    );
    texture
}
