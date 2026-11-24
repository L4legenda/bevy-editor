//! Copy texture data at unchanged local UVs when two UDIM sets can share a stack.
use super::{MaterialSlot, Object};
use crate::editor::model_paint_data::{DEFAULT_SIZE, Layer, Surface, mask::FilterKind};
use std::{ops::Range, sync::Arc};

const GUTTER: usize = 2;

pub(super) fn surfaces(
    source: &Object,
    destination: &Object,
    source_slot: usize,
    target_slot: usize,
    moved: &[usize],
) -> Option<Option<Arc<Surface>>> {
    let from = &source.materials[source_slot];
    let to = &destination.materials[target_slot];
    let paint = match (&from.paint, &to.paint) {
        (Some(a), Some(b)) if compatible_surfaces(a, b) => Some((a, b)),
        (None, None)
            if from.material == to.material && from.source["material"] == to.source["material"] =>
        {
            None
        }
        _ => return None,
    };
    if metadata(from) != metadata(to) || from.authored != to.authored || from.bake_fixes != to.bake_fixes {
        return None;
    }
    let size = paint.map_or(DEFAULT_SIZE, |(a, _)| a.size);
    let selected = moved.iter().map(|&index| &source.geometry.faces[index]);
    let occupied = destination
        .geometry
        .faces
        .iter()
        .filter(|f| f.material == target_slot);
    let (selected, selected_complete) =
        source
            .geometry
            .uv_coverage(selected, source.uv_offset(source_slot), size);
    let (occupied, occupied_complete) =
        destination
            .geometry
            .uv_coverage(occupied, destination.uv_offset(target_slot), size);
    if !selected_complete || !occupied_complete {
        return None;
    }
    let moved = padded(&selected, size as usize);
    let reserved = padded(&occupied, size as usize);
    if moved.iter().zip(reserved).any(|(a, b)| *a && b) {
        return None;
    }
    let Some((from, to)) = paint else {
        return Some(None);
    };
    if Arc::ptr_eq(from, to) {
        return Some(Some(to.clone()));
    }

    let bytes = moved.len() * 4;
    let regions: Vec<_> = runs(&moved).map(|run| run.start * 4..run.end * 4).collect();
    let mut merged = (**to).clone();
    for (input, output) in from.base.iter().zip(&mut merged.base) {
        copy(&regions, Some(input), output);
    }
    for (input, output) in from.stack.layers.iter().zip(&mut merged.stack.layers) {
        for (src, dst) in input.channels.iter().zip(&mut output.channels) {
            if src.is_some() || dst.is_some() {
                let target = dst.get_or_insert_with(|| Arc::new(vec![0; bytes]));
                copy(&regions, src.as_ref(), target);
            }
        }
        if let (Some(src), Some(dst)) = (&input.mask, &mut output.mask)
            && src.pixels != dst.pixels
        {
            let mut pixels = dst.pixels.clone();
            copy(&regions, Some(&src.pixels), &mut pixels);
            *dst = Arc::new(dst.with_pixels(pixels));
        }
    }
    Some(Some(Arc::new(merged)))
}

fn metadata(slot: &MaterialSlot) -> serde_json::Value {
    let mut value = slot.source.clone();
    if let Some(source) = value.as_object_mut() {
        for key in ["attributes", "indices", "mode", "material"] {
            source.remove(key);
        }
        if let Some(extras) = source
            .get_mut("extras")
            .and_then(serde_json::Value::as_object_mut)
        {
            extras.remove("bevy_editor_udim");
            extras.remove("bevy_editor_paint");
            extras.remove(crate::editor::model_paint_data::map_fixes::EXTRAS);
            if extras.is_empty() {
                source.remove("extras");
            }
        }
    }
    value
}

fn compatible_surfaces(a: &Surface, b: &Surface) -> bool {
    a.size > 0
        && a.size <= 4096
        && a.size == b.size
        && a.material == b.material
        && a.baked.is_none()
        && b.baked.is_none()
        && a.stack.layers.len() == b.stack.layers.len()
        && a.stack
            .layers
            .iter()
            .zip(&b.stack.layers)
            .all(|(a, b)| compatible_layers(a, b))
}

fn compatible_layers(a: &Layer, b: &Layer) -> bool {
    let compatible_mask = match (&a.mask, &b.mask) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            !a.uses_geometry()
                && a.graph.is_none()
                && a.geometry_coverage().is_none()
                && a.filters.iter().all(|f| {
                    matches!(
                        f.kind,
                        FilterKind::Levels | FilterKind::Contrast | FilterKind::Invert
                    )
                })
                && a.same_settings(b)
        }
        _ => false,
    };
    if !compatible_mask || a.texture_fill.is_some() || b.texture_fill.is_some() {
        return false;
    }
    let settings = |layer: &Layer| Layer {
        channels: Default::default(),
        mask: None,
        ..layer.clone()
    };
    settings(a) == settings(b) && a.folder_graph.is_none()
}

fn runs(covered: &[bool]) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        start += covered[start..].iter().position(|&value| value)?;
        let end = covered[start..]
            .iter()
            .position(|&value| !value)
            .map_or(covered.len(), |n| start + n);
        let run = start..end;
        start = end;
        Some(run)
    })
}

fn padded(covered: &[bool], size: usize) -> Vec<bool> {
    let mut result = vec![false; covered.len()];
    for (y, row) in covered.chunks_exact(size).enumerate() {
        for run in runs(row) {
            let start = run.start.saturating_sub(GUTTER);
            let end = (run.end + GUTTER).min(size);
            for row in y.saturating_sub(GUTTER)..(y + GUTTER + 1).min(size) {
                result[row * size + start..row * size + end].fill(true);
            }
        }
    }
    result
}

fn copy(regions: &[Range<usize>], source: Option<&Arc<Vec<u8>>>, target: &mut Arc<Vec<u8>>) {
    if source.is_some_and(|source| source == target) {
        return;
    }
    let target = Arc::make_mut(target);
    for region in regions {
        let output = &mut target[region.clone()];
        if let Some(source) = source {
            output.copy_from_slice(&source[region.clone()]);
        } else {
            output.fill(0);
        }
    }
}
