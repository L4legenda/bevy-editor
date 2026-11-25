//! Bevy paint-layer bridge for the procedural material graph runtime.
use super::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use material_painter_graph::{CHANNEL_USAGE, Graph, Library, ParamKind, Runtime, Value};
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
pub(super) const RESOLUTIONS: [u32; 5] = [256, 512, 1024, 2048, 4096];

pub(super) fn render_limit(device: &RenderDevice) -> u32 {
    let limits = device.limits();
    RESOLUTIONS.into_iter().rev().find(|size| {
        *size <= limits.max_texture_dimension_2d
            && u64::from(*size).pow(2) * 8 <= limits.max_buffer_size
    }).unwrap_or_else(|| limits.max_texture_dimension_2d.min(128))
}

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
    AddConnected(usize, IVec2, WireSource),
    UpdateSource(u64, usize),
    Move(u64, IVec2),
    Remove(u64),
    Connect(u64, u64, usize),
    Reconnect(WireSource, GraphInput),
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
    Resolution(Option<u32>),
    Batch(u64, BatchKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) struct GraphInput {
    pub(super) node: u64,
    pub(super) index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) struct WireSource {
    pub(super) node: u64,
    pub(super) input: Option<GraphInput>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum BatchKind {
    Move,
    Remove,
    Paste(u64),
    ReplaceSource,
}

pub(super) enum Batch {
    Move(Vec<u64>, Vec2),
    Remove(Vec<u64>),
    ReplaceSource { node: u64, definition: String },
    Paste {
        nodes: Vec<material_painter_graph::Node>,
        offset: Vec2,
        keep_links: bool,
        images: Arc<texture_fill::GraphImages>,
    },
}

impl Batch {
    pub(super) fn apply(
        self,
        graph: &mut Graph,
        images: &mut Arc<texture_fill::GraphImages>,
        shading: bool,
        folder: bool,
    ) -> Result<Option<u64>, String> {
        match self {
            Self::Move(ids, offset) => {
                for id in &ids {
                    if let Some(node) = graph.node_mut(*id) {
                        node.pos = (Vec2::from_array(node.pos) + offset).to_array();
                    }
                }
                Ok(ids.last().copied())
            }
            Self::Remove(ids) => {
                for id in ids {
                    if !folder || folder_graph::managed(graph, id).is_none() {
                        graph.remove(id);
                        if folder {
                            graph.tags.retain(|tag| tag.node != id);
                        }
                    }
                }
                Ok(None)
            }
            Self::ReplaceSource { node, definition } => {
                if !can_replace_source(graph, node, folder) {
                    return Err("Этот узел нельзя заменить копией".into());
                }
                let target = graph.node_mut(node).ok_or("Узел не найден")?;
                let library = library();
                let original = library.find(&target.def).ok_or("Исходный код недоступен")?;
                let copy = library.find(&definition).filter(|copy| {
                    copy.id.starts_with("user/") && node_library::available(&copy.id)
                        && copy.shading == shading && copy.source == node_library::builtin_copy_source(original)
                }).ok_or("Исходник копии уже изменился")?;
                target.def = copy.id.clone();
                Ok(Some(node))
            }
            Self::Paste { nodes, offset, keep_links, images: copied_images } => {
                let library = library();
                if nodes.iter().any(|node| !library.find(&node.def).is_some_and(|def| {
                    def.shading == shading && (!folder || def.id != material_painter_graph::OUTPUT_DEF)
                })) {
                    return Err("Узлы не подходят для этого графа".into());
                }
                let first = graph.next_id;
                if graph.nodes.len() + nodes.len() > 256 || first.checked_add(nodes.len() as u64).is_none() {
                    return Err("Достигнут предел числа узлов графа".into());
                }
                let ids: HashMap<_, _> = nodes.iter().map(|node| {
                    let position = (Vec2::from_array(node.pos) + offset).to_array();
                    (node.id, graph.add(&node.def, position))
                }).collect();
                let last = nodes.last().map(|node| ids[&node.id]);
                let mut relocated = HashMap::<PathBuf, PathBuf>::new();
                for mut node in nodes {
                    let id = ids[&node.id];
                    for value in node.params.values_mut() {
                        if let Value::Image(Some(path)) = value
                            && let Some(key) = path.to_str()
                            && let Some(bitmap) = copied_images.get(key)
                        {
                            if let Some(destination) = relocated.get(path) {
                                *path = destination.clone();
                            } else if let Some(existing) = images.get(key) {
                                if !Arc::ptr_eq(existing, bitmap) {
                                    let filename = path.file_name().unwrap_or_default().to_string_lossy();
                                    let destination = (1..).map(|number| format!("graph-images/copy-{number}-{filename}"))
                                        .find(|candidate| !images.contains_key(candidate)).unwrap();
                                    Arc::make_mut(images).insert(destination.clone(), bitmap.clone());
                                    relocated.insert(path.clone(), destination.clone().into());
                                    *path = destination.into();
                                }
                            } else {
                                Arc::make_mut(images).insert(key.to_owned(), bitmap.clone());
                            }
                        }
                    }
                    graph.node_mut(id).unwrap().params = node.params;
                    for (input, from) in node.links {
                        if let Some(source) = ids.get(&from).copied().or_else(|| {
                            (keep_links && from < first && (!folder || folder_graph::managed(graph, from).is_none()))
                                .then_some(from)
                        }) {
                            graph.connect(source, id, &input);
                        }
                    }
                }
                Ok(last)
            }
        }
    }
}

pub(super) fn replaceable_builtin(id: &str) -> bool {
    !id.starts_with("user/") && !node_library::is_embedded(id)
        && !matches!(id, material_painter_graph::OUTPUT_DEF | material_painter_graph::shading::OUTPUT)
}

fn can_replace_source(graph: &Graph, id: u64, folder: bool) -> bool {
    graph.node(id).is_some_and(|node| replaceable_builtin(&node.def))
        && (!folder || folder_graph::managed(graph, id).is_none())
}

impl Edit {
    pub(super) fn rebuilds(self) -> bool {
        match self {
            Self::AddConnected(_, _, WireSource { input: Some(_), .. }) => true,
            Self::Add(index, _) | Self::AddConnected(index, _, _) => library()
                .defs
                .get(index)
                .is_some_and(|def| def.id == material_painter_graph::OUTPUT_DEF),
            Self::Move(..)
            | Self::Batch(_, BatchKind::Move | BatchKind::ReplaceSource)
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
        let original_input = |graph: &Graph, source: WireSource| -> Result<Option<(u64, &str)>, String> {
            graph.node(source.node).ok_or("Исходный узел не найден")?;
            source.input.map(|input| {
                let name = port(graph, input.node, input.index)?;
                if graph.node(input.node).and_then(|node| node.links.get(name)) != Some(&source.node) {
                    return Err("Исходная связь уже изменилась".into());
                }
                Ok((input.node, name))
            }).transpose()
        };
        match self {
            Self::Batch(..) => Err("Пакет правок графа недоступен".into()),
            Self::UpdateSource(id, definition) => {
                let next = lib.defs.get(definition)
                    .filter(|def| def.id.starts_with("user/") && def.shading == shading
                        && node_library::available(&def.id))
                    .ok_or("Сохранённый исходник узла недоступен")?;
                update_source(graph, id, lib, next)?;
                Ok(Some(id))
            }
            Self::Resolution(size) => {
                if shading || size.is_some_and(|size| !RESOLUTIONS.contains(&size)) {
                    return Err("Неподдерживаемое разрешение графа".into());
                }
                graph.match_set = size.is_none();
                if let Some(size) = size {
                    graph.res = size;
                }
                Ok(None)
            }
            Self::Add(index, pos) | Self::AddConnected(index, pos, _) => {
                let def = lib
                    .defs
                    .get(index)
                    .filter(|def| def.shading == shading && node_library::available(&def.id))
                    .ok_or("Узел не найден")?;
                if graph.nodes.len() >= 256 || graph.next_id == u64::MAX {
                    return Err("Достигнут предел числа узлов графа".into());
                }
                let connection = if let Self::AddConnected(_, _, source) = self {
                    let old = original_input(graph, source)?;
                    let input = def.inputs.first().ok_or("У узла нет входов")?.name.as_str();
                    Some((source.node, input, old))
                } else {
                    None
                };
                let id = graph.add(&def.id, pos.as_vec2().to_array());
                if let Some((source, input, old)) = connection {
                    graph.connect(source, id, input);
                    if let Some((node, name)) = old {
                        graph.node_mut(node).unwrap().links.remove(name);
                    }
                }
                Ok(Some(id))
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
            Self::Reconnect(source, target) => {
                let old = original_input(graph, source)?;
                if source.input == Some(target) {
                    return Ok(Some(target.node));
                }
                let input = port(graph, target.node, target.index)?;
                if !graph.connect(source.node, target.node, input) {
                    return Err("Связь замыкает цикл или один из узлов отсутствует".into());
                }
                if let Some((node, name)) = old {
                    graph.node_mut(node).unwrap().links.remove(name);
                }
                Ok(Some(target.node))
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

fn update_source(
    graph: &mut Graph, id: u64, library: &Library, next: &material_painter_graph::def::NodeDef,
) -> Result<(), String> {
    let node = graph.node(id).ok_or("Узел не найден")?;
    let previous = library.find(&node.def).ok_or("Прежний исходник узла недоступен")?;
    let connects_to_other_nodes = graph.nodes.iter().any(|node| node.links.values().any(|value| *value == id));
    if (connects_to_other_nodes && previous.output != next.output)
        || !node.links.keys().all(|name| {
            previous.inputs.iter().find(|input| &input.name == name).is_some_and(|old| {
                next.inputs.iter().any(|input| input.name == old.name && input.kind == old.kind)
            })
        })
    {
        return Err("Тип выхода или подключённого входа изменился · проверьте связи узла".into());
    }
    let parameter_compatible = |name: &str| {
        previous.params.iter().find(|param| param.name == name).is_some_and(|old| {
            next.params.iter().any(|param| param.name == name && compatible(&old.kind, &param.kind))
        })
    };
    if !node.params.keys().all(|name| parameter_compatible(name))
        || !graph.params.iter().flat_map(|param| &param.binds)
            .all(|(node, name)| *node != id || parameter_compatible(name))
    {
        return Err("Тип или имя используемого параметра изменились · обновите параметры графа".into());
    }
    graph.node_mut(id).expect("узел проверен").def = next.id.clone();
    Ok(())
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
    graph.res = size;
    graph.match_set = true;
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
    set_size: Option<u32>,
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
                    let cached = if mask {
                        layer.mask.as_ref()?.graph.as_ref()?.output.as_deref()
                    } else {
                        layer.texture_fill.as_ref()?.images.iter().flatten().next().map(Arc::as_ref)
                    };
                    let needs_resize = graph.match_set && set_size.is_some_and(|size| {
                        cached.is_some_and(|image| image.pixels().width() != size || image.pixels().height() != size)
                    });
                    (needs_resize || (geometry && pending_import(payload))
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
    let (graph, images, mut outputs) = render(graph, images, set_size, inputs, device, queue)?;
    let channels = CHANNEL_USAGE.map(|usage| outputs[usage].take());
    graph_layer(graph, images, channels)
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
    let authored_resolution = graph.res;
    let (runtime, images) = evaluate(&mut graph, images, set_size, inputs, device, queue, library)?;
    let outputs = runtime.read_outputs(device.wgpu_device(), queue, &graph, library)?;
    graph.res = authored_resolution;
    Ok((graph, images, outputs))
}

fn evaluate(
    graph: &mut Graph,
    images: &texture_fill::GraphImages,
    set_size: u32,
    inputs: Inputs<'_>,
    device: &RenderDevice,
    queue: &RenderQueue,
    library: &Library,
) -> Result<(Runtime, texture_fill::GraphImages), String> {
    graph.validate_texture_graph(library)?;
    let resolution = if graph.match_set {
        set_size
    } else {
        graph.res
    };
    graph.res = resolution.clamp(16, render_limit(device));
    let mut runtime = Runtime::new(device.wgpu_device(), queue, assets());
    let images = file::bind_images(graph, images, &mut runtime, device, queue)?;
    let mesh = mesh_maps::Textures::new(
        &mesh_maps::used(graph),
        inputs.maps,
        graph.res,
        device,
        queue,
    );
    let surface = if surface::used(graph) {
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
        graph,
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
    Ok((runtime, images))
}

pub(super) fn export_outputs(
    directory: &std::path::Path,
    payload: &serde_json::Value,
    images: &texture_fill::GraphImages,
    set_size: u32,
    inputs: Inputs<'_>,
    device: &RenderDevice,
    queue: &RenderQueue,
) -> Result<export::Report, String> {
    let mut graph = read(payload)?;
    let library = library();
    if graph.outputs(&library).iter().all(Option::is_none) {
        return Err("В графе нет выходов".into());
    }
    let (runtime, _) = evaluate(&mut graph, images, set_size, inputs, device, queue, &library)?;
    let outputs = runtime.read_outputs_with(
        device.wgpu_device(), queue, &graph, &library, decode_output,
    )?;
    export::write_graph_outputs(directory, &graph.name, outputs).map_err(|error| error.to_string())
}

fn decode_output(
    usage: usize,
    width: u32,
    height: u32,
    stride: usize,
    data: &[u8],
) -> Result<image::DynamicImage, String> {
    let pixels = data.chunks_exact(stride)
        .flat_map(|row| row[..width as usize * 8].chunks_exact(8));
    let sample = |pixel: &[u8], channel: usize| {
        let offset = channel * 2;
        half::f16::from_le_bytes([pixel[offset], pixel[offset + 1]]).to_f32().clamp(0., 1.)
    };
    let byte = |pixel: &[u8], channel| (sample(pixel, channel) * 255.).round() as u8;
    let output = match usage {
        0 | 6 | 7 => {
            let rgb = pixels.flat_map(|pixel| [byte(pixel, 0), byte(pixel, 1), byte(pixel, 2)]).collect();
            image::RgbImage::from_raw(width, height, rgb).map(image::DynamicImage::ImageRgb8)
        }
        3 => {
            let height_values = pixels.map(|pixel| (sample(pixel, 0) * 65535.).round() as u16).collect();
            image::ImageBuffer::from_raw(width, height, height_values).map(image::DynamicImage::ImageLuma16)
        }
        _ => {
            let values = pixels.map(|pixel| byte(pixel, 0)).collect();
            image::GrayImage::from_raw(width, height, values).map(image::DynamicImage::ImageLuma8)
        }
    };
    output.ok_or_else(|| "Неверный размер выхода графа".into())
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
    fn reconnect_validates_both_ends_before_moving_an_existing_wire() {
        let mut graph = Graph::new("Reroute");
        let color = graph.add("color", [0., 0.]);
        let source = graph.add("invert", [200., 0.]);
        let old = graph.add("output", [400., 0.]);
        let target = graph.add("invert", [400., 160.]);
        assert!(graph.connect(color, source, "input"));
        assert!(graph.connect(source, old, "input"));
        assert!(graph.connect(color, target, "input"));
        let old_port = GraphInput { node: old, index: 0 };
        let dragged = WireSource { node: source, input: Some(old_port) };
        let original = graph.clone();
        let cycle = Edit::Reconnect(dragged, GraphInput { node: source, index: 0 });
        assert!(cycle.apply(&mut graph, None).is_err());
        Edit::Reconnect(dragged, old_port).apply(&mut graph, None).unwrap();
        assert_eq!(graph, original);
        let reroute = Edit::Reconnect(dragged, GraphInput { node: target, index: 0 });
        assert_eq!(reroute.apply(&mut graph, None).unwrap(), Some(target));
        assert!(!graph.node(old).unwrap().links.contains_key("input"));
        assert_eq!(graph.node(target).unwrap().links["input"], source);
        let changed = graph.clone();
        assert!(reroute.apply(&mut graph, None).is_err());
        assert_eq!(graph, changed);
    }

    #[test]
    fn batch_paste_remaps_internal_links_and_only_keeps_local_inputs() {
        let mut graph = Graph::new("Batch");
        let input = graph.add("color", [0., 0.]);
        let first = graph.add("invert", [220., 0.]);
        let last = graph.add("invert", [440., 0.]);
        assert!(graph.connect(input, first, "input"));
        assert!(graph.connect(first, last, "input"));
        let copied = vec![graph.node(first).unwrap().clone(), graph.node(last).unwrap().clone()];
        let mut images = Arc::default();
        let pasted = Batch::Paste {
            nodes: copied.clone(), offset: Vec2::splat(40.), keep_links: true, images: Arc::default(),
        }.apply(&mut graph, &mut images, false, false).unwrap().unwrap();
        assert_eq!(graph.node(pasted).unwrap().links["input"], pasted - 1);
        assert_eq!(graph.node(pasted - 1).unwrap().links["input"], input);
        assert_eq!(graph.node(pasted - 1).unwrap().pos, [260., 40.]);

        let mut another = Graph::new("Another graph");
        assert_eq!(another.add("color", [0., 0.]), input);
        let pasted = Batch::Paste {
            nodes: copied, offset: Vec2::ZERO, keep_links: false, images: Arc::default(),
        }.apply(&mut another, &mut images, false, false).unwrap().unwrap();
        assert_eq!(another.node(pasted).unwrap().links["input"], pasted - 1);
        assert!(another.node(pasted - 1).unwrap().links.is_empty());

        Batch::Move(vec![pasted - 1, pasted], Vec2::new(20., 4.))
            .apply(&mut another, &mut images, false, false).unwrap();
        assert_eq!(another.node(pasted - 1).unwrap().pos, [240., 4.]);
        assert_eq!(another.node(pasted).unwrap().pos, [460., 4.]);
        Batch::Remove(vec![pasted - 1, pasted]).apply(&mut another, &mut images, false, false).unwrap();
        assert_eq!(another.nodes.len(), 1);
    }

    #[test]
    fn batch_paste_keeps_both_bitmaps_when_graph_paths_collide() {
        let path = "graph-images/input.png";
        let red = Arc::new(texture_fill::Bitmap::from_rgba("red.png".into(),
            image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))).unwrap());
        let blue = Arc::new(texture_fill::Bitmap::from_rgba("blue.png".into(),
            image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 255, 255]))).unwrap());
        let mut images = Arc::new(texture_fill::GraphImages::from([(path.into(), red.clone())]));
        let mut graph = Graph::new("Images");
        let id = Batch::Paste {
            nodes: vec![material_painter_graph::Node {
                id: 1, def: "bitmap".into(), pos: [0., 0.], params: HashMap::from([
                    ("image".into(), Value::Image(Some(PathBuf::from(path)))),
                ]), links: HashMap::new(),
            }],
            offset: Vec2::ZERO, keep_links: false,
            images: Arc::new(texture_fill::GraphImages::from([(path.into(), blue.clone())])),
        }.apply(&mut graph, &mut images, false, false).unwrap().unwrap();
        let Value::Image(Some(copied)) = &graph.node(id).unwrap().params["image"] else { panic!() };
        assert!(Arc::ptr_eq(&images[path], &red));
        assert!(Arc::ptr_eq(&images[copied.to_str().unwrap()], &blue));
    }

    #[test]
    fn saved_source_keeps_connected_ports_and_published_parameters() {
        use material_painter_graph::def::parse;

        let old = "// @output gray\n// @input input: gray = 0\n// @param gain: float(0, 1) = 1\nfn node(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(in_input(uv).r * params.gain); }";
        let mut library = Library::load(None);
        library.defs.push(parse("graph/old", old).unwrap());
        let mut graph = Graph::new("Source update");
        let input = graph.add("noise", [0., 0.]);
        let target = graph.add("graph/old", [240., 0.]);
        let output = graph.add("output", [480., 0.]);
        graph.connect(input, target, "input");
        graph.connect(target, output, "input");
        graph.expose(target, "gain", &library);
        let before = graph.clone();

        for source in [old.replace("@input input", "@input renamed"), old.replace("@param gain", "@param renamed")] {
            let next = parse("user/next", &source).unwrap();
            assert!(update_source(&mut graph, target, &library, &next).is_err());
            assert_eq!(graph, before);
        }
        let next = parse("user/next", &old.replace("* params.gain", "+ params.gain")).unwrap();
        update_source(&mut graph, target, &library, &next).unwrap();
        let mut expected = before;
        expected.node_mut(target).unwrap().def = next.id;
        assert_eq!(graph, expected);
    }

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
