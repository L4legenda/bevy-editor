//! Retargetable parameter animation; the exported asset always uses the input values.
use crate::editor::model_primitives::{Parameter, Parameters, Primitive};

const DURATION: f32 = 0.24;

pub(super) struct Transition {
    pub kind: Primitive,
    from: Parameters,
    current: Parameters,
    target: Parameters,
    elapsed: f32,
}

impl Transition {
    pub fn new(kind: Primitive, parameters: Parameters) -> Self {
        Self {
            kind,
            from: parameters,
            current: parameters,
            target: parameters,
            elapsed: DURATION,
        }
    }

    pub fn retarget(&mut self, target: Parameters) {
        if self.target != target {
            self.from = self.current;
            self.target = target;
            self.elapsed = 0.0;
        }
    }

    pub fn advance(&mut self, delta: f32) -> Parameters {
        self.elapsed = (self.elapsed + delta.max(0.0).min(0.05)).min(DURATION);
        if self.elapsed >= DURATION {
            self.current = self.target;
            return self.current;
        }
        let t = self.elapsed / DURATION;
        let eased = t * t * (3.0 - 2.0 * t);
        let mut parameters = self.from;
        for field in Parameter::ALL {
            let start = self.from.get(field);
            let value = start + (self.target.get(field) - start) * eased;
            parameters.set(
                field,
                if field.integer() {
                    value.round()
                } else {
                    value
                },
            );
        }
        // Both endpoints can be valid while their intermediate segment product
        // exceeds the mesh budget (e.g. 256×64 -> 64×256).
        if matches!(self.kind, Primitive::UvSphere | Primitive::Torus) {
            parameters.rings = parameters
                .rings
                .min(Parameters::MAX_CELLS / parameters.segments);
        }
        self.current = parameters;
        parameters
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;

    #[test]
    fn primitive_preview_animates_between_dimensions_then_stops_exactly() {
        let start = Primitive::Cube.defaults();
        let target = Parameters {
            size: Vec3::new(4.0, 3.0, 1.0),
            ..start
        };
        let mut transition = Transition::new(Primitive::Cube, start);
        assert_eq!(transition.advance(0.0), start);
        transition.retarget(target);
        assert_eq!(transition.advance(0.0), start);
        let middle = transition.advance(DURATION / 6.0);
        assert!(middle.size.x > start.size.x && middle.size.x < target.size.x);
        assert!(middle.size.y > start.size.y && middle.size.y < target.size.y);
        assert!(middle.size.z < start.size.z && middle.size.z > target.size.z);
        for _ in 0..30 {
            transition.advance(1.0 / 60.0);
        }
        assert_eq!(transition.advance(0.0), target);
        assert_eq!(transition.advance(1.0 / 60.0), target);
    }

    #[test]
    fn primitive_preview_retargets_from_the_visible_shape_without_a_jump() {
        let start = Primitive::Cone.defaults();
        let mut transition = Transition::new(Primitive::Cone, start);
        transition.retarget(Parameters {
            radius: 4.0,
            top_radius: 2.0,
            ..start
        });
        for _ in 0..5 {
            transition.advance(1.0 / 60.0);
        }
        let visible = transition.advance(0.0);
        let target = Parameters {
            radius: 0.5,
            top_radius: 0.0,
            ..start
        };
        transition.retarget(target);
        assert_eq!(transition.advance(0.0), visible);
        let next = transition.advance(1.0 / 60.0);
        assert!(next.radius < visible.radius && next.radius > target.radius);
        assert!((next.radius - visible.radius).abs() < 0.1);
        for _ in 0..30 {
            transition.advance(1.0 / 60.0);
        }
        assert_eq!(transition.advance(0.0), target);
    }

    #[test]
    fn primitive_preview_topology_animation_preserves_parameter_constraints() {
        for kind in [Primitive::UvSphere, Primitive::Torus] {
            let start = Parameters {
                segments: 256,
                rings: 64,
                ..kind.defaults()
            };
            let target = Parameters {
                segments: 64,
                rings: 256,
                major_radius: 0.5,
                minor_radius: 0.4,
                ..start
            };
            let mut transition = Transition::new(kind, start);
            transition.retarget(target);
            for _ in 0..30 {
                transition.advance(1.0 / 60.0).validate(kind).unwrap();
            }
            assert_eq!(transition.advance(0.0), target);
        }
    }
}
