//! Captured layer drags with a lifted row, explicit insertion point and group targets.
use super::*;
use bevy::{picking::events::PointerPress, window::SystemCursorIcon};

#[path = "image_layer_drag_fx.rs"]
pub(in crate::editor) mod fx;
#[path = "image_layer_drag_layout.rs"]
mod layout;
pub(super) use layout::extent;
pub(in crate::editor) use layout::{CopyExtent, Slide};

const THRESHOLD: f32 = 6.0;
const INDENT: f32 = 10.0;

#[derive(Component)]
pub(in crate::editor) struct DropRow {
    pub id: u64,
    pub depth: usize,
    pub group: bool,
}
#[derive(Component)]
pub(in crate::editor) struct LayerList;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Place {
    Above,
    Below,
    Into,
}
impl Place {
    fn command(self) -> &'static str {
        match self {
            Self::Above => "above",
            Self::Below => "below",
            Self::Into => "into",
        }
    }
}
#[derive(Clone)]
pub(in crate::editor) struct Target {
    pub id: u64,
    pub place: Place,
    pub line: Rect,
    pub group: Option<Rect>,
    pub unchanged: bool,
}
struct Gesture {
    image: usize,
    document: photocraft_doc::DocId,
    source: u64,
    selected: Vec<LayerId>,
    roots: Vec<LayerId>,
    origin: Vec2,
    width: f32,
    name: String,
    folder: bool,
}
#[derive(Resource, Default)]
pub(in crate::editor) struct LayerDrag {
    gesture: Option<Gesture>,
    pub active: bool,
    pub target: Option<Target>,
    pub cancelled_this_frame: bool,
    suppress_click: bool,
    cursor: Vec2,
    copy: bool,
    source_rects: Vec<Rect>,
    hover: Option<(u64, f32)>,
    preview: Option<layout::Preview>,
    viewport: Rect,
    landing_serial: u64,
    landing_rect: Rect,
}
impl LayerDrag {
    pub fn capturing(&self) -> bool {
        self.gesture.is_some() || self.cancelled_this_frame
    }
    pub fn suppresses(&self, point: Option<Vec2>) -> bool {
        self.suppress_click
            || self.active
            || self
                .gesture
                .as_ref()
                .zip(point)
                .is_some_and(|(g, p)| g.origin.distance(p) >= THRESHOLD)
    }
    fn finish(&mut self) {
        self.gesture = None;
        self.target = None;
        self.active = false;
        self.source_rects.clear();
        self.hover = None;
        self.preview = None;
    }
    fn cancel(&mut self) {
        self.cancelled_this_frame = self.gesture.is_some();
        self.suppress_click |= self.active;
        self.finish();
    }
}

/// Captures stable IDs, rather than row entities that selection/scrolling can rebuild.
pub(super) fn press(
    event: On<PointerPress>,
    rows: Query<(&LayerRow, &ChildOf)>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    viewer: Res<ImageViewer>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut drag: ResMut<LayerDrag>,
) {
    if event.button != PointerButton::Primary {
        return;
    }
    drag.suppress_click = false;
    drag.finish();
    let Ok((row, parent)) = rows.get(event.entity) else {
        return;
    };
    let Some(image) = viewer.tab.filter(|id| viewer.edit.editing(Some(*id))) else {
        return;
    };
    let Some(doc) = viewer.edit.document(Some(image)) else {
        return;
    };
    let Some(layer) = doc.document.layer(LayerId(row.id)) else {
        return;
    };
    let Ok((node, transform)) = geometry.get(parent.parent()) else {
        return;
    };
    let rect = logical_rect(node, transform, window.scale_factor());
    let selected = if doc.selected.contains(&layer.id) {
        doc.selected.clone()
    } else {
        vec![layer.id]
    };
    let roots = roots(&doc.document, &selected);
    let origin = event.pointer.position;
    let lead = if roots.contains(&layer.id) {
        layer
    } else {
        roots
            .first()
            .and_then(|id| doc.document.layer(*id))
            .unwrap_or(layer)
    };
    drag.gesture = Some(Gesture {
        image,
        document: doc.document.id,
        source: row.id,
        selected,
        roots,
        origin,
        width: rect.width(),
        name: lead.name.clone(),
        folder: lead.is_group(),
    });
}

fn roots(doc: &photocraft_doc::Document, selected: &[LayerId]) -> Vec<LayerId> {
    let paths = selected
        .iter()
        .filter_map(|id| doc.path_of(*id).map(|path| (*id, path)))
        .collect::<Vec<_>>();
    paths
        .iter()
        .filter(|(id, path)| {
            !paths
                .iter()
                .any(|(parent, p)| parent != id && path.starts_with(p))
        })
        .map(|(id, _)| *id)
        .collect()
}

fn allowed(
    doc: &photocraft_doc::Document,
    sources: &[LayerId],
    target: u64,
    place: Place,
    copy: bool,
) -> bool {
    let Some(path) = doc.path_of(LayerId(target)) else {
        return false;
    };
    if place == Place::Into && !doc.layer(LayerId(target)).is_some_and(|l| l.is_group()) {
        return false;
    }
    if copy {
        let paths = sources
            .iter()
            .filter_map(|id| doc.path_of(*id))
            .collect::<Vec<_>>();
        let added = doc
            .walk()
            .iter()
            .filter(|(p, _, _)| paths.iter().any(|path| p.starts_with(path)))
            .count();
        if doc.layer_count() + added > photo_document::MAX_LAYERS {
            return false;
        }
    }
    sources.iter().all(|id| {
        doc.path_of(*id).is_some_and(|source| {
            !path.starts_with(&source) || (copy && id.0 == target && place != Place::Into)
        })
    })
}

fn unchanged(
    doc: &photocraft_doc::Document,
    sources: &[LayerId],
    target: u64,
    place: Place,
) -> bool {
    let Some(path) = doc.path_of(LayerId(target)) else {
        return false;
    };
    let (parent, at) = if place == Place::Into {
        let Some(children) = doc.layer(LayerId(target)).and_then(|l| l.children()) else {
            return false;
        };
        (path.as_slice(), children.len())
    } else {
        let Some((index, parent)) = path.split_last() else {
            return false;
        };
        (parent, *index + usize::from(place == Place::Above))
    };
    let mut positions = Vec::new();
    for id in sources {
        let Some(path) = doc.path_of(*id) else {
            return false;
        };
        let Some((index, p)) = path.split_last() else {
            return false;
        };
        if p != parent {
            return false;
        }
        positions.push(*index);
    }
    positions.sort_unstable();
    let at = at.saturating_sub(positions.iter().filter(|&&index| index < at).count());
    positions.first().copied() == Some(at) && positions.windows(2).all(|w| w[0] + 1 == w[1])
}

#[derive(Clone)]
struct VisibleRow {
    id: u64,
    depth: usize,
    group: bool,
    rect: Rect,
}

fn destination(
    doc: &photocraft_doc::Document,
    sources: &[LayerId],
    rows: &[VisibleRow],
    viewport: Rect,
    cursor: Vec2,
    copy: bool,
) -> Option<Target> {
    if !viewport.contains(cursor) {
        return None;
    }
    let first = rows.first()?;
    let last = rows.last()?;
    let (row, place) = if cursor.y < first.rect.min.y {
        (first, Place::Above)
    } else if cursor.y > last.rect.max.y {
        (last, Place::Below)
    } else {
        let row = rows.iter().min_by(|a, b| {
            let distance = |r: &VisibleRow| (cursor.y - r.rect.center().y).abs();
            distance(a).total_cmp(&distance(b))
        })?;
        let ratio = (cursor.y - row.rect.min.y) / row.rect.height().max(1.0);
        (
            row,
            if row.group && (0.25..0.75).contains(&ratio) {
                Place::Into
            } else if ratio < 0.5 {
                Place::Above
            } else {
                Place::Below
            },
        )
    };
    let mut id = row.id;
    let mut depth = row.depth;
    if place != Place::Into && depth > 0 {
        // Pulling left, or into the empty tail of the list, takes a layer out of its group.
        let wanted = if cursor.y > last.rect.max.y + 6.0 {
            0
        } else {
            ((cursor.x - viewport.min.x - 26.0).max(0.0) / INDENT) as usize
        };
        if wanted < depth {
            let path = doc.path_of(LayerId(id))?;
            if let Some(layer) = doc.layer_at(&path[..(wanted + 1).min(path.len())]) {
                id = layer.id.0;
                depth = wanted;
            }
        }
    }
    if !allowed(doc, sources, id, place, copy) {
        return None;
    }
    let target = rows.iter().find(|r| r.id == id).unwrap_or(row);
    let y = match place {
        Place::Above => target.rect.min.y - 1.0,
        Place::Into => target.rect.max.y + 1.0,
        Place::Below => {
            let path = doc.path_of(LayerId(id))?;
            rows.iter()
                .filter(|r| {
                    doc.path_of(LayerId(r.id))
                        .is_some_and(|p| p.starts_with(&path))
                })
                .map(|r| r.rect.max.y)
                .fold(target.rect.max.y, f32::max)
                + 1.0
        }
    }
    .clamp(viewport.min.y + 2.0, viewport.max.y - 2.0);
    let left =
        (viewport.min.x + 6.0 + (depth + usize::from(place == Place::Into)).min(6) as f32 * INDENT)
            .min(viewport.max.x - 12.0);
    let line = Rect::from_corners(
        Vec2::new(left, y - 1.5),
        Vec2::new(viewport.max.x - 3.0, y + 1.5),
    );
    let same = !copy && unchanged(doc, sources, id, place);
    // Keep the position typed: the visual and the committed engine command share this target.
    Some(Target {
        id,
        place,
        line,
        group: (place == Place::Into).then_some(target.rect.intersect(viewport)),
        unchanged: same,
    })
}

fn logical_rect(node: &ComputedNode, transform: &UiGlobalTransform, scale: f32) -> Rect {
    Rect::from_center_size(transform.affine().translation / scale, node.size() / scale)
}

pub(in crate::editor) fn input(
    mut drag: ResMut<LayerDrag>,
    editor: Res<Editor>,
    viewer: Res<ImageViewer>,
    window: Single<&Window, With<PrimaryWindow>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    rows: Query<(&DropRow, &ComputedNode, &UiGlobalTransform, &Slide)>,
    mut lists: Query<(&ComputedNode, &UiGlobalTransform, &mut ScrollPosition), With<LayerList>>,
    drawer: Option<Res<DrawerMotion>>,
    time: Res<Time>,
    mut focus: ResMut<InputFocus>,
    mut actions: MessageWriter<Action>,
    mut ui: ResMut<LayerUi>,
) {
    drag.cancelled_this_frame = false;
    let Some(gesture) = drag.gesture.as_ref() else {
        // Keep suppression through the release observers, then restore ordinary
        // row clicks, context menus and keyboard activation on the next frame.
        if !mouse.pressed(MouseButton::Left) && !mouse.just_released(MouseButton::Left) {
            drag.suppress_click = false;
        }
        return;
    };
    let doc = viewer.edit.document(editor.viewing_image);
    let valid = editor.viewing_image == Some(gesture.image)
        && viewer.edit.editing(editor.viewing_image)
        && doc.is_some_and(|d| {
            d.document.id == gesture.document
                && gesture
                    .selected
                    .iter()
                    .all(|id| d.document.layer(*id).is_some())
        });
    if !valid
        || !window.focused
        || editor.pending.is_some()
        || editor.browser_popup
        || editor.hdri_library_open
        || viewer.edit.layers.settings_open
        || viewer.edit.layers.menu
        || viewer.edit.layers.form.is_some()
        || viewer.edit.color_picker.is_some()
        || viewer.edit.busy()
        || keys.just_pressed(KeyCode::Escape)
    {
        drag.cancel();
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        if !mouse.pressed(MouseButton::Left) {
            drag.cancel();
        }
        return;
    };
    let origin = gesture.origin;
    drag.cursor = cursor;
    if cursor.distance(origin) >= THRESHOLD {
        drag.active = true;
        drag.suppress_click = true;
        ui.last_click = None;
    }
    if drag.active {
        focus.clear();
        let Some(doc) = doc else {
            drag.cancel();
            return;
        };
        let Ok((list, transform, mut scroll)) = lists.single_mut() else {
            drag.cancel();
            return;
        };
        let mut viewport = logical_rect(list, transform, window.scale_factor());
        if let Some(drawer) = drawer {
            viewport = drawer.input_rect(viewport, window.height());
        }
        if viewport.width() < 1.0 || viewport.height() < 8.0 {
            drag.cancel();
            return;
        }
        let mut visible = rows
            .iter()
            .filter_map(|(row, node, transform, slide)| {
                if node.is_empty() {
                    return None;
                }
                let mut rect = logical_rect(node, transform, window.scale_factor());
                rect.min.y -= slide.offset;
                rect.max.y -= slide.offset;
                Some(VisibleRow {
                    id: row.id,
                    depth: row.depth,
                    group: row.group,
                    rect,
                })
            })
            .collect::<Vec<_>>();
        visible.sort_by(|a, b| a.rect.min.y.total_cmp(&b.rect.min.y));
        let Some(gesture) = drag.gesture.as_ref() else {
            return;
        };
        let copy = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
        let source_paths = gesture
            .roots
            .iter()
            .filter_map(|id| doc.document.path_of(*id))
            .collect::<Vec<_>>();
        let source_rects = visible
            .iter()
            .filter(|r| {
                doc.document
                    .path_of(LayerId(r.id))
                    .is_some_and(|p| source_paths.iter().any(|root| p.starts_with(root)))
            })
            .filter_map(|r| {
                let rect = r.rect.intersect(viewport);
                (rect.width() > 0.0 && rect.height() > 0.0).then_some(rect)
            })
            .collect();
        let mut target = layout::pick(
            &doc.document,
            &gesture.roots,
            &visible,
            viewport,
            cursor,
            copy,
            drag.target.as_ref(),
        );
        let preview = target.as_ref().and_then(|t| {
            layout::project(&doc.document, &gesture.roots, &visible, viewport, t, copy)
        });
        if let (Some(target), Some(preview)) = (&mut target, &preview) {
            preview.decorate(target, viewport);
        }
        drag.copy = copy;
        drag.source_rects = if preview.is_some() {
            Vec::new()
        } else {
            source_rects
        };
        drag.target = target;
        drag.preview = preview;
        drag.viewport = viewport;
        if mouse.pressed(MouseButton::Left) && viewport.contains(cursor) {
            let edge = 28.0_f32.min(viewport.height() * 0.25);
            let direction = if cursor.y < viewport.min.y + edge {
                -(1.0 - (cursor.y - viewport.min.y) / edge)
            } else if cursor.y > viewport.max.y - edge {
                1.0 - (viewport.max.y - cursor.y) / edge
            } else {
                0.0
            };
            let max =
                ((list.content_size().y - list.size().y) * list.inverse_scale_factor()).max(0.0);
            scroll.y = (scroll.y + direction * 650.0 * time.delta_secs().min(0.05)).clamp(0.0, max);
        }
        let expand = drag
            .target
            .as_ref()
            .filter(|t| t.place == Place::Into && !t.unchanged)
            .map(|t| t.id)
            .filter(|id| doc.document.layer(LayerId(*id)).is_some_and(collapsed));
        if let Some(id) = expand {
            let hover = drag.hover.get_or_insert((id, 0.0));
            if hover.0 != id {
                *hover = (id, 0.0);
            }
            hover.1 += time.delta_secs();
            if hover.1 >= 0.65 {
                actions.write(act(LayerAction::Collapse(id)));
                drag.hover = None;
            }
        } else {
            drag.hover = None;
        }
    }
    if mouse.just_released(MouseButton::Left) || !mouse.pressed(MouseButton::Left) {
        if drag.active
            && let Some(target) = drag.target.as_ref().filter(|t| !t.unchanged)
            && let Some(gesture) = drag.gesture.as_ref()
            && let Some(doc) = doc
        {
            if doc.selected != gesture.selected {
                for (i, id) in gesture.selected.iter().enumerate() {
                    if let Some(index) =
                        doc.document.walk().iter().position(|(_, _, l)| l.id == *id)
                    {
                        actions.write(act(LayerAction::Select(index, i != 0, false)));
                    }
                }
            }
            actions.write(act(LayerAction::Move(
                gesture.source,
                target.id,
                target.place.command(),
                drag.copy,
            )));
            if target.place == Place::Into
                && doc
                    .document
                    .layer(LayerId(target.id))
                    .is_some_and(collapsed)
            {
                actions.write(act(LayerAction::Collapse(target.id)));
            }
            if let Some(preview) = &drag.preview {
                let rect = preview.gap.intersect(drag.viewport);
                drag.landing_rect = rect;
                drag.landing_serial = drag.landing_serial.wrapping_add(1);
            }
        }
        drag.finish();
    }
}

#[derive(Resource)]
pub(in crate::editor) struct Overlay {
    parent: Entity,
    root: Entity,
    name: Entity,
    hint: Entity,
    image: Entity,
    folder: Entity,
    line: Entity,
    group: Entity,
    gap: Entity,
    veils: Vec<Entity>,
    cursor_owned: bool,
    positions: HashMap<u64, Vec2>,
}

pub(super) fn setup(commands: &mut Commands, root: Entity, font: &Handle<Font>) {
    let floating = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: px(248),
            padding: UiRect::all(px(8)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(10)),
            ..row(8.0)
        },
        PANEL,
    );
    commands.entity(floating).insert((
        GlobalZIndex(1200),
        Pickable::IGNORE,
        BorderColor::all(FOCUS),
        AccessibleLabel("Перетаскиваемый слой".into()),
        BoxShadow(vec![ShadowStyle {
            color: Color::srgba(0.0, 0.0, 0.0, 0.48),
            x_offset: px(0),
            y_offset: px(8),
            spread_radius: px(1),
            blur_radius: px(18),
        }]),
    ));
    let image = commands
        .spawn((
            ImageNode::default(),
            Node {
                width: px(32),
                height: px(32),
                flex_shrink: 0.0,
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(floating),
        ))
        .id();
    let folder = icons::spawn(commands, floating, icons::Glyph::Folder, 26.0, TEXT);
    let names = node(
        commands,
        floating,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            overflow: Overflow::clip(),
            ..column(3.0)
        },
    );
    commands.entity(names).insert(Pickable::IGNORE);
    let name = text(commands, names, font, "", 12.0, TEXT);
    let hint = text(commands, names, font, "", 10.0, MUTED);
    for e in [name, hint] {
        commands
            .entity(e)
            .insert((Pickable::IGNORE, TextLayout::no_wrap()));
    }
    let line = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            height: px(3),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        FOCUS,
    );
    commands.entity(line).insert((
        GlobalZIndex(1101),
        Pickable::IGNORE,
        AccessibleLabel("Место вставки слоя".into()),
    ));
    let group = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(8)),
            ..default()
        },
        GREEN.with_alpha(0.15),
    );
    commands.entity(group).insert((
        GlobalZIndex(1100),
        Pickable::IGNORE,
        BorderColor::all(GREEN),
        AccessibleLabel("Перенос внутрь группы".into()),
    ));
    let gap = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            ..default()
        },
        FOCUS.with_alpha(0.08),
    );
    commands.entity(gap).insert((
        GlobalZIndex(1100),
        Pickable::IGNORE,
        BorderColor::all(FOCUS.with_alpha(0.55)),
        AccessibleLabel("Место для слоя".into()),
    ));
    commands.insert_resource(Overlay {
        parent: root,
        root: floating,
        name,
        hint,
        image,
        folder,
        line,
        group,
        gap,
        veils: Vec::new(),
        cursor_owned: false,
        positions: HashMap::new(),
    });
}

pub(in crate::editor) fn visuals(
    mut commands: Commands,
    drag: Res<LayerDrag>,
    mut overlay: ResMut<Overlay>,
    ui: Res<LayerUi>,
    viewer: Res<ImageViewer>,
    window: Single<(Entity, &Window, Option<&CursorIcon>), With<PrimaryWindow>>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<(&mut Text, &mut TextColor)>,
    mut images: Query<&mut ImageNode>,
    mut colors: Query<&mut BackgroundColor>,
    mut rows: Query<(
        &DropRow,
        &ComputedNode,
        &mut Slide,
        &mut UiTransform,
        &mut Visibility,
    )>,
    extents: Query<Entity, With<CopyExtent>>,
    time: Res<Time>,
) {
    let (window_entity, window, cursor) = *window;
    let active = drag.active && drag.gesture.is_some();
    // The document's row order stays untouched. Translations preview its future
    // layout; hiding the lifted rows preserves the scroll extent while dragging.
    let preview = drag.preview.as_ref().filter(|_| active);
    for (row, computed, mut slide, mut transform, mut visibility) in &mut rows {
        let goal = preview
            .and_then(|p| p.offsets.iter().find(|(id, _)| *id == row.id))
            .map_or(0.0, |(_, y)| *y);
        // Stable IDs keep their motion when expanding a group rebuilds the UI.
        let previous = if computed.is_empty() {
            overlay
                .positions
                .get(&row.id)
                .copied()
                .unwrap_or(Vec2::new(slide.offset, slide.velocity))
        } else {
            Vec2::new(slide.offset, slide.velocity)
        };
        let (offset, velocity) = if preview.is_some() {
            fx::spring(previous.x, previous.y, goal, time.delta_secs())
        } else {
            (0.0, 0.0)
        };
        slide.velocity = velocity;
        if slide.offset != offset {
            slide.offset = offset;
            transform.translation.y = px(offset);
        }
        visibility.set_if_neq(if preview.is_some_and(|p| p.removed.contains(&row.id)) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
        if active {
            overlay
                .positions
                .insert(row.id, Vec2::new(offset, velocity));
        }
    }
    if !active {
        overlay.positions.clear();
    }
    for entity in &extents {
        if let Ok(mut node) = nodes.get_mut(entity) {
            let extra = preview.map_or(0.0, |p| p.extra);
            let display = if extra > 0.0 {
                Display::Flex
            } else {
                Display::None
            };
            if node.display != display {
                node.display = display;
            }
            if node.height != px(extra) {
                node.height = px(extra);
            }
        }
    }
    for e in [overlay.root, overlay.line, overlay.group, overlay.gap]
        .into_iter()
        .chain(overlay.veils.iter().copied())
    {
        if let Ok(mut node) = nodes.get_mut(e)
            && node.display != Display::None
        {
            node.display = Display::None;
        }
    }
    if !active {
        if overlay.cursor_owned && cursor == Some(&CursorIcon::from(SystemCursorIcon::Grabbing)) {
            commands.entity(window_entity).insert(CursorIcon::default());
        }
        overlay.cursor_owned = false;
        return;
    }
    let Some(gesture) = drag.gesture.as_ref() else {
        return;
    };
    let doc = viewer.edit.document(Some(gesture.image));
    let color = if drag.target.as_ref().is_some_and(|t| t.place == Place::Into) {
        GREEN
    } else {
        FOCUS
    };
    let width = gesture
        .width
        .clamp(200.0, (window.width() - 16.0).max(200.0));
    // Keep the card beside the pointer so it cannot cover the insertion edge.
    // Near the right edge, flip it to the pointer's left instead of clamping it
    // over the very row/group the user is trying to aim at.
    let x = if drag.cursor.x + width + 14.0 < window.width() - 5.0 {
        drag.cursor.x + 14.0
    } else {
        drag.cursor.x - width - 14.0
    };
    let mut y = drag.cursor.y + 16.0;
    if let Some(group) = drag.target.as_ref().and_then(|t| t.group) {
        y = y.max(group.max.y + 8.0);
    }
    if y + 56.0 > window.height() - 5.0 {
        y = drag.cursor.y - 64.0;
    }
    let position = Vec2::new(x, y).clamp(
        Vec2::splat(5.0),
        Vec2::new(
            (window.width() - width - 5.0).max(5.0),
            (window.height() - 64.0).max(5.0),
        ),
    );
    if let Ok(mut node) = nodes.get_mut(overlay.root) {
        node.display = Display::Flex;
        node.width = px(width);
        node.left = px(position.x);
        node.top = px(position.y);
    }
    let count = gesture.roots.len();
    let name = if count > 1 {
        format!("{} · ещё {}", gesture.name, count - 1)
    } else {
        gesture.name.clone()
    };
    let prefix = if drag.copy {
        "Копия"
    } else {
        "Перенос"
    };
    let hint = match &drag.target {
        Some(t) if t.unchanged => "Уже на этом месте".to_owned(),
        Some(t) => format!(
            "{prefix}: {} «{}»",
            match t.place {
                Place::Into => "в группу",
                Place::Above => "над",
                Place::Below => "под",
            },
            doc.and_then(|d| d.document.layer(LayerId(t.id)))
                .map_or("", |l| l.name.as_str())
        ),
        None => "Здесь нельзя разместить · Esc — отмена".into(),
    };
    for (e, value, tint) in [
        (overlay.name, name, TEXT),
        (
            overlay.hint,
            hint,
            if drag.target.is_some() { color } else { MUTED },
        ),
    ] {
        if let Ok((mut text, mut c)) = labels.get_mut(e) {
            text.set_if_neq(Text(value));
            c.set_if_neq(TextColor(tint));
        }
    }
    if let Ok(mut node) = nodes.get_mut(overlay.folder) {
        node.display = if gesture.folder {
            Display::Flex
        } else {
            Display::None
        };
    }
    let thumbnail =
        ui.thumbnails
            .get(&(gesture.source, false, viewer.edit.layers.thumbnail_content));
    if let Ok(mut node) = nodes.get_mut(overlay.image) {
        node.display = if !gesture.folder && thumbnail.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Some((_, handle)) = thumbnail
        && let Ok(mut image) = images.get_mut(overlay.image)
    {
        image.image = handle.clone();
    }
    let set = |node: &mut Node, rect: Rect| {
        node.display = Display::Flex;
        node.left = px(rect.min.x);
        node.top = px(rect.min.y);
        node.width = px(rect.width());
        node.height = px(rect.height());
    };
    if let Some(preview) = preview {
        let rect = preview.gap.intersect(drag.viewport);
        if rect.width() > 0.0
            && rect.height() > 0.0
            && let Ok(mut node) = nodes.get_mut(overlay.gap)
        {
            set(&mut node, rect);
        }
    }
    if let Some(target) = &drag.target {
        if let Ok(mut node) = nodes.get_mut(overlay.line) {
            set(&mut node, target.line);
        }
        if let Ok(mut fill) = colors.get_mut(overlay.line) {
            fill.0 = if target.unchanged { MUTED } else { color };
        }
        if let Some(rect) = target.group
            && let Ok(mut node) = nodes.get_mut(overlay.group)
        {
            set(&mut node, rect);
        }
    }
    while overlay.veils.len() < drag.source_rects.len() {
        let e = surface(
            &mut commands,
            overlay.parent,
            Node {
                position_type: PositionType::Absolute,
                border_radius: BorderRadius::all(px(5)),
                ..default()
            },
            CHROME.with_alpha(0.72),
        );
        commands
            .entity(e)
            .insert((GlobalZIndex(1099), Pickable::IGNORE));
        overlay.veils.push(e);
    }
    for (entity, rect) in overlay.veils.iter().zip(&drag.source_rects) {
        if let Ok(mut node) = nodes.get_mut(*entity) {
            set(&mut node, *rect);
        }
    }
    let desired = CursorIcon::from(SystemCursorIcon::Grabbing);
    if cursor != Some(&desired) {
        commands.entity(window_entity).insert(desired);
    }
    overlay.cursor_owned = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Document, Layer, PixelFormat, SampleType, Size};

    fn drawing(name: &str) -> Layer {
        Layer::raster(name, PixelFormat::RGBA8)
    }
    fn document(layers: Vec<Layer>) -> Document {
        let mut doc = Document::new("Layers", Size::new(24, 24), ColorMode::Rgb, SampleType::U8);
        doc.layers = layers;
        doc
    }
    fn visible(id: LayerId, depth: usize, group: bool, y: f32) -> VisibleRow {
        VisibleRow {
            id: id.0,
            depth,
            group,
            rect: Rect::from_corners(
                Vec2::new(100.0 + depth as f32 * INDENT, y),
                Vec2::new(400.0, y + 40.0),
            ),
        }
    }
    #[test]
    fn layer_drag_rejects_cycles_and_moves_selected_subtrees_once() {
        let child = drawing("child");
        let c = child.id;
        let group = Layer::group("group", vec![child]);
        let g = group.id;
        let other = drawing("other");
        let o = other.id;
        let doc = document(vec![group, other]);
        assert_eq!(roots(&doc, &[c, g, o]), [g, o]);
        assert!(!allowed(&doc, &[g], c.0, Place::Above, false));
        assert!(!allowed(&doc, &[g], c.0, Place::Below, true));
        assert!(!allowed(&doc, &[g], g.0, Place::Into, true));
        assert!(allowed(&doc, &[g], g.0, Place::Above, true));
        assert!(allowed(&doc, &[c], g.0, Place::Into, false));
        assert!(!allowed(&doc, &[g], o.0, Place::Into, false));
    }
    #[test]
    fn layer_drag_skips_noop_history_for_contiguous_siblings_and_group_tail() {
        let a = drawing("a");
        let ai = a.id;
        let b = drawing("b");
        let bi = b.id;
        let c = drawing("c");
        let ci = c.id;
        let doc = document(vec![a, b, c]);
        assert!(unchanged(&doc, &[ai, bi], ci.0, Place::Below));
        assert!(unchanged(&doc, &[bi, ci], ai.0, Place::Above));
        assert!(!unchanged(&doc, &[ai, ci], bi.0, Place::Above));
        assert!(!unchanged(&doc, &[ai], ci.0, Place::Above));
        let child = drawing("child");
        let c = child.id;
        let group = Layer::group("group", vec![child]);
        let g = group.id;
        let doc = document(vec![group]);
        assert!(unchanged(&doc, &[c], g.0, Place::Into));
    }
    #[test]
    fn layer_drag_preview_distinguishes_group_middle_edges_outdent_and_outside() {
        let a = drawing("a");
        let ai = a.id;
        let child = drawing("child");
        let ci = child.id;
        let group = Layer::group("group", vec![child]);
        let gi = group.id;
        let b = drawing("b");
        let bi = b.id;
        let doc = document(vec![a, group, b]);
        let rows = [
            visible(bi, 0, false, 0.0),
            visible(gi, 0, true, 44.0),
            visible(ci, 1, false, 88.0),
            visible(ai, 0, false, 132.0),
        ];
        let viewport = Rect::from_corners(Vec2::new(100.0, 0.0), Vec2::new(400.0, 400.0));
        let pick = |source: LayerId, x, y| {
            destination(&doc, &[source], &rows, viewport, Vec2::new(x, y), false).unwrap()
        };
        let t = pick(ai, 220.0, 48.0);
        assert_eq!((t.id, t.place), (gi.0, Place::Above));
        assert!(t.group.is_none());
        assert_eq!(t.line.center().y, 43.0);
        let t = pick(ai, 220.0, 64.0);
        assert_eq!((t.id, t.place), (gi.0, Place::Into));
        assert!(t.group.is_some());
        let t = pick(ai, 220.0, 81.0);
        assert_eq!((t.id, t.place), (gi.0, Place::Below));
        assert_eq!(
            t.line.center().y,
            129.0,
            "Below an expanded group means below its entire subtree"
        );
        let t = pick(ai, 108.0, 120.0);
        assert_eq!(
            (t.id, t.place),
            (gi.0, Place::Below),
            "Pulling a nested target left means beside its parent"
        );
        let t = pick(ci, 220.0, 210.0);
        assert_eq!((t.id, t.place), (ai.0, Place::Below));
        assert!(destination(&doc, &[ai], &rows, viewport, Vec2::new(99.0, 64.0), false).is_none());
    }
}
