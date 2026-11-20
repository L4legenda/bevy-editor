use super::*;
use crate::editor::model_mesh::KnifeSegment;

pub(super) struct Projection {
    pub rect: Rect,
    pub pose: Transform,
    pub orthographic: bool,
    pub distance: f32,
    triangles: Vec<ProjectedTriangle>,
}

struct ProjectedTriangle {
    object: usize,
    face: usize,
    screen: [Vec2; 3],
    world: [Vec3; 3],
    scores: [f32; 3],
}

struct Span<'a> {
    triangle: &'a ProjectedTriangle,
    start: f32,
    end: f32,
    weights: [Vec3; 2],
    score: [f32; 2],
}

impl Projection {
    pub fn new(bvh: &Bvh, viewer: &Viewer, rect: Rect) -> Self {
        let pose = view_pose(viewer);
        let triangles = bvh
            .triangles()
            .iter()
            .filter_map(|triangle| {
                let [a, b, c] = triangle.points.map(|p| project(viewer, rect, p));
                let screen = [a?, b?, c?];
                let depth = triangle
                    .points
                    .map(|p| (p - pose.translation).dot(*pose.forward()));
                let scores = if viewer.orthographic {
                    depth.map(|d| -d)
                } else {
                    depth.map(f32::recip)
                };
                Some(ProjectedTriangle {
                    object: triangle.object,
                    face: triangle.face,
                    screen,
                    world: triangle.points,
                    scores,
                })
            })
            .collect();
        Self {
            rect,
            pose,
            orthographic: viewer.orthographic,
            distance: viewer.distance,
            triangles,
        }
    }

    pub fn matches(&self, viewer: &Viewer, rect: Rect) -> bool {
        self.rect == rect
            && self.pose == view_pose(viewer)
            && self.orthographic == viewer.orthographic
            && self.distance == viewer.distance
    }

    pub fn trace(
        &self,
        object: &Object,
        from: Vec2,
        to: Vec2,
        through: bool,
        selected: Option<&BTreeSet<usize>>,
    ) -> Vec<KnifeSegment> {
        if from.distance_squared(to) < 0.0001 {
            return Vec::new();
        }
        let accepts = |span: &Span| {
            span.triangle.object == object.id
                && selected.is_none_or(|selected| selected.contains(&span.triangle.face))
        };
        let mut spans: Vec<_> = self
            .triangles
            .iter()
            .filter_map(|triangle| triangle.intersect(from, to))
            .collect();
        spans.sort_by(|a, b| a.start.total_cmp(&b.start));
        let inverse = object.transform.to_matrix().inverse();
        let mut result = BTreeMap::<usize, Vec<KnifeSegment>>::new();
        let mut append = |span: &Span, start: f32, end: f32| {
            if end - start <= 1.0e-5 {
                return;
            }
            let segment = KnifeSegment {
                face: span.triangle.face,
                ends: [start, end]
                    .map(|t| inverse.transform_point3(span.point(t, self.orthographic))),
            };
            let path = result.entry(segment.face).or_default();
            if let Some(last) = path.last_mut() {
                let a = last.ends[1] - last.ends[0];
                let b = segment.ends[1] - segment.ends[0];
                let scale = a.length().max(b.length()).max(0.001);
                if last.ends[1].abs_diff_eq(segment.ends[0], scale * 1.0e-4)
                    && a.normalize_or_zero().dot(b.normalize_or_zero()) > 0.99999
                {
                    last.ends[1] = segment.ends[1];
                    return;
                }
            }
            path.push(segment);
        };
        if through {
            for span in spans.iter().filter(|span| accepts(span)) {
                append(span, span.start, span.end);
            }
        } else if spans.iter().any(&accepts) {
            let mut boundaries: Vec<_> = spans.iter().flat_map(|s| [s.start, s.end]).collect();
            for (i, a) in spans.iter().enumerate() {
                for b in &spans[i + 1..] {
                    if b.start >= a.end {
                        break;
                    }
                    let start = b.start;
                    let end = a.end.min(b.end);
                    let at_start = a.depth_score(start) - b.depth_score(start);
                    let at_end = a.depth_score(end) - b.depth_score(end);
                    if at_start * at_end < 0.0 {
                        boundaries.push(start + (end - start) * at_start / (at_start - at_end));
                    }
                }
            }
            boundaries.sort_by(f32::total_cmp);
            boundaries.dedup_by(|a, b| (*a - *b).abs() <= 1.0e-6);
            let mut pending = 0;
            let mut active: Vec<&Span> = Vec::new();
            for interval in boundaries.windows(2) {
                let (start, end) = (interval[0], interval[1]);
                let t = (start + end) * 0.5;
                while pending < spans.len() && spans[pending].start <= t {
                    active.push(&spans[pending]);
                    pending += 1;
                }
                active.retain(|span| span.end >= t);
                if let Some(span) = active
                    .iter()
                    .copied()
                    .max_by(|a, b| a.depth_score(t).total_cmp(&b.depth_score(t)))
                    && accepts(span)
                {
                    append(span, start, end);
                }
            }
        }
        result.into_values().flatten().collect()
    }
}

impl ProjectedTriangle {
    fn intersect(&self, from: Vec2, to: Vec2) -> Option<Span<'_>> {
        let [a, b, c] = self.screen;
        let area = (b - a).perp_dot(c - a);
        if area.abs() < 0.00001 || !area.is_finite() {
            return None;
        }
        let weights = [from, to].map(|p| {
            let v = (p - a).perp_dot(c - a) / area;
            let w = (b - a).perp_dot(p - a) / area;
            Vec3::new(1.0 - v - w, v, w)
        });
        let [mut start, mut end] = [0.0_f32, 1.0];
        for i in 0..3 {
            let a = weights[0][i];
            let b = weights[1][i];
            if a < 0.0 && b < 0.0 {
                return None;
            }
            if a < 0.0 {
                start = start.max(a / (a - b));
            } else if b < 0.0 {
                end = end.min(a / (a - b));
            }
        }
        if end - start <= 1.0e-6 {
            return None;
        }
        let score = weights.map(|w| w.dot(Vec3::from_array(self.scores)));
        Some(Span {
            triangle: self,
            start,
            end,
            weights,
            score,
        })
    }
}

impl Span<'_> {
    fn depth_score(&self, t: f32) -> f32 {
        self.score[0].lerp(self.score[1], t)
    }

    fn point(&self, t: f32, orthographic: bool) -> Vec3 {
        let mut weights = self.weights[0].lerp(self.weights[1], t);
        if !orthographic {
            weights *= Vec3::from_array(self.triangle.scores);
            weights /= weights.element_sum();
        }
        let [a, b, c] = self.triangle.world;
        a * weights.x + b * weights.y + c * weights.z
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_mesh::{Corner, Face, Geometry};

    #[test]
    fn knife_projection_clips_segment_and_filters_depth_and_selection() {
        let positions: Vec<_> = [1.0, 0.0]
            .into_iter()
            .flat_map(|z| {
                [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                    .map(|(x, y)| Vec3::new(x, y, z))
            })
            .collect();
        let faces = [0..4, 4..8]
            .map(|vertices| Face {
                corners: vertices.map(Corner::new).collect(),
                material: 0,
                smooth: false,
            })
            .to_vec();
        let object = Object {
            id: 0,
            source_node: None,
            name: "Грани".into(),
            geometry: Geometry {
                positions,
                faces,
                ..default()
            }
            .into(),
            transform: Transform::from_scale(Vec3::new(1.3, 0.8, 1.0)),
            materials: Vec::new(),
            hidden: false,
        };
        let bvh = Bvh::from_objects(std::slice::from_ref(&object));
        let rect = Rect::from_center_size(Vec2::splat(300.0), Vec2::splat(600.0));
        for orthographic in [true, false] {
            let viewer = Viewer {
                orthographic,
                radius: 2.0,
                distance: 4.0,
                ..default()
            };
            let projection = Projection::new(&bvh, &viewer, rect);
            let start = rect.center() - Vec2::X * 20.0;
            let end = rect.center() + Vec2::X * 20.0;
            let visible = projection.trace(&object, start, end, false, None);
            assert_eq!(
                visible.len(),
                1,
                "orthographic={orthographic} visible={visible:?}"
            );
            assert_eq!(visible[0].face, 0);
            assert!(visible[0].ends.iter().all(|p| p.x.abs() < 0.5));
            let through = projection.trace(&object, start, end, true, None);
            assert_eq!(
                through.iter().map(|cut| cut.face).collect::<BTreeSet<_>>(),
                [0, 1].into()
            );
            let selection = [1].into();
            assert!(
                projection
                    .trace(&object, start, end, false, Some(&selection))
                    .is_empty()
            );
            assert!(
                projection
                    .trace(&object, start, end, true, Some(&selection))
                    .iter()
                    .all(|cut| cut.face == 1)
            );
        }
    }
}
