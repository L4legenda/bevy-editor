use super::{FileOptions, Padding, Template, recipe};
use bevy::{
    app::AppExit,
    prelude::{Local, MessageReader, ResMut, Resource, warn},
    tasks::{IoTaskPool, Task, block_on, poll_once},
};
use bevy_scene_format::{Revision, atomic_write, read_limited};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(in crate::editor::ui::model_viewer::paint) struct Settings {
    pub padding: Padding,
    pub width: u32,
    pub template: Template,
    pub file: FileOptions,
    pub reveal_folder: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            padding: Padding::default(),
            width: 8,
            template: Template::default(),
            file: FileOptions::default(),
            reveal_folder: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    settings: Settings,
    folder: Option<PathBuf>,
    destination: Option<PathBuf>,
    recipes: Vec<Arc<recipe::Recipe>>,
}

impl Preferences {
    fn load(path: &Path) -> io::Result<Self> {
        let bytes = read_limited(path, 4 * 1024 * 1024)?;
        let mut prefs: Self = serde_json::from_slice(&bytes)?;
        if prefs.recipes.len() > recipe::MAX_TEMPLATES {
            return Err(io::Error::other("Слишком много шаблонов экспорта"));
        }
        for item in &prefs.recipes {
            item.validate()?;
        }
        if let Template::Custom(index) = prefs.settings.template
            && index >= prefs.recipes.len()
        {
            prefs.settings.template = Template::default();
        }
        let file = &mut prefs.settings.file;
        file.set_format(file.format);
        file.size = file
            .size
            .filter(|size| crate::editor::model_paint_data::RESOLUTIONS.contains(size));
        file.normal.height_strength = file.normal.height_strength.clamp(0., 8.);
        prefs.settings.width = prefs.settings.width.clamp(1, 256);
        prefs.destination = prefs.destination.filter(|path| path.is_absolute());
        Ok(prefs)
    }

    fn save(&self, path: &Path) -> io::Result<()> {
        atomic_write(
            path,
            &serde_json::to_vec_pretty(self)?,
            Revision::read(path)?,
        )?;
        Ok(())
    }
}

#[derive(Resource)]
pub(in crate::editor::ui::model_viewer) struct Store {
    path: Option<PathBuf>,
    current: Preferences,
    scheduled: Preferences,
    worker: Option<Task<io::Result<()>>>,
    revision: usize,
}

impl Default for Store {
    fn default() -> Self {
        Self::open(
            super::presets::directory()
                .and_then(|folder| folder.parent().map(|root| root.join("paint-export.json"))),
        )
    }
}

impl Store {
    pub(in crate::editor::ui::model_viewer::paint) fn open(path: Option<PathBuf>) -> Self {
        let current = path.as_deref().map_or_else(Preferences::default, |path| {
            match Preferences::load(path) {
                Ok(prefs) => prefs,
                Err(error) => {
                    if error.kind() != io::ErrorKind::NotFound {
                        warn!("Could not read texture export preferences: {error}");
                    }
                    Preferences::default()
                }
            }
        });
        Self {
            path,
            scheduled: current.clone(),
            current,
            worker: None,
            revision: 0,
        }
    }

    pub(in crate::editor::ui::model_viewer::paint) fn settings(&self) -> Settings {
        self.current.settings
    }

    pub(super) fn set_settings(&mut self, settings: Settings) {
        self.current.settings = settings;
    }

    pub(in crate::editor::ui::model_viewer::paint) fn template_label(
        &self,
        template: Template,
    ) -> &str {
        if let Template::Custom(index) = template
            && let Some(recipe) = self.current.recipes.get(index)
        {
            return &recipe.name;
        }
        template.label()
    }

    pub(in crate::editor::ui::model_viewer::paint) fn recipe_labels(
        &self,
    ) -> impl Iterator<Item = (Template, &str)> {
        self.current
            .recipes
            .iter()
            .enumerate()
            .map(|(index, recipe)| (Template::Custom(index), recipe.name.as_str()))
    }

    pub(super) fn selected_recipe(&self) -> Option<Arc<recipe::Recipe>> {
        let Template::Custom(index) = self.current.settings.template else {
            return None;
        };
        self.current.recipes.get(index).cloned()
    }

    pub(in crate::editor::ui::model_viewer::paint) fn recipe(
        &self,
        index: usize,
    ) -> Option<&recipe::Recipe> {
        self.current.recipes.get(index).map(Arc::as_ref)
    }

    pub(in crate::editor::ui::model_viewer::paint) fn revision(&self) -> usize {
        self.revision
    }

    pub(super) fn duplicate(&mut self) -> io::Result<()> {
        let recipe = self
            .selected_recipe()
            .map(|recipe| (*recipe).clone())
            .or_else(|| recipe::Recipe::from_builtin(self.current.settings.template))
            .ok_or_else(|| io::Error::other("Шаблон не найден"))?;
        let root: String = recipe.name.chars().take(85).collect();
        self.add_recipe(recipe, format!("{root} · копия"))
    }

    pub(super) fn create_recipe(&mut self) -> io::Result<()> {
        let name = "Мой шаблон".to_owned();
        self.add_recipe(recipe::Recipe { name: name.clone(), maps: Vec::new() }, name)
    }

    fn add_recipe(&mut self, mut recipe: recipe::Recipe, stem: String) -> io::Result<()> {
        let index = self.current.recipes.len();
        if index == recipe::MAX_TEMPLATES {
            return Err(io::Error::other("В библиотеке уже 64 пользовательских шаблона"));
        }
        recipe.name = stem.clone();
        for suffix in 2.. {
            if self
                .current
                .recipes
                .iter()
                .all(|item| item.name != recipe.name)
            {
                break;
            }
            recipe.name = format!("{stem} {suffix}");
        }
        self.current.recipes.push(Arc::new(recipe));
        self.current.settings.template = Template::Custom(index);
        self.revision += 1;
        Ok(())
    }

    pub(super) fn delete_selected(&mut self, index: usize) -> bool {
        if self.current.settings.template != Template::Custom(index) || self.recipe(index).is_none() {
            return false;
        }
        self.current.recipes.remove(index);
        self.current.settings.template = match self.current.recipes.len() {
            0 => Template::default(),
            len => Template::Custom(index.min(len - 1)),
        };
        self.revision += 1;
        true
    }

    pub(in crate::editor::ui::model_viewer::paint) fn template_has_maps(&self) -> bool {
        match self.current.settings.template {
            Template::Custom(index) => self.recipe(index).is_some_and(|recipe| !recipe.maps.is_empty()),
            _ => true,
        }
    }

    pub(super) fn edit(
        &mut self,
        index: usize,
        output: usize,
        channel: usize,
        edit: recipe::Edit,
        text: Option<&str>,
    ) -> io::Result<()> {
        let original = self
            .recipe(index)
            .ok_or_else(|| io::Error::other("Шаблон не найден"))?;
        let mut changed = original.clone();
        changed.apply_edit(output, channel, edit, text)?;
        if changed.name != original.name
            && self
                .current
                .recipes
                .iter()
                .any(|item| item.name == changed.name)
        {
            return Err(io::Error::other("Шаблон с таким именем уже есть"));
        }
        if &changed != original {
            self.current.recipes[index] = Arc::new(changed);
            self.revision += 1;
        }
        Ok(())
    }

    pub(super) fn import(&mut self, recipes: Vec<recipe::Recipe>) -> io::Result<usize> {
        let mut catalog = self.current.recipes.clone();
        let mut selected = self.current.settings.template;
        let old_len = catalog.len();
        for mut recipe in recipes {
            if let Some(index) = catalog.iter().position(|item| item.as_ref() == &recipe) {
                selected = Template::Custom(index);
                continue;
            }
            if catalog.len() == recipe::MAX_TEMPLATES {
                return Err(io::Error::other(
                    "В библиотеке уже 64 пользовательских шаблона",
                ));
            }
            let root: String = recipe.name.chars().take(96).collect();
            let mut suffix = 2;
            while catalog.iter().any(|item| item.name == recipe.name) {
                recipe.name = format!("{root} {suffix}");
                suffix += 1;
            }
            selected = Template::Custom(catalog.len());
            catalog.push(Arc::new(recipe));
        }
        let added = catalog.len() - old_len;
        self.current.recipes = catalog;
        self.current.settings.template = selected;
        self.revision += 1;
        Ok(added)
    }

    pub(super) fn folder(&self) -> Option<&Path> {
        self.current.folder.as_deref()
    }

    pub(super) fn set_folder(&mut self, folder: &Path) {
        self.current.folder = Some(folder.to_owned());
    }

    pub(in crate::editor::ui::model_viewer::paint) fn destination(&self) -> Option<&Path> {
        self.current.destination.as_deref()
    }

    pub(super) fn set_destination(&mut self, destination: Option<PathBuf>) {
        if self.current.destination != destination {
            self.current.destination = destination;
            self.revision += 1;
        }
    }

    pub(super) fn schedule_save(&mut self) {
        if let Some(worker) = &mut self.worker {
            let Some(result) = block_on(poll_once(worker)) else {
                return;
            };
            self.worker = None;
            if let Err(error) = result {
                warn!("Could not save texture export preferences: {error}");
            }
        }
        if self.scheduled != self.current
            && let Some(path) = self.path.clone()
        {
            let snapshot = self.current.clone();
            self.scheduled = snapshot.clone();
            self.worker = Some(IoTaskPool::get().spawn(async move { snapshot.save(&path) }));
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let previous = self.worker.take().map(block_on).transpose();
        if let Some(path) = &self.path
            && (self.scheduled != self.current || previous.is_err())
        {
            self.current.save(path)?;
            self.scheduled = self.current.clone();
            return Ok(());
        }
        previous.map(|_| ())
    }
}

pub(in crate::editor::ui::model_viewer) fn on_exit(
    mut exits: MessageReader<AppExit>,
    mut store: ResMut<Store>,
    mut flushed: Local<bool>,
) {
    if exits.read().next().is_some() && !*flushed {
        *flushed = true;
        if let Err(error) = store.flush() {
            warn!("Could not save texture export preferences: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::ui::model_viewer::paint::export::{Depth, Format};
    use bevy::tasks::TaskPool;
    use std::fs;

    #[test]
    fn export_preferences_flush_pending_changes_and_restore_the_last_folder() {
        IoTaskPool::get_or_init(TaskPool::new);
        let root = std::env::temp_dir().join(format!(
            "bevy-export-settings-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        let path = root.join("paint-export.json");
        let folder = root.join("Текстуры");
        let destination = root.join("Следующий экспорт");
        let mut prefs = Store::open(Some(path.clone()));
        let mut settings = Settings {
            template: Template::Godot,
            padding: Padding::Fixed,
            width: 24,
            reveal_folder: true,
            ..Settings::default()
        };
        settings.file.set_format(Format::Tiff);
        settings.file.depth = Depth::U16;
        settings.file.normal.height_strength = 2.5;
        settings.file.normal.use_baked = false;
        prefs.set_settings(settings);
        prefs.set_folder(&folder);
        prefs.set_destination(Some(destination.clone()));
        prefs.schedule_save();
        settings.file.size = Some(2048);
        prefs.set_settings(settings);
        prefs.flush().unwrap();

        let mut reopened = Store::open(Some(path.clone()));
        assert_eq!(reopened.settings(), settings);
        assert_eq!(reopened.folder(), Some(folder.as_path()));
        assert_eq!(reopened.destination(), Some(destination.as_path()));
        reopened.set_destination(None);
        reopened.flush().unwrap();
        let cleared = Store::open(Some(path));
        assert!(cleared.destination().is_none());
        assert_eq!(cleared.folder(), Some(folder.as_path()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_imported_templates_survive_reopening_the_profile() {
        let root = std::env::temp_dir().join(format!(
            "bevy-export-recipes-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let file = root.join("custom.json");
        let path = root.join("paint-export.json");
        fs::write(&file, br#"{"name":"Mask","maps":[{"name":"$mesh_mask","layout":"Grayscale","channels":[{"src":"Metallic"}]}]}"#).unwrap();
        let mut prefs = Store::open(Some(path.clone()));
        let recipe = recipe::load(&file).unwrap();
        assert_eq!(prefs.import(recipe.clone()).unwrap(), 1);
        assert_eq!(prefs.import(recipe).unwrap(), 0);
        prefs.flush().unwrap();
        let reopened = Store::open(Some(path));
        assert_eq!(reopened.settings().template, Template::Custom(0));
        assert_eq!(reopened.template_label(Template::Custom(0)), "Mask");
        assert_eq!(reopened.selected_recipe().unwrap().name, "Mask");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_export_templates_persist_and_deletion_keeps_a_valid_selection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("paint-export.json");
        let mut store = Store::open(Some(path.clone()));
        store.create_recipe().unwrap();
        store.create_recipe().unwrap();
        assert_ne!(store.recipe(0).unwrap().name, store.recipe(1).unwrap().name);
        assert!(!store.template_has_maps());
        store.edit(1, 0, 0, recipe::Edit::Add, None).unwrap();
        assert!(store.template_has_maps());
        store.edit(1, 0, 0, recipe::Edit::Remove, None).unwrap();
        assert!(!store.template_has_maps());
        store.flush().unwrap();

        let mut reopened = Store::open(Some(path.clone()));
        assert_eq!(reopened.settings().template, Template::Custom(1));
        let draft = reopened.selected_recipe().unwrap();
        assert!(draft.maps.is_empty());
        let json = draft.write_json(directory.path()).unwrap();
        assert_eq!(recipe::load(&json).unwrap(), vec![draft.as_ref().clone()]);
        assert!(!reopened.delete_selected(0));
        assert!(reopened.delete_selected(1));
        assert_eq!(reopened.settings().template, Template::Custom(0));
        assert!(reopened.delete_selected(0));
        assert!(!reopened.delete_selected(0));
        assert_eq!(reopened.settings().template, Template::default());
        reopened.flush().unwrap();
        assert_eq!(Store::open(Some(path)).recipe_labels().count(), 0);
    }

    #[test]
    fn export_preferences_preserve_defaults_and_normalize_unsupported_combinations() {
        let root = std::env::temp_dir().join(format!(
            "bevy-export-defaults-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("paint-export.json");
        fs::write(&path, br#"{"settings":{"width":9999,"file":{"format":"jpeg","depth":"u16","size":600,"normal":{"height_strength":-5.0}}}}"#).unwrap();
        let reopened = Store::open(Some(path));
        let settings = reopened.settings();
        assert_eq!(settings.template, Template::Separate);
        assert_eq!(settings.width, 256);
        assert_eq!(settings.file.depth, Depth::U8);
        assert_eq!(settings.file.size, None);
        assert_eq!(settings.file.normal.height_strength, 0.);
        assert!(settings.file.normal.use_baked);
        assert!(reopened.folder().is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
