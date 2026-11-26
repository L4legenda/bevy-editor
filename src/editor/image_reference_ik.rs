//! Two-bone reach controls for humanoid skin joints. All saved edits are
//! ordinary local rotation offsets, shared with the rotation rings.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum Effector {
    Hand,
    Foot,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Segment {
    Upper,
    Middle,
    Tip,
}

fn role(name: &str) -> Option<(Effector, Segment)> {
    use Effector::*;
    use Segment::*;
    let part = |name: &str| {
        Some(match name {
            "upperarm" | "uparm" | "arm" | "humerus" => (Hand, Upper),
            "lowerarm" | "forearm" | "elbow" => (Hand, Middle),
            "hand" | "wrist" => (Hand, Tip),
            "upperleg" | "upleg" | "thigh" => (Foot, Upper),
            "lowerleg" | "leg" | "calf" | "shin" | "knee" => (Foot, Middle),
            "foot" | "ankle" => (Foot, Tip),
            _ => return None,
        })
    };
    let local = name.rsplit([':', '|']).next()?;
    let name: String = local
        .chars()
        .filter(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let name = ["mixamorig", "jbip", "bip", "def"]
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .unwrap_or(&name);
    part(name).or_else(|| {
        ["left", "right", "l", "r"].iter().find_map(|side| {
            name.strip_prefix(side)
                .and_then(part)
                .or_else(|| name.strip_suffix(side).and_then(part))
        })
    })
}

pub(super) struct Limb {
    pub(super) kind: Effector,
    upper: usize,
    middle: usize,
    pub(super) tip: usize,
}

impl Limb {
    pub(super) fn find(nodes: &[Node]) -> Vec<Self> {
        let roles: Vec<_> = nodes
            .iter()
            .map(|n| n.joint.then(|| role(&n.name)).flatten())
            .collect();
        // Helper and twist nodes may sit between the three deforming joints.
        let parent = |index: usize| {
            std::iter::successors(nodes[index].parent, |i| nodes[*i].parent)
                .find(|i| roles[*i].is_some())
        };
        roles
            .iter()
            .enumerate()
            .filter_map(|(tip, role)| {
                let (kind, Segment::Tip) = (*role)? else {
                    return None;
                };
                let middle = parent(tip)?;
                let upper = parent(middle)?;
                (roles[middle] == Some((kind, Segment::Middle))
                    && roles[upper] == Some((kind, Segment::Upper)))
                .then_some(Self {
                    kind,
                    upper,
                    middle,
                    tip,
                })
            })
            .collect()
    }
}

struct JointRotation {
    key: String,
    basis: Quat,
    initial: Quat,
}

impl JointRotation {
    fn write(&self, inherited: Quat, desired: Quat, rotations: &mut Rotations) {
        let mut offset =
            ((inherited * self.basis).inverse() * desired * self.basis * self.initial).normalize();
        // Both signs describe the same rotation. Retain the starting sign
        // and the exact authored value when a joint has effectively not moved.
        if offset.dot(self.initial) < 0.0 {
            offset = -offset;
        }
        if offset.abs_diff_eq(self.initial, 1e-6) {
            return;
        }
        if offset.xyz().length_squared() < 1e-12 {
            rotations.remove(&self.key);
        } else {
            rotations.insert(self.key.clone(), offset.to_array());
        }
    }
}

/// Geometry and local rest bases at mouse-down, in the upper joint's parent
/// space. Imported armature units, mirrored ancestors and canvas zoom can
/// change the input mapping without changing the lengths used by the solver.
pub(in crate::editor) struct Reach {
    model_to_parent: Affine3A,
    upper: Vec3,
    lower: Vec3,
    aim: Vec3,
    bend: Vec3,
    joints: [JointRotation; 3],
}

impl Rig {
    pub(in crate::editor) fn reach(&self, key: &str, rotations: &Rotations) -> Option<Reach> {
        let limb = self
            .limbs
            .iter()
            .find(|limb| self.nodes[limb.tip].key == key)?;
        let transforms = self.transforms(rotations);
        let parent = self.nodes[limb.upper].parent.map(|i| &transforms[i]);
        let model_to_parent = parent.map_or(Affine3A::IDENTITY, |p| p.affine.inverse());
        let parent_rotation = parent.map_or(Quat::IDENTITY, |p| p.rotation.inverse());
        let position = |i: usize| Vec3::from(transforms[i].affine.translation);
        let upper = model_to_parent.transform_vector3(position(limb.middle) - position(limb.upper));
        let lower = model_to_parent.transform_vector3(position(limb.tip) - position(limb.middle));
        let first = upper.try_normalize()?;
        lower.try_normalize()?;
        let aim = (upper + lower).try_normalize().unwrap_or(first);
        let perpendicular = |v: Vec3| v - aim * aim.dot(v);
        let bent = perpendicular(first);
        let bend = if bent.length_squared() > 1e-8 {
            bent.normalize()
        } else {
            // A fully extended limb has no bend plane. Recover the imported
            // elbow/knee direction and carry it through the current shoulder/hip rotation.
            let rest = self.transforms(&Rotations::new());
            let rest_parent = self.nodes[limb.upper].parent.map(|i| &rest[i]);
            let inverse = rest_parent.map_or(Affine3A::IDENTITY, |p| p.affine.inverse());
            let offset = |i: usize| {
                inverse.transform_vector3(
                    (rest[i].affine.translation - rest[limb.upper].affine.translation).into(),
                )
            };
            let rest_aim = offset(limb.tip).try_normalize().unwrap_or(first);
            let rest_upper = offset(limb.middle).try_normalize()?;
            let rest_bend = rest_upper - rest_aim * rest_aim.dot(rest_upper);
            let anatomical = match limb.kind {
                Effector::Hand => Vec3::NEG_Z,
                Effector::Foot => Vec3::Z,
            };
            let hint = if rest_bend.length_squared() > 1e-8 {
                rest_bend
            } else {
                inverse.transform_vector3(anatomical)
            };
            let rest_rotation = self.nodes[limb.upper].rest.rotation;
            let turn = parent_rotation * transforms[limb.upper].rotation * rest_rotation.inverse();
            perpendicular(turn * hint)
                .try_normalize()
                .unwrap_or_else(|| aim.any_orthonormal_vector())
        };
        let joints = [limb.upper, limb.middle, limb.tip].map(|i| {
            let node = &self.nodes[i];
            JointRotation {
                key: node.key.clone(),
                basis: (parent_rotation
                    * node
                        .parent
                        .map_or(Quat::IDENTITY, |p| transforms[p].rotation)
                    * node.rest.rotation)
                    .normalize(),
                initial: rotations
                    .get(&node.key)
                    .map_or(Quat::IDENTITY, |q| Quat::from_array(*q)),
            }
        });
        Some(Reach {
            model_to_parent,
            upper,
            lower,
            aim,
            bend,
            joints,
        })
    }
}

impl Reach {
    pub(in crate::editor) fn apply(&self, model_delta: Vec3, rotations: &mut Rotations) {
        if model_delta == Vec3::ZERO {
            return;
        }
        let target = self.upper + self.lower + self.model_to_parent.transform_vector3(model_delta);
        if !target.is_finite() {
            return;
        }
        let direction = target.try_normalize().unwrap_or(self.aim);
        let a = self.upper.length();
        let b = self.lower.length();
        let distance = target
            .length()
            .clamp((a - b).abs().max((a + b) * 1e-6), a + b);
        let along = (0.5 * (distance + (a - b) * (a + b) / distance)).clamp(-a, a);
        let height = ((a - along) * (a + along)).max(0.0).sqrt();

        // Transport the initial bend plane with the target. At a half turn
        // choose the saved bend direction as the otherwise ambiguous swing axis.
        let swing = if self.aim.dot(direction) < -0.9999 {
            Quat::from_rotation_arc(-self.aim, direction)
                * Quat::from_axis_angle(self.bend, std::f32::consts::PI)
        } else {
            Quat::from_rotation_arc(self.aim, direction)
        };
        let bend = swing * self.bend;
        let normal = direction.cross(bend).normalize();
        let elbow = direction * along + bend * height;
        let turn = |start: Vec3, end: Vec3| {
            let start = swing * start;
            let angle = normal.dot(start.cross(end)).atan2(start.dot(end));
            (Quat::from_axis_angle(normal, angle) * swing).normalize()
        };
        let upper = turn(self.upper, elbow);
        let lower = turn(self.lower, direction * distance - elbow);
        self.joints[0].write(Quat::IDENTITY, upper, rotations);
        self.joints[1].write(upper, lower, rotations);
        // Keep the wrist or sole's orientation as it was at the start of the drag.
        self.joints[2].write(lower, Quat::IDENTITY, rotations);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig(names: [&str; 3], parent: Transform, bend: Quat) -> Rig {
        let nodes = [
            ("armature", None, parent, false),
            (names[0], Some(0), Transform::IDENTITY, true),
            ("twist helper", Some(1), Transform::IDENTITY, false),
            (
                names[1],
                Some(2),
                Transform::from_translation(Vec3::X).with_rotation(bend),
                true,
            ),
            (
                names[2],
                Some(3),
                Transform::from_translation(Vec3::X * 0.8),
                true,
            ),
            ("finger_01", Some(4), Transform::from_xyz(0.1, 0., 0.), true),
        ]
        .into_iter()
        .map(|(name, parent, rest, joint)| Node {
            entity: Entity::PLACEHOLDER,
            name: name.into(),
            key: name.into(),
            parent,
            rest,
            joint,
        })
        .collect();
        Rig::from_nodes(nodes)
    }

    #[test]
    fn reach_keeps_link_lengths_tip_orientation_and_existing_pose() {
        let parent = Transform::from_xyz(2., -3., 0.8)
            .with_rotation(Quat::from_rotation_y(0.73))
            .with_scale(Vec3::new(-0.7, 1.3, 1.8));
        let rig = rig(
            [
                "mixamorig:LeftArm",
                "mixamorig:LeftForeArm",
                "mixamorig:LeftHand",
            ],
            parent,
            Quat::from_rotation_y(0.14),
        );
        let initial = Rotations::from([
            (
                "mixamorig:LeftArm".into(),
                Quat::from_rotation_z(0.3).to_array(),
            ),
            (
                "mixamorig:LeftHand".into(),
                Quat::from_rotation_x(0.7).to_array(),
            ),
            ("finger_01".into(), Quat::from_rotation_y(0.4).to_array()),
        ]);
        let reach = rig.reach("mixamorig:LeftHand", &initial).unwrap();
        let before = rig.transforms(&initial);
        let old_tip = Vec3::from(before[4].affine.translation);
        let goal = parent.transform_point(Vec3::new(0.9, 0.4, -0.7));
        let mut pose = initial.clone();
        reach.apply(goal - old_tip, &mut pose);
        let after = rig.transforms(&pose);
        assert!((Vec3::from(after[4].affine.translation) - goal).length() < 1e-4);
        assert!(after[4].rotation.dot(before[4].rotation).abs() > 0.99999);
        assert_eq!(pose["finger_01"], initial["finger_01"]);
        let local = |i: usize| {
            parent
                .compute_affine()
                .inverse()
                .transform_point3(after[i].affine.translation.into())
        };
        assert!((local(1).distance(local(3)) - 1.0).abs() < 1e-4);
        assert!((local(3).distance(local(4)) - 0.8).abs() < 1e-4);
        assert!(valid(&pose));
    }

    #[test]
    fn straight_limbs_bend_consistently_and_stop_at_their_reach_limits() {
        for (names, sign) in [
            (["upperarm_l", "lowerarm_l", "hand_l"], -1.0),
            (["thigh_r", "calf_r", "foot_r"], 1.0),
        ] {
            let rig = rig(names, Transform::IDENTITY, Quat::IDENTITY);
            let reach = rig.reach(names[2], &Rotations::new()).unwrap();
            assert!(rig.reach("finger_01", &Rotations::new()).is_none());
            let mut clean = Rotations::new();
            reach.apply(Vec3::ZERO, &mut clean);
            assert!(clean.is_empty());
            for (target, expected) in [(1.2, 1.2), (4., 1.8), (-4., -1.8), (0., 0.2)] {
                let mut pose = Rotations::new();
                reach.apply((target - 1.8) * Vec3::X, &mut pose);
                let transforms = rig.transforms(&pose);
                let tip = Vec3::from(transforms[4].affine.translation);
                assert!(
                    tip.distance(expected * Vec3::X) < 1e-3,
                    "{tip:?} for {target}"
                );
                if target == 1.2 {
                    assert!(transforms[3].affine.translation.z * sign > 0.2);
                }
                assert!(valid(&pose));
            }
        }
    }
}
