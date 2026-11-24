//! Click and rectangle picking share the same visibility and X-Ray rules.
use super::*;
use crate::editor::area_selection::{self, Mode as AreaMode};
use crate::editor::model_mesh::ComponentHit;
use input::{project, ray, segment, visible};

pub(super) fn click(
    doc: &mut Document,
    state: &State,
    viewer: &Viewer,
    rect: Rect,
    cursor: Vec2,
    mode: AreaMode,
) {
    let (origin, direction) = ray(viewer, rect, cursor);
    if doc.mode == Mode::Object {
        if state.tool != Tool::Universal {
            return;
        }
        let hit = state.bvh.cast(origin, direction, None).map(|(id, _, _)| id);
        if mode == AreaMode::Subtract {
            if let Some(id) = hit {
                doc.selected.remove(&id);
                if doc.active == Some(id) {
                    doc.active = doc.selected.last().copied();
                }
            }
        } else if let Some(id) = hit {
            doc.select_object(id, mode == AreaMode::Add);
        } else if mode == AreaMode::Replace {
            doc.selected.clear();
        }
        doc.ui_revision += 1;
        return;
    }
    let Some(object) = doc.active() else {
        return;
    };
    let mesh = &object.geometry;
    let (mut vertex, mut edge, mut face) = (None, None, None);
    match doc.selection.kind {
        Element::Face => {
            if state.xray {
                face = mesh
                    .faces
                    .iter()
                    .enumerate()
                    .filter_map(|(i, f)| {
                        let p = object.transform.transform_point(mesh.center(f));
                        let distance = project(viewer, rect, p)?.distance(cursor);
                        (distance < 12.0).then_some((distance, i))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, i)| i);
            }
            face = face.or_else(|| {
                state
                    .bvh
                    .cast(origin, direction, Some(object.id))
                    .map(|(_, f, _)| f)
            });
        }
        Element::Vertex => {
            vertex = mesh
                .positions
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    let world = object.transform.transform_point(*p);
                    let distance = project(viewer, rect, world)?.distance(cursor);
                    (distance < 12.0 && (state.xray || visible(&state.bvh, viewer, rect, world)))
                        .then_some((distance, i))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, i)| i);
        }
        Element::Edge => {
            edge = mesh
                .edges()
                .into_keys()
                .filter_map(|e| {
                    let a = object.transform.transform_point(mesh.positions[e.0]);
                    let b = object.transform.transform_point(mesh.positions[e.1]);
                    let distance =
                        segment(cursor, project(viewer, rect, a)?, project(viewer, rect, b)?);
                    (distance < 10.0
                        && (state.xray || visible(&state.bvh, viewer, rect, (a + b) * 0.5)))
                    .then_some((distance, e))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, e)| e);
        }
    }
    if mode == AreaMode::Replace {
        doc.selection.clear();
    }
    if let Some(hit) = vertex
        .map(ComponentHit::Vertex)
        .or(edge.map(ComponentHit::Edge))
        .or(face.map(ComponentHit::Face))
    {
        doc.selection
            .click(hit, mode == AreaMode::Add, mode == AreaMode::Subtract);
    }
    doc.ui_revision += 1;
}

pub(super) fn select(
    doc: &mut Document,
    state: &State,
    viewer: &Viewer,
    viewport: Rect,
    area: Rect,
    mode: AreaMode,
) {
    if doc.mode == Mode::Object {
        let hits = doc
            .objects
            .iter()
            .filter(|o| !o.hidden)
            .filter(|o| {
                area_selection::projected_bounds(
                    o.geometry
                        .positions
                        .iter()
                        .map(|p| o.transform.transform_point(*p)),
                    |p| project(viewer, viewport, p),
                )
                .is_some_and(|bounds| area_selection::overlaps(area, bounds))
            })
            .map(|o| o.id)
            .collect();
        mode.apply(&mut doc.selected, hits);
        if !doc.active.is_some_and(|id| doc.selected.contains(&id)) {
            doc.active = doc.selected.last().copied().or(doc.active);
        }
    } else if let Some(object) = doc.active() {
        let mut hits = MeshSelection {
            kind: doc.selection.kind,
            ..default()
        };
        let visible = |p| state.xray || visible(&state.bvh, viewer, viewport, p);
        match doc.selection.kind {
            Element::Vertex => {
                for (i, p) in object.geometry.positions.iter().enumerate() {
                    let p = object.transform.transform_point(*p);
                    if project(viewer, viewport, p).is_some_and(|p| area.contains(p)) && visible(p)
                    {
                        hits.vertices.insert(i);
                    }
                }
            }
            Element::Edge => {
                for e in object.geometry.edges().into_keys() {
                    let a = object
                        .transform
                        .transform_point(object.geometry.positions[e.0]);
                    let b = object
                        .transform
                        .transform_point(object.geometry.positions[e.1]);
                    if project(viewer, viewport, a)
                        .zip(project(viewer, viewport, b))
                        .is_some_and(|(a, b)| area_selection::segment_hits(area, a, b))
                        && visible((a + b) * 0.5)
                    {
                        hits.edges.insert(e);
                    }
                }
            }
            Element::Face => {
                for (i, f) in object.geometry.faces.iter().enumerate() {
                    let p = object.transform.transform_point(object.geometry.center(f));
                    if project(viewer, viewport, p).is_some_and(|p| area.contains(p)) && visible(p)
                    {
                        hits.faces.insert(i);
                    }
                }
            }
        }
        if mode == AreaMode::Replace {
            doc.selection.history.clear();
        }
        mode.apply(&mut doc.selection.vertices, hits.vertices);
        mode.apply(&mut doc.selection.edges, hits.edges);
        mode.apply(&mut doc.selection.faces, hits.faces);
        let selection = &mut doc.selection;
        selection.history.retain(|hit| match *hit {
            ComponentHit::Vertex(i) => selection.vertices.contains(&i),
            ComponentHit::Edge(e) => selection.edges.contains(&e),
            ComponentHit::Face(i) => selection.faces.contains(&i),
        });
    }
    doc.ui_revision += 1;
}
