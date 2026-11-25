//! Convert the editable parts of a Painter layer stack.
use super::{CHANNELS, PolygonReplay, Stencil, StencilCache, StrokeCoverage, Summary, Tile};
use crate::editor::model_document::MeshData;
use crate::editor::model_paint_data::{
    Blend, Layer, LayerKind, MAX_LAYER_DEPTH, MAX_LAYERS,
    mask::{
        BakedInputs, Filter, FilterKind, Generator, GeneratorInput, GeneratorKind, GeneratorSurface, MAX_FILTERS,
        MAX_MASK_LAYERS, Mask, MaskBlend, MaskLayer,
    },
    over, solid,
    texture_fill::{Bitmap, Fill, Projection, pad_texels, raster, triplanar_coordinates},
};

#[path = "model_paint_spp_anchors.rs"]
mod anchors;
use anchors::{Anchor, Anchors};
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc};
use subcraft_spp::{
    FileReader,
    document::{
        ActionInfo, ActionKind, BrushInfo, GraphRef, LayerInfo, LayerKind as SourceKind,
        LevelsInfo, SourceInfo, StrokeInfo, UvTransform,
    },
};

pub(super) fn estimated_planes(layers: &[LayerInfo]) -> usize {
    layers
        .iter()
        .map(|layer| {
            let bits = layer
                .sources
                .iter()
                .chain(
                    layer
                        .content
                        .iter()
                        .filter_map(|action| match &action.kind {
                            ActionKind::Paint {
                                brush: Some(brush), ..
                            } if action.enabled => Some(&brush.sources),
                            ActionKind::Fill { sources, .. } if action.enabled => Some(sources),
                            _ => None,
                        })
                        .flatten(),
                )
                .filter_map(|source| match source {
                    SourceInfo::Uniform { channels, .. }
                    | SourceInfo::Bitmap { channels, .. }
                    | SourceInfo::Reference { channels, .. } => Some(*channels),
                    _ => None,
                })
                .fold(0, |bits, source| bits | source);
            usize::from(layer.has_mask)
                + CHANNELS
                    .iter()
                    .filter(|(channel, ..)| bits & channel != 0)
                    .count()
                + estimated_planes(&layer.children)
        })
        .sum()
}

pub(super) fn add(
    file: &mut FileReader,
    source: &[LayerInfo],
    size: u32,
    has_cache: bool,
    remaining: &mut usize,
    summary: &mut Summary,
    target: &mut Vec<Layer>,
    (mesh, slot, replay): (&MeshData, usize, Option<&PolygonReplay>),
    tile: Option<Tile>,
    scope: &str,
    baked: Option<&Arc<BakedInputs>>,
) {
    if source.is_empty() {
        return;
    }
    let mut rebuild = Rebuild {
        file,
        remaining,
        summary,
        size,
        solids: HashMap::new(),
        bitmaps: HashMap::new(),
        polygon_replay: replay.map(|replay| (replay, slot)),
        anchors: Anchors::requested(source),
        mesh,
        slot,
        tile,
        scope,
        baked,
        generator_surface: None,
        stencil_cache: StencilCache::default(),
        uv_owners: None,
        stroke_padding: None,
    };
    let end = MAX_LAYERS - 1;
    rebuild.layers(source, target, 1, end);
    for plane in [rebuild.uv_owners.take(), rebuild.stroke_padding.take()]
        .into_iter()
        .flatten()
    {
        *rebuild.remaining += plane.len() * size_of::<u32>();
    }
    let mut group = Layer::new("Слои Substance Painter".into(), LayerKind::Folder);
    group.visible = !has_cache;
    target.push(group);
}

struct Rebuild<'a> {
    file: &'a mut FileReader,
    remaining: &'a mut usize,
    summary: &'a mut Summary,
    size: u32,
    solids: HashMap<[u8; 4], Arc<Vec<u8>>>,
    bitmaps: HashMap<String, Option<Arc<Bitmap>>>,
    polygon_replay: Option<(&'a PolygonReplay, usize)>,
    anchors: Anchors,
    mesh: &'a MeshData,
    slot: usize,
    tile: Option<Tile>,
    scope: &'a str,
    baked: Option<&'a Arc<BakedInputs>>,
    generator_surface: Option<Arc<GeneratorSurface>>,
    stencil_cache: StencilCache,
    uv_owners: Option<Vec<u32>>,
    stroke_padding: Option<Vec<u32>>,
}

impl Rebuild<'_> {
    fn reserve(&mut self, bytes: usize) -> bool {
        if bytes > *self.remaining {
            return false;
        }
        *self.remaining -= bytes;
        true
    }

    fn plane(&mut self, rgba: [u8; 4]) -> Option<Arc<Vec<u8>>> {
        if let Some(pixels) = self.solids.get(&rgba) {
            return Some(pixels.clone());
        }
        if !self.reserve(self.size as usize * self.size as usize * 4) {
            return None;
        }
        let pixels = solid(self.size, rgba);
        self.solids.insert(rgba, pixels.clone());
        Some(pixels)
    }

    fn bitmap(&mut self, url: &str) -> Option<Arc<Bitmap>> {
        if let Some(result) = self.bitmaps.get(url) {
            return result.clone();
        }
        let result = (|| {
            let plane = self.file.bitmap(url, 2048).ok()??;
            // Include mip levels and the encoded PNG in the working-set estimate.
            if !self.reserve(plane.width as usize * plane.height as usize * 12) {
                return None;
            }
            let pixels = image::RgbaImage::from_fn(plane.width, plane.height, |x, y| {
                let value = plane.pixel(x, y);
                let rgb = if value.len() >= 3 {
                    [value[0], value[1], value[2]]
                } else {
                    [value[0]; 3]
                };
                image::Rgba([
                    byte(rgb[0]),
                    byte(rgb[1]),
                    byte(rgb[2]),
                    byte(value.get(3).copied().unwrap_or(1.)),
                ])
            });
            let name = url
                .split('?')
                .next()
                .unwrap_or_default()
                .rsplit('/')
                .next()
                .unwrap_or("Bitmap");
            Bitmap::from_rgba(name.to_owned(), pixels)
                .ok()
                .map(Arc::new)
        })();
        self.bitmaps.insert(url.to_owned(), result.clone());
        result
    }

    fn layers(&mut self, source: &[LayerInfo], target: &mut Vec<Layer>, depth: usize, end: usize) {
        for (index, original) in source.iter().enumerate() {
            if target.len() >= end {
                let skipped = count(&source[index..]);
                self.summary.unrestored += skipped;
                self.summary.layer_issues.push(format!(
                    "{} · слои сверх допустимого количества: {skipped}",
                    self.scope
                ));
                break;
            }
            let content_strokes = original
                .content
                .iter()
                .any(|action| action.enabled && stroke_parts(action).is_some());
            let kind = match original.kind {
                SourceKind::Group => LayerKind::Folder,
                SourceKind::Fill if !content_strokes => LayerKind::Fill,
                _ => LayerKind::Paint,
            };
            let name: String = original
                .name
                .chars()
                .filter(|c| !c.is_control())
                .take(120)
                .collect();
            let mut layer = Layer::new(
                if name.trim().is_empty() {
                    "Слой Painter".into()
                } else {
                    name
                },
                kind,
            );
            layer.depth = depth;
            let tile_hidden = self.tile.is_some_and(|tile| {
                original
                    .uv_filter
                    .as_ref()
                    .is_some_and(|(default, exceptions)| {
                        *default == exceptions.contains(&tile.indices())
                    })
            });
            layer.visible = original.visible && !tile_hidden;
            let mut partial = false;
            let mut placeholder = false;
            match kind {
                LayerKind::Folder => {
                    if depth < MAX_LAYER_DEPTH {
                        self.layers(&original.children, target, depth + 1, end - 1);
                    } else {
                        let skipped = count(&original.children);
                        self.summary.unrestored += skipped;
                        if skipped > 0 {
                            self.summary.layer_issues.push(format!(
                                "{} · «{}» · слои сверх допустимой вложенности: {skipped}",
                                self.scope, layer.name
                            ));
                        }
                    }
                    self.summary.groups += 1;
                }
                LayerKind::Fill => {
                    partial |= self.fill(original, &mut layer);
                    if layer.texture_fill.is_some() || layer.channels.iter().any(Option::is_some) {
                        self.summary.fills += 1;
                    } else {
                        placeholder = true;
                    }
                }
                _ if original.kind == SourceKind::Paint || content_strokes => {
                    for action in original.content.iter().filter(|action| action.enabled) {
                        if let ActionKind::Fill { sources, .. } = &action.kind {
                            let mut triangles = None;
                            let mut painted = false;
                            for source in sources {
                                match source {
                                    SourceInfo::Uniform { channels, color } => {
                                        let Some((replay, slot)) = self.polygon_replay else {
                                            partial = true;
                                            continue;
                                        };
                                        let triangles = triangles.get_or_insert_with(|| {
                                            replay.set_triangles(slot, self.size)
                                        });
                                        let (applied, omitted) = self
                                            .paint_fill(&mut layer, *channels, *color, triangles);
                                        painted |= applied;
                                        partial |= omitted;
                                    }
                                    SourceInfo::Reference {
                                        channels,
                                        anchor,
                                        levels,
                                    } => {
                                        partial |= self.reference_channels(
                                            &mut layer,
                                            None,
                                            *channels,
                                            *anchor,
                                            levels.as_ref(),
                                        );
                                    }
                                    _ => partial = true,
                                }
                            }
                            self.summary.polygon_fills += usize::from(painted);
                        } else if let Some(stroke) = stroke_parts(action) {
                            partial |= self.paint_stroke(&mut layer, action.channels, stroke);
                        } else if let ActionKind::Anchor { uid } = action.kind {
                            partial |= !self.capture_content(uid, &layer, None);
                        } else {
                            partial = true;
                        }
                    }
                }
                _ => placeholder = true,
            }
            if let Some(&(_, mode, opacity)) = original.blends.first() {
                layer.opacity = percent(opacity) as f32;
                match blend(mode) {
                    Some(blend) => layer.blend = blend,
                    None => partial = true,
                }
                partial |= original
                    .blends
                    .iter()
                    .any(|&(_, other, weight)| other != mode || weight != opacity);
            }
            if (original.has_mask && original.mask_enabled) || original.mesh_filter.is_some() {
                partial |= self.mask(original, &mut layer);
            }
            if placeholder {
                layer.visible = false;
                self.summary.unrestored += 1;
                self.summary.layer_issues.push(format!(
                    "{} · «{}» · содержимое не восстановлено",
                    self.scope, layer.name
                ));
            } else if partial {
                self.summary.partial += 1;
                self.summary.layer_issues.push(format!(
                    "{} · «{}» · восстановлен частично",
                    self.scope, layer.name
                ));
            }
            target.push(layer);
        }
    }

    fn solid_channels(&mut self, layer: &mut Layer, channels: u64, value: [f32; 3]) -> bool {
        let mut omitted = channels & !CHANNELS.iter().fold(0, |mask, (bit, ..)| mask | bit) != 0;
        for (index, (bit, ..)) in CHANNELS.into_iter().enumerate() {
            if channels & bit == 0 {
                continue;
            }
            let rgb = rgb(index, value);
            let pixels = self.plane([rgb[0], rgb[1], rgb[2], 255]);
            omitted |= pixels.is_none();
            if pixels.is_some() {
                layer.channels[index] = pixels;
            }
        }
        omitted
    }

    fn paint_plane<'a>(&mut self, plane: &'a mut Option<Arc<Vec<u8>>>) -> Option<&'a mut Vec<u8>> {
        if plane.as_ref().is_none_or(|p| Arc::strong_count(p) > 1) {
            if !self.reserve(self.size as usize * self.size as usize * 4) {
                return None;
            }
            *plane = Some(
                plane
                    .as_ref()
                    .map_or_else(|| solid(self.size, [0; 4]), |old| Arc::new((**old).clone())),
            );
        }
        plane.as_mut().map(Arc::make_mut)
    }

    fn paint_fill(
        &mut self,
        layer: &mut Layer,
        channels: u64,
        value: [f32; 3],
        triangles: &[[Vec2; 3]],
    ) -> (bool, bool) {
        if triangles.is_empty() {
            return (false, true);
        }
        let mut omitted = channels & !CHANNELS.iter().fold(0, |mask, (bit, ..)| mask | bit) != 0;
        let mut targets = Vec::new();
        for (index, ((bit, ..), plane)) in CHANNELS.iter().zip(&mut layer.channels).enumerate() {
            if channels & bit == 0 {
                continue;
            }
            if let Some(pixels) = self.paint_plane(plane) {
                let [r, g, b] = rgb(index, value);
                targets.push((pixels, [r, g, b, 255]));
            } else {
                omitted = true;
            }
        }
        let mut applied = false;
        if !targets.is_empty() {
            PolygonReplay::rasterize(triangles, self.size, |pixel| {
                for (plane, color) in &mut targets {
                    plane[pixel * 4..pixel * 4 + 4].copy_from_slice(color);
                }
                applied = true;
            });
        }
        (applied, omitted || !applied)
    }

    fn coverage(&mut self, stroke: &RecordedStroke<'_>) -> Option<StrokeCoverage> {
        let (replay, slot) = self.polygon_replay?;
        let bitmap = stroke
            .brush
            .stencil
            .as_ref()
            .and_then(|(url, _)| self.bitmap(url));
        let stencil = stroke.brush.stencil.as_ref().and_then(|(url, matrix)| {
            let wrap = stroke.brush.stencil_wrap;
            let repeats = [matches!(wrap, 1 | 3), matches!(wrap, 2 | 3)];
            if let Some(bitmap) = bitmap.as_deref() {
                Stencil::new(bitmap, matrix, repeats)
            } else {
                Stencil::procedural(url, &stroke.brush.stencil_tweaks, matrix, repeats, &mut self.stencil_cache)
            }
        });
        let tip = stroke
            .brush
            .tip_bitmap
            .as_ref()
            .and_then(|url| self.bitmap(url));
        // A temporary floating point plane is released after each recorded action.
        let bytes = (self.size * self.size * 4) as usize;
        if bytes > *self.remaining {
            return None;
        }
        let model_stroke = stroke.path.iter().any(|stroke| !stroke.uv_space);
        if model_stroke
            && self.uv_owners.is_none()
            && bytes * 2 <= *self.remaining
            && self.reserve(bytes)
        {
            self.uv_owners = Some(replay.uv_owners(slot, self.size));
        }
        let mut coverage = replay.stroke(
            stroke.path,
            stroke.brush,
            (slot, self.size, self.uv_owners.as_deref()),
            stencil.as_ref(),
            tip.as_deref(),
        );
        coverage.partial |= stroke.brush.stencil.is_some()
            && !(0..=3).contains(&stroke.brush.stencil_wrap);
        if model_stroke && self.uv_owners.is_some() && coverage.pixels(1.).next().is_some() {
            const PADDING: u32 = 2;
            if self.stroke_padding.is_none() && bytes * 2 <= *self.remaining && self.reserve(bytes)
            {
                self.stroke_padding = Some(PolygonReplay::padding_sources(
                    self.uv_owners.as_deref().unwrap(),
                    self.size,
                    PADDING,
                ));
            }
            if let Some(sources) = &self.stroke_padding {
                coverage.pad(sources, PADDING);
            } else {
                coverage.partial = true;
            }
        }
        Some(coverage)
    }

    fn paint_stroke(
        &mut self,
        layer: &mut Layer,
        channels: u64,
        stroke: RecordedStroke<'_>,
    ) -> bool {
        let Some(coverage) = self.coverage(&stroke) else {
            return true;
        };
        if coverage.pixels(stroke.opacity).next().is_none() {
            return coverage.partial;
        }
        let mut colors = [None; CHANNELS.len()];
        let mut partial = coverage.partial;
        for source in &stroke.brush.sources {
            if stroke.erase {
                break;
            }
            if let SourceInfo::Uniform {
                channels: source_channels,
                color,
            } = source
            {
                let selected = channels & source_channels;
                let supported = CHANNELS
                    .iter()
                    .fold(0, |bits, (channel, ..)| bits | channel);
                partial |= selected & !supported != 0;
                for (i, (bit, ..)) in CHANNELS.iter().enumerate() {
                    if selected & bit != 0 {
                        colors[i] = Some(rgb(i, *color));
                    }
                }
            } else {
                partial = true;
            }
        }
        if !stroke.erase && colors.iter().all(Option::is_none) {
            return true;
        }
        let mut applied = false;
        for (i, (bit, ..)) in CHANNELS.iter().enumerate() {
            if colors[i].is_none()
                && (!stroke.erase || layer.channels[i].is_none() || channels & bit == 0)
            {
                continue;
            }
            let Some(pixels) = self.paint_plane(&mut layer.channels[i]) else {
                partial = true;
                continue;
            };
            let color = colors[i].unwrap_or_default();
            for (index, weight) in coverage.pixels(stroke.opacity) {
                let target = &mut pixels[index..index + 4];
                let replacement = over(target.try_into().unwrap(), color, weight, stroke.erase);
                target.copy_from_slice(&replacement);
            }
            applied = true;
        }
        self.summary.stroke_actions += usize::from(applied);
        partial
    }

    fn fill(&mut self, original: &LayerInfo, layer: &mut Layer) -> bool {
        let mut fill = Fill::default();
        let mut omitted = false;
        let mut approximate_texture = false;
        let active: Vec<_> = original
            .content
            .iter()
            .filter_map(|action| match &action.kind {
                ActionKind::Fill {
                    sources,
                    projection,
                } if action.enabled => Some((sources, *projection, action.uv)),
                _ => None,
            })
            .collect();
        if let Some(&(_, projection, uv)) = active.first() {
            approximate_texture |= active
                .iter()
                .any(|(_, other, transform)| *other != projection || *transform != uv);
            if projection == 2 {
                fill.settings.projection = Projection::Triplanar;
            } else {
                approximate_texture |= projection != 0;
            }
            if let Some(uv) = uv {
                approximate_texture |= place(&mut fill, uv);
            }
        }
        for action in original.content.iter().filter(|action| action.enabled) {
            match &action.kind {
                ActionKind::Fill { sources, .. } => {
                    for source in sources {
                        omitted |= self.fill_source(layer, &mut fill, source);
                    }
                }
                ActionKind::Anchor { uid } => {
                    omitted |= !self.capture_content(*uid, layer, Some(&fill))
                }
                _ => omitted = true,
            }
        }
        if !original
            .content
            .iter()
            .any(|action| matches!(action.kind, ActionKind::Fill { .. }))
        {
            for source in &original.sources {
                omitted |= self.fill_source(layer, &mut fill, source);
            }
        }
        if fill.images.iter().any(Option::is_some) {
            omitted |= approximate_texture;
            layer.texture_fill = Some(Arc::new(fill));
        }
        omitted
    }

    fn fill_source(&mut self, layer: &mut Layer, fill: &mut Fill, source: &SourceInfo) -> bool {
        let mut omitted = false;
        match source {
            SourceInfo::Uniform { channels, color } => {
                omitted |= self.solid_channels(layer, *channels, *color);
                for (index, (bit, ..)) in CHANNELS.into_iter().enumerate() {
                    if channels & bit != 0 && layer.channels[index].is_some() {
                        fill.images[index] = None;
                    }
                }
            }
            SourceInfo::Bitmap { channels, url } => {
                let image = self.bitmap(url);
                for (index, (bit, ..)) in CHANNELS.into_iter().enumerate() {
                    if channels & bit != 0 {
                        if image.is_some()
                            && self.reserve(self.size as usize * self.size as usize * 4)
                        {
                            fill.images[index] = image.clone();
                        } else {
                            omitted = true;
                        }
                    }
                }
            }
            SourceInfo::Reference {
                channels,
                anchor,
                levels,
            } => {
                omitted |=
                    self.reference_channels(layer, Some(fill), *channels, *anchor, levels.as_ref());
            }
            _ => omitted = true,
        }
        omitted
    }

    fn capture_content(&mut self, uid: i64, layer: &Layer, fill: Option<&Fill>) -> bool {
        if !self.anchors.pending(uid) {
            return true;
        }
        let mut sampled = None;
        if let Some(fill) = fill {
            let planes = fill.images.iter().filter(|image| image.is_some()).count();
            if planes != 0 {
                if !self.reserve(planes * (self.size * self.size * 4) as usize) {
                    return false;
                }
                let mut fill = fill.clone();
                fill.bind(
                    self.mesh,
                    self.slot,
                    self.size,
                    self.tile.map_or(Vec2::ZERO, Tile::offset),
                    Transform::IDENTITY,
                    &layer.channels,
                );
                sampled = Some(fill.sampled);
            }
        }
        self.anchors.store(
            uid,
            Anchor::Content(std::array::from_fn(|index| {
                sampled
                    .as_ref()
                    .and_then(|planes| planes[index].clone())
                    .or_else(|| layer.channels[index].clone())
            })),
        );
        true
    }

    fn reference_channels(
        &mut self,
        layer: &mut Layer,
        mut fill: Option<&mut Fill>,
        channels: u64,
        uid: i64,
        levels: Option<&LevelsInfo>,
    ) -> bool {
        let mut applied = false;
        let supported = CHANNELS.iter().fold(0, |bits, (bit, ..)| bits | bit);
        let mut partial = channels & !supported != 0;
        for (index, (bit, ..)) in CHANNELS.into_iter().enumerate() {
            if channels & bit == 0 {
                continue;
            }
            let Some((source, gray)) = self.anchors.channel(uid, index) else {
                partial = true;
                continue;
            };
            let output = if let Some(levels) = levels {
                if !self.reserve(source.len()) {
                    partial = true;
                    continue;
                }
                Arc::new(
                    source
                        .chunks_exact(4)
                        .flat_map(|pixel| {
                            let mut rgba = [0, 0, 0, pixel[3]];
                            for i in 0..3 {
                                rgba[i] = byte(anchors::level(
                                    pixel[i] as f32 / 255.,
                                    Some(levels),
                                    if gray { 0 } else { i },
                                ));
                            }
                            rgba
                        })
                        .collect(),
                )
            } else {
                source
            };
            layer.channels[index] = Some(output);
            if let Some(fill) = fill.as_deref_mut() {
                fill.images[index] = None;
            }
            applied = true;
        }
        self.summary.anchor_references += usize::from(applied);
        partial || !applied
    }

    fn mask(&mut self, original: &LayerInfo, layer: &mut Layer) -> bool {
        let mut steps: Vec<_> = original
            .mask
            .iter()
            .filter(|action| original.mask_enabled && action.enabled)
            .collect();
        let covering = steps.iter().rposition(|action| {
            let (mode, opacity) = mask_blend(action);
            mode == MaskBlend::Normal && mask_blend_code(action) == 25 && opacity == 1.
                && matches!(&action.kind, ActionKind::Fill { sources, .. } if sources.len() == 1
                    && matches!(&sources[0], SourceInfo::Procedural { graph, .. } if generator(graph).is_some()))
        });
        let leading_generator = covering
            .filter(|&index| {
                !steps[..index].iter().any(|action| {
                    matches!(action.kind,
                ActionKind::Anchor { uid } if self.anchors.pending(uid))
                })
            })
            .and_then(|index| {
                let ActionKind::Fill { sources, .. } = &steps[index].kind else {
                    return None;
                };
                let SourceInfo::Procedural { graph, .. } = &sources[0] else {
                    return None;
                };
                let generator = generator(graph);
                steps.drain(..=index);
                generator
            });
        let value = if original.mask_initial_white
            || !original.mask_enabled
            || !original.has_mask
            || leading_generator.is_some()
        {
            255
        } else {
            0
        };
        let pixels = if leading_generator.is_some() {
            self.plane([255, 0, 0, 255])
        } else {
            self.plane([value; 4])
        };
        let Some(pixels) = pixels else {
            return true;
        };
        let mut mask = Mask::new(pixels, Vec::new());
        mask.affine = leading_generator.is_some();
        mask.generator = leading_generator;
        self.summary.mask_effects += usize::from(leading_generator.is_some());
        if let Some(generator) = leading_generator {
            self.summary.mask_generator(generator);
        }
        let mut before_paint = None;
        let mut omitted = leading_generator.is_some();
        let mut filters = Vec::new();
        for action in steps {
            if let ActionKind::Anchor { uid } = action.kind {
                omitted |= !self.capture_mask(uid, &mask, &filters);
                continue;
            }
            if let Some((filter, approximate)) = mask_filter(action) {
                if filters.len() == MAX_FILTERS {
                    omitted = true;
                } else {
                    filters.push(filter);
                    self.summary.mask_effects += 1;
                    omitted |= approximate;
                }
                continue;
            }
            if let ActionKind::Decal { hits } = &action.kind {
                let Some((replay, slot)) = self.polygon_replay else {
                    omitted = true;
                    continue;
                };
                let mut applied = false;
                for hit in hits {
                    let triangles = replay.triangles(hit, slot, self.size);
                    if triangles.is_empty() {
                        omitted = true;
                        continue;
                    }
                    if !self.prepare_mask_paint(&mut mask, &mut before_paint, &mut filters) {
                        omitted = true;
                        break;
                    }
                    let pixels = Arc::make_mut(&mut mask.pixels);
                    let affine = mask.affine;
                    PolygonReplay::rasterize(&triangles, self.size, |index| {
                        let index = index * 4;
                        apply_mask_value(
                            &mut pixels[index..index + 4],
                            affine,
                            MaskBlend::Normal,
                            hit.value,
                            1.,
                        );
                    });
                    applied = true;
                }
                self.summary.polygon_fills += usize::from(applied);
                omitted |= !applied;
                continue;
            }
            if let Some(stroke) = stroke_parts(action) {
                let Some(coverage) = self.coverage(&stroke) else {
                    omitted = true;
                    continue;
                };
                omitted |= coverage.partial;
                if coverage.pixels(stroke.opacity).next().is_some() {
                    if !self.prepare_mask_paint(&mut mask, &mut before_paint, &mut filters) {
                        omitted = true;
                        continue;
                    }
                    let pixels = Arc::make_mut(&mut mask.pixels);
                    let base = before_paint.as_ref().unwrap();
                    for (index, weight) in coverage.pixels(stroke.opacity) {
                        let target = &mut pixels[index..index + 4];
                        if stroke.erase {
                            for (dst, &src) in target[..3].iter_mut().zip(&base[index..index + 3]) {
                                *dst = byte(
                                    (*dst as f32 * (1. - weight) + src as f32 * weight) / 255.,
                                );
                            }
                            target[3] = 255;
                        } else {
                            apply_mask_value(
                                target,
                                mask.affine,
                                MaskBlend::Normal,
                                stroke.brush.value,
                                weight,
                            );
                        }
                    }
                    self.summary.stroke_actions += 1;
                }
                continue;
            }
            if let ActionKind::Fill {
                sources,
                projection,
            } = &action.kind
            {
                omitted |= self.mask_fill(
                    &mut mask,
                    &mut before_paint,
                    &mut filters,
                    action,
                    sources,
                    *projection,
                );
                continue;
            }
            omitted |= !matches!(action.kind, ActionKind::Anchor { .. });
        }
        *mask.edit_filters() = filters;
        if let Some((default_on, names)) = &original.mesh_filter {
            omitted |= !self.geometry_mask(&mut mask, *default_on, names);
        }
        self.summary.masks += 1;
        layer.mask = Some(Arc::new(mask));
        omitted
    }

    fn capture_mask(&mut self, uid: i64, mask: &Mask, filters: &[Filter]) -> bool {
        if !self.anchors.pending(uid) {
            return true;
        }
        if !self.can_evaluate_mask(mask) || !self.reserve(mask.pixels.len()) {
            return false;
        }
        let mut snapshot = mask.clone();
        snapshot.edit_filters().extend_from_slice(filters);
        self.bind_import_mask(&mut snapshot);
        let pixels = snapshot
            .effective()
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], 255])
            .collect();
        self.anchors.store(uid, Anchor::Mask(Arc::new(pixels)));
        true
    }

    fn bind_import_mask(&mut self, mask: &mut Mask) {
        if mask.uses_geometry() {
            let geometry = self.generator_surface.get_or_insert_with(|| {
                Arc::new(GeneratorSurface::new(
                    self.mesh.clone(),
                    self.slot,
                    self.size,
                    self.tile.map_or(Vec2::ZERO, Tile::offset),
                    Transform::IDENTITY,
                    self.baked.cloned(),
                ))
            });
            mask.bind(geometry.clone());
        }
    }

    fn can_evaluate_mask(&self, mask: &Mask) -> bool {
        mask.generator
            .into_iter()
            .chain(mask.layers.iter().filter_map(|layer| layer.generator))
            .all(|generator| !generator.missing_bake_inputs(self.baked.map(Arc::as_ref)))
    }

    fn flatten_mask(&mut self, mask: &mut Mask) {
        self.summary.baked_mask_generators += usize::from(mask.generator.is_some())
            + mask.layers.iter().filter(|layer| layer.generator.is_some()).count();
        self.bind_import_mask(mask);
        *mask = Mask::new(Arc::new(mask.effective().to_vec()), Vec::new());
    }

    fn geometry_mask(&mut self, mask: &mut Mask, default_on: bool, names: &[String]) -> bool {
        let Some((replay, slot)) = self.polygon_replay else {
            return false;
        };
        let triangles = replay.excluded_triangles(slot, self.size, default_on, names);
        if triangles.is_empty() {
            return true;
        }
        let area = (self.size * self.size) as usize;
        if !self.reserve(area) {
            return false;
        }
        let mut coverage = vec![255; area];
        PolygonReplay::rasterize(&triangles, self.size, |i| coverage[i] = 0);
        mask.set_geometry_coverage(coverage);
        self.summary.mesh_masks += 1;
        true
    }

    fn mask_fill(
        &mut self,
        mask: &mut Mask,
        before_paint: &mut Option<Arc<Vec<u8>>>,
        filters: &mut Vec<Filter>,
        action: &ActionInfo,
        sources: &[SourceInfo],
        projection: i64,
    ) -> bool {
        let (blend, opacity) = mask_blend(action);
        let mut partial =
            sources.is_empty() || !matches!(mask_blend_code(action), 25 | 4 | 10 | 29);
        for source in sources {
            match source {
                SourceInfo::Uniform { color, .. } if mask.affine && mask.layers.is_empty() => {
                    if !self.prepare_mask_paint(mask, before_paint, filters) {
                        partial = true;
                        continue;
                    }
                    for pixel in Arc::make_mut(&mut mask.pixels).chunks_exact_mut(4) {
                        apply_mask_value(pixel, mask.affine, blend, color[0], opacity);
                    }
                    partial |= mask.affine && blend == MaskBlend::Subtract;
                }
                SourceInfo::Uniform { color, .. }
                    if mask.layers.len() < MAX_MASK_LAYERS || !filters.is_empty() =>
                {
                    if !self.bake_mask_filters(mask, before_paint, filters) {
                        partial = true;
                        continue;
                    }
                    mask.layers.push(MaskLayer {
                        value: percent(color[0]),
                        opacity: percent(opacity),
                        blend,
                        ..MaskLayer::default()
                    });
                }
                SourceInfo::Bitmap { url, .. } => {
                    let Some(bitmap) = self.bitmap(url) else {
                        partial = true;
                        continue;
                    };
                    let transform = action.uv.unwrap_or(UvTransform {
                        scale: [1.; 2],
                        rotation: 0.,
                        offset: [0.; 2],
                        tiling: true,
                    });
                    let Some(mapping) = MaskBitmapTransform::new(transform) else {
                        partial = true;
                        continue;
                    };
                    if projection == 2 {
                        let applied = self.triplanar_mask_bitmap(mask, before_paint, filters,
                            &bitmap, &mapping, blend, opacity);
                        self.summary.mask_effects += usize::from(applied);
                        self.summary.triplanar_mask_bitmaps += usize::from(applied);
                        // Painter's projection volume is not available in the imported document.
                        partial = true;
                        continue;
                    }
                    if !self.prepare_mask_paint(mask, before_paint, filters) {
                        partial = true;
                        continue;
                    }
                    let size = self.size as usize;
                    for (index, pixel) in Arc::make_mut(&mut mask.pixels)
                        .chunks_exact_mut(4)
                        .enumerate()
                    {
                        let uv =
                            Vec2::new((index % size) as f32 + 0.5, (index / size) as f32 + 0.5)
                                / self.size as f32;
                        if let Some(value) = mapping.sample(&bitmap, uv, 0.) {
                            apply_mask_value(pixel, mask.affine, blend, value, opacity);
                        }
                    }
                    self.summary.mask_effects += 1;
                    partial |= projection != 0 || (mask.affine && blend == MaskBlend::Subtract);
                }
                SourceInfo::Reference {
                    channels,
                    anchor,
                    levels,
                } => {
                    let Some((source, gray)) = self.anchors.mask_source(*anchor, *channels) else {
                        partial = true;
                        continue;
                    };
                    if !self.prepare_mask_paint(mask, before_paint, filters) {
                        partial = true;
                        continue;
                    }
                    for (pixel, src) in Arc::make_mut(&mut mask.pixels)
                        .chunks_exact_mut(4)
                        .zip(source.chunks_exact(4))
                    {
                        let value = if gray {
                            src[0] as f32 / 255.
                        } else {
                            Vec3::new(src[0].into(), src[1].into(), src[2].into())
                                .dot(Vec3::new(0.2126, 0.7152, 0.0722))
                                / 255.
                        };
                        let strength = if gray {
                            opacity
                        } else {
                            (src[3] as f32 / 255.) * opacity
                        };
                        apply_mask_value(
                            pixel,
                            mask.affine,
                            blend,
                            anchors::level(value, levels.as_ref(), 0),
                            strength,
                        );
                    }
                    self.summary.anchor_references += 1;
                    partial |= projection != 0
                        || action.uv.is_some()
                        || (mask.affine && blend == MaskBlend::Subtract);
                }
                SourceInfo::Procedural { graph, .. }
                    if mask.layers.len() < MAX_MASK_LAYERS || !filters.is_empty() =>
                {
                    if let Some(generator) = generator(graph) {
                        if !self.bake_mask_filters(mask, before_paint, filters) {
                            partial = true;
                            continue;
                        }
                        mask.layers.push(MaskLayer {
                            generator: Some(generator),
                            opacity: percent(opacity),
                            blend,
                            ..MaskLayer::default()
                        });
                        self.summary.mask_effects += 1;
                        self.summary.mask_generator(generator);
                    }
                    partial = true;
                }
                _ => partial = true,
            }
        }
        partial
    }

    fn triplanar_mask_bitmap(
        &mut self,
        mask: &mut Mask,
        before_paint: &mut Option<Arc<Vec<u8>>>,
        filters: &mut Vec<Filter>,
        bitmap: &Bitmap,
        mapping: &MaskBitmapTransform,
        blend: MaskBlend,
        opacity: f32,
    ) -> bool {
        let texels = (self.size * self.size) as usize;
        let temporary_bytes = texels * (size_of::<[u8; 2]>() + size_of::<u8>())
            + self.mesh.positions.len() * size_of::<Vec3>();
        if !self.reserve(temporary_bytes) {
            return false;
        }
        let mut covered = vec![0; texels];
        let mut samples = vec![[0, 0]; texels];
        let offset = self.tile.map_or(Vec2::ZERO, Tile::offset);
        raster(self.mesh, self.slot, self.size, offset, Transform::IDENTITY,
            &self.mesh.positions, &mut covered, |index, _, position, normal, _, width| {
                let (uvs, weights) = triplanar_coordinates(position, normal, 4.);
                let mut value = 0.;
                let mut coverage = 0.;
                for (uv, weight) in uvs.into_iter().zip(weights.to_array()) {
                    if weight > 0. && let Some(sample) = mapping.sample(bitmap, uv, width) {
                        value += sample * weight;
                        coverage += weight;
                    }
                }
                if coverage > 0. {
                    samples[index] = [byte(value / coverage), byte(coverage)];
                }
            });
        let applied = samples.iter().any(|sample| sample[1] != 0)
            && self.prepare_mask_paint(mask, before_paint, filters);
        if applied {
            pad_texels(self.size as usize, &mut covered, |to, from| samples[to] = samples[from]);
            for (pixel, [value, coverage]) in Arc::make_mut(&mut mask.pixels).chunks_exact_mut(4).zip(samples) {
                if coverage != 0 {
                    apply_mask_value(pixel, mask.affine, blend, value as f32 / 255., opacity * coverage as f32 / 255.);
                }
            }
        }
        *self.remaining += temporary_bytes;
        applied
    }

    fn bake_mask_filters(
        &mut self,
        mask: &mut Mask,
        base: &mut Option<Arc<Vec<u8>>>,
        filters: &mut Vec<Filter>,
    ) -> bool {
        if filters.is_empty() {
            return true;
        }
        if !self.can_evaluate_mask(mask) || !self.reserve(mask.pixels.len() * 2) {
            return false;
        }
        self.summary.baked_mask_filters += filters.len();
        *mask.edit_filters() = std::mem::take(filters);
        self.flatten_mask(mask);
        *base = None;
        true
    }

    fn prepare_mask_paint(
        &mut self,
        mask: &mut Mask,
        base: &mut Option<Arc<Vec<u8>>>,
        filters: &mut Vec<Filter>,
    ) -> bool {
        if !self.bake_mask_filters(mask, base, filters) {
            return false;
        }
        if !mask.layers.is_empty() && !self.can_evaluate_mask(mask) {
            return false;
        }
        let planes = if !mask.layers.is_empty() {
            2
        } else {
            usize::from(base.is_none())
        };
        if !self.reserve(mask.pixels.len() * planes) {
            return false;
        }
        if !mask.layers.is_empty() {
            // Earlier fill effects become the mask underneath these brush actions.
            self.flatten_mask(mask);
            *base = None;
        }
        if base.is_none() {
            *base = Some(mask.pixels.clone());
        }
        true
    }
}

struct MaskBitmapTransform {
    scale: Vec2,
    center: Vec2,
    sin: f32,
    cos: f32,
    repeat: bool,
}

impl MaskBitmapTransform {
    fn new(transform: UvTransform) -> Option<Self> {
        let scale = Vec2::from_array(transform.scale);
        let offset = Vec2::from_array(transform.offset);
        if !scale.is_finite() || !offset.is_finite() || !transform.rotation.is_finite() {
            return None;
        }
        let (sin, cos) = transform.rotation.sin_cos();
        Some(Self { scale, center: Vec2::splat(0.5) + offset, sin, cos, repeat: transform.tiling })
    }

    fn sample(&self, bitmap: &Bitmap, uv: Vec2, width: f32) -> Option<f32> {
        let d = (Vec2::new(uv.x, 1. - uv.y) - self.center) * self.scale;
        let uv = Vec2::new(0.5 + d.x * self.cos - d.y * self.sin, 0.5 - d.x * self.sin - d.y * self.cos);
        if !uv.is_finite() || (!self.repeat && (uv.min_element() < 0. || uv.max_element() > 1.)) {
            return None;
        }
        let width = width * self.scale.abs().max_element();
        let rgba = if self.repeat { bitmap.sample(uv, width) } else { bitmap.sample_clamped(uv, width) };
        Some(rgba.truncate().dot(Vec3::new(0.2126, 0.7152, 0.0722)))
    }
}

struct RecordedStroke<'a> {
    brush: &'a BrushInfo,
    path: &'a [StrokeInfo],
    opacity: f32,
    erase: bool,
}

fn mask_blend_code(action: &ActionInfo) -> i32 {
    action
        .own_blends
        .first()
        .or(action.blends.first())
        .map_or(25, |&(_, mode, _)| mode)
}

fn mask_blend(action: &ActionInfo) -> (MaskBlend, f32) {
    let opacity = action
        .own_blends
        .first()
        .or(action.blends.first())
        .map_or(1., |&(_, _, opacity)| opacity);
    let mode = match mask_blend_code(action) {
        4 | 10 => MaskBlend::Multiply,
        29 => MaskBlend::Subtract,
        _ => MaskBlend::Normal,
    };
    (
        mode,
        if opacity.is_finite() {
            opacity.clamp(0., 1.)
        } else {
            0.
        },
    )
}

fn apply_mask_value(pixel: &mut [u8], affine: bool, blend: MaskBlend, value: f32, opacity: f32) {
    let mix = |base: u8| (base as f32 / 255.) * (1. - opacity) + value.clamp(0., 1.) * opacity;
    let scale = 1. + (value.clamp(0., 1.) - 1.) * opacity;
    let result = |base: u8| match blend {
        MaskBlend::Multiply => (base as f32 / 255.) * scale,
        MaskBlend::Subtract => base as f32 / 255. - value * opacity,
        _ => mix(base),
    };
    if affine {
        let factor = match blend {
            MaskBlend::Multiply => scale,
            MaskBlend::Subtract => 1.,
            _ => 1. - opacity,
        };
        pixel[0] = byte(pixel[0] as f32 / 255. * factor);
        pixel[1] = byte(result(pixel[1]));
        pixel[2] = 0;
    } else {
        let value = byte(result(pixel[0]));
        pixel[..3].fill(value);
    }
    pixel[3] = 255;
}

pub(super) fn mask_generators(layers: &[LayerInfo], generators: &mut Vec<Generator>) {
    for layer in layers {
        if layer.mask_enabled {
            for action in layer.mask.iter().filter(|action| action.enabled) {
                if let ActionKind::Fill { sources, .. } = &action.kind {
                    generators.extend(sources.iter().filter_map(|source| match source {
                        SourceInfo::Procedural { graph, .. } => generator(graph),
                        _ => None,
                    }));
                }
            }
        }
        mask_generators(&layer.children, generators);
    }
}

fn generator(graph: &GraphRef) -> Option<Generator> {
    let name = graph.name().to_ascii_lowercase();
    let kind = match name.as_str() {
        "mg_dirt" | "mg_ambient_occlusion" | "mg_mask_editor" | "mg_mask_builder" => {
            GeneratorKind::Dirt
        }
        "mg_curvature" => GeneratorKind::EdgeWear,
        "mg_position" => GeneratorKind::Height,
        "light" | "mg_light" | "mg_world_space_normals" => GeneratorKind::Dust,
        "mg_thickness" => GeneratorKind::ThinAreas,
        name if name.contains("edge_wear") => GeneratorKind::EdgeWear,
        name if name.contains("dust") => GeneratorKind::Dust,
        name if name.contains("leak") || name.contains("drip") => GeneratorKind::Drips,
        name if ["noise", "grunge", "fractal", "spots"]
            .iter()
            .any(|needle| name.contains(needle)) =>
        {
            GeneratorKind::Clouds
        }
        _ => return None,
    };
    let mut generator = Generator::new(kind);
    let value = |keys: &[&str]| keys.iter().find_map(|key| graph.value(key).filter(|v| v.is_finite()));
    let unit = |keys: &[&str]| value(keys).map(|v| v.clamp(0., 1.));
    let enabled = |key| unit(&[key]).is_some_and(|v| v > 0.5);
    generator.invert = unit(&["global_invert", "invert"]).is_some_and(|value| value > 0.5);
    // Substance balance raises coverage. Our generator's level is a threshold.
    if let Some(balance) = unit(&["global_balance", "dirt_level", "balance", "position"]) {
        generator.values[0] = (1. - balance) * 100.;
    }
    if let Some(contrast) = unit(&["global_contrast", "dirt_contrast", "contrast"]) {
        generator.values[1] = ((1. - contrast) * 50.).max(0.1);
    }
    if let Some(scale) = value(&["grunge_scale", "tiling_amount", "scale"]) {
        generator.values[2] = scale.clamp(0.1, 64.);
    }
    if let Some(amount) = unit(&["grunge_amount"]) {
        generator.values[3] = amount * 100.;
    }
    if let Some(seed) = value(&["$randomseed"]) {
        generator.values[4] = seed.round().rem_euclid(101.);
    }
    match name.as_str() {
        "mg_dirt" => {
            if let Some(amount) = unit(&["edges_masking"]) {
                generator.set_weight(GeneratorInput::Convexity, -(amount * 100.).round() as i16);
            }
        }
        "mg_ambient_occlusion" => {
            generator.values[3] = 0.;
            generator.set_weight(GeneratorInput::Cavity, 0);
            generator.set_weight(GeneratorInput::Concavity, 0);
            set_spp_map_weight(&mut generator, GeneratorInput::Occlusion, 1., !enabled("invert_ao"));
        }
        "mg_mask_editor" => {
            if let Some(ao) = unit(&["ao"]) {
                set_spp_map_weight(&mut generator, GeneratorInput::Occlusion, ao, !enabled("invert_ao"));
            }
            if let Some(curvature) = unit(&["curv"]) {
                generator.set_weight(GeneratorInput::Convexity, (curvature * 100.).round() as i16);
                generator.set_weight(GeneratorInput::Cavity, 0);
                generator.set_weight(GeneratorInput::Concavity, 0);
            }
            if let Some(thickness) = unit(&["thick"]) {
                set_spp_map_weight(&mut generator, GeneratorInput::Thin, thickness, !enabled("thick_invert"));
            }
            if let Some(position) = unit(&["position_gradient"]) {
                set_spp_map_weight(&mut generator, GeneratorInput::Height, position, enabled("grad_invert"));
            }
            if let Some((amount, scale)) = [(unit(&["tex"]), value(&["scale_tex"])), (unit(&["tex2"]), value(&["tex2_scale"]))]
                .into_iter().filter_map(|(amount, scale)| Some((amount?, scale)))
                .max_by(|a, b| a.0.total_cmp(&b.0))
            {
                generator.values[3] = amount * 100.;
                if let Some(scale) = scale {
                    generator.values[2] = scale.clamp(0.1, 64.);
                }
            }
        }
        _ => {}
    }
    generator.values[0] = generator.values[0].clamp(-100., 200.);
    Some(generator)
}

fn set_spp_map_weight(generator: &mut Generator, input: GeneratorInput, amount: f32, complement: bool) {
    let weight = (amount * 100.).round() as i16;
    if complement {
        // w × (1 - map) is equivalent to -w × map with a lower threshold.
        generator.values[0] -= f32::from(weight);
        generator.set_weight(input, -weight);
    } else {
        generator.set_weight(input, weight);
    }
}

fn mask_filter(action: &ActionInfo) -> Option<(Filter, bool)> {
    match &action.kind {
        ActionKind::Levels(levels) => {
            let mut filter = Filter::new(FilterKind::Levels);
            let wanted = [
                levels.in_min[0] * 100.,
                levels.in_max[0] * 100.,
                levels.gamma[0] * 100.,
                levels.out_min[0] * 100.,
                levels.out_max[0] * 100.,
            ];
            for (slot, value) in wanted.into_iter().enumerate() {
                filter.set_value(slot, value);
            }
            let partial = wanted
                .into_iter()
                .enumerate()
                .any(|(slot, value)| filter.value(slot) != value);
            Some((filter, partial))
        }
        ActionKind::Filter { graph } => {
            let name = graph.name().to_ascii_lowercase();
            let kind = match name.as_str() {
                "blur" => FilterKind::Blur,
                "sharpen" | "fx_sharpen" => FilterKind::Sharpen,
                "warp" if graph.value("source_type").is_none_or(|source| source == 0.) => {
                    FilterKind::Warp
                }
                "mask_outline" => FilterKind::Outline,
                "invert" | "invert_grayscale" => FilterKind::Invert,
                "threshold" => FilterKind::Threshold,
                "histogram_scan" => FilterKind::HistogramScan,
                "contrast_luminosity" => FilterKind::ContrastLuminosity,
                "blur_directional" => FilterKind::DirectionalBlur,
                "clamp" => FilterKind::Clamp,
                "smoothstep" => FilterKind::Smoothstep,
                "posterize" => FilterKind::Posterize,
                "pixelate" => FilterKind::Pixelate,
                _ => return None,
            };
            let mut filter = Filter::new(kind);
            let controls: &[(&str, f32, f32)] = match kind {
                FilterKind::Threshold => &[("threshold", 0.01, 100.), ("mode", 0., 1.)],
                FilterKind::HistogramScan => &[
                    ("position", 0.5, 100.), ("contrast", 0., 100.), ("invert_position", 0., 1.),
                ],
                FilterKind::ContrastLuminosity => &[("contrast", 0., 100.), ("luminosity", 0., 100.)],
                FilterKind::DirectionalBlur => &[("intensity", 1., 1.), ("mblurangle", 0., 360.)],
                FilterKind::Clamp | FilterKind::Smoothstep => &[("min", 0., 100.), ("max", 1., 100.)],
                FilterKind::Posterize => &[("colors_amount", 8., 1.)],
                FilterKind::Pixelate => &[("pixel_amount", 32., 1.)],
                _ => &[],
            };
            if !controls.is_empty() {
                let mut partial = !matches!(kind, FilterKind::Threshold | FilterKind::Clamp | FilterKind::Smoothstep);
                for (slot, &(key, default, scale)) in controls.iter().enumerate() {
                    let value = graph.value(key).unwrap_or(default) * scale;
                    filter.set_value(slot, value);
                    partial |= filter.value(slot) != value;
                }
                if matches!(kind, FilterKind::Clamp | FilterKind::Smoothstep)
                    && filter.value(0) >= filter.value(1)
                {
                    partial = true;
                }
                return Some((filter, partial));
            }
            let intensity = graph.value("intensity").filter(|v| v.is_finite());
            if kind == FilterKind::Outline {
                for (slot, key, units, default) in [
                    (0, "input_threshold", 100., 1.),
                    (1, "width", 100., 0.05),
                    (2, "mask_position", 1., 1.),
                    (3, "blur", 100., 0.),
                    (4, "curve", 100., 0.),
                ] {
                    let value = graph
                        .value(key)
                        .filter(|value| value.is_finite())
                        .unwrap_or(default);
                    filter.set_value(slot, value * units);
                }
                if let Some(value) = graph.value("switch") {
                    filter.set_value(5, if value >= 0.5 { 0. } else { 1. });
                }
            } else if kind == FilterKind::Warp {
                // The native warp is normalized to UV units and uses its own noise field.
                let divider = graph
                    .value("intensity_divider")
                    .filter(|v| v.is_finite() && *v > 0.)
                    .unwrap_or(1.);
                if let Some(value) = intensity {
                    filter.set_value(0, value / divider * 500.);
                }
                if let Some(tiling) = graph.value("noise_scale").filter(|v| v.is_finite()) {
                    filter.set_value(1, filter.value(1) * tiling);
                }
                if let Some(seed) = graph.value("$randomseed").filter(|v| v.is_finite()) {
                    filter.set_value(2, seed.round());
                }
            } else if let Some(intensity) = intensity {
                // Approximate Painter's blur intensity using the native radius at intensity 1.
                let units = if kind == FilterKind::Blur {
                    filter.values[0]
                } else {
                    1.
                };
                filter.set_value(0, intensity * units);
            }
            if kind == FilterKind::Blur {
                filter.wrap = graph.value("blur_tilling").is_some_and(|v| v >= 0.5);
            }
            Some((filter, kind != FilterKind::Invert))
        }
        _ => None,
    }
}

fn stroke_parts(action: &ActionInfo) -> Option<RecordedStroke<'_>> {
    let (strokes, brush, opacity, erase) = match &action.kind {
        ActionKind::Paint {
            strokes,
            brush,
            opacity,
        } => (strokes, brush.as_ref()?, *opacity, false),
        ActionKind::Eraser {
            strokes,
            brush,
            opacity,
        } => (strokes, brush.as_ref()?, *opacity, true),
        _ => return None,
    };
    Some(RecordedStroke {
        path: strokes,
        brush,
        opacity: opacity.clamp(0., 1.),
        erase,
    })
}

pub(super) fn count(layers: &[LayerInfo]) -> usize {
    layers.iter().map(|layer| 1 + count(&layer.children)).sum()
}

fn byte(value: f32) -> u8 {
    (value.clamp(0., 1.) * 255.).round() as u8
}

fn rgb(index: usize, value: [f32; 3]) -> [u8; 3] {
    match index {
        0 | 5 => {
            let c = Srgba::from(LinearRgba::new(value[0], value[1], value[2], 1.));
            [byte(c.red), byte(c.green), byte(c.blue)]
        }
        6 => [128, 128, 255],
        _ => [byte(value[0]); 3],
    }
}
fn percent(value: f32) -> u8 {
    (value.clamp(0., 1.) * 100.).round() as u8
}

fn blend(code: i32) -> Option<Blend> {
    Some(match code {
        25 => Blend::Normal,
        4 => Blend::Multiply,
        29 => Blend::Subtract,
        2 | 11 | 15 => Blend::Overlay,
        _ => return None,
    })
}

fn place(fill: &mut Fill, source: UvTransform) -> bool {
    let [u, v] = source.scale;
    let scale = (u + v) / 2.;
    let (sin, cos) = source.rotation.sin_cos();
    let [x, y] = source.offset;
    let offset = [-scale * (x * cos - y * sin), scale * (x * sin + y * cos)];
    let approximated = (u - v).abs() > 1e-4
        || !source.tiling
        || !(0.05..=64.).contains(&scale)
        || offset.iter().any(|&value| !(-1. ..=1.).contains(&value));
    if scale.is_finite() && source.rotation.is_finite() && offset.iter().all(|v| v.is_finite()) {
        fill.settings.tiling = scale.clamp(0.05, 64.);
        fill.settings.rotation = (-source.rotation.to_degrees() + 180.).rem_euclid(360.) - 180.;
        fill.settings.offset = offset.map(|value| value.rem_euclid(1.));
    }
    approximated
}
