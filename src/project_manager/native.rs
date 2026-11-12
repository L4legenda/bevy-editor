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
            let value = choose_folder().map_err(|error| {
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

fn choose_folder() -> io::Result<Option<PathBuf>> {
    #[cfg(target_os = "macos")]
    let output = Command::new("/usr/bin/osascript").arg("-e").arg("try\nreturn POSIX path of (choose folder with prompt \"Выберите папку проекта Bevy Editor\")\non error number -128\nreturn \"\"\nend try").output()?;
    #[cfg(target_os = "windows")]
    let output = Command::new("powershell.exe").args(["-NoProfile", "-STA", "-Command", "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Add-Type -AssemblyName System.Windows.Forms; $dialog = New-Object System.Windows.Forms.FolderBrowserDialog; $dialog.Description = 'Bevy Editor: project folder'; if ($dialog.ShowDialog() -eq 'OK') { [Console]::Write($dialog.SelectedPath) }; $dialog.Dispose()"]).output()?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let output = match Command::new("zenity")
        .args([
            "--file-selection",
            "--directory",
            "--title=Bevy Editor: папка проекта",
        ])
        .output()
    {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Command::new("kdialog")
            .args(["--getexistingdirectory", "."])
            .output()?,
        result => result?,
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

pub(super) fn reveal(folder: &Path) -> io::Result<()> {
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
