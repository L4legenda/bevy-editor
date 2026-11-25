//! Material Painter's v1 smart-material manifest. Its images stay in the archive.
use super::super::material_graph;
use super::*;
use crate::editor::model_paint_data::mask::{FilterKind, MaskBlend};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use texture_fill::{OpacitySource, Projection, Settings, Variation};

pub(super) const EXTENSION: &str = "mpsmart";

#[derive(Deserialize)]
struct SmartMaterial {
    format: String,
    version: u64,
    #[serde(default = "material_name")]
    name: String,
    layers_bottom_to_top: Vec<ForeignLayer>,
    #[serde(default)]
    graphs: Vec<ForeignGraph>,
}

#[derive(Deserialize)]
struct ForeignGraph {
    id: u64,
    graph: Value,
    #[serde(default)]
    bevy_graph_images: BTreeMap<String, ImageRef>,
    bevy_mask_output: Option<ImageRef>,
}

struct ArchiveGraph {
    mask: MaskGraphPreset,
    outputs: [Option<u64>; CHANNEL_COUNT],
}

#[derive(Deserialize)]
struct ForeignLayer {
    id: u64,
    parent: Option<u64>,
    #[serde(default = "material_name")]
    name: String,
    #[serde(default = "true_value")]
    visible: bool,
    #[serde(default = "one")]
    opacity: f32,
    #[serde(default)]
    blend: String,
    content: Value,
    mask: Option<Value>,
    #[serde(default)]
    images: Vec<Option<ForeignImage>>,
}

#[derive(Deserialize)]
struct ForeignImage {
    file: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    source: String,
}

fn material_name() -> String {
    "Умный материал".into()
}
fn true_value() -> bool {
    true
}
fn one() -> f32 {
    1.
}

fn graph_image_file(file: &str) -> bool {
    file.starts_with("images/")
        && file
            .split('/')
            .all(|part| !matches!(part, "" | "." | "..") && !part.contains(['\\', ':', '\0']))
}

fn lookup_graph<'a>(
    graphs: &'a HashMap<u64, ArchiveGraph>,
    value: &Value,
    layer: &str,
) -> Result<Option<&'a ArchiveGraph>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let graph = value
        .as_u64()
        .and_then(|id| graphs.get(&id))
        .ok_or_else(|| {
            format!("Слой «{layer}» ссылается на отсутствующий граф Material Painter")
        })?;
    Ok(Some(graph))
}

pub(super) fn decode(archive: &mut Archive) -> Result<(Manifest, usize), String> {
    let bytes = read(archive, "smart.json", MAX_MANIFEST)?;
    decode_bytes(archive, &bytes)
}

pub(super) fn decode_bytes(
    archive: &mut Archive<impl Read + Seek>,
    bytes: &[u8],
) -> Result<(Manifest, usize), String> {
    let source: SmartMaterial =
        serde_json::from_slice(bytes).map_err(|error| format!("smart.json · {error}"))?;
    if source.format != "material-painter-smart-material" || source.version != 1 {
        return Err("Ожидается пресет Material Painter версии 1".into());
    }
    if source.layers_bottom_to_top.is_empty() || source.layers_bottom_to_top.len() > MAX_LAYERS {
        return Err("Некорректное число слоёв в пресете Material Painter".into());
    }
    let ids: HashSet<_> = source
        .layers_bottom_to_top
        .iter()
        .map(|layer| layer.id)
        .collect();
    if ids.len() != source.layers_bottom_to_top.len() {
        return Err("В пресете повторяются идентификаторы слоёв".into());
    }
    let mut ancestors = Vec::new();
    let mut depths = HashMap::new();
    let mut layers = Vec::new();
    let mut largest_images = HashSet::new();
    if source.graphs.len() > 2 * MAX_LAYERS {
        return Err("Слишком много графов в пресете Material Painter".into());
    }
    let mut graphs = HashMap::new();
    for mut entry in source.graphs {
        let graph = material_graph::read(&entry.graph)?;
        let library = material_graph::library();
        let library = library.as_ref();
        if !graph.nodes.is_empty() {
            graph.validate_texture_graph(library)?;
        }
        for path in material_graph::file::image_paths(&graph).filter_map(Path::to_str) {
            if graph_image_file(path) && archive.index_for_name(path).is_some() {
                let name = path.rsplit('/').next().unwrap_or(path);
                entry
                    .bevy_graph_images
                    .entry(path.to_owned())
                    .or_insert_with(|| ImageRef {
                        file: path.to_owned(),
                        name: name.to_owned(),
                    });
            }
        }
        for image in entry
            .bevy_graph_images
            .values()
            .chain(entry.bevy_mask_output.iter())
        {
            if !graph_image_file(&image.file) {
                return Err("Некорректная ссылка на изображение графа".into());
            }
            largest_images.insert(image.file.clone());
        }
        let outputs = material_painter_graph::channel_outputs(&graph.outputs(library));
        if graphs
            .insert(
                entry.id,
                ArchiveGraph {
                    mask: MaskGraphPreset {
                        payload: Arc::new(entry.graph),
                        images: entry.bevy_graph_images,
                        output: entry.bevy_mask_output,
                    },
                    outputs,
                },
            )
            .is_some()
        {
            return Err("В пресете повторяются идентификаторы графов".into());
        }
    }
    let mut omitted = 0;
    for layer in source.layers_bottom_to_top.into_iter().rev() {
        let parent = layer.parent.filter(|id| ids.contains(id));
        let depth = match parent {
            Some(parent) => {
                *depths
                    .get(&parent)
                    .ok_or("Повреждённый порядок родительских групп")?
                    + 1
            }
            None => 0,
        };
        if depth > MAX_LAYER_DEPTH
            || depth > ancestors.len()
            || (depth > 0 && parent != Some(ancestors[depth - 1]))
        {
            return Err("Повреждённая вложенность пресета Material Painter".into());
        }
        ancestors.truncate(depth);
        let kind = match layer.content["kind"].as_str() {
            Some("folder") => {
                ancestors.push(layer.id);
                depths.insert(layer.id, depth);
                LayerKind::Folder
            }
            Some("fill") => LayerKind::Fill,
            _ => return Err(format!("Неподдерживаемый слой «{}»", layer.name)),
        };
        let fill_graph = lookup_graph(&graphs, &layer.content["fill"]["graph"], &layer.name)?;
        if layer.images.len() > CHANNEL_COUNT {
            return Err(format!("Неизвестные каналы в слое «{}»", layer.name));
        }
        let mut opacity_source = OpacitySource::Auto;
        let mut images = std::array::from_fn(|_| None);
        for (channel, image) in layer.images.into_iter().enumerate() {
            if let Some(image) = image {
                if kind != LayerKind::Fill || !graph_image_file(&image.file) {
                    return Err(format!(
                        "Некорректная ссылка на изображение в слое «{}»",
                        layer.name
                    ));
                }
                largest_images.insert(image.file.clone());
                if channel == 4 {
                    opacity_source = match image.source.as_str() {
                        "Alpha" => OpacitySource::Alpha,
                        "Brightness" => OpacitySource::Brightness,
                        "Red" => OpacitySource::Red,
                        "Green" => OpacitySource::Green,
                        "Blue" => OpacitySource::Blue,
                        _ => OpacitySource::Auto,
                    };
                }
                images[channel] = Some(ImageRef {
                    file: image.file,
                    name: image.name,
                });
            }
        }
        let (channels, fill) = if kind == LayerKind::Fill {
            read_fill(&layer.content["fill"], images, opacity_source, fill_graph)?
        } else {
            (Default::default(), None)
        };
        let blend = match layer.blend.as_str() {
            "" | "Normal" => Blend::Normal,
            "Multiply" => Blend::Multiply,
            "Screen" => Blend::Screen,
            "Overlay" => Blend::Overlay,
            "Linear Dodge (Add)" if channels[3].is_some() => Blend::AddHeight,
            "Linear Dodge (Add)" => Blend::Add,
            "Subtract" => Blend::Subtract,
            "Darken" => Blend::Darken,
            "Lighten" => Blend::Lighten,
            other => return Err(format!("Неизвестный режим смешивания «{other}»")),
        };
        let mask = layer
            .mask
            .as_ref()
            .map(|mask| {
                let graph = lookup_graph(&graphs, &mask["graph"], &layer.name)?;
                read_mask(mask, &mut omitted, graph.map(|graph| graph.mask.clone()), &[])
            })
            .transpose()?;
        layers.push(LayerPreset {
            name: layer.name,
            graph_id: None,
            folder_graph: None,
            kind,
            visible: layer.visible,
            collapsed: kind == LayerKind::Folder && layer.content["open"].as_bool() == Some(false),
            opacity: layer.opacity * 100.,
            blend,
            depth,
            channels,
            mask,
            fill,
        });
    }
    if largest_images.len() > 64 {
        return Err("Слишком много изображений в пресете Material Painter".into());
    }
    let mut total = 0u64;
    for image in largest_images {
        let file = archive
            .by_name(&image)
            .map_err(|error| format!("{image} · {error}"))?;
        if file.is_dir() || file.size() > MAX_IMAGE {
            return Err(format!("Некорректный размер изображения «{image}»"));
        }
        total += file.size();
        if total > 192 * 1024 * 1024 {
            return Err("Слишком большой объём изображений в пресете Material Painter".into());
        }
    }
    layers.reverse();
    let content = Content::Material(layers);
    if !valid(&content) {
        return Err("Параметры пресета Material Painter вне поддерживаемого диапазона".into());
    }
    Ok((
        Manifest {
            version: 1,
            name: source.name,
            content,
        },
        omitted,
    ))
}

type Channels = [Option<[u8; 4]>; CHANNEL_COUNT];

fn read_fill(
    value: &Value,
    mut images: [Option<ImageRef>; CHANNEL_COUNT],
    opacity_source: OpacitySource,
    graph: Option<&ArchiveGraph>,
) -> Result<(Channels, Option<FillPreset>), String> {
    let enabled: [bool; CHANNEL_COUNT] = value["enabled"]
        .as_array()
        .filter(|enabled| enabled.len() >= 4)
        .map_or(
            [true, true, true, false, false, false, false, false],
            |enabled| {
                std::array::from_fn(|i| {
                    enabled
                        .get(i)
                        .is_some_and(|flag| flag.as_bool().unwrap_or(true))
                })
            },
        );
    for (image, enabled) in images.iter_mut().zip(enabled) {
        if !enabled {
            *image = None;
        }
    }
    let height = number(value, "height", 0.);
    let opacity = number(value, "opacity", 1.);
    let specular = number(value, "specular", 0.5);
    let emission = vector(value, "emission", [0.; 3]);
    let projection = match value["projection"]
        .as_str()
        .unwrap_or("Uv")
        .to_ascii_lowercase()
        .as_str()
    {
        "uv" => Projection::Uv,
        "triplanar" => Projection::Triplanar,
        "planarx" | "planar_x" => Projection::PlanarX,
        "planary" | "planar_y" => Projection::PlanarY,
        "planarz" | "planar_z" => Projection::PlanarZ,
        "spherical" => Projection::Spherical,
        "cylindrical" => Projection::Cylindrical,
        other => return Err(format!("Неизвестная проекция «{other}»")),
    };
    let rotation = number(value, "rotation", 0.);
    let offset = vector(value, "offset", [0.; 2]).map(|offset| {
        if (-1.0..=1.0).contains(&offset) {
            offset
        } else {
            offset % 1.
        }
    });
    let has_image =
        |i: usize| images[i].is_some() || graph.is_some_and(|graph| graph.outputs[i].is_some());
    let settings = Settings {
        projection,
        tiling: number(value, "tiling", 1.),
        rotation: if (-180.0..=180.0).contains(&rotation) {
            rotation
        } else {
            (rotation + 180.).rem_euclid(360.) - 180.
        },
        offset,
        sharpness: number(value, "triplanar_sharpness", 4.),
        normal_strength: number(value, "normal_strength", 1.),
        height_strength: if has_image(3) { 2. * height } else { 1. },
        opacity_source: if graph.is_some() {
            OpacitySource::Red
        } else {
            opacity_source
        },
        variation_channels: [enabled[0], enabled[1], enabled[3]],
        image_alpha_coverage: false,
        graph_channels: graph.map_or([true; CHANNEL_COUNT], |_| enabled),
        opacity_strength: if has_image(4) { opacity } else { 1. },
        emission_tint: if has_image(5) { emission } else { [1.; 3] },
        specular_strength: if has_image(7) { 2. * specular } else { 1. },
        variation: Variation {
            color: if enabled[0] {
                number(value, "color_variation", 0.)
            } else {
                0.
            },
            roughness: if enabled[1] {
                number(value, "roughness_variation", 0.)
            } else {
                0.
            },
            height: if enabled[3] {
                number(value, "height_variation", 0.)
            } else {
                0.
            },
            scale: number(value, "variation_scale", 4.),
            seed: number(value, "seed", 0.),
            parts: if enabled[0] || enabled[1] {
                number(value, "part_variation", 0.)
            } else {
                0.
            },
        },
    };
    if !settings.valid() {
        return Err("Параметры заливки Material Painter вне поддерживаемого диапазона".into());
    }
    let values = [
        vector(value, "color", [0.8; 3]),
        [number(value, "roughness", 0.5); 3],
        [number(value, "metallic", 0.); 3],
        [0.5 + height * 0.5; 3],
        [opacity; 3],
        emission,
        [0.5, 0.5, 1.],
        [specular; 3],
    ];
    let channels = std::array::from_fn(|index| {
        enabled[index].then(|| {
            let [r, g, b] = values[index].map(pack);
            [r, g, b, 255]
        })
    });
    Ok((
        channels,
        (graph.is_some() || settings != Settings::default() || images.iter().any(Option::is_some))
            .then(|| FillPreset {
                settings,
                images,
                graph: graph.map(|graph| graph.mask.payload.clone()),
                graph_images: graph.map_or_else(BTreeMap::new, |graph| graph.mask.images.clone()),
            }),
    ))
}

pub(in crate::editor) struct ProjectGraph {
    pub payload: Arc<Value>,
    pub images: Arc<texture_fill::GraphImages>,
    outputs: [Option<u64>; CHANNEL_COUNT],
    incomplete: bool,
}

pub(in crate::editor) fn project_graph(
    value: &Value,
    directory: &Path,
    id: u64,
    reserve: impl FnMut(usize) -> Result<(), String>,
    warnings: &mut Vec<String>,
) -> Result<ProjectGraph, String> {
    let mut graph = material_graph::read(value)?;
    let library = material_graph::library();
    graph.validate_texture_graph(&library)?;
    let outputs = material_painter_graph::channel_outputs(&graph.outputs(&library));
    let warning_count = warnings.len();
    let (payload, images) = if material_graph::file::image_paths(&graph).next().is_some() {
        let images = material_graph::file::embed_project_images(
            &mut graph, directory, id, reserve, warnings,
        );
        (material_graph::encode(&graph), images)
    } else {
        (value.clone(), texture_fill::GraphImages::new())
    };
    Ok(ProjectGraph {
        payload: Arc::new(payload),
        images: Arc::new(images),
        outputs,
        incomplete: warnings.len() != warning_count,
    })
}

pub(in crate::editor) fn project_fill(
    layer: &mut Layer,
    value: &Value,
    images: [Option<Arc<texture_fill::Bitmap>>; CHANNEL_COUNT],
    opacity_source: OpacitySource,
    graph: Option<&ProjectGraph>,
    mut plane: impl FnMut([u8; 4]) -> Result<Arc<Vec<u8>>, String>,
) -> Result<(), String> {
    let archive_graph = graph.map(|graph| ArchiveGraph {
        mask: project_graph_preset(graph),
        outputs: graph.outputs,
    });
    // The common fill reader uses image presence to pick the correct factors and channels.
    let references = std::array::from_fn(|channel| {
        images[channel].as_ref().map(|image| ImageRef { file: String::new(), name: image.name.clone() })
    });
    let (channels, fill) = read_fill(value, references, opacity_source, archive_graph.as_ref())?;
    for (target, source) in layer.channels.iter_mut().zip(channels) {
        *target = source.map(&mut plane).transpose()?;
    }
    if let Some(payload) = fill {
        let mut fill = texture_fill::Fill::default();
        fill.settings = payload.settings;
        fill.graph = payload.graph.map(pending_graph);
        if let Some(graph) = graph {
            fill.graph_images = graph.images.clone();
        }
        fill.images = std::array::from_fn(|i| payload.images[i].as_ref().and(images[i].clone()));
        layer.texture_fill = Some(Arc::new(fill));
    }
    Ok(())
}

fn pending_graph(mut payload: Arc<Value>) -> Arc<Value> {
    Arc::make_mut(&mut payload)[material_graph::MPP_GRAPH_PENDING] = Value::Bool(true);
    payload
}

fn project_graph_preset(graph: &ProjectGraph) -> MaskGraphPreset {
    MaskGraphPreset {
        payload: graph.payload.clone(),
        images: BTreeMap::new(),
        output: None,
    }
}

pub(in crate::editor) fn project_mask(
    value: &Value,
    pixels: Arc<Vec<u8>>,
    graph: Option<&ProjectGraph>,
    output: Option<Arc<texture_fill::Bitmap>>,
    maps: &[crate::editor::model_paint_data::custom::Recipe],
    omitted: &mut usize,
) -> Result<Arc<Mask>, String> {
    let preset = read_mask(value, omitted, graph.map(project_graph_preset), maps)?;
    if !preset.valid() {
        return Err("Параметры маски Material Painter вне поддерживаемого диапазона".into());
    }
    let mut mask = preset.with_pixels(pixels);
    if let Some(graph) = graph {
        let mut payload = graph.payload.clone();
        if output.is_none() {
            payload = pending_graph(payload);
        } else if graph.incomplete {
            Arc::make_mut(&mut payload)[material_graph::MPP_GRAPH_KEEP_CACHE] = Value::Bool(true);
        }
        Arc::make_mut(&mut mask).set_graph(Arc::new(GraphMask::new(
            payload,
            graph.images.clone(),
            output,
        )));
    }
    Ok(mask)
}

fn read_mask(
    value: &Value,
    omitted: &mut usize,
    graph: Option<MaskGraphPreset>,
    maps: &[crate::editor::model_paint_data::custom::Recipe],
) -> Result<MaskPreset, String> {
    let generator = value
        .get("generator")
        .filter(|value| !value.is_null())
        .map(|value| read_generator(value, omitted, maps))
        .transpose()?;
    let filters = value["filters"]
        .as_array()
        .into_iter()
        .flatten()
        .map(read_filter)
        .collect::<Result<Vec<_>, _>>()?;
    let rows = value["layers"].as_array().map_or(&[][..], Vec::as_slice);
    // Use the exact native base only if its portable stand-ins are intact.
    let base = serde_json::from_value::<[u8; 2]>(value["bevy_mask_base"].clone())
        .ok()
        .and_then(|base| {
            let mut prefix = mask_base_layers(base, generator.is_some() || graph.is_some());
            let start = prefix.len();
            let rows = rows.get(..start)?;
            for (expected, actual) in prefix.iter_mut().zip(rows) {
                let fill = actual["source"]["fill"].as_f64()?;
                if (fill - expected["source"]["fill"].as_f64()?).abs() > 1.0e-7 {
                    return None;
                }
                expected["source"]["fill"] = actual["source"]["fill"].clone();
            }
            (prefix == rows).then_some((base, start))
        });
    let start = base.map_or(0, |(_, start)| start);
    let layers = rows[start..]
        .iter()
        .map(|row| {
            let source = &row["source"];
            let generator = source
                .get("generator")
                .map(|generator| read_generator(generator, omitted, maps))
                .transpose()?;
            let blend = match row["blend"].as_str().unwrap_or("multiply") {
                "normal" => MaskBlend::Normal,
                "multiply" => MaskBlend::Multiply,
                "add" => MaskBlend::Add,
                "subtract" => MaskBlend::Subtract,
                "lighten" => MaskBlend::Lighten,
                "darken" => MaskBlend::Darken,
                other => return Err(format!("Неизвестный режим маски «{other}»")),
            };
            Ok(MaskLayer {
                value: percent(number(source, "fill", 1.)),
                opacity: percent(number(row, "opacity", 1.)),
                enabled: row["enabled"].as_bool().unwrap_or(true),
                blend,
                generator,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(MaskPreset {
        pixels: base.map_or([255, 0, 0, 255], |([scale, offset], _)| {
            [scale, offset, 0, 255]
        }),
        affine: true,
        generator,
        filters,
        layers,
        graph,
    })
}

fn read_filter(row: &Value) -> Result<Filter, String> {
    if let Some(saved) = row.get("bevy_filter") {
        let filter: Filter = serde_json::from_value(saved.clone())
            .map_err(|error| format!("Повреждённый фильтр маски: {error}"))?;
        return filter.valid().then_some(filter)
            .ok_or_else(|| "Недопустимые параметры фильтра маски".into());
    }
    if let Some(outline) = row.get("bevy_outline") {
        let filter: Filter = serde_json::from_value(outline.clone())
            .map_err(|error| format!("Повреждённый контур маски: {error}"))?;
        return (filter.kind == FilterKind::Outline && filter.valid())
            .then_some(filter)
            .ok_or_else(|| "Недопустимые параметры контура маски".into());
    }
    let name = row["kind"].as_str().unwrap_or("");
    let &(kind, _, units) = FILTERS
        .iter()
        .find(|(_, key, _)| *key == name)
        .ok_or_else(|| format!("Неизвестный фильтр маски «{name}»"))?;
    let mut filter = Filter::new(kind);
    filter.enabled = row["enabled"].as_bool().unwrap_or(true);
    for ((value, factor), key) in filter.values.iter_mut().zip(units).zip(["a", "b", "c"]) {
        *value = number(row, key, *value / factor) * factor;
    }
    if kind == FilterKind::Levels {
        for (value, key) in filter.output.iter_mut().zip(["bevy_output_min", "bevy_output_max"]) {
            *value = number(row, key, *value / 100.) * 100.;
        }
    }
    if kind == FilterKind::Blur {
        filter.wrap = row["bevy_wrap"].as_bool().unwrap_or(false);
    }
    if kind == FilterKind::Warp {
        filter.set_value(2, number(row, "bevy_warp_seed", 0.));
    }
    Ok(filter)
}

#[cfg(test)]
#[test]
fn mpsmart_levels_output_round_trips_and_reports_external_limit() {
    let legacy: Filter = serde_json::from_value(json!({
        "kind": "Levels", "enabled": true, "values": [0., 100., 100.]
    })).unwrap();
    assert_eq!(legacy.output, [0., 100.]);

    let mut filter = legacy;
    filter.set_value(3, 25.);
    filter.set_value(4, 75.);
    let mask = Mask::new(solid(16, [255; 4]), vec![filter]);
    let (preset, _, _) = MaskPreset::capture(&mask, &mut Vec::new());
    let mut adjusted = 0;
    let data = write_mask(&preset, LayerKind::Fill, &mut adjusted);
    assert_eq!(adjusted, 1);
    assert_eq!(read_filter(&data["filters"][0]).unwrap(), filter);
}

#[cfg(test)]
#[test]
fn mpsmart_warp_seed_uses_the_editor_extension() {
    let old = read_filter(&json!({"kind": "warp", "a": 0.15, "b": 8., "c": 11.})).unwrap();
    assert_eq!(old.value(2), 0.);
    let mut filter = old;
    filter.set_value(2, 1234.);
    let mask = Mask::new(solid(16, [255; 4]), vec![filter]);
    let (preset, _, _) = MaskPreset::capture(&mask, &mut Vec::new());
    let mut adjusted = 0;
    let data = write_mask(&preset, LayerKind::Fill, &mut adjusted);
    assert_eq!(adjusted, 1);
    assert_eq!(data["filters"][0]["c"], json!(0.));
    assert_eq!(read_filter(&data["filters"][0]).unwrap(), filter);
}

#[cfg(test)]
#[test]
fn mpsmart_mask_outline_survives_through_an_editor_extension() {
    let mut filter = Filter::new(FilterKind::Outline);
    filter.set_value(2, 1.);
    filter.set_value(3, 1.);
    filter.set_value(4, -20.);
    filter.set_value(5, 1.);
    let mask = Mask::new(solid(16, [255; 4]), vec![filter]);
    let (preset, _, _) = MaskPreset::capture(&mask, &mut Vec::new());
    let mut adjusted = 0;
    let data = write_mask(&preset, LayerKind::Fill, &mut adjusted);
    assert_eq!(adjusted, 1);
    assert_eq!(data["filters"][0]["enabled"], false);
    assert_eq!(read_filter(&data["filters"][0]).unwrap(), filter);
}

#[cfg(test)]
#[test]
fn mpsmart_native_filters_round_trip_and_report_external_limit() {
    let filters = [FilterKind::Threshold, FilterKind::HistogramScan, FilterKind::ContrastLuminosity,
        FilterKind::DirectionalBlur, FilterKind::Clamp, FilterKind::Smoothstep,
        FilterKind::Posterize, FilterKind::Pixelate]
        .map(|kind| {
            let mut filter = Filter::new(kind);
            filter.set_value(0, 43.);
            filter
        });
    let mask = Mask::new(solid(16, [255; 4]), filters.to_vec());
    let (preset, _, _) = MaskPreset::capture(&mask, &mut Vec::new());
    let mut adjusted = 0;
    let data = write_mask(&preset, LayerKind::Fill, &mut adjusted);
    assert_eq!(adjusted, filters.len());
    for (row, filter) in data["filters"].as_array().unwrap().iter().zip(filters) {
        assert_eq!(row["enabled"], false);
        assert_eq!(read_filter(row).unwrap(), filter);
    }
}

fn read_generator(
    value: &Value,
    omitted: &mut usize,
    maps: &[crate::editor::model_paint_data::custom::Recipe],
) -> Result<Generator, String> {
    let name = value["kind"].as_str().unwrap_or("");
    let &(kind, _) = GENERATORS
        .iter()
        .find(|(_, key)| *key == name)
        .ok_or_else(|| format!("Неизвестный генератор «{name}»"))?;
    let mut generator = Generator::new(kind);
    let factors = GENERATOR_UNITS;
    let names = ["level", "contrast", "scale", "noise", "seed"];
    for ((parameter, factor), name) in generator.values.iter_mut().zip(factors).zip(names) {
        *parameter = number(value, name, *parameter / factor) * factor;
    }
    generator.invert = value["invert"].as_bool().unwrap_or(false);
    generator.edge_width = (number(value, "edge_width", 0.04) * 1000.).round() as u16;
    generator.id_color = Some(vector(value, "id_color", [1., 0., 0.]).map(pack));
    generator.id_tolerance = percent(number(value, "id_tolerance", 0.15));
    let mut legacy_custom = &[][..];
    if let Some(weights) = value["weights"].as_array() {
        let mut converted = generator.weights().map(|_| 0);
        let legacy = value.get("custom").is_none() && weights.len() > 19;
        let built_in = if legacy { 19 } else { converted.len() };
        for (target, source) in converted.iter_mut().zip(weights.iter().take(built_in)) {
            *target = (source.as_f64().unwrap_or(0.) * 100.).round() as i16;
        }
        if legacy {
            legacy_custom = &weights[built_in..];
        }
        generator.weights = Some(converted);
    }
    let explicit = value["custom"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|slot| Some((slot["id"].as_u64()?, slot["weight"].as_f64().unwrap_or(0.))));
    let legacy = legacy_custom.iter().enumerate()
        .filter_map(|(i, weight)| Some((i as u64 + 1, weight.as_f64()?)))
        .filter(|&(_, weight)| weight != 0.);
    for (id, weight) in explicit.chain(legacy).filter(|&(id, _)| id != 0) {
        if maps.iter().any(|map| map.id == id)
            && !generator.custom.iter().any(|entry| entry.id == id)
            && let Some(slot) = generator.custom.iter_mut().find(|entry| entry.id == 0)
        {
            let weight = (weight * 100.).round();
            *omitted += usize::from(!(-200. ..=200.).contains(&weight));
            slot.id = id;
            slot.weight = weight.clamp(-200., 200.) as i16;
        } else {
            *omitted += 1;
        }
    }
    Ok(generator)
}

fn number(value: &Value, key: &str, default: f32) -> f32 {
    value[key].as_f64().map_or(default, |v| v as f32)
}

fn vector<const N: usize>(value: &Value, key: &str, default: [f32; N]) -> [f32; N] {
    std::array::from_fn(|i| value[key][i].as_f64().map_or(default[i], |v| v as f32))
}

fn pack(value: f32) -> u8 {
    (value.clamp(0., 1.) * 255.).round() as u8
}
fn percent(value: f32) -> u8 {
    (value.clamp(0., 1.) * 100.).round() as u8
}

const FILTERS: [(FilterKind, &str, [f32; 3]); 7] = [
    (FilterKind::Levels, "levels", [100.; 3]),
    (FilterKind::Invert, "invert", [1.; 3]),
    (FilterKind::Contrast, "contrast", [100., 100., 1.]),
    (FilterKind::Blur, "blur", [1.; 3]),
    (FilterKind::Sharpen, "sharpen", [1.; 3]),
    (FilterKind::Warp, "warp", [100., 1., 1.]),
    (FilterKind::Noise, "noise", [100., 1., 1.]),
];

use super::super::GeneratorKind;

const GENERATORS: [(GeneratorKind, &str); 14] = {
    use GeneratorKind::*;
    [
        (Clouds, "Clouds"),
        (Height, "World Gradient"),
        (EdgeWear, "Edge Wear"),
        (Dirt, "Dirt"),
        (Dust, "Dust"),
        (RainStreaks, "Rain Streaks"),
        (Drips, "Drips & Bleeding"),
        (CavityGrime, "Cavity Grime"),
        (CreaseGrime, "Crease Grime"),
        (ThinAreas, "Thin Areas"),
        (PartGradient, "Part Gradient"),
        (PartRandom, "Random Parts"),
        (IdMask, "ID Mask"),
        (CustomMap, "Custom Bake Map"),
    ]
};
const GENERATOR_UNITS: [f32; 5] = [100., 100., 1., 100., 1.];

pub(super) struct EncodedImage<'a> {
    name: Cow<'a, str>,
    pub bytes: Cow<'a, [u8]>,
    has_alpha: bool,
}

pub(super) fn prepare_images(
    bitmaps: &[Arc<texture_fill::Bitmap>],
) -> Result<Vec<EncodedImage<'_>>, String> {
    use image::ImageFormat::{Bmp, Jpeg, OpenExr, Png, Tga, Tiff};
    let mut total = 0;
    bitmaps
        .iter()
        .map(|bitmap| {
            // Material Painter's FillImage decoder uses the file signature. Encode
            // the base mip as PNG for formats its image build cannot open.
            let (name, bytes) = if matches!(
                image::guess_format(&bitmap.bytes),
                Ok(Bmp | Jpeg | OpenExr | Png | Tiff | Tga | image::ImageFormat::Hdr)
            ) {
                (
                    Cow::Borrowed(bitmap.name.as_str()),
                    Cow::Borrowed(bitmap.bytes.as_slice()),
                )
            } else {
                let name = Path::new(&bitmap.name)
                    .with_extension("png")
                    .to_string_lossy()
                    .into_owned();
                (
                    Cow::Owned(name),
                    Cow::Owned(texture_fill::encode_png(bitmap.pixels())?),
                )
            };
            total += bytes.len();
            if bytes.len() as u64 > MAX_IMAGE || total > 192 * 1024 * 1024 {
                return Err("Изображения для экспорта превышают допустимый размер".into());
            }
            Ok(EncodedImage {
                name,
                bytes,
                has_alpha: bitmap.has_alpha(),
            })
        })
        .collect()
}

pub(super) fn encode(
    manifest: &Manifest,
    bitmaps: &[EncodedImage<'_>],
) -> Result<(Value, usize), String> {
    let Content::Material(rows) = &manifest.content else {
        return Err("Выберите заливку или группу с заливками".into());
    };
    let mut adjusted = 0;
    let mut parents = Vec::new();
    let mut layers = Vec::with_capacity(rows.len());
    let mut graphs = Vec::new();
    for (index, row) in rows.iter().enumerate().rev() {
        let id = index + 1;
        parents.truncate(row.depth);
        let parent = parents.last().copied();
        let (mut content, images) = if row.kind == LayerKind::Folder {
            parents.push(id);
            (
                json!({"kind": "folder", "open": !row.collapsed}),
                [const { Value::Null }; CHANNEL_COUNT],
            )
        } else {
            write_fill(row, bitmaps, &mut adjusted)
        };
        if let Some(fill) = &row.fill
            && let Some(graph) = &fill.graph
        {
            content["fill"]["graph"] = register_graph(&mut graphs, graph, &fill.graph_images, None);
        }
        let blend = match row.blend {
            Blend::Normal => "Normal",
            Blend::Multiply => "Multiply",
            Blend::Screen => "Screen",
            Blend::Overlay => "Overlay",
            Blend::Add | Blend::AddHeight => "Linear Dodge (Add)",
            Blend::Subtract => "Subtract",
            Blend::Darken => "Darken",
            Blend::Lighten => "Lighten",
        };
        let mask = row.mask.as_ref().map(|mask| {
            let mut stored = write_mask(mask, row.kind, &mut adjusted);
            if let Some(graph) = &mask.graph {
                stored["graph"] = register_graph(
                    &mut graphs,
                    &graph.payload,
                    &graph.images,
                    graph.output.as_ref(),
                );
            }
            stored
        });
        layers.push(json!({
            "id": id, "parent": parent, "name": row.name, "visible": row.visible,
            "opacity": row.opacity / 100., "blend": blend, "content": content,
            "mask": mask, "images": images,
        }));
    }
    layers.reverse();
    Ok((
        json!({
            "format": "material-painter-smart-material", "version": 1,
            "name": manifest.name, "app_version": env!("CARGO_PKG_VERSION"),
            "layers_bottom_to_top": layers, "graphs": graphs,
        }),
        adjusted,
    ))
}

fn register_graph(
    graphs: &mut Vec<Value>,
    graph: &Value,
    images: &BTreeMap<String, ImageRef>,
    mask_output: Option<&ImageRef>,
) -> Value {
    let id = graphs.len() + 1;
    let mut record = json!({"id": id, "graph": graph});
    if !images.is_empty() {
        record["bevy_graph_images"] = json!(images);
    }
    if let Some(output) = mask_output {
        record["bevy_mask_output"] = json!(output);
    }
    graphs.push(record);
    json!(id)
}

fn write_fill(
    row: &LayerPreset,
    bitmaps: &[EncodedImage<'_>],
    adjusted: &mut usize,
) -> (Value, [Value; CHANNEL_COUNT]) {
    let settings = row
        .fill
        .as_ref()
        .map_or_else(Settings::default, |fill| fill.settings);
    let images = row.fill.as_ref().map(|fill| &fill.images);
    let image = |i: usize| images.and_then(|images| images[i].as_ref());
    let graph_outputs = row
        .fill
        .as_ref()
        .and_then(|fill| fill.graph.as_ref())
        .and_then(|payload| material_graph::read(payload).ok())
        .map(|graph| {
            material_painter_graph::channel_outputs(&graph.outputs(&material_graph::library()))
        });
    let has_image = |i: usize| {
        image(i).is_some()
            || graph_outputs
                .as_ref()
                .is_some_and(|outputs| outputs[i].is_some())
    };
    let variation = settings.variation;
    let [var_color, var_rough, var_height] = settings.variation_channels;
    let varying = |enabled, value| if enabled { value } else { 0. };
    let enabled: [bool; CHANNEL_COUNT] = std::array::from_fn(|i| {
        (graph_outputs.is_none() || settings.graph_channels[i])
            && (row.channels[i].is_some()
                || has_image(i)
                || match i {
                    0 => var_color && (variation.color != 0. || variation.parts != 0.),
                    1 => var_rough && (variation.roughness != 0. || variation.parts != 0.),
                    3 => var_height && variation.height != 0.,
                    _ => false,
                })
    });
    let part_variation = match (enabled[0] && var_color, enabled[1] && var_rough) {
        (false, false) => 0.,
        (true, false) | (false, true) if enabled[0] && enabled[1] => {
            *adjusted += usize::from(variation.parts != 0.);
            0.
        }
        _ => variation.parts,
    };
    let rgb =
        |i: usize, fallback| row.channels[i].map_or(fallback, |[r, g, b, _]| [r, g, b].map(byte));
    let scalar = |i, fallback| rgb(i, [fallback; 3])[0];
    let height = if has_image(3) {
        settings.height_strength / 2.
    } else {
        2. * scalar(3, 0.5) - 1.
    };
    let opacity = if has_image(4) {
        settings.opacity_strength
    } else {
        scalar(4, 1.)
    };
    let emission = if has_image(5) {
        settings.emission_tint
    } else {
        rgb(5, [0.; 3])
    };
    let specular = if has_image(7) {
        settings.specular_strength / 2.
    } else {
        scalar(7, 0.5)
    };
    *adjusted += row
        .channels
        .iter()
        .enumerate()
        .filter(|(i, pixel)| !has_image(*i) && pixel.is_some_and(|rgba| rgba[3] != 255))
        .count();
    if enabled[3]
        && matches!(
            row.blend,
            Blend::Add | Blend::Subtract | Blend::Multiply | Blend::Screen | Blend::Overlay
        )
    {
        *adjusted += 1;
    }
    if let Some([r, g, b, _]) = row.channels[6] {
        *adjusted += usize::from([r, g, b] != [128, 128, 255] && !has_image(6));
    }
    let images = std::array::from_fn(|i| {
        image(i).map_or(Value::Null, |image| {
            let bitmap = image
                .file
                .strip_prefix("images/")
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(|index| bitmaps.get(index));
            if settings.image_alpha_coverage
                && matches!(i, 0 | 5)
                && bitmap.is_some_and(|bitmap| bitmap.has_alpha)
            {
                *adjusted += 1;
            }
            let source = if i == 4 {
                settings.opacity_source
            } else {
                OpacitySource::Auto
            };
            let name = bitmap.map_or(image.name.as_str(), |bitmap| bitmap.name.as_ref());
            json!({"file": image.file, "name": name, "source": format!("{source:?}")})
        })
    });
    (
        json!({"kind": "fill", "fill": {
            "enabled": enabled, "color": rgb(0, [0.8; 3]), "roughness": scalar(1, 0.5),
            "metallic": scalar(2, 0.), "height": height, "opacity": opacity,
            "emission": emission, "specular": specular, "normal_strength": settings.normal_strength,
            "projection": format!("{:?}", settings.projection), "offset": settings.offset,
            "tiling": settings.tiling, "rotation": settings.rotation, "triplanar_sharpness": settings.sharpness,
            "color_variation": varying(var_color, variation.color),
            "roughness_variation": varying(var_rough, variation.roughness),
            "height_variation": varying(var_height, variation.height),
            "part_variation": part_variation, "variation_scale": variation.scale, "seed": variation.seed,
            "graph": null,
        }}),
        images,
    )
}

fn mask_base_layers([scale, offset]: [u8; 2], generated: bool) -> Vec<Value> {
    let fill = |value, blend| {
        json!({
            "source": {"fill": value}, "blend": blend, "opacity": 1., "enabled": true,
        })
    };
    let mut layers = Vec::new();
    if generated {
        if scale != 255 {
            layers.push(fill(byte(scale), "multiply"));
        }
        if offset != 0 {
            layers.push(fill(byte(offset), "add"));
        }
    } else {
        let value = scale.saturating_add(offset);
        if value != 255 {
            layers.push(fill(byte(value), "normal"));
        }
    }
    layers
}

fn write_mask(mask: &MaskPreset, kind: LayerKind, adjusted: &mut usize) -> Value {
    let [scale, offset, _, _] = mask.pixels;
    let base = [scale, if mask.affine { offset } else { 0 }];
    let mut layers = mask_base_layers(base, mask.generator.is_some() || mask.graph.is_some());
    // Material Painter's folder merge samples the base generator directly.
    if kind == LayerKind::Folder {
        *adjusted += usize::from(mask.graph.is_some())
            + layers.len()
            + mask.filters.iter().filter(|filter| filter.enabled).count()
            + mask
                .layers
                .iter()
                .filter(|layer| layer.enabled && layer.opacity != 0)
                .count();
    }
    let filters: Vec<_> = mask
        .filters
        .iter()
        .map(|filter| {
            if filter.kind == FilterKind::Outline {
                *adjusted += usize::from(filter.enabled && kind != LayerKind::Folder);
                return json!({"kind": "invert", "enabled": false, "bevy_outline": filter});
            }
            let Some((_, key, units)) = FILTERS
                .iter()
                .find(|(kind, _, _)| *kind == filter.kind)
            else {
                *adjusted += usize::from(filter.enabled && kind != LayerKind::Folder);
                return json!({"kind": "invert", "enabled": false, "bevy_filter": filter});
            };
            let [a, b, mut c] = std::array::from_fn(|i| filter.values[i] / units[i]);
            if filter.kind == FilterKind::Warp {
                c = 0.;
            }
            let mut row = json!({"kind": key, "enabled": filter.enabled, "a": a, "b": b, "c": c});
            if filter.kind == FilterKind::Levels && filter.output != [0., 100.] {
                row["bevy_output_min"] = json!(filter.output[0] / 100.);
                row["bevy_output_max"] = json!(filter.output[1] / 100.);
                *adjusted += usize::from(filter.enabled);
            }
            if filter.kind == FilterKind::Blur && filter.wrap {
                row["bevy_wrap"] = json!(true);
                *adjusted += usize::from(filter.enabled);
            }
            if filter.kind == FilterKind::Warp && filter.value(2) != 0. {
                row["bevy_warp_seed"] = json!(filter.value(2));
                *adjusted += usize::from(filter.enabled);
            }
            row
        })
        .collect();
    layers.extend(mask.layers.iter().map(|layer| {
        let source = layer.generator.map_or_else(
            || json!({"fill": f32::from(layer.value) / 100.}),
            |generator| json!({"generator": write_generator(generator, adjusted)}),
        );
        let blend = match layer.blend {
            MaskBlend::Normal => "normal",
            MaskBlend::Multiply => "multiply",
            MaskBlend::Add => "add",
            MaskBlend::Subtract => "subtract",
            MaskBlend::Lighten => "lighten",
            MaskBlend::Darken => "darken",
        };
        json!({"source": source, "blend": blend, "opacity": f32::from(layer.opacity) / 100., "enabled": layer.enabled})
    }));
    json!({"generator": mask.generator.map(|generator| write_generator(generator, adjusted)),
        "filters": filters, "layers": layers, "graph": null, "bevy_mask_base": base})
}

fn write_generator(generator: Generator, adjusted: &mut usize) -> Value {
    let (_, kind) = GENERATORS
        .iter()
        .find(|(kind, _)| *kind == generator.kind)
        .unwrap();
    let [level, contrast, scale, noise, seed] =
        std::array::from_fn(|i| generator.values[i] / GENERATOR_UNITS[i]);
    *adjusted +=
        usize::from(generator.kind == GeneratorKind::IdMask && generator.id_color.is_none());
    json!({
        "kind": kind, "weights": generator.weights().map(|weight| f32::from(weight) / 100.),
        "level": level, "contrast": contrast, "scale": scale, "noise": noise, "seed": seed,
        "invert": generator.invert, "edge_width": f32::from(generator.edge_width) / 1000.,
        "id_color": generator.id_color.map_or([1., 0., 0.], |color| color.map(byte)),
        "id_tolerance": f32::from(generator.id_tolerance) / 100., "custom": [],
    })
}

fn byte(value: u8) -> f32 {
    f32::from(value) / 255.
}
