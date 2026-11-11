//! Open documents retain data and undo history independently of their loaded ECS entities.
use super::*;

pub(super) struct SceneDocument {
    pub(super) edits: RoomEdits,
    pub(super) saved: RoomEdits,
    pub(super) undo: Vec<RoomEdits>,
    pub(super) redo: Vec<RoomEdits>,
    selection: Option<Selection>,
    imported_selection: Option<String>,
    view: Transform,
    pivot: Vec3,
}

impl SceneDocument {
    fn loaded(editor: &Editor, room: Room, edits: RoomEdits) -> Self {
        Self {
            saved: edits.clone(),
            edits,
            undo: Vec::new(),
            redo: Vec::new(),
            selection: None,
            imported_selection: None,
            view: editor.project.scenes[room].cameras[0]
                .placement
                .pose
                .transform(),
            pivot: Vec3::ZERO,
        }
    }
}

impl Editor {
    pub(super) fn has_scene(&self) -> bool {
        self.open_tabs.contains(&self.room)
    }

    pub(super) fn document_dirty(&self, room: Room) -> bool {
        if self.has_scene() && room == self.room {
            self.dirty()
        } else {
            self.documents
                .get(&room)
                .is_some_and(|document| document.edits != document.saved)
        }
    }

    pub(super) fn preview_edits(&self, room: Room) -> Option<&RoomEdits> {
        if self.has_scene() && room == self.room {
            Some(&self.edits)
        } else if self.open_tabs.contains(&room) {
            self.documents.get(&room).map(|document| &document.edits)
        } else {
            None
        }
    }

    fn stash_document(&mut self) {
        if !self.has_scene() {
            return;
        }
        self.finish_camera_navigation();
        let imported_selection = match self.selection {
            Some(Selection::Imported(index)) => {
                self.imported.get(index).map(|node| node.name.clone())
            }
            _ => self.restore_selection.clone(),
        };
        self.documents.insert(
            self.room,
            SceneDocument {
                edits: std::mem::take(&mut self.edits),
                saved: std::mem::take(&mut self.saved),
                undo: std::mem::take(&mut self.undo),
                redo: std::mem::take(&mut self.redo),
                selection: self
                    .selection
                    .filter(|selection| !matches!(selection, Selection::Imported(_))),
                imported_selection,
                view: self.view,
                pivot: self.pivot,
            },
        );
    }

    pub(super) fn open_document(&mut self, room: Room) -> bool {
        if room >= self.project.scenes.len() {
            self.status = "Не удалось открыть сцену · неизвестная сцена".into();
            return false;
        }
        if self.has_scene() && self.room == room {
            return true;
        }
        // Read and validate a new document before changing the current tab.
        let document = if let Some(document) = self.documents.remove(&room) {
            document
        } else {
            match self.project.read_scene(room) {
                Ok(edits) => SceneDocument::loaded(self, room, edits),
                Err(error) => {
                    self.status = format!("Не удалось открыть сцену · {error}");
                    return false;
                }
            }
        };
        self.stash_document();
        if !self.open_tabs.contains(&room) {
            self.open_tabs.push(room);
        }
        self.room = room;
        self.edits = document.edits;
        self.saved = document.saved;
        self.undo = document.undo;
        self.redo = document.redo;
        self.selection = document.selection;
        self.restore_selection = document.imported_selection;
        self.view = document.view;
        self.pivot = document.pivot;
        self.imported.clear();
        self.pilot = false;
        self.focus = false;
        self.ready = false;
        self.reload = true;
        self.status = format!("Загрузка · {}", self.definition().label);
        true
    }

    pub(super) fn close_document(&mut self, room: Room) {
        let Some(index) = self.open_tabs.iter().position(|&tab| tab == room) else {
            return;
        };
        self.open_tabs.remove(index);
        self.documents.remove(&room);
        if self.room != room {
            return;
        }
        self.edits = RoomEdits::default();
        self.saved = RoomEdits::default();
        self.undo.clear();
        self.redo.clear();
        self.camera_navigation = None;
        self.selection = None;
        self.restore_selection = None;
        self.imported.clear();
        self.pilot = false;
        self.focus = false;
        self.ready = false;
        self.reload = true;
        if let Some(&next) = self
            .open_tabs
            .get(index.min(self.open_tabs.len().saturating_sub(1)))
        {
            self.open_document(next);
        } else {
            self.status = "Откройте сцену в файловом менеджере".into();
        }
    }

    pub(super) fn request_exit(&mut self) {
        self.finish_camera_navigation();
        // Start with the visible document, then follow the tab order.
        self.exit_queue = self
            .open_tabs
            .iter()
            .copied()
            .filter(|&room| room != self.room && self.document_dirty(room))
            .collect();
        if self.dirty() {
            self.exit_queue.insert(0, self.room);
        }
        self.continue_exit();
    }

    pub(super) fn continue_exit(&mut self) {
        if self.exit_queue.is_empty() {
            self.exit_requested = true;
            return;
        }
        let room = self.exit_queue.remove(0);
        if self.open_document(room) {
            self.pending = Some(PendingAction::Close);
            self.status = "Есть несохранённые изменения".into();
        } else {
            self.exit_queue.clear();
        }
    }
}
