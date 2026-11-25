//! Drag feedback and pointer gestures over clipped outliner rows.
use super::*;
use crate::{editor::scene_hierarchy::HierarchyAction, hierarchy::TreeDrop};
use bevy::{
    ecs::system::ParamSet,
    ui::{CalculatedClip, UiTransform},
};
use std::time::Instant;

const THRESHOLD: f32 = 6.0;
const CARD_WIDTH: f32 = 218.0;
const CARD_HEIGHT: f32 = 28.0;

#[derive(Clone, Copy)]
struct Press {
    room: Room,
    source: Selection,
    glyph: icons::Glyph,
    origin: Vec2,
}

#[derive(Clone, Copy)]
struct Target {
    indicator: Entity,
    drop: TreeDrop<Selection>,
}

#[derive(Resource, Default)]
pub(in crate::editor) struct TreeDrag {
    press: Option<Press>,
    active: bool,
    suppress: bool,
    target: Option<Target>,
    released: Option<Instant>,
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
    item: Option<(Selection, icons::Glyph)>,
    indicator: Entity,
}

impl Row {
    fn selection(&self) -> Option<Selection> {
        self.item.map(|(selection, _)| selection)
    }
}

#[derive(Component)]
pub(in crate::editor) struct Indicator;

pub(super) fn mark(
    commands: &mut Commands,
    item: Entity,
    source: Option<(Selection, icons::Glyph)>,
) {
    let indicator = commands
        .spawn((
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                border_radius: BorderRadius::all(px(4)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            BorderColor::all(Color::NONE),
            ZIndex(2),
            Pickable::IGNORE,
            Indicator,
            ChildOf(item),
        ))
        .id();
    commands.entity(item).insert(Row {
        item: source,
        indicator,
    });
}

#[derive(Resource)]
pub(in crate::editor) struct CardUi {
    card: Entity,
    icon: Entity,
    label: Entity,
}

pub(super) fn setup(commands: &mut Commands, root: Entity, font: &Handle<Font>) {
    let card = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            max_width: px(CARD_WIDTH),
            height: px(CARD_HEIGHT),
            padding: UiRect::horizontal(px(9)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(7)),
            overflow: Overflow::clip(),
            ..row(6.0)
        },
        PANEL,
    );
    commands.entity(card).insert((
        GlobalZIndex(88),
        Pickable::IGNORE,
        BorderColor::all(FOCUS),
        BoxShadow::default(),
    ));
    let icon = icons::spawn(commands, card, icons::Glyph::Box, 14.0, FOCUS);
    let label = text(commands, card, font, "", 12.0, TEXT);
    commands.entity(label).insert((
        Pickable::IGNORE,
        TextLayout::no_wrap(),
        Node {
            min_width: px(0),
            overflow: Overflow::clip(),
            ..default()
        },
    ));
    commands.insert_resource(CardUi { card, icon, label });
}

type Geometry<'a> = (
    &'a ComputedNode,
    &'a UiGlobalTransform,
    Option<&'a CalculatedClip>,
);
type TreeRows<'a> = (
    &'a Row,
    &'a ComputedNode,
    &'a UiGlobalTransform,
    Option<&'a CalculatedClip>,
);

fn contains(
    node: &ComputedNode,
    transform: UiGlobalTransform,
    clip: Option<&CalculatedClip>,
    point: Vec2,
) -> bool {
    !node.is_empty()
        && node.contains_point(transform, point)
        && clip.is_none_or(|clip| clip.contains_point(point))
}

fn drop_at(row: &Row, y: f32, height: f32, previous: Option<Target>) -> TreeDrop<Selection> {
    let Some(target) = row.selection() else {
        return TreeDrop::Root;
    };
    let quarter = height * 0.25;
    let hysteresis = height * 0.11;
    let zone = previous
        .filter(|p| p.indicator == row.indicator)
        .map(|p| p.drop);
    let (before, after) = match zone {
        Some(TreeDrop::Before(_)) => (-quarter + hysteresis, quarter),
        Some(TreeDrop::After(_)) => (-quarter, quarter - hysteresis),
        Some(TreeDrop::Inside(_)) => (-quarter - hysteresis, quarter + hysteresis),
        _ => (-quarter, quarter),
    };
    if y < before {
        TreeDrop::Before(target)
    } else if y > after {
        TreeDrop::After(target)
    } else {
        TreeDrop::Inside(target)
    }
}

pub(in crate::editor) fn update(
    editor: Res<Editor>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    assets: Res<AssetDrag>,
    lighting: Res<LightingUi>,
    mut focus: ResMut<InputFocus>,
    mut drag: ResMut<TreeDrag>,
    rows: Query<TreeRows>,
    mut actions: MessageWriter<Action>,
    mut list: Query<(Geometry, &mut ScrollPosition), With<sidebar_resize::SceneTree>>,
    time: Res<Time<Real>>,
    mut edge_time: Local<f32>,
) {
    let pressed = mouse.just_pressed(MouseButton::Left);
    let released = mouse.just_released(MouseButton::Left);
    if pressed || !mouse.pressed(MouseButton::Left) && !released {
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
        && !editor.sidebar.tree_collapsed()
        && !assets.active()
        && !lighting.is_menu_open()
        && window.focused
        && !keys.just_pressed(KeyCode::Escape)
        && drag.press.is_none_or(|press| press.room == editor.room);
    let hit = if valid && (drag.press.is_some() || pressed) {
        cursor
            .map(|point| point * window.scale_factor())
            .and_then(|point| {
                rows.iter().find_map(|(row, node, transform, clip)| {
                    if !contains(node, *transform, clip, point) {
                        return None;
                    }
                    let local = transform.affine().inverse().transform_point2(point);
                    Some((row, drop_at(row, local.y, node.size().y, drag.target)))
                })
            })
    } else {
        None
    };

    if !valid {
        drag.suppress |= drag.active;
        drag.press = None;
        drag.active = false;
        drag.released = None;
    } else if pressed {
        drag.released = None;
        drag.press = hit.and_then(|(row, _)| {
            let (source, glyph) = row.item?;
            Some(Press {
                room: editor.room,
                source,
                glyph,
                origin: cursor?,
            })
        });
    }
    if !drag.active
        && let Some(press) = drag.press
        && cursor.is_some_and(|point| point.distance(press.origin) >= THRESHOLD)
    {
        drag.active = true;
        drag.suppress = true;
        focus.clear();
    }
    drag.target = if drag.active {
        drag.press.zip(hit).and_then(|(press, (row, drop))| {
            (row.selection() != Some(press.source) && editor.can_drop_in_tree(press.source, drop))
                .then_some(Target {
                    indicator: row.indicator,
                    drop,
                })
        })
    } else {
        None
    };
    if !mouse.pressed(MouseButton::Left) || released {
        if released
            && let Some(press) = drag.press
            && let Some(target) = drag.target
        {
            drag.released = Some(Instant::now());
            actions.write(Action::Hierarchy(HierarchyAction::Drop(
                press.room,
                press.source,
                target.drop,
            )));
        }
        drag.active = false;
        drag.press = None;
        drag.target = None;
    }

    let mut edge = 0.0;
    if drag.active
        && let Some(point) = cursor.map(|p| p * window.scale_factor())
        && let Ok(((node, transform, clip), mut scroll)) = list.single_mut()
        && contains(node, *transform, clip, point)
    {
        let half = node.size() * 0.5;
        let relative = (point - transform.affine().translation).y * node.inverse_scale_factor();
        let half_height = half.y * node.inverse_scale_factor();
        let zone = 28.0_f32.min(half_height);
        if zone > 0.0 {
            let strength = (1.0 - (half_height - relative.abs()) / zone).clamp(0.0, 1.0);
            edge = relative.signum() * strength;
            if *edge_time >= 0.15 && edge != 0.0 {
                let max = ((node.content_size().y - node.size().y) * node.inverse_scale_factor())
                    .max(0.0);
                scroll.y = (scroll.y + edge * 420.0 * time.delta_secs()).clamp(0.0, max);
            }
        }
    }
    *edge_time = if edge == 0.0 {
        0.0
    } else {
        *edge_time + time.delta_secs()
    };
}

struct Flight {
    press: Press,
    position: Vec2,
    elapsed: f32,
    opacity: f32,
    landing: Option<Landing>,
}

struct Landing {
    target: Selection,
    start: Vec2,
    opacity: f32,
    accepted: bool,
    elapsed: f32,
}

#[derive(Default)]
pub(in crate::editor) struct Animation {
    flight: Option<Flight>,
    previous: Vec<Entity>,
    target: Option<(Entity, TreeDrop<Selection>)>,
    highlight: f32,
}

pub(in crate::editor) fn animate(
    mut commands: Commands,
    editor: Res<Editor>,
    drag: Res<TreeDrag>,
    ui: Res<CardUi>,
    window: Single<&Window, With<PrimaryWindow>>,
    time: Res<Time<Real>>,
    rows: Query<TreeRows>,
    mut animation: Local<Animation>,
    mut visuals: ParamSet<(
        Query<(
            &mut Node,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut UiTransform,
            &mut BoxShadow,
            &ComputedNode,
        )>,
        Query<(&mut Node, &mut BackgroundColor, &mut BorderColor), With<Indicator>>,
    )>,
    mut labels: Query<(&mut Text, &mut TextColor)>,
) {
    let dt = time.delta_secs().min(0.05);
    let card_width = visuals.p0().get(ui.card).map_or(0.0, |(.., node)| {
        node.size().x * node.inverse_scale_factor()
    });
    let Animation {
        flight,
        previous,
        target,
        highlight,
    } = &mut *animation;
    let active = drag.press.filter(|_| drag.active);
    if let Some(press) = active {
        if flight
            .as_ref()
            .is_none_or(|f| f.landing.is_some() || f.press.source != press.source)
        {
            *flight = Some(Flight {
                press,
                position: press.origin,
                elapsed: 0.0,
                opacity: 0.0,
                landing: None,
            });
            if let Ok((mut label, _)) = labels.get_mut(ui.label) {
                label.0 = editor.label(press.source);
            }
            icons::set_glyph(&mut commands, ui.icon, press.glyph);
        }
    } else if let Some(flight) = flight.as_mut().filter(|f| f.landing.is_none()) {
        let accepted = editor.tree_drop_arrival.filter(|&(room, source, _, when)| {
            room == flight.press.room
                && source == flight.press.source
                && drag.released.is_some_and(|release| when >= release)
        });
        flight.landing = Some(Landing {
            target: accepted.map_or(flight.press.source, |(_, _, result, _)| result),
            start: flight.position,
            opacity: flight.opacity,
            accepted: accepted.is_some(),
            elapsed: 0.0,
        });
    }

    let locate = |selection| {
        rows.iter().find_map(|(row, node, transform, clip)| {
            let center = transform.affine().translation;
            (row.selection() == Some(selection) && contains(node, *transform, clip, center)).then(
                || {
                    let scale = node.inverse_scale_factor();
                    let position = center * scale
                        - Vec2::new(node.size().x * scale, CARD_HEIGHT) * 0.5
                        + Vec2::X * card_width;
                    (row, position)
                },
            )
        })
    };
    let current = drag.target.map(|t| (t.indicator, t.drop));
    if *target != current {
        *target = current;
        *highlight = 0.0;
    }
    *highlight = (*highlight + dt / 0.08).min(1.0);

    // Reset only the rows that received drag feedback last frame.
    let mut indicators = visuals.p1();
    for entity in previous.drain(..) {
        if let Ok((mut node, _, _)) = indicators.get_mut(entity) {
            node.display = Display::None;
        }
    }
    let mut paint = |entity, zone: Option<TreeDrop<Selection>>, fill: Color, border: Color| {
        if let Ok((mut node, mut background, mut edge)) = indicators.get_mut(entity) {
            let before = matches!(zone, Some(TreeDrop::Before(_)));
            let after = matches!(zone, Some(TreeDrop::After(_)));
            node.display = Display::Flex;
            node.top = if after { Val::Auto } else { px(0) };
            node.bottom = if before { Val::Auto } else { px(0) };
            node.height = if before || after { px(2) } else { Val::Auto };
            node.border = if before || after {
                UiRect::ZERO
            } else {
                UiRect::all(px(1))
            };
            background.set_if_neq(BackgroundColor(fill));
            edge.set_if_neq(BorderColor::all(border));
            previous.push(entity);
        }
    };

    let mut visible = None;
    if let Some(f) = flight
        .as_mut()
        .filter(|f| f.press.room == editor.room && !editor.asset_viewer_open())
    {
        if let Some(landing) = &mut f.landing {
            let travel = if landing.accepted { 0.15 } else { 0.17 };
            let progress = (landing.elapsed / travel).min(1.0);
            let end = locate(landing.target);
            let destination = end
                .as_ref()
                .map_or(f.press.origin, |(_, position)| *position);
            let eased = 1.0 - (1.0 - progress).powi(3);
            if progress < 1.0 {
                visible = Some((
                    landing.start.lerp(destination, eased),
                    landing.opacity * (1.0 - progress),
                    1.0 - 0.04 * progress,
                ));
            }
            if landing.accepted
                && let Some((row, _)) = end
            {
                let glow = (1.0 - landing.elapsed / 0.34).max(0.0).powi(2);
                if glow > 0.0 {
                    paint(
                        row.indicator,
                        None,
                        FOCUS.with_alpha(0.23 * glow),
                        FOCUS.with_alpha(0.72 * glow),
                    );
                }
            } else if progress < 1.0
                && let Some((row, _)) = end
            {
                paint(
                    row.indicator,
                    None,
                    PANEL.with_alpha(0.56 * (1.0 - progress)),
                    Color::NONE,
                );
            }
            landing.elapsed += dt;
        } else {
            f.elapsed += dt;
            if let Some(cursor) = window.cursor_position() {
                f.position = Vec2::new(
                    cursor.x - 12.0,
                    (cursor.y + 10.0).min((window.height() - CARD_HEIGHT - 4.0).max(4.0)),
                );
            }
            let appear = 1.0 - (1.0 - (f.elapsed / 0.10).min(1.0)).powi(3);
            let opacity = if drag.target.is_some() { 1.0 } else { 0.57 };
            f.opacity += (opacity * appear - f.opacity) * (dt * 30.0).min(1.0);
            visible = Some((f.position, f.opacity, 0.97 + 0.03 * appear));
            if let Some((row, _)) = locate(f.press.source) {
                paint(
                    row.indicator,
                    None,
                    PANEL.with_alpha(0.56 * appear),
                    Color::NONE,
                );
            }
            if let Some((root, ..)) = rows
                .iter()
                .find(|(row, node, ..)| row.item.is_none() && !node.is_empty())
                && drag
                    .target
                    .is_none_or(|target| root.indicator != target.indicator)
            {
                paint(
                    root.indicator,
                    None,
                    FOCUS.with_alpha(0.05 * appear),
                    FOCUS.with_alpha(0.27 * appear),
                );
            }
            if let Some(target) = drag.target {
                let line = matches!(target.drop, TreeDrop::Before(_) | TreeDrop::After(_));
                let fill = if line { 1.0 } else { 0.14 };
                paint(
                    target.indicator,
                    Some(target.drop),
                    FOCUS.with_alpha(fill * *highlight),
                    FOCUS.with_alpha(*highlight),
                );
            }
        }
    }
    if flight.as_ref().is_some_and(|f| {
        f.press.room != editor.room
            || editor.asset_viewer_open()
            || f.landing
                .as_ref()
                .is_some_and(|landing| landing.elapsed > 0.34)
    }) {
        *flight = None;
    }
    drop(indicators);

    if let Ok((mut node, mut background, mut border, mut transform, mut shadow, _)) =
        visuals.p0().get_mut(ui.card)
    {
        if let Some((position, opacity, scale)) = visible {
            node.display = Display::Flex;
            node.right = px((window.width() - position.x)
                .clamp(0.0, (window.width() - card_width - 4.0).max(0.0)));
            node.top = px(position.y);
            transform.scale = Vec2::splat(scale);
            background.set_if_neq(BackgroundColor(PANEL.with_alpha(0.95 * opacity)));
            border.set_if_neq(BorderColor::all(FOCUS.with_alpha(0.72 * opacity)));
            *shadow = BoxShadow::new(
                Color::BLACK.with_alpha(0.48 * opacity),
                px(0),
                px(4),
                px(0),
                px(11),
            );
            icons::set_tint(&mut commands, ui.icon, FOCUS.with_alpha(opacity));
            if let Ok((_, mut color)) = labels.get_mut(ui.label) {
                color.set_if_neq(TextColor(TEXT.with_alpha(opacity)));
            }
        } else if node.display != Display::None {
            node.display = Display::None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{
        scene_hierarchy::tests::fixture,
        selection_outline::render_tests::{capture_when, frame},
        tests::lighting_ui_render::{fixture_project, mouse, move_pointer, scroll_to_entity},
    };
    use bevy::input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    };
    use std::path::PathBuf;

    fn center(app: &mut App, selection: Option<Selection>) -> Vec2 {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &Row)>();
        let entity = query
            .iter(world)
            .find(|(_, row)| row.selection() == selection)
            .unwrap()
            .0;
        scroll_to_entity(app, entity);
        app.world()
            .get::<UiGlobalTransform>(entity)
            .unwrap()
            .affine()
            .translation
    }

    #[test]
    #[ignore = "requires GPU adapter; exercises drag, cancellation, and reparenting in the real outliner"]
    fn gpu_scene_tree_drag_shows_destination_and_settles_or_returns() {
        let files = fixture();
        let (mut app, window, target) = fixture_project(files.project.clone());
        let (group, camera) = {
            let editor = app.world().resource::<Editor>();
            let graph = editor.edits.hierarchy.as_ref().unwrap();
            (
                graph.by_id("room").unwrap().key,
                graph.by_id("camera").unwrap().key,
            )
        };
        let source = Selection::Camera(0);
        let destination = Selection::Node(group);
        let before = app.world().resource::<Editor>().edits.clone();
        let card = app.world().resource::<CardUi>().card;
        let is_visible =
            |app: &App| app.world().get::<Node>(card).unwrap().display == Display::Flex;
        let start = |app: &mut App, source, destination| {
            let from = center(app, Some(source));
            let to = center(app, destination);
            move_pointer(app, window, from);
            mouse(app, window, ButtonState::Pressed);
            move_pointer(app, window, to);
            assert!(is_visible(app));
        };
        let escape = |app: &mut App| {
            for state in [ButtonState::Pressed, ButtonState::Released] {
                app.world_mut().write_message(KeyboardInput {
                    window,
                    key_code: KeyCode::Escape,
                    logical_key: Key::Escape,
                    state,
                    text: None,
                    repeat: false,
                });
                frame(app);
            }
        };

        start(&mut app, source, Some(destination));
        let drop_target = app.world().resource::<TreeDrag>().target.unwrap();
        assert_eq!(drop_target.drop, TreeDrop::Inside(destination));
        assert_eq!(
            app.world()
                .get::<Node>(drop_target.indicator)
                .unwrap()
                .display,
            Display::Flex
        );
        if let Some(path) = std::env::var_os("BEVY_TREE_DRAG_SCREENSHOT") {
            capture_when(&mut app, &target, &PathBuf::from(path), |_| true);
        }
        escape(&mut app);
        mouse(&mut app, window, ButtonState::Released);
        assert!(is_visible(&app), "The card should ease back on cancel");
        for _ in 0..24 {
            frame(&mut app);
        }
        assert!(!is_visible(&app));
        assert_eq!(app.world().resource::<Editor>().edits, before);

        start(&mut app, source, Some(destination));
        mouse(&mut app, window, ButtonState::Released);
        assert!(
            is_visible(&app),
            "The card should settle before disappearing"
        );
        let editor = app.world().resource::<Editor>();
        let graph = editor.edits.hierarchy.as_ref().unwrap();
        assert_eq!(graph.parent(camera).unwrap().key, group);
        assert_eq!(editor.undo.len(), 1);
        for _ in 0..24 {
            frame(&mut app);
        }
        assert!(!is_visible(&app));

        start(&mut app, destination, Some(source));
        assert!(app.world().resource::<TreeDrag>().target.is_none());
        mouse(&mut app, window, ButtonState::Released);
        assert_eq!(app.world().resource::<Editor>().undo.len(), 1);

        start(&mut app, source, None);
        assert_eq!(
            app.world().resource::<TreeDrag>().target.unwrap().drop,
            TreeDrop::Root
        );
        mouse(&mut app, window, ButtonState::Released);
        let editor = app.world().resource::<Editor>();
        assert!(
            editor
                .edits
                .hierarchy
                .as_ref()
                .unwrap()
                .parent(camera)
                .is_none()
        );
        assert!(!editor.sidebar.tree_collapsed());
    }
}
