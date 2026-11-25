use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum SnapTarget {
    #[default]
    Step,
    Vertex,
    Edge,
    Surface,
}

pub(super) struct GeometryTargets {
    bvh: input::Bvh,
    moving_vertices: BTreeSet<usize>,
}

impl GeometryTargets {
    pub fn new(base: &Snapshot, mode: Mode) -> Self {
        let moving_vertices = base
            .objects
            .iter()
            .find(|o| Some(o.id) == base.active && mode == Mode::Edit)
            .map_or_else(BTreeSet::new, |o| {
                base.selection.selected_vertices(&o.geometry)
            });
        Self {
            bvh: input::Bvh::from_objects(&base.objects),
            moving_vertices,
        }
    }

    pub fn at(
        &self,
        base: &Snapshot,
        mode: Mode,
        target: SnapTarget,
        viewer: &Viewer,
        rect: Rect,
        cursor: Vec2,
    ) -> Option<Vec3> {
        if target == SnapTarget::Step || !rect.contains(cursor) {
            return None;
        }
        let object = |id| base.objects.iter().find(|object| object.id == id);
        let moves_vertex = |id, vertex| {
            mode == Mode::Edit && base.active == Some(id) && self.moving_vertices.contains(&vertex)
        };
        let cast = |point| {
            let (origin, direction) = input::ray(viewer, rect, point);
            self.bvh
                .cast_where(origin, direction, |triangle| {
                    if mode == Mode::Object {
                        return !base.selected.contains(&triangle.object);
                    }
                    target != SnapTarget::Surface
                        || base.active != Some(triangle.object)
                        || object(triangle.object).is_some_and(|o| {
                            o.geometry.faces[triangle.face]
                                .corners
                                .iter()
                                .all(|c| !moves_vertex(triangle.object, c.vertex))
                        })
                })
                .map(|(id, face, depth)| (id, face, origin + direction * depth))
        };
        if target == SnapTarget::Surface {
            return cast(cursor).map(|(_, _, point)| point);
        }

        const RADIUS: f32 = 12.0;
        let mut nearby = BTreeSet::new();
        for x in [-1.0, 0.0, 1.0] {
            for y in [-1.0, 0.0, 1.0] {
                let offset = Vec2::new(x, y) * (RADIUS * 0.7);
                if let Some((id, face, _)) = cast(cursor + offset) {
                    nearby.insert((id, face));
                }
            }
        }
        let pose = view_pose(viewer);
        let mut best = None;
        let mut distance = RADIUS.powi(2);
        let mut consider = |point| {
            if let Some(pixel) = input::project(viewer, rect, point) {
                let error = pixel.distance_squared(cursor);
                if error < distance {
                    distance = error;
                    best = Some(point);
                }
            }
        };
        for (id, face) in nearby {
            let o = object(id)?;
            let face = &o.geometry.faces[face];
            let point = |v| o.transform.transform_point(o.geometry.positions[v]);
            if target == SnapTarget::Vertex {
                for corner in &face.corners {
                    if !moves_vertex(id, corner.vertex) {
                        consider(point(corner.vertex));
                    }
                }
            } else {
                for (a, b) in face.edges() {
                    if moves_vertex(id, a) || moves_vertex(id, b) {
                        continue;
                    }
                    let (a, b) = (point(a), point(b));
                    if let Some((start, end)) =
                        input::project(viewer, rect, a).zip(input::project(viewer, rect, b))
                    {
                        let span = end - start;
                        let t = ((cursor - start).dot(span) / span.length_squared().max(0.00001))
                            .clamp(0.0, 1.0);
                        let weight = if viewer.orthographic {
                            t
                        } else {
                            let da = (a - pose.translation).dot(*pose.forward());
                            let db = (b - pose.translation).dot(*pose.forward());
                            t * da / ((1.0 - t) * db + t * da)
                        };
                        consider(a.lerp(b, weight));
                    }
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_mesh::{Corner, Face};

    #[test]
    fn snapping_uses_stationary_vertices_edges_and_surface_under_the_cursor() {
        let quad = Geometry {
            positions: vec![
                Vec3::new(-1.0, -1.0, 0.0),
                Vec3::new(1.0, -1.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(-1.0, 1.0, 0.0),
            ],
            faces: vec![Face {
                corners: (0..4).map(Corner::new).collect(),
                material: 0,
                smooth: false,
            }],
            ..default()
        };
        let object = |id, z| Object {
            id,
            source_node: None,
            name: format!("Плоскость {id}"),
            geometry: quad.clone().into(),
            transform: Transform::from_xyz(0.0, 0.0, z),
            materials: Vec::new(),
            hidden: false,
        };
        let base = Snapshot {
            paint_set: None,
            mesh_replacement: false,
            bake_settings: default(),
            cage_edits: default(),
            objects: vec![object(0, 1.0), object(1, 0.0), object(2, -1.0)],
            custom_bakes: default(),
            active: Some(0),
            selected: [0].into(),
            selection: default(),
        };
        let viewer = Viewer {
            distance: 5.0,
            radius: 2.0,
            orthographic: true,
            ..default()
        };
        let rect = Rect::from_corners(Vec2::ZERO, Vec2::splat(600.0));
        let targets = GeometryTargets::new(&base, Mode::Object);
        for (kind, expected) in [
            (SnapTarget::Vertex, Vec3::new(1.0, 1.0, 0.0)),
            (SnapTarget::Edge, Vec3::new(0.2, 1.0, 0.0)),
            (SnapTarget::Surface, Vec3::new(0.2, 0.1, 0.0)),
        ] {
            let cursor = input::project(&viewer, rect, expected).unwrap();
            let actual = targets
                .at(&base, Mode::Object, kind, &viewer, rect, cursor)
                .unwrap();
            assert!(actual.abs_diff_eq(expected, 0.0001), "{kind:?}: {actual}");
        }
    }
}
