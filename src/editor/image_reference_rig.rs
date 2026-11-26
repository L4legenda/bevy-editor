use std::collections::{BTreeMap, HashMap, HashSet};

use bevy::{math::Affine3A, mesh::skinning::SkinnedMesh, prelude::*};

#[path = "image_reference_ik.rs"]
mod ik;
pub(in crate::editor) use ik::{Effector, Reach};

/// Sparse local rotation offsets keyed by a path through the imported scene.
pub(in crate::editor) type Rotations = BTreeMap<String, [f32; 4]>;

pub(super) fn valid(rotations: &Rotations) -> bool {
    rotations.len() <= 4096
        && rotations.iter().all(|(path, rotation)| {
            !path.is_empty() && path.len() <= 4096 && Quat::from_array(*rotation).is_normalized()
        })
}

#[derive(bevy::ecs::system::SystemParam)]
pub(in crate::editor) struct Source<'w, 's> {
    nodes: Query<'w, 's, (&'static Transform, Option<&'static Name>)>,
    skins: Query<'w, 's, &'static SkinnedMesh>,
}

struct Node {
    entity: Entity,
    parent: Option<usize>,
    rest: Transform,
    key: String,
    name: String,
    joint: bool,
}

#[derive(Default)]
pub(in crate::editor) struct Rig {
    nodes: Vec<Node>,
    limbs: Vec<ik::Limb>,
}

pub(in crate::editor) struct Joint<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub position: Vec3,
    pub effector: Option<Effector>,
    /// Converts the saved rotation offset from rest into model space.
    pub basis: Quat,
}

struct JointTransform {
    affine: Affine3A,
    rotation: Quat,
}

impl Source<'_, '_> {
    pub(super) fn read(&self, root: Entity, children: &Query<&Children>) -> Rig {
        let joints: HashSet<_> = children
            .iter_descendants(root)
            .filter_map(|entity| self.skins.get(entity).ok())
            .flat_map(|skin| skin.joints.iter().copied())
            .collect();
        let mut nodes: Vec<Node> = Vec::new();
        let mut pending = Vec::new();
        if !joints.is_empty()
            && let Ok(children) = children.get(root)
        {
            pending.extend(
                children
                    .into_iter()
                    .copied()
                    .enumerate()
                    .rev()
                    .map(|(i, child)| (child, None, i)),
            );
        }
        while let Some((entity, parent, sibling)) = pending.pop() {
            let Ok((rest, name)) = self.nodes.get(entity) else {
                continue;
            };
            let name = name.map(Name::as_str).unwrap_or("Сустав");
            let prefix = parent.map_or("", |index: usize| nodes[index].key.as_str());
            let key = format!("{prefix}/{sibling}:{}:{name}", name.len());
            let index = nodes.len();
            nodes.push(Node {
                entity,
                parent,
                rest: *rest,
                key,
                name: name.into(),
                joint: joints.contains(&entity),
            });
            if let Ok(children) = children.get(entity) {
                pending.extend(
                    children
                        .into_iter()
                        .copied()
                        .enumerate()
                        .rev()
                        .map(|(i, child)| (child, Some(index), i)),
                );
            }
        }
        Rig::from_nodes(nodes)
    }
}

impl Rig {
    fn from_nodes(nodes: Vec<Node>) -> Self {
        let limbs = ik::Limb::find(&nodes);
        Self { nodes, limbs }
    }

    pub(super) fn contains_pose(&self, rotations: &Rotations) -> bool {
        rotations
            .keys()
            .all(|key| self.nodes.iter().any(|node| node.joint && &node.key == key))
    }

    pub(super) fn is_empty(&self) -> bool {
        !self.nodes.iter().any(|node| node.joint)
    }

    fn local(node: &Node, rotations: &Rotations) -> Transform {
        let mut local = node.rest;
        if node.joint
            && let Some(rotation) = rotations.get(&node.key)
        {
            local.rotation *= Quat::from_array(*rotation);
        }
        local
    }

    pub(super) fn apply(&self, rotations: &Rotations, commands: &mut Commands) {
        for node in self.nodes.iter().filter(|node| node.joint) {
            commands
                .entity(node.entity)
                .try_insert(Self::local(node, rotations));
        }
    }

    fn transforms(&self, rotations: &Rotations) -> Vec<JointTransform> {
        let mut transforms: Vec<JointTransform> = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let local = Self::local(node, rotations);
            let transform = if let Some(parent) = node.parent {
                JointTransform {
                    affine: transforms[parent].affine * local.compute_affine(),
                    rotation: transforms[parent].rotation * local.rotation,
                }
            } else {
                JointTransform {
                    affine: local.compute_affine(),
                    rotation: local.rotation,
                }
            };
            transforms.push(transform);
        }
        transforms
    }

    pub(super) fn matrices(&self, rotations: &Rotations) -> HashMap<Entity, Affine3A> {
        self.nodes
            .iter()
            .zip(self.transforms(rotations))
            .filter_map(|(node, transform)| node.joint.then_some((node.entity, transform.affine)))
            .collect()
    }

    pub(super) fn positions(&self, rotations: &Rotations) -> Vec<Vec3> {
        if rotations.is_empty() {
            return Vec::new();
        }
        self.nodes
            .iter()
            .zip(self.transforms(rotations))
            .filter_map(|(node, transform)| node.joint.then(|| transform.affine.translation.into()))
            .collect()
    }

    pub(in crate::editor) fn joints(&self, rotations: &Rotations) -> Vec<Joint<'_>> {
        let transforms = self.transforms(rotations);
        self.nodes
            .iter()
            .zip(&transforms)
            .enumerate()
            .filter_map(|(index, (node, transform))| {
                (node.joint
                    && !node.name.ends_with("_leaf")
                    && !node.name.ends_with("_leaf_l")
                    && !node.name.ends_with("_leaf_r"))
                .then(|| Joint {
                    key: &node.key,
                    name: &node.name,
                    position: transform.affine.translation.into(),
                    effector: self
                        .limbs
                        .iter()
                        .find(|limb| limb.tip == index)
                        .map(|limb| limb.kind),
                    basis: node
                        .parent
                        .map_or(Quat::IDENTITY, |index| transforms[index].rotation)
                        * node.rest.rotation,
                })
            })
            .collect()
    }
}
