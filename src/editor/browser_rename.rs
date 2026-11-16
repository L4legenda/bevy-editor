//! A rename keeps the file format, document identity and move transaction intact.
use super::*;

pub(super) struct Target {
    pub entry: browser::Entry,
    pub revision: u64,
    pub path: PathBuf,
}

impl Target {
    pub fn new(editor: &Editor, browser: &Browser, entry: browser::Entry) -> Result<Self, String> {
        if matches!(entry, browser::Entry::Folder(_)) {
            return Err("Выберите файл для переименования".into());
        }
        let path = crate::editor::file_moves::entry_path(editor, browser, entry)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            entry,
            revision: browser.scan_revision,
            path,
        })
    }

    pub fn stem(&self) -> String {
        self.path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }

    pub fn extension(&self) -> Option<String> {
        self.path
            .extension()
            .map(|extension| format!(".{}", extension.to_string_lossy()))
    }

    pub fn destination(&self, name: &str) -> Result<PathBuf, String> {
        let extension = self.extension().unwrap_or_default();
        let stem = if !extension.is_empty()
            && name
                .get(name.len().saturating_sub(extension.len())..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(&extension))
        {
            &name[..name.len() - extension.len()]
        } else {
            name
        };
        if stem.trim().is_empty() {
            return Err("Введите имя файла".into());
        }
        let filename = format!("{stem}{extension}");
        browser_files::validate_name(&filename).map_err(str::to_owned)?;
        Ok(self.path.with_file_name(filename))
    }

    pub fn apply(
        &self,
        name: &str,
        editor: &mut Editor,
        browser: &mut Browser,
    ) -> Result<PathBuf, String> {
        if self.revision != browser.scan_revision {
            return Err("Список файлов изменился. Закройте окно и выберите файл снова".into());
        }
        let to = self.destination(name)?;
        crate::editor::file_moves::rename_entry(
            editor,
            browser,
            self.entry,
            to.file_name()
                .and_then(|name| name.to_str())
                .ok_or("Недопустимое имя файла")?,
        )
        .map_err(|error| error.to_string())?;
        Ok(to)
    }
}

pub(super) fn entry_at_path(
    editor: &Editor,
    browser: &Browser,
    path: &std::path::Path,
) -> Option<browser::Entry> {
    if let Some(index) = editor
        .project
        .catalog
        .iter()
        .position(|model| std::path::Path::new(&model.asset) == path)
    {
        return Some(browser::Entry::Model(index));
    }
    let file = browser.files.iter().position(|file| file.path == path)?;
    Some(match browser.files[file].kind {
        browser::FileKind::Image => browser::Entry::Image(file),
        browser::FileKind::Scene => editor
            .project
            .scenes
            .iter()
            .position(|scene| std::path::Path::new(&scene.file) == path)
            .map_or(browser::Entry::File(file), |room| browser::Entry::Scene {
                file,
                room,
            }),
        _ => browser::Entry::File(file),
    })
}
