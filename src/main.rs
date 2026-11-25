#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
mod windows_launcher;

use bevy_editor::{
    create_app_from_path,
    project_manager::{create_project_manager_app, remember_opened_project},
};
use std::path::PathBuf;

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "windows")]
    let console = windows_launcher::attach_parent_console();
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if std::env::var_os("BEVY_EDITOR_PLAY_SNAPSHOT").is_some() {
                eprintln!("Bevy: {error}");
                return std::process::ExitCode::FAILURE;
            }
            #[cfg(target_os = "windows")]
            windows_launcher::report_error(&error.to_string(), console);
            #[cfg(not(target_os = "windows"))]
            eprintln!("Bevy Editor: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if let [flag, snapshot] = args.as_slice()
        && flag == "--play-snapshot"
    {
        let launch = bevy_editor::runtime::GameLaunch::read(snapshot)?;
        let mut app = bevy_editor::runtime::create_game_app(launch.asset_root, launch.level)?;
        return if app.run().is_error() {
            Err("Bevy game exited with an error".into())
        } else {
            Ok(())
        };
    }
    let managed = match args.as_slice() {
        [flag, _, reply] if flag == "--managed-project" => Some(PathBuf::from(reply)),
        _ => None,
    };
    let path = match args.as_slice() {
        [] => None,
        [flag] if flag == "--manager" => None,
        [flag] if flag == "--help" || flag == "-h" => {
            println!(
                "bevy-editor [project folder | project.bevyeditor | project.json]\n\
                Without arguments, opens the project manager.\n\
                --project PATH    Open a project directly\n\
                --play-snapshot PATH  Run an editor level snapshot in Bevy\n\
                --manager         Open the project manager"
            );
            return Ok(());
        }
        [path] if !path.to_string_lossy().starts_with('-') => Some(PathBuf::from(path)),
        [flag, path] if flag == "--project" => Some(PathBuf::from(path)),
        [_, path, _] if managed.is_some() => Some(PathBuf::from(path)),
        _ => return Err("Usage: bevy-editor [--project] [project folder or file]".into()),
    };
    let mut app = if let Some(path) = path {
        let opened = create_app_from_path(&path);
        if let Some(dir) = &managed {
            let result = opened
                .as_ref()
                .map(|(title, _)| title)
                .map_err(|e| e.to_string());
            let mut staged = tempfile::NamedTempFile::new_in(dir)?;
            serde_json::to_writer(staged.as_file_mut(), &result)?;
            staged.persist_noclobber(dir.join("ready"))?;
            // The launcher displays the error and keeps the recent-project list open.
            if opened.is_err() {
                return Ok(());
            }
        }
        let (title, app) = opened?;
        #[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
        let app = {
            let mut app = app;
            app.add_plugins(bevy_editor::mcp::McpPlugin);
            app
        };
        if managed.is_none()
            && !std::env::var_os("BEVY_EDITOR_RENDER_WORKER").is_some_and(|value| value == "1")
            && let Err(error) = remember_opened_project(&path, &title)
        {
            eprintln!("Could not save recent project: {error}");
        }
        app
    } else {
        create_project_manager_app()?
    };
    if app.run().is_error() {
        return Err("Bevy Editor exited with an error".into());
    }
    Ok(())
}
