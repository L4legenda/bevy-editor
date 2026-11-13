//! MCP clients own this stdio process; the editor keeps its own GUI process.
use std::{path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let project = match args.as_slice() {
        [flag] if flag == "--help" || flag == "-h" => {
            println!(
                "bevy-editor-mcp --project <project folder or manifest>\nConnect to this project in an already running Bevy Editor."
            );
            return ExitCode::SUCCESS;
        }
        [flag, path] if flag == "--project" => PathBuf::from(path),
        [path] if !path.to_string_lossy().starts_with('-') => PathBuf::from(path),
        _ => {
            eprintln!("Usage: bevy-editor-mcp --project <project folder or manifest>");
            return ExitCode::FAILURE;
        }
    };
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())
        .and_then(|runtime| {
            runtime
                .block_on(bevy_editor::mcp::run_stdio(&project))
                .map_err(|error| error.to_string())
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Bevy Editor MCP: {error}");
            ExitCode::FAILURE
        }
    }
}
