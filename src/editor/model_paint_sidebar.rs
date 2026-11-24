//! Dock the painting sidebar panels without rebuilding their contents on a move.
use super::{
    dock::{DIVIDER, Side},
    *,
};
use bevy_scene_format::{Revision, atomic_write, read_limited};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, io, path::PathBuf};

pub(super) const PANELS: [Panel; 5] = [
    Panel::Layers,
    Panel::Maps,
    Panel::Sets,
    Panel::Preview,
    Panel::Shader,
];

pub(super) fn panel_index(panel: Panel) -> usize {
    PANELS.iter().position(|&p| p == panel).unwrap()
}

#[derive(Component)]
pub(in crate::editor::ui) struct PanelBody(pub Panel);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
enum Tab {
    Panel(Panel),
    Properties,
    Shelf,
    NodeList,
    Code,
    ShaderPreview,
    GraphPreview,
    Graph,
    NodeSettings,
}

impl Tab {
    fn index(self) -> usize {
        match self {
            Self::Panel(panel) => panel_index(panel),
            Self::Properties => PANELS.len(),
            Self::Shelf => PANELS.len() + 1,
            Self::NodeList => 7,
            Self::Code => 8,
            Self::ShaderPreview => 9,
            Self::GraphPreview => 10,
            Self::Graph => 11,
            Self::NodeSettings => 12,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Panel(Panel::Layers) => "Слои",
            Self::Panel(Panel::Maps) => "Карты",
            Self::Panel(Panel::Sets) => "Наборы",
            Self::Panel(Panel::Preview) => "Просмотр",
            Self::Panel(Panel::Shader) => "WGSL",
            Self::Properties => "Свойства",
            Self::Shelf => "Библиотека",
            Self::NodeList => "Узлы",
            Self::Code => "Код WGSL",
            Self::ShaderPreview => "Предпросмотр WGSL",
            Self::GraphPreview => "Модель",
            Self::Graph => "Холст",
            Self::NodeSettings => "Параметры узлов",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Layout {
    Group {
        tabs: Vec<Tab>,
        active: Tab,
    },
    Split {
        horizontal: bool,
        fraction: f32,
        children: Box<[Layout; 2]>,
    },
}

impl Default for Layout {
    fn default() -> Self {
        Self::Split {
            horizontal: false,
            fraction: 0.43,
            children: Box::new([
                Self::Group {
                    tabs: PANELS.map(Tab::Panel).to_vec(),
                    active: Tab::Panel(Panel::Layers),
                },
                Self::single(Tab::Properties),
            ]),
        }
    }
}

impl Layout {
    #[cfg(test)]
    fn valid(&self) -> bool {
        self.valid_for(Workspace::Paint)
    }

    fn single(tab: Tab) -> Self {
        Self::Group {
            tabs: vec![tab],
            active: tab,
        }
    }

    fn walk(&self, visit: &mut impl FnMut(&[Tab], Tab)) {
        match self {
            Self::Group { tabs, active } => visit(tabs, *active),
            Self::Split { children, .. } => children.iter().for_each(|child| child.walk(visit)),
        }
    }

    fn valid_for(&self, workspace: Workspace) -> bool {
        let mut seen = HashSet::new();
        let mut valid = true;
        let mut remaining = vec![self];
        while let Some(node) = remaining.pop() {
            if let Self::Split {
                fraction, children, ..
            } = node
            {
                if !(0.2..=0.8).contains(fraction) {
                    return false;
                }
                remaining.extend(children.iter());
            }
        }
        self.walk(&mut |tabs, active| {
            valid &= tabs.contains(&active) && tabs.iter().all(|&tab| seen.insert(tab));
        });
        valid
            && match workspace {
                Workspace::Shader => {
                    seen == HashSet::from([Tab::NodeList, Tab::Code, Tab::ShaderPreview])
                }
                Workspace::Graph => {
                    seen == HashSet::from([Tab::GraphPreview, Tab::Graph, Tab::NodeSettings])
                }
                Workspace::Paint => {
                    PANELS
                        .iter()
                        .all(|&panel| seen.contains(&Tab::Panel(panel)))
                        && seen.contains(&Tab::Properties)
                        && seen.len() == PANELS.len() + 1 + usize::from(seen.contains(&Tab::Shelf))
                }
            }
    }

    fn shader() -> Self {
        Self::Split {
            horizontal: true,
            fraction: 0.2,
            children: Box::new([
                Self::single(Tab::NodeList),
                Self::Split {
                    horizontal: true,
                    fraction: 0.7,
                    children: Box::new([Self::single(Tab::Code), Self::single(Tab::ShaderPreview)]),
                },
            ]),
        }
    }

    fn graph() -> Self {
        Self::Split {
            horizontal: true,
            fraction: 0.24,
            children: Box::new([
                Self::single(Tab::GraphPreview),
                Self::Split {
                    horizontal: true,
                    fraction: 0.70,
                    children: Box::new([Self::single(Tab::Graph), Self::single(Tab::NodeSettings)]),
                },
            ]),
        }
    }

    fn group(&mut self, tab: Tab) -> Option<&mut Self> {
        match self {
            Self::Group { tabs, .. } if tabs.contains(&tab) => Some(self),
            Self::Group { .. } => None,
            Self::Split { children, .. } => children.iter_mut().find_map(|child| child.group(tab)),
        }
    }

    fn activate(&mut self, tab: Tab) {
        if let Some(Self::Group { active, .. }) = self.group(tab) {
            *active = tab;
        }
    }

    fn contains(&self, tab: Tab) -> bool {
        let mut found = false;
        self.walk(&mut |tabs, _| found |= tabs.contains(&tab));
        found
    }

    fn split_at(&self, path: &[usize]) -> Option<(&[Layout; 2], f32)> {
        let Self::Split {
            children, fraction, ..
        } = self
        else {
            return None;
        };
        match path.split_first() {
            Some((&next, tail)) => children[next].split_at(tail),
            None => Some((children, *fraction)),
        }
    }

    fn set_fraction(&mut self, path: &[usize], value: f32) {
        if let Self::Split {
            children, fraction, ..
        } = self
        {
            if let Some((&next, tail)) = path.split_first() {
                children[next].set_fraction(tail, value);
            } else {
                *fraction = value;
            }
        }
    }

    fn minimum_size(&self) -> Vec2 {
        match self {
            Self::Group { .. } => Vec2::new(82., 70.),
            Self::Split {
                horizontal,
                children,
                ..
            } => {
                let first = children[0].minimum_size();
                let second = children[1].minimum_size();
                if *horizontal {
                    Vec2::new(first.x + DIVIDER + second.x, first.y.max(second.y))
                } else {
                    Vec2::new(first.x.max(second.x), first.y + DIVIDER + second.y)
                }
            }
        }
    }

    fn without(self, tab: Tab) -> Option<Self> {
        match self {
            Self::Group {
                mut tabs,
                mut active,
            } => {
                if let Some(index) = tabs.iter().position(|&t| t == tab) {
                    tabs.remove(index);
                    if active == tab {
                        active = *tabs.get(index.min(tabs.len().saturating_sub(1)))?;
                    }
                }
                Some(Self::Group { tabs, active })
            }
            Self::Split {
                horizontal,
                fraction,
                children,
            } => {
                let [a, b] = *children;
                match (a.without(tab), b.without(tab)) {
                    (Some(a), Some(b)) => Some(Self::Split {
                        horizontal,
                        fraction,
                        children: Box::new([a, b]),
                    }),
                    (other, None) | (None, other) => other,
                }
            }
        }
    }

    fn move_tab(&self, tab: Tab, target: Tab, placement: Placement) -> Option<Self> {
        if tab == target {
            return None;
        }
        let mut layout = self.clone().without(tab)?;
        let group = layout.group(target)?;
        match placement {
            Placement::Tabs { before } => {
                let Self::Group { tabs, active } = group else {
                    return None;
                };
                let index = before
                    .and_then(|before| tabs.iter().position(|&t| t == before))
                    .unwrap_or(tabs.len());
                tabs.insert(index, tab);
                *active = tab;
            }
            Placement::Edge(side) => {
                let moved = Self::single(tab);
                let previous = std::mem::replace(group, Self::single(target));
                let children = if matches!(side, Side::Top | Side::Left) {
                    [moved, previous]
                } else {
                    [previous, moved]
                };
                *group = Self::Split {
                    horizontal: side.horizontal(),
                    fraction: 0.5,
                    children: Box::new(children),
                };
            }
        }
        (layout != *self).then_some(layout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{
        selection_outline::render_tests::frame,
        tests::lighting_ui_render::{fixture_project, mouse, move_pointer},
    };
    use bevy::input::ButtonState;

    #[test]
    fn paint_sidebar_splits_merges_and_round_trips_unique_tabs() {
        let maps = Tab::Panel(Panel::Maps);
        let layers = Tab::Panel(Panel::Layers);
        let split = Layout::default()
            .move_tab(maps, Tab::Properties, Placement::Edge(Side::Top))
            .unwrap();
        assert!(split.valid());
        let reopened: Layout =
            serde_json::from_slice(&serde_json::to_vec(&split).unwrap()).unwrap();
        assert_eq!(split, reopened);
        let merged = reopened
            .move_tab(
                maps,
                layers,
                Placement::Tabs {
                    before: Some(layers),
                },
            )
            .unwrap();
        assert!(merged.valid());
        let mut groups = Vec::new();
        merged.walk(&mut |tabs, active| groups.push((tabs.to_vec(), active)));
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0[..2], [maps, layers]);
        assert_eq!(groups[0].1, maps);
        let docked = merged
            .move_tab(
                Tab::Shelf,
                Tab::Properties,
                Placement::Tabs { before: None },
            )
            .unwrap();
        assert!(docked.valid());
        assert!(docked.contains(Tab::Shelf));
        let restored = docked.without(Tab::Shelf).unwrap();
        assert_eq!(restored, merged);
    }

    #[test]
    #[ignore = "requires GPU; moves the library and drops a filtered material onto the model"]
    fn gpu_paint_library_moves_between_viewport_and_sidebar() {
        let project = crate::EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        let (mut app, window, _target) = fixture_project(project);
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
        for _ in 0..15 {
            frame(&mut app);
        }
        let canvas = {
            let world = app.world_mut();
            world
                .query_filtered::<Entity, With<Canvas>>()
                .single(world)
                .unwrap()
        };
        let search = {
            let world = app.world_mut();
            world
                .query_filtered::<Entity, With<ui::ShelfSearch>>()
                .single(world)
                .unwrap()
        };
        app.world_mut()
            .get_mut::<bevy::text::EditableText>(search)
            .unwrap()
            .editor_mut()
            .set_text("Gold");
        for _ in 0..6 {
            frame(&mut app);
        }
        let rect = |app: &App, entity| {
            let world = app.world();
            engine::node_rect(
                world.get::<ComputedNode>(entity).unwrap(),
                world.get::<UiGlobalTransform>(entity).unwrap(),
            )
            .unwrap()
        };
        let drag = |app: &mut App, from, to| {
            move_pointer(app, window, from);
            mouse(app, window, ButtonState::Pressed);
            move_pointer(app, window, from + Vec2::X * 8.);
            move_pointer(app, window, to);
            mouse(app, window, ButtonState::Released);
            for _ in 0..6 {
                frame(app);
            }
        };
        let (handle, shelf, home, overlay, properties) = {
            let state = app.world().resource::<State>();
            let shelf = state.shelf.as_ref().unwrap();
            (
                shelf.handle,
                shelf.body,
                shelf.home,
                shelf.overlay,
                state.dock.groups[1].frame,
            )
        };
        let old_height = rect(&app, canvas).height();
        let from = rect(&app, handle).center();
        let to = rect(&app, properties).center();
        drag(&mut app, from, to);
        let state = app.world().resource::<State>();
        assert!(state.shelf_in_sidebar());
        assert_eq!(
            app.world().get::<Node>(home).unwrap().display,
            Display::None
        );
        assert!(rect(&app, canvas).height() > old_height + 170.);
        assert!(rect(&app, shelf).width() < 400.);
        assert_eq!(
            app.world()
                .get::<bevy::text::EditableText>(search)
                .unwrap()
                .value(),
            "Gold"
        );

        let card = {
            let world = app.world_mut();
            world
                .query::<(Entity, &Control)>()
                .iter(world)
                .find(|(_, control)| {
                    matches!(
                        control.action,
                        Action::ModelView(ModelViewAction::Paint(PaintAction::Preset(0)))
                    )
                })
                .unwrap()
                .0
        };
        let from = rect(&app, card).center();
        let to = rect(&app, canvas).center();
        assert!(rect(&app, shelf).contains(from));
        drag(&mut app, from, to);
        let session = &app.world().resource::<PaintState>().sessions[&0];
        assert_eq!(session.stack.layers[session.stack.selected].name, "Gold");

        let tab = app
            .world()
            .resource::<State>()
            .dock
            .groups
            .iter()
            .flat_map(|group| &group.tabs)
            .find(|(_, tab)| *tab == Tab::Shelf)
            .unwrap()
            .0;
        let from = rect(&app, tab).center();
        let area = rect(&app, overlay);
        drag(&mut app, from, Vec2::new(area.center().x, area.max.y - 20.));
        let state = app.world().resource::<State>();
        assert!(!state.shelf_in_sidebar());
        assert_eq!(app.world().get::<ChildOf>(shelf).unwrap().parent(), home);
        assert!((rect(&app, canvas).height() - old_height).abs() < 1.);
        assert_eq!(
            app.world()
                .get::<bevy::text::EditableText>(search)
                .unwrap()
                .value(),
            "Gold"
        );
    }

    #[test]
    #[ignore = "requires GPU; drives sidebar tab dragging and split resizing"]
    fn gpu_paint_sidebar_splits_tabs_and_keeps_both_panel_bodies() {
        let project = crate::EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        let (mut app, window, _target) = fixture_project(project);
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
        let maps = Tab::Panel(Panel::Maps);
        let layers = Tab::Panel(Panel::Layers);
        let tab = |app: &App, tab| {
            app.world()
                .resource::<State>()
                .dock
                .groups
                .iter()
                .flat_map(|g| &g.tabs)
                .find(|(_, candidate)| *candidate == tab)
                .unwrap()
                .0
        };
        let rect = |app: &App, entity| {
            let world = app.world();
            engine::node_rect(
                world.get::<ComputedNode>(entity).unwrap(),
                world.get::<UiGlobalTransform>(entity).unwrap(),
            )
            .unwrap()
        };
        let drag = |app: &mut App, from, to| {
            move_pointer(app, window, from);
            mouse(app, window, ButtonState::Pressed);
            move_pointer(app, window, to);
            mouse(app, window, ButtonState::Released);
            for _ in 0..5 {
                frame(app);
            }
        };
        let properties = rect(&app, app.world().resource::<State>().dock.groups[1].frame);
        let from = rect(&app, tab(&app, maps)).center();
        drag(
            &mut app,
            from,
            Vec2::new(properties.center().x, properties.max.y - 8.),
        );
        let state = app.world().resource::<State>();
        assert_eq!(state.dock.groups.len(), 3);
        let (layers_body, maps_body) = (state.bodies[0].unwrap(), state.bodies[1].unwrap());
        assert!(rect(&app, layers_body).max.y < rect(&app, maps_body).min.y);
        for body in [layers_body, maps_body] {
            assert!(!app.world().get::<Children>(body).unwrap().is_empty());
        }
        assert_eq!(
            app.world().resource::<PaintState>().sessions[&0].panel,
            Panel::Layers
        );
        let map_options = {
            let world = app.world_mut();
            let mut controls = world.query::<(Entity, &Control)>();
            controls
                .iter(world)
                .find(|(_, control)| {
                    matches!(
                        control.action,
                        Action::ModelView(ModelViewAction::Paint(PaintAction::BakeMapOptions))
                    )
                })
                .unwrap()
                .0
        };
        let options = rect(&app, map_options).center();
        assert!(rect(&app, maps_body).contains(options));
        move_pointer(&mut app, window, options);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
        assert_eq!(
            app.world().resource::<PaintState>().sessions[&0].panel,
            Panel::Maps
        );
        assert_eq!(
            app.world().get::<Node>(layers_body).unwrap().display,
            Display::Flex
        );
        let from = rect(&app, tab(&app, maps)).center();
        let to = rect(&app, tab(&app, layers)).center();
        drag(&mut app, from, to);
        let state = app.world().resource::<State>();
        assert_eq!(state.dock.groups.len(), 2);
        assert_eq!(
            app.world().get::<Node>(layers_body).unwrap().display,
            Display::None
        );
        assert_eq!(
            app.world().get::<Node>(maps_body).unwrap().display,
            Display::Flex
        );

        let folder = std::env::temp_dir().join(format!("bevy-sidebar-{}", uuid::Uuid::new_v4()));
        let file = folder.join("layout.json");
        app.world_mut().resource_mut::<State>().path = Some(file.clone());
        let (grip, first, tree) = {
            let state = app.world().resource::<State>();
            let split = state
                .dock
                .dividers
                .iter()
                .find(|split| split.path.is_empty())
                .unwrap();
            (split.handle, split.members[0], state.tree)
        };
        let from = rect(&app, grip).center();
        let height = rect(&app, first).height();
        move_pointer(&mut app, window, from);
        mouse(&mut app, window, ButtonState::Pressed);
        move_pointer(&mut app, window, from + Vec2::Y * 50.);
        assert!(rect(&app, first).height() > height + 45.);
        assert!(!file.exists(), "a live drag does not commit preferences");
        mouse(&mut app, window, ButtonState::Released);
        assert_eq!(app.world().resource::<State>().tree, tree);
        let saved: Layout = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert!(saved.split_at(&[]).unwrap().1 > 0.43);

        let properties = rect(&app, app.world().resource::<State>().dock.groups[1].frame);
        let from = rect(&app, tab(&app, layers)).center();
        drag(
            &mut app,
            from,
            Vec2::new(properties.max.x - 8., properties.center().y),
        );
        let (grip, right) = {
            let state = app.world().resource::<State>();
            let split = state
                .dock
                .dividers
                .iter()
                .find(|split| split.side.horizontal())
                .unwrap();
            (split.handle, split.members[1])
        };
        let from = rect(&app, grip).center();
        let width = rect(&app, right).width();
        drag(&mut app, from, from - Vec2::X * 30.);
        assert!(rect(&app, right).width() > width + 25.);
        let saved: Layout = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert!(saved.split_at(&[1]).unwrap().1 < 0.5);
        app.world_mut()
            .write_message(Action::ModelView(ModelViewAction::Paint(
                PaintAction::Reset,
            )));
        for _ in 0..5 {
            frame(&mut app);
        }
        let saved: Layout = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(saved, Layout::default());
        std::fs::remove_dir_all(folder).unwrap();
    }
}

#[derive(Clone, Copy)]
enum Placement {
    Tabs { before: Option<Tab> },
    Edge(Side),
}

struct Group {
    frame: Entity,
    header: Entity,
    tabs: Vec<(Entity, Tab)>,
}

struct Divider {
    handle: Entity,
    frame: Entity,
    members: [Entity; 2],
    path: Vec<usize>,
    side: Side,
}

impl Divider {
    fn fraction(&self, layout: &Layout, frame: Rect, cursor: Vec2) -> Option<f32> {
        let (children, _) = layout.split_at(&self.path)?;
        let axis = if self.side.horizontal() { 0 } else { 1 };
        let length = (frame.size()[axis] - DIVIDER).max(1.);
        let lower = (children[0].minimum_size()[axis] / length).max(0.2);
        let upper = (1. - children[1].minimum_size()[axis] / length).min(0.8);
        (lower <= upper).then(|| self.side.size_from_edge(frame, cursor).clamp(lower, upper))
    }
}

#[derive(Clone, Copy)]
enum Drag {
    Tab { tab: Tab, start: Vec2 },
    Resize { divider: usize, grab: Vec2 },
}

#[derive(Default)]
struct DockNodes {
    groups: Vec<Group>,
    dividers: Vec<Divider>,
}

#[derive(Resource)]
pub(in crate::editor::ui::model_viewer) struct State {
    path: Option<PathBuf>,
    layout: Layout,
    rendered: Option<Layout>,
    focus: Option<(usize, Panel)>,
    root: Entity,
    tree: Option<Entity>,
    bodies: Vec<Option<Entity>>,
    shelf: Option<ShelfDock>,
    preview: Entity,
    dock: DockNodes,
    drag: Option<Drag>,
}

#[derive(Resource)]
pub(in crate::editor::ui::model_viewer) struct ShaderDock(State);

#[derive(Resource)]
pub(in crate::editor::ui::model_viewer) struct GraphDock(State);

#[derive(Clone, Copy)]
enum Workspace {
    Paint,
    Shader,
    Graph,
}

impl Workspace {
    fn file(self) -> &'static str {
        match self {
            Self::Paint => "paint-sidebar.json",
            Self::Shader => "paint-shader-layout.json",
            Self::Graph => "paint-graph-layout.json",
        }
    }

    fn layout(self) -> Layout {
        match self {
            Self::Paint => Layout::default(),
            Self::Shader => Layout::shader(),
            Self::Graph => Layout::graph(),
        }
    }
}

impl ShaderDock {
    pub(super) fn reset(&mut self) {
        self.0.reset(Workspace::Shader);
    }
}

impl GraphDock {
    pub(super) fn reset(&mut self) {
        self.0.reset(Workspace::Graph);
    }
}

pub(super) struct ShelfDock {
    pub home: Entity,
    pub handle: Entity,
    pub body: Entity,
    pub overlay: Entity,
    pub preview: Entity,
}

impl State {
    fn reset(&mut self, workspace: Workspace) {
        self.layout = workspace.layout();
        self.drag = None;
        if let Err(error) = self.save() {
            warn!("Could not save paint workspace layout: {error}");
        }
    }

    pub(super) fn visible_panels(&self) -> [bool; 5] {
        let mut visible = [false; 5];
        self.layout.walk(&mut |_, active| {
            if let Tab::Panel(panel) = active {
                visible[panel_index(panel)] = true;
            }
        });
        visible
    }

    pub(super) fn shelf_in_sidebar(&self) -> bool {
        self.layout.contains(Tab::Shelf)
    }

    pub(super) fn shelf_tab_visible(&self) -> bool {
        let mut visible = false;
        self.layout
            .walk(&mut |_, active| visible |= active == Tab::Shelf);
        visible
    }

    fn save(&self) -> io::Result<()> {
        if let Some(path) = &self.path {
            atomic_write(
                path,
                &serde_json::to_vec(&self.layout)?,
                Revision::read(path)?,
            )?;
        }
        Ok(())
    }
}

pub(super) fn setup(
    commands: &mut Commands,
    root: Entity,
    stacks: [Entity; 5],
    properties: Entity,
    shelf: ShelfDock,
) {
    let bodies = stacks
        .into_iter()
        .chain([properties, shelf.body])
        .map(Some)
        .collect();
    let state = make_state(commands, root, bodies, Some(shelf), Workspace::Paint);
    commands.insert_resource(state);
    for (body, panel) in stacks.into_iter().zip(PANELS) {
        commands.entity(body).insert(PanelBody(panel));
    }
}

pub(super) fn setup_shader(
    commands: &mut Commands,
    root: Entity,
    library: Entity,
    code: Entity,
    preview: Entity,
) {
    let mut bodies = vec![None; 7];
    bodies.extend([Some(library), Some(code), Some(preview)]);
    let state = make_state(commands, root, bodies, None, Workspace::Shader);
    commands.insert_resource(ShaderDock(state));
}

pub(super) fn setup_graph(
    commands: &mut Commands,
    root: Entity,
    preview: Entity,
    graph: Entity,
    settings: Entity,
) {
    let mut bodies = vec![None; Tab::GraphPreview.index()];
    bodies.extend([Some(preview), Some(graph), Some(settings)]);
    let state = make_state(commands, root, bodies, None, Workspace::Graph);
    commands.insert_resource(GraphDock(state));
}

fn make_state(
    commands: &mut Commands,
    root: Entity,
    bodies: Vec<Option<Entity>>,
    shelf: Option<ShelfDock>,
    workspace: Workspace,
) -> State {
    let path = presets::directory()
        .and_then(|folder| folder.parent().map(|root| root.join(workspace.file())));
    let layout = path
        .as_deref()
        .and_then(|path| {
            let layout = serde_json::from_slice::<Layout>(&read_limited(path, 8192).ok()?).ok()?;
            layout.valid_for(workspace).then_some(layout)
        })
        .unwrap_or_else(|| workspace.layout());
    let preview_zone = commands
        .spawn((
            Node {
                display: Display::None,
                ..default()
            },
            BackgroundColor(FOCUS.with_alpha(0.22)),
            BorderColor::all(FOCUS),
            Pickable::IGNORE,
            ZIndex(90),
            ChildOf(root),
        ))
        .id();
    State {
        path,
        layout,
        root,
        bodies,
        shelf,
        rendered: None,
        focus: None,
        tree: None,
        preview: preview_zone,
        dock: DockNodes::default(),
        drag: None,
    }
}

fn paint_tab(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    tab: Tab,
    active: bool,
) -> Entity {
    let button = match tab {
        Tab::Panel(panel) => ui::button(
            commands,
            parent,
            font,
            tab.label(),
            PaintAction::Panel(panel),
        ),
        _ => {
            let entity = commands
                .spawn((
                    Button,
                    Node {
                        height: px(29),
                        padding: UiRect::horizontal(px(5)),
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(6)),
                        ..row(3.)
                    },
                    BackgroundColor(if active { ACTIVE } else { Color::NONE }),
                    BorderColor::all(if active { FOCUS } else { EDGE }),
                    AccessibleLabel(tab.label().into()),
                    ChildOf(parent),
                ))
                .id();
            text(commands, entity, font, tab.label(), 13., TEXT);
            if tab == Tab::Shelf {
                control_hints::attach(
                    commands,
                    entity,
                    "Перетащите к нижнему краю 3D- или UV-области, чтобы вернуть библиотеку вниз.",
                );
            }
            entity
        }
    };
    commands
        .entity(button)
        .entry::<Node>()
        .and_modify(|mut node| node.padding = UiRect::horizontal(px(5)));
    button
}

fn spawn_layout(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    layout: &Layout,
    weight: f32,
    bodies: &[Option<Entity>],
    dock: &mut DockNodes,
    path: &[usize],
    nodes: &mut Query<&mut Node>,
) -> Entity {
    let shape = Node {
        align_self: AlignSelf::Stretch,
        min_width: px(0),
        min_height: px(0),
        flex_grow: weight,
        flex_basis: px(0),
        overflow: Overflow::clip(),
        ..column(0.)
    };
    match layout {
        Layout::Split {
            horizontal,
            fraction,
            children,
        } => {
            let frame = node(
                commands,
                parent,
                Node {
                    flex_direction: if *horizontal {
                        FlexDirection::Row
                    } else {
                        FlexDirection::Column
                    },
                    ..shape
                },
            );
            let mut child_path = path.to_vec();
            child_path.push(0);
            let first = spawn_layout(
                commands,
                frame,
                font,
                &children[0],
                *fraction,
                bodies,
                dock,
                &child_path,
                nodes,
            );
            let (side, width, height, label) = if *horizontal {
                (
                    Side::Left,
                    px(DIVIDER),
                    percent(100),
                    "Изменить ширину групп",
                )
            } else {
                (
                    Side::Top,
                    percent(100),
                    px(DIVIDER),
                    "Изменить высоту групп",
                )
            };
            let handle = surface(
                commands,
                frame,
                Node {
                    width,
                    height,
                    flex_shrink: 0.,
                    ..default()
                },
                CHROME,
            );
            commands
                .entity(handle)
                .insert(AccessibleLabel(label.into()));
            *child_path.last_mut().unwrap() = 1;
            let second = spawn_layout(
                commands,
                frame,
                font,
                &children[1],
                1. - *fraction,
                bodies,
                dock,
                &child_path,
                nodes,
            );
            dock.dividers.push(Divider {
                handle,
                frame,
                members: [first, second],
                path: path.to_vec(),
                side,
            });
            frame
        }
        Layout::Group { tabs, active } => {
            let frame = node(commands, parent, shape);
            let header = surface(
                commands,
                frame,
                Node {
                    width: percent(100),
                    min_height: px(34),
                    padding: UiRect::all(px(3)),
                    flex_shrink: 0.,
                    flex_wrap: FlexWrap::Wrap,
                    ..row(1.)
                },
                CHROME,
            );
            let buttons = tabs
                .iter()
                .map(|&tab| (paint_tab(commands, header, font, tab, tab == *active), tab))
                .collect();
            dock.groups.push(Group {
                frame,
                header,
                tabs: buttons,
            });
            for &tab in tabs {
                let Some(body) = bodies.get(tab.index()).copied().flatten() else {
                    continue;
                };
                commands.entity(body).insert(ChildOf(frame));
                if let Ok(mut node) = nodes.get_mut(body) {
                    node.display = if tab == *active {
                        Display::Flex
                    } else {
                        Display::None
                    };
                }
            }
            frame
        }
    }
}

fn show_preview(node: &mut Mut<Node>, zone: Option<Rect>, root: Option<Rect>) {
    if let Some((zone, root)) = zone.zip(root) {
        let start = zone.min - root.min;
        node.set_if_neq(Node {
            position_type: PositionType::Absolute,
            left: px(start.x),
            top: px(start.y),
            width: px(zone.width()),
            height: px(zone.height()),
            border: UiRect::all(px(1)),
            ..default()
        });
    } else if node.display != Display::None {
        node.display = Display::None;
    }
}

pub(in crate::editor::ui::model_viewer) fn update(
    mut commands: Commands,
    mut state: ResMut<State>,
    mut shader_dock: ResMut<ShaderDock>,
    mut graph_dock: ResMut<GraphDock>,
    editor: Res<Editor>,
    editing: Res<editing::State>,
    paint: Res<PaintState>,
    interface: Res<Interface>,
    drawer: Res<DrawerMotion>,
    mut actions: MessageReader<Action>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    mut nodes: Query<&mut Node>,
    mut colors: Query<&mut BackgroundColor>,
) {
    let mut reset = false;
    let mut explicit_focus = None;
    for action in actions.read() {
        match action {
            Action::ModelView(ModelViewAction::Paint(PaintAction::Panel(panel))) => {
                explicit_focus = Some(*panel)
            }
            Action::ModelView(ModelViewAction::Paint(PaintAction::Reset)) => reset = true,
            _ => {}
        }
    }
    let active = editor.viewing_model.filter(|model| {
        editing
            .documents
            .get(model)
            .is_some_and(|doc| doc.mode == Mode::Paint)
            && editor.pending.is_none()
            && editor.viewing_image.is_none()
    });
    let mut persist = false;
    if active.is_some() && reset {
        persist = state.layout != Layout::default();
        state.layout = Layout::default();
        state.drag = None;
    }
    let focus = active.and_then(|model| Some((model, paint.sessions.get(&model)?.panel)));
    if let Some((_, panel)) = focus
        && (focus != state.focus || explicit_focus == Some(panel))
    {
        state.layout.activate(Tab::Panel(panel));
    }
    if focus != state.focus {
        state.drag = None;
        state.focus = focus;
    }

    let cursor =
        (active.is_some() && window.focused && !editor.browser_popup && !editor.hdri_library_open)
            .then(|| window.cursor_position())
            .flatten()
            .filter(|&cursor| !drawer.covers(cursor, &window));
    update_dock(
        &mut commands,
        &mut state,
        &interface.font,
        cursor,
        &mouse,
        &geometry,
        &mut nodes,
        &mut colors,
        persist,
        reset,
    );
    let shader_cursor = cursor.filter(|_| focus.is_some_and(|(_, panel)| panel == Panel::Shader));
    update_dock(
        &mut commands,
        &mut shader_dock.0,
        &interface.font,
        shader_cursor,
        &mouse,
        &geometry,
        &mut nodes,
        &mut colors,
        false,
        false,
    );
    let graph_cursor = cursor.filter(|_| {
        active
            .and_then(|model| paint.sessions.get(&model))
            .is_some_and(Session::graph_canvas_open)
    });
    update_dock(
        &mut commands,
        &mut graph_dock.0,
        &interface.font,
        graph_cursor,
        &mouse,
        &geometry,
        &mut nodes,
        &mut colors,
        false,
        false,
    );
}

fn update_dock(
    commands: &mut Commands,
    state: &mut State,
    font: &Handle<Font>,
    cursor: Option<Vec2>,
    mouse: &ButtonInput<MouseButton>,
    geometry: &Query<(&ComputedNode, &UiGlobalTransform)>,
    nodes: &mut Query<&mut Node>,
    colors: &mut Query<&mut BackgroundColor>,
    mut persist: bool,
    reset: bool,
) {
    let rect = |entity| {
        let (node, transform) = geometry.get(entity).ok()?;
        engine::node_rect(node, transform)
    };
    let mut zone = None;
    let mut home_zone = None;
    let mut hot_divider = None;
    let mut resize = None;
    if let Some(cursor) = cursor {
        let grip_at_cursor = state
            .dock
            .dividers
            .iter()
            .enumerate()
            .find_map(|(i, divider)| {
                rect(divider.handle)
                    .filter(|bounds| bounds.contains(cursor))
                    .map(|bounds| (i, bounds))
            });
        hot_divider = grip_at_cursor.map(|(i, _)| i);
        if mouse.just_pressed(MouseButton::Left) && !reset {
            state.drag = if let Some((divider, bounds)) = grip_at_cursor {
                Some(Drag::Resize {
                    divider,
                    grab: cursor - bounds.center(),
                })
            } else if !state.shelf_in_sidebar()
                && state
                    .shelf
                    .as_ref()
                    .and_then(|shelf| rect(shelf.handle))
                    .is_some_and(|r| r.contains(cursor))
            {
                Some(Drag::Tab {
                    tab: Tab::Shelf,
                    start: cursor,
                })
            } else {
                state
                    .dock
                    .groups
                    .iter()
                    .flat_map(|group| &group.tabs)
                    .find(|(entity, _)| rect(*entity).is_some_and(|r| r.contains(cursor)))
                    .map(|&(_, tab)| Drag::Tab { tab, start: cursor })
            };
        }
        if let Some(Drag::Resize { divider, grab }) = state.drag {
            hot_divider = Some(divider);
            if (mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left))
                && state.rendered.as_ref() == Some(&state.layout)
                && let Some(split) = state.dock.dividers.get(divider)
                && let Some(fraction) = rect(split.frame)
                    .and_then(|frame| split.fraction(&state.layout, frame, cursor - grab))
            {
                if mouse.just_released(MouseButton::Left) {
                    let path = split.path.clone();
                    if state
                        .layout
                        .split_at(&path)
                        .is_some_and(|(_, previous)| previous != fraction)
                    {
                        state.layout.set_fraction(&path, fraction);
                        if let Some(rendered) = &mut state.rendered {
                            rendered.set_fraction(&path, fraction);
                        }
                        persist = true;
                    }
                } else {
                    resize = Some((divider, fraction));
                }
            }
        } else if let Some(Drag::Tab { tab, start }) = state.drag {
            if crate::editor::area_selection::dragged(start, cursor) {
                let home_target = (tab == Tab::Shelf && state.shelf_in_sidebar())
                    .then(|| {
                        let mut zone = rect(state.shelf.as_ref()?.overlay)?;
                        zone.min.x += 44.;
                        zone.min.y = (zone.max.y - 64.).max(zone.min.y);
                        zone.contains(cursor).then_some(zone)
                    })
                    .flatten();
                if let Some(target) = home_target {
                    if mouse.just_released(MouseButton::Left) {
                        state.layout = state.layout.clone().without(Tab::Shelf).unwrap_or_default();
                        persist = true;
                    } else {
                        home_zone = Some(target);
                    }
                } else {
                    for group in &state.dock.groups {
                        let Some(frame) = rect(group.frame).filter(|r| r.contains(cursor)) else {
                            continue;
                        };
                        let Some(&(_, anchor)) =
                            group.tabs.iter().find(|(_, target)| *target != tab)
                        else {
                            break;
                        };
                        let header = rect(group.header);
                        let (placement, area) = if header.is_some_and(|r| r.contains(cursor)) {
                            let before = group
                                .tabs
                                .iter()
                                .find(|(entity, candidate)| {
                                    *candidate != tab
                                        && rect(*entity).is_some_and(|r| r.contains(cursor))
                                })
                                .map(|&(_, t)| t);
                            (Placement::Tabs { before }, header.unwrap())
                        } else {
                            let inner = Rect::from_center_size(frame.center(), frame.size() * 0.48);
                            if inner.contains(cursor) {
                                (Placement::Tabs { before: None }, frame)
                            } else {
                                let side = Side::nearest(frame, cursor).unwrap();
                                (Placement::Edge(side), side.region(frame, 0.5))
                            }
                        };
                        if let Some(next) = state.layout.move_tab(tab, anchor, placement) {
                            if mouse.just_released(MouseButton::Left) {
                                state.layout = next;
                                persist = true;
                            } else {
                                zone = Some(area);
                            }
                        }
                        break;
                    }
                }
            } else if !matches!(tab, Tab::Panel(_)) && mouse.just_released(MouseButton::Left) {
                state.layout.activate(tab);
            }
        }
        if !mouse.pressed(MouseButton::Left) {
            state.drag = None;
        }
    } else {
        state.drag = None;
    }
    if persist && let Err(error) = state.save() {
        warn!("Could not save paint sidebar: {error}");
    }
    if state.rendered.as_ref() != Some(&state.layout) {
        state.drag = None;
        let mut dock = DockNodes::default();
        let next = spawn_layout(
            commands,
            state.root,
            font,
            &state.layout,
            1.,
            &state.bodies,
            &mut dock,
            &[],
            nodes,
        );
        if !state.shelf_in_sidebar()
            && let Some(shelf) = &state.shelf
        {
            commands.entity(shelf.body).insert(ChildOf(shelf.home));
        }
        if let Some(previous) = state.tree.replace(next) {
            commands.entity(previous).despawn();
        }
        state.dock = dock;
        state.rendered = Some(state.layout.clone());
    } else {
        for (index, divider) in state.dock.dividers.iter().enumerate() {
            let Some((_, saved)) = state.layout.split_at(&divider.path) else {
                continue;
            };
            let fraction = resize
                .filter(|&(active, _)| active == index)
                .map_or(saved, |(_, fraction)| fraction);
            for (entity, weight) in divider.members.into_iter().zip([fraction, 1. - fraction]) {
                if let Ok(mut node) = nodes.get_mut(entity)
                    && node.flex_grow != weight
                {
                    node.flex_grow = weight;
                }
            }
            if let Ok(mut color) = colors.get_mut(divider.handle) {
                color.set_if_neq(BackgroundColor(if hot_divider == Some(index) {
                    FOCUS
                } else {
                    CHROME
                }));
            }
        }
    }
    if let Ok(mut node) = nodes.get_mut(state.preview) {
        show_preview(&mut node, zone, rect(state.root));
    }
    if let Some(shelf) = &state.shelf
        && let Ok(mut node) = nodes.get_mut(shelf.preview)
    {
        show_preview(&mut node, home_zone, rect(shelf.overlay));
    }
}
