//! Local library of self-contained material graph packages.
use super::{
    material_graph::file::package,
    texture_fill::{self, Bitmap, GraphImages},
};
use bevy::tasks::{IoTaskPool, Task, block_on, poll_once};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub(super) struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub thumbnail: Option<Arc<Bitmap>>,
}

fn entry(path: PathBuf) -> Result<Entry, String> {
    let (mut name, thumbnail) = package::shelf_metadata(&path)?;
    let version = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.rsplit_once(" ~"))
        .and_then(|(_, version)| version.parse::<usize>().ok());
    if let Some(version) = version {
        name.push_str(&format!(" {version}"));
    }
    Ok(Entry {
        path,
        name,
        thumbnail,
    })
}

fn list(directory: &Path) -> Result<Vec<Entry>, String> {
    let files = match fs::read_dir(directory) {
        Ok(files) => files,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut entries: Vec<_> = files
        .flatten()
        .filter(|file| file.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|file| file.path())
        .filter(|path| package::matches(path))
        .filter_map(|path| entry(path).ok())
        .collect();
    entries.sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.path.clone()));
    Ok(entries)
}

pub(super) fn thumbnail(preview: Option<&Bitmap>) -> Result<Option<Vec<u8>>, String> {
    preview
        .map(|bitmap| {
            let pixels = bitmap.pixels();
            let scale = (144. / pixels.width().max(pixels.height()) as f32).min(1.);
            let size = |value: u32| (value as f32 * scale).round().max(1.) as u32;
            let thumbnail =
                image::imageops::thumbnail(pixels, size(pixels.width()), size(pixels.height()));
            texture_fill::encode_png(&thumbnail)
        })
        .transpose()
}

#[derive(Default)]
pub(super) struct Library {
    pub entries: Vec<Entry>,
    pub revision: u64,
    root: Option<PathBuf>,
    scan: Option<Task<Result<Vec<Entry>, String>>>,
    save: Option<Task<Result<Entry, String>>>,
}

impl Library {
    pub fn ensure(&mut self, root: Option<&Path>) {
        if self.root.is_some() {
            return;
        }
        if let Some(directory) = root.map(|root| root.join("graphs")) {
            self.root = Some(directory.clone());
            self.scan = Some(IoTaskPool::get().spawn(async move { list(&directory) }));
        }
    }

    pub fn start_save(
        &mut self,
        payload: Arc<Value>,
        images: Arc<GraphImages>,
        preview: Option<Arc<Bitmap>>,
    ) -> Result<(), String> {
        if self.save.is_some() {
            return Err("Дождитесь сохранения предыдущего графа".into());
        }
        let directory = self
            .root
            .clone()
            .ok_or("Папка библиотеки графов недоступна")?;
        self.save = Some(IoTaskPool::get().spawn(async move {
            let preview = thumbnail(preview.as_deref())?;
            let path = package::export_shelf(&directory, &payload, &images, preview.as_deref())?;
            entry(path)
        }));
        Ok(())
    }

    pub fn poll(&mut self) -> Option<Result<String, String>> {
        if let Some(result) = self
            .scan
            .as_mut()
            .and_then(|task| block_on(poll_once(task)))
        {
            self.scan = None;
            match result {
                Ok(entries) => {
                    for entry in entries {
                        self.add(entry);
                    }
                }
                Err(error) => return Some(Err(error)),
            }
        }
        let result = self
            .save
            .as_mut()
            .and_then(|task| block_on(poll_once(task)))?;
        self.save = None;
        Some(result.map(|entry| {
            let name = entry.name.clone();
            self.add(entry);
            name
        }))
    }

    fn add(&mut self, entry: Entry) {
        if !self.entries.iter().any(|known| known.path == entry.path) {
            self.entries.push(entry);
            self.revision += 1;
        }
    }
}
