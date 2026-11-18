//! Pointer gestures over clipped outliner rows.
use super::*;
use crate::{editor::scene_hierarchy::HierarchyAction, hierarchy::TreeDrop};
use bevy::ui::CalculatedClip;

const THRESHOLD: f32 = 6.0;

#[derive(Clone, Copy)]
struct Press {
    room: Room,
    source: Selection,
    origin: Vec2,
}

#[derive(Resource, Default)]
pub(in crate::editor) struct TreeDrag {
    press: Option<Press>,
    active: bool,
    suppress: bool,
}

impl TreeDrag {
    pub(super) fn suppresses(&self, cursor: Option<Vec2>) -> bool {
        self.suppress
            || self.active
            || self
                .press
                .zip(cursor)
                .is_some_and(|(press, cursor)| press.origin.distance(cursor) >= THRESHOLD)
    }
}

#[derive(Component)]
pub(in crate::editor) struct Row {
    target: Option<Selection>,
    indicator: Entity,
}

pub(super) fn mark(commands: &mut Commands, item: Entity, target: Option<Selection>) {
    let indicator = commands
        .spawn((
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                border_radius: BorderRadius::all(px(3)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            BorderColor::all(FOCUS),
            ZIndex(2),
            Pickable::IGNORE,
            ChildOf(item),
        ))
        .id();
    commands.entity(item).insert(Row { target, indicator });
}

pub(super) fn root_row(commands: &mut Commands, parent: Entity, font: &Handle<Font>) {
    let root = node(
        commands,
        parent,
        Node {
            width: percent(100),
            height: px(20),
            padding: UiRect::left(px(8)),
            flex_shrink: 0.0,
            ..row(0.0)
        },
    );
    text(commands, root, font, "Корень сцены", 11.0, MUTED);
    control_hints::attach(
        commands,
        root,
        "Перетащите узел сюда, чтобы вынести его в корень",
    );
    mark(commands, root, None);
}

pub(in crate::editor) fn update(
    editor: Res<Editor>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    assets: Res<AssetDrag>,
    mut focus: ResMut<InputFocus>,
    mut drag: ResMut<TreeDrag>,
    rows: Query<(
        &Row,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&CalculatedClip>,
    )>,
    mut indicators: Query<(&mut Node, &mut BackgroundColor)>,
    mut actions: MessageWriter<Action>,
    mut previous: Local<Option<Entity>>,
) {
    if mouse.just_pressed(MouseButton::Left)
        || !mouse.pressed(MouseButton::Left) && !mouse.just_released(MouseButton::Left)
    {
        drag.suppress = false;
    }
    let cursor = window.cursor_position();
    let valid = editor.has_scene()
        && editor.ready
        && editor.definition().is_native()
        && !editor.asset_viewer_open()
        && editor.pending.is_none()
        && !editor.browser_popup
        && !editor.scene_object_popup
        && editor.hierarchy_parenting.is_none()
        && !editor.sidebar.resizing()
        && !assets.active()
        && window.focused
        && !keys.just_pressed(KeyCode::Escape)
        && drag.press.is_none_or(|press| press.room == editor.room);
    let hit = if valid && (drag.press.is_some() || mouse.just_pressed(MouseButton::Left)) {
        cursor
            .map(|point| point * window.scale_factor())
            .and_then(|point| {
                rows.iter().find_map(|(row, node, transform, clip)| {
                    if node.is_empty()
                        || !node.contains_point(*transform, point)
                        || clip.is_some_and(|clip| !clip.contains_point(point))
                    {
                        return None;
                    }
                    let local = transform.affine().inverse().transform_point2(point);
                    let fraction = local.y / node.size().y + 0.5;
                    let destination = match row.target {
                        None => TreeDrop::Root,
                        Some(target) if fraction < 0.25 => TreeDrop::Before(target),
                        Some(target) if fraction > 0.75 => TreeDrop::After(target),
                        Some(target) => TreeDrop::Inside(target),
                    };
                    Some((row, destination))
                })
            })
    } else {
        None
    };
    if !valid {
        drag.suppress |= drag.active;
        drag.press = None;
        drag.active = false;
    } else if mouse.just_pressed(MouseButton::Left) {
        drag.press = hit.and_then(|(row, _)| {
            Some(Press {
                room: editor.room,
                source: row.target?,
                origin: cursor?,
            })
        });
    }
    if !drag.active
        && let Some(press) = drag.press
    {
        if cursor.is_some_and(|point| point.distance(press.origin) >= THRESHOLD) {
            drag.active = true;
            drag.suppress = true;
            focus.clear();
        }
    }
    let target = hit.filter(|(row, _)| {
        drag.active
            && drag
                .press
                .is_some_and(|press| row.target != Some(press.source))
    });
    if mouse.just_released(MouseButton::Left) || !mouse.pressed(MouseButton::Left) {
        if mouse.just_released(MouseButton::Left)
            && let Some(press) = drag.press
            && let Some((_, target)) = target
        {
            actions.write(Action::Hierarchy(HierarchyAction::Drop(
                press.room,
                press.source,
                target,
            )));
        }
        drag.active = false;
        drag.press = None;
    }
    let target = target.filter(|_| drag.active);
    let current = target.map(|(row, _)| row.indicator);
    if let Some(entity) = *previous
        && Some(entity) != current
        && let Ok((mut node, _)) = indicators.get_mut(entity)
    {
        node.display = Display::None;
    }
    *previous = current;
    if let Some((row, destination)) = target
        && let Ok((mut node, mut background)) = indicators.get_mut(row.indicator)
    {
        let line = matches!(destination, TreeDrop::Before(_) | TreeDrop::After(_));
        node.display = Display::Flex;
        node.top = if matches!(destination, TreeDrop::After(_)) {
            Val::Auto
        } else {
            px(0)
        };
        node.bottom = if matches!(destination, TreeDrop::Before(_)) {
            Val::Auto
        } else {
            px(0)
        };
        node.height = if line { px(2) } else { Val::Auto };
        node.border = if line {
            UiRect::ZERO
        } else {
            UiRect::all(px(1))
        };
        background.set_if_neq(BackgroundColor(if line {
            FOCUS
        } else {
            FOCUS.with_alpha(0.15)
        }));
    }
}
