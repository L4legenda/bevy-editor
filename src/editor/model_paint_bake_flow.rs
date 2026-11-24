//! Upstream surface walks for fluid-flow maps, following Material Painter's bake shader.
use super::{FlowNormals, Prepared, RaySurface, Rays, cone, hash};
use bevy::prelude::*;

const STEPS: u32 = 32;

#[derive(Clone, Copy)]
struct Landing {
    point: Vec3,
    normal: Vec3,
}

impl Prepared {
    pub(super) fn flow(&self, rays: &Rays<'_>, surface: RaySurface, bias: f32) -> f32 {
        let walks = (rays.sampling.sky / 8).clamp(2, 8);
        let samples = walks * STEPS;
        let step = self.distance / STEPS as f32;
        let frequency = 6. / self.distance.max(1e-6);
        let seed = cell_hash(surface.point / (rays.scale * 2. / rays.resolution as f32));
        let a = hash(seed ^ 0x3c6ef372);
        let b = hash(a ^ 0x9e3779b9);
        let offset = (fraction(a), fraction(b));
        let mut total = 0.;
        for walk in 0..walks {
            let mut current = Landing {
                point: surface.point,
                normal: surface.normal,
            };
            let mut heading = Vec3::ZERO;
            let mut gathered = 0.;
            let noise_offset = Vec3::new(17.13, 5.71, 11.37) * walk as f32;
            for index in 0..STEPS {
                let direction = cone(
                    self.direction,
                    self.cos_spread,
                    walk * STEPS + index,
                    samples,
                    ((offset.0 + 0.5 / samples as f32).fract(), offset.1),
                );
                let facing = current.normal.dot(direction);
                let covered = facing > 0.
                    && rays
                        .scene
                        .cast_triangle_up_to(
                            current.point + current.normal * bias,
                            direction,
                            f32::INFINITY,
                            |_| true,
                        )
                        .is_none();
                let wet = if covered { facing } else { 0. };
                gathered += wet;

                let tangent = |v: Vec3| v - current.normal * current.normal.dot(v);
                let mut upstream = tangent(self.direction);
                if upstream.length_squared() < 0.04 {
                    upstream = tangent(heading);
                }
                if upstream.length_squared() < 1e-8 {
                    // A face looking straight into the flow has no upstream slope.
                    gathered += wet * (STEPS - 1 - index) as f32;
                    break;
                }
                let mut upstream = upstream.normalize();
                if self.wander > 0. {
                    let angle =
                        (noise(current.point * frequency + noise_offset) - 0.5) * 2. * self.wander;
                    let (sin, cos) = angle.sin_cos();
                    upstream = upstream * cos + current.normal.cross(upstream) * sin;
                }
                heading = upstream;
                let Some(next) = surface_step(rays, current, upstream, step) else {
                    break;
                };
                current = next;
            }
            total += 1. - (-self.strength * gathered / STEPS as f32).exp();
        }
        total / walks as f32
    }
}

fn surface_step(rays: &Rays<'_>, surface: Landing, tangent: Vec3, length: f32) -> Option<Landing> {
    let lift = length * 0.25;
    let start = surface.point + surface.normal * lift;
    let forward = start + tangent * length;
    let down = lift + length * 2.;
    // Meet a concave wall, follow the current face, or turn around a convex edge.
    land(rays, start, tangent, length)
        .or_else(|| land(rays, forward, -surface.normal, down))
        .or_else(|| land(rays, forward - surface.normal * down, -tangent, length * 2.))
}

fn land(rays: &Rays<'_>, origin: Vec3, direction: Vec3, limit: f32) -> Option<Landing> {
    let (index, distance) = rays
        .scene
        .cast_triangle_up_to(origin, direction, limit, |_| true)?;
    let point = origin + direction * distance;
    let normal = match &rays.flow_normals {
        Some(FlowNormals::High(projector)) => projector.surface_normal(index, point),
        normals => {
            let [a, b, c] = rays.scene.triangle(index).points;
            let cross = (b - a).cross(c - a);
            let face = cross.try_normalize().unwrap_or(-direction);
            match normals {
                Some(FlowNormals::Mesh(normals)) => {
                    let [na, nb, nc] = normals[index];
                    let v = (point - a).cross(c - a).dot(cross) / cross.length_squared();
                    let w = (b - a).cross(point - a).dot(cross) / cross.length_squared();
                    (na * (1. - v - w) + nb * v + nc * w)
                        .try_normalize()
                        .unwrap_or(face)
                }
                _ => face,
            }
        }
    };
    let normal = if normal.dot(direction) > 0. {
        -normal
    } else {
        normal
    };
    Some(Landing { point, normal })
}

fn fraction(bits: u32) -> f32 {
    (bits & 0xff_ffff) as f32 / 16777216.
}

fn cell_hash(point: Vec3) -> u32 {
    let [x, y, z] = point.floor().to_array().map(|v| v as i32 as u32);
    hash(x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663) ^ z.wrapping_mul(83492791))
}

fn noise(point: Vec3) -> f32 {
    let cell = point.floor();
    let f = point - cell;
    let [x, y, z] = (f * f * (Vec3::splat(3.) - 2. * f)).to_array();
    let mix = |a, b, t| a * (1. - t) + b * t;
    let sample = |x, y, z| fraction(cell_hash(cell + Vec3::new(x, y, z)));
    let plane = |z| {
        mix(
            mix(sample(0., 0., z), sample(1., 0., z), x),
            mix(sample(0., 1., z), sample(1., 1., z), x),
            y,
        )
    };
    mix(plane(0.), plane(1.), z)
}

#[cfg(test)]
mod tests {
    use super::super::super::super::Quality;
    use super::super::super::{Bvh, PickTriangle};
    use super::*;

    #[test]
    fn baked_flow_walk_crosses_folds_and_stops_at_open_edges() {
        let floor = PickTriangle {
            object: 0,
            face: 0,
            points: [Vec3::ZERO, Vec3::Z, Vec3::X],
        };
        let trace = |wall: Option<f32>| {
            let triangles = std::iter::once(floor)
                .chain(wall.map(|height| PickTriangle {
                    object: 0,
                    face: 1,
                    points: [Vec3::ZERO, Vec3::X, Vec3::Y * height],
                }))
                .collect();
            let scene = Bvh::from_triangles(triangles);
            let rays = Rays {
                scene: &scene,
                scale: 1.,
                sampling: Quality::Draft.sampling(),
                maps: super::super::super::MapSelection::ALL,
                resolution: 32,
                flow_normals: None,
            };
            surface_step(
                &rays,
                Landing {
                    point: Vec3::new(0.35, 0., 0.05),
                    normal: Vec3::Y,
                },
                Vec3::NEG_Z,
                0.1,
            )
        };
        let concave = trace(Some(1.)).unwrap();
        assert!(concave.point.y > 0.);
        assert!(concave.normal.dot(Vec3::Z) > 0.99);
        let convex = trace(Some(-1.)).unwrap();
        assert!(convex.point.y < 0.);
        assert!(convex.normal.dot(Vec3::NEG_Z) > 0.99);
        assert!(trace(None).is_none());
    }
}
