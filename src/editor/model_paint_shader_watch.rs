//! Observe the file behind the current WGSL buffer without blocking the UI.
use super::*;

const POLL_INTERVAL: f64 = 0.75;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum SourceFile {
    Opened(PathBuf),
    Library { directory: PathBuf, id: String },
}

impl SourceFile {
    fn read(&self, baseline: &str, validated: bool) -> Result<Arc<str>, String> {
        let path = match self {
            Self::Opened(path) => path.clone(),
            Self::Library { directory, id } => node_library::source_path(directory, id)?,
        };
        let disk = open_source(&path).map_err(|error| format!("{} · {error}", path.display()))?;
        if let Self::Library { directory, id } = self
            && validated
            && disk == baseline
        {
            node_library::accept_external(directory, id, &disk)?;
        }
        Ok(disk.into())
    }
}

struct Poll {
    baseline: Arc<str>,
    result: Result<Arc<str>, String>,
}

enum Notice {
    Reloaded,
    Conflict,
    Error(String),
}

#[derive(Default)]
pub(super) struct Watch {
    target: Option<SourceFile>,
    next: f64,
    task: Option<Task<Poll>>,
    notice: Option<Notice>,
}

impl Watch {
    pub(super) fn poll(
        &mut self,
        target: Option<SourceFile>,
        now: f64,
        baseline: &Arc<str>,
        validated: bool,
        source: &EditableText,
        idle: bool,
    ) -> Option<Arc<str>> {
        if self.target != target {
            self.target = target;
            self.next = now;
            self.task = None;
            self.notice = None;
        }
        if !idle || !source.pending_edits.is_empty() || source.pending_paste.is_some() {
            return None;
        }
        let response = self
            .task
            .as_mut()
            .and_then(|task| block_on(poll_once(task)));
        if let Some(response) = response {
            self.task = None;
            if response.baseline == *baseline {
                return self.observe(response.result, baseline, source);
            }
            self.next = now;
        }
        if self.task.is_none()
            && now >= self.next
            && let Some(file) = self.target.clone()
        {
            let baseline = baseline.clone();
            self.next = now + POLL_INTERVAL;
            self.task = Some(IoTaskPool::get().spawn(async move {
                let result = file.read(&baseline, validated);
                Poll { baseline, result }
            }));
        }
        None
    }

    fn observe(
        &mut self,
        result: Result<Arc<str>, String>,
        baseline: &str,
        source: &EditableText,
    ) -> Option<Arc<str>> {
        match result {
            Ok(disk) if disk.as_ref() == baseline => {
                if !matches!(self.notice, Some(Notice::Reloaded)) {
                    self.notice = None;
                }
                None
            }
            Ok(disk) if source.value() == baseline => {
                self.notice = Some(Notice::Reloaded);
                Some(disk)
            }
            Ok(_) => {
                self.notice = Some(Notice::Conflict);
                None
            }
            Err(error) => {
                self.notice = Some(Notice::Error(format!(
                    "Не удалось проверить файл WGSL · {error}"
                )));
                None
            }
        }
    }

    pub(super) fn notice(&self) -> &str {
        match &self.notice {
            Some(Notice::Reloaded) => "WGSL обновлён с диска",
            Some(Notice::Conflict) => {
                "Файл изменился на диске · локальные правки остались в редакторе"
            }
            Some(Notice::Error(error)) => error,
            None => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opened_file_is_read_again_after_an_external_write() {
        let path = std::env::temp_dir().join(format!(
            "bevy-wgsl-watch-{}.wgsl",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, "initial").unwrap();
        let file = SourceFile::Opened(path.clone());
        let clean = file.read("", false).unwrap();
        assert_eq!(&*clean, "initial");
        std::fs::write(&path, "changed externally").unwrap();
        assert_eq!(&*file.read(&clean, true).unwrap(), "changed externally");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn changed_source_reloads_only_when_the_buffer_is_clean() {
        let mut watch = Watch::default();
        let mut input = EditableText::new("saved");
        let disk = Arc::<str>::from("external");
        assert_eq!(
            watch.observe(Ok(disk.clone()), "saved", &input),
            Some(disk.clone())
        );
        input.editor_mut().set_text("local");
        assert_eq!(watch.observe(Ok(disk), "saved", &input), None);
        assert_eq!(input.value(), "local");
        assert!(watch.notice().contains("локальные правки"));
    }
}
