//! Resample pointer positions into pressure-aware stamps at a fixed spacing.
use bevy::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Stamp {
    pub point: Vec2,
    pub radius: f32,
    pub flow: f32,
    pub heading: Option<f32>,
}

impl From<Vec2> for Stamp {
    fn from(point: Vec2) -> Self {
        Self {
            point,
            radius: 1.,
            flow: 1.,
            heading: None,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Variation {
    pub scatter: f32,
    pub size: f32,
    pub angle: f32,
}

pub(super) struct Noise(u32);

impl Noise {
    pub fn new(stroke: u64) -> Self {
        // Mix consecutive stroke IDs before seeding the per-stamp xorshift.
        let mut seed = stroke.wrapping_add(0x9e3779b97f4a7c15);
        seed = (seed ^ (seed >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        seed = (seed ^ (seed >> 27)).wrapping_mul(0x94d049bb133111eb);
        Self(((seed ^ (seed >> 31)) as u32).max(1))
    }

    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32
    }

    pub fn vary(&mut self, mut stamp: Stamp, radius: f32, variation: Variation) -> (Stamp, f32) {
        if variation.scatter == 0. && variation.size == 0. && variation.angle == 0. {
            return (stamp, 0.);
        }
        stamp.radius *= 1. - variation.size * self.next();
        let offset = Vec2::new(self.next(), self.next()) * 2. - Vec2::ONE;
        stamp.point += offset * (radius * variation.scatter);
        let angle = (self.next() - 0.5) * std::f32::consts::TAU * variation.angle;
        (stamp, angle)
    }
}

pub(super) struct Path {
    pub lazy: Vec2,
    last: Vec2,
    pressure: f32,
    spacing: f32,
    stabilizer: f32,
    pressure_size: bool,
    pressure_flow: bool,
}

impl Path {
    pub fn new(
        point: Vec2,
        pressure: f32,
        spacing: f32,
        stabilizer: f32,
        pressure_size: bool,
        pressure_flow: bool,
    ) -> Self {
        Self {
            lazy: point,
            last: point,
            pressure: pressure.clamp(0.02, 1.),
            spacing,
            stabilizer,
            pressure_size,
            pressure_flow,
        }
    }

    pub fn first(&self) -> Stamp {
        self.stamp(self.last, self.pressure)
    }

    pub fn end(&self) -> Vec2 {
        self.last
    }

    pub fn line_to(&mut self, point: Vec2, pressure: f32) -> Vec<Stamp> {
        self.lazy = point;
        let mut points = self.resample(pressure);
        if self.last.distance_squared(point) > 1e-6 {
            self.last = point;
            self.pressure = pressure.clamp(0.02, 1.);
            points.push(self.first());
        }
        points
    }

    fn stamp(&self, point: Vec2, pressure: f32) -> Stamp {
        Stamp {
            point,
            radius: if self.pressure_size { pressure } else { 1. },
            flow: if self.pressure_flow { pressure } else { 1. },
            heading: None,
        }
    }

    pub fn advance(&mut self, point: Vec2, pressure: f32) -> Vec<Stamp> {
        let pull = point - self.lazy;
        let distance = pull.length();
        if distance > self.stabilizer {
            self.lazy = point - pull * (self.stabilizer / distance);
        }
        self.resample(pressure)
    }

    fn resample(&mut self, pressure: f32) -> Vec<Stamp> {
        let delta = self.lazy - self.last;
        let distance = delta.length();
        let pressure = pressure.clamp(0.02, 1.);
        let diameter = if self.pressure_size {
            pressure.max(0.1)
        } else {
            1.
        };
        let step = (diameter * self.spacing).max(1.);
        if distance < step {
            return Vec::new();
        }
        let count = (distance / step).floor().min(512.) as usize;
        let step = step.max(distance / 512.);
        let last = self.last;
        let initial = self.pressure;
        let mut reached = 0.;
        let points = (1..=count)
            .map(|i| {
                reached = (i as f32 * step / distance).min(1.);
                self.stamp(
                    last + delta * reached,
                    initial + (pressure - initial) * reached,
                )
            })
            .collect();
        self.last += delta * reached;
        self.pressure = initial + (pressure - initial) * reached;
        points
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stabilizer_keeps_spacing_and_interpolates_stamp_pressure() {
        let mut path = Path::new(Vec2::ZERO, 0.25, 4., 10., true, true);
        assert_eq!(path.first().radius, 0.25);
        assert!(path.advance(Vec2::X * 9., 0.75).is_empty());
        let points = path.advance(Vec2::X * 25., 0.75);
        assert!(path.lazy.distance(Vec2::X * 15.) < 0.0001);
        assert_eq!(points.len(), 5);
        for (stamp, x) in points.iter().zip([3., 6., 9., 12., 15.]) {
            assert!((stamp.point.x - x).abs() < 0.0001);
        }
        assert!((points[0].radius - 0.35).abs() < 0.0001);
        assert!((points[4].flow - 0.75).abs() < 0.0001);
        assert!(path.advance(Vec2::X * 25., 0.75).is_empty());
    }

    #[test]
    fn straight_paint_line_reaches_endpoint_despite_stabilizer() {
        let mut path = Path::new(Vec2::ZERO, 1., 4., 100., false, false);
        let points = path.line_to(Vec2::new(0., 10.), 1.);
        let positions: Vec<_> = points.iter().map(|p| p.point).collect();
        assert_eq!(positions, [Vec2::Y * 4., Vec2::Y * 8., Vec2::Y * 10.]);
        assert_eq!(path.end(), Vec2::Y * 10.);
        assert!(path.advance(Vec2::Y * 10., 1.).is_empty());
    }

    #[test]
    fn paint_stamp_variation_preserves_flow_and_respects_each_limit() {
        let variation = Variation {
            scatter: 0.5,
            size: 0.4,
            angle: 0.25,
        };
        let stamp = Stamp {
            radius: 0.6,
            flow: 0.3,
            ..Stamp::from(Vec2::new(100., 100.))
        };
        let mut a = Noise::new(42);
        let mut b = Noise::new(42);
        assert_eq!(a.vary(stamp, 20., Variation::default()), (stamp, 0.));
        for _ in 0..128 {
            let (actual, angle) = a.vary(stamp, 20., variation);
            assert_eq!((actual, angle), b.vary(stamp, 20., variation));
            assert!((0.36..=0.6).contains(&actual.radius));
            assert_eq!(actual.flow, stamp.flow);
            assert!((actual.point - stamp.point).abs().max_element() <= 10.);
            assert!(angle.abs() <= std::f32::consts::FRAC_PI_4);
        }
        assert_ne!(
            Noise::new(42).vary(stamp, 20., variation),
            Noise::new(43).vary(stamp, 20., variation)
        );
    }
}
