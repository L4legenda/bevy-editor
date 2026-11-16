//! A presentation-only reorder: lift the source slots and leave room at the drop target.
use super::*;

const SPACING: f32 = 2.0; // Matches the layer tree's column gap.

#[derive(Component, Default)]
pub(in crate::editor) struct Slide {
    pub offset: f32,
    pub velocity: f32,
}

#[derive(Component)]
pub(in crate::editor) struct CopyExtent;

pub(in crate::editor) fn extent(commands: &mut Commands, host: Entity) {
    commands.spawn((
        Node {
            display: Display::None,
            width: px(1),
            flex_shrink: 0.0,
            ..default()
        },
        CopyExtent,
        Pickable::IGNORE,
        ChildOf(host),
    ));
}

pub(super) struct Preview {
    pub rows: Vec<VisibleRow>,
    pub removed: Vec<u64>,
    pub offsets: Vec<(u64, f32)>,
    pub gap: Rect,
    pub extra: f32,
}

fn source_rows(
    doc: &photocraft_doc::Document,
    sources: &[LayerId],
    rows: &[VisibleRow],
) -> Vec<u64> {
    let paths = sources
        .iter()
        .filter_map(|id| doc.path_of(*id))
        .collect::<Vec<_>>();
    rows.iter()
        .filter(|row| {
            doc.path_of(LayerId(row.id))
                .is_some_and(|p| paths.iter().any(|root| p.starts_with(root)))
        })
        .map(|r| r.id)
        .collect()
}

/// All rectangles are in logical window pixels, before any animated translation.
pub(super) fn project(
    doc: &photocraft_doc::Document,
    sources: &[LayerId],
    rows: &[VisibleRow],
    viewport: Rect,
    target: &Target,
    copy: bool,
) -> Option<Preview> {
    let at = rows.iter().position(|r| r.id == target.id)?;
    let boundary = match target.place {
        Place::Above => at,
        Place::Into => at + 1,
        Place::Below => {
            let path = doc.path_of(LayerId(target.id))?;
            at + rows[at..]
                .iter()
                .take_while(|r| {
                    doc.path_of(LayerId(r.id))
                        .is_some_and(|p| p.starts_with(&path))
                })
                .count()
        }
    };
    let sources = source_rows(doc, sources, rows);
    if sources.is_empty() {
        return None;
    }
    let pitches = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            rows.get(i + 1).map_or(r.rect.height() + SPACING, |next| {
                next.rect.min.y - r.rect.min.y
            })
        })
        .collect::<Vec<_>>();
    let pitch = rows
        .iter()
        .zip(&pitches)
        .filter(|(r, _)| sources.contains(&r.id))
        .map(|(_, p)| *p)
        .sum::<f32>();
    let before = rows
        .iter()
        .zip(&pitches)
        .take(boundary)
        .filter(|(r, _)| !copy && sources.contains(&r.id))
        .map(|(_, p)| *p)
        .sum::<f32>();
    let y = rows.get(boundary).map_or_else(
        || rows.last().unwrap().rect.max.y + SPACING,
        |r| r.rect.min.y,
    ) - before;
    let depth = rows[at].depth + usize::from(target.place == Place::Into);
    let gap = Rect::from_corners(
        Vec2::new(
            (viewport.min.x + 6.0 + depth.min(6) as f32 * INDENT).min(viewport.max.x - 12.0),
            y,
        ),
        Vec2::new(viewport.max.x - 3.0, y + (pitch - SPACING).max(1.0)),
    );
    let mut result = Preview {
        rows: Vec::new(),
        removed: if copy { Vec::new() } else { sources.clone() },
        offsets: Vec::new(),
        gap,
        extra: if copy { pitch - SPACING } else { 0.0 },
    };
    let mut lifted = 0.0;
    for (i, row) in rows.iter().enumerate() {
        if !copy && sources.contains(&row.id) {
            lifted += pitches[i];
            continue;
        }
        let offset = -lifted + if i >= boundary { pitch } else { 0.0 };
        let mut shifted = row.clone();
        shifted.rect.min.y += offset;
        shifted.rect.max.y += offset;
        result.rows.push(shifted);
        result.offsets.push((row.id, offset));
    }
    Some(result)
}

/// Aim at the settled preview rather than the animation in flight. A pointer
/// inside the vacant slot keeps its target: opening a gap must not move its own
/// hit boundary and make the rows oscillate under a stationary pointer.
pub(super) fn pick(
    doc: &photocraft_doc::Document,
    sources: &[LayerId],
    rows: &[VisibleRow],
    viewport: Rect,
    cursor: Vec2,
    copy: bool,
    previous: Option<&Target>,
) -> Option<Target> {
    if !viewport.contains(cursor) {
        return None;
    }
    let previous = previous.filter(|t| allowed(doc, sources, t.id, t.place, copy));
    let preview = previous.and_then(|t| project(doc, sources, rows, viewport, t, copy));
    if let (Some(target), Some(preview)) = (previous, &preview)
        && preview.gap.contains(cursor)
    {
        let mut target = target.clone();
        target.unchanged = !copy && unchanged(doc, sources, target.id, target.place);
        return Some(target);
    }
    let lifted;
    let displayed = if let Some(preview) = &preview {
        &preview.rows
    } else {
        let sources = source_rows(doc, sources, rows);
        lifted = rows
            .iter()
            .filter(|r| copy || !sources.contains(&r.id))
            .cloned()
            .collect::<Vec<_>>();
        &lifted
    };
    destination(doc, sources, displayed, viewport, cursor, copy)
}

impl Preview {
    pub fn decorate(&self, target: &mut Target, viewport: Rect) {
        let y = self
            .gap
            .min
            .y
            .clamp(viewport.min.y + 2.0, viewport.max.y - 2.0);
        target.line = Rect::from_corners(
            Vec2::new(self.gap.min.x, y - 1.5),
            Vec2::new(self.gap.max.x, y + 1.5),
        );
        target.group = (target.place == Place::Into)
            .then(|| {
                self.rows
                    .iter()
                    .find(|r| r.id == target.id)
                    .map(|r| r.rect.intersect(viewport))
            })
            .flatten();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Document, Layer, PixelFormat, SampleType, Size};

    fn raster(name: &str) -> Layer {
        Layer::raster(name, PixelFormat::RGBA8)
    }
    fn document(layers: Vec<Layer>) -> Document {
        let mut d = Document::new("drag", Size::new(24, 24), ColorMode::Rgb, SampleType::U8);
        d.layers = layers;
        d
    }
    fn row(id: LayerId, depth: usize, group: bool, top: f32, height: f32) -> VisibleRow {
        VisibleRow {
            id: id.0,
            depth,
            group,
            rect: Rect::from_corners(Vec2::new(100.0, top), Vec2::new(400.0, top + height)),
        }
    }
    fn viewport() -> Rect {
        Rect::from_corners(Vec2::new(100.0, 0.0), Vec2::new(400.0, 600.0))
    }
    fn target(id: LayerId, place: Place) -> Target {
        Target {
            id: id.0,
            place,
            line: Rect::default(),
            group: None,
            unchanged: false,
        }
    }

    #[test]
    fn layer_gap_lifts_source_and_opens_its_exact_height_at_the_target() {
        let a = raster("a");
        let ai = a.id;
        let b = raster("b");
        let bi = b.id;
        let c = raster("c");
        let ci = c.id;
        let d = document(vec![a, b, c]);
        // A smart-object row may contain filter controls; its entire block moves.
        let rows = [
            row(ci, 0, false, 0.0, 42.0),
            row(bi, 0, false, 44.0, 100.0),
            row(ai, 0, false, 146.0, 42.0),
        ];
        let p = project(
            &d,
            &[bi],
            &rows,
            viewport(),
            &target(ci, Place::Above),
            false,
        )
        .unwrap();
        assert_eq!(p.removed, [bi.0]);
        assert_eq!(p.gap.height(), 100.0);
        assert_eq!(
            p.rows
                .iter()
                .map(|r| (r.id, r.rect.min.y))
                .collect::<Vec<_>>(),
            [(ci.0, 102.0), (ai.0, 146.0)]
        );
        assert_eq!(p.extra, 0.0);
        let copy = project(
            &d,
            &[bi],
            &rows,
            viewport(),
            &target(ci, Place::Above),
            true,
        )
        .unwrap();
        assert!(copy.removed.is_empty());
        assert_eq!(copy.extra, 100.0);
        assert_eq!(
            copy.rows.iter().map(|r| r.rect.min.y).collect::<Vec<_>>(),
            [102.0, 146.0, 248.0]
        );
    }

    #[test]
    fn layer_gap_moves_a_group_with_children_and_indents_an_inside_drop() {
        let a = raster("a");
        let ai = a.id;
        let child = raster("child");
        let ci = child.id;
        let group = Layer::group("group", vec![child]);
        let gi = group.id;
        let b = raster("b");
        let bi = b.id;
        let d = document(vec![a, group, b]);
        let rows = [
            row(bi, 0, false, 0.0, 42.0),
            row(gi, 0, true, 44.0, 42.0),
            row(ci, 1, false, 88.0, 42.0),
            row(ai, 0, false, 132.0, 42.0),
        ];
        let p = project(
            &d,
            &[gi, ci],
            &rows,
            viewport(),
            &target(bi, Place::Above),
            false,
        )
        .unwrap();
        assert_eq!(p.removed, [gi.0, ci.0]);
        assert_eq!(p.gap.height(), 86.0);
        assert_eq!(p.rows[0].rect.min.y, 88.0);
        assert_eq!(p.rows[1].rect.min.y, 132.0);
        let inside = project(
            &d,
            &[bi],
            &rows,
            viewport(),
            &target(gi, Place::Into),
            false,
        )
        .unwrap();
        assert_eq!(inside.gap.min, Vec2::new(116.0, 44.0));
        assert_eq!(inside.rows[0].id, gi.0);
        assert_eq!(inside.rows[0].rect.min.y, 0.0);
        assert_eq!(inside.rows[1].rect.min.y, 88.0);
    }

    #[test]
    fn layer_gap_stays_under_a_stationary_pointer_and_follows_the_next_row() {
        let a = raster("a");
        let ai = a.id;
        let b = raster("b");
        let bi = b.id;
        let c = raster("c");
        let ci = c.id;
        let top = raster("top");
        let ti = top.id;
        let d = document(vec![a, b, c, top]);
        let rows = [
            row(ti, 0, false, 0.0, 42.0),
            row(ci, 0, false, 44.0, 42.0),
            row(bi, 0, false, 88.0, 42.0),
            row(ai, 0, false, 132.0, 42.0),
        ];
        let mut previous = None;
        for _ in 0..80 {
            let next = pick(
                &d,
                &[ci],
                &rows,
                viewport(),
                Vec2::new(200.0, 5.0),
                false,
                previous.as_ref(),
            )
            .unwrap();
            assert_eq!((next.id, next.place), (ti.0, Place::Above));
            previous = Some(next);
        }
        for _ in 0..80 {
            let next = pick(
                &d,
                &[ci],
                &rows,
                viewport(),
                Vec2::new(200.0, 120.0),
                false,
                previous.as_ref(),
            )
            .unwrap();
            assert_eq!((next.id, next.place), (bi.0, Place::Below));
            previous = Some(next);
        }
        let p = project(
            &d,
            &[ci],
            &rows,
            viewport(),
            previous.as_ref().unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(p.gap.min.y, 88.0);
        assert!(
            pick(
                &d,
                &[ci],
                &rows,
                viewport(),
                Vec2::new(90.0, 120.0),
                false,
                previous.as_ref()
            )
            .is_none()
        );
    }
}
