use std::path::PathBuf;

use bevy_editor::{EditorProject, create_app};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let path = match args.as_slice() {
        [] => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"),
        [flag] if flag == "--help" || flag == "-h" => {
            println!(
                "bevy-editor [project.json]\nWithout arguments, opens the bundled example project."
            );
            return Ok(());
        }
        [path] => PathBuf::from(path),
        _ => return Err("Usage: bevy-editor [project.json]".into()),
    };
    let mut app = create_app(EditorProject::load(path)?)?;
    let exit = app.run();
    if exit.is_error() {
        return Err("Editor exited with an error".into());
    }
    Ok(())
}
