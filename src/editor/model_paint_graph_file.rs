//! Material Painter graph files and their bitmap dependencies.
use super::*;
use material_painter_graph::Value as ParamValue;
use serde_json::{Map, Value};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use texture_fill::{Bitmap, GraphImages};
#[path = "model_paint_graph_package.rs"]
pub(in crate::editor::ui::model_viewer::paint) mod package;
pub(in crate::editor::ui::model_viewer::paint) use package::export as export_package;

const MANIFEST_LIMIT: usize = 2 * 1024 * 1024;
const IMAGE_LIMIT: usize = 32 * 1024 * 1024;
const TOTAL_LIMIT: usize = 64 * 1024 * 1024;
const RELOCATIONS: &str = "bevy_image_files";
pub(super) const EXTENSION: &str = "mpgraph";

fn paths(graph: &mut Graph) -> impl Iterator<Item = &mut PathBuf> {
    graph
        .nodes
        .iter_mut()
        .flat_map(|node| node.params.values_mut())
        .chain(
            graph
                .params
                .iter_mut()
                .filter_map(|param| param.default.as_mut()),
        )
        .filter_map(|value| match value {
            ParamValue::Image(Some(path)) => Some(path),
            _ => None,
        })
}

pub(in crate::editor::ui::model_viewer::paint) fn image_paths(
    graph: &Graph,
) -> impl Iterator<Item = &Path> {
    graph
        .nodes
        .iter()
        .flat_map(|node| node.params.values())
        .chain(
            graph
                .params
                .iter()
                .filter_map(|parameter| parameter.default.as_ref()),
        )
        .filter_map(|value| match value {
            ParamValue::Image(Some(path)) => Some(path.as_path()),
            _ => None,
        })
}

pub(in crate::editor::ui::model_viewer::paint) fn read_limited(
    path: &Path,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(limit as u64 + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("{} · {error}", path.display()))?;
    if bytes.len() > limit {
        return Err(format!("Файл слишком большой · {}", path.display()));
    }
    Ok(bytes)
}

fn inside(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let resolved = root.join(path).canonicalize().ok()?;
    (resolved.starts_with(root) && resolved.is_file()).then_some(resolved)
}

fn source_image(root: &Path, path: &Path, hints: &Value) -> Result<PathBuf, String> {
    // The relocation hint is only followed inside the chosen graph directory.
    // Material Painter itself reads the ordinary image path in the node.
    let fallback = path
        .to_str()
        .and_then(|key| hints[key].as_str())
        .and_then(|value| inside(root, Path::new(value)));
    inside(root, path)
        .or(fallback)
        .or_else(|| {
            path.is_absolute()
                .then(|| path.canonicalize().ok())
                .flatten()
                .filter(|path| path.is_file())
        })
        .or_else(|| inside(&assets(), path))
        .or_else(|| {
            path.to_str()
                .and_then(|path| path.rsplit(['/', '\\']).next())
                .filter(|name| !name.is_empty())
                .and_then(|name| inside(root, Path::new(name)))
        })
        .ok_or_else(|| format!("Текстура графа не найдена · {}", path.display()))
}

fn read_manifest(path: &Path) -> Result<Value, String> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(EXTENSION))
    {
        return Err("Выберите файл графа .mpgraph".into());
    }
    serde_json::from_slice(&read_limited(path, MANIFEST_LIMIT)?)
        .map_err(|error| format!("Не удалось прочитать граф · {error}"))
}

pub(in crate::editor::ui::model_viewer::paint) fn load_shader(
    path: &Path,
) -> Result<Graph, String> {
    let graph = if package::matches(path) {
        package::load(path)?.0
    } else {
        read(&read_manifest(path)?)?
    };
    let library = library();
    if !material_painter_graph::shading::is_shading(&graph, &library) {
        return Err("Выберите граф шейдинга Material Painter".into());
    }
    for node in &graph.nodes {
        let def = library
            .find(&node.def)
            .ok_or_else(|| format!("Не найден тип узла «{}»", node.def))?;
        if !def.shading || !node.pos.iter().all(|value| value.is_finite()) {
            return Err(format!("Недопустимый узел в графе шейдинга · {}", def.name));
        }
    }
    Ok(graph)
}

pub(in crate::editor::ui::model_viewer::paint) fn load(
    path: &Path,
) -> Result<(Graph, GraphImages), String> {
    if package::matches(path) {
        let (graph, images) = package::load(path)?;
        if !graph.nodes.is_empty() {
            graph.validate_texture_graph(&library())?;
        }
        return Ok((graph, images));
    }
    let manifest = read_manifest(path)?;
    let mut graph = read(&manifest)?;
    if !graph.nodes.is_empty() {
        graph.validate_texture_graph(&library())?;
    }
    let directory = path.parent().ok_or("У графа нет папки")?;
    let mut images = GraphImages::new();
    let mut seen: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut bytes = 0;
    let mut texels = 0;
    for value in paths(&mut graph) {
        let source = source_image(directory, value, &manifest[RELOCATIONS])?;
        if let Some(key) = seen.get(&source) {
            *value = key.clone();
            continue;
        }
        let bitmap = Arc::new(read_bitmap(&source)?);
        check_size(&mut bytes, &mut texels, images.len() + 1, &bitmap)?;
        let key = format!("graph-images/{}.png", images.len() + 1);
        let relative = PathBuf::from(&key);
        images.insert(key, bitmap);
        seen.insert(source, relative.clone());
        *value = relative;
    }
    Ok((graph, images))
}

pub(in crate::editor::ui::model_viewer::paint) fn check_size(
    bytes: &mut usize,
    texels: &mut u64,
    count: usize,
    bitmap: &Bitmap,
) -> Result<(), String> {
    *bytes += bitmap.bytes.len();
    *texels += u64::from(bitmap.pixels().width()) * u64::from(bitmap.pixels().height());
    if *bytes > TOTAL_LIMIT || *texels > 64 * 1024 * 1024 || count > 64 {
        return Err("Текстуры графа слишком большие".into());
    }
    Ok(())
}

pub(in crate::editor::ui::model_viewer::paint) fn read_bitmap(path: &Path) -> Result<Bitmap, String> {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    Bitmap::decode(name, &read_limited(path, IMAGE_LIMIT)?)
        .map_err(|error| format!("{} · {error}", path.display()))
}

fn find_bitmap(images: &GraphImages, path: &Path) -> Result<Arc<Bitmap>, String> {
    if let Some(image) = path.to_str().and_then(|key| images.get(key)) {
        return Ok(image.clone());
    }
    let path = inside(&assets(), path)
        .ok_or_else(|| format!("Изображение графа отсутствует · {}", path.display()))?;
    read_bitmap(&path).map(Arc::new)
}

pub(in crate::editor::ui::model_viewer::paint) fn bind_images(
    graph: &mut Graph,
    images: &GraphImages,
    runtime: &mut Runtime,
    device: &RenderDevice,
    queue: &RenderQueue,
) -> Result<GraphImages, String> {
    let mut seen = GraphImages::new();
    let mut bytes = 0;
    let mut texels = 0;
    for path in paths(graph) {
        let key = path.to_str().ok_or("Некорректный путь к текстуре графа")?;
        if seen.contains_key(key) {
            continue;
        }
        let bitmap = find_bitmap(images, path)?;
        check_size(&mut bytes, &mut texels, seen.len() + 1, &bitmap)?;
        runtime.bind_image(
            device.wgpu_device(),
            queue,
            path.clone(),
            bitmap.pixels().clone(),
        );
        seen.insert(key.to_owned(), bitmap);
    }
    Ok(seen)
}

pub(in crate::editor::ui::model_viewer::paint) fn export(
    directory: &Path,
    data: &Value,
    images: &GraphImages,
) -> Result<PathBuf, String> {
    let mut graph = read(data)?;
    let mut written = Vec::new();
    let result = (|| {
        let root = directory
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let mut seen = HashMap::<PathBuf, PathBuf>::new();
        let mut hints = Map::new();
        let mut total = 0;
        let mut texels = 0;
        let graph_name = graph.name.clone();
        for path in paths(&mut graph) {
            if let Some(exported) = seen.get(path) {
                *path = exported.clone();
                continue;
            }
            let bitmap = find_bitmap(images, path)?;
            check_size(&mut total, &mut texels, written.len() + 1, &bitmap)?;
            let png = if matches!(
                image::guess_format(&bitmap.bytes),
                Ok(image::ImageFormat::Png)
            ) {
                std::borrow::Cow::Borrowed(bitmap.bytes.as_slice())
            } else {
                std::borrow::Cow::Owned(texture_fill::encode_png(bitmap.pixels())?)
            };
            let name = format!("{} {}", graph_name, written.len() + 1);
            let output = presets::publish_named(&root, "Текстура", &name, "png", |file| {
                file.write_all(&png).map_err(|error| error.to_string())
            })?;
            let local = output.file_name().unwrap().to_string_lossy().into_owned();
            hints.insert(output.to_string_lossy().into_owned(), Value::String(local));
            seen.insert(path.clone(), output.clone());
            *path = output.clone();
            written.push(output);
        }
        let mut manifest = encode(&graph);
        if !hints.is_empty() {
            manifest[RELOCATIONS] = Value::Object(hints);
        }
        let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
        if bytes.len() > MANIFEST_LIMIT {
            return Err("Граф и исходники WGSL превышают лимит .mpgraph".into());
        }
        presets::publish_named(&root, "Граф", &graph.name, EXTENSION, |file| {
            file.write_all(&bytes).map_err(|error| error.to_string())
        })
    })();
    if result.is_err() {
        for path in written {
            let _ = fs::remove_file(path);
        }
    }
    result
}

#[cfg(test)]
pub(in crate::editor::ui::model_viewer::paint) mod tests {
    use super::*;

    pub(in crate::editor::ui::model_viewer::paint) const PIXEL: [u8; 4] = [197, 83, 36, 255];

    pub(in crate::editor::ui::model_viewer::paint) fn fixture(
        root: &Path,
        image_directory: &Path,
    ) -> (PathBuf, Graph) {
        fs::create_dir_all(root).unwrap();
        fs::create_dir_all(image_directory).unwrap();
        let texture = image_directory.join("Ткань.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba(PIXEL))
            .save(&texture)
            .unwrap();
        let bitmap = ParamValue::Image(Some(texture));
        let mut graph = Graph::new("Ткань");
        graph.res = 64;
        graph.match_set = true;
        let node = graph.add("bitmap", [12.0, 32.0]);
        graph
            .node_mut(node)
            .unwrap()
            .params
            .insert("image".into(), bitmap.clone());
        let output = graph.add("output", [340.0, 72.0]);
        assert!(graph.connect(node, output, "input"));
        graph.params.push(material_painter_graph::GraphParam {
            name: "Текстура ткани".into(),
            group: "Исходные текстуры".into(),
            binds: vec![(node, "image".into())],
            default: Some(bitmap),
        });
        let path = root.join("cloth.mpgraph");
        material_painter_graph::save_file(&path, &graph, &library()).unwrap();
        (path, graph)
    }

    #[test]
    fn mpgraph_round_trip_keeps_bitmap_links_parameters_and_layout() {
        let directory = std::env::temp_dir().join(format!(
            "bevy-mpgraph-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        let (source, mut expected) =
            fixture(&directory.join("source"), &directory.join("textures"));
        assert!(load_shader(&source).unwrap_err().contains("граф шейдинга"));
        let (graph, images) = load(&source).unwrap();
        assert_eq!(
            images.len(),
            1,
            "node and public default share the same bitmap"
        );
        for path in paths(&mut expected) {
            *path = "graph-images/1.png".into();
        }
        assert_eq!(graph, expected);
        assert_eq!(
            images.values().next().unwrap().pixels().get_pixel(0, 0).0,
            PIXEL
        );

        let export_dir = directory.join("export");
        fs::create_dir(&export_dir).unwrap();
        let manifest = graph.to_json(&library());
        let saved = export(&export_dir, &manifest, &images).unwrap();
        let independent = export(&export_dir, &manifest, &images).unwrap();
        assert_ne!(independent, saved);
        let mut external = material_painter_graph::load_file(&saved, &library()).unwrap();
        let paths = paths(&mut external)
            .map(|path| path.clone())
            .collect::<Vec<_>>();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], paths[1]);
        assert!(paths[0].is_absolute());
        assert_eq!(
            image::open(&paths[0]).unwrap().to_rgba8().get_pixel(0, 0).0,
            PIXEL
        );

        let moved_dir = directory.join("moved");
        fs::rename(&export_dir, &moved_dir).unwrap();
        let (reopened, restored) = load(&moved_dir.join(saved.file_name().unwrap())).unwrap();
        assert_eq!(reopened, graph);
        assert_eq!(
            restored.values().next().unwrap().bytes,
            images.values().next().unwrap().bytes
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
