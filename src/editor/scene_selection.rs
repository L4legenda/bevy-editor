//! Scene selection groups keep one active object for the inspector and pivot.
use super::area_selection::Mode;
use super::*;
impl Editor {
    pub(super) fn selected_targets(&self) -> Vec<Selection> {
        if let Some((room, targets)) = &self.selection_group
            && *room == self.room
            && targets.last().copied() == self.selection
        {
            targets
                .iter()
                .copied()
                .filter(|&t| self.pose(t).is_some())
                .collect()
        } else {
            self.selection.into_iter().collect()
        }
    }
    pub(super) fn multiple_selected(&self) -> bool {
        self.selection_group
            .as_ref()
            .is_some_and(|(room, targets)| {
                *room == self.room
                    && targets.last().copied() == self.selection
                    && targets
                        .iter()
                        .copied()
                        .filter(|&t| self.pose(t).is_some())
                        .take(2)
                        .count()
                        == 2
            })
    }

    pub(super) fn is_selected(&self, target: Selection) -> bool {
        self.selection == Some(target)
            || self
                .selection_group
                .as_ref()
                .is_some_and(|(room, targets)| {
                    *room == self.room
                        && targets.last().copied() == self.selection
                        && targets.contains(&target)
                })
    }
    pub(super) fn select_targets(&mut self, hits: Vec<Selection>, mode: Mode) {
        let mut selected = if mode == Mode::Replace {
            Vec::new()
        } else {
            self.selected_targets()
                .into_iter()
                .filter(|&target| self.pose(target).is_some())
                .collect()
        };
        for hit in hits {
            if mode == Mode::Subtract {
                selected.retain(|&t| t != hit);
            } else if !selected.contains(&hit)
                && self.pose(hit).is_some()
                && !self.object_hidden(hit)
            {
                selected.push(hit);
            }
        }
        self.select(selected.last().copied());
        if selected.len() > 1 {
            self.selection_group = Some((self.room, selected));
        }
    }
    pub(super) fn pick_target(&mut self, hit: Option<Selection>, mode: Mode) {
        if mode == Mode::Add && hit.is_some_and(|t| self.is_selected(t)) {
            self.select_targets(hit.into_iter().collect(), Mode::Subtract);
        } else {
            self.select_targets(hit.into_iter().collect(), mode);
        }
    }
    pub(super) fn batch_editable(&self) -> bool {
        let targets = self.selected_targets();
        !targets.is_empty()
            && targets.iter().all(|t| {
                self.hierarchy_key(*t).is_some()
                    || matches!(t, Selection::Model(_) | Selection::Light(_))
            })
    }
    pub(super) fn object_editable(&self, target: Selection) -> bool {
        (self.hierarchy_key(target).is_some()
            || matches!(
                target,
                Selection::Imported(_) | Selection::Model(_) | Selection::Light(_)
            ))
            && self.pose(target).is_some()
    }
    pub(super) fn object_deleted(&self, target: Selection) -> bool {
        match target {
            Selection::Imported(index) => self.imported.get(index).is_some_and(|node| {
                self.edits.deleted_nodes.contains(&node.name)
                    || self
                        .imported_ancestors
                        .get(&node.name)
                        .is_some_and(|ancestors| {
                            ancestors
                                .iter()
                                .any(|name| self.edits.deleted_nodes.contains(name))
                        })
            }),
            _ => false,
        }
    }
    pub(super) fn deletable_targets(&self) -> bool {
        let targets = self.selected_targets();
        !targets.is_empty() && targets.iter().all(|&target| self.object_editable(target))
    }
    pub(super) fn rename_object(
        &mut self,
        target: Selection,
        name: &str,
    ) -> Result<(), &'static str> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 128 || name.chars().any(char::is_control) {
            return Err("Введите имя от 1 до 128 символов");
        }
        if !self.object_editable(target) {
            return Err("Объект больше не существует");
        }
        if self.label(target) == name {
            return Ok(());
        }
        self.finish_camera_navigation();
        let before = self.edits.clone();
        if let Some(key) = self.hierarchy_key(target) {
            if self.edit_hierarchy(|h| h.rename(key, name)).is_none() {
                return Err("Не удалось переименовать объект");
            }
            self.finish_edit(before);
            self.status = format!("Объект переименован · {name}");
            return Ok(());
        }
        match target {
            Selection::Model(id) => {
                self.edits.model_names.insert(id, name.into());
            }
            Selection::Imported(index) => {
                self.edits
                    .node_names
                    .insert(self.imported[index].name.clone(), name.into());
            }
            Selection::Light(id) => {
                if let Some(light) = self.edits.lights.iter_mut().find(|light| light.id == id) {
                    light.name = name.into();
                }
            }
            _ => unreachable!(),
        }
        self.finish_edit(before);
        self.status = format!("Объект переименован · {name}");
        Ok(())
    }
    pub(super) fn delete_targets(&mut self) {
        if !self.deletable_targets() {
            return;
        }
        if self.edits.hierarchy.is_some() {
            let keys = self.hierarchy_roots();
            if self.edit_hierarchy(|h| h.delete(&keys)).is_some() {
                self.select(None);
            }
            return;
        }
        let targets = self.selected_targets();
        for &target in &targets {
            match target {
                Selection::Imported(index) => {
                    let name = self.imported[index].name.clone();
                    self.edits.deleted_nodes.insert(name.clone());
                    self.edits.node_names.remove(&name);
                    self.edits.nodes.remove(&name);
                }
                Selection::Model(id) => {
                    self.edits.model_names.remove(&id);
                }
                _ => {}
            }
        }
        self.edits
            .models
            .retain(|m| !targets.contains(&Selection::Model(m.id)));
        self.edits
            .lights
            .retain(|l| !targets.contains(&Selection::Light(l.id)));
        self.select(None);
    }
    pub(super) fn duplicate_targets(&mut self) {
        if !self.batch_editable() {
            return;
        }
        if self.edits.hierarchy.is_some() {
            let keys = self.hierarchy_roots();
            if let Some(copies) = self.edit_hierarchy(|h| {
                keys.into_iter()
                    .map(|k| h.duplicate(k))
                    .collect::<std::io::Result<Vec<_>>>()
            }) {
                let targets = copies
                    .into_iter()
                    .map(|k| self.hierarchy_selection(k))
                    .collect();
                self.select_targets(targets, Mode::Replace);
            }
            return;
        }
        let mut copies = Vec::new();
        for target in self.selected_targets() {
            match target {
                Selection::Model(id) => {
                    let name = self.edits.model_names.get(&id).cloned();
                    if let Some(mut model) = self.edits.models.iter().find(|m| m.id == id).cloned()
                    {
                        model.pose.position[0] += 0.5;
                        self.add_model(model);
                        if let (Some(name), Some(Selection::Model(new_id))) = (name, self.selection)
                        {
                            self.edits.model_names.insert(new_id, name);
                        }
                        copies.extend(self.selection);
                    }
                }
                Selection::Light(id) => {
                    if let Some(mut light) = self.light(id).cloned() {
                        light.name = format!(
                            "{} · копия",
                            light.name.chars().take(115).collect::<String>()
                        );
                        light.pose.position[0] += 0.5;
                        if let Some(id) = self.add_light(light) {
                            copies.push(Selection::Light(id));
                        }
                    }
                }
                _ => {}
            }
        }
        self.select_targets(copies, Mode::Replace);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn editor() -> Editor {
        let project = EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        let mut editor = Editor::new(project, RoomEdits::default());
        for x in [-1.0, 1.0] {
            editor.add_model(PlacedModel {
                selector: None,
                uid: None,
                id: 0,
                asset: "models/block.glb".into(),
                pose: Transform::from_xyz(x, 0.0, 0.0).into(),
                bounds: None,
            });
        }
        editor
    }
    #[test]
    fn groups_duplicate_and_delete_all_members_and_single_picking_replaces_them() {
        let mut editor = editor();
        editor.select_targets(
            vec![Selection::Model(1), Selection::Model(2)],
            Mode::Replace,
        );
        let originals = editor.edits.models.clone();
        editor.duplicate_targets();
        assert_eq!(editor.edits.models.len(), 4);
        assert_eq!(
            editor.selected_targets(),
            vec![Selection::Model(3), Selection::Model(4)]
        );
        editor.delete_targets();
        assert_eq!(editor.edits.models, originals);
        editor.select_targets(
            vec![Selection::Model(1), Selection::Model(2)],
            Mode::Replace,
        );
        editor.select(Some(Selection::Model(1)));
        assert_eq!(editor.selected_targets(), vec![Selection::Model(1)]);
        assert!(!editor.is_selected(Selection::Model(2)));
    }
    #[test]
    fn rectangle_addition_excludes_environment_and_groups_belong_to_their_scene() {
        let mut editor = editor();
        editor.select(Some(Selection::Environment));
        editor.select_targets(vec![Selection::Model(1)], Mode::Add);
        assert_eq!(editor.selected_targets(), vec![Selection::Model(1)]);
        assert!(!editor.is_selected(Selection::Environment));
        editor.select_targets(vec![Selection::Model(2)], Mode::Add);
        editor.room = 1;
        assert_eq!(editor.selected_targets(), vec![Selection::Model(2)]);
        assert!(!editor.is_selected(Selection::Model(1)));
    }
}
