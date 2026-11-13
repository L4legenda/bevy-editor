//! Keep Explorer launches window-only while preserving a caller's terminal.
use std::{fs::OpenOptions, io::Write, path::PathBuf};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn AttachConsole(process: u32) -> i32;
    fn GetConsoleWindow() -> *mut std::ffi::c_void;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBoxW(
        window: *mut std::ffi::c_void,
        text: *const u16,
        caption: *const u16,
        kind: u32,
    ) -> i32;
}

pub(super) fn attach_parent_console() -> bool {
    // ATTACH_PARENT_PROCESS never creates a console. It only keeps CLI help
    // and Bevy logs available when the editor is called from a terminal.
    unsafe {
        AttachConsole(u32::MAX);
        !GetConsoleWindow().is_null()
    }
}

fn error_log() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("BevyEditor/logs/editor.log")
}

pub(super) fn report_error(error: &str, console: bool) {
    let path = error_log();
    let logged = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(path.parent().unwrap())?;
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        writeln!(file, "[{time}] {error}")
    })()
    .is_ok();
    if console {
        eprintln!("Bevy Editor: {error}");
        return;
    }
    let message = if logged {
        format!(
            "Не удалось запустить редактор.\n\n{error}\n\nЖурнал: {}",
            path.display()
        )
    } else {
        format!("Не удалось запустить редактор.\n\n{error}")
    };
    let text: Vec<_> = message.encode_utf16().chain(Some(0)).collect();
    let caption: Vec<_> = "Bevy Editor".encode_utf16().chain(Some(0)).collect();
    // MB_OK | MB_ICONERROR. The editor has no window yet on startup failures.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10);
    }
}
