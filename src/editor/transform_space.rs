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

    pub(super) fn set_inspector_pose(&mut self, target: Selection, pose: Transform, field: Field) {
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
