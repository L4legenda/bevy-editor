//! Owned play sessions. A snapshot keeps unsaved editor state out of game writes.
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{Editor, Selection};
use crate::runtime::{GameLaunch, GameLevel};
use bevy::prelude::*;

#[derive(Default)]
pub(super) struct Session {
    child: Option<Child>,
    files: Option<LaunchFiles>,
    pub error: Option<String>,
}

impl Session {
    pub fn running(&self) -> bool {
        self.child.is_some()
    }

    pub fn stop(&mut self) -> io::Result<()> {
        if let Some(child) = &mut self.child {
            if child.try_wait()?.is_none() {
                child.kill()?;
            }
            child.wait()?;
        }
        self.child = None;
        self.files = None;
        self.error = None;
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct LaunchFiles {
    snapshot: PathBuf,
    log: PathBuf,
}

impl Drop for LaunchFiles {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.snapshot);
        let _ = fs::remove_file(&self.log);
    }
}

pub(super) fn snapshot(editor: &Editor) -> GameLaunch {
    let definition = editor.definition();
    let cameras = editor
        .edits
        .scene
        .as_ref()
        .map_or(&definition.cameras, |settings| &settings.cameras);
    let camera = match editor.selection {
        Some(Selection::Camera(index)) => definition.cameras.get(index),
        _ => None,
    }
    .filter(|camera| cameras.iter().any(|item| item.id == camera.id))
    .or_else(|| cameras.first())
    .map(|camera| camera.id.clone())
    .unwrap_or_default();
    GameLaunch {
        scene: None,
        asset_root: editor.root.clone(),
        level: GameLevel {
            uid: definition.uid.clone(),
            title: format!(
                "{} · {}",
                editor.project.title,
                editor
                    .edits
                    .scene
                    .as_ref()
                    .map_or(definition.label.as_str(), |settings| settings
                        .label
                        .as_str())
            ),
            asset: definition.asset.clone(),
            groups: definition.groups.clone(),
            excluded_nodes: definition.excluded_nodes.clone(),
            cameras: cameras.clone(),
            camera,
            edits: editor.edits.clone(),
            environment: editor.environment().clone(),
        },
    }
}

fn write_snapshot(launch: &GameLaunch) -> io::Result<(LaunchFiles, File)> {
    launch.level.validate(&launch.asset_root)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let name = format!("bevy-editor-play-{}-{stamp}", std::process::id());
    let files = LaunchFiles {
        snapshot: std::env::temp_dir().join(format!("{name}.json")),
        log: std::env::temp_dir().join(format!("{name}.log")),
    };
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&files.snapshot)?;
    serde_json::to_writer(file, launch).map_err(io::Error::other)?;
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&files.log)?;
    Ok((files, log))
}

pub(super) fn start(editor: &mut Editor) -> io::Result<()> {
    if editor.play.running() {
        return Ok(());
    }
    let mut launch = snapshot(editor);
    if editor.definition().is_native() {
        let path = editor.root.join(&editor.definition().file);
        let previous = fs::read(path)?;
        let index = crate::asset_registry::index(&editor.root)?;
        let bytes = crate::scene_io::encode_indexed(&editor.edits, Some(&previous), Some(&index))?;
        launch.scene =
            Some(bevy_scene_format::Scene::from_slice(&bytes).map_err(io::Error::other)?);
    }
    if editor.definition().play.is_some() {
        crate::hdri::export_preset(&editor.root, launch.level.edits.lighting.as_ref())?;
    }
    let (files, log) = write_snapshot(&launch)?;
    let mut command = if let Some(play) = &editor.definition().play {
        play.command()?
    } else {
        let mut command = Command::new(std::env::current_exe()?);
        command.arg("--play-snapshot").arg(&files.snapshot);
        command
    };
    command
        .env_remove("BEVY_EDITOR_RENDER_WORKER")
        .env("BEVY_EDITOR_PLAY_SNAPSHOT", &files.snapshot)
        .env("BEVY_EDITOR_ASSET_ROOT", &editor.root)
        .env("BEVY_EDITOR_LEVEL", &editor.definition().file)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // GUI window without a console window.
    }
    let child = command.spawn()?;
    editor.play.child = Some(child);
    editor.play.files = Some(files);
    editor.play.error = None;
    editor.status = "Игровое окно запущено".into();
    Ok(())
}

pub(super) fn poll(mut editor: ResMut<Editor>) {
    let Some(child) = &mut editor.play.child else {
        return;
    };
    match child.try_wait() {
        Ok(None) => {}
        Ok(Some(status)) => {
            editor.play.child = None;
            if !status.success() {
                let detail = editor
                    .play
                    .files
                    .as_ref()
                    .and_then(|files| fs::read_to_string(&files.log).ok())
                    .and_then(|log| {
                        log.lines()
                            .rev()
                            .find(|line| !line.trim().is_empty())
                            .map(|line| line.chars().take(250).collect::<String>())
                    });
                editor.play.error = Some(format!(
                    "Игра завершилась с ошибкой ({status}){}",
                    detail
                        .map(|detail| format!(" · {detail}"))
                        .unwrap_or_default()
                ));
            }
            editor.play.files = None;
            editor.status = "Игра остановлена".into();
        }
        Err(error) => {
            editor.play.error = Some(format!("Не удалось проверить игровое окно · {error}"));
        }
    }
}

#[cfg(test)]
#[path = "play_tests.rs"]
mod tests;
