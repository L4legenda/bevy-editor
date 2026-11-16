//! Open asset folders in the system file manager, selecting individual files.
use std::{
    io,
    path::Path,
    process::{Command, Stdio},
};

pub(super) fn open(root: &Path, relative: &Path) -> io::Result<()> {
    let path = root.join(relative).canonicalize()?;
    let mut child = command(&path, path.is_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(target_os = "windows")]
fn command(path: &Path, directory: bool) -> Command {
    use std::{
        ffi::OsString,
        os::windows::{ffi::OsStringExt, process::CommandExt},
    };

    // canonicalize returns extended-length paths; Explorer expects DOS/UNC paths.
    // Work in UTF-16 to retain every character in native filenames.
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<_> = path.as_os_str().encode_wide().collect();
    let unc: Vec<_> = r"\\?\UNC\".encode_utf16().collect();
    let verbatim: Vec<_> = r"\\?\".encode_utf16().collect();
    let path = if let Some(tail) = wide.strip_prefix(unc.as_slice()) {
        let mut path = OsString::from(r"\\");
        path.push(OsString::from_wide(tail));
        path
    } else {
        OsString::from_wide(wide.strip_prefix(verbatim.as_slice()).unwrap_or(&wide))
    };
    let mut command = Command::new("explorer.exe");
    if directory {
        command.arg(path);
    } else {
        // Explorer's /select syntax needs the path quoted after the comma,
        // including names containing spaces, commas or Cyrillic characters.
        let mut argument = OsString::from("/select,\"");
        argument.push(path);
        argument.push("\"");
        command.raw_arg(argument);
    }
    command
}

#[cfg(target_os = "macos")]
fn command(path: &Path, directory: bool) -> Command {
    let mut command = Command::new("/usr/bin/open");
    if !directory {
        command.arg("-R");
    }
    command.arg(path);
    command
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn command(path: &Path, directory: bool) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(if directory {
        path
    } else {
        path.parent().unwrap_or(path)
    });
    command
}
