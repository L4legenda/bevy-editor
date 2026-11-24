use super::{Parameter, Session, presets};
use bevy::log::warn;
use bevy_scene_format::{Revision, atomic_write, read_limited};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Settings {
    pub size: f32,
    pub hardness: f32,
    pub spacing: f32,
    pub fill_padding: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            size: 70.,
            hardness: 55.,
            spacing: 8.,
            fill_padding: 2.,
        }
    }
}

impl Settings {
    pub(super) fn capture(session: &Session) -> Self {
        Self {
            size: session.size,
            hardness: session.hardness,
            spacing: session.spacing,
            fill_padding: session.fill_padding,
        }
    }

    pub(super) fn apply(self, session: &mut Session) {
        session.size = self.size;
        session.hardness = self.hardness;
        session.spacing = self.spacing;
        session.fill_padding = self.fill_padding;
    }

    fn load(path: &Path) -> io::Result<Self> {
        fn bounded(value: f32, parameter: Parameter) -> f32 {
            let (min, max) = parameter.bounds();
            value.clamp(min as f32, max as f32)
        }

        let values: Self = serde_json::from_slice(&read_limited(path, 4096)?)?;
        Ok(Self {
            size: bounded(values.size, Parameter::Size),
            hardness: bounded(values.hardness, Parameter::Hardness),
            spacing: bounded(values.spacing, Parameter::Spacing),
            fill_padding: bounded(values.fill_padding, Parameter::FillPadding),
        })
    }
}

pub(super) struct Store {
    path: Option<PathBuf>,
    settings: Settings,
}

impl Default for Store {
    fn default() -> Self {
        Self::open(presets::directory().and_then(|folder| {
            folder
                .parent()
                .map(|root| root.join("paint-brush-defaults.json"))
        }))
    }
}

impl Store {
    fn open(path: Option<PathBuf>) -> Self {
        let settings = path.as_deref().map_or_else(Settings::default, |path| {
            Settings::load(path).unwrap_or_else(|error| {
                if error.kind() != io::ErrorKind::NotFound {
                    warn!("Could not read paint brush defaults: {error}");
                }
                Settings::default()
            })
        });
        Self { path, settings }
    }

    pub(super) fn settings(&self) -> Settings {
        self.settings
    }

    pub(super) fn save(&mut self, settings: Settings) -> io::Result<()> {
        let path = self
            .path
            .as_deref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "папка профиля недоступна"))?;
        atomic_write(path, &serde_json::to_vec(&settings)?, Revision::read(path)?)?;
        self.settings = settings;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::ui::model_viewer::paint::PaintState;

    #[test]
    fn paint_brush_defaults_survive_restart_and_initialize_new_sessions() {
        let directory =
            std::env::temp_dir().join(format!("bevy-brush-settings-{}", uuid::Uuid::new_v4()));
        let path = directory.join("defaults.json");
        let mut store = Store::open(Some(path.clone()));
        let saved = Settings {
            size: 140.,
            hardness: 79.,
            spacing: 13.,
            fill_padding: 7.5,
        };
        let mut current = Session::default();
        saved.apply(&mut current);
        store.save(Settings::capture(&current)).unwrap();

        let mut restarted = PaintState {
            brush_defaults: Store::open(Some(path.clone())),
            ..Default::default()
        };
        restarted.activate(0);
        assert_eq!(Settings::capture(&restarted.sessions[&0]), saved);
        restarted.sessions.get_mut(&0).unwrap().size = 32.;
        restarted.activate(1);
        assert_eq!(restarted.sessions[&1].size, 32.);
        assert_eq!(restarted.sessions[&1].fill_padding, saved.fill_padding);

        restarted.brush_defaults.save(Settings::default()).unwrap();
        assert_eq!(
            Store::open(Some(path.clone())).settings(),
            Settings::default()
        );
        std::fs::write(&path, br#"{"size":900,"fill_padding":-2}"#).unwrap();
        assert_eq!(
            Store::open(Some(path)).settings(),
            Settings {
                size: 800.,
                fill_padding: 0.,
                ..Default::default()
            }
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
