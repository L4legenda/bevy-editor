//! Inspector transforms are relative to the parent in local space; saved poses stay in world space.
use super::{Editor, Field, Selection};
use bevy::prelude::*;

impl Editor {
    pub(super) fn gizmo_rotation(&self, pose: Transform) -> Quat {
        if self.local_space {
            pose.rotation
        } else {
            Quat::IDENTITY
        }
    }

    pub(super) fn inspector_pose(&self, target: Selection) -> Option<Transform> {
        if self.local_space
            && let Some(key) = self.hierarchy_key(target)
        {
            return self
                .edits
                .hierarchy
                .as_ref()?
                .node(key)
                .map(|n| n.local().transform());
        }
        let world = self.pose(target)?.transform();
        if self.local_space
            && let Selection::Imported(index) = target
        {
            let node = self.imported.get(index)?;
            if !self.edits.nodes.contains_key(&node.name) {
                return Some(node.local);
            }
            return Some(GlobalTransform::from(world).reparented_to(&node.parent));
        }
        Some(world)
    }

    pub(super) fn inspector_targets(&self, field: Field) -> impl Iterator<Item = Selection> + '_ {
        let batch = matches!(
            field,
            Field::Position(_) | Field::Rotation(_) | Field::Scale(_)
        );
        self.selection_group
            .as_ref()
            .filter(|(room, targets)| {
                batch && *room == self.room && targets.last().copied() == self.selection
            })
            .map_or(self.selection.as_slice(), |(_, targets)| targets.as_slice())
            .iter()
            .copied()
            .filter(move |&target| !batch || self.pose(target).is_some())
    }

    pub(super) fn inspector_field_allowed(&self, field: Field) -> bool {
        (matches!(
            field,
            Field::Position(_) | Field::Rotation(_) | Field::Scale(_)
        ) || !self.multiple_selected())
            && self
                .selection
                .is_some_and(|target| self.field_allowed(target, field))
            && self
                .inspector_targets(field)
                .all(|target| self.field_allowed(target, field))
    }

    pub(super) fn inspector_field_value(&self, field: Field) -> Option<f32> {
        let mut targets = self.inspector_targets(field);
        let value = self.field_value(targets.next()?, field)?;
        targets
            .all(|target| {
                self.field_value(target, field)
                    .is_some_and(|other| (other - value).abs() <= 1e-5)
            })
            .then_some(value)
    }

    pub(super) fn set_inspector_field(&mut self, field: Field, value: f32) {
        if !value.is_finite() || !self.inspector_field_allowed(field) {
            return;
        }
        let targets: Vec<_> = self.inspector_targets(field).collect();
        if targets.len() <= 1 {
            self.set_field(field, value);
            return;
        }
        let poses: Vec<_> = targets
            .into_iter()
            .filter_map(|target| {
                let pose = self.inspector_pose(target)?;
                Some((target, Self::with_transform_field(pose, field, value)))
            })
            .collect();
        if let Some(graph) = self.edits.hierarchy.as_deref()
            && let Some(mut by_key) = poses
                .iter()
                .map(|(t, p)| {
                    self.hierarchy_key(*t)
                        .map(|key| (key, crate::Pose::from(*p)))
                })
                .collect::<Option<Vec<_>>>()
        {
            let local = self.local_space;
            if !local {
                by_key.sort_by_cached_key(|(key, _)| {
                    let mut depth = 0;
                    let mut node = graph.parent(*key);
                    while let Some(parent) = node {
                        depth += 1;
                        node = graph.parent(parent.key);
                    }
                    depth
                });
            }
            self.edit_hierarchy(|graph| {
                if local {
                    return graph.set_locals(&by_key);
                }
                for (key, pose) in by_key {
                    graph.set_world(key, pose)?;
                }
                Ok(())
            });
        } else {
            for (target, pose) in poses {
                self.set_inspector_pose(target, pose, field);
            }
        }
    }

    pub(super) fn set_inspector_pose(&mut self, target: Selection, pose: Transform, field: Field) {
        if self.local_space
            && let Some(key) = self.hierarchy_key(target)
        {
            self.edit_hierarchy(|h| h.set_local(key, pose.into()));
            return;
        }
        let mut world = pose;
        if self.local_space
            && let Selection::Imported(index) = target
        {
            let Some(node) = self.imported.get(index) else {
                return;
            };
            world = node.parent.mul_transform(pose).compute_transform();
            if matches!(field, Field::Position(_))
                && let Some(current) = self.pose(target)
            {
                // Position edits must not decompose and change the object's rotation or scale.
                world.rotation = current.transform().rotation;
                world.scale = current.transform().scale;
            }
        }
        world.rotation = world.rotation.normalize();
        if crate::Pose::from(world).valid() {
            self.set_pose(target, world);
        }
    }
}
