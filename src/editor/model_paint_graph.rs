//! Bevy paint-layer bridge for the procedural material graph runtime.
use super::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use material_painter_graph::{Graph, Library, ParamKind, Runtime, Value, channel_outputs};
pub(super) use node_library::encode;
pub(super) use node_library::library;
use std::path::PathBuf;

#[path = "model_paint_graph_file.rs"]
pub(super) mod file;
#[path = "model_paint_graph_maps.rs"]
pub(super) mod mesh_maps;
#[path = "model_paint_graph_previews.rs"]
pub(super) mod previews;
#[path = "model_paint_graph_surface.rs"]
pub(super) mod surface;
pub(super) use material_painter_graph::presets::PRESETS;

pub(super) const MPP_GRAPH_PENDING: &str = "bevy_mpp_graph_pending";
pub(super) const MPP_GRAPH_KEEP_CACHE: &str = "bevy_mpp_graph_keep_cache";

pub(super) fn encode_without_rebuild(graph: &Graph, previous: &serde_json::Value) -> serde_json::Value {
    let mut payload = encode(graph);
    for key in [MPP_GRAPH_PENDING, MPP_GRAPH_KEEP_CACHE] {
        if previous[key].as_bool() == Some(true) {
            payload[key] = serde_json::Value::Bool(true);
        }
    }
    payload
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum ParameterTarget {
    Node(u64, usize),
    Published(usize),
}

impl ParameterTarget {
    fn set(self, graph: &mut Graph, kind: ParamKind, value: Value) -> Result<Option<u64>, String> {
        let lib = library();
        let lib = lib.as_ref();
        match self {
            Self::Node(node, parameter) => {
                let parameter = node_parameter(lib, graph, node, parameter)?;
                if parameter.kind != kind {
                    return Err("Неподходящий тип параметра узла".into());
                }
                set_node_parameter(graph, node, &parameter.name, value);
                Ok(Some(node))
            }
            Self::Published(index) => {
                if !graph
                    .param_value(index, lib)
                    .is_some_and(|(param, _)| param.kind == kind)
                {
                    return Err("Параметр графа не найден".into());
                }
                graph.set_param(index, value, lib);
                Ok(None)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum Edit {
    Add(usize, IVec2),
    Move(u64, IVec2),
    Remove(u64),
    Connect(u64, u64, usize),
    Disconnect(u64, usize),
    Publish(u64, usize),
    Bind(u64, usize, usize),
    Unbind(u64, usize),
    RemoveParameter(usize),
    MoveParameter(usize, i8),
    AdjustParameter(usize, usize, i8),
    ResetParameter(usize),
    LabelParameter(usize, bool),
    ClearBitmap(ParameterTarget),
    SetMeshMap(ParameterTarget, usize),
}

impl Edit {
    pub(super) fn rebuilds(self) -> bool {
        match self {
            Self::Add(index, _) => library()
                .defs
                .get(index)
                .is_some_and(|def| def.id == material_painter_graph::OUTPUT_DEF),
            Self::Move(..)
            | Self::Publish(..)
            | Self::Unbind(..)
            | Self::RemoveParameter(_)
            | Self::MoveParameter(..)
            | Self::LabelParameter(..) => false,
            _ => true,
        }
    }

    pub(super) fn apply(
        self,
        graph: &mut Graph,
        label: Option<&str>,
    ) -> Result<Option<u64>, String> {
        self.apply_for(graph, label, false)
    }

    pub(super) fn apply_for(
        self,
        graph: &mut Graph,
        label: Option<&str>,
        shading: bool,
    ) -> Result<Option<u64>, String> {
        let lib = library();
        let lib = lib.as_ref();
        let port = |graph: &Graph, node: u64, index: usize| {
            graph
                .node(node)
                .and_then(|node| lib.find(&node.def))
                .and_then(|def| def.inputs.get(index))
                .map(|input| input.name.as_str())
                .ok_or("Вход узла не найден")
        };
        match self {
            Self::Add(index, pos) => {
                let def = lib
                    .defs
                    .get(index)
                    .filter(|def| def.shading == shading && node_library::available(&def.id))
                    .ok_or("Узел не найден")?;
                if graph.nodes.len() >= 256 || graph.next_id == u64::MAX {
                    return Err("Достигнут предел числа узлов графа".into());
                }
                Ok(Some(graph.add(&def.id, pos.as_vec2().to_array())))
            }
            Self::Move(id, pos) => {
                graph.node_mut(id).ok_or("Узел не найден")?.pos = pos.as_vec2().to_array();
                Ok(Some(id))
            }
            Self::Remove(id) => {
                if graph.node(id).is_none() {
                    return Err("Узел не найден".into());
                }
                graph.remove(id);
                Ok(None)
            }
            Self::Connect(from, to, input) => {
                let input = port(graph, to, input)?;
                if !graph.connect(from, to, input) {
                    return Err("Связь замыкает цикл или один из узлов отсутствует".into());
                }
                Ok(Some(to))
            }
            Self::Disconnect(id, index) => {
                let input = port(graph, id, index)?;
                graph.node_mut(id).unwrap().links.remove(input);
                Ok(Some(id))
            }
            Self::Publish(id, index) => {
                let name = node_parameter(lib, graph, id, index)?.name.as_str();
                graph.expose(id, name, lib);
                Ok(Some(id))
            }
            Self::Bind(id, index, published) => {
                let parameter = node_parameter(lib, graph, id, index)?;
                let current = graph.param_of(id, &parameter.name);
                if current == Some(published) {
                    return Ok(Some(id));
                }
                if !graph
                    .param_value(published, lib)
                    .is_some_and(|(source, _)| compatible(&source.kind, &parameter.kind))
                {
                    return Err("Типы параметров различаются".into());
                }
                // Unbinding can remove the previous entry and shift the target index.
                let shifted =
                    current.is_some_and(|i| i < published && graph.params[i].binds.len() == 1);
                graph.bind(published - usize::from(shifted), id, &parameter.name, lib);
                Ok(Some(id))
            }
            Self::Unbind(id, index) => {
                let name = &node_parameter(lib, graph, id, index)?.name;
                graph.unbind(id, name);
                Ok(Some(id))
            }
            Self::RemoveParameter(index) if index < graph.params.len() => {
                graph.params.remove(index);
                Ok(None)
            }
            Self::MoveParameter(index, delta) => {
                let other = index.saturating_add_signed(isize::from(delta.signum()));
                if index.max(other) >= graph.params.len() {
                    return Err("Параметр графа не найден".into());
                }
                graph.params.swap(index, other);
                Ok(None)
            }
            Self::ResetParameter(index) => {
                let value = graph
                    .params
                    .get(index)
                    .and_then(|p| p.default.clone())
                    .ok_or("У параметра нет исходного значения")?;
                graph.set_param(index, value, lib);
                Ok(None)
            }
            Self::AdjustParameter(index, component, direction) => {
                let (param, value) = graph
                    .param_value(index, lib)
                    .ok_or("Параметр графа не найден")?;
                if let Some(changed) = step(&param.kind, &value, component, direction) {
                    graph.set_param(index, changed, lib);
                }
                Ok(None)
            }
            Self::LabelParameter(index, group) => {
                let label: String = label
                    .ok_or("Поле параметра не найдено")?
                    .trim()
                    .chars()
                    .take(80)
                    .collect();
                let parameter = graph
                    .params
                    .get_mut(index)
                    .ok_or("Параметр графа не найден")?;
                if group {
                    parameter.group = label;
                } else if !label.is_empty() {
                    parameter.name = label;
                } else {
                    return Err("Введите имя параметра".into());
                }
                Ok(None)
            }
            Self::ClearBitmap(target) => target.set(graph, ParamKind::Image, Value::Image(None)),
            Self::SetMeshMap(target, value) => {
                target.set(graph, ParamKind::MeshMap, Value::MeshMap(value))
            }
            Self::RemoveParameter(_) => Err("Параметр графа не найден".into()),
        }
    }
}

fn node_parameter<'a>(
    library: &'a Library,
    graph: &Graph,
    id: u64,
    index: usize,
) -> Result<&'a material_painter_graph::def::Param, String> {
    graph
        .node(id)
        .and_then(|node| library.find(&node.def))
        .and_then(|definition| definition.params.get(index))
        .ok_or_else(|| "Параметр узла не найден".into())
}

pub(super) fn compatible(a: &ParamKind, b: &ParamKind) -> bool {
    match (a, b) {
        (ParamKind::Float { .. }, ParamKind::Float { .. })
        | (ParamKind::Int { .. }, ParamKind::Int { .. }) => true,
        _ => a == b,
    }
}

pub(super) fn prepare(
    preset: usize,
    size: u32,
    inputs: Inputs<'_>,
    device: &RenderDevice,
    queue: &RenderQueue,
) -> Result<Layer, String> {
    let preset = PRESETS.get(preset).ok_or("Граф материала не найден")?;
    let mut graph = (preset.build)();
    graph.res = size.min(1024);
    graph.match_set = false;
    prepare_graph(graph, &Default::default(), size, inputs, device, queue)
}

pub(super) fn read(data: &serde_json::Value) -> Result<Graph, String> {
    node_library::decode(data)
}

fn graph_payloads(layer: &Layer) -> [Option<&serde_json::Value>; 2] {
    let fill = layer
        .texture_fill
        .as_ref()
        .and_then(|fill| fill.graph.as_deref());
    let mask = layer
        .mask
        .as_ref()
        .and_then(|mask| mask.graph.as_ref())
        .map(|graph| graph.payload.as_ref());
    [fill, mask]
}

fn pending_import(payload: &serde_json::Value) -> bool {
    payload[MPP_GRAPH_PENDING].as_bool() == Some(true)
}

pub(super) fn needs_initial_render(stack: &Stack) -> bool {
    stack.layers.iter().flat_map(graph_payloads).flatten().any(pending_import)
}

pub(super) fn affected_layers(
    stack: &Stack,
    maps: bool,
    geometry: bool,
) -> Vec<(usize, bool, Graph)> {
    stack
        .layers
        .iter()
        .enumerate()
        .flat_map(|(index, layer)| {
            let [fill, mask] = graph_payloads(layer);
            [(false, fill), (true, mask)]
                .into_iter()
                .filter_map(move |(mask, payload)| {
                    let payload = payload?;
                    if payload[MPP_GRAPH_KEEP_CACHE].as_bool() == Some(true) {
                        return None;
                    }
                    let graph = read(payload).ok()?;
                    ((geometry && pending_import(payload))
                        || (maps && !mesh_maps::used(&graph).is_empty())
                        || ((maps || geometry) && surface::used(&graph)))
                    .then_some((index, mask, graph))
                })
        })
        .collect()
}

pub(super) fn refresh_layer(
    layer: &mut Layer,
    is_mask: bool,
    graph: Graph,
    size: u32,
    inputs: Inputs<'_>,
    device: &RenderDevice,
    queue: &RenderQueue,
) -> Result<(), String> {
    if is_mask {
        let images = layer
            .mask
            .as_ref()
            .unwrap()
            .graph
            .as_ref()
            .unwrap()
            .images
            .clone();
        apply_mask(layer, graph, &images, size, inputs, device, queue)?;
    } else {
        let previous = layer.texture_fill.as_ref().unwrap();
        let mut updated =
            prepare_graph(graph, &previous.graph_images, size, inputs, device, queue)?
                .texture_fill
                .unwrap();
        apply_settings(Arc::make_mut(&mut updated), previous.settings);
        layer.texture_fill = Some(updated);
    }
    Ok(())
}

pub(super) fn apply_settings(fill: &mut texture_fill::Fill, settings: texture_fill::Settings) {
    fill.settings = settings;
    for (image, enabled) in fill.images.iter_mut().zip(settings.graph_channels) {
        if !enabled {
            *image = None;
        }
    }
}

pub(super) fn prepare_imported(
    layers: &mut [Layer],
    size: u32,
    inputs: Inputs<'_>,
    renderer: Option<(&RenderDevice, &RenderQueue)>,
) -> Result<(), String> {
    for layer in layers {
        let [fill, mask] = graph_payloads(layer).map(|payload| payload.map(read).transpose());
        for (is_mask, graph) in [(false, fill?), (true, mask?)] {
            if let Some(graph) = graph {
                let (device, queue) = renderer.ok_or("GPU для расчёта графа недоступен")?;
                refresh_layer(layer, is_mask, graph, size, inputs, device, queue)?;
            }
        }
    }
    Ok(())
}

pub(super) fn adjustable(value: &material_painter_graph::Value) -> bool {
    use material_painter_graph::Value;
    matches!(
        value,
        Value::Float(_) | Value::Int(_) | Value::Bool(_) | Value::Enum(_) | Value::Color(_)
    )
}

pub(super) fn adjust(
    graph: &mut Graph,
    node: u64,
    parameter: usize,
    component: usize,
    direction: i8,
) -> bool {
    let lib = library();
    let Some((param, value)) = graph.node(node).and_then(|node| {
        let param = lib.find(&node.def)?.params.get(parameter)?;
        Some((param, lib.value(node, &param.name)?))
    }) else {
        return false;
    };
    let Some(changed) = step(&param.kind, &value, component, direction) else {
        return false;
    };
    set_node_parameter(graph, node, &param.name, changed);
    true
}

fn set_node_parameter(graph: &mut Graph, node: u64, name: &str, value: Value) {
    if let Some(exposed) = graph.param_of(node, name) {
        graph.set_param(exposed, value, &library());
    } else if let Some(node) = graph.node_mut(node) {
        node.params.insert(name.to_owned(), value);
    }
}

pub(super) fn set_bitmap(
    graph: &mut Graph,
    images: &mut Arc<texture_fill::GraphImages>,
    target: ParameterTarget,
    path: &std::path::Path,
) -> Result<(), String> {
    let key = format!("graph-images/{}.png", bevy::asset::uuid::Uuid::new_v4());
    target.set(
        graph,
        ParamKind::Image,
        Value::Image(Some(PathBuf::from(&key))),
    )?;
    let image = file::read_bitmap(path)?;
    Arc::make_mut(images).insert(key, Arc::new(image));
    Ok(())
}

fn step(kind: &ParamKind, value: &Value, component: usize, direction: i8) -> Option<Value> {
    let direction = direction.signum();
    let changed = match (kind, value) {
        (ParamKind::Float { min, max }, Value::Float(v)) => {
            Value::Float((v + (max - min) * f32::from(direction) / 20.).clamp(*min, *max))
        }
        (ParamKind::Int { min, max }, Value::Int(v)) => {
            Value::Int(v.saturating_add(i32::from(direction)).clamp(*min, *max))
        }
        (ParamKind::Bool, Value::Bool(_)) => Value::Bool(direction > 0),
        (ParamKind::Enum(options), Value::Enum(v)) => Value::Enum(
            v.saturating_add_signed(isize::from(direction))
                .min(options.len().saturating_sub(1)),
        ),
        (ParamKind::Color, Value::Color(rgb)) if component < rgb.len() => {
            let mut rgb = *rgb;
            rgb[component] = (rgb[component] + 0.05 * f32::from(direction)).clamp(0., 1.);
            Value::Color(rgb)
        }
        _ => return None,
    };
    (changed != *value).then_some(changed)
}

#[derive(Clone, Copy, Default)]
pub(super) struct Inputs<'a> {
    pub maps: Option<&'a bake::Maps>,
    pub surface: Option<&'a surface::Source>,
}

pub(super) fn prepare_graph(
    graph: Graph,
    images: &texture_fill::GraphImages,
    set_size: u32,
    inputs: Inputs<'_>,
    device: &RenderDevice,
    queue: &RenderQueue,
) -> Result<Layer, String> {
    let (graph, images, outputs) = render(graph, images, set_size, inputs, device, queue)?;
    graph_layer(graph, images, channel_outputs(&outputs))
}

pub(super) fn mask_starter() -> Graph {
    let mut graph = Graph::new("Граф маски");
    graph.match_set = true;
    let map = graph.add("mesh_map", [0., 0.]);
    graph
        .node_mut(map)
        .unwrap()
        .params
        .insert("map".into(), Value::MeshMap(1));
    let scan = graph.add("histogram_scan", [220., 0.]);
    graph
        .node_mut(scan)
        .unwrap()
        .params
        .insert("position".into(), Value::Float(0.55));
    let output = graph.add("output", [440., 0.]);
    graph.node_mut(output).unwrap().params.insert(
        "usage".into(),
        Value::Enum(material_painter_graph::MASK_USAGE),
    );
    graph.connect(map, scan, "input");
    graph.connect(scan, output, "input");
    graph
}

pub(super) fn prepare_mask(
    graph: Graph,
    images: &texture_fill::GraphImages,
    set_size: u32,
    inputs: Inputs<'_>,
    device: &RenderDevice,
    queue: &RenderQueue,
) -> Result<Arc<crate::editor::model_paint_data::mask::GraphMask>, String> {
    let (graph, images, mut outputs) = render(graph, images, set_size, inputs, device, queue)?;
    let output = outputs[material_painter_graph::MASK_USAGE]
        .take()
        .or_else(|| outputs[0].take());
    let output = output
        .map(|image| texture_fill::Bitmap::from_rgba(format!("{}-Mask.png", graph.name), image))
        .transpose()?
        .map(Arc::new);
    Ok(Arc::new(
        crate::editor::model_paint_data::mask::GraphMask::new(
            Arc::new(encode(&graph)),
            Arc::new(images),
            output,
        ),
    ))
}

pub(super) fn apply_mask(
    layer: &mut Layer,
    graph: Graph,
    images: &texture_fill::GraphImages,
    size: u32,
    inputs: Inputs<'_>,
    device: &RenderDevice,
    queue: &RenderQueue,
) -> Result<(), String> {
    use crate::editor::model_paint_data::{mask::Mask, solid};
    let graph = prepare_mask(graph, images, size, inputs, device, queue)?;
    let mask = layer
        .mask
        .get_or_insert_with(|| Arc::new(Mask::new(solid(size, [255; 4]), Vec::new())));
    Arc::make_mut(mask).set_graph(graph);
    Ok(())
}

type Outputs = [Option<image::RgbaImage>; material_painter_graph::USAGES.len()];

fn render(
    mut graph: Graph,
    images: &texture_fill::GraphImages,
    set_size: u32,
    inputs: Inputs<'_>,
    device: &RenderDevice,
    queue: &RenderQueue,
) -> Result<(Graph, texture_fill::GraphImages, Outputs), String> {
    let library = library();
    let library = library.as_ref();
    if graph.nodes.is_empty() {
        return Ok((graph, Default::default(), Default::default()));
    }
    graph.validate_texture_graph(library)?;
    let authored_resolution = graph.res;
    let resolution = if graph.match_set {
        set_size
    } else {
        authored_resolution
    };
    graph.res = resolution.clamp(16, 1024);
    let mut runtime = Runtime::new(device.wgpu_device(), queue, assets());
    let images = file::bind_images(&mut graph, images, &mut runtime, device, queue)?;
    let mesh = mesh_maps::Textures::new(
        &mesh_maps::used(&graph),
        inputs.maps,
        graph.res,
        device,
        queue,
    );
    let surface = if surface::used(&graph) {
        let source = inputs
            .surface
            .ok_or("Для поверхностного узла нужен набор текстур модели")?;
        Some(surface::Textures::new(
            source,
            inputs.maps,
            graph.res,
            device,
            queue,
        ))
    } else {
        None
    };
    let device = device.wgpu_device();
    runtime.evaluate(
        device,
        queue,
        &graph,
        library,
        &|id| mesh.get(id),
        surface.as_ref().map(surface::Textures::inputs).as_ref(),
        0,
        true,
    );
    if let Some((node, error)) = runtime.errors.iter().min_by_key(|(id, _)| *id) {
        let title = graph.node(*node).map_or("Узел", |node| node.def.as_str());
        return Err(format!("{title} · {error}"));
    }
    let outputs = runtime.read_outputs(device, queue, &graph, library)?;
    graph.res = authored_resolution;
    Ok((graph, images, outputs))
}

pub(super) fn assets() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/paint")
}

fn graph_layer(
    graph: Graph,
    images: texture_fill::GraphImages,
    maps: [Option<image::RgbaImage>; 8],
) -> Result<Layer, String> {
    let mut fill = texture_fill::Fill::default();
    fill.settings.image_alpha_coverage = false;
    fill.settings.opacity_source = texture_fill::OpacitySource::Red;
    fill.graph = Some(Arc::new(encode(&graph)));
    fill.graph_images = Arc::new(images);
    for (i, (image, value)) in fill.images.iter_mut().zip(maps).enumerate() {
        if let Some(value) = value {
            *image = Some(Arc::new(texture_fill::Bitmap::from_rgba(
                format!("{}-{}.png", graph.name, CHANNELS[i]),
                value,
            )?));
        }
    }
    let mut layer = Layer::new(graph.name, LayerKind::Fill);
    layer.texture_fill = Some(Arc::new(fill));
    Ok(layer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebind_published_parameter_after_earlier_entry_is_removed() {
        let mut graph = Graph::new("Параметры");
        let first = graph.add("tiles", [0.; 2]);
        let second = graph.add("tiles", [0.; 2]);
        let lib = library();
        let lib = lib.as_ref();
        let columns = lib
            .find("tiles")
            .unwrap()
            .params
            .iter()
            .position(|param| param.name == "columns")
            .unwrap();
        Edit::Publish(first, columns)
            .apply(&mut graph, None)
            .unwrap();
        Edit::Publish(second, columns)
            .apply(&mut graph, None)
            .unwrap();
        Edit::AdjustParameter(1, 0, 1)
            .apply(&mut graph, None)
            .unwrap();

        Edit::Bind(first, columns, 1)
            .apply(&mut graph, None)
            .unwrap();
        assert_eq!(graph.params.len(), 1);
        assert_eq!(graph.params[0].binds.len(), 2);
        let value = |graph: &Graph, id| lib.value(graph.node(id).unwrap(), "columns");
        assert_eq!(value(&graph, first), value(&graph, second));
        Edit::LabelParameter(0, false)
            .apply(&mut graph, Some("Частота"))
            .unwrap();
        Edit::LabelParameter(0, true)
            .apply(&mut graph, Some("Узор"))
            .unwrap();
        Edit::ResetParameter(0).apply(&mut graph, None).unwrap();
        assert_eq!(value(&graph, first), graph.params[0].default);
        assert_eq!(value(&graph, second), graph.params[0].default);
        assert_eq!(read(&graph.to_json(lib)).unwrap(), graph);
        Edit::RemoveParameter(0).apply(&mut graph, None).unwrap();
        assert!(graph.params.is_empty());
        assert_eq!(value(&graph, first), value(&graph, second));
    }
}
