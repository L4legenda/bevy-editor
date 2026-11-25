//! Screen-space rectangle and lasso input for polygon fill.
use super::Brush;
use bevy::prelude::*;

pub(super) const MAX_POINTS: usize = 384;

pub(super) struct Drag {
    pub brush: Brush,
    pub layer: usize,
    pub erase: bool,
    pub start: Vec2,
    end: Vec2,
    lasso: Option<Vec<Vec2>>,
    through: bool,
}

#[derive(Clone)]
pub(super) struct Region {
    bounds: Rect,
    outline: Vec<Vec2>,
    pub through: bool,
}

impl Drag {
    pub fn new(
        brush: Brush,
        layer: usize,
        start: Vec2,
        lasso: bool,
        through: bool,
        erase: bool,
    ) -> Self {
        Self {
            brush,
            layer,
            erase,
            start,
            end: start,
            lasso: lasso.then(|| vec![start]),
            through,
        }
    }

    pub fn advance(&mut self, point: Vec2, viewport: Rect) {
        self.end = point.clamp(viewport.min, viewport.max);
        if let Some(points) = &mut self.lasso
            && points
                .last()
                .is_none_or(|last| last.distance(self.end) >= 3.)
        {
            if points.len() == MAX_POINTS {
                *points = points.iter().step_by(2).copied().collect();
            }
            points.push(self.end);
        }
    }

    pub fn region(&self) -> Option<Region> {
        let points = self.lasso.as_deref().unwrap_or_default();
        let did_drag = points
            .iter()
            .chain(std::iter::once(&self.end))
            .any(|&point| crate::editor::area_selection::dragged(self.start, point));
        if !did_drag {
            return None;
        }
        let outline: Vec<_> = if points.len() >= 3 {
            points
                .iter()
                .copied()
                .chain(std::iter::once(self.end))
                .collect()
        } else {
            Vec::new()
        };
        Some(if outline.is_empty() {
            Region::rectangle(self.start, self.end, self.through)
        } else {
            Region::lasso(outline, self.through)
        })
    }
}

impl Region {
    pub(super) fn rectangle(start: Vec2, end: Vec2, through: bool) -> Self {
        Self { bounds: Rect::from_corners(start, end), outline: Vec::new(), through }
    }

    pub(super) fn lasso(outline: Vec<Vec2>, through: bool) -> Self {
        let bounds = outline.iter().fold(Rect::from_corners(outline[0], outline[0]), |bounds, &point| Rect {
            min: bounds.min.min(point), max: bounds.max.max(point),
        });
        Self { bounds, outline, through }
    }

    pub fn contains(&self, point: Vec2) -> bool {
        if !self.bounds.contains(point) {
            return false;
        }
        if self.outline.is_empty() {
            return true;
        }
        self.edges().fold(false, |inside, [a, b]| {
            inside
                ^ ((a.y > point.y) != (b.y > point.y)
                    && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x)
        })
    }

    pub fn edges(&self) -> impl Iterator<Item = [Vec2; 2]> + '_ {
        let Rect { min, max } = self.bounds;
        let corners = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)];
        // Own the four rectangle corners so this iterator can also borrow a lasso.
        let count = if self.outline.is_empty() {
            corners.len()
        } else {
            self.outline.len()
        };
        (0..count).map(move |i| {
            let points = if self.outline.is_empty() {
                &corners[..]
            } else {
                &self.outline
            };
            [points[i], points[(i + 1) % count]]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_fill_lasso_contains_concave_region_without_its_bounding_box() {
        let outline = [(0., 0.), (6., 0.), (6., 2.), (2., 2.), (2., 6.), (0., 6.)]
            .map(|(x, y)| Vec2::new(x, y));
        let region = Region {
            bounds: Rect::from_corners(Vec2::ZERO, Vec2::splat(6.)),
            outline: outline.to_vec(),
            through: false,
        };
        assert!(region.contains(Vec2::new(1., 5.)));
        assert!(region.contains(Vec2::new(5., 1.)));
        assert!(!region.contains(Vec2::new(5., 5.)));
        assert_eq!(region.edges().last().unwrap(), [outline[5], outline[0]]);
    }
}
