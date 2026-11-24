//! Directional and local masks evaluated on the same projected surface as the mesh maps.
use super::{Bounds, PickTriangle, RaySurface, Rays, byte, hash, project};
use crate::editor::model_paint_data::custom::{Kind, Recipe};
use bevy::prelude::*;

pub(super) struct Prepared {
    kind: Kind,
    direction: Vec3,
    cos_spread: f32,
    distance: f32,
    point: Vec3,
    strength: f32,
    wander: f32,
}

#[path = "model_paint_bake_flow.rs"]
mod flow;

pub(super) enum FlowNormals<'a> {
    Mesh(&'a [[Vec3; 3]]),
    High(&'a project::Projection),
}

impl FlowNormals<'_> {
    pub fn corners(&self, triangle: usize) -> [Vec3; 3] {
        match self {
            Self::Mesh(normals) => normals[triangle],
            Self::High(projector) => projector.corner_normals(triangle),
        }
    }
}

pub(super) struct GpuRecipe {
    pub kind: u32,
    pub direction: Vec3,
    pub distance: f32,
    pub point: Vec3,
    pub cosine: f32,
    pub strength: f32,
    pub wander: f32,
}

impl Prepared {
    pub fn new(recipe: &Recipe, bounds: Bounds) -> Self {
        let unit = bounds.custom_unit();
        Self {
            kind: recipe.kind,
            direction: recipe.direction(),
            cos_spread: f32::from(recipe.spread).to_radians().cos(),
            distance: unit * f32::from(recipe.distance) / 100.,
            point: recipe.point_at(bounds.center(), unit),
            strength: f32::from(recipe.strength) / 100.,
            wander: f32::from(recipe.wander).to_radians(),
        }
    }

    pub fn gpu_parameters(&self) -> GpuRecipe {
        let kind = match self.kind {
            Kind::Flow => 1,
            Kind::Exposure => 2,
            Kind::Deposit => 3,
            Kind::Contact => 4,
            Kind::Point => 5,
        };
        GpuRecipe {
            kind,
            direction: self.direction,
            distance: self.distance,
            point: self.point,
            cosine: self.cos_spread,
            strength: self.strength,
            wander: self.wander,
        }
    }

    pub fn is_flow(&self) -> bool {
        self.kind == Kind::Flow
    }

    pub fn sample(
        &self,
        rays: &Rays<'_>,
        surface: RaySurface,
        width: f32,
        different_part: &impl Fn(&PickTriangle) -> bool,
    ) -> u8 {
        let point = surface.point;
        if self.kind == Kind::Point {
            return byte(1. - smooth(point.distance(self.point) / self.distance));
        }
        let normal = surface.normal;
        let ground = smooth((normal.y - 0.25) / 0.5);
        if self.kind == Kind::Deposit && ground == 0. {
            return 0;
        }
        let normalized_bias = (width * 0.125 / rays.scale).clamp(1e-5, 4e-3);
        if self.kind == Kind::Flow {
            return byte(self.flow(rays, surface, normalized_bias * rays.scale));
        }
        let origin = point + normal * (normalized_bias * rays.scale);
        let count = rays.sampling.sky.max(8);
        let point_seed = (point / rays.scale * 4096.)
            .to_array()
            .map(|v| v.round() as i32 as u32);
        let seed = hash(point_seed[0] ^ hash(point_seed[1]) ^ hash(point_seed[2]));
        let offset = (
            (seed >> 8) as f32 / 16777216.,
            (hash(seed) >> 8) as f32 / 16777216.,
        );
        let trace = |direction, reach, accepts: &dyn Fn(&PickTriangle) -> bool| {
            rays.scene
                .cast_triangle_up_to(origin, direction, reach, accepts)
        };
        let same_face = |triangle: &PickTriangle| {
            triangle.object != surface.object || triangle.face != surface.face
        };
        let excludes_part =
            |triangle: &PickTriangle| same_face(triangle) && different_part(triangle);
        let mut result = 0.;
        let mut nearest = self.distance;
        for index in 0..count {
            let (axis, cosine) = if self.kind == Kind::Contact {
                (normal, 0.)
            } else {
                (self.direction, self.cos_spread)
            };
            let mut direction = cone(axis, cosine, index, count, offset);
            match self.kind {
                Kind::Exposure => {
                    let facing = direction.dot(normal);
                    if facing > 0. && trace(direction, self.distance, &same_face).is_none() {
                        result += facing;
                    }
                }
                Kind::Deposit => {
                    let facing = direction.dot(normal);
                    if facing < 0. {
                        direction -= 2. * facing * normal;
                    }
                    if trace(direction, self.distance, &same_face).is_some() {
                        result += ground;
                    }
                }
                Kind::Contact => {
                    if let Some((_, distance)) = trace(direction, nearest, &excludes_part) {
                        nearest = distance;
                    }
                }
                Kind::Flow | Kind::Point => unreachable!(),
            }
        }
        if self.kind == Kind::Contact {
            byte((1. - nearest / self.distance).powi(2))
        } else {
            byte(result / count as f32)
        }
    }
}

fn smooth(value: f32) -> f32 {
    let t = value.clamp(0., 1.);
    t * t * (3. - 2. * t)
}

fn cone(axis: Vec3, cosine: f32, index: u32, count: u32, offset: (f32, f32)) -> Vec3 {
    let u = (index as f32 / count as f32 + offset.0).fract();
    let v = (index.reverse_bits() as f32 * (1. / 4294967296.) + offset.1).fract();
    let axial = 1. - u * (1. - cosine);
    let radius = (1. - axial * axial).max(0.).sqrt();
    let (sin, cos) = (v * std::f32::consts::TAU).sin_cos();
    let tangent = axis.any_orthonormal_vector();
    axis * axial + tangent * (radius * cos) + axis.cross(tangent) * (radius * sin)
}
