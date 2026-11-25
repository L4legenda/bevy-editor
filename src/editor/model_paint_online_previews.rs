//! Bounded thumbnail jobs for the current page of a material shelf.
use super::{Material, Provider, cache, downloaded::Record};
use crate::editor::{
    hdri_catalog::public_thumbnail,
    image_previews::{DecodedImage, texture_image},
};
use bevy::{
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, poll_once},
};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

const EDGE: u32 = 128;
const WORKERS: usize = 3;

struct Source {
    key: String,
    origin: Origin,
}

#[derive(Clone)]
enum Origin {
    Remote(String),
    Local(PathBuf, Record),
}

struct Slot {
    image: Handle<Image>,
    requested: bool,
}

struct Job {
    key: String,
    task: Task<Result<DecodedImage, String>>,
}

#[derive(Default)]
pub(in super::super) struct Previews {
    visible: Vec<Source>,
    slots: HashMap<String, Slot>,
    jobs: Vec<Job>,
}

impl Previews {
    pub fn page<'a>(
        &mut self,
        provider: Provider,
        entries: impl IntoIterator<Item = &'a Material>,
        images: &mut Assets<Image>,
        placeholder: &Handle<Image>,
    ) {
        let sources = entries
            .into_iter()
            .filter_map(|entry| {
                Some(Source {
                    key: key(provider, entry)?,
                    origin: Origin::Remote(entry.thumbnail.clone()?),
                })
            })
            .collect();
        self.set_page(sources, images, placeholder);
    }

    pub fn local_page<'a>(
        &mut self,
        root: &Path,
        records: impl IntoIterator<Item = &'a Record>,
        images: &mut Assets<Image>,
        placeholder: &Handle<Image>,
    ) {
        let sources = records
            .into_iter()
            .map(|record| Source {
                key: local_key(record),
                origin: Origin::Local(root.to_path_buf(), record.clone()),
            })
            .collect();
        self.set_page(sources, images, placeholder);
    }

    fn set_page(
        &mut self,
        sources: Vec<Source>,
        images: &mut Assets<Image>,
        placeholder: &Handle<Image>,
    ) {
        self.visible = sources;
        self.slots
            .retain(|key, _| self.visible.iter().any(|source| source.key == *key));
        for source in &self.visible {
            self.slots.entry(source.key.clone()).or_insert_with(|| {
                let placeholder = images.get(placeholder).cloned().unwrap_or_default();
                Slot {
                    image: images.add(placeholder),
                    requested: false,
                }
            });
        }
    }

    pub fn clear(&mut self) {
        self.visible.clear();
        self.slots.clear();
    }

    pub fn image(&self, provider: Provider, entry: &Material) -> Option<Handle<Image>> {
        self.slots
            .get(&key(provider, entry)?)
            .map(|slot| slot.image.clone())
    }

    pub fn local_image(&self, record: &Record) -> Option<Handle<Image>> {
        self.slots
            .get(&local_key(record))
            .map(|slot| slot.image.clone())
    }

    pub fn update(&mut self, active: bool, images: &mut Assets<Image>) {
        let mut index = 0;
        while index < self.jobs.len() {
            let Some(result) = block_on(poll_once(&mut self.jobs[index].task)) else {
                index += 1;
                continue;
            };
            let job = self.jobs.swap_remove(index);
            if let Some(slot) = self.slots.get_mut(&job.key) {
                slot.requested = true;
                if let Ok(decoded) = result
                    && let Some(mut image) = images.get_mut(&slot.image)
                {
                    *image = texture_image(decoded);
                }
            }
        }
        if !active {
            return;
        }
        for source in &self.visible {
            if self.jobs.len() >= WORKERS {
                break;
            }
            let Some(slot) = self.slots.get_mut(&source.key) else {
                continue;
            };
            if slot.requested {
                continue;
            }
            slot.requested = true;
            if self.jobs.iter().any(|job| job.key == source.key) {
                continue;
            }
            let origin = source.origin.clone();
            let key = source.key.clone();
            self.jobs.push(Job {
                key: source.key.clone(),
                task: IoTaskPool::get().spawn(async move {
                    match origin {
                        Origin::Remote(url) => {
                            let file = cache().and_then(|directory| cache_file(&directory, &key));
                            public_thumbnail(&url, file.as_deref(), UVec2::splat(EDGE))
                        }
                        Origin::Local(root, record) => record.preview(&root, EDGE),
                    }
                }),
            });
        }
    }
}

fn local_key(record: &Record) -> String {
    format!("downloaded-{}", record.key())
}

fn key(provider: Provider, entry: &Material) -> Option<String> {
    if !super::valid_id(&entry.id) {
        return None;
    }
    let hash = blake3::hash(entry.thumbnail.as_ref()?.as_bytes()).to_hex();
    Some(format!("{}-{}-{}", provider.key(), entry.id, &hash[..12]))
}

fn cache_file(cache: &Path, key: &str) -> Option<PathBuf> {
    let directory = cache.join("previews");
    fs::create_dir_all(&directory).ok()?;
    Some(directory.join(format!("{key}.png")))
}

#[cfg(test)]
pub(super) fn live_thumbnail(
    root: &Path,
    provider: Provider,
    entry: &Material,
) -> Option<(DecodedImage, PathBuf)> {
    let file = cache_file(root, &key(provider, entry)?)?;
    let image =
        public_thumbnail(entry.thumbnail.as_ref()?, Some(&file), UVec2::splat(EDGE)).ok()?;
    Some((image, file))
}
