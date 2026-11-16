//! A grabbed point follows the logical window cursor, independent of raw device motion.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Anchor {
    pub document: usize,
    pub press: Vec2,
    grab: Vec2,
}
impl Anchor {
    pub fn new(
        document: usize,
        press: Vec2,
        local: Vec2,
        pan: Vec2,
        zoom: f32,
        position: Vec2,
        center: Vec2,
        scale: f32,
    ) -> Self {
        let graph = (local - pan) / zoom;
        Self {
            document,
            press,
            grab: center + (graph - position - center) / scale,
        }
    }
    pub fn position(self, local: Vec2, pan: Vec2, zoom: f32, center: Vec2, scale: f32) -> Vec2 {
        (local - pan) / zoom - center - (self.grab - center) * scale
    }
    pub fn moved(self, point: Vec2) -> bool {
        point.distance_squared(self.press) >= 4.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_owns_position_at_every_zoom_and_dpi_without_accumulating_errors() {
        for zoom in [0.35, 0.7, 1., 1.5] {
            for dpi in [1., 1.25, 1.5, 2.] {
                let position = Vec2::new(120., 85.);
                let pan = Vec2::new(-34., 57.);
                let center = Vec2::new(180., 204.);
                let grab = Vec2::new(71., 19.);
                let cursor = (position + grab) * zoom + pan;
                let anchor = Anchor::new(7, cursor, cursor, pan, zoom, position, center, 1.);
                for displacement in [Vec2::new(173., -44.), Vec2::new(-281., 123.), Vec2::ZERO] {
                    let physical = (cursor + displacement) * dpi;
                    let actual = anchor.position(physical / dpi, pan, zoom, center, 1.);
                    assert!(actual.abs_diff_eq(position + displacement / zoom, 0.001));
                }
            }
        }
    }
    #[test]
    fn the_grab_stays_fixed_during_birth_animation_and_view_changes() {
        let position = Vec2::new(400., 180.);
        let center = Vec2::new(180., 204.);
        let grab = Vec2::new(60., 20.);
        let cursor = position + center + (grab - center) * 0.975;
        let anchor = Anchor::new(7, cursor, cursor, Vec2::ZERO, 1., position, center, 0.975);
        let pan = Vec2::new(-90., 30.);
        let zoom = 0.7;
        let point = Vec2::new(480., 211.);
        let moved = anchor.position(point, pan, zoom, center, 1.);
        assert!(((moved + grab) * zoom + pan).abs_diff_eq(point, 0.001));
        assert!(!anchor.moved(cursor + Vec2::ONE * 0.5));
        assert!(anchor.moved(cursor + Vec2::new(3., 0.)));
    }
}
