use bevy_editor::{
    EditorProject, create_app,
    project_manager::{create_project_manager_app, remember_project},
};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let path = match args.as_slice() {
        [] => None,
        [flag] if flag == "--manager" => None,
        [flag] if flag == "--help" || flag == "-h" => {
            println!(
                "bevy-editor [project folder | project.bevyeditor | project.json]\n\
                Without arguments, opens the project manager.\n\
                --example         Open the bundled example\n\
                --project PATH    Open a project directly\n\
                --manager         Open the project manager"
            );
            return Ok(());
        }
        [flag] if flag == "--example" => {
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"))
        }
        [path] if !path.to_string_lossy().starts_with('-') => Some(PathBuf::from(path)),
        [flag, path] if flag == "--project" => Some(PathBuf::from(path)),
        _ => return Err("Usage: bevy-editor [--project] [project folder or file]".into()),
    };
    let mut app = if let Some(path) = path {
        let project = EditorProject::load(&path)?;
        let app = create_app(project)?;
        if let Err(error) = remember_project(&path) {
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
