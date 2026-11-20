//! Cascading asset-creation menus and exclusive creation of editable GLB assets.
use super::*;
use crate::editor::{
    model_document::{MaterialSlot, Object, glb::Glb},
    model_primitives::{Parameters, Primitive},
};
use std::{fs::OpenOptions, io::Write, path::Path};

const WIDTH: f32 = 216.0;
const HEIGHT: f32 = 8.0 * 32.0 + 7.0 * 3.0 + 14.0;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Category {
    #[default]
    Primitives,
    Dialogues,
}

impl Category {
    fn menu(self, ui: &ContextUi) -> &MenuUi {
        match self {
            Self::Primitives => &ui.primitives,
            Self::Dialogues => &ui.dialogues,
        }
    }

    fn size(self) -> Vec2 {
        match self {
            Self::Primitives => Vec2::new(WIDTH, HEIGHT),
            Self::Dialogues => Vec2::new(256.0, 81.0),
        }
    }
}

pub(super) struct MenuUi {
    pub root: Entity,
    pub trigger: Entity,
    first: Entity,
}

#[derive(Resource, Default)]
struct MenuState {
    open: bool,
    category: Category,
    pointer: Option<Vec2>,
    keyboard: bool,
    grace: f32,
    suppress_hover: bool,
}

pub(in crate::editor) struct BrowserPrimitivePlugin;
impl Plugin for BrowserPrimitivePlugin {
    fn build(&self, app: &mut App) {
        super::primitive_preview::register(app);
        app.init_resource::<MenuState>().add_systems(
            Update,
            (
                input
                    .after(super::input)
                    .before(crate::editor::ui::shortcuts),
                refresh
                    .after(super::refresh)
                    .after(crate::editor::ui::refresh),
            ),
        );
    }
}

fn item(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    action: Action,
    glyph: icons::Glyph,
) -> Entity {
    let item = button(commands, parent, font, label, action, Look::Regular);
    commands
        .entity(item)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.width = percent(100);
            node.height = px(32);
            node.border = UiRect::all(px(0));
        });
    let icon = icons::themed(commands, item, glyph, 16.0);
    commands.entity(item).insert_children(0, &[icon]);
    item
}

pub(super) fn setup(commands: &mut Commands, menu: Entity, font: &Handle<Font>) -> MenuUi {
    let trigger = item(
        commands,
        menu,
        font,
        "Создать 3D-фигуру",
        Action::BrowserPrimitiveMenu,
        icons::Glyph::Box,
    );
    commands
        .entity(trigger)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.height = px(34);
        });
    node(
        commands,
        trigger,
        Node {
            flex_grow: 1.0,
            ..default()
        },
    );
    icons::themed(commands, trigger, icons::Glyph::ChevronRight, 12.0);
    let root = surface(
        commands,
        menu,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: px(WIDTH),
            padding: UiRect::all(px(6)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            overflow: Overflow::scroll_y(),
            ..column(3.0)
        },
        PANEL,
    );
    commands.entity(root).insert((
        ZIndex(1),
        Scroller,
        AccessibleLabel("Создать 3D-фигуру · Выбор фигуры".into()),
    ));
    let mut first = None;
    for primitive in Primitive::ALL {
        let glyph = match primitive {
            Primitive::Plane => icons::Glyph::PrimitivePlane,
            Primitive::Cube => icons::Glyph::Box,
            Primitive::Circle => icons::Glyph::PrimitiveCircle,
            Primitive::UvSphere => icons::Glyph::PrimitiveUvSphere,
            Primitive::IcoSphere => icons::Glyph::PrimitiveIcoSphere,
            Primitive::Cylinder => icons::Glyph::PrimitiveCylinder,
            Primitive::Cone => icons::Glyph::PrimitiveCone,
            Primitive::Torus => icons::Glyph::PrimitiveTorus,
        };
        let entity = item(
            commands,
            root,
            font,
            primitive.label(),
            Action::BrowserCreatePrimitive(primitive),
            glyph,
        );
        first.get_or_insert(entity);
    }
    MenuUi {
        root,
        trigger,
        first: first.unwrap(),
    }
}

pub(super) fn setup_dialogues(
    commands: &mut Commands,
    menu: Entity,
    font: &Handle<Font>,
) -> MenuUi {
    let trigger = item(
        commands,
        menu,
        font,
        "Диалоги",
        Action::BrowserDialogueMenu,
        icons::Glyph::Layers,
    );
    commands
        .entity(trigger)
        .entry::<Node>()
        .and_modify(|mut n| n.height = px(34));
    node(
        commands,
        trigger,
        Node {
            flex_grow: 1.0,
            ..default()
        },
    );
    icons::themed(commands, trigger, icons::Glyph::ChevronRight, 12.0);
    let root = surface(
        commands,
        menu,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: px(Category::Dialogues.size().x),
            padding: UiRect::all(px(6)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            overflow: Overflow::scroll_y(),
            ..column(3.0)
        },
        PANEL,
    );
    commands.entity(root).insert((
        ZIndex(1),
        Scroller,
        AccessibleLabel("Диалоги · Выбор документа".into()),
    ));
    let first = item(
        commands,
        root,
        font,
        "Диалог · .dialog",
        Action::Narrative(NarrativeAction::Create(false)),
        icons::Glyph::File,
    );
    item(
        commands,
        root,
        font,
        "Персонаж · .dcharacter",
        Action::Narrative(NarrativeAction::Create(true)),
        icons::Glyph::Image,
    );
    MenuUi {
        root,
        trigger,
        first,
    }
}

fn input(
    mut state: ResMut<MenuState>,
    context: Res<BrowserContext>,
    ui: Res<ContextUi>,
    window: Single<&Window, With<PrimaryWindow>>,
    geometry: Geometry,
    mut actions: MessageReader<Action>,
    mut focus: ResMut<InputFocus>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    time: Res<Time>,
    parents: Query<&ChildOf>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let clicked = actions
        .read()
        .filter_map(|action| match action {
            Action::BrowserPrimitiveMenu => Some(Category::Primitives),
            Action::BrowserDialogueMenu => Some(Category::Dialogues),
            _ => None,
        })
        .last();
    if context.phase != Phase::Menu || context.file_menu() {
        state.open = false;
        state.grace = 0.0;
        state.suppress_hover = false;
        state.keyboard = false;
        return;
    }
    if mouse.just_pressed(MouseButton::Right) {
        state.open = false;
    }
    let pointer = window.cursor_position().map(|p| p * window.scale_factor());
    let moved = pointer != state.pointer;
    state.pointer = pointer;
    if moved || clicked.is_some() {
        state.keyboard = false;
    }
    let hovered = [Category::Primitives, Category::Dialogues]
        .into_iter()
        .find(|category| pointer.is_some_and(|p| hit(&geometry, category.menu(&ui).trigger, p)));
    let navigated = [Category::Primitives, Category::Dialogues]
        .into_iter()
        .find(|category| {
            focus.get() == Some(category.menu(&ui).trigger)
                && keys.just_pressed(KeyCode::ArrowRight)
        });
    if navigated.is_some() {
        state.keyboard = true;
    }
    let hovered = hovered.filter(|category| !state.keyboard || state.category == *category);
    if let Some(category) = clicked.or(navigated).or(hovered) {
        if state.category != category {
            state.suppress_hover = false;
        }
        state.category = category;
    }
    let ui = state.category.menu(&ui);
    let in_submenu = focus.get().is_some_and(|entity| {
        parents
            .iter_ancestors(entity)
            .any(|parent| parent == ui.root)
    });
    let right = focus.get() == Some(ui.trigger) && keys.just_pressed(KeyCode::ArrowRight);
    if right || clicked.is_some() {
        state.suppress_hover = false;
        state.open = true;
        state.grace = 0.2;
        if right {
            keys.clear_just_pressed(KeyCode::ArrowRight);
            focus.set(ui.first, FocusCause::Navigated);
        }
    }
    if state.open
        && (keys.just_pressed(KeyCode::Escape)
            || (in_submenu && keys.just_pressed(KeyCode::ArrowLeft)))
    {
        keys.clear_just_pressed(KeyCode::Escape);
        keys.clear_just_pressed(KeyCode::ArrowLeft);
        state.open = false;
        state.suppress_hover = true;
        focus.set(ui.trigger, FocusCause::Navigated);
        return;
    }
    let on_trigger = pointer.is_some_and(|p| hit(&geometry, ui.trigger, p));
    let on_submenu = state.open && pointer.is_some_and(|p| hit(&geometry, ui.root, p));
    if !on_trigger {
        state.suppress_hover = false;
    }
    if (on_trigger && !state.suppress_hover)
        || on_submenu
        || in_submenu
        || clicked.is_some()
        || right
    {
        state.open = true;
        state.grace = 0.2;
    } else if state.open {
        state.grace -= time.delta_secs();
        if state.grace <= 0.0 {
            state.open = false;
        }
        redraw.write(RequestRedraw);
    }
}

fn refresh(
    mut state: ResMut<MenuState>,
    context: Res<BrowserContext>,
    ui: Res<ContextUi>,
    geometry: Geometry,
    window: Single<&Window, With<PrimaryWindow>>,
    mut nodes: Query<&mut Node>,
    mut colors: Query<&mut BackgroundColor>,
) {
    if context.phase != Phase::Menu || context.file_menu() {
        state.open = false;
    }
    for category in [Category::Primitives, Category::Dialogues] {
        let shown = state.open && state.category == category;
        let submenu = category.menu(&ui);
        let size = category.size();
        if let Ok(mut node) = nodes.get_mut(submenu.root) {
            node.display = if shown { Display::Flex } else { Display::None };
            node.max_height = px((window.height() - 16.0).max(80.0));
            if let Some(menu) = bounds(&geometry, ui.menu)
                && let Some(trigger) = bounds(&geometry, submenu.trigger)
            {
                let scale = window.scale_factor();
                let border = geometry
                    .get(ui.menu)
                    .map(|(_, node, _, _)| node.border())
                    .unwrap_or_default();
                let height = size.y.min((window.height() - 16.0).max(80.0));
                let right = menu.max.x / scale + 3.0;
                node.left = px((if right + size.x <= window.width() - 8.0 {
                    menu.width() / scale + 3.0
                } else {
                    -size.x - 3.0
                }) - border.min_inset.x / scale);
                node.top = px((trigger.min.y / scale)
                    .clamp(8.0, (window.height() - height - 8.0).max(8.0))
                    - (menu.min.y + border.min_inset.y) / scale);
            }
        }
        if shown && let Ok(mut color) = colors.get_mut(submenu.trigger) {
            color.set_if_neq(BackgroundColor(ACTIVE));
        }
    }
}

pub(super) fn stem(name: &str) -> Result<String, String> {
    let mut name = name.to_owned();
    if name.to_ascii_lowercase().ends_with(".glb") {
        name.truncate(name.len() - 4);
    }
    if name.trim().is_empty() {
        return Err("Введите название 3D-модели.".into());
    }
    browser_files::validate_name(&name).map_err(str::to_owned)?;
    Ok(name)
}

pub(super) fn suggested_name(root: &Path, parent: &Path, primitive: Primitive) -> String {
    for suffix in 1..10_000 {
        let name = if suffix == 1 {
            primitive.label().into()
        } else {
            format!("{} ({suffix})", primitive.label())
        };
        if !root.join(parent).join(format!("{name}.glb")).exists() {
            return name;
        }
    }
    primitive.label().into()
}

pub(super) fn create(
    root: &Path,
    parent: &Path,
    primitive: Primitive,
    name: &str,
    parameters: &Parameters,
) -> Result<PathBuf, String> {
    let stem = stem(name)?;
    let directory = browser_files::directory(root, parent).map_err(|e| e.to_string())?;
    let mut object = Object {
        id: 0,
        source_node: None,
        name: stem.clone(),
        geometry: primitive.geometry(parameters)?.into(),
        transform: Transform::IDENTITY,
        materials: vec![MaterialSlot {
            authored: None,
            paint: None,
            material: default(),
            source: serde_json::json!({"material": 0}),
        }],
        hidden: false,
    };
    for suffix in 1..10_000 {
        let name = if suffix == 1 {
            stem.clone()
        } else {
            format!("{stem} ({suffix})")
        };
        object.name = name.clone();
        let bytes = Glb::new_asset(std::slice::from_ref(&object))?;
        let name = format!("{name}.glb");
        let destination = directory.join(&name);
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        };
        if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = std::fs::remove_file(&destination);
            return Err(error.to_string());
        }
        return Ok(parent.join(name));
    }
    Err("Не удалось подобрать свободное имя 3D-модели".into())
}

#[cfg(test)]
#[path = "browser_primitives_tests.rs"]
mod tests;
