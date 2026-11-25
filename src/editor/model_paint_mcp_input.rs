//! Convert MCP points into the same queued strokes as pointer input.
use super::*;
use crate::editor::ui::model_viewer::paint::mcp_state::{
    BrushValues, PaintSpace, RasterOperation, RasterTarget, StrokeSample,
};

#[derive(SystemParam)]
pub(in crate::editor) struct McpPaintInput<'w, 's> {
    runtime: Option<ResMut<'w, Runtime>>,
    viewer: Option<Res<'w, Viewer>>,
    canvas: Query<'w, 's, (&'static ComputedNode, &'static UiGlobalTransform), With<Canvas>>,
}

impl McpPaintInput<'_, '_> {
    pub(in crate::editor::ui::model_viewer::paint) fn submit(
        &mut self,
        model: usize,
        doc: &mut Document,
        session: &mut Session,
        tips: &tips::Library,
        target: RasterTarget,
        values: BrushValues,
        operation: RasterOperation<'_>,
    ) -> Result<(), String> {
        let viewer = self
            .viewer
            .as_deref()
            .ok_or("The model viewport is unavailable")?;
        let runtime = self
            .runtime
            .as_deref_mut()
            .ok_or("The paint engine is unavailable")?;
        if runtime.gesture.is_some()
            || runtime.brush_adjustment.is_some()
            || session.panel == Panel::Shader
            || session.graph_canvas_open()
            || session.cage_brush_active()
            || session.custom_gizmo_active()
            || session.showing_bake()
            || session.diagnostic_bake().is_some()
        {
            return Err("Finish the current paint gesture or return to the paint workspace".into());
        }
        let (object, slot) = doc.paint_target(session.set).ok_or("Unknown texture set")?;
        let key = (model, object.id, slot);
        if object.hidden
            || !session.set_appearance.visibility.shown(object.id, slot)
            || !has_uv(object, slot)
        {
            return Err("Painting requires a visible texture set with UVs".into());
        }
        let live = runtime
            .lives
            .get_mut(&key)
            .filter(|live| live.gpu.is_some() && live.work.is_some())
            .ok_or("Wait for the GPU paint workspace to be ready")?;
        let uv = target.space == PaintSpace::Uv;
        let rect = if uv {
            Rect::from_corners(Vec2::ZERO, Vec2::splat(session.texture_size as f32))
        } else {
            if session.view == View::Two {
                return Err("Open the 3D viewport before using viewport coordinates".into());
            }
            self.canvas
                .single()
                .ok()
                .and_then(|(node, transform)| node_rect(node, transform))
                .ok_or("The 3D viewport has no visible canvas")?
        };
        queue_operation(
            live,
            doc,
            object,
            slot,
            session,
            tips,
            camera_map(viewer, rect),
            target,
            values,
            operation,
        )?;
        session.busy = true;
        doc.paint_busy = true;
        doc.paint_pending = true;
        Ok(())
    }
}

fn queue_operation(
    live: &mut Live,
    doc: &Document,
    object: &Object,
    slot: usize,
    session: &mut Session,
    tips: &tips::Library,
    camera: CameraMap,
    target: RasterTarget,
    values: BrushValues,
    operation: RasterOperation<'_>,
) -> Result<(), String> {
    let layer = session
        .stack
        .layers
        .get(target.layer)
        .ok_or("Unknown layer index")?;
    if !session.stack.shown(target.layer)
        || (target.mask && layer.mask.is_none())
        || (!target.mask && layer.kind != LayerKind::Paint)
    {
        return Err("Select a visible paint layer or a visible layer with a mask".into());
    }
    let colors = values.channels.as_ref().map(|channels| channels.colors());
    if colors.is_some_and(|colors| target.mask || colors.iter().all(Option::is_none))
        || values.mask_value.is_some() && (!target.mask || values.erase)
    {
        return Err(
            "Specify material channels for content, or a mask value for mask painting".into(),
        );
    }
    let size = brush_value(values.size_px, Parameter::Size, session.size)?;
    let flow = brush_value(values.flow_percent, Parameter::Flow, session.flow)?;
    let hardness = brush_value(
        values.hardness_percent,
        Parameter::Hardness,
        session.hardness,
    )?;
    let opacity = brush_value(values.opacity_percent, Parameter::Opacity, session.opacity)?;
    let spacing = brush_value(values.spacing_percent, Parameter::Spacing, session.spacing)?;
    let (stamps, fill_mode) = match operation {
        RasterOperation::Stroke(samples) => (
            stroke_stamps(samples, camera.rect, session, size * spacing / 100.)?,
            None,
        ),
        RasterOperation::Fill(position, mode) => (
            vec![Stamp::from(point(position, camera.rect)?)],
            Some(mode.unwrap_or(session.fill_mode)),
        ),
    };

    if session.stack.selected != target.layer || session.editing_mask != target.mask {
        let select = if target.mask {
            PaintAction::SelectLayerMask(target.layer)
        } else {
            PaintAction::Select(target.layer)
        };
        session.edit(select);
    }
    let uv = target.space == PaintSpace::Uv;
    let mut brush = brush_for(session, tips, object, uv, values.erase, fill_mode.is_some());
    brush.radius = size * 0.5;
    brush.flow = flow / 100.;
    brush.hardness = hardness / 100.;
    if let Some(mode) = fill_mode {
        brush.fill_mode = mode;
    }
    if let Some(colors) = colors {
        let channels = colors.iter().enumerate().fold(0, |bits, (i, color)| {
            bits | (u8::from(color.is_some()) << i)
        });
        brush.material = Some(BrushMaterial {
            channels,
            colors: colors.map(Option::unwrap_or_default),
        });
        brush = brush.for_channel(channels.trailing_zeros() as usize);
    }
    if let Some(value) = values.mask_value {
        brush.color = [value; 3];
    }
    if brush.channel != 4 || brush.material.is_some() {
        brush.opacity = opacity / 100.;
    } else if values.opacity_percent.is_some() {
        // In single-channel opacity mode this control is the value of the map.
        brush.color = [(opacity * 2.55).round() as u8; 3];
    }
    let map = Arc::new(Mapping::for_camera(
        doc,
        object,
        slot,
        camera,
        uv,
        &session.set_appearance.visibility,
    ));
    live.queue.push_back(Event::Begin(brush));
    live.enqueue_stamps(stamps, map);
    live.queue.push_back(Event::End);
    Ok(())
}

fn brush_value(input: Option<f32>, parameter: Parameter, fallback: f32) -> Result<f32, String> {
    let (min, max) = parameter.bounds();
    if input.is_some_and(|value| !(min..=max).contains(&f64::from(value))) {
        return Err(format!(
            "Invalid brush setting {}; expected {min}–{max}",
            parameter.label()
        ));
    }
    Ok(input.unwrap_or(fallback))
}

fn point(position: [f32; 2], rect: Rect) -> Result<Vec2, String> {
    if position.iter().any(|value| !(0. ..=1.).contains(value)) {
        return Err("Paint coordinates must be in the range 0–1".into());
    }
    Ok(rect.min + Vec2::from_array(position) * rect.size())
}

fn stroke_stamps(
    samples: &[StrokeSample],
    rect: Rect,
    session: &Session,
    spacing: f32,
) -> Result<Vec<Stamp>, String> {
    const MAX_SAMPLES: usize = 512;
    const MAX_STAMPS: usize = 16_384;
    if samples.is_empty() || samples.len() > MAX_SAMPLES {
        return Err(format!(
            "Provide between 1 and {MAX_SAMPLES} stroke samples"
        ));
    }
    let sample = |sample: &StrokeSample| -> Result<(Vec2, f32), String> {
        let pressure = sample.pressure.unwrap_or(1.);
        if !(0. ..=1.).contains(&pressure) {
            return Err("Stroke pressure must be in the range 0–1".into());
        }
        Ok((point(sample.position, rect)?, pressure))
    };
    let (first, pressure) = sample(&samples[0])?;
    let mut path = stroke::Path::new(
        first,
        pressure,
        spacing,
        session.stabilizer,
        session.pressure_size,
        session.pressure_flow,
    );
    let mut stamps = vec![path.first()];
    for entry in &samples[1..] {
        let (position, pressure) = sample(entry)?;
        stamps.extend(path.advance(position, pressure));
        if stamps.len() > MAX_STAMPS {
            return Err("The stroke exceeds the stamp limit; split it into shorter strokes".into());
        }
    }
    Ok(stamps)
}
