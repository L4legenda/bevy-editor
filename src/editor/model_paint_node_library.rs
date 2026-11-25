//! User WGSL definitions shared by the paint graph editor and its workers.
use super::*;
use bevy_scene_format::{Revision, atomic_write};
use material_painter_graph::{
    Graph, Library,
    def::{self, NodeDef},
};
use serde_json::{Map, Value};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, RwLock},
};

pub(super) const SOURCE_LIMIT: usize = 128 * 1024;
pub(super) const EMBEDDED_SOURCES: &str = "bevy_wgsl_nodes";
const PORTABLE_PREFIX: &str = "graph/";
const REMOVED: &str = ".removed";

struct Catalog {
    library: Arc<Library>,
    archived: Arc<HashSet<String>>,
    revision: usize,
}

pub(super) fn is_embedded(id: &str) -> bool {
    id.starts_with(PORTABLE_PREFIX)
}

fn custom(id: &str) -> bool {
    id.starts_with("user/") || is_embedded(id)
}

pub(super) fn directory() -> Option<PathBuf> {
    presets::directory().map(|path| path.join("nodes"))
}

pub(super) fn node_id(path: &Path) -> Result<String, String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .ok_or("Имя файла узла не поддерживается")?;
    Ok(format!("user/{stem}"))
}

fn load_folder(library: &mut Library, dir: &Path) -> Vec<String> {
    let mut ids = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut paths: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("wgsl"))
            })
            .collect();
        paths.sort();
        for path in paths {
            let definition = (|| {
                let source = material_graph::file::read_limited(&path, SOURCE_LIMIT)?;
                let source = std::str::from_utf8(&source).map_err(|_| "Ожидался WGSL в UTF-8")?;
                def::parse(&node_id(&path)?, source)
            })();
            match definition {
                Ok(definition) if library.find(&definition.id).is_none() => {
                    ids.push(definition.id.clone());
                    library.defs.push(definition);
                }
                Ok(_) => {}
                Err(error) => library.errors.push((path, error)),
            }
        }
    }
    ids
}

fn load(folder: Option<PathBuf>) -> Catalog {
    let mut library = Library::load(None);
    let archived = folder.as_ref().map_or_else(HashSet::new, |dir| {
        load_folder(&mut library, dir);
        load_folder(&mut library, &dir.join(REMOVED))
            .into_iter()
            .collect()
    });
    library.user_dir = folder;
    Catalog {
        library: Arc::new(library),
        archived: Arc::new(archived),
        revision: 0,
    }
}

fn current() -> &'static RwLock<Catalog> {
    static LIBRARY: OnceLock<RwLock<Catalog>> = OnceLock::new();
    LIBRARY.get_or_init(|| RwLock::new(load(directory())))
}

pub(super) fn library() -> Arc<Library> {
    current()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .library
        .clone()
}

pub(super) fn palette() -> (Arc<Library>, Arc<HashSet<String>>) {
    let current = current().read().unwrap_or_else(|error| error.into_inner());
    (current.library.clone(), current.archived.clone())
}

pub(super) fn visible(id: &str, archived: &HashSet<String>) -> bool {
    !is_embedded(id) && !archived.contains(id)
}

pub(super) fn revision() -> usize {
    current()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .revision
}

pub(super) fn available(id: &str) -> bool {
    let current = current().read().unwrap_or_else(|error| error.into_inner());
    visible(id, &current.archived)
}

pub(super) fn source_path(directory: &Path, id: &str) -> Result<PathBuf, String> {
    let stem = id
        .strip_prefix("user/")
        .filter(|stem| !matches!(*stem, "" | "." | "..") && !stem.contains(['/', '\\', ':', '\0']))
        .ok_or("Выберите пользовательский узел из библиотеки")?;
    Ok(directory.join(format!("{stem}.wgsl")))
}

static FILE_EDIT: Mutex<()> = Mutex::new(());

pub(super) fn publish(directory: &Path, source: &str) -> Result<PathBuf, String> {
    let id = format!("user/{}", bevy::asset::uuid::Uuid::new_v4());
    publish_as(directory, &id, source)
}

pub(super) fn publish_named(directory: &Path, filename: &Path, source: &str) -> Result<PathBuf, String> {
    publish_as(directory, &node_id(filename)?, source)
}

fn publish_as(directory: &Path, id: &str, source: &str) -> Result<PathBuf, String> {
    if source.len() > SOURCE_LIMIT {
        return Err("Исходник WGSL слишком большой".into());
    }
    let _file = FILE_EDIT.lock().unwrap_or_else(|error| error.into_inner());
    let definition = def::parse(id, source)?;
    // Load disk definitions before writing, so the newly written node is
    // appended once and existing palette indices keep their meanings.
    let catalog = current();
    let path = source_path(directory, id)?;
    let mut current = catalog.write().unwrap_or_else(|error| error.into_inner());
    if let Some(existing) = current.library.find(id) {
        if !current.archived.contains(id) && existing.source == source
            && material_graph::file::read_limited(&path, SOURCE_LIMIT).is_ok_and(|bytes| bytes == source.as_bytes())
        {
            return Ok(path);
        }
        return Err(format!("Узел с таким именем уже существует · {id}"));
    }
    atomic_write(&path, source.as_bytes(), Revision::Missing).map_err(|error| error.to_string())?;
    current.library = Arc::new(append(&current.library, [definition]));
    current.revision += 1;
    Ok(path)
}

pub(super) fn update(
    directory: &Path,
    id: &str,
    expected: &str,
    source: &str,
) -> Result<(), String> {
    if source.len() > SOURCE_LIMIT {
        return Err("Исходник WGSL слишком большой".into());
    }
    let _file = FILE_EDIT.lock().unwrap_or_else(|error| error.into_inner());
    let path = source_path(directory, id)?;
    let definition = def::parse(id, source)?;
    let index = library()
        .defs
        .iter()
        .position(|entry| entry.id == id)
        .filter(|_| available(id))
        .ok_or("Пользовательский узел больше недоступен")?;
    atomic_write(&path, source.as_bytes(), Revision::of(expected.as_bytes()))
        .map_err(|error| format!("Не удалось изменить узел · {error}"))?;
    replace(index, definition);
    Ok(())
}

/// Accept an externally saved source after the editor has validated that exact text.
pub(super) fn accept_external(directory: &Path, id: &str, source: &str) -> Result<(), String> {
    let _file = FILE_EDIT.lock().unwrap_or_else(|error| error.into_inner());
    let library = library();
    let index = library
        .defs
        .iter()
        .position(|entry| entry.id == id)
        .filter(|_| available(id))
        .ok_or("Пользовательский узел больше недоступен")?;
    if library.defs[index].source == source {
        return Ok(());
    }
    let path = source_path(directory, id)?;
    let bytes = material_graph::file::read_limited(&path, SOURCE_LIMIT)?;
    if bytes != source.as_bytes() {
        return Err("Исходник снова изменился на диске".into());
    }
    replace(index, def::parse(id, source)?);
    Ok(())
}

fn replace(index: usize, definition: NodeDef) {
    let mut current = current().write().unwrap_or_else(|error| error.into_inner());
    let mut library = append(&current.library, []);
    library.defs[index] = definition;
    current.library = Arc::new(library);
    current.revision += 1;
}

pub(super) fn remove(directory: &Path, id: &str, expected: &str) -> Result<PathBuf, String> {
    let _file = FILE_EDIT.lock().unwrap_or_else(|error| error.into_inner());
    let path = source_path(directory, id)?;
    if !available(id) || library().find(id).is_none() {
        return Err("Пользовательский узел больше недоступен".into());
    }
    let archived = source_path(&directory.join(REMOVED), id)?;
    std::fs::create_dir_all(directory.join(REMOVED)).map_err(|error| error.to_string())?;
    let bytes = material_graph::file::read_limited(&path, SOURCE_LIMIT)?;
    if bytes != expected.as_bytes() {
        return Err("Исходник изменился на диске. Откройте узел заново".into());
    }
    std::fs::hard_link(&path, &archived).map_err(|error| error.to_string())?;
    if let Err(error) = std::fs::remove_file(&path) {
        let _ = std::fs::remove_file(&archived);
        return Err(error.to_string());
    }
    let mut current = current().write().unwrap_or_else(|error| error.into_inner());
    Arc::make_mut(&mut current.archived).insert(id.to_owned());
    current.library = Arc::new(append(&current.library, []));
    current.revision += 1;
    Ok(archived)
}

fn append(current: &Library, definitions: impl IntoIterator<Item = NodeDef>) -> Library {
    let mut defs = current.defs.clone();
    defs.extend(definitions);
    Library {
        defs,
        errors: current.errors.clone(),
        user_dir: current.user_dir.clone(),
    }
}

pub(super) fn encode(graph: &Graph) -> Value {
    encode_with(graph, &library())
}

fn encode_with(graph: &Graph, library: &Library) -> Value {
    let mut value = graph.to_json(library);
    let mut sources = Map::new();
    for node in &graph.nodes {
        if custom(&node.def)
            && !sources.contains_key(&node.def)
            && let Some(definition) = library.find(&node.def)
        {
            sources.insert(node.def.clone(), Value::String(definition.source.clone()));
        }
    }
    if !sources.is_empty() {
        value[EMBEDDED_SOURCES] = Value::Object(sources);
    }
    value
}

pub(super) fn decode(value: &Value) -> Result<Graph, String> {
    let (graph, definitions) = decode_with(value, &library())?;
    if !definitions.is_empty() {
        let mut current = current().write().unwrap_or_else(|error| error.into_inner());
        let definitions: Vec<_> = definitions
            .into_iter()
            .filter(|entry| current.library.find(&entry.id).is_none())
            .collect();
        if !definitions.is_empty() {
            current.library = Arc::new(append(&current.library, definitions));
        }
    }
    Ok(graph)
}

fn decode_with(value: &Value, library: &Library) -> Result<(Graph, Vec<NodeDef>), String> {
    let Some(sources) = value.get(EMBEDDED_SOURCES) else {
        return Ok((Graph::from_json(value, library)?, Vec::new()));
    };
    let sources = sources
        .as_object()
        .ok_or("Некорректный список исходников WGSL")?;
    let nodes = value["nodes"]
        .as_array()
        .ok_or("У графа нет списка узлов")?;
    if nodes.len() > 256 || sources.len() > 256 {
        return Err("Слишком много узлов в графе".into());
    }
    let mut definitions = Vec::new();
    let mut aliases = HashMap::new();
    let mut seen_aliases = HashSet::new();
    for node in nodes {
        let id = node["def"].as_str().ok_or("Не указан тип узла графа")?;
        if aliases.contains_key(id) {
            continue;
        }
        let Some(source) = sources.get(id) else {
            continue;
        };
        let source = source.as_str().ok_or("Исходник WGSL должен быть текстом")?;
        if !custom(id) || source.len() > SOURCE_LIMIT {
            return Err(format!("Недопустимый исходник узла · {id}"));
        }
        if library.find(id).is_some_and(|local| local.source == source) {
            continue;
        }
        // The same code can be shared by several graphs. A local node of the
        // same name can have different code and must retain its meaning.
        let alias = format!(
            "{PORTABLE_PREFIX}{}",
            blake3::hash(source.as_bytes()).to_hex()
        );
        if library.find(&alias).is_none() && seen_aliases.insert(alias.clone()) {
            definitions
                .push(def::parse(&alias, source).map_err(|error| format!("{id} · {error}"))?);
        }
        aliases.insert(id, alias);
    }
    let graph = if aliases.is_empty() {
        Graph::from_json(value, library)?
    } else {
        let mut portable = value.clone();
        for node in portable["nodes"].as_array_mut().unwrap() {
            if let Some(alias) = node["def"].as_str().and_then(|id| aliases.get(id)) {
                node["def"] = Value::String(alias.clone());
            }
        }
        if definitions.is_empty() {
            Graph::from_json(&portable, library)?
        } else {
            Graph::from_json(&portable, &append(library, definitions.iter().cloned()))?
        }
    };
    Ok((graph, definitions))
}

#[cfg(test)]
pub(super) fn reopen(directory: PathBuf) -> Arc<Library> {
    load(Some(directory)).library
}

#[cfg(test)]
mod tests {
    use super::*;
    use material_painter_graph::Value as Parameter;

    const ID: &str = "user/Portable";
    const SOURCE: &str = "// @name Portable\n// @output gray\n// @param amount: float(0, 1) = 0.25\nfn node(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(params.amount); }";

    #[test]
    fn portable_graph_keeps_user_code_and_parameters_with_or_without_local_conflicts() {
        let builtin = Library::load(None);
        let author = append(&builtin, [def::parse(ID, SOURCE).unwrap()]);
        let mut graph = Graph::new("Portable graph");
        let node = graph.add(ID, [50., 20.]);
        graph
            .node_mut(node)
            .unwrap()
            .params
            .insert("amount".into(), Parameter::Float(0.75));
        let output = graph.add(material_painter_graph::OUTPUT_DEF, [250., 20.]);
        assert!(graph.connect(node, output, "input"));
        assert_eq!(graph.expose(node, "amount", &author), 0);
        let payload = encode_with(&graph, &author);
        assert_eq!(decode_with(&payload, &author).unwrap().0, graph);

        let foreign = append(
            &builtin,
            [def::parse(ID, &SOURCE.replace("params.amount", "0.0")).unwrap()],
        );
        let (clean, definitions) = decode_with(&payload, &builtin).unwrap();
        let (conflict, additional) = decode_with(&payload, &foreign).unwrap();
        assert_eq!(conflict, clean);
        assert_eq!(definitions.len(), 1);
        assert_eq!(additional.len(), 1);
        assert_eq!(definitions[0].source, SOURCE);
        assert_ne!(foreign.find(ID).unwrap().source, SOURCE);
        let alias = clean.node(node).unwrap().def.as_str();
        assert!(is_embedded(alias));
        assert_eq!(definitions[0].id, alias);
        assert_eq!(clean.node(output).unwrap().links["input"], node);
        let imported = append(&builtin, definitions);
        assert_eq!(
            clean.param_value(0, &imported).unwrap().1,
            Parameter::Float(0.75)
        );

        let saved_again = encode_with(&clean, &imported);
        let (reopened, _) = decode_with(&saved_again, &builtin).unwrap();
        assert_eq!(reopened, clean);

        let mut override_builtin = payload.clone();
        override_builtin["nodes"][0]["def"] = Value::String("noise".into());
        override_builtin[EMBEDDED_SOURCES]["noise"] = Value::String(SOURCE.into());
        assert!(decode_with(&override_builtin, &builtin).is_err());
    }

    #[test]
    fn library_edits_preserve_saved_sources_and_removal_survives_reload() {
        let directory = std::env::temp_dir().join(format!(
            "bevy-wgsl-edit-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        let path = publish(&directory, SOURCE).unwrap();
        let id = node_id(&path).unwrap();
        let index = library()
            .defs
            .iter()
            .position(|definition| definition.id == id)
            .unwrap();
        let mut graph = Graph::new("Previous revision");
        let node = graph.add(&id, [0., 0.]);
        let snapshot = encode(&graph);
        let modified = SOURCE.replace("params.amount", "0.0");
        assert!(update(&directory, &id, "stale revision", &modified).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SOURCE);
        update(&directory, &id, SOURCE, &modified).unwrap();
        assert_eq!(library().defs[index].source, modified);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), modified);
        let saved_graph = decode(&snapshot).unwrap();
        let resolved = saved_graph.node(node).unwrap().def.as_str();
        assert!(is_embedded(resolved));
        assert_eq!(library().find(resolved).unwrap().source, SOURCE);

        let archived = remove(&directory, &id, &modified).unwrap();
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(&archived).unwrap(), modified);
        assert!(!available(&id));
        let reopened = load(Some(directory.clone()));
        assert!(reopened.archived.contains(&id));
        assert_eq!(reopened.library.find(&id).unwrap().source, modified);
        assert_eq!(decode(&snapshot).unwrap(), saved_graph);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
