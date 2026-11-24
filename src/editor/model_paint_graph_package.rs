//! Material Painter graph ZIP layout, including WGSL and optional Bitmap assets.
use super::*;
use crate::editor::ui::model_viewer::paint::node_library::{EMBEDDED_SOURCES, SOURCE_LIMIT};
use std::{
    collections::{BTreeMap, HashSet},
    io::Seek,
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const ZIP_EXT: &str = "zip";
const ENTRY_LIMIT: usize = 384;

pub(in crate::editor::ui::model_viewer::paint) fn safe_name(name: &str) -> String {
    let name = name
        .chars()
        .take(48)
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let name = name.trim_matches('-');
    if name.is_empty() {
        "package".into()
    } else {
        name.into()
    }
}

pub(in crate::editor::ui::model_viewer::paint) fn matches(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(ZIP_EXT))
}

pub(in crate::editor::ui::model_viewer::paint) fn relative(name: &str) -> bool {
    !name.contains(['\\', ':', '\0'])
        && name.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

fn read_entry(archive: &mut ZipArchive<File>, name: &str, limit: usize) -> Result<Vec<u8>, String> {
    if !relative(name) {
        return Err(format!("Недопустимый путь в пакете · {name}"));
    }
    let mut entry = archive
        .by_name(name)
        .map_err(|error| format!("{name} · {error}"))?;
    if entry.is_dir() || entry.size() > limit as u64 {
        return Err(format!("Недопустимый размер файла в пакете · {name}"));
    }
    let mut data = Vec::new();
    entry
        .by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|error| error.to_string())?;
    if data.len() > limit {
        return Err(format!("Файл в пакете слишком большой · {name}"));
    }
    Ok(data)
}

fn parse_entry(archive: &mut ZipArchive<File>, name: &str) -> Result<Value, String> {
    serde_json::from_slice(&read_entry(archive, name, MANIFEST_LIMIT)?)
        .map_err(|error| format!("{name} · {error}"))
}

pub(in crate::editor::ui::model_viewer::paint) struct Package {
    archive: ZipArchive<File>,
    root: String,
    entries: HashSet<String>,
    files: Vec<String>,
    pub main: String,
}

impl Package {
    pub fn open(path: &Path, kind: &str, extension: &str) -> Result<Self, String> {
        let mut archive = ZipArchive::new(File::open(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        if archive.len() > ENTRY_LIMIT {
            return Err("Слишком много файлов в ZIP-пакете".into());
        }
        let mut entries = HashSet::new();
        for index in 0..archive.len() {
            let entry = archive
                .by_index_raw(index)
                .map_err(|error| error.to_string())?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().map_err(|error| error.to_string())?;
            if !relative(&name) || !entries.insert(name.into_owned()) {
                return Err("В ZIP-пакете есть повторяющийся или недопустимый путь".into());
            }
        }
        let mut manifests = entries
            .iter()
            .filter(|path| *path == "info.json" || path.ends_with("/info.json"));
        let info_path = manifests.next().ok_or("В ZIP-пакете нет info.json")?;
        if manifests.next().is_some() {
            return Err("В ZIP-пакете несколько файлов info.json".into());
        }
        let root = info_path.strip_suffix("info.json").unwrap();
        let info = parse_entry(&mut archive, info_path)?;
        if info["kind"] != kind {
            return Err(format!(
                "Выберите ZIP-пакет Material Painter с файлом .{extension}"
            ));
        }
        let files = info["files"]
            .as_array()
            .ok_or("В пакете не указан список файлов")?;
        if files.len() > ENTRY_LIMIT {
            return Err("Слишком много файлов в описании ZIP-пакета".into());
        }
        let files: Vec<_> = files
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .filter(|path| relative(path))
                    .map(str::to_owned)
                    .ok_or("Некорректный путь в описании пакета")
            })
            .collect::<Result<_, _>>()?;
        let mut main_files = files.iter().filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
        });
        let main = main_files
            .next()
            .ok_or_else(|| format!("В ZIP-пакете нет .{extension}"))?;
        if main_files.next().is_some() {
            return Err(format!("В ZIP-пакете несколько файлов .{extension}"));
        }
        let (root, main) = (root.to_owned(), main.clone());
        Ok(Self {
            archive,
            root,
            entries,
            files,
            main,
        })
    }

    pub fn read_main(&mut self, limit: usize) -> Result<Vec<u8>, String> {
        read_entry(
            &mut self.archive,
            &format!("{}{}", self.root, self.main),
            limit,
        )
    }

    pub fn sources(&mut self) -> Result<Map<String, Value>, String> {
        let mut sources = Map::new();
        let mut total_source = 0;
        for name in self.files.iter().filter(|name| name.ends_with(".wgsl")) {
            let data = read_entry(
                &mut self.archive,
                &format!("{}{name}", self.root),
                SOURCE_LIMIT,
            )?;
            total_source += data.len();
            if total_source > MANIFEST_LIMIT {
                return Err("Исходники WGSL в пакете слишком большие".into());
            }
            let code = String::from_utf8(data).map_err(|_| "Ожидался исходник WGSL в UTF-8")?;
            let stem = Path::new(name)
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or("Некорректное имя узла в пакете")?;
            let key = format!("user/{stem}");
            if sources
                .get(&key)
                .is_some_and(|prior| prior.as_str() != Some(&code))
            {
                return Err(format!("В пакете разные версии узла · {key}"));
            }
            sources.insert(key, Value::String(code));
        }
        Ok(sources)
    }
}

pub(in crate::editor::ui::model_viewer::paint) fn attach_sources(
    payload: &mut Value,
    sources: &Map<String, Value>,
) -> Result<(), String> {
    let used: Vec<_> = payload["nodes"]
        .as_array()
        .ok_or("В графе нет списка узлов")?
        .iter()
        .filter_map(|node| node["def"].as_str().filter(|id| sources.contains_key(*id)))
        .map(str::to_owned)
        .collect();
    if used.is_empty() {
        return Ok(());
    }
    let embedded = payload
        .as_object_mut()
        .unwrap()
        .entry(EMBEDDED_SOURCES)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or("Некорректные исходники WGSL")?;
    for id in used {
        let source = &sources[&id];
        if embedded.get(&id).is_some_and(|previous| previous != source) {
            return Err(format!("В пакете разные версии узла · {id}"));
        }
        embedded.insert(id, source.clone());
    }
    Ok(())
}

pub(super) fn load(path: &Path) -> Result<(Graph, GraphImages), String> {
    let mut package = Package::open(path, "graph", EXTENSION)?;
    let mut payload = serde_json::from_slice(&package.read_main(MANIFEST_LIMIT)?)
        .map_err(|error| error.to_string())?;
    attach_sources(&mut payload, &package.sources()?)?;
    let mut graph = read(&payload)?;
    let mut images = GraphImages::new();
    let mut seen = HashMap::<PathBuf, PathBuf>::new();
    let mut bytes = 0;
    let mut texels = 0;
    for path in paths(&mut graph) {
        if let Some(key) = seen.get(path) {
            *path = key.clone();
            continue;
        }
        let name = path
            .to_str()
            .filter(|name| relative(name))
            .ok_or("Bitmap в ZIP-пакете должен ссылаться на файл внутри пакета")?;
        let archive_name = format!("{}{name}", package.root);
        let bitmap = if package.entries.contains(&archive_name) {
            Bitmap::decode(
                name.into(),
                &read_entry(&mut package.archive, &archive_name, IMAGE_LIMIT)?,
            )?
        } else {
            let asset =
                inside(&assets(), path).ok_or_else(|| format!("В пакете нет Bitmap · {name}"))?;
            read_bitmap(&asset)?
        };
        check_size(&mut bytes, &mut texels, images.len() + 1, &bitmap)?;
        let key = format!("graph-images/{}.png", images.len() + 1);
        seen.insert(path.clone(), PathBuf::from(&key));
        *path = PathBuf::from(&key);
        images.insert(key, Arc::new(bitmap));
    }
    Ok((graph, images))
}

pub(in crate::editor::ui::model_viewer::paint) fn split_sources(
    payload: &mut Value,
    sources: &mut BTreeMap<String, String>,
) {
    let mut aliases = HashMap::new();
    if let Some(embedded) = payload.as_object_mut().unwrap().remove(EMBEDDED_SOURCES) {
        for (id, value) in embedded.as_object().unwrap() {
            let code = value.as_str().unwrap();
            let stem = format!("bevy-{}", blake3::hash(code.as_bytes()).to_hex());
            aliases.insert(id.clone(), format!("user/{stem}"));
            sources.insert(format!("nodes/{stem}.wgsl"), code.to_owned());
        }
    }
    for node in payload["nodes"].as_array_mut().unwrap() {
        if let Some(id) = node["def"].as_str().and_then(|id| aliases.get(id)) {
            node["def"] = Value::String(id.clone());
        }
    }
}

pub(in crate::editor::ui::model_viewer::paint) fn check_sources(
    sources: &BTreeMap<String, String>,
) -> Result<(), String> {
    if sources.len() + 4 > ENTRY_LIMIT {
        return Err("В ZIP-пакете слишком много узлов WGSL".into());
    }
    let mut total = 0;
    for code in sources.values() {
        total += code.len();
        if code.len() > SOURCE_LIMIT || total > MANIFEST_LIMIT {
            return Err("Исходники WGSL в пакете слишком большие".into());
        }
    }
    Ok(())
}

pub(in crate::editor::ui::model_viewer::paint) fn add_entry(
    archive: &mut ZipWriter<impl Write + Seek>,
    name: &str,
    plain: bool,
    bytes: &[u8],
) -> Result<(), String> {
    let method = if plain {
        zip::CompressionMethod::Stored
    } else {
        zip::CompressionMethod::Deflated
    };
    archive
        .start_file(
            name,
            SimpleFileOptions::default().compression_method(method),
        )
        .map_err(|error| error.to_string())?;
    archive.write_all(bytes).map_err(|error| error.to_string())
}

pub(in crate::editor::ui::model_viewer::paint) fn export(
    directory: &Path,
    payload: &Value,
    images: &GraphImages,
) -> Result<PathBuf, String> {
    let mut graph = read(payload)?;
    let library = library();
    if let Some(node) = graph
        .nodes
        .iter()
        .find(|node| library.find(&node.def).is_none())
    {
        return Err(format!("Не найден исходник узла · {}", node.def));
    }
    let mut bitmaps = BTreeMap::<String, Arc<Bitmap>>::new();
    let mut relocated = HashMap::<PathBuf, PathBuf>::new();
    let mut bytes = 0;
    let mut texels = 0;
    for path in paths(&mut graph) {
        if let Some(relative) = relocated.get(path) {
            *path = relative.clone();
            continue;
        }
        let bitmap = find_bitmap(images, path)?;
        check_size(&mut bytes, &mut texels, bitmaps.len() + 1, &bitmap)?;
        let name = format!("images/{}.png", bitmaps.len() + 1);
        let relative = PathBuf::from(&name);
        relocated.insert(path.clone(), relative.clone());
        *path = relative;
        bitmaps.insert(name, bitmap);
    }
    let mut payload = encode(&graph);
    let mut sources = BTreeMap::new();
    split_sources(&mut payload, &mut sources);
    check_sources(&sources)?;
    let id = safe_name(&graph.name);
    let main = format!("{id}.mpgraph");
    let files: Vec<_> = std::iter::once(main.as_str())
        .chain(sources.keys().map(String::as_str))
        .chain(bitmaps.keys().map(String::as_str))
        .collect();
    let info = serde_json::json!({
        "id": id, "name": graph.name, "kind": "graph", "path": format!("items/{id}"),
        "files": files, "author": "", "description": "", "tags": [], "thumb": null,
    });
    let graph_bytes = serde_json::to_vec_pretty(&payload).map_err(|error| error.to_string())?;
    if graph_bytes.len() > MANIFEST_LIMIT {
        return Err("Граф слишком большой для ZIP-пакета".into());
    }
    presets::publish_named(directory, "Граф", &graph.name, ZIP_EXT, |file| {
        let mut archive = ZipWriter::new(file);
        let mut write = |name: &str, data: &[u8]| -> Result<(), String> {
            add_entry(&mut archive, &format!("{id}/{name}"), false, data)
        };
        write(
            "info.json",
            &serde_json::to_vec_pretty(&info).map_err(|error| error.to_string())?,
        )?;
        write(&main, &graph_bytes)?;
        for (name, source) in &sources {
            write(name, source.as_bytes())?;
        }
        let mut written = 0;
        for (name, bitmap) in bitmaps {
            let png = if matches!(
                image::guess_format(&bitmap.bytes),
                Ok(image::ImageFormat::Png)
            ) {
                std::borrow::Cow::Borrowed(bitmap.bytes.as_slice())
            } else {
                std::borrow::Cow::Owned(texture_fill::encode_png(bitmap.pixels())?)
            };
            written += png.len();
            if png.len() > IMAGE_LIMIT || written > TOTAL_LIMIT {
                return Err("Изображения PNG слишком большие для ZIP-пакета".into());
            }
            write(&name, &png)?;
        }
        archive.finish().map_err(|error| error.to_string())?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::ui::model_viewer::paint::node_library;

    const WGSL: &str = "// @name Custom filter\n// @output color\n// @input source: color = 1\n// @param gain: float(0, 1) = 0.8\nfn node(uv: vec2<f32>) -> vec4<f32> { return in_source(uv) * params.gain; }\n";

    fn fixture(entries: &[(&str, &[u8])], path: &Path) {
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        for (name, contents) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn zip_graph_reads_material_painter_layout_and_round_trips_source_and_bitmap() {
        let directory = std::env::temp_dir().join(format!(
            "bevy-graph-zip-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&directory).unwrap();
        let info = serde_json::json!({
            "id": "shared-graph", "name": "Shared graph", "kind": "graph",
            "path": "items/shared-graph",
            "files": ["shared-graph.mpgraph", "nodes/package-filter.wgsl", "images/test.png"],
        });
        let graph = serde_json::json!({
            "name": "Shared graph", "res": 64, "next_id": 4,
            "nodes": [
                {"id": 1, "def": "bitmap", "pos": [0, 0], "params": {"image": "images/test.png"}},
                {"id": 2, "def": "user/package-filter", "pos": [200, 0], "links": {"source": 1}, "params": {"gain": 0.65}},
                {"id": 3, "def": "output", "pos": [400, 0], "links": {"input": 2}},
            ],
            "params": [{"name": "Gain", "group": "Color", "binds": [[2, "gain"]], "default": 0.8}],
        });
        let rgba = [14, 97, 214, 255];
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba(rgba));
        let png = texture_fill::encode_png(&image).unwrap();
        let input = directory.join("material-painter.zip");
        fixture(
            &[
                (
                    "shared-graph/info.json",
                    &serde_json::to_vec(&info).unwrap(),
                ),
                (
                    "shared-graph/shared-graph.mpgraph",
                    &serde_json::to_vec(&graph).unwrap(),
                ),
                ("shared-graph/nodes/package-filter.wgsl", WGSL.as_bytes()),
                ("shared-graph/images/test.png", &png),
            ],
            &input,
        );

        let (imported, images) = super::super::load(&input).unwrap();
        let node = imported.node(2).unwrap();
        assert!(node_library::is_embedded(&node.def));
        assert_eq!(library().find(&node.def).unwrap().source, WGSL);
        assert_eq!(node.links["source"], 1);
        assert_eq!(
            imported.param_value(0, &library()).unwrap().1,
            ParamValue::Float(0.65)
        );
        assert_eq!(images.len(), 1);
        assert_eq!(
            images.values().next().unwrap().pixels().get_pixel(0, 0).0,
            rgba
        );

        let output = export(&directory, &encode(&imported), &images).unwrap();
        let mut archive = ZipArchive::new(File::open(&output).unwrap()).unwrap();
        let info = parse_entry(&mut archive, "shared-graph/info.json").unwrap();
        let native = parse_entry(&mut archive, "shared-graph/shared-graph.mpgraph").unwrap();
        let node_path = info["files"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|value| value.as_str().filter(|name| name.starts_with("nodes/")))
            .unwrap();
        let node_dir = directory.join("native-nodes");
        fs::create_dir(&node_dir).unwrap();
        fs::write(
            node_dir.join(Path::new(node_path).file_name().unwrap()),
            read_entry(
                &mut archive,
                &format!("shared-graph/{node_path}"),
                SOURCE_LIMIT,
            )
            .unwrap(),
        )
        .unwrap();
        let native_lib = material_painter_graph::Library::load(Some(node_dir));
        let native_graph = Graph::from_json(&native, &native_lib).unwrap();
        native_graph.validate_texture_graph(&native_lib).unwrap();
        assert_eq!(
            native_lib
                .find(&native_graph.node(2).unwrap().def)
                .unwrap()
                .source,
            WGSL
        );

        let (restored, restored_images) = super::super::load(&output).unwrap();
        assert_eq!(restored, imported);
        assert_eq!(
            restored_images
                .values()
                .next()
                .unwrap()
                .pixels()
                .get_pixel(0, 0)
                .0,
            rgba
        );

        let unsafe_package = directory.join("unsafe.zip");
        let bad_info = br#"{"kind":"graph","files":["../outside.mpgraph"]}"#;
        fixture(&[("bad/info.json", bad_info)], &unsafe_package);
        assert!(
            super::super::load(&unsafe_package)
                .unwrap_err()
                .contains("путь")
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
