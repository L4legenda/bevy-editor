//! Arrange the painting viewports while retaining the editor's camera and UV nodes.
use super::*;
use bevy_scene_format::{Revision, atomic_write, read_limited};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

pub(super) const DIVIDER: f32 = 6.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) enum Side {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    fn direction(self) -> FlexDirection {
        match self {
            Self::Left => FlexDirection::Row,
            Self::Right => FlexDirection::RowReverse,
            Self::Top => FlexDirection::Column,
            Self::Bottom => FlexDirection::ColumnReverse,
        }
    }

    pub(super) fn horizontal(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
        }
    }

    pub(super) fn nearest(rect: Rect, point: Vec2) -> Option<Self> {
        if !rect.contains(point) {
            return None;
        }
        let point = (point - rect.center()) / rect.size();
        Some(if point.x.abs() >= point.y.abs() {
            if point.x < 0. {
                Self::Left
            } else {
                Self::Right
            }
        } else if point.y < 0. {
            Self::Top
        } else {
            Self::Bottom
        })
    }

    pub(super) fn size_from_edge(self, rect: Rect, point: Vec2) -> f32 {
        let (distance, length) = match self {
            Self::Left => (point.x - rect.min.x, rect.width()),
            Self::Right => (rect.max.x - point.x, rect.width()),
            Self::Top => (point.y - rect.min.y, rect.height()),
            Self::Bottom => (rect.max.y - point.y, rect.height()),
        };
        ((distance - DIVIDER * 0.5) / (length - DIVIDER).max(1.)).clamp(0.2, 0.8)
    }

    pub(super) fn region(self, rect: Rect, fraction: f32) -> Rect {
        let mut zone = rect;
        match self {
            Self::Left => zone.max.x = rect.min.x + rect.width() * fraction,
            Self::Right => zone.min.x = rect.max.x - rect.width() * fraction,
            Self::Top => zone.max.y = rect.min.y + rect.height() * fraction,
            Self::Bottom => zone.min.y = rect.max.y - rect.height() * fraction,
        }
        zone
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Layout {
    model_side: Side,
    model_fraction: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            model_side: Side::Left,
            model_fraction: 0.51,
        }
    }
}

impl Layout {
    fn load(path: &Path) -> io::Result<Self> {
        let mut value: Self = serde_json::from_slice(&read_limited(path, 4096)?)?;
        value.model_fraction = value.model_fraction.clamp(0.2, 0.8);
        Ok(value)
    }

    fn save(self, path: &Path) -> io::Result<()> {
        atomic_write(path, &serde_json::to_vec(&self)?, Revision::read(path)?)?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Drag {
    Pane { model: bool, start: Vec2 },
    Divider { before: Layout, grab: Vec2 },
}

#[derive(Resource)]
pub(in crate::editor::ui::model_viewer) struct Panels {
    pub content: Entity,
    pub overlay: Entity,
    pub model: Entity,
    pub model_title: Entity,
    pub uv: Entity,
    pub uv_title: Entity,
    pub divider: Entity,
    pub preview: Entity,
}

#[derive(Resource)]
pub(in crate::editor::ui::model_viewer) struct State {
    path: Option<PathBuf>,
    layout: Layout,
    drag: Option<Drag>,
}

impl State {
    fn open(path: Option<PathBuf>) -> Self {
        let layout = path
            .as_deref()
            .and_then(|path| match Layout::load(path) {
                Ok(layout) => Some(layout),
                Err(error) => {
                    if error.kind() != io::ErrorKind::NotFound {
                        warn!("Could not read paint viewport layout: {error}");
                    }
                    None
                }
            })
            .unwrap_or_default();
        Self {
            path,
            layout,
            drag: None,
        }
    }

    fn save(&self) {
        if let Some(path) = &self.path
            && let Err(error) = self.layout.save(path)
        {
            warn!("Could not save paint viewport layout: {error}");
        }
    }

    fn cancel_drag(&mut self) {
        if let Some(Drag::Divider { before, .. }) = self.drag.take() {
            self.layout = before;
        }
    }
}

pub(super) fn setup(commands: &mut Commands, panels: Panels) {
    let path = presets::directory()
        .and_then(|folder| folder.parent().map(|root| root.join("paint-layout.json")));
    commands.insert_resource(State::open(path));
    commands.insert_resource(panels);
}

pub(in crate::editor::ui::model_viewer) fn update(
    panels: Res<Panels>,
    mut state: ResMut<State>,
    editor: Res<Editor>,
    paint: Res<PaintState>,
    maker: Res<ShaderMaker>,
    drawer: Res<DrawerMotion>,
    mut actions: MessageReader<Action>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    mut nodes: Query<&mut Node>,
    mut colors: Query<&mut BackgroundColor>,
) {
    let reset = actions.read().any(|action| {
        matches!(
            action,
            Action::ModelView(ModelViewAction::Paint(PaintAction::Reset))
        )
    });
    let visible = |entity| {
        nodes
            .get(entity)
            .is_ok_and(|node| node.display != Display::None)
    };
    let showing = visible(panels.model_title) || visible(panels.uv);
    let split = visible(panels.divider);
    if showing && reset {
        state.cancel_drag();
        if state.layout != Layout::default() {
            state.layout = Layout::default();
            state.save();
        }
    }

    let rect = |entity| {
        let (node, transform) = geometry.get(entity).ok()?;
        engine::node_rect(node, transform)
    };
    let mut target = None;
    let mut handle_hot = false;
    let workspace = rect(panels.model)
        .zip(rect(panels.uv))
        .map(|(model, uv)| model.union(uv));
    if split
        && window.focused
        && !editor.browser_popup
        && !editor.hdri_library_open
        && !paint.blocks_view_navigation(editor.viewing_model, maker.shows_model())
        && let Some((point, workspace)) = window.cursor_position().zip(workspace)
    {
        let divider = rect(panels.divider).filter(|r| r.contains(point));
        handle_hot = divider.is_some() || matches!(state.drag, Some(Drag::Divider { .. }));
        if !reset && mouse.just_pressed(MouseButton::Left) && !drawer.covers(point, &window) {
            state.drag = if let Some(divider) = divider {
                Some(Drag::Divider {
                    before: state.layout,
                    grab: point - divider.center(),
                })
            } else {
                [(true, panels.model_title), (false, panels.uv_title)]
                    .into_iter()
                    .find_map(|(model, title)| {
                        rect(title)
                            .filter(|r| r.contains(point))
                            .map(|_| Drag::Pane {
                                model,
                                start: point,
                            })
                    })
            };
        }
        match state.drag {
            Some(Drag::Pane { model, start })
                if crate::editor::area_selection::dragged(start, point) =>
            {
                if let Some(side) = Side::nearest(workspace, point) {
                    let model_side = if model { side } else { side.opposite() };
                    let fraction = if model {
                        state.layout.model_fraction
                    } else {
                        1. - state.layout.model_fraction
                    };
                    target = Some(side.region(workspace, fraction));
                    if mouse.just_released(MouseButton::Left)
                        && state.layout.model_side != model_side
                    {
                        state.layout.model_side = model_side;
                        state.save();
                    }
                }
            }
            Some(Drag::Divider { before, grab }) => {
                if mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left) {
                    state.layout.model_fraction =
                        before.model_side.size_from_edge(workspace, point - grab);
                    if mouse.just_released(MouseButton::Left) && state.layout != before {
                        state.save();
                    }
                }
            }
            _ => {}
        }
        if mouse.just_released(MouseButton::Left) {
            state.drag = None;
            target = None;
        } else if !mouse.pressed(MouseButton::Left) {
            state.cancel_drag();
            target = None;
        }
    } else {
        state.cancel_drag();
    }

    let side = state.layout.model_side;
    let direction = if split {
        side.direction()
    } else {
        FlexDirection::Row
    };
    if let Ok(mut content) = nodes.get_mut(panels.content)
        && content.flex_direction != direction
    {
        content.flex_direction = direction;
    }
    for (entity, fraction) in [
        (panels.model, state.layout.model_fraction),
        (panels.uv, 1. - state.layout.model_fraction),
    ] {
        if let Ok(mut panel) = nodes.get_mut(entity) {
            let grow = if split { fraction } else { 1. };
            if panel.flex_grow != grow {
                panel.flex_grow = grow;
            }
        }
    }
    if let Ok(mut divider) = nodes.get_mut(panels.divider) {
        let size = if side.horizontal() {
            (px(DIVIDER), percent(100))
        } else {
            (percent(100), px(DIVIDER))
        };
        if (divider.width, divider.height) != size {
            (divider.width, divider.height) = size;
        }
    }
    if let Ok(mut color) = colors.get_mut(panels.divider) {
        color.set_if_neq(BackgroundColor(if handle_hot { FOCUS } else { CHROME }));
    }
    if let Ok(mut node) = nodes.get_mut(panels.preview) {
        if let Some((target, overlay)) = target.zip(rect(panels.overlay)) {
            let start = target.min - overlay.min;
            node.set_if_neq(Node {
                position_type: PositionType::Absolute,
                left: px(start.x),
                top: px(start.y),
                width: px(target.width()),
                height: px(target.height()),
                border: UiRect::all(px(1.)),
                ..default()
            });
        } else if node.display != Display::None {
            node.display = Display::None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{
        model_document::Mode,
        selection_outline::render_tests::frame,
        tests::lighting_ui_render::{fixture_project, mouse, move_pointer},
    };
    use bevy::input::ButtonState;

    #[test]
    fn paint_dock_restores_the_saved_split_and_validates_its_size() {
        let folder =
            std::env::temp_dir().join(format!("bevy-paint-layout-{}", uuid::Uuid::new_v4()));
        let path = folder.join("layout.json");
        let custom = Layout {
            model_side: Side::Bottom,
            model_fraction: 0.71,
        };
        custom.save(&path).unwrap();
        assert_eq!(State::open(Some(path.clone())).layout, custom);
        let outside = Layout {
            model_fraction: 20.,
            ..custom
        };
        outside.save(&path).unwrap();
        assert_eq!(Layout::load(&path).unwrap().model_fraction, 0.8);
        Layout::default().save(&path).unwrap();
        assert_eq!(Layout::load(&path).unwrap(), Layout::default());
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    #[ignore = "requires GPU; drives viewport headers and the split divider through the real UI"]
    fn gpu_paint_dock_arranges_swaps_and_resizes_viewports() {
        let project = crate::EditorProject::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        let (mut app, window, _target) = fixture_project(project);
        *app.world_mut().resource_mut::<State>() = State::open(None);
        app.world_mut().resource_mut::<Editor>().drawer = false;
        app.world_mut().write_message(Action::ViewModel(0));
        for _ in 0..240 {
            frame(&mut app);
            if app
                .world()
                .resource::<editing::State>()
                .documents
                .contains_key(&0)
            {
                break;
            }
        }
        app.world_mut()
            .write_message(Action::ModelView(ModelViewAction::Edit(
                editing::EditAction::Mode(Mode::Paint),
            )));
        for _ in 0..8 {
            frame(&mut app);
        }

        let panel_rect = |app: &App, entity: Entity| {
            let world = app.world();
            engine::node_rect(
                world.get::<ComputedNode>(entity).unwrap(),
                world.get::<UiGlobalTransform>(entity).unwrap(),
            )
            .unwrap()
        };
        let (model, uv, model_title, uv_title, split) = {
            let panels = app.world().resource::<Panels>();
            (
                panels.model,
                panels.uv,
                panels.model_title,
                panels.uv_title,
                panels.divider,
            )
        };
        let left = panel_rect(&app, model);
        let right = panel_rect(&app, uv);
        assert!(left.max.x <= right.min.x);
        let workspace = left.union(right);
        let drag = |app: &mut App, entity, to| {
            let from = panel_rect(app, entity).center();
            move_pointer(app, window, from);
            mouse(app, window, ButtonState::Pressed);
            move_pointer(app, window, to);
            mouse(app, window, ButtonState::Released);
            for _ in 0..4 {
                frame(app);
            }
        };
        drag(
            &mut app,
            model_title,
            Vec2::new(workspace.center().x, workspace.min.y + 12.),
        );
        assert!(panel_rect(&app, model).max.y <= panel_rect(&app, uv).min.y + 1.);
        drag(
            &mut app,
            uv_title,
            Vec2::new(workspace.min.x + 12., workspace.center().y),
        );
        assert!(panel_rect(&app, uv).max.x <= panel_rect(&app, model).min.x + 1.);
        drag(
            &mut app,
            split,
            Vec2::new(
                workspace.min.x + workspace.width() * 0.35,
                workspace.center().y,
            ),
        );
        let fraction = app.world().resource::<State>().layout.model_fraction;
        assert!((fraction - 0.65).abs() < 0.02);
        let send = |app: &mut App, action| {
            app.world_mut()
                .write_message(Action::ModelView(ModelViewAction::Paint(action)));
            for _ in 0..4 {
                frame(app);
            }
        };
        send(&mut app, PaintAction::View(View::Two));
        assert!(panel_rect(&app, uv).width() > workspace.width() - 2.);
        assert_eq!(
            app.world().get::<Node>(model).unwrap().display,
            Display::None
        );
        send(&mut app, PaintAction::View(View::Split));
        assert!(panel_rect(&app, uv).max.x <= panel_rect(&app, model).min.x + 1.);
        send(&mut app, PaintAction::Reset);
        assert_eq!(app.world().resource::<State>().layout, Layout::default());
        assert!(panel_rect(&app, model).max.x <= panel_rect(&app, uv).min.x + 1.);
    }
}
