//! Convert MCP points into the same queued strokes as pointer input.
use super::*;
use crate::editor::ui::model_viewer::paint::mcp_state::{
    BrushValues, FillRegion, FillSource, PaintSpace, RasterOperation, RasterTarget, RasterView, StrokeOptions,
    StrokeSample, SurfaceSeed,
};
use crate::editor::ui::model_viewer::paint::surface_selection::{self, Selection, Source};

#[derive(SystemParam)]
pub(in crate::editor) struct McpPaintInput<'w, 's> {
    runtime: Option<ResMut<'w, Runtime>>,
    gpu_queue: Option<ResMut<'w, gpu::Queue>>,
    viewer: Option<Res<'w, Viewer>>,
    images: Option<ResMut<'w, Assets<Image>>>,
    canvas: Query<'w, 's, (&'static ComputedNode, &'static UiGlobalTransform), With<Canvas>>,
}

impl McpPaintInput<'_, '_> {
    pub(in crate::editor::ui::model_viewer::paint) fn surface_source(
        &self,
        model: usize,
        session: &Session,
        seed: SurfaceSeed,
    ) -> Result<Source, String> {
        Ok(match seed {
            SurfaceSeed::Parts { indices } => Source::Parts(indices),
            SurfaceSeed::Point { position } => {
                let point = Vec3::from_array(position);
                if !point.is_finite() {
                    return Err("The world-space selection point must be finite".into());
                }
                Source::Point(point)
            }
            SurfaceSeed::Viewport { position, view } => {
                let camera = self.viewport_camera(model, session, view)?;
                let (origin, direction) = camera.ray(point(position, camera.rect)?);
                Source::Ray(origin, direction)
            }
        })
    }

    fn viewport_camera(&self, model: usize, session: &Session, view: Option<RasterView>) -> Result<CameraMap, String> {
        let viewer = self.viewer.as_deref().ok_or("The model viewport is unavailable")?;
        if let Some(view) = view {
            if !(16..=4096).contains(&view.width) || !(16..=4096).contains(&view.height) {
                return Err("Request view width and height must be between 16 and 4096 pixels".into());
            }
            if viewer.model != Some(model) || !viewer.fitted || viewer.pending_document_camera {
                return Err("Wait for the model camera to load".into());
            }
            let current = viewer.camera();
            let camera = view.camera.map_or(Ok(current), |patch| patch.apply(current))?;
            let rect = Rect::from_corners(Vec2::ZERO, Vec2::new(view.width as f32, view.height as f32));
            return Ok(projection::model_camera_map(camera, viewer.radius, rect));
        }
        if session.view == View::Two {
            return Err("Open the 3D viewport or supply a request view".into());
        }
        let rect = self.canvas.single().ok()
            .and_then(|(node, transform)| node_rect(node, transform))
            .ok_or("The 3D viewport has no visible canvas")?;
        Ok(camera_map(viewer, rect))
    }

    pub(in crate::editor::ui::model_viewer::paint) fn submit_surface(
        &mut self,
        model: usize,
        doc: &mut Document,
        session: &mut Session,
        layer: usize,
        selection: Selection,
    ) -> Result<(), String> {
        let runtime = self.runtime.as_deref_mut().ok_or("The paint engine is unavailable")?;
        check_gesture(runtime, session)?;
        let queue = self.gpu_queue.as_deref_mut().ok_or("The paint engine is unavailable")?;
        if layer >= session.stack.layers.len() || !session.stack.shown(layer) {
            return Err("Select a visible layer".into());
        }
        let (object, slot) = doc.paint_target(session.set).ok_or("Unknown texture set")?;
        let target = (object.id, slot);
        if object.hidden || !session.set_appearance.visibility.shown(target.0, target.1)
            || !has_uv(object, slot)
        {
            return Err("Selection requires a visible texture set with UVs".into());
        }
        let live = runtime.lives.get_mut(&(model, target.0, target.1))
            .filter(|live| live.task.is_none() && live.queue.is_empty() && !live.controller.busy())
            .ok_or("Wait for the active texture set to be ready")?;
        if !data::same_layer_list(&session.stack, &live.ui_stack) {
            return Err("Wait for the active layer changes to finish".into());
        }
        let mut work = live.work.take().ok_or("Wait for the active texture set to be ready")?;
        let objects = doc.objects.clone();
        let visibility = session.set_appearance.visibility.clone();
        work.surface.stack.selected = session.stack.selected;
        live.controller.invalidate(queue);
        live.task = Some(AsyncComputeTaskPool::get().spawn(async move {
            let before = work.surface.stack.clone();
            let result = surface_selection::apply(
                &mut work.surface.stack, layer, work.surface.size,
                &objects, target, &visibility, selection,
            );
            let changed = matches!(&result, Ok((_, true)));
            if result.is_ok() {
                work.surface.stack.selected = layer;
            }
            let mut batch = if changed || before.selected != work.surface.stack.selected {
                compose(work)
            } else {
                Batch { work, maps: None, history: Vec::new(), surface_selection: None }
            };
            if changed {
                batch.history.push(before);
            }
            batch.surface_selection = Some(result.map(|(report, _)| report));
            batch
        }));
        session.surface_selection = None;
        session.busy = true;
        doc.paint_busy = true;
        doc.paint_pending = true;
        Ok(())
    }

    pub(in crate::editor::ui::model_viewer::paint) fn submit(
        &mut self,
        model: usize,
        doc: &mut Document,
        session: &mut Session,
        tips: &mut tips::Library,
        target: RasterTarget,
        values: BrushValues,
        operation: RasterOperation<'_>,
    ) -> Result<(), String> {
        let camera = if target.space == PaintSpace::Uv {
            if target.view.is_some() {
                return Err("A request view requires viewport coordinates".into());
            }
            let viewer = self.viewer.as_deref().ok_or("The model viewport is unavailable")?;
            let rect = Rect::from_corners(Vec2::ZERO, Vec2::splat(session.texture_size as f32));
            camera_map(viewer, rect)
        } else {
            self.viewport_camera(model, session, target.view)?
        };
        let runtime = self
            .runtime
            .as_deref_mut()
            .ok_or("The paint engine is unavailable")?;
        check_gesture(runtime, session)?;
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
        queue_operation(
            live,
            doc,
            object,
            slot,
            session,
            tips,
            self.images.as_deref_mut(),
            camera,
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

fn check_gesture(runtime: &Runtime, session: &Session) -> Result<(), String> {
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
    Ok(())
}

fn queue_operation(
    live: &mut Live,
    doc: &Document,
    object: &Object,
    slot: usize,
    session: &mut Session,
    tips: &mut tips::Library,
    images: Option<&mut Assets<Image>>,
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
    let stroke_opacity = values.stroke_opacity_percent
        .map(|value| brush_value(Some(value), Parameter::Opacity, session.opacity))
        .transpose()?;
    let spacing = brush_value(values.spacing_percent, Parameter::Spacing, session.spacing)?;
    let uv = target.space == PaintSpace::Uv;
    let mut style = StrokeStyle::from(&*session);
    let (points, fill) = match operation {
        RasterOperation::Stroke(samples, options) => {
            let mut dynamics = (session.stabilizer, session.pressure_size, session.pressure_flow);
            if let Some(options) = options {
                override_style(&mut style, options, tips, uv)?;
                dynamics = (
                    brush_value(options.stabilizer_px, Parameter::Stabilizer, dynamics.0)?,
                    options.pressure_size.unwrap_or(dynamics.1),
                    options.pressure_flow.unwrap_or(dynamics.2),
                );
            }
            let stamps = stroke_stamps(samples, camera.rect, size * spacing / 100., dynamics)?;
            if style.tip.is_some() && tips.entries.is_empty() || style.grain.is_some() && tips.grains.is_empty() {
                let images = images.ok_or("Brush textures are unavailable")?;
                if style.tip.is_some() { tips.ensure(images); }
                if style.grain.is_some() { tips.ensure_grains(images); }
            }
            (RasterPoints::Stamps(stamps), None)
        }
        RasterOperation::Fill(source, settings) => {
            let points = match source {
                FillSource::Point(position) => RasterPoints::Stamps(vec![Stamp::from(point(position, camera.rect)?)]),
                FillSource::Region(region) => RasterPoints::Region(fill_region(region, camera.rect, uv, session.fill_through)?),
            };
            let angle = brush_value(settings.angle, Parameter::FillAngle, session.fill_angle)?;
            let padding = brush_value(settings.padding, Parameter::FillPadding, session.fill_padding)?;
            (points, Some((settings.mode.unwrap_or(session.fill_mode), angle, padding)))
        }
    };

    if session.stack.selected != target.layer || session.editing_mask != target.mask {
        let select = if target.mask {
            PaintAction::SelectLayerMask(target.layer)
        } else {
            PaintAction::Select(target.layer)
        };
        session.edit(select);
    }
    let mut brush = brush_for_style(session, tips, object, uv, values.erase, fill.is_some(), style);
    brush.radius = size * 0.5;
    brush.flow = flow / 100.;
    brush.hardness = hardness / 100.;
    if let Some((mode, angle, padding)) = fill {
        brush.fill_mode = mode;
        brush.fill_angle = angle;
        brush.fill_padding = padding;
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
    if brush.channel == 4 && brush.material.is_none() && values.opacity_percent.is_some() {
        brush.color = [(opacity * 2.55).round() as u8; 3];
        brush.opacity = stroke_opacity.unwrap_or(100.) / 100.;
    } else {
        brush.opacity = stroke_opacity.unwrap_or(opacity) / 100.;
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
    match points {
        RasterPoints::Stamps(stamps) => live.enqueue_stamps(stamps, map),
        RasterPoints::Region(region) => live.queue.push_back(Event::Region(region, map)),
    }
    live.queue.push_back(Event::End);
    Ok(())
}

enum RasterPoints {
    Stamps(Vec<Stamp>),
    Region(fill_drag::Region),
}

fn fill_region(region: &FillRegion, rect: Rect, uv: bool, default_through: bool) -> Result<fill_drag::Region, String> {
    let through = match region {
        FillRegion::Rectangle { through, .. } | FillRegion::Lasso { through, .. } => *through,
    };
    if uv && through == Some(true) {
        return Err("Selecting hidden surfaces requires the 3D viewport".into());
    }
    let through = !uv && through.unwrap_or(default_through);
    match region {
        FillRegion::Rectangle { start, end, .. } => {
            let start = point(*start, rect)?;
            let end = point(*end, rect)?;
            if start.x == end.x || start.y == end.y {
                return Err("A fill rectangle must have nonzero width and height".into());
            }
            Ok(fill_drag::Region::rectangle(start, end, through))
        }
        FillRegion::Lasso { points, .. } => {
            let max = fill_drag::MAX_POINTS;
            if !(3..=max).contains(&points.len()) {
                return Err(format!("Provide between 3 and {max} lasso points"));
            }
            let outline: Vec<_> = points.iter().map(|&p| point(p, rect)).collect::<Result<_, _>>()?;
            let origin = outline[0];
            let spans_area = outline.iter().find(|&&p| p != origin).is_some_and(|&p| {
                outline.iter().any(|&q| (p - origin).perp_dot(q - origin) != 0.)
            });
            if !spans_area {
                return Err("A fill lasso must enclose an area".into());
            }
            Ok(fill_drag::Region::lasso(outline, through))
        }
    }
}

fn override_style(style: &mut StrokeStyle, options: &StrokeOptions, tips: &tips::Library, uv: bool) -> Result<(), String> {
    style.projection = options.projection.unwrap_or(style.projection);
    style.symmetry = options.symmetry.unwrap_or(style.symmetry);
    style.paint_through = options.paint_through.unwrap_or(style.paint_through);
    if (uv || style.projection != BrushProjection::Screen) && options.paint_through == Some(true) {
        return Err("Paint-through requires screen projection in the 3D viewport".into());
    }
    if uv && (options.projection == Some(BrushProjection::Surface)
        || options.symmetry.is_some_and(|axis| axis != Symmetry::Off))
    {
        return Err("Surface projection and model symmetry require the 3D viewport".into());
    }
    style.angle_limit = brush_value(options.angle_limit_degrees, Parameter::BrushAngleLimit, style.angle_limit)?;
    if let Some(name) = &options.tip {
        style.tip = brush_texture(name, "round", "tip", tips.names())?;
    }
    style.tip_angle = brush_value(options.tip_angle_degrees, Parameter::TipAngle, style.tip_angle)?;
    style.follow_stroke = options.follow_stroke.unwrap_or(style.follow_stroke);
    if let Some(name) = &options.grain {
        style.grain = brush_texture(name, "none", "grain", tips.grain_names())?;
    }
    style.grain_strength = brush_value(options.grain_strength_percent, Parameter::GrainStrength, style.grain_strength)?;
    style.grain_scale = brush_value(options.grain_scale, Parameter::GrainScale, style.grain_scale)?;
    style.grain_angle = brush_value(options.grain_angle_degrees, Parameter::GrainAngle, style.grain_angle)?;
    style.grain_invert = options.grain_invert.unwrap_or(style.grain_invert);
    style.grain_uv = options.grain_uv.unwrap_or(style.grain_uv);
    for (percent, parameter, factor) in [
        (options.scatter_percent, Parameter::BrushScatter, &mut style.variation.scatter),
        (options.size_jitter_percent, Parameter::BrushSizeJitter, &mut style.variation.size),
        (options.angle_jitter_percent, Parameter::BrushAngleJitter, &mut style.variation.angle),
    ] {
        if let Some(percent) = percent {
            *factor = brush_value(Some(percent), parameter, 0.)? / 100.;
        }
    }
    Ok(())
}

fn brush_texture(name: &str, disabled: &str, kind: &str, names: Vec<&str>) -> Result<Option<usize>, String> {
    let key = name.to_lowercase();
    if key == disabled {
        return Ok(None);
    }
    names.iter().position(|name| name.to_lowercase() == key).map(Some)
        .ok_or_else(|| format!("Unknown brush {kind} '{name}'; use list_paint_presets"))
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
    spacing: f32,
    (stabilizer, pressure_size, pressure_flow): (f32, bool, bool),
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
        stabilizer,
        pressure_size,
        pressure_flow,
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
