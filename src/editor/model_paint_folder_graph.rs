//! Live view of the fill layers directly inside a material group.
use super::*;
use material_painter_graph::{CHANNEL_USAGE, Graph, Role, Tag, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Weak;

const CHANNELS: [usize; 5] = [0, 1, 2, 3, 5];
const MASK_SLOTS: usize = 4;

pub(super) struct View {
    pub folder: u64,
    pub revision: u64,
    pub graph: Graph,
    pub payload: Arc<serde_json::Value>,
    pub images: Arc<texture_fill::GraphImages>,
    pub labels: Arc<std::collections::HashMap<u64, String>>,
    preview_masks: HashMap<u64, Weak<Mask>>,
}

pub(super) fn is_group(stack: &Stack) -> bool {
    stack
        .layers
        .get(stack.selected)
        .is_some_and(|layer| layer.kind == LayerKind::Folder)
}

fn fresh_id() -> u64 {
    loop {
        let id = bevy::asset::uuid::Uuid::new_v4().as_u128() as u64;
        if id != 0 {
            return id;
        }
    }
}

pub(super) fn rekey(layers: &mut [Layer]) {
    let mut ids = HashMap::new();
    for layer in &mut *layers {
        if let Some(old) = layer.graph_id {
            let id = fresh_id();
            ids.insert(old, id);
            layer.graph_id = Some(id);
        }
    }
    for layer in layers {
        let Some(graph) = layer.folder_graph.as_mut() else {
            continue;
        };
        let graph = Arc::make_mut(graph);
        let Some(tags) = graph["tags"].as_array_mut() else {
            continue;
        };
        let mut orphaned = HashSet::new();
        tags.retain_mut(|tag| {
            if let Some(id) = tag["layer"].as_u64().and_then(|old| ids.get(&old)) {
                tag["layer"] = serde_json::json!(id);
                true
            } else {
                if tag["role"] != "source"
                    && let Some(id) = tag["node"].as_u64()
                {
                    orphaned.insert(id);
                }
                false
            }
        });
        if let Some(nodes) = graph["nodes"].as_array_mut() {
            nodes.retain(|node| node["id"].as_u64().is_none_or(|id| !orphaned.contains(&id)));
            for node in nodes {
                if let Some(links) = node["links"].as_object_mut() {
                    links.retain(|_, id| id.as_u64().is_none_or(|id| !orphaned.contains(&id)));
                }
            }
        }
    }
}

pub(super) fn can_open(stack: &Stack) -> bool {
    can_open_at(stack, stack.selected)
}

pub(super) fn can_open_at(stack: &Stack, folder: usize) -> bool {
    if !stack.layers.get(folder).is_some_and(|layer| layer.kind == LayerKind::Folder) {
        return false;
    }
    let mut total = 0;
    let mut outputs = [false; CHANNELS.len()];
    for index in fills(stack, folder) {
        let layer = &stack.layers[index];
        total += usize::from(layer.mask.as_ref().is_some_and(|mask| {
            mask.generator.is_some() || has_mapped_pattern(mask)
        }));
        for (output, channel) in outputs.iter_mut().zip(CHANNELS) {
            if layer
                .channel(channel)
                .is_some_and(|pixels| pixels.len() >= 4)
            {
                *output = true;
                total += 1;
            }
        }
    }
    let outputs = outputs.into_iter().filter(|&enabled| enabled).count();
    let custom = stack.layers[folder]
        .folder_graph
        .as_deref()
        .and_then(|payload| payload["nodes"].as_array().zip(payload["tags"].as_array()))
        .map_or(0, |(nodes, tags)| {
            nodes
                .iter()
                .filter(|node| {
                    !tags
                        .iter()
                        .any(|tag| tag["node"] == node["id"] && tag["role"] != "source")
                })
                .count()
        });
    outputs > 0 && total + outputs + custom <= 256
}

fn fills(stack: &Stack, folder: usize) -> Vec<usize> {
    let depth = stack.layers[folder].depth + 1;
    stack
        .block(folder)
        .filter(|&i| {
            let layer = &stack.layers[i];
            layer.depth == depth && layer.kind == LayerKind::Fill && layer.visible
        })
        .collect()
}

pub(super) fn standalone(
    mut stack: Stack,
    size: u32,
) -> Result<(Graph, Arc<texture_fill::GraphImages>), String> {
    if !can_open(&stack) {
        return Err("Материал не помещается в один граф заливки".into());
    }
    let View { mut graph, images, .. } = start(&mut stack, size, 0, None);
    let library = material_graph::library();
    let mut groups = HashSet::new();
    for index in fills(&stack, stack.selected) {
        let layer = &stack.layers[index];
        let id = layer.graph_id.unwrap();
        let mut group = layer.name.clone();
        let mut suffix = 2;
        while !groups.insert(group.clone()) {
            group = format!("{} {suffix}", layer.name);
            suffix += 1;
        }
        let nodes: [Option<u64>; CHANNELS.len()] =
            std::array::from_fn(|channel| graph.tagged(id, Role::Channel(channel)));
        let mask = graph.tagged(id, Role::Mask).filter(|id| {
            graph.node(*id).is_some_and(|node| node.def == "smart_mask")
        });
        let mut expose = |nodes: &[u64], parameter: &str, label: &str| {
            let Some((&first, rest)) = nodes.split_first() else { return; };
            let index = graph.expose(first, parameter, &library);
            for &node in rest {
                graph.bind(index, node, parameter, &library);
            }
            graph.params[index].name = label.into();
            graph.params[index].group.clone_from(&group);
        };
        let all: Vec<_> = nodes.into_iter().flatten().collect();
        let scanned = layer.texture_fill.as_ref().is_some_and(|fill| fill.images.iter().any(Option::is_some));
        expose(&all, "opacity", "Непрозрачность");
        if scanned {
            expose(&all, "tiling", "Повтор текстуры");
        } else {
            for (channel, key, label) in [
                (0, "color", "Цвет"),
                (1, "value", "Шероховатость"),
                (2, "value", "Металличность"),
            ] {
                expose(nodes[channel].as_slice(), key, label);
            }
        }
        expose(nodes[3].as_slice(), "value", "Высота");
        expose(mask.as_slice(), "level", "Уровень маски");
        expose(mask.as_slice(), "scale", "Масштаб маски");
    }
    graph.tags.clear();
    Ok((graph, images))
}

pub(super) fn start(stack: &mut Stack, size: u32, revision: u64, previous: Option<&View>) -> View {
    let folder = stack.selected;
    let children = fills(stack, folder);
    let mut seen = std::collections::HashSet::new();
    for index in std::iter::once(folder).chain(children.iter().copied()) {
        let id = &mut stack.layers[index].graph_id;
        if id.is_none_or(|id| !seen.insert(id)) {
            *id = Some(loop {
                let id = fresh_id();
                if seen.insert(id) {
                    break id;
                }
            });
        }
    }
    let layer = &stack.layers[folder];
    let mut graph = layer
        .folder_graph
        .as_deref()
        .and_then(|data| material_graph::read(data).ok())
        .filter(|graph| graph.next_id < u64::MAX - 256)
        .unwrap_or_else(|| Graph::new(&layer.name));
    graph.name.clone_from(&layer.name);
    graph.res = size;
    graph.match_set = true;
    let depth = layer.depth + 1;
    let mask_ids: HashSet<_> = stack
        .block(folder)
        .filter_map(|index| {
            let layer = &stack.layers[index];
            (layer.depth == depth
                && layer.kind == LayerKind::Fill
                && layer.mask.as_ref().is_some_and(|mask| mask.graph.is_some()))
            .then_some(layer.graph_id)
            .flatten()
        })
        .collect();
    graph
        .tags
        .retain(|tag| !matches!(tag.role, Role::Source(_)) || mask_ids.contains(&tag.layer));
    let managed: HashSet<_> = graph
        .tags
        .iter()
        .filter(|tag| !matches!(tag.role, Role::Source(_)))
        .map(|tag| tag.node)
        .collect();
    let mut used: HashSet<_> = graph
        .nodes
        .iter()
        .map(|node| node.id)
        .filter(|id| !managed.contains(id))
        .collect();
    let mut images = layer.folder_images.as_ref().clone();
    let mut preview_masks = HashMap::new();
    let mut top = [None; CHANNELS.len()];
    let end = children.len() as f32 * 260.;
    for (column, &index) in children.iter().enumerate() {
        let layer = &stack.layers[index];
        let layer_id = layer.graph_id.unwrap();
        let x = column as f32 * 260.;
        let position = [x, -220.];
        let mask = layer.mask.as_ref().and_then(|mask| {
            if has_mapped_pattern(mask) {
                let path = format!("graph-images/folder-mask-{layer_id}.png");
                let source = Arc::downgrade(mask);
                let image = previous
                    .filter(|view| {
                        view.preview_masks.get(&layer_id).is_some_and(|old| old.ptr_eq(&source))
                    })
                    .and_then(|view| view.images.get(&path).cloned())
                    .or_else(|| pattern_mask_bitmap(mask, &path))?;
                preview_masks.insert(layer_id, source);
                images.insert(path.clone(), image);
                let node = ensure(&mut graph, layer_id, Role::Mask, "bitmap", position);
                let entry = graph.node_mut(node).unwrap();
                entry.params.clear();
                entry.params.insert("image".into(), Value::Image(Some(PathBuf::from(path))));
                return Some(node);
            }
            mask.generator.map(|generator| {
                let node = ensure(&mut graph, layer_id, Role::Mask, "smart_mask", position);
                write_mask(graph.node_mut(node).unwrap(), generator);
                node
            })
        });
        used.extend(mask);
        let source = graph.tagged(layer_id, Role::Source(0));
        let mask = if preview_masks.contains_key(&layer_id) { mask } else { source.or(mask) };
        for (channel_index, channel) in CHANNELS.into_iter().enumerate() {
            if !layer
                .channel(channel)
                .is_some_and(|pixels| pixels.len() >= 4)
            {
                continue;
            }
            let fill = layer.texture_fill.as_deref();
            // The sampled cache already includes variation. Feed the authored value into the node.
            let neutral = [if channel == 0 { 255 } else { 128 }; 3];
            let pixels = layer.channels[channel]
                .as_ref()
                .and_then(|pixels| pixels.get(..3))
                .unwrap_or(&neutral);
            let node = ensure(
                &mut graph,
                layer_id,
                Role::Channel(channel_index),
                "layer",
                [x, channel_index as f32 * 195.],
            );
            used.insert(node);
            let entry = graph.node_mut(node).unwrap();
            entry.links.clear();
            if let Some(below) = top[channel_index] {
                entry.links.insert("below".into(), below);
            }
            if let Some(mask) = mask {
                entry.links.insert("mask".into(), mask);
            }
            top[channel_index] = Some(node);
            let params = &mut entry.params;
            params.clear();
            params.insert("channel".into(), Value::Enum(channel_index));
            params.insert("opacity".into(), Value::Float(layer.opacity / 100.));
            params.insert("blend".into(), Value::Enum(blend_index(layer.blend)));
            if matches!(channel, 0 | 5) {
                let color = LinearRgba::from(Srgba::new(
                    f32::from(pixels[0]) / 255.,
                    f32::from(pixels[1]) / 255.,
                    f32::from(pixels[2]) / 255.,
                    1.,
                ));
                params.insert(
                    "color".into(),
                    Value::Color([color.red, color.green, color.blue]),
                );
            } else {
                let value = if channel == 3 {
                    (f32::from(pixels[0]) - 128.) / 127.
                } else {
                    f32::from(pixels[0]) / 255.
                };
                params.insert("value".into(), Value::Float(value.clamp(-1., 1.)));
            }
            if let Some(settings) = variation_settings(layer, channel) {
                let variation = settings.variation;
                let strength = match channel {
                    0 => variation.color,
                    1 => variation.roughness,
                    _ => variation.height,
                };
                params.insert("variation".into(), Value::Float(strength));
                params.insert("variation_scale".into(), Value::Float(variation.scale));
                params.insert("seed".into(), Value::Float(variation.seed));
                if matches!(channel, 0 | 1) {
                    params.insert("part_variation".into(), Value::Float(variation.parts));
                }
            }
            if let Some(fill) = fill {
                params.insert("tiling".into(), Value::Float(fill.settings.tiling));
                params.insert("sharpness".into(), Value::Float(fill.settings.sharpness));
                if let Some(image) = &fill.images[channel] {
                    let key = format!("graph-images/layer-{layer_id}-{channel}.png");
                    params.insert("image".into(), Value::Image(Some(PathBuf::from(&key))));
                    images.insert(key, image.clone());
                    if channel == 3 {
                        params.insert("value".into(), Value::Float(fill.settings.height_strength));
                    }
                }
            }
        }
    }
    let folder = layer.graph_id.unwrap();
    for (channel_index, from) in top.into_iter().enumerate() {
        if let Some(from) = from {
            let usage = CHANNEL_USAGE[CHANNELS[channel_index]];
            let id = ensure(
                &mut graph,
                folder,
                Role::Output(usage),
                "output",
                [end, channel_index as f32 * 195.],
            );
            used.insert(id);
            let node = graph.node_mut(id).unwrap();
            node.params.insert("usage".into(), Value::Enum(usage));
            node.links.insert("input".into(), from);
        }
    }
    graph.nodes.retain(|node| used.contains(&node.id));
    graph.tags.retain(|tag| used.contains(&tag.node));
    let mut restored_images = stack.layers[stack.selected].folder_images.clone();
    let mut restored = false;
    for (column, index) in children.into_iter().enumerate() {
        let layer = &stack.layers[index];
        let layer_id = layer.graph_id.unwrap();
        if graph.tagged(layer_id, Role::Source(0)).is_some() {
            continue;
        }
        let Some(mask) = layer.mask.as_ref().and_then(|mask| mask.graph.as_ref()) else {
            continue;
        };
        let position = [column as f32 * 260., -450. - column as f32 * 210.];
        if let Some(source) =
            restore_mask(&mut graph, &mut restored_images, layer_id, position, mask)
        {
            if !preview_masks.contains_key(&layer_id) {
                wire_mask(&mut graph, layer_id, source);
            }
            restored = true;
        }
    }
    if restored {
        images.extend(
            restored_images
                .iter()
                .map(|(path, bitmap)| (path.clone(), bitmap.clone())),
        );
        let layer = &mut stack.layers[stack.selected];
        layer.folder_images = restored_images;
        store(layer, &graph);
    }
    let labels = graph
        .tags
        .iter()
        .filter_map(|tag| {
            let layer = stack
                .layers
                .iter()
                .find(|layer| layer.graph_id == Some(tag.layer))?;
            let role = match tag.role {
                Role::Channel(index) => super::CHANNELS[*CHANNELS.get(index)?],
                Role::Mask if preview_masks.contains_key(&tag.layer) => "Итоговая маска",
                Role::Mask => "Маска",
                Role::Output(usage) => {
                    let channel = CHANNEL_USAGE.iter().position(|&value| value == usage)?;
                    super::CHANNELS[channel]
                }
                _ => return None,
            };
            Some((tag.node, format!("{role} · {}", layer.name)))
        })
        .collect();
    View {
        folder,
        revision,
        payload: Arc::new(material_graph::encode(&graph)),
        graph,
        images: Arc::new(images),
        labels: Arc::new(labels),
        preview_masks,
    }
}

fn has_mapped_pattern(mask: &Mask) -> bool {
    mask.generator.is_some_and(|generator| generator.kind.uses_pattern_mapping())
        || mask.layers.iter().any(|layer| {
            layer.enabled && layer.opacity != 0
                && layer.generator.is_some_and(|generator| generator.kind.uses_pattern_mapping())
        })
}

fn pattern_mask_bitmap(mask: &Mask, path: &str) -> Option<Arc<texture_fill::Bitmap>> {
    let pixels = mask.effective();
    let edge = (pixels.len() / 4).isqrt() as u32;
    let pixels = if edge > material_graph::previews::PREVIEW_SIZE {
        let source = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(edge, edge, pixels)?;
        let edge = material_graph::previews::PREVIEW_SIZE;
        image::imageops::resize(&source, edge, edge, image::imageops::FilterType::Triangle)
    } else {
        image::RgbaImage::from_raw(edge, edge, pixels.to_vec())?
    };
    let name = path.rsplit('/').next().unwrap();
    texture_fill::Bitmap::from_rgba(name.into(), pixels).ok().map(Arc::new)
}

fn wire_mask(graph: &mut Graph, layer: u64, source: u64) {
    for role in 0..CHANNELS.len() {
        if let Some(channel) = graph.tagged(layer, Role::Channel(role)) {
            graph.connect(source, channel, "mask");
        }
    }
}

pub(super) fn sync_standalone_mask(stack: &mut Stack, child: usize) {
    let layer = &stack.layers[child];
    let Some(id) = layer.graph_id else { return };
    let Some(parent) = stack.parents(child).next() else {
        return;
    };
    let Some(mut graph) = stack.layers[parent]
        .folder_graph
        .as_deref()
        .and_then(|payload| material_graph::read(payload).ok())
    else {
        return;
    };
    let Some(source) = graph.tagged(id, Role::Source(0)) else {
        return;
    };
    let mut position = graph.node(source).unwrap().pos;
    let mask = layer.mask.as_ref().and_then(|mask| mask.graph.clone());

    graph
        .tags
        .retain(|tag| tag.layer != id || !matches!(tag.role, Role::Source(_)));
    for role in 0..CHANNELS.len() {
        if let Some(channel) = graph.tagged(id, Role::Channel(role)) {
            graph.node_mut(channel).unwrap().links.remove("mask");
        }
    }
    let branch = upstream(&graph, [source]);
    // Keep any part of the old branch used by another mask or by an independent node.
    let retained = upstream(
        &graph,
        graph
            .tags
            .iter()
            .filter(|tag| matches!(tag.role, Role::Source(_)))
            .map(|tag| tag.node)
            .chain(
                graph
                    .nodes
                    .iter()
                    .filter(|node| !branch.contains(&node.id))
                    .flat_map(|node| node.links.values().copied()),
            ),
    );
    if retained.contains(&source) {
        let (min, max) = branch
            .iter()
            .filter_map(|id| graph.node(*id))
            .map(|node| node.pos[0])
            .fold((position[0], position[0]), |(min, max), x| {
                (min.min(x), max.max(x))
            });
        position[0] += max - min + 260.;
    }
    for &id in branch.difference(&retained) {
        graph.remove(id);
    }
    let folder = &mut stack.layers[parent];
    retain_images(&mut folder.folder_images, &graph);
    if let Some(mask) = mask
        && let Some(source) =
            restore_mask(&mut graph, &mut folder.folder_images, id, position, &mask)
    {
        wire_mask(&mut graph, id, source);
    }
    store(folder, &graph);
}

fn ensure(graph: &mut Graph, layer: u64, role: Role, def: &str, pos: [f32; 2]) -> u64 {
    if let Some(node) = graph.tagged(layer, role)
        && graph.node(node).is_some_and(|node| node.def == def)
    {
        return node;
    }
    graph
        .tags
        .retain(|tag| tag.layer != layer || tag.role != role);
    let node = graph.add(def, pos);
    graph.tags.push(Tag { node, layer, role });
    node
}

pub(super) fn managed(graph: &Graph, id: u64) -> Option<Tag> {
    graph
        .tags
        .iter()
        .copied()
        .find(|tag| tag.node == id && !matches!(tag.role, Role::Source(_)))
}

fn custom(graph: &Graph, id: u64) -> bool {
    graph.node(id).is_some() && managed(graph, id).is_none()
}

fn image_parameter(graph: &Graph, id: u64, index: usize) -> bool {
    let library = material_graph::library();
    graph
        .node(id)
        .and_then(|node| library.find(&node.def)?.params.get(index))
        .is_some_and(|param| param.kind == material_painter_graph::ParamKind::Image)
}

fn channel_image_target(
    stack: &Stack,
    graph: &Graph,
    target: material_graph::ParameterTarget,
) -> Option<(usize, usize)> {
    let material_graph::ParameterTarget::Node(id, parameter) = target else {
        return None;
    };
    let tag = managed(graph, id)?;
    let Role::Channel(channel) = tag.role else {
        return None;
    };
    let layer = stack.layers.iter().position(|layer| {
        layer.graph_id == Some(tag.layer)
            && layer
                .texture_fill
                .as_ref()
                .is_none_or(|fill| fill.graph.is_none())
    })?;
    image_parameter(graph, id, parameter).then_some((layer, *CHANNELS.get(channel)?))
}

pub(super) fn bitmap_target(
    stack: &Stack,
    graph: &Graph,
    target: material_graph::ParameterTarget,
) -> bool {
    use material_graph::ParameterTarget;
    let library = material_graph::library();
    match target {
        ParameterTarget::Node(id, index) => {
            (custom(graph, id) && image_parameter(graph, id, index))
                || channel_image_target(stack, graph, target).is_some()
        }
        ParameterTarget::Published(index) => graph
            .param_value(index, &library)
            .is_some_and(|(param, _)| param.kind == material_painter_graph::ParamKind::Image),
    }
}

fn retain_images(images: &mut Arc<texture_fill::GraphImages>, graph: &Graph) {
    let paths: HashSet<_> = material_graph::file::image_paths(graph)
        .filter_map(|path| path.to_str())
        .collect();
    if images.keys().any(|path| !paths.contains(path.as_str())) {
        Arc::make_mut(images).retain(|key, _| paths.contains(key.as_str()));
    }
}

pub(super) fn store(layer: &mut Layer, graph: &Graph) {
    retain_images(&mut layer.folder_images, graph);
    let mut payload = material_graph::encode(graph);
    // The preview is rebuilt from the mask on open. Keep the node position, but
    // avoid saving a reference to a generated image in the editable folder.
    let generated: HashSet<_> = graph.tags.iter()
        .filter(|tag| tag.role == Role::Mask)
        .filter_map(|tag| graph.node(tag.node).filter(|node| node.def == "bitmap").map(|node| node.id))
        .collect();
    if let Some(nodes) = payload["nodes"].as_array_mut() {
        for node in nodes.iter_mut().filter(|node| node["id"].as_u64().is_some_and(|id| generated.contains(&id))) {
            if let Some(params) = node["params"].as_object_mut() {
                params.remove("image");
            }
        }
    }
    layer.folder_graph = Some(Arc::new(payload));
}

pub(super) fn replace_bitmap(
    stack: &mut Stack,
    previous: &Graph,
    target: material_graph::ParameterTarget,
    path: &std::path::Path,
) -> Result<Masks, String> {
    if let Some((layer, channel)) = channel_image_target(stack, previous, target) {
        let image = Arc::new(texture_fill::Bitmap::open(path)?);
        stack.layers[layer].edit_texture_fill(|fill| {
            fill.images[channel] = Some(image);
            fill.settings.projection = texture_fill::Projection::Triplanar;
        });
        return Ok(Vec::new());
    }
    let mut graph = previous.clone();
    let folder = &mut stack.layers[stack.selected];
    material_graph::set_bitmap(&mut graph, &mut folder.folder_images, target, path)?;
    store(folder, &graph);
    let (mut bytes, mut texels) = (0, 0);
    let count = folder.folder_images.len();
    for bitmap in folder.folder_images.values() {
        material_graph::file::check_size(&mut bytes, &mut texels, count, bitmap)?;
    }
    Ok(changed_masks(stack, previous, &graph))
}

pub(super) fn clear_channel_image(
    stack: &mut Stack,
    graph: &Graph,
    target: material_graph::ParameterTarget,
) -> bool {
    let Some((layer, channel)) = channel_image_target(stack, graph, target) else {
        return false;
    };
    stack.layers[layer].edit_texture_fill(|fill| fill.images[channel] = None);
    true
}

pub(super) fn can_edit(stack: &Stack, graph: &Graph, edit: material_graph::Edit) -> bool {
    use material_graph::{Edit, ParameterTarget};
    let input = |id, index: usize| {
        graph
            .node(id)
            .and_then(|node| {
                material_graph::library().find(&node.def).map(|def| {
                    def.id == "layer"
                        && def
                            .inputs
                            .get(index)
                            .is_some_and(|port| port.name == "mask")
                })
            })
            .unwrap_or(false)
    };
    let mask = |id, index| {
        managed(graph, id)
            .is_some_and(|tag| matches!(tag.role, Role::Channel(_)) && input(id, index))
    };
    let can_connect = |from, to, index| custom(graph, from) && (custom(graph, to) || mask(to, index));
    let can_disconnect = |id, index| {
        custom(graph, id) || (mask(id, index)
            && managed(graph, id).is_some_and(|tag| graph.tagged(tag.layer, Role::Source(0)).is_some()))
    };
    match edit {
        Edit::Batch(..) => true,
        #[cfg(test)]
        Edit::Move(id, _) => graph.node(id).is_some(),
        Edit::Add(index, _) | Edit::AddConnected(index, _, _) => {
            graph.nodes.len() < 256
                && material_graph::library()
                    .defs
                    .get(index)
                    .is_some_and(|def| !def.shading && def.id != material_painter_graph::OUTPUT_DEF)
                && match edit {
                    Edit::AddConnected(_, _, source) => custom(graph, source.node)
                        && source.input.is_none_or(|old| can_disconnect(old.node, old.index)),
                    _ => true,
                }
        }
        Edit::Remove(id) => custom(graph, id),
        Edit::UpdateSource(id, _) => custom(graph, id),
        Edit::Connect(from, to, index) => can_connect(from, to, index),
        Edit::Reconnect(source, target) => can_connect(source.node, target.node, target.index)
            && source.input.is_none_or(|old| can_disconnect(old.node, old.index)),
        Edit::Disconnect(id, index) => can_disconnect(id, index),
        Edit::SetMeshMap(ParameterTarget::Node(id, _), _) => custom(graph, id),
        Edit::SetMeshMap(ParameterTarget::Published(index), _)
        | Edit::AdjustParameter(index, ..)
        | Edit::ResetParameter(index) => index < graph.params.len(),
        Edit::ClearBitmap(target) => bitmap_target(stack, graph, target),
        _ => false,
    }
}

pub(super) fn edit_graph(
    stack: &Stack,
    graph: &mut Graph,
    edit: material_graph::Edit,
) -> Result<Option<u64>, String> {
    use material_graph::Edit;
    let (old, new) = match edit {
        Edit::Connect(from, to, _) => (None, managed(graph, to).map(|tag| (tag, from))),
        Edit::Disconnect(to, _) => (managed(graph, to), None),
        Edit::Reconnect(source, target) => (
            source.input.and_then(|port| managed(graph, port.node)),
            managed(graph, target.node).map(|tag| (tag, source.node)),
        ),
        Edit::AddConnected(_, _, source) => (source.input.and_then(|port| managed(graph, port.node)), None),
        _ => (None, None),
    };
    // Layer channel nodes can expose the same mask as several graph inputs.
    if let Edit::Reconnect(_, target) = edit
        && old.zip(new).is_some_and(|(old, (new, _))| old.layer == new.layer)
    {
        return Ok(Some(target.node));
    }
    if let Some((tag, _)) = new
        && graph.tagged(tag.layer, Role::Source(0)).is_none()
        && stack.layers.iter().any(|layer| {
            layer.graph_id == Some(tag.layer)
                && layer.mask.as_ref().is_some_and(|mask| mask.graph.is_some())
        })
    {
        return Err("У слоя уже есть отдельный граф маски".into());
    }
    let selection = edit.apply(graph, None)?;
    for tag in old.into_iter().chain(new.map(|(tag, _)| tag)) {
        graph.tags.retain(|t| t.layer != tag.layer || !matches!(t.role, Role::Source(_)));
    }
    if let Some((tag, from)) = new {
        graph.tags.push(Tag { node: from, layer: tag.layer, role: Role::Source(0) });
    }
    graph
        .tags
        .retain(|tag| graph.nodes.iter().any(|node| node.id == tag.node));
    Ok(selection)
}

fn upstream(graph: &Graph, roots: impl IntoIterator<Item = u64>) -> HashSet<u64> {
    let mut keep = HashSet::new();
    let mut frontier: Vec<_> = roots.into_iter().collect();
    while let Some(id) = frontier.pop() {
        if keep.insert(id)
            && let Some(node) = graph.node(id)
        {
            frontier.extend(node.links.values().copied());
        }
    }
    keep
}

fn mask_subgraph(graph: &Graph, source: u64, name: &str) -> Graph {
    let keep = upstream(graph, [source]);
    let mut result = Graph::new(name);
    result.res = graph.res;
    result.match_set = true;
    result.nodes.extend(
        graph
            .nodes
            .iter()
            .filter(|node| keep.contains(&node.id))
            .cloned(),
    );
    result
        .params
        .extend(graph.params.iter().filter_map(|parameter| {
            let mut parameter = parameter.clone();
            parameter.binds.retain(|(id, _)| keep.contains(id));
            (!parameter.binds.is_empty()).then_some(parameter)
        }));
    // Derive the output ID from the retained nodes so unrelated edits leave the mask intact.
    result.next_id = result
        .nodes
        .iter()
        .map(|node| node.id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|id| *id < u64::MAX)
        .unwrap_or_else(|| (1..).find(|id| !keep.contains(id)).unwrap());
    let output = result.add("output", [0., 0.]);
    result.node_mut(output).unwrap().params.insert(
        "usage".into(),
        Value::Enum(material_painter_graph::MASK_USAGE),
    );
    result.connect(source, output, "input");
    result
}

fn restore_mask(
    graph: &mut Graph,
    images: &mut Arc<texture_fill::GraphImages>,
    layer: u64,
    position: [f32; 2],
    mask: &crate::editor::model_paint_data::mask::GraphMask,
) -> Option<u64> {
    let source = material_graph::read(&mask.payload).ok()?;
    let library = material_graph::library();
    source.validate_texture_graph(&library).ok()?;
    let outputs = source.outputs(&library);
    let output = outputs[material_painter_graph::MASK_USAGE].or(outputs[0])?;
    let root = *source.node(output)?.links.get("input")?;
    let mut branch = mask_subgraph(&source, root, &source.name);
    branch.nodes.pop(); // The folder connects the source directly to its Layer nodes.
    if graph.nodes.len() + branch.nodes.len() > 256
        || graph
            .next_id
            .checked_add(branch.nodes.len() as u64)
            .is_none()
    {
        return None;
    }
    let mut merged = images.as_ref().clone();
    let mut image_ids = HashMap::new();
    for path in material_graph::file::image_paths(&branch) {
        let Some((key, bitmap)) = path
            .to_str()
            .and_then(|key| mask.images.get(key).map(|bitmap| (key, bitmap)))
        else {
            continue;
        };
        if !image_ids.contains_key(key) {
            let renamed = format!(
                "graph-images/mask-{}-{}.png",
                graph.next_id,
                image_ids.len()
            );
            merged.insert(renamed.clone(), bitmap.clone());
            image_ids.insert(key.to_owned(), PathBuf::from(renamed));
        }
    }
    let (mut bytes, mut texels) = (0, 0);
    for bitmap in merged.values() {
        material_graph::file::check_size(&mut bytes, &mut texels, merged.len(), bitmap).ok()?;
    }
    let rewrite_image = |value: &mut Value| {
        if let Value::Image(Some(path)) = value
            && let Some(renamed) = path.to_str().and_then(|key| image_ids.get(key))
        {
            path.clone_from(renamed);
        }
    };
    let ids: HashMap<_, _> = branch
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id, graph.next_id + index as u64))
        .collect();
    let origin = source.node(root)?.pos;
    for mut node in branch.nodes {
        node.id = ids[&node.id];
        for from in node.links.values_mut() {
            *from = ids[from];
        }
        for value in node.params.values_mut() {
            rewrite_image(value);
        }
        node.pos = [
            node.pos[0] - origin[0] + position[0],
            node.pos[1] - origin[1] + position[1],
        ];
        graph.nodes.push(node);
    }
    for mut parameter in branch.params {
        for (id, _) in &mut parameter.binds {
            *id = ids[id];
        }
        if let Some(default) = &mut parameter.default {
            rewrite_image(default);
        }
        graph.params.push(parameter);
    }
    graph.next_id += ids.len() as u64;
    let node = ids[&root];
    graph.tags.push(Tag {
        node,
        layer,
        role: Role::Source(0),
    });
    *images = Arc::new(merged);
    Some(node)
}

pub(super) type Masks = Vec<(usize, Option<Graph>)>;

pub(super) fn changed_masks(stack: &Stack, old: &Graph, new: &Graph) -> Masks {
    let depth = stack.layers[stack.selected].depth + 1;
    stack
        .block(stack.selected)
        .filter_map(|index| {
            let layer = &stack.layers[index];
            if layer.depth != depth || layer.kind != LayerKind::Fill {
                return None;
            }
            let id = layer.graph_id?;
            let name = format!("{} · Маска группы", layer.name);
            let extract = |graph: &Graph| {
                graph.tagged(id, Role::Source(0)).map(|source| {
                    let mut branch = mask_subgraph(graph, source, &name);
                    if let Some(mask) = layer.mask.as_ref().and_then(|mask| mask.graph.as_ref()) {
                        branch.res = mask.payload["res"]
                            .as_u64()
                            .and_then(|size| u32::try_from(size).ok())
                            .unwrap_or(branch.res);
                        branch.match_set = mask.payload["match_set"].as_bool().unwrap_or(false);
                    }
                    branch
                })
            };
            let before = extract(old);
            let after = extract(new);
            let same = match (&before, &after) {
                (Some(a), Some(b)) => {
                    a.nodes.len() == b.nodes.len()
                        && a.nodes.iter().zip(&b.nodes).all(|(a, b)| {
                            a.id == b.id
                                && a.def == b.def
                                && a.params == b.params
                                && a.links == b.links
                        })
                }
                (None, None) => true,
                _ => false,
            };
            (!same).then_some((index, after))
        })
        .collect()
}

fn blend_index(blend: Blend) -> usize {
    if blend == Blend::AddHeight {
        Blend::Add as usize
    } else {
        blend as usize
    }
}

fn variation_settings(layer: &Layer, channel: usize) -> Option<texture_fill::Settings> {
    let index = texture_fill::VARIATION_CHANNELS
        .iter()
        .position(|&item| item == channel)?;
    let settings = layer
        .texture_fill
        .as_ref()
        .map_or_else(texture_fill::Settings::default, |fill| fill.settings);
    settings.variation_channels[index].then_some(settings)
}

#[derive(Clone, Copy)]
struct MaskSlot {
    input: usize,
    weight: i16,
}

impl MaskSlot {
    fn read(node: &material_painter_graph::Node, slot: usize) -> Option<Self> {
        let Value::Enum(input) = node.params.get(&format!("input_{slot}"))? else {
            return None;
        };
        let weight = match node.params.get(&format!("weight_{slot}")) {
            Some(Value::Float(weight)) => (weight * 100.).round() as i16,
            _ => 100,
        };
        (*input <= GeneratorInput::ALL.len()).then_some(Self {
            input: *input,
            weight,
        })
    }

    fn write(self, node: &mut material_painter_graph::Node, slot: usize) {
        node.params
            .insert(format!("input_{slot}"), Value::Enum(self.input));
        node.params.insert(
            format!("weight_{slot}"),
            Value::Float(f32::from(self.weight) / 100.),
        );
    }
}

fn mask_slot_param(key: &str) -> Option<(usize, bool)> {
    let (name, slot) = key.split_once('_')?;
    let slot = slot
        .parse()
        .ok()
        .filter(|slot| (1..=MASK_SLOTS).contains(slot))?;
    match name {
        "input" => Some((slot, true)),
        "weight" => Some((slot, false)),
        _ => None,
    }
}

fn write_mask(node: &mut material_painter_graph::Node, generator: Generator) {
    let mut slots: [Option<MaskSlot>; MASK_SLOTS] =
        std::array::from_fn(|index| MaskSlot::read(node, index + 1));
    let mut used = HashSet::new();
    let weights = generator.weights();
    for slot in &mut slots {
        *slot = slot.and_then(|mut current| {
            let index = current.input.checked_sub(1)?;
            if (weights[index] == 0 && current.weight != 0) || !used.insert(current.input) {
                return None;
            }
            current.weight = weights[index];
            Some(current)
        });
    }
    let mut strongest: Vec<_> = weights
        .into_iter()
        .enumerate()
        .filter(|&(index, weight)| weight != 0 && !used.contains(&(index + 1)))
        .collect();
    strongest.sort_by_key(|&(_, weight)| std::cmp::Reverse(weight.unsigned_abs()));
    let mut strongest = strongest.into_iter();
    node.params.clear();
    node.links.clear();
    for (key, value) in ["level", "contrast", "scale", "noise", "seed"]
        .into_iter()
        .zip(generator.values)
    {
        let value = if matches!(key, "level" | "contrast" | "noise") {
            value / 100.
        } else {
            value
        };
        node.params.insert(key.into(), Value::Float(value));
    }
    node.params
        .insert("invert".into(), Value::Bool(generator.invert));
    node.params.insert(
        "edge_width".into(),
        Value::Float(f32::from(generator.edge_width) / 1000.),
    );
    for (index, slot) in slots.into_iter().enumerate() {
        let slot = slot
            .or_else(|| {
                strongest.next().map(|(index, weight)| MaskSlot {
                    input: index + 1,
                    weight,
                })
            })
            .unwrap_or(MaskSlot {
                input: 0,
                weight: 100,
            });
        slot.write(node, index + 1);
    }
}

fn adjust_mask_slot(
    generator: &mut Generator,
    previous: &material_painter_graph::Node,
    node: &mut material_painter_graph::Node,
    slot: usize,
    changing_input: bool,
) -> bool {
    let Some(old) = MaskSlot::read(previous, slot) else {
        return false;
    };
    let Some(mut new) = MaskSlot::read(node, slot) else {
        return false;
    };
    let input = |value: MaskSlot| {
        value
            .input
            .checked_sub(1)
            .map(|index| GeneratorInput::ALL[index])
    };
    if changing_input {
        if new.input != 0
            && let Some(other) = (1..=MASK_SLOTS)
                .filter(|&index| index != slot)
                .find(|&index| {
                    MaskSlot::read(previous, index).is_some_and(|other| other.input == new.input)
                })
        {
            old.write(node, other);
            new.weight = generator.weight(input(new).unwrap());
        } else {
            if let Some(old_input) = input(old) {
                generator.set_weight(old_input, 0);
            }
            new.weight = input(new).map_or(100, |new_input| {
                let existing = generator.weight(new_input);
                let weight = if existing != 0 {
                    existing
                } else if old.weight != 0 {
                    old.weight
                } else {
                    100
                };
                generator.set_weight(new_input, weight);
                weight
            });
        }
    } else {
        let Some(input) = input(new) else {
            return false;
        };
        new.weight = new
            .weight
            .clamp(-Generator::WEIGHT_LIMIT, Generator::WEIGHT_LIMIT);
        if new.weight == old.weight {
            return false;
        }
        generator.set_weight(input, new.weight);
    }
    new.write(node, slot);
    true
}

pub(super) fn adjustment_allowed(stack: &Stack, graph: &Graph, id: u64, parameter: usize) -> bool {
    let lib = material_graph::library();
    if custom(graph, id) {
        return graph
            .node(id)
            .and_then(|node| {
                let key = &lib.find(&node.def)?.params.get(parameter)?.name;
                lib.value(node, key)
            })
            .is_some_and(|value| material_graph::adjustable(&value));
    }
    managed(graph, id)
        .zip(graph.node(id))
        .is_some_and(|(tag, node)| {
            let Some(key) = lib
                .find(&node.def)
                .and_then(|def| def.params.get(parameter))
                .map(|p| p.name.as_str())
            else {
                return false;
            };
            match tag.role {
                Role::Channel(index) => {
                    let Some(&channel) = CHANNELS.get(index) else {
                        return false;
                    };
                    let Some(layer) = stack
                        .layers
                        .iter()
                        .find(|layer| layer.graph_id == Some(tag.layer))
                    else {
                        return false;
                    };
                    let image = node.params.contains_key("image");
                    let scalar_or_color = if matches!(channel, 0 | 5) {
                        key == "color"
                    } else {
                        key == "value"
                    };
                    matches!(key, "opacity" | "blend")
                        || (scalar_or_color
                            && !image
                            && layer
                                .texture_fill
                                .as_ref()
                                .is_none_or(|fill| fill.graph.is_none()))
                        || (matches!(key, "tiling" | "sharpness") && image)
                        || (variation_settings(layer, channel).is_some()
                            && (matches!(key, "variation" | "variation_scale" | "seed")
                                || (key == "part_variation" && matches!(channel, 0 | 1))))
                }
                Role::Mask => {
                    matches!(
                        key,
                        "level" | "contrast" | "scale" | "noise" | "seed" | "edge_width" | "invert"
                    ) || mask_slot_param(key).is_some_and(|(slot, input)| {
                        input || MaskSlot::read(node, slot).is_some_and(|slot| slot.input != 0)
                    })
                }
                _ => false,
            }
        })
}

pub(super) fn step_parameter(
    graph: &mut Graph,
    id: u64,
    parameter: usize,
    component: usize,
    direction: i8,
) -> bool {
    if managed(graph, id).is_some_and(|tag| tag.role == Role::Mask)
        && let Some((slot, false)) = material_graph::library()
            .find("smart_mask")
            .and_then(|def| mask_slot_param(&def.params.get(parameter)?.name))
        && let Some(node) = graph.node_mut(id)
        && let Some(mut input) = MaskSlot::read(node, slot)
    {
        let weight = (input.weight + 5 * i16::from(direction.signum()))
            .clamp(-Generator::WEIGHT_LIMIT, Generator::WEIGHT_LIMIT);
        if weight == input.weight {
            return false;
        }
        input.weight = weight;
        input.write(node, slot);
        return true;
    }
    material_graph::adjust(graph, id, parameter, component, direction)
}

pub(super) fn adjust(
    stack: &mut Stack,
    previous: &Graph,
    graph: &mut Graph,
    id: u64,
    parameter: usize,
    size: u32,
) -> bool {
    if !adjustment_allowed(stack, previous, id, parameter) {
        return false;
    }
    let Some(tag) = managed(graph, id) else {
        return false;
    };
    let node = graph.node(id).unwrap();
    let lib = material_graph::library();
    let key = &lib.find(&node.def).unwrap().params[parameter].name;
    let Some(value) = lib.value(node, key) else {
        return false;
    };
    let Some(layer) = stack
        .layers
        .iter_mut()
        .find(|layer| layer.graph_id == Some(tag.layer))
    else {
        return false;
    };
    match tag.role {
        Role::Channel(index) => adjust_layer(layer, CHANNELS[index], key, value, size),
        Role::Mask => {
            let Some(mask) = layer.mask.as_mut() else {
                return false;
            };
            let mask = Arc::make_mut(mask);
            let Some(mut generator) = mask.generator else {
                return false;
            };
            if let Some((slot, changing_input)) = mask_slot_param(key) {
                let Some(previous) = previous.node(id) else {
                    return false;
                };
                if !adjust_mask_slot(
                    &mut generator,
                    previous,
                    graph.node_mut(id).unwrap(),
                    slot,
                    changing_input,
                ) {
                    return false;
                }
                *mask.edit_generator() = Some(generator);
                store(&mut stack.layers[stack.selected], graph);
                return true;
            }
            match (key.as_str(), value) {
                ("invert", Value::Bool(value)) => generator.invert = value,
                ("edge_width", Value::Float(value)) => {
                    generator.edge_width = (value * 1000.).round().clamp(2., 500.) as u16
                }
                (key, Value::Float(value)) => {
                    let Some(index) = ["level", "contrast", "scale", "noise", "seed"]
                        .iter()
                        .position(|&name| name == key)
                    else {
                        return false;
                    };
                    let scale = if matches!(index, 0 | 1 | 3) { 100. } else { 1. };
                    let Some(&(_, min, max, _)) = generator.controls().get(index) else {
                        return false;
                    };
                    generator.set_value(index, (value * scale).clamp(min, max));
                }
                _ => return false,
            }
            *mask.edit_generator() = Some(generator);
            true
        }
        _ => false,
    }
}

fn adjust_layer(layer: &mut Layer, channel: usize, key: &str, value: Value, size: u32) -> bool {
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    match (key, value) {
        ("opacity", Value::Float(value)) => layer.opacity = (value * 100.).clamp(0., 100.),
        ("blend", Value::Enum(value)) => {
            let Some(&blend) = Blend::ALL.get(value) else {
                return false;
            };
            layer.blend = blend;
        }
        ("color", Value::Color([r, g, b])) if matches!(channel, 0 | 5) => {
            let color = Srgba::from(LinearRgba::new(r, g, b, 1.));
            layer.channels[channel] = Some(crate::editor::model_paint_data::solid(
                size,
                [byte(color.red), byte(color.green), byte(color.blue), 255],
            ));
        }
        ("value", Value::Float(value)) => {
            let value = if channel == 3 {
                (128. + value * 127.).round() as u8
            } else {
                byte(value)
            };
            layer.channels[channel] = Some(crate::editor::model_paint_data::solid(
                size,
                [value, value, value, 255],
            ));
        }
        ("tiling" | "sharpness", Value::Float(value)) => layer.edit_texture_fill(|fill| {
            if key == "tiling" {
                fill.settings.tiling = value;
            } else {
                fill.settings.sharpness = value;
            }
        }),
        ("variation" | "variation_scale" | "seed" | "part_variation", Value::Float(value)) => layer
            .edit_texture_fill(|fill| {
                let variation = &mut fill.settings.variation;
                match (key, channel) {
                    ("variation", 0) => variation.color = value,
                    ("variation", 1) => variation.roughness = value,
                    ("variation", 3) => variation.height = value,
                    ("variation_scale", _) => variation.scale = value.clamp(0.5, 60.),
                    ("seed", _) => variation.seed = value,
                    ("part_variation", _) => variation.parts = value,
                    _ => {}
                }
            }),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_paint_data::{mask::GraphMask, solid};

    #[test]
    fn folder_graph_previews_projected_gradient_stack_and_keeps_the_authored_branch() {
        use crate::editor::{
            model_mesh::{Corner, Face, Geometry},
            model_paint_data::mask::{GeneratorSurface, GradientMapping, GradientProjection, GradientVolume, MaskBlend},
        };

        let size = 16;
        let mut session = Session::default();
        session.texture_size = size;
        session.edit(PaintAction::Add(LayerKind::Folder));
        session.edit(PaintAction::Add(LayerKind::Fill));
        let child = session.stack.selected;
        let folder = session.stack.parents(child).next().unwrap();
        let mut mask = Mask::new(solid(size, [64; 4]), Vec::new());
        let mut generator = Generator::new(GeneratorKind::GradientLinear);
        generator.gradient_mapping = Some(GradientMapping {
            volume: Some(GradientVolume {
                center: [0.5, 0.5, 0.],
                size: [0.5, 1., 1.],
                rotation: [0.; 3],
                crop: true,
                projection: GradientProjection::Planar { depth: None },
                backface: None,
            }),
            ..Default::default()
        });
        mask.edit_layers().push(MaskLayer {
            generator: Some(generator),
            blend: MaskBlend::Normal,
            ..Default::default()
        });
        let mesh = Geometry {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y],
            faces: vec![Face {
                corners: [Vec2::Y, Vec2::ONE, Vec2::X, Vec2::ZERO]
                    .into_iter()
                    .enumerate()
                    .map(|(index, uv)| {
                        let mut corner = Corner::new(index);
                        corner.uv = uv;
                        corner
                    })
                    .collect(),
                material: 0,
                smooth: false,
            }],
            ..default()
        };
        mask.bind(Arc::new(GeneratorSurface::new(
            mesh.into(), 0, size, Vec2::ZERO, Transform::IDENTITY, None,
        )));
        let mut authored = Graph::new("Отдельная маска");
        let root = authored.add("color", [0., 0.]);
        let authored = mask_subgraph(&authored, root, "Отдельная маска");
        mask.set_graph(Arc::new(GraphMask::new(
            Arc::new(material_graph::encode(&authored)), Arc::default(), None,
        )));
        session.stack.layers[child].mask = Some(Arc::new(mask));
        session.stack.selected = folder;

        let preview_image = |view: &View, layer_id| {
            let id = view.graph.tagged(layer_id, Role::Mask).unwrap();
            let node = view.graph.node(id).unwrap();
            assert_eq!(node.def, "bitmap");
            let Value::Image(Some(path)) = &node.params["image"] else { panic!("Missing mask bitmap") };
            let channel = view.graph.tagged(layer_id, Role::Channel(0)).unwrap();
            assert_eq!(view.graph.node(channel).unwrap().links["mask"], id);
            view.images[path.to_str().unwrap()].clone()
        };
        let first = start(&mut session.stack, size, 0, None);
        let id = session.stack.layers[child].graph_id.unwrap();
        let pixels = preview_image(&first, id);
        assert!(first.graph.tagged(id, Role::Source(0)).is_some());
        assert_eq!(pixels.pixels().get_pixel(0, 0)[0], 64);
        assert!(pixels.pixels().get_pixel(8, 0)[0] > 220);
        let saved = material_graph::read(session.stack.layers[folder].folder_graph.as_deref().unwrap()).unwrap();
        let node = saved.node(saved.tagged(id, Role::Mask).unwrap()).unwrap();
        assert!(!node.params.contains_key("image"));
        assert!(session.stack.layers[folder].folder_images.is_empty());

        let cached = start(&mut session.stack, size, 1, Some(&first));
        assert!(Arc::ptr_eq(&pixels, &preview_image(&cached, id)));
        let mask = session.stack.layers[child].mask.as_mut().unwrap();
        Arc::make_mut(mask).edit_layers()[0].generator.as_mut().unwrap().invert = true;
        let updated = start(&mut session.stack, size, 2, Some(&cached));
        let updated = preview_image(&updated, id);
        assert_eq!(updated.pixels().get_pixel(0, 0)[0], 64);
        assert!(updated.pixels().get_pixel(8, 0)[0] < 35);
    }

    #[test]
    fn folder_graph_survives_native_preset_and_duplicates_with_independent_ids() {
        let directory = std::env::temp_dir().join(format!("bevy-folder-preset-{}", fresh_id()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("Источник.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([33, 79, 181, 255]))
            .save(&path)
            .unwrap();

        let mut session = Session::default();
        session.texture_size = 16;
        session.edit(PaintAction::Add(LayerKind::Folder));
        session.edit(PaintAction::Add(LayerKind::Fill));
        let child = session.stack.selected;
        let folder = session.stack.parents(child).next().unwrap();
        let mut mask_graph = Graph::new("Основа");
        let root = mask_graph.add("color", [0., 0.]);
        let mut mask = Mask::new(solid(16, [255; 4]), Vec::new());
        let mask_graph = mask_subgraph(&mask_graph, root, "Основа");
        mask.set_graph(Arc::new(GraphMask::new(
            Arc::new(material_graph::encode(&mask_graph)),
            Arc::default(),
            None,
        )));
        session.stack.layers[child].mask = Some(Arc::new(mask));
        session.edit(PaintAction::Select(folder));
        session.edit(PaintAction::GraphCanvas);
        let definition = material_graph::library()
            .defs
            .iter()
            .position(|def| def.id == "bitmap")
            .unwrap();
        let position = IVec2::new(-190, 88);
        session
            .edit_folder_view(PaintAction::GraphEdit(material_graph::Edit::Add(
                definition, position,
            )))
            .unwrap();
        let bitmap = session.graph_node.unwrap();
        let previous = session.active_folder_view().unwrap().graph.clone();
        let target = material_graph::ParameterTarget::Node(bitmap, 0);
        replace_bitmap(&mut session.stack, &previous, target, &path).unwrap();
        session.revision += 1;
        std::fs::remove_file(path).unwrap();
        let source_id = session.stack.layers[child].graph_id.unwrap();
        let old_payload = session.stack.layers[folder].folder_graph.clone();
        let images = session.stack.layers[folder].folder_images.clone();
        let (entry, _) = presets::save(
            &directory,
            presets::Capture::Material {
                name: "Группа".into(),
                layers: session.stack.layers[session.stack.block(folder)].to_vec(),
            },
        )
        .unwrap();
        let loaded = presets::load_material(&entry.path, 16).unwrap();
        assert_eq!(loaded[1].folder_graph, old_payload);
        assert_eq!(loaded[1].folder_images, images);

        session.edit(PaintAction::Duplicate);
        let copied = session.stack.selected;
        session.refresh_folder_view();
        let view = session.active_folder_view().unwrap();
        assert_eq!(
            view.graph.node(bitmap).unwrap().pos,
            position.as_vec2().to_array()
        );
        let copied_id = session.stack.layers[copied - 1].graph_id.unwrap();
        assert_ne!(copied_id, source_id);
        assert!(view.graph.tagged(copied_id, Role::Source(0)).is_some());
        assert!(view.graph.tagged(source_id, Role::Source(0)).is_none());

        let end = session.stack.layers.len();
        session.insert_smart_material(loaded, (end, 0));
        session.refresh_folder_view();
        let view = session.active_folder_view().unwrap();
        let imported_id = session.stack.layers[end].graph_id.unwrap();
        assert_ne!(imported_id, source_id);
        assert_ne!(imported_id, copied_id);
        assert!(view.graph.tagged(imported_id, Role::Source(0)).is_some());
        assert_eq!(view.images.as_ref(), images.as_ref());
        assert_eq!(session.stack.layers[folder].folder_graph, old_payload);
        std::fs::remove_file(entry.path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn folder_graph_smart_mask_edits_visible_inputs_without_losing_hidden_weights() {
        let mut session = Session::default();
        session.texture_size = 16;
        session.edit(PaintAction::Add(LayerKind::Folder));
        session.edit(PaintAction::Add(LayerKind::Fill));
        let child = session.stack.selected;
        let folder = session.stack.parents(child).next().unwrap();
        let mut mask = Mask::new(solid(16, [255; 4]), Vec::new());
        let mut generator = Generator::new(GeneratorKind::Drips);
        generator.set_weight(GeneratorInput::Scratches, -220);
        *mask.edit_generator() = Some(generator);
        session.stack.layers[child].mask = Some(Arc::new(mask));
        session.edit(PaintAction::Select(folder));
        session.edit(PaintAction::GraphCanvas);
        let child_id = session.stack.layers[child].graph_id.unwrap();
        let graph = &session.active_folder_view().unwrap().graph;
        let node = graph.tagged(child_id, Role::Mask).unwrap();
        assert_eq!(
            MaskSlot::read(graph.node(node).unwrap(), 1).unwrap().input,
            GeneratorInput::Scratches as usize + 1
        );
        let parameter = |name| {
            material_graph::library()
                .find("smart_mask")
                .unwrap()
                .params
                .iter()
                .position(|param| param.name == name)
                .unwrap()
        };
        session
            .edit_folder_view(PaintAction::GraphAdjust(node, parameter("input_1"), 0, 1))
            .unwrap();
        session
            .edit_folder_view(PaintAction::GraphAdjust(node, parameter("weight_1"), 0, -1))
            .unwrap();
        let current = |session: &Session| {
            session.stack.layers[child]
                .mask
                .as_ref()
                .unwrap()
                .generator
                .unwrap()
        };
        let generator = current(&session);
        assert_eq!(generator.weight(GeneratorInput::Front), -225);
        assert_eq!(generator.weight(GeneratorInput::Scratches), 0);
        assert_eq!(generator.weight(GeneratorInput::Overhang), 30);
        assert!(generator.valid());
        session.edit(PaintAction::Undo);
        assert_eq!(current(&session).weight(GeneratorInput::Front), -220);
        session.edit(PaintAction::Undo);
        assert_eq!(current(&session).weight(GeneratorInput::Scratches), -220);
    }

    #[test]
    fn rerouting_folder_mask_wire_updates_the_layer_sources() {
        use material_graph::{Edit, GraphInput, WireSource};
        let mut session = Session::default();
        session.texture_size = 16;
        session.edit(PaintAction::Add(LayerKind::Folder));
        session.edit(PaintAction::Add(LayerKind::Fill));
        let first = session.stack.selected;
        session.stack.layers[first].channels[1] = Some(solid(16, [128; 4]));
        session.edit(PaintAction::Add(LayerKind::Fill));
        let second = session.stack.selected;
        let folder = session.stack.parents(second).next().unwrap();
        session.edit(PaintAction::Select(folder));
        let view = start(&mut session.stack, 16, 0, None);
        let mut graph = view.graph;
        let first_id = session.stack.layers[first].graph_id.unwrap();
        let second_id = session.stack.layers[second].graph_id.unwrap();
        let source = graph.add("color", [0., 0.]);
        let library = material_graph::library();
        let index = library.find("layer").unwrap().inputs.iter().position(|input| input.name == "mask").unwrap();
        let old = GraphInput { node: graph.tagged(first_id, Role::Channel(0)).unwrap(), index };
        let alias = GraphInput { node: graph.tagged(first_id, Role::Channel(1)).unwrap(), index };
        let target = GraphInput { node: graph.tagged(second_id, Role::Channel(0)).unwrap(), index };
        edit_graph(&session.stack, &mut graph, Edit::Connect(source, old.node, old.index)).unwrap();
        wire_mask(&mut graph, first_id, source);
        let original = graph.clone();
        let dragged = WireSource { node: source, input: Some(old) };
        edit_graph(&session.stack, &mut graph, Edit::Reconnect(dragged, alias)).unwrap();
        assert_eq!(graph, original);
        let edit = Edit::Reconnect(dragged, target);
        assert!(can_edit(&session.stack, &graph, edit));
        edit_graph(&session.stack, &mut graph, edit).unwrap();
        assert_eq!(graph.tagged(first_id, Role::Source(0)), None);
        assert_eq!(graph.tagged(second_id, Role::Source(0)), Some(source));
        let masks = changed_masks(&session.stack, &original, &graph);
        assert!(masks.iter().any(|(index, value)| *index == first && value.is_none()));
        assert!(masks.iter().any(|(index, value)| *index == second && value.is_some()));
    }

    #[test]
    fn standalone_mask_edit_keeps_shared_branch_on_other_layer() {
        let mut session = Session::default();
        session.texture_size = 16;
        session.edit(PaintAction::Add(LayerKind::Folder));
        session.edit(PaintAction::Add(LayerKind::Fill));
        let first = session.stack.selected;
        session.edit(PaintAction::Add(LayerKind::Fill));
        let second = session.stack.selected;
        let folder = session.stack.parents(second).next().unwrap();
        let mask = |shade| {
            let mut graph = Graph::new("Маска");
            let color = graph.add("color", [0., 0.]);
            graph
                .node_mut(color)
                .unwrap()
                .params
                .insert("color".into(), Value::Color([shade; 3]));
            let graph = mask_subgraph(&graph, color, "Маска");
            let mut mask = Mask::new(solid(16, [255; 4]), Vec::new());
            mask.set_graph(Arc::new(GraphMask::new(
                Arc::new(material_graph::encode(&graph)),
                Arc::default(),
                None,
            )));
            Arc::new(mask)
        };
        session.stack.layers[first].mask = Some(mask(0.2));
        session.stack.layers[second].mask = Some(mask(0.2));
        session.edit(PaintAction::Select(folder));
        let mut view = start(&mut session.stack, 16, 0, None);
        let first_id = session.stack.layers[first].graph_id.unwrap();
        let second_id = session.stack.layers[second].graph_id.unwrap();
        let shared = view.graph.tagged(first_id, Role::Source(0)).unwrap();
        let second_source = view.graph.tagged(second_id, Role::Source(0)).unwrap();
        view.graph
            .tags
            .iter_mut()
            .find(|tag| tag.layer == second_id && tag.role == Role::Source(0))
            .unwrap()
            .node = shared;
        wire_mask(&mut view.graph, second_id, shared);
        view.graph.remove(second_source);
        store(&mut session.stack.layers[folder], &view.graph);

        session.stack.layers[first].mask = Some(mask(0.8));
        sync_standalone_mask(&mut session.stack, first);
        let view = start(&mut session.stack, 16, 1, None);
        let updated = view.graph.tagged(first_id, Role::Source(0)).unwrap();
        assert_ne!(updated, shared);
        assert_eq!(
            view.graph.node(updated).unwrap().params["color"],
            Value::Color([0.8; 3])
        );
        assert_eq!(view.graph.tagged(second_id, Role::Source(0)), Some(shared));
        assert_eq!(
            view.graph.node(shared).unwrap().params["color"],
            Value::Color([0.2; 3])
        );
        assert_ne!(
            view.graph.node(updated).unwrap().pos,
            view.graph.node(shared).unwrap().pos
        );
    }

    #[test]
    fn folder_graph_edits_child_layers_and_keeps_layout_across_rename() {
        let mut session = Session::default();
        session.texture_size = 16;
        session.edit(PaintAction::Add(LayerKind::Folder));
        session.edit(PaintAction::Add(LayerKind::Fill));
        session.edit(PaintAction::Add(LayerKind::Fill));
        let child = session.stack.selected;
        let folder = session.stack.parents(child).next().unwrap();
        session.edit(PaintAction::Select(folder));
        session.edit(PaintAction::GraphCanvas);
        let child_id = session.stack.layers[child].graph_id.unwrap();
        let view = session.active_folder_view().unwrap();
        let id = view.graph.tagged(child_id, Role::Channel(0)).unwrap();
        let below_node = view.graph.node(id).unwrap().links["below"];
        assert_ne!(view.graph.tag_of(below_node).unwrap().layer, child_id);
        let opacity = material_graph::library()
            .find("layer")
            .unwrap()
            .params
            .iter()
            .position(|param| param.name == "opacity")
            .unwrap();
        assert!(
            session
                .edit_folder_view(PaintAction::GraphAdjust(id, opacity, 0, -1))
                .unwrap()
                .is_none()
        );
        assert_eq!(session.stack.layers[child].opacity, 95.);
        assert_eq!(session.stack.layers[child - 1].opacity, 100.);
        session.edit(PaintAction::Undo);
        assert_eq!(session.stack.layers[child].opacity, 100.);
        session.edit(PaintAction::Redo);
        assert_eq!(session.stack.layers[child].opacity, 95.);

        let position = IVec2::new(333, -222);
        assert!(
            session
                .edit_folder_view(PaintAction::GraphEdit(material_graph::Edit::Move(
                    id, position,
                )))
                .unwrap()
                .is_none()
        );
        session.edit(PaintAction::Select(child));
        session.rename_layer("Металл корпуса").unwrap();
        session.edit(PaintAction::Select(folder));
        session.refresh_folder_view();
        let view = session.active_folder_view().unwrap();
        assert_eq!(
            view.graph.node(id).unwrap().pos,
            position.as_vec2().to_array()
        );
        assert_eq!(view.labels[&id], "Цвет · Металл корпуса");

        session.edit(PaintAction::Visible(child));
        session.refresh_folder_view();
        assert!(
            session
                .active_folder_view()
                .unwrap()
                .graph
                .node(id)
                .is_none()
        );
        session.edit(PaintAction::Visible(child));
        session.refresh_folder_view();
        assert_eq!(
            session
                .active_folder_view()
                .unwrap()
                .graph
                .node(id)
                .unwrap()
                .pos,
            position.as_vec2().to_array()
        );
    }

    #[test]
    fn folder_graph_variation_uses_authored_color_and_shared_seed() {
        let mut session = Session::default();
        session.texture_size = 16;
        session.edit(PaintAction::Add(LayerKind::Folder));
        session.edit(PaintAction::Add(LayerKind::Fill));
        let child = session.stack.selected;
        let folder = session.stack.parents(child).next().unwrap();
        let layer = &mut session.stack.layers[child];
        layer.channels[0] = Some(solid(16, [255, 0, 0, 255]));
        layer.edit_texture_fill(|fill| {
            fill.settings.variation.color = 0.4;
            fill.settings.variation.parts = 0.2;
            fill.sampled[0] = Some(solid(16, [0, 0, 0, 255]));
        });
        session.edit(PaintAction::Select(folder));
        session.edit(PaintAction::GraphCanvas);
        let child_id = session.stack.layers[child].graph_id.unwrap();
        let view = session.active_folder_view().unwrap();
        let color = view.graph.tagged(child_id, Role::Channel(0)).unwrap();
        let rough = view.graph.tagged(child_id, Role::Channel(1)).unwrap();
        let node = view.graph.node(color).unwrap();
        assert_eq!(node.params["color"], Value::Color([1., 0., 0.]));
        assert_eq!(node.params["variation"], Value::Float(0.4));
        assert_eq!(node.params["part_variation"], Value::Float(0.2));
        let seed = material_graph::library()
            .find("layer")
            .unwrap()
            .params
            .iter()
            .position(|param| param.name == "seed")
            .unwrap();
        session
            .edit_folder_view(PaintAction::GraphAdjust(rough, seed, 0, 1))
            .unwrap();
        let view = session.active_folder_view().unwrap();
        assert_eq!(
            view.graph.node(color).unwrap().params["seed"],
            Value::Float(5.)
        );
        assert_eq!(
            session.stack.layers[child]
                .texture_fill
                .as_ref()
                .unwrap()
                .settings
                .variation
                .seed,
            5.
        );
        session.edit(PaintAction::Undo);
        assert_eq!(
            session.stack.layers[child]
                .texture_fill
                .as_ref()
                .unwrap()
                .settings
                .variation
                .seed,
            0.
        );
    }
}
