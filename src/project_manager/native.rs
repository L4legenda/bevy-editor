//! Platform folder pickers run off the rendering thread; manual paths work too.
use super::{Dialog, Manager, ui::Input};
use bevy::{
    input_focus::{FocusCause, InputFocus},
    prelude::*,
    text::EditableText,
};
use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

type PickResult = Result<Option<PathBuf>, String>;
#[derive(Default, Resource)]
pub(super) struct Picker {
    pending: Option<(Dialog, Arc<Mutex<Option<PickResult>>>)>,
}
impl Picker {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn start(&mut self, dialog: Dialog) {
        if self.busy() {
            return;
        }
        let result = Arc::new(Mutex::new(None));
        let output = result.clone();
        std::thread::spawn(move || {
            let value = choose_folder("Bevy Editor: папка проекта").map_err(|error| {
                format!("Не удалось открыть выбор папки: {error}. Введите путь вручную.")
            });
            if let Ok(mut output) = output.lock() {
                *output = Some(value);
            }
        });
        self.pending = Some((dialog, result));
    }
}

pub(super) fn poll(
    mut picker: ResMut<Picker>,
    mut manager: ResMut<Manager>,
    mut inputs: Query<(Entity, &Input, &mut EditableText)>,
    mut focus: ResMut<InputFocus>,
) {
    let result = picker
        .pending
        .as_ref()
        .and_then(|(dialog, pending)| pending.lock().ok()?.take().map(|result| (*dialog, result)));
    let Some((dialog, result)) = result else {
        return;
    };
    picker.pending = None;
    if manager
        .form
        .as_ref()
        .is_none_or(|form| form.dialog != dialog)
    {
        return;
    }
    match result {
        Ok(Some(path)) => {
            let value = path.to_string_lossy().into_owned();
            if let Some(form) = &mut manager.form {
                form.folder = value.clone();
                form.error.clear();
            }
            for (entity, field, mut input) in &mut inputs {
                if *field == Input::Folder {
                    input.editor_mut().set_text(&value);
                    focus.set(entity, FocusCause::Auto);
                }
            }
        }
        Ok(None) => {}
        Err(error) => manager.error(error),
    }
}

pub(crate) fn choose_folder(prompt: &str) -> io::Result<Option<PathBuf>> {
    choose_folder_in(prompt, None)
}

pub(crate) fn choose_folder_in(prompt: &str, directory: Option<&Path>) -> io::Result<Option<PathBuf>> {
    let directory = directory.filter(|path| path.is_dir());
    #[cfg(target_os = "macos")]
    let output = Command::new("/usr/bin/osascript").arg("-e").arg("on run argv\ntry\nif (item 2 of argv) is \"\" then\nreturn POSIX path of (choose folder with prompt (item 1 of argv))\nelse\nreturn POSIX path of (choose folder with prompt (item 1 of argv) default location (POSIX file (item 2 of argv)))\nend if\non error number -128\nreturn \"\"\nend try\nend run").arg(prompt).arg(directory.unwrap_or_else(|| Path::new(""))).output()?;
    #[cfg(target_os = "windows")]
    let output = {
        use std::os::windows::process::CommandExt;
        Command::new("powershell.exe")
            .creation_flags(0x08000000)
            .env("BEVY_EDITOR_DIRECTORY_PROMPT", prompt)
            .env("BEVY_EDITOR_START_DIRECTORY", directory.unwrap_or_else(|| Path::new("")))
            .args(["-NoProfile", "-STA", "-Command", "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Add-Type -AssemblyName System.Windows.Forms; $dialog = New-Object Windows.Forms.FolderBrowserDialog; $dialog.Description = $env:BEVY_EDITOR_DIRECTORY_PROMPT; if ($env:BEVY_EDITOR_START_DIRECTORY) { $dialog.SelectedPath = $env:BEVY_EDITOR_START_DIRECTORY }; if ($dialog.ShowDialog() -eq 'OK') { [Console]::Write($dialog.SelectedPath) }; $dialog.Dispose()"])
            .output()?
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let output = {
        let mut picker = Command::new("zenity");
        picker.args(["--file-selection", "--directory"]).arg(format!("--title={prompt}"));
        if let Some(directory) = directory {
            let mut filename = std::ffi::OsString::from("--filename=");
            filename.push(directory);
            filename.push("/");
            picker.arg(filename);
        }
        match picker.output() {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Command::new("kdialog")
                .arg("--getexistingdirectory")
                .arg(directory.unwrap_or_else(|| Path::new(".")))
                .args(["--title", prompt])
                .output()?,
            result => result?,
        }
    };
    if !output.status.success() {
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let value = String::from_utf8(output.stdout).map_err(io::Error::other)?;
    let value = value.trim_end_matches(['\r', '\n']);
    Ok((!value.is_empty()).then(|| PathBuf::from(value)))
}

pub(crate) fn reveal(folder: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("/usr/bin/open");
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer.exe");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = Command::new("xdg-open");
    let mut child = command
        .arg(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
