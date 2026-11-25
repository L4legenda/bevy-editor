//! Commands and spatial entities for authored hierarchies and scene instances.
use super::*;
use crate::hierarchy::{SceneHierarchy, TreeDrop};
use std::io;

#[cfg(test)]
#[path = "scene_hierarchy_tests.rs"]
pub(super) mod tests;

#[derive(Clone, Copy, Debug)]
pub(super) enum HierarchyAction {
    AddFolder,
    Fold(u32),
    PickParent,
    ToRoot,
    SiblingEarlier,
    SiblingLater,
    CancelParent,
    OpenSource,
    ResetOverrides,
    Unpack,
    Insert(Room),
    Drop(Room, Selection, TreeDrop<Selection>),
}

pub(super) fn available(editor: &Editor, action: HierarchyAction) -> bool {
    if !editor.has_scene() || !editor.ready || !editor.definition().is_native() {
        return false;
    }
    let node = editor
        .selection
        .and_then(|s| editor.hierarchy_key(s))
        .and_then(|k| editor.edits.hierarchy.as_ref()?.node(k));
    match action {
        HierarchyAction::AddFolder => true,
        HierarchyAction::Fold(key) => editor
            .edits
            .hierarchy
            .as_ref()
            .is_some_and(|h| h.node(key).is_some()),
        HierarchyAction::PickParent => node.is_some(),
        HierarchyAction::ToRoot => node.is_some_and(|n| n.entity.parent.is_some()),
        HierarchyAction::SiblingEarlier | HierarchyAction::SiblingLater => node.is_some_and(|n| {
            editor
                .edits
                .hierarchy
                .as_ref()
                .unwrap()
                .sibling_offset(n.key, matches!(action, HierarchyAction::SiblingLater))
                .is_some()
        }),
        HierarchyAction::CancelParent => editor.hierarchy_parenting.is_some(),
        HierarchyAction::OpenSource => node.is_some_and(|n| n.inherited() || n.instance.is_some()),
        HierarchyAction::ResetOverrides => {
            node.is_some_and(|n| editor.edits.hierarchy.as_ref().unwrap().overridden(n.key))
        }
        HierarchyAction::Unpack => node.is_some_and(|n| !n.inherited() && n.instance.is_some()),
        HierarchyAction::Insert(room) => {
            room != editor.room
                && editor
                    .project
                    .scenes
                    .get(room)
                    .is_some_and(|s| s.is_native())
        }
        HierarchyAction::Drop(room, _, _) => {
            room == editor.room && editor.hierarchy_parenting.is_none()
        }
    }
}

impl Editor {
    pub(super) fn hierarchy_selection_snapshot(&self) -> Option<Vec<(u32, Selection)>> {
        self.edits.hierarchy.as_ref()?;
        Some(
            self.selected_targets()
                .into_iter()
                .filter_map(|s| self.hierarchy_key(s).map(|key| (key, s)))
                .collect(),
        )
    }
    pub(super) fn restore_hierarchy_selection(&mut self, snapshot: Option<Vec<(u32, Selection)>>) {
        let Some(snapshot) = snapshot else { return };
        let Some(graph) = &self.edits.hierarchy else {
            return;
        };
        let targets: Vec<_> = snapshot
            .into_iter()
            .filter_map(|(key, kind)| {
                let node = graph.node(key)?;
                match kind {
                    Selection::Camera(_) => node
                        .camera
                        .as_ref()
                        .and_then(|id| self.definition().cameras.iter().position(|c| c.id == *id))
                        .map(Selection::Camera),
                    Selection::Model(_) if node.entity.model.is_some() => {
                        Some(Selection::Model(key))
                    }
                    Selection::Light(_) if node.entity.components.contains_key("light") => {
                        Some(Selection::Light(key))
                    }
                    Selection::Node(_) => Some(Selection::Node(key)),
                    _ => None,
                }
            })
            .collect();
        // Environment and the synthetic preview camera have no authoring node.
        if targets.is_empty() && self.selection.is_some_and(|s|matches!(s,Selection::Environment)||matches!(s,Selection::Camera(shot) if self.definition().cameras.get(shot).is_some_and(|c|c.id=="__scene_preview"))){return}
        self.selection = targets.last().copied();
        self.selection_group = (targets.len() > 1).then_some((self.room, targets));
    }
    pub(super) fn upgrade_flat_history(&mut self, before: &RoomEdits) -> io::Result<RoomEdits> {
        let previous = std::fs::read(self.root.join(&self.definition().file)).ok();
        let mut source = crate::scene_format::Scene::from_slice(&crate::scene_io::encode(
            before,
            previous.as_deref(),
        )?)
        .map_err(io::Error::other)?;
        source.id = self.edits.hierarchy.as_ref().unwrap().source().id.clone();
        let bytes = source.to_source().map_err(io::Error::other)?;
        let upgrade = |edits: &RoomEdits| -> io::Result<RoomEdits> {
            if edits.hierarchy.is_some() {
                return Ok(edits.clone());
            }
            let source = crate::scene_format::Scene::from_slice(&crate::scene_io::encode(
                edits,
                Some(&bytes),
            )?)
            .map_err(io::Error::other)?;
            SceneHierarchy::load(source, &self.root)?.project()
        };
        let before = upgrade(before)?;
        let saved = upgrade(&self.saved)?;
        let undo = self
            .undo
            .iter()
            .map(upgrade)
            .collect::<io::Result<Vec<_>>>()?;
        let redo = self
            .redo
            .iter()
            .map(upgrade)
            .collect::<io::Result<Vec<_>>>()?;
        let navigation = self.camera_navigation.as_ref().map(upgrade).transpose()?;
        self.saved = saved;
        self.undo = undo;
        self.redo = redo;
        self.camera_navigation = navigation;
        Ok(before)
    }
    pub(super) fn set_poses(&mut self, updates: &[(Selection, Transform)]) {
        if self.edits.hierarchy.is_some() {
            let poses: Option<Vec<_>> = updates
                .iter()
                .map(|(target, pose)| {
                    self.hierarchy_key(*target)
                        .map(|key| (key, Pose::from(*pose)))
                })
                .collect();
            if let Some(poses) = poses {
                self.edit_poses(|h| h.set_worlds(&poses));
                return;
            }
        }
        for &(target, pose) in updates {
            self.set_pose(target, pose);
        }
    }
    pub(super) fn hierarchy_key(&self, target: Selection) -> Option<u32> {
        let graph = self.edits.hierarchy.as_ref()?;
        match target {
            Selection::Node(key) => graph.node(key).map(|n| n.key),
            Selection::Model(key) => graph
                .node(key)
                .filter(|n| n.entity.model.is_some())
                .map(|n| n.key),
            Selection::Light(key) => graph
                .node(key)
                .filter(|n| n.entity.components.contains_key("light"))
                .map(|n| n.key),
            Selection::Camera(shot) => {
                let id = &self.definition().cameras.get(shot)?.id;
                graph
                    .nodes
                    .iter()
                    .find(|n| n.camera.as_ref() == Some(id))
                    .map(|n| n.key)
            }
            _ => None,
        }
    }
    pub(super) fn hierarchy_selection(&self, key: u32) -> Selection {
        let Some(node) = self.edits.hierarchy.as_ref().and_then(|h| h.node(key)) else {
            return Selection::Node(key);
        };
        if node.entity.model.is_some() {
            Selection::Model(key)
        } else if let Some(camera) = &node.camera {
            self.definition()
                .cameras
                .iter()
                .position(|c| c.id == *camera)
                .map(Selection::Camera)
                .unwrap_or(Selection::Node(key))
        } else if node.entity.components.contains_key("light") {
            Selection::Light(key)
        } else {
            Selection::Node(key)
        }
    }
    pub(super) fn sync_hierarchy_metadata(&mut self) {
        if self.has_scene()
            && self.definition().is_native()
            && let Some(settings) = &self.edits.scene
        {
            self.project.scenes[self.room].label = settings.label.clone();
            self.project.scenes[self.room].cameras = settings.cameras.clone();
        }
        if self.selection.is_some_and(|t| match t {
            Selection::Node(k) => self
                .edits
                .hierarchy
                .as_ref()
                .is_none_or(|h| h.node(k).is_none()),
            Selection::Model(k) => self.model(k).is_none(),
            Selection::Light(k) => self.light(k).is_none(),
            Selection::Camera(s) => s >= self.definition().cameras.len(),
            _ => false,
        }) {
            self.selection = None;
            self.selection_group = None;
        }
    }
    /// A transform edit preserves IDs, order and asset references. Only publish the
    /// new snapshot after every affected world transform has passed validation.
    pub(super) fn edit_poses(
        &mut self,
        change: impl FnOnce(&mut SceneHierarchy) -> io::Result<()>,
    ) {
        let Some(previous) = self.edits.hierarchy.clone() else {
            return;
        };
        let mut graph = (*previous).clone();
        let result = change(&mut graph).and_then(|()| {
            let mut changed = HashMap::new();
            let mut cameras_changed = false;
            for (node, old) in graph.nodes.iter().zip(&previous.nodes) {
                let moved = node.world != old.world;
                if moved {
                    let pose = node.pose();
                    if !pose.valid() {
                        return Err(io::Error::other("Недопустимая мировая трансформация"));
                    }
                    changed.insert(node.key, pose);
                }
                if node.camera.is_some()
                    && (moved
                        || node.entity.components != old.entity.components
                        || graph.parent(node.key).map(|n| n.world)
                            != previous.parent(old.key).map(|n| n.world))
                {
                    cameras_changed = true;
                }
            }
            if cameras_changed {
                // Camera reset placements are relative to their parent; use the canonical bridge.
                let next = graph.project()?;
                self.edits = next;
                self.sync_hierarchy_metadata();
            } else {
                for model in &mut self.edits.models {
                    if let Some(&pose) = changed.get(&model.id) {
                        model.pose = pose;
                    }
                }
                for light in &mut self.edits.lights {
                    if let Some(&pose) = changed.get(&light.id) {
                        light.pose = pose;
                    }
                }
                self.edits.hierarchy = Some(std::sync::Arc::new(graph));
            }
            Ok(())
        });
        if let Err(error) = result {
            self.status = format!("Изменение не применено · {error}");
        }
    }

    pub(super) fn edit_hierarchy<R>(
        &mut self,
        edit: impl FnOnce(&mut SceneHierarchy) -> io::Result<R>,
    ) -> Option<R> {
        let selected = self.hierarchy_selection_snapshot();
        let mut next = (**self.edits.hierarchy.as_ref()?).clone();
        let result = edit(&mut next).and_then(|value| {
            let edits = next.project()?;
            edits.validate(&self.root)?;
            Ok((value, edits))
        });
        match result {
            Ok((value, edits)) => {
                self.edits = edits;
                self.sync_hierarchy_metadata();
                self.restore_hierarchy_selection(selected);
                Some(value)
            }
            Err(error) => {
                self.status = format!("Изменение не применено · {error}");
                None
            }
        }
    }
    pub(super) fn enable_hierarchy(&mut self) -> io::Result<()> {
        if self.edits.hierarchy.is_some() {
            return Ok(());
        }
        if !self.definition().is_native() {
            return Err(io::Error::other("Иерархия доступна в документах .scene"));
        }
        let previous = std::fs::read(self.root.join(&self.definition().file)).ok();
        let bytes = crate::scene_io::encode(&self.edits, previous.as_deref())?;
        let source = crate::scene_format::Scene::from_slice(&bytes).map_err(io::Error::other)?;
        let graph = SceneHierarchy::load(source, &self.root)?;
        let old_selection = self.selection;
        let source_id = match old_selection {
            Some(Selection::Model(id)) => graph
                .nodes
                .iter()
                .find(|n| {
                    n.entity.model.is_some() && crate::scene_io::number(&n.entity.id, "model") == id
                })
                .map(|n| n.entity.id.clone()),
            Some(Selection::Light(id)) => graph
                .nodes
                .iter()
                .find(|n| {
                    n.entity.components.contains_key("light")
                        && crate::scene_io::number(&n.entity.id, "light") == id
                })
                .map(|n| n.entity.id.clone()),
            Some(Selection::Camera(shot)) => graph
                .nodes
                .iter()
                .find(|n| n.camera.as_ref() == Some(&self.definition().cameras[shot].id))
                .map(|n| n.entity.id.clone()),
            _ => None,
        };
        self.edits = graph.project()?;
        self.sync_hierarchy_metadata();
        self.selection = source_id
            .and_then(|id| self.edits.hierarchy.as_ref()?.by_id(&id).map(|n| n.key))
            .map(|k| self.hierarchy_selection(k));
        self.selection_group = None;
        Ok(())
    }
    pub(super) fn can_drop_in_tree(
        &self,
        source: Selection,
        destination: TreeDrop<Selection>,
    ) -> bool {
        let Some(graph) = &self.edits.hierarchy else {
            return true;
        };
        self.hierarchy_key(source)
            .zip(destination.try_map(|target| self.hierarchy_key(target)))
            .is_some_and(|(key, target)| graph.can_drop_node(key, target))
    }

    fn drop_in_tree(&mut self, source: Selection, destination: TreeDrop<Selection>) {
        let before = self.edits.clone();
        let result = (|| -> io::Result<_> {
            let flat = self.edits.hierarchy.is_none();
            let mut graph = if let Some(graph) = &self.edits.hierarchy {
                (**graph).clone()
            } else {
                let previous = std::fs::read(self.root.join(&self.definition().file)).ok();
                let bytes = crate::scene_io::encode(&self.edits, previous.as_deref())?;
                let mut scene =
                    crate::scene_format::Scene::from_slice(&bytes).map_err(io::Error::other)?;
                // Preserve the order displayed by the flat outliner on first use.
                scene.entities.sort_by(|a, b| {
                    let section = |entity: &crate::scene_format::Entity| {
                        if entity.components.contains_key("camera") {
                            0
                        } else if entity.components.contains_key("light") {
                            2
                        } else {
                            1
                        }
                    };
                    section(a)
                        .cmp(&section(b))
                        .then(a.order.cmp(&b.order))
                        .then(a.id.cmp(&b.id))
                });
                for (index, entity) in scene.entities.iter_mut().enumerate() {
                    entity.order = index as i32;
                }
                SceneHierarchy::load(scene, &self.root)?
            };
            let key_for = |target| {
                if !flat {
                    return self.hierarchy_key(target);
                }
                graph
                    .nodes
                    .iter()
                    .find(|node| match target {
                        Selection::Model(id) => {
                            node.entity.model.is_some()
                                && crate::scene_io::number(&node.entity.id, "model") == id
                        }
                        Selection::Light(id) => {
                            node.entity.components.contains_key("light")
                                && crate::scene_io::number(&node.entity.id, "light") == id
                        }
                        Selection::Camera(shot) => self
                            .definition()
                            .cameras
                            .get(shot)
                            .is_some_and(|camera| node.camera.as_ref() == Some(&camera.id)),
                        _ => false,
                    })
                    .map(|node| node.key)
            };
            let missing = || io::Error::other("Объект больше не существует");
            let key = key_for(source).ok_or_else(missing)?;
            let target = destination.try_map(key_for).ok_or_else(missing)?;
            if !graph.drop_node(key, target)? {
                return Ok(None);
            }
            let parent = graph.parent(key).map(|node| node.key);
            let edits: RoomEdits = graph.project()?;
            edits.validate(&self.root)?;
            Ok(Some((edits, key, parent)))
        })();
        match result {
            Ok(Some((edits, key, parent))) => {
                self.edits = edits;
                self.sync_hierarchy_metadata();
                self.finish_edit(before);
                if self.hierarchy_key(self.hierarchy_selection(key)) == Some(key) {
                    if let Some(parent) = parent {
                        self.hierarchy_collapsed.remove(&(self.room, parent));
                    }
                    let result = self.hierarchy_selection(key);
                    self.select(Some(result));
                    self.tree_drop_arrival =
                        Some((self.room, source, result, std::time::Instant::now()));
                }
            }
            Ok(None) => {}
            Err(error) => self.status = format!("Перенос не выполнен · {error}"),
        }
    }
    pub(super) fn hierarchy_roots(&self) -> Vec<u32> {
        let keys: Vec<_> = self
            .selected_targets()
            .into_iter()
            .filter_map(|s| self.hierarchy_key(s))
            .collect();
        let Some(h) = &self.edits.hierarchy else {
            return keys;
        };
        keys.iter()
            .copied()
            .filter(|&key| {
                let mut parent = h.parent(key);
                while let Some(node) = parent {
                    if keys.contains(&node.key) {
                        return false;
                    }
                    parent = h.parent(node.key);
                }
                true
            })
            .collect()
    }
    pub(super) fn transform_targets(&self) -> Vec<Selection> {
        if self.edits.hierarchy.is_none() {
            return self.selected_targets();
        }
        let roots = self.hierarchy_roots();
        self.selected_targets()
            .into_iter()
            .filter(|&target| {
                self.hierarchy_key(target)
                    .is_none_or(|key| roots.contains(&key))
            })
            .collect()
    }
    pub(super) fn hierarchy_parent_target(&mut self, target: Selection) -> bool {
        let Some((room, key)) = self.hierarchy_parenting else {
            return false;
        };
        if room != self.room {
            self.hierarchy_parenting = None;
            return false;
        }
        let parent = if target == Selection::Environment {
            None
        } else {
            self.hierarchy_key(target)
        };
        if target != Selection::Environment && parent.is_none() {
            return true;
        }
        let before = self.edits.clone();
        if self.edit_hierarchy(|h| h.reparent(key, parent)).is_some() {
            self.hierarchy_parenting = None;
            self.finish_edit(before);
            self.status = "Родитель изменён · положение в мире сохранено".into();
        }
        true
    }
    pub(super) fn hierarchy_action(&mut self, action: HierarchyAction) {
        if !available(self, action) {
            return;
        }
        if let HierarchyAction::Drop(_, source, destination) = action {
            self.drop_in_tree(source, destination);
            return;
        }
        if let HierarchyAction::Fold(key) = action {
            if !self.hierarchy_collapsed.remove(&(self.room, key)) {
                self.hierarchy_collapsed.insert((self.room, key));
            }
            return;
        }
        if matches!(action, HierarchyAction::CancelParent) {
            self.hierarchy_parenting = None;
            self.status = "Смена родителя отменена".into();
            return;
        }
        if matches!(action, HierarchyAction::OpenSource) {
            let path = self
                .selection
                .and_then(|s| self.hierarchy_key(s))
                .and_then(|key| {
                    let h = self.edits.hierarchy.as_ref()?;
                    h.source_path(key).map(str::to_owned)
                });
            if let Some(path) = path {
                if let Some(room) = self.project.scenes.iter().position(|s| s.file == path) {
                    self.open_document(room);
                } else {
                    self.status = format!("Исходная сцена не найдена в проекте · {path}");
                }
            }
            return;
        }
        let before = self.edits.clone();
        // Preserve selection source IDs while a flat document is upgraded.
        let old_targets = self.selected_targets();
        if let Err(error) = self.enable_hierarchy() {
            self.status = error.to_string();
            return;
        }
        if before.hierarchy.is_none() && !old_targets.is_empty() {
            let targets = old_targets
                .iter()
                .filter_map(|target| {
                    let h = self.edits.hierarchy.as_ref()?;
                    let key = match target {
                        Selection::Model(id) => h
                            .nodes
                            .iter()
                            .find(|n| {
                                n.entity.model.is_some()
                                    && crate::scene_io::number(&n.entity.id, "model") == *id
                            })
                            .map(|n| n.key),
                        Selection::Light(id) => h
                            .nodes
                            .iter()
                            .find(|n| {
                                n.entity.components.contains_key("light")
                                    && crate::scene_io::number(&n.entity.id, "light") == *id
                            })
                            .map(|n| n.key),
                        _ => self.hierarchy_key(*target),
                    }?;
                    Some(self.hierarchy_selection(key))
                })
                .collect();
            self.select_targets(targets, area_selection::Mode::Replace);
        }
        let key = self.selection.and_then(|s| self.hierarchy_key(s));
        let root = self.root.clone();
        let success = match action {
            HierarchyAction::AddFolder => {
                if let Some(key) = self.edit_hierarchy(|h| {
                    // Put the new folder at the top of the tree where its entrance is visible.
                    let first = h
                        .nodes
                        .iter()
                        .find(|node| node.entity.parent.is_none())
                        .map(|node| node.key);
                    let key = h.add_group(None)?;
                    if let Some(first) = first {
                        h.drop_node(key, TreeDrop::Before(first))?;
                    }
                    Ok(key)
                }) {
                    self.folder_arrival = Some((self.room, key, std::time::Instant::now()));
                    self.select(Some(Selection::Node(key)));
                    true
                } else {
                    false
                }
            }
            HierarchyAction::PickParent => {
                self.tool = Tool::Universal;
                self.hierarchy_parenting = key.map(|key| (self.room, key));
                self.status =
                    "Выберите нового родителя в дереве · «В корень» уберёт родителя".into();
                true
            }
            HierarchyAction::ToRoot => key.is_some_and(|key| {
                let changed = self.edit_hierarchy(|h| h.reparent(key, None)).is_some();
                if changed {
                    self.hierarchy_parenting = None;
                }
                changed
            }),
            HierarchyAction::SiblingEarlier | HierarchyAction::SiblingLater => {
                key.is_some_and(|key| {
                    self.edit_hierarchy(|h| {
                        h.move_sibling(key, matches!(action, HierarchyAction::SiblingLater))
                    })
                    .is_some()
                })
            }
            HierarchyAction::ResetOverrides => {
                key.is_some_and(|key| self.edit_hierarchy(|h| h.reset_overrides(key)).is_some())
            }
            HierarchyAction::Unpack => {
                key.is_some_and(|key| self.edit_hierarchy(|h| h.unpack(key)).is_some())
            }
            HierarchyAction::Insert(room) => {
                if let Some(scene) = self.project.scenes.get(room).cloned() {
                    let pose = Transform::from_translation(self.pivot).into();
                    if let Some(key) = self
                        .edit_hierarchy(|h| h.insert_scene(&scene.file, &scene.label, pose, &root))
                    {
                        self.select(Some(Selection::Node(key)));
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            _ => false,
        };
        if success {
            self.finish_edit(before);
        } else if before.hierarchy.is_none() {
            self.edits = before;
            self.sync_hierarchy_metadata();
        }
    }
}

/// Reparent render roots under canonical local TRS nodes, preserving affine shear.
pub(super) fn sync(world: &mut World) {
    let editor = world.resource::<Editor>();
    let room = editor.has_scene().then_some(editor.room);
    let graph = room.and(editor.edits.hierarchy.clone());
    let scene = world.resource::<Scene>();
    if graph.is_none()
        && scene.spatial.is_empty()
        && scene.spatial_graph.is_none()
        && scene.spatial_room == room
    {
        return;
    }
    let mut spatial = std::mem::take(&mut world.resource_mut::<Scene>().spatial);
    let changed_room = world.resource::<Scene>().spatial_room != room;
    let changed_graph = changed_room
        || match (&graph, &world.resource::<Scene>().spatial_graph) {
            (Some(a), Some(b)) => !std::sync::Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
    if graph.is_none() && spatial.is_empty() {
        let mut scene = world.resource_mut::<Scene>();
        scene.spatial_graph = None;
        scene.spatial_room = room;
        return;
    }
    let topology_changed = changed_room
        || (changed_graph
            && match (&graph, &world.resource::<Scene>().spatial_graph) {
                (Some(new), Some(old)) => {
                    new.nodes.len() != old.nodes.len()
                        || new.nodes.iter().any(|n| {
                            old.node(n.key)
                                .is_none_or(|previous| previous.entity.parent != n.entity.parent)
                        })
                }
                _ => true,
            });
    if topology_changed {
        let old_ids: std::collections::HashSet<_> = spatial.values().copied().collect();
        // Detach first, then reconcile. Deleting a parent must not despawn surviving children.
        let detach: Vec<_> = world
            .query::<(Entity, &ChildOf)>()
            .iter(world)
            .filter(|(_, p)| old_ids.contains(&p.parent()))
            .map(|(e, _)| e)
            .collect();
        for entity in detach {
            world.entity_mut(entity).remove::<ChildOf>();
        }
        let remove: Vec<_> = spatial
            .iter()
            .filter(|(key, _)| {
                changed_room || graph.as_ref().is_none_or(|g| g.node(**key).is_none())
            })
            .map(|(&key, &entity)| (key, entity))
            .collect();
        for (key, entity) in remove {
            spatial.remove(&key);
            let _ = world.despawn(entity);
        }
    }
    if let Some(graph) = &graph {
        if changed_graph {
            for node in &graph.nodes {
                let entity = *spatial.entry(node.key).or_insert_with(|| {
                    world
                        .spawn((
                            Transform::IDENTITY,
                            GlobalTransform::default(),
                            Visibility::Inherited,
                        ))
                        .id()
                });
                world
                    .get_mut::<Transform>(entity)
                    .unwrap()
                    .set_if_neq(node.local().transform());
            }
        }
        if topology_changed {
            for node in &graph.nodes {
                if let Some(parent) = graph.parent(node.key) {
                    world
                        .entity_mut(spatial[&node.key])
                        .insert(ChildOf(spatial[&parent.key]));
                }
            }
        }
        let models: Vec<_> = world
            .resource::<Scene>()
            .models
            .iter()
            .map(|(&key, &(entity, _))| (key, entity))
            .collect();
        for (key, entity) in models {
            if let Some(parent) = spatial.get(&key)
                && world.get_entity(entity).is_ok()
            {
                attach(world, entity, *parent);
            }
        }
        let lights: Vec<_> = world
            .query::<(Entity, &lighting::LightEntity)>()
            .iter(world)
            .map(|(e, l)| (e, l.0))
            .collect();
        for (entity, key) in lights {
            if let Some(parent) = spatial.get(&key) {
                attach(world, entity, *parent);
            }
        }
    }
    let mut scene = world.resource_mut::<Scene>();
    scene.spatial = spatial;
    scene.spatial_room = room;
    scene.spatial_graph = graph;
}
fn attach(world: &mut World, entity: Entity, parent: Entity) {
    if world
        .get::<ChildOf>(entity)
        .is_none_or(|p| p.parent() != parent)
    {
        world.entity_mut(entity).insert(ChildOf(parent));
    }
    if let Some(mut transform) = world.get_mut::<Transform>(entity) {
        transform.set_if_neq(Transform::IDENTITY);
    }
}
