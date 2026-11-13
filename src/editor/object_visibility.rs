//! Persistent editor visibility, kept separately from authored scene data.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    io::{self, Write},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub(super) enum ObjectKey {
    Imported(String),
    Model(u32),
    Light(u32),
}

const SETTINGS: &str = ".bevy-editor/visibility.json";
type StoredVisibility = BTreeMap<String, BTreeSet<ObjectKey>>;

fn read(root: &Path) -> io::Result<StoredVisibility> {
    match fs::read(root.join(SETTINGS)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error),
    }
}

pub(super) fn load(project: &EditorProject) -> io::Result<HashMap<Room, HashSet<ObjectKey>>> {
    let mut stored = read(&project.asset_root)?;
    let mut hidden = HashMap::new();
    for (room, scene) in project.scenes.iter().enumerate() {
        let key = crate::asset_paths::original_asset_path(&project.asset_root, &scene.file)?;
        if let Some(objects) = stored.remove(&key) {
            hidden.insert(room, objects.into_iter().collect());
        }
    }
    Ok(hidden)
}

fn save(root: &Path, file: &str, hidden: &HashSet<ObjectKey>) -> io::Result<()> {
    let mut stored = read(root)?;
    let key = crate::asset_paths::original_asset_path(root, file)?;
    if hidden.is_empty() {
        stored.remove(&key);
    } else {
        stored.insert(key, hidden.iter().cloned().collect());
    }
    let path = root.join(SETTINGS);
    fs::create_dir_all(path.parent().expect("settings directory"))?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        serde_json::to_writer_pretty(&mut output, &stored)?;
        output.write_all(b"\n")?;
        output.sync_all()
    })();
    drop(output);
    let result = result.and_then(|()| fs::rename(&temp, &path));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

impl Editor {
    fn object_key(&self, target: Selection) -> Option<ObjectKey> {
        match target {
            Selection::Imported(index) => self
                .imported
                .get(index)
                .map(|node| ObjectKey::Imported(node.name.clone())),
            Selection::Model(id) => self.model(id).map(|_| ObjectKey::Model(id)),
            Selection::Light(id) => self.light(id).map(|_| ObjectKey::Light(id)),
            _ => None,
        }
    }

    pub(super) fn object_hidden(&self, target: Selection) -> bool {
        self.object_key(target).is_some_and(|key| {
            self.hidden_objects
                .get(&self.room)
                .is_some_and(|hidden| hidden.contains(&key))
        })
    }

    pub(super) fn toggle_visibility(&mut self, target: Selection) {
        let Some(key) = self.object_key(target) else {
            return;
        };
        let mut hidden = self
            .hidden_objects
            .get(&self.room)
            .cloned()
            .unwrap_or_default();
        if !hidden.remove(&key) {
            hidden.insert(key);
        }
        if let Err(error) = save(&self.root, &self.definition().file, &hidden) {
            self.status = format!("Не удалось сохранить видимость объектов · {error}");
            return;
        }
        self.hidden_objects.insert(self.room, hidden);
    }
}

#[derive(Component)]
pub(super) struct BeforeHide(Visibility);

pub(super) fn sync(
    mut commands: Commands,
    editor: Res<Editor>,
    scene: Res<Scene>,
    children: Query<&Children>,
    mut objects: Query<(Entity, &mut Visibility, Option<&BeforeHide>)>,
) {
    if !editor.has_scene() || !editor.ready {
        return;
    }
    let imported = editor
        .imported
        .iter()
        .enumerate()
        .map(|(index, node)| (Selection::Imported(index), node.entity));
    let models = scene
        .models
        .iter()
        .map(|(&id, (entity, _))| (Selection::Model(id), *entity));
    // Explicitly visible children bypass Bevy's inherited visibility, so hide
    // the complete hierarchy and restore every original value on showing it.
    let mut hidden = std::collections::HashSet::new();
    for (target, entity) in imported.chain(models) {
        if editor.object_hidden(target) {
            hidden.insert(entity);
            hidden.extend(children.iter_descendants(entity));
        }
    }
    for (entity, mut visibility, before) in &mut objects {
        if hidden.contains(&entity) {
            if before.is_none() {
                commands.entity(entity).insert(BeforeHide(*visibility));
            }
            visibility.set_if_neq(Visibility::Hidden);
        } else if let Some(before) = before {
            visibility.set_if_neq(before.0);
            commands.entity(entity).remove::<BeforeHide>();
        }
    }
}
