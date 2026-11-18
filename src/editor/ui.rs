use bevy::{
    input::{
        ButtonState,
        keyboard::KeyboardInput,
        mouse::{AccumulatedMouseScroll, MouseScrollUnit},
    },
    input_focus::{
        FocusedInput, InputFocus,
        tab_navigation::{TabGroup, TabIndex},
    },
    picking::{events::PointerClick, hover::Hovered, pointer::PointerButton},
    prelude::*,
    text::{EditableText, TextReadWriteMode},
    ui::{ComputedNode, InteractionDisabled, Pressed, UiGlobalTransform},
    ui_widgets::{Activate, Button, TextInput},
    window::PrimaryWindow,
};

use super::Room;
use crate::{project::LensKind, ui_theme::*};

use super::{
    Action, Editor, Field, Reset, Selection, Tool,
    browser::{self, Browser},
    starfield, viewport,
};

#[cfg(test)]
#[path = "asset_activation_tests.rs"]
mod asset_activation_tests;
#[path = "asset_loading.rs"]
pub(crate) mod asset_loading;
#[path = "browser_context.rs"]
mod browser_context;
#[path = "button_fx.rs"]
pub(crate) mod button_fx;
#[path = "camera_preview.rs"]
mod camera_preview;
#[path = "confirmation.rs"]
mod confirmation;
#[path = "control_hints.rs"]
mod control_hints;
#[path = "play_ui.rs"]
mod play_ui;
#[path = "scene_hierarchy_ui.rs"]
mod scene_hierarchy_ui;
#[path = "scene_tree_drag.rs"]
pub(super) mod scene_tree_drag;
pub(super) use play_ui::update as refresh_play;
#[path = "drawer.rs"]
mod drawer;
#[path = "icons.rs"]
pub(crate) mod icons;
#[path = "image_viewer.rs"]
mod image_viewer;
#[path = "model_viewer.rs"]
pub(in crate::editor) mod model_viewer;
pub(super) use model_viewer::editing::State as ModelEditingState;
pub(super) use model_viewer::{ModelViewAction, ModelViewerPlugin};
#[path = "navigation_gizmo.rs"]
pub(super) mod navigation_gizmo;
pub(super) use navigation_gizmo::NavigationGizmoPlugin;
#[path = "hdri_library.rs"]
mod hdri_library;
#[path = "lighting_ui.rs"]
mod lighting_ui;
pub(super) use hdri_library::{HdriAction, HdriLibrary, HdriLibraryPlugin};
#[path = "generation_ui.rs"]
pub(super) mod generation;
pub(super) use generation::{GenerationAction, GenerationPlugin};
#[path = "narrative_ui.rs"]
pub(super) mod narrative;
#[path = "numeric_input.rs"]
mod numeric_input;
#[path = "orbital.rs"]
pub(crate) mod orbital;
#[path = "scene_tabs.rs"]
mod scene_tabs;
pub(super) use narrative::{NarrativeAction, NarrativePlugin};
#[path = "scene_objects_ui.rs"]
mod scene_objects;
#[path = "sidebar_resize.rs"]
mod sidebar_resize;
#[path = "tool_icons.rs"]
mod tool_icons;
pub(super) use asset_loading::AssetLoadingPlugin;
pub(super) use browser_context::{
    BrowserContext, BrowserDeletionPlugin, BrowserPrimitivePlugin, input as browser_context_input,
    refresh as refresh_browser_context, update as browser_context_actions,
};
pub(super) use button_fx::ButtonFxPlugin;
pub(super) use camera_preview::PreviewCamera;
pub(super) use camera_preview::{rect as camera_preview_rect, update as update_camera_preview};
pub(super) use confirmation::update as update_confirmation;
pub(super) use control_hints::update as update_control_hints;
pub(super) use drawer::{
    AssetDrag, DrawerMotion, DrawerSize, actions as browser_actions, animate as animate_drawer,
    drag_assets, drag_visuals, refresh as refresh_browser, resize_drawer, search as search_browser,
    update_files as update_browser_files,
};
pub(super) use icons::SvgIconsPlugin;
pub(super) use image_viewer::edit::layers::ui::drag_ui::fx::LayerDragFxPlugin;
pub(super) use image_viewer::edit::layers::ui::flyout_fx::LayerFlyoutFxPlugin;
pub(super) use image_viewer::edit::color_drop::{ColorDropPlugin, input as color_drop_input};
pub(super) use image_viewer::edit::layers::ui::drag_ui::{
    LayerDrag, input as drag_photo_layers, visuals as refresh_photo_layer_drag,
};
pub(super) use image_viewer::edit::layers::ui::{
    input as photo_layers_input, refresh as refresh_photo_layers, sync_form as refresh_layer_form,
};
pub(super) use image_viewer::edit::selection_ui::ImageSelectionPlugin;
pub(super) use image_viewer::edit::{
    color_input as photo_color_input, color_pointer as photo_editor_input,
    refresh as refresh_photo_editor, refresh_color_picker as refresh_photo_color_picker,
};
pub(super) use image_viewer::{
    ImageAction, ImageCanvasPlugin, ImageViewer, cleanup_cursor as cleanup_image_cursor,
    cursor as image_viewer_cursor, edit::EditAction as ImageEditAction,
    refresh as refresh_image_viewer, resize_panel as resize_photo_editor, update as view_images,
};
pub(super) use lighting_ui::{
    LightingUi, edit_controls as edit_lighting_controls, edit_name as edit_light_name,
    refresh as refresh_lighting, refresh_controls as refresh_lighting_controls,
};
pub(super) use numeric_input::lock_text as lock_numeric_text;
pub(super) use numeric_input::{
    Number, Scrub as NumberScrub, scrub_cursor as number_scrub_cursor, scrub_input as scrub_numbers,
};
pub(super) use numeric_input::{NumericInput, update as update_numeric_input};
pub(super) use scene_objects::{ObjectAction as SceneObjectAction, SceneObjectsPlugin};
pub(super) use sidebar_resize::{
    SidebarLayout, layout as layout_sidebar, resize as resize_sidebar,
};

pub(super) const TOP: f32 = 94.0;
pub(super) const IMAGE_TOP: f32 = 52.0;
pub(super) const SIDE: f32 = 316.0;
pub(super) const DRAWER: f32 = 350.0;
pub(super) const FOOT: f32 = 34.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    Regular,
    Primary,
    Outline,
    Asset,
    Tool,
    Tab,
    TabClose,
}

#[derive(Component)]
pub(super) struct Control {
    pub(super) action: Action,
    look: Look,
}

#[derive(Component)]
pub(super) struct Scroller;

#[derive(Component)]
pub(super) struct HorizontalScroller;

#[derive(Resource)]
pub(super) struct Interface {
    font: Handle<Font>,
    tabs: Entity,
    toolbar: Entity,
    tool_bar: Entity,
    body: Entity,
    side: Entity,
    pub(super) viewport: Entity,
    scene_image: Entity,
    scene_badge: Entity,
    empty_scene: Entity,
    outliner: Entity,
    tree_chevron: Entity,
    inspector: Entity,
    dirty: Entity,
    selected: Entity,
    kind: Entity,
    hierarchy_info: Entity,
    perspective: Entity,
    lens_label: Entity,
    transforms: Entity,
    scale_row: Entity,
    lens_row: Entity,
    field_error: Entity,
    navigation_hint: Entity,
}

#[derive(Default)]
pub(super) struct OutlineState {
    room: Option<Room>,
    model: Option<usize>,
    imported: usize,
    models: Vec<u32>,
    hidden: Vec<bool>,
    objects: Vec<(Selection, String, bool)>,
    lights: Vec<(u32, String, bool, crate::lighting::LightKind)>,
    hierarchy: Vec<(
        u32,
        Option<String>,
        String,
        bool,
        bool,
        Option<String>,
        Option<String>,
        Option<String>,
        bool,
    )>,
    hierarchy_selection: Option<Selection>,
    hierarchy_parenting: Option<(Room, u32)>,
}

pub(super) fn viewport_rect(window: &Window, sidebar: &SidebarLayout) -> Rect {
    Rect::from_corners(
        Vec2::new(0.0, TOP),
        Vec2::new(
            window.width() - sidebar.width(window.width()),
            window.height() - FOOT,
        ),
    )
}

pub(super) fn scene_input_rect(
    window: &Window,
    drawer: &DrawerMotion,
    sidebar: &SidebarLayout,
) -> Rect {
    if drawer.resizing || sidebar.resizing() {
        return Rect::from_corners(Vec2::splat(-1.0), Vec2::splat(-1.0));
    }
    let mut rect = viewport_rect(window, sidebar);
    rect.max.y -= drawer.visible_height;
    rect
}

fn node(commands: &mut Commands, parent: Entity, node: Node) -> Entity {
    commands.spawn((node, ChildOf(parent))).id()
}

fn font(handle: &Handle<Font>, size: f32) -> TextFont {
    TextFont::from_font_size(size).with_font(handle.clone())
}

fn text(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    label: impl Into<String>,
    size: f32,
    color: Color,
) -> Entity {
    commands
        .spawn((
            Text::new(label),
            font(handle, size),
            TextColor(color),
            TextLayout::no_wrap(),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}

fn row(gap: f32) -> Node {
    Node {
        align_items: AlignItems::Center,
        column_gap: px(gap),
        ..default()
    }
}

fn column(gap: f32) -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        row_gap: px(gap),
        ..default()
    }
}

fn group(commands: &mut Commands, parent: Entity) -> Entity {
    surface(
        commands,
        parent,
        Node {
            padding: UiRect::all(px(3)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(9)),
            flex_shrink: 0.0,
            ..row(2.0)
        },
        WELL,
    )
}

fn inspector_card(commands: &mut Commands, parent: Entity) -> Entity {
    let card = surface(
        commands,
        parent,
        Node {
            width: percent(100),
            padding: UiRect::all(px(8)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            flex_shrink: 0.,
            ..column(6.)
        },
        WELL.with_alpha(0.55),
    );
    commands
        .entity(card)
        .insert(BorderColor::all(EDGE.with_alpha(0.65)));
    card
}

fn inspector_section(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    title: &str,
    hint: &str,
) -> Entity {
    let card = inspector_card(commands, parent);
    let heading = text(commands, card, font, title, 13., TEXT);
    control_hints::attach(commands, heading, hint);
    card
}

fn control(
    commands: &mut Commands,
    parent: Entity,
    action: Action,
    look: Look,
    node: Node,
) -> Entity {
    let mut entity = commands.spawn((
        Button,
        TabIndex(0),
        Hovered::default(),
        Control { action, look },
        node,
        BackgroundColor(Color::NONE),
        BackgroundGradient::default(),
        BorderColor::all(EDGE),
        ChildOf(parent),
    ));
    entity.observe(activate_control::<Activate, false>);
    if browser_open_action(action) {
        // File entries opt into double-click activation. Tabs, breadcrumbs and
        // context-menu commands keep the regular Button activation behavior.
        entity
            .observe(activate_control::<AssetActivate, true>)
            .observe(asset_pointer_click)
            .observe(asset_keypress);
        if matches!(look, Look::Asset) {
            entity.insert(AssetOpen);
        }
    }
    entity.id()
}

fn button(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: impl Into<String>,
    action: Action,
    look: Look,
) -> Entity {
    let outline = matches!(look, Look::Outline);
    let entity = control(
        commands,
        parent,
        action,
        look,
        Node {
            width: if outline { percent(100) } else { Val::Auto },
            height: px(if outline { 27 } else { 29 }),
            padding: UiRect::horizontal(px(9)),
            flex_shrink: 0.0,
            border_radius: BorderRadius::all(px(6)),
            border: UiRect::all(px(1)),
            overflow: Overflow::clip(),
            ..row(4.0)
        },
    );
    let label = label.into();
    commands
        .entity(entity)
        .insert(AccessibleLabel(label.clone()));
    text(commands, entity, font, label, 13.0, TEXT);
    entity
}

pub(super) fn setup(
    mut commands: Commands,
    mut fonts: ResMut<Assets<Font>>,
    mut images: ResMut<Assets<Image>>,
    scene_surface: Res<viewport::Surface>,
    mut transform_handless: ResMut<Assets<viewport::HandleMaterial>>,
    stars: Res<starfield::Starfield>,
    editor: Res<Editor>,
) {
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../assets/font/LiberationSans.ttf").to_vec(),
    ));
    let root = commands
        .spawn((
            Node {
                width: percent(100),
                height: percent(100),
                overflow: Overflow::clip(),
                ..column(0.0)
            },
            Pickable::IGNORE,
            TabGroup::default(),
        ))
        .id();
    let toolbar = surface(
        &mut commands,
        root,
        Node {
            position_type: PositionType::Absolute,
            top: px(0),
            left: px(0),
            right: px(0),
            height: px(TOP),
            padding: UiRect::axes(px(12), px(5)),
            border: UiRect::bottom(px(1)),
            ..column(3.0)
        },
        CHROME,
    );
    horizon(&mut commands, toolbar);
    starfield::background(&mut commands, toolbar, &stars.chrome);
    let project_bar = node(
        &mut commands,
        toolbar,
        Node {
            height: px(41),
            flex_shrink: 0.0,
            ..row(10.0)
        },
    );
    let brand = node(
        &mut commands,
        project_bar,
        Node {
            width: px(190),
            min_width: px(0),
            flex_shrink: 0.0,
            overflow: Overflow::clip(),
            ..row(8.0)
        },
    );
    crate::window_chrome::drag_region(&mut commands, project_bar);
    orbital::brand(&mut commands, brand);
    let name = node(
        &mut commands,
        brand,
        Node {
            min_width: px(0),
            overflow: Overflow::clip(),
            ..column(0.0)
        },
    );
    text(
        &mut commands,
        name,
        &font,
        &editor.project.title,
        16.0,
        TEXT,
    );
    text(&mut commands, name, &font, "РЕДАКТОР СЦЕН", 9.0, MUTED);

    let tabs = scene_tabs::setup(&mut commands, project_bar);
    crate::window_chrome::controls(&mut commands, project_bar);
    crate::window_chrome::resize_edges(&mut commands, root);
    let tool_bar = node(
        &mut commands,
        toolbar,
        Node {
            min_height: px(36),
            ..row(12.0)
        },
    );
    tool_icons::setup(&mut commands, tool_bar);
    model_viewer::editing::controls::geometry_toolbar(&mut commands, tool_bar);
    let tools = group(&mut commands, tool_bar);
    icons::button(
        &mut commands,
        tools,
        icons::Glyph::Box,
        "Использовать локальное пространство",
        Action::LocalSpace,
        Look::Regular,
    );
    icons::button(
        &mut commands,
        tools,
        icons::Glyph::Magnet,
        "Привязка",
        Action::Snap,
        Look::Regular,
    );
    let navigation_hint = text(
        &mut commands,
        tool_bar,
        &font,
        "Выберите объект в сцене или дереве",
        12.0,
        MUTED,
    );
    commands.entity(navigation_hint).insert(Node {
        min_width: px(0),
        flex_shrink: 1.0,
        overflow: Overflow::clip(),
        ..default()
    });
    play_ui::setup(&mut commands, tool_bar, root, &font);

    let body = node(
        &mut commands,
        root,
        Node {
            position_type: PositionType::Absolute,
            top: px(TOP),
            bottom: px(FOOT),
            left: px(0),
            right: px(0),
            min_height: px(0),
            ..default()
        },
    );
    let viewport = node(
        &mut commands,
        body,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            ..default()
        },
    );
    commands.entity(viewport).insert(Pickable::IGNORE);
    let scene_image = commands
        .spawn((
            ImageNode::new(scene_surface.0.clone()),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(viewport),
        ))
        .id();
    viewport::spawn_overlay(&mut commands, &mut transform_handless, viewport);
    let badge = surface(
        &mut commands,
        viewport,
        Node {
            position_type: PositionType::Absolute,
            top: px(14),
            left: px(14),
            padding: UiRect::axes(px(12), px(7)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(9)),
            ..default()
        },
        Color::srgba(0.043, 0.049, 0.066, 0.94),
    );
    commands.entity(badge).insert(Pickable::IGNORE);
    let perspective = text(
        &mut commands,
        badge,
        &font,
        "Перспектива · Свет сцены",
        12.0,
        TEXT,
    );

    camera_preview::setup(&mut commands, &mut images, viewport, &font);
    let empty_scene = scene_tabs::empty_workspace(&mut commands, viewport, &font, &stars);

    let side = surface(
        &mut commands,
        body,
        Node {
            width: px(SIDE),
            min_height: px(0),
            flex_shrink: 0.0,
            border: UiRect::left(px(1)),
            padding: UiRect::left(px(sidebar_resize::GRIP)),
            ..column(0.0)
        },
        PANEL,
    );
    sidebar_resize::width_grip(&mut commands, side);
    starfield::background(&mut commands, side, &stars.chrome);
    let header = surface(
        &mut commands,
        side,
        Node {
            height: px(37),
            flex_shrink: 0.0,
            padding: UiRect::horizontal(px(13)),
            border: UiRect::bottom(px(1)),
            ..row(0.0)
        },
        CHROME,
    );
    let tree_toggle = control(
        &mut commands,
        header,
        Action::ToggleSceneTree,
        Look::Regular,
        Node {
            height: percent(100),
            flex_grow: 1.0,
            min_width: px(0),
            padding: UiRect::horizontal(px(4)),
            border_radius: BorderRadius::all(px(4)),
            overflow: Overflow::clip(),
            ..row(6.0)
        },
    );
    commands
        .entity(tree_toggle)
        .insert(AccessibleLabel("Дерево сцены".into()));
    let tree_chevron = icons::themed(&mut commands, tree_toggle, icons::Glyph::ChevronDown, 12.0);
    text(
        &mut commands,
        tree_toggle,
        &font,
        "Дерево сцены",
        14.0,
        TEXT,
    );
    horizon(&mut commands, header);
    let outliner = node(
        &mut commands,
        side,
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            overflow: Overflow::scroll_y(),
            padding: UiRect::axes(px(6), px(5)),
            ..column(2.0)
        },
    );
    commands.entity(outliner).insert((
        Scroller,
        sidebar_resize::SceneTree,
        AccessibleLabel("Список объектов сцены".into()),
    ));
    sidebar_resize::height_grip(&mut commands, side);

    let inspector = surface(
        &mut commands,
        side,
        Node {
            height: px(300),
            flex_shrink: 0.0,
            padding: UiRect::all(px(12)),
            border: UiRect::top(px(1)),
            overflow: Overflow::scroll_y(),
            ..column(6.0)
        },
        CHROME,
    );
    commands.entity(inspector).insert(Scroller);
    sidebar_resize::inspector(&mut commands, inspector);
    starfield::background(&mut commands, inspector, &stars.chrome);
    horizon(&mut commands, inspector);
    text(&mut commands, inspector, &font, "Инспектор", 13.0, MUTED);
    let selected = text(
        &mut commands,
        inspector,
        &font,
        "Выберите объект",
        17.0,
        TEXT,
    );
    let kind = text(&mut commands, inspector, &font, "", 12.0, MUTED);
    let hierarchy_info = node(&mut commands, inspector, column(4.0));
    lighting_ui::setup(&mut commands, header, inspector, root, &font, &mut images);
    let transforms = node(&mut commands, inspector, column(6.0));
    transform_row(&mut commands, transforms, &font, "Позиция", Field::Position);
    transform_row(&mut commands, transforms, &font, "Поворот", Field::Rotation);
    let scale_row = transform_row(&mut commands, transforms, &font, "Масштаб", Field::Scale);
    let lens_row = node(&mut commands, transforms, row(4.0));
    let lens_label = field_caption(&mut commands, lens_row, &font, "Обзор, °");
    number(&mut commands, lens_row, &font, Field::Lens, "·", BLUE);
    let actions = node(
        &mut commands,
        transforms,
        Node {
            flex_wrap: FlexWrap::Wrap,
            ..row(3.0)
        },
    );
    for (label, action) in [
        ("Управление камерой", Action::Pilot),
        ("Из вида", Action::Align),
        ("Дублировать", Action::Duplicate),
        ("Удалить", Action::Delete),
    ] {
        button(&mut commands, actions, &font, label, action, Look::Regular);
    }
    let field_error = text(&mut commands, transforms, &font, "", 11.0, ERROR);
    commands.entity(field_error).insert((
        TextLayout::default(),
        Node {
            display: Display::None,
            ..default()
        },
    ));

    let checker = image_viewer::checker_texture(&mut images);
    drawer::setup(&mut commands, root, &font, &stars, &checker);
    image_viewer::setup(&mut commands, viewport, root, &font, &checker);
    browser_context::setup(&mut commands, root, &font);
    narrative::setup(&mut commands, root, &font, &stars, &mut fonts);
    generation::setup(&mut commands, root, &font, &checker);

    let footer = surface(
        &mut commands,
        root,
        Node {
            position_type: PositionType::Absolute,
            bottom: px(0),
            left: px(0),
            right: px(0),
            height: px(FOOT),
            padding: UiRect::horizontal(px(8)),
            border: UiRect::top(px(1)),
            overflow: Overflow::clip(),
            ..row(8.0)
        },
        CHROME,
    );
    commands.entity(footer).insert(ZIndex(20));
    drawer::footer_toggle(&mut commands, footer, &font);
    generation::footer(&mut commands, footer, &font);
    node(
        &mut commands,
        footer,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            overflow: Overflow::clip(),
            ..row(0.0)
        },
    );
    generation::service_footer(&mut commands, footer, &font);
    let dirty = text(&mut commands, footer, &font, "●", 11.0, GREEN);
    control_hints::setup(&mut commands, root, &font);
    confirmation::setup(&mut commands, root, &font);
    commands.insert_resource(Interface {
        font,
        tabs,
        toolbar,
        tool_bar,
        body,
        side,
        viewport,
        scene_image,
        scene_badge: badge,
        empty_scene,
        outliner,
        tree_chevron,
        inspector,
        dirty,
        selected,
        kind,
        hierarchy_info,
        perspective,
        lens_label,
        transforms,
        scale_row,
        lens_row,
        field_error,
        navigation_hint,
    });
}

fn outline_entry(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    selection: Selection,
    kind: Option<crate::lighting::LightKind>,
    enabled: bool,
) -> Entity {
    let entry = button(
        commands,
        parent,
        font,
        label,
        Action::Select(selection),
        Look::Outline,
    );
    commands
        .entity(entry)
        .entry::<Node>()
        .and_modify(|mut node| node.padding.left = px(26));
    let color = if enabled { AMBER } else { MUTED };
    let glyph = match selection {
        Selection::Node(_) => icons::Glyph::Layers,
        Selection::Environment => icons::Glyph::Globe,
        Selection::Camera(_) => icons::Glyph::Camera,
        Selection::Imported(_) | Selection::Model(_) => icons::Glyph::Box,
        Selection::Light(_) => match kind {
            Some(crate::lighting::LightKind::Directional) => icons::Glyph::Sun,
            Some(crate::lighting::LightKind::Spot) => icons::Glyph::Spotlight,
            Some(crate::lighting::LightKind::Rect) => icons::Glyph::Rectangle,
            _ => icons::Glyph::Lightbulb,
        },
    };
    let tint = if matches!(selection, Selection::Light(_) | Selection::Environment) {
        color
    } else {
        TEXT
    };
    let icon = icons::spawn(commands, entry, glyph, 14.0, tint);
    commands
        .entity(icon)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.position_type = PositionType::Absolute;
            node.left = px(8);
            node.top = px(6);
        });
    entry
}

fn object_outline_entry(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    editor: &Editor,
    target: Selection,
) {
    let hidden = editor.object_hidden(target);
    let label = editor.label(target);
    let container = node(
        commands,
        parent,
        Node {
            width: percent(100),
            height: px(28),
            flex_shrink: 0.0,
            ..row(0.0)
        },
    );
    let name = node(
        commands,
        container,
        Node {
            flex_grow: 1.0,
            flex_basis: px(0),
            min_width: px(0),
            overflow: Overflow::clip(),
            ..default()
        },
    );
    let light = match target {
        Selection::Light(id) => editor.light(id),
        _ => None,
    };
    let item = outline_entry(
        commands,
        name,
        font,
        &label,
        target,
        light.map(|light| light.kind),
        !hidden && light.is_none_or(|light| light.enabled),
    );
    commands.entity(item).observe(scene_objects::pointer_menu);
    if editor.has_scene() && editor.definition().is_native()
        && matches!(target, Selection::Model(_) | Selection::Light(_) | Selection::Camera(_))
    {
        scene_tree_drag::mark(commands, item, Some(target));
    }
    let eye = control(
        commands,
        container,
        Action::ToggleVisibility(target),
        Look::Regular,
        Node {
            width: px(28),
            height: px(28),
            flex_shrink: 0.0,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
    );
    commands.entity(eye).insert(AccessibleLabel(format!(
        "{}: {label}",
        if hidden {
            "Показать объект"
        } else {
            "Скрыть объект"
        }
    )));
    icons::spawn(
        commands,
        eye,
        if hidden {
            icons::Glyph::EyeOff
        } else {
            icons::Glyph::Eye
        },
        16.0,
        if hidden { MUTED } else { TEXT },
    );
}

fn field_caption(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
) -> Entity {
    let cell = node(
        commands,
        parent,
        Node {
            width: px(70),
            flex_shrink: 0.0,
            ..default()
        },
    );
    text(commands, cell, font, label, 12.0, MUTED)
}

fn transform_row(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    field: fn(usize) -> Field,
) -> Entity {
    let line = node(commands, parent, row(4.0));
    field_caption(commands, line, font, label);
    for (index, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
        number(commands, line, font, field(index), axis, AXIS[index]);
    }
    line
}

fn number(
    commands: &mut Commands,
    parent: Entity,
    font_handle: &Handle<Font>,
    field: Field,
    axis: &str,
    accent: Color,
) {
    let cell = node(
        commands,
        parent,
        Node {
            width: px(69),
            ..row(3.0)
        },
    );
    text(commands, cell, font_handle, axis, 11.0, accent);
    commands.spawn((
        field,
        Number::scene(field),
        TextInput,
        AccessibleLabel(match field {
            Field::Position(_) => format!("Позиция {axis}"),
            Field::Rotation(_) => format!("Поворот {axis}"),
            Field::Scale(_) => format!("Масштаб {axis}"),
            Field::Lens => "Объектив камеры".to_owned(),
            Field::Light(_) => format!("Параметр света {axis}"),
            Field::Environment(_) => format!("Освещение сцены {axis}"),
        }),
        EditableText::new("0.00"),
        crate::ui_theme::text_cursor_style(),
        font(font_handle, 12.0),
        TextColor(TEXT),
        TextLayout::no_wrap(),
        TabIndex(0),
        BackgroundColor(WELL),
        BorderColor::all(EDGE),
        ChildOf(cell),
        Node {
            width: px(55),
            height: px(27),
            padding: UiRect::axes(px(4), px(3)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            overflow: Overflow::clip(),
            ..default()
        },
    ));
}

fn available(editor: &Editor, browser: &Browser, action: Action) -> bool {
    if matches!(action, Action::Play) && editor.play.running() {
        return true;
    }
    if editor.scene_object_popup && !matches!(action, Action::SceneObject(_)) {
        return false;
    }
    if editor.pending.is_some() {
        return match action {
            Action::SaveAndContinue => {
                editor.ready
                    || matches!(
                        editor.pending,
                        Some(
                            crate::editor::PendingAction::CloseNarrative(_)
                                | crate::editor::PendingAction::ExitNarrative(_)
                                | crate::editor::PendingAction::CloseModelTab(_)
                                | crate::editor::PendingAction::CloseModelEditor(_)
                        )
                    )
            }
            Action::DiscardAndContinue | Action::CancelPending => true,
            _ => false,
        };
    }
    if editor.hdri_library_open {
        return matches!(
            action,
            Action::HdriLibrary(_) | Action::UseDownloadedHdri(_)
        );
    }
    if editor.generation.active && !Editor::model_tab_action(action) {
        return false;
    }
    if let Action::Generation(action) = action {
        return generation::available(editor, action);
    }
    if editor.narrative.active.is_some() && !Editor::model_tab_action(action) {
        return false;
    }
    if let Action::Narrative(narrative) = action {
        return (!editor.browser_popup || matches!(narrative, NarrativeAction::Create(_)))
            && narrative::available(editor, narrative);
    }
    if editor.viewing_image.is_some() && !Editor::model_tab_action(action) {
        return false;
    }
    if editor.viewing_model.is_some() && model_viewer::editing::toolbar_action(action).is_some() {
        return !editor.browser_popup;
    }
    if editor.viewing_model.is_some() && !Editor::model_tab_action(action) {
        return false;
    }
    if editor.browser_popup {
        return matches!(
            action,
            Action::BrowserNewFolder
                | Action::BrowserNewImage
                | Action::BrowserCreateImage
                | Action::BrowserNewScene
                | Action::BrowserCreateScene
                | Action::BrowserDialogueMenu
                | Action::BrowserPrimitiveMenu
                | Action::BrowserCreatePrimitive(_)
                | Action::BrowserConfirmPrimitive
                | Action::BrowserCreateFolder
                | Action::BrowserCancelPopup
                | Action::BrowserDeleteFile
                | Action::BrowserOpenEntry
                | Action::BrowserRevealEntry
                | Action::BrowserInstanceScene
                | Action::BrowserRenameFile
                | Action::BrowserConfirmRename
                | Action::BrowserDuplicateFile
                | Action::BrowserRestoreFile
        );
    }
    match action {
        Action::Hierarchy(action) => super::scene_hierarchy::available(editor, action),
        Action::ImageEdit(_) => editor.viewing_image.is_some(),
        Action::EditBrowserImage(file, revision) => {
            editor.drawer
                && revision == browser.scan_revision
                && browser
                    .files
                    .get(file)
                    .is_some_and(|file| file.kind == browser::FileKind::Image)
        }
        Action::ImageView(ImageAction::Previous) => {
            editor.viewing_image.is_some()
                && image_viewer::neighbor(editor, browser, false).is_some()
        }
        Action::ImageView(ImageAction::Next) => {
            editor.viewing_image.is_some()
                && image_viewer::neighbor(editor, browser, true).is_some()
        }
        Action::ImageView(_) => editor.viewing_image.is_some(),
        Action::OpenImageTab(id) | Action::CloseImageTab(id) => editor.image_tab(id).is_some(),
        Action::HdriLibrary(HdriAction::Open) | Action::UseDownloadedHdri(_) => {
            editor.has_scene() && editor.ready
        }
        Action::HdriLibrary(_) => false,
        Action::ModelView(_) => editor.viewing_model.is_some(),
        Action::CloseModelTab(index) => editor.model_tabs.contains(&index),
        Action::ViewModel(index) => index < editor.project.catalog.len(),
        Action::BrowserCreateFolder
        | Action::BrowserCreateImage
        | Action::BrowserCreateScene
        | Action::BrowserConfirmPrimitive
        | Action::BrowserCancelPopup => false,
        Action::BrowserNewFolder
        | Action::BrowserNewImage
        | Action::BrowserNewScene
        | Action::BrowserOpenEntry
        | Action::BrowserRevealEntry
        | Action::BrowserInstanceScene
        | Action::BrowserRenameFile
        | Action::BrowserConfirmRename
        | Action::BrowserDuplicateFile
        | Action::BrowserDialogueMenu
        | Action::BrowserPrimitiveMenu
        | Action::BrowserCreatePrimitive(_) => editor.drawer,
        Action::SaveAndContinue | Action::DiscardAndContinue | Action::CancelPending => false,
        Action::Open(room) => room < editor.project.scenes.len(),
        Action::CloseTab(room) => editor.open_tabs.contains(&room),
        Action::Tool(tool) => {
            editor.has_scene()
                && editor.ready
                && (tool == Tool::Universal
                    || match editor.selection {
                        Some(Selection::Environment) => false,
                        Some(target @ Selection::Light(_)) => {
                            (0..3).any(|axis| viewport::allowed(editor, target, tool, axis))
                        }
                        _ => true,
                    })
        }
        Action::Select(_) => editor.has_scene() && editor.ready,
        Action::ToggleVisibility(target) => {
            editor.has_scene() && editor.ready && editor.can_toggle_visibility(target)
        }
        Action::LocalSpace | Action::Snap => editor.has_scene() && editor.ready,
        Action::Lighting(_) | Action::LightMenu | Action::LightAdvanced => {
            editor.has_scene() && editor.ready
        }
        Action::BrowseBack => browser.can_go_back(),
        Action::BrowseForward => browser.can_go_forward(),
        Action::BrowseUp => browser.current != 0,
        Action::Save => editor.has_scene() && editor.ready,
        Action::Play => editor.has_scene() && editor.ready,
        Action::Undo => {
            editor.has_scene() && (!editor.undo.is_empty() || editor.camera_navigation.is_some())
        }
        Action::Redo => editor.has_scene() && !editor.redo.is_empty(),
        #[cfg(test)]
        Action::Add(_) => editor.has_scene() && editor.ready,
        Action::Pilot => {
            editor.has_scene()
                && editor.ready
                && matches!(editor.selection, Some(Selection::Camera(_)))
        }
        Action::Align => {
            editor.has_scene()
                && matches!(editor.selection, Some(Selection::Camera(_)))
                && !editor.pilot
        }
        Action::Focus => editor.has_scene() && editor.ready,
        Action::Reset(_) => editor.has_scene() && editor.ready && editor.selection.is_some(),
        Action::Delete => editor.has_scene() && editor.deletable_targets(),
        Action::Duplicate => editor.has_scene() && editor.batch_editable(),
        Action::SceneObject(action) => scene_objects::available(editor, action),
        _ => true,
    }
}

pub(super) fn input(
    editor: Res<Editor>,
    drawer: Res<DrawerMotion>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut focus: ResMut<InputFocus>,
    lighting: Option<Res<LightingUi>>,
) {
    if editor.scene_object_popup {
        return;
    }
    if editor.pending.is_some()
        || editor.browser_popup
        || editor.asset_viewer_open()
        || lighting
            .as_ref()
            .is_some_and(|lighting| lighting.blocks_scene_input())
    {
        return;
    }
    if mouse.any_just_pressed([MouseButton::Left, MouseButton::Right, MouseButton::Middle])
        && window.cursor_position().is_some_and(|cursor| {
            scene_input_rect(&window, &drawer, &editor.sidebar).contains(cursor)
        })
    {
        focus.clear();
    }
}

#[derive(EntityEvent)]
struct AssetActivate {
    entity: Entity,
}

#[derive(Component)]
pub(super) struct AssetOpen;

fn browser_open_action(action: Action) -> bool {
    matches!(
        action,
        Action::Browse(_)
            | Action::ViewImage(_)
            | Action::ViewModel(_)
            | Action::Open(_)
            | Action::BrowseFile(_)
            | Action::Narrative(NarrativeAction::File(_))
    )
}

fn asset_pointer_click(
    mut click: On<PointerClick>,
    controls: Query<(&Control, Option<&drawer::FileCard>), With<AssetOpen>>,
    editor: Res<Editor>,
    browser: Res<Browser>,
    drag: Res<AssetDrag>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut browser_ui: Option<ResMut<drawer::BrowserUi>>,
    mut focus: ResMut<InputFocus>,
    mut commands: Commands,
) {
    let Ok((control, card)) = controls.get(click.entity) else {
        return;
    };
    click.propagate(false);
    if click.button != PointerButton::Primary
        || drag.suppress_activation_at(window.cursor_position())
        || !available(&editor, &browser, control.action)
        || card.is_some_and(|card| card.1 != browser.scan_revision)
    {
        return;
    }
    if let Some(ui) = browser_ui.as_mut() {
        let entry = card.map(|card| card.0).or_else(|| match control.action {
            Action::Browse(folder) => Some(browser::Entry::Folder(folder)),
            Action::ViewModel(model) => Some(browser::Entry::Model(model)),
            Action::ViewImage(file) => Some(browser::Entry::Image(file)),
            Action::BrowseFile(file) | Action::Narrative(NarrativeAction::File(file)) => {
                Some(browser::Entry::File(file))
            }
            _ => None,
        });
        ui.select(entry);
    }
    focus.set(click.entity, bevy::input_focus::FocusCause::Pressed);
    if click.count == 2 {
        commands.trigger(AssetActivate {
            entity: click.entity,
        });
    }
}

fn asset_keypress(
    mut event: On<FocusedInput<KeyboardInput>>,
    entries: Query<(), With<AssetOpen>>,
    mut commands: Commands,
) {
    if !entries.contains(event.focused_entity) {
        return;
    }
    let input = &event.input;
    if !input.repeat
        && input.state == ButtonState::Pressed
        && matches!(input.key_code, KeyCode::Enter | KeyCode::Space)
    {
        event.propagate(false);
        commands.trigger(AssetActivate {
            entity: event.focused_entity,
        });
    }
}

fn activate_control<E: EntityEvent, const EXPLICIT: bool>(
    activate: On<E>,
    editor: Res<Editor>,
    browser: Res<Browser>,
    drag: Res<AssetDrag>,
    window: Single<&Window, With<PrimaryWindow>>,
    controls: Query<&Control>,
    entries: Query<(), With<AssetOpen>>,
    mut effects: Query<&mut button_fx::Effect>,
    mut focus: ResMut<InputFocus>,
    mut messages: MessageWriter<Action>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    color_drop: Option<Res<image_viewer::edit::color_drop::ColorDrop>>,
    tree_drag: Option<Res<scene_tree_drag::TreeDrag>>,
) {
    // Bevy's Button emits Activate for secondary clicks as well. Reserve those
    // for context menus even when press and release arrive in the same frame.
    if mouse.as_ref().is_some_and(|mouse| {
        mouse.any_pressed([MouseButton::Right, MouseButton::Middle])
            || mouse.any_just_released([MouseButton::Right, MouseButton::Middle])
    }) {
        return;
    }
    let entity = activate.event_target();
    if tree_drag.is_some_and(|drag| drag.suppresses(window.cursor_position())) {
        return;
    }
    if color_drop.as_ref().is_some_and(|drag| drag.suppresses(entity)) {
        return;
    }
    if !EXPLICIT && entries.contains(entity) {
        return;
    }
    if !drag.suppress_activation_at(window.cursor_position())
        && let Ok(control) = controls.get(entity)
        && available(&editor, &browser, control.action)
    {
        if let Ok(mut effect) = effects.get_mut(entity) {
            effect.activate();
        }
        if !matches!(
            control.action,
            Action::SaveAndContinue | Action::DiscardAndContinue | Action::CancelPending
        ) {
            focus.clear();
        }
        messages.write(control.action);
    }
}

pub(super) fn shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut focus: ResMut<InputFocus>,
    numbers: Query<(), (With<Field>, With<TextInput>)>,
    inputs: Query<(), With<TextInput>>,
    mut actions: MessageWriter<Action>,
    editor: Res<Editor>,
    drag: Option<Res<AssetDrag>>,
    layer_drag: Option<Res<LayerDrag>>,
    lighting: Option<Res<LightingUi>>,
    browser_context: Option<Res<BrowserContext>>,
    image_viewer: Option<Res<ImageViewer>>,
    model_edit: Option<Res<model_viewer::editing::State>>,
    sliders: Query<(), With<bevy::ui_widgets::Slider>>,
    (pointer, scrub, tree_drag): (
        Option<Res<viewport::PointerState>>,
        Option<Res<NumberScrub>>,
        Option<Res<scene_tree_drag::TreeDrag>>,
    ),
) {
    if editor.scene_object_popup {
        return;
    }
    if layer_drag.as_ref().is_some_and(|drag| drag.capturing())
        || tree_drag.is_some_and(|drag| drag.suppresses(None))
    {
        return;
    }
    if image_viewer.as_ref().is_some_and(|viewer| viewer.edit.color_dragging) {
        return;
    }
    if scrub.as_ref().is_some_and(|scrub| scrub.capturing())
        || pointer.as_ref().is_some_and(|pointer| pointer.selecting())
        || model_edit
            .as_ref()
            .is_some_and(|model| model.box_start.is_some())
    {
        return;
    }
    if keys.any_just_pressed([
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
    ]) && focus.get().is_some_and(|entity| sliders.contains(entity))
    {
        return;
    }
    if editor.hdri_library_open && editor.pending.is_none() {
        if keys.just_pressed(KeyCode::Escape) {
            actions.write(Action::HdriLibrary(HdriAction::Close));
        } else if keys.just_pressed(KeyCode::Enter)
            && focus.get().is_some_and(|entity| inputs.contains(entity))
        {
            actions.write(Action::HdriLibrary(HdriAction::Search));
        }
        return;
    }
    if editor.browser_popup && editor.pending.is_none() {
        browser_context::shortcuts(&keys, browser_context.as_deref(), &mut actions);
        return;
    }
    if editor.generation.active && editor.pending.is_none() {
        if keys.just_pressed(KeyCode::Escape)
            && drag.as_ref().is_some_and(|drag| drag.cancelled_this_frame)
        {
            return;
        }
        generation::shortcuts(
            &keys,
            focus.get().is_some_and(|e| inputs.contains(e)),
            &mut actions,
        );
        return;
    }
    if editor.narrative.active.is_some() && editor.pending.is_none() {
        narrative::shortcuts(
            &keys,
            focus.get().is_some_and(|e| inputs.contains(e)),
            &mut actions,
        );
        return;
    }
    if editor.viewing_model.is_some() && editor.viewing_image.is_none() && editor.pending.is_none()
    {
        if mouse.pressed(MouseButton::Right) {
            return;
        }
        let command = keys.any_pressed([
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
        ]);
        if focus.get().is_some_and(|entity| inputs.contains(entity)) {
            return;
        }
        if model_viewer::editing::shortcuts(
            &keys,
            model_edit.as_deref(),
            editor.viewing_model,
            &mut actions,
        ) {
            return;
        }
        if command && keys.just_pressed(KeyCode::KeyW) {
            actions.write(Action::ModelView(ModelViewAction::Close));
        }
        if keys.just_pressed(KeyCode::F5) {
            actions.write(Action::ModelView(ModelViewAction::Retry));
        }
        if command && keys.just_pressed(KeyCode::Space) {
            actions.write(Action::Drawer);
        }
        if command && keys.just_pressed(KeyCode::KeyO) {
            actions.write(Action::OpenBrowser);
        }
        return;
    }
    if editor.viewing_image.is_some() && editor.pending.is_none() {
        if image_viewer
            .as_ref()
            .is_some_and(|viewer| viewer.edit.color_picker.is_some())
        {
            if keys.just_pressed(KeyCode::Escape) {
                actions.write(Action::ImageEdit(ImageEditAction::ColorCancel));
            }
            return;
        }
        if image_viewer
            .as_ref()
            .is_some_and(|viewer| viewer.edit.layers.form.is_some() || viewer.edit.layers.menu)
        {
            return;
        }
        if image_viewer
            .as_ref()
            .is_some_and(|viewer| viewer.resizing_panel())
        {
            return;
        }
        if keys.just_pressed(KeyCode::Escape)
            && image_viewer
                .as_ref()
                .is_some_and(|viewer| viewer.edit.resizing_brush())
        {
            return;
        }
        if keys.just_pressed(KeyCode::Escape)
            && image_viewer
                .as_ref()
                .is_some_and(|viewer| viewer.edit.size_menu)
        {
            return;
        }
        let command = keys.any_pressed([
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
        ]);
        if command && keys.just_pressed(KeyCode::KeyW) {
            actions.write(Action::CloseImageTab(editor.viewing_image.unwrap()));
            return;
        }
        if command && keys.just_pressed(KeyCode::KeyS) {
            actions.write(Action::ImageEdit(ImageEditAction::Save));
            return;
        }
        if command && keys.just_pressed(KeyCode::Space) {
            actions.write(Action::Drawer);
            return;
        }
        if command && keys.just_pressed(KeyCode::KeyO) {
            actions.write(Action::OpenBrowser);
            return;
        }
        if command
            && editor.drawer
            && keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight])
            && keys.just_pressed(KeyCode::KeyN)
        {
            actions.write(Action::BrowserNewFolder);
            return;
        }
        if focus.get().is_some_and(|entity| inputs.contains(entity)) {
            if keys.just_pressed(KeyCode::Escape) {
                focus.clear();
            }
            return;
        }
        if keys.just_pressed(KeyCode::Escape) {
            if let Some(viewer) = image_viewer
                .as_ref()
                .filter(|viewer| viewer.edit.paint_menu)
            {
                actions.write(Action::ImageEdit(
                    if viewer
                        .edit
                        .active(ImageEditAction::EraserMenu, editor.viewing_image)
                    {
                        ImageEditAction::EraserMenu
                    } else {
                        ImageEditAction::PaintMenu
                    },
                ));
            } else if image_viewer.as_ref().is_some_and(|viewer| viewer.edit.menu) {
                actions.write(Action::ImageEdit(ImageEditAction::Menu));
            } else if editor.drawer {
                actions.write(Action::Drawer);
            } else {
                focus.clear();
            }
            return;
        }
        if !image_viewer::edit::shortcuts(&keys, image_viewer.as_deref(), &mut actions) {
            image_viewer::shortcuts(&keys, &mut actions);
        }
        return;
    }
    if lighting
        .as_ref()
        .is_some_and(|lighting| lighting.is_menu_open())
        || (keys.just_pressed(KeyCode::Escape)
            && (editor.sidebar.resizing()
                || lighting
                    .as_ref()
                    .is_some_and(|lighting| lighting.is_control_active())))
    {
        return;
    }
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    if editor.pending.is_some() {
        if keys.just_pressed(KeyCode::Escape) {
            actions.write(Action::CancelPending);
        } else if command && keys.just_pressed(KeyCode::KeyS) {
            actions.write(Action::SaveAndContinue);
        }
        return;
    }
    if editor.drawer
        && command
        && keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight])
        && keys.just_pressed(KeyCode::KeyN)
    {
        actions.write(Action::BrowserNewFolder);
        return;
    }
    if keys.just_pressed(KeyCode::Escape)
        && drag.as_ref().is_some_and(|drag| drag.cancelled_this_frame)
    {
        return;
    }
    if command && keys.just_pressed(KeyCode::Space) {
        focus.clear();
        actions.write(Action::Drawer);
        return;
    }
    if command && keys.just_pressed(KeyCode::KeyO) {
        focus.clear();
        actions.write(Action::OpenBrowser);
        return;
    }
    if command && keys.just_pressed(KeyCode::KeyW) && editor.has_scene() {
        focus.clear();
        actions.write(Action::CloseTab(editor.room));
        return;
    }
    if command && keys.just_pressed(KeyCode::KeyS) {
        focus.clear();
        actions.write(Action::Save);
        return;
    }
    if keys.just_pressed(KeyCode::F5)
        && (editor.play.running() || (editor.has_scene() && editor.ready))
    {
        focus.clear();
        actions.write(Action::Play);
        return;
    }
    if focus.get().is_some_and(|entity| numbers.contains(entity)) {
        return;
    }
    // Typing a model name must not trigger scene tools or Delete.
    if focus.get().is_some_and(|entity| inputs.contains(entity)) {
        if keys.just_pressed(KeyCode::Escape) {
            focus.clear();
            if editor.drawer {
                actions.write(Action::Drawer);
            }
        }
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if keys.just_pressed(KeyCode::Escape) {
        if editor.drawer {
            actions.write(Action::Drawer);
        } else if editor.pilot {
            actions.write(Action::Pilot);
        }
        return;
    }
    if command {
        for (key, action) in [
            (
                KeyCode::KeyZ,
                if shift { Action::Redo } else { Action::Undo },
            ),
            (KeyCode::KeyY, Action::Redo),
            (KeyCode::KeyD, Action::Duplicate),
        ] {
            if keys.just_pressed(key) {
                actions.write(action);
            }
        }
    } else if !editor.pilot && !mouse.pressed(MouseButton::Right) {
        if keys.just_pressed(KeyCode::F2)
            && let Some(target) = editor
                .selection
                .filter(|&target| editor.object_editable(target))
        {
            actions.write(Action::SceneObject(SceneObjectAction::Rename(target)));
            return;
        }
        if keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]) {
            for (key, reset) in [
                (KeyCode::KeyQ, Reset::All),
                (KeyCode::KeyW, Reset::Position),
                (KeyCode::KeyE, Reset::Rotation),
                (KeyCode::KeyR, Reset::Scale),
            ] {
                if keys.just_pressed(key) {
                    actions.write(Action::Reset(reset));
                }
            }
            return;
        }
        for (key, action) in [
            (KeyCode::KeyQ, Action::Tool(Tool::Universal)),
            (KeyCode::KeyW, Action::Tool(Tool::Move)),
            (KeyCode::KeyE, Action::Tool(Tool::Rotate)),
            (KeyCode::KeyR, Action::Tool(Tool::Scale)),
            (KeyCode::NumpadDecimal, Action::Focus),
            (KeyCode::Delete, Action::Delete),
        ] {
            if keys.just_pressed(key) {
                actions.write(action);
            }
        }
    }
}

pub(super) fn scroll(
    editor: Res<Editor>,
    viewer: Option<Res<ImageViewer>>,
    window: Single<&Window, With<PrimaryWindow>>,
    drawer: Res<DrawerMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    lighting: Option<Res<LightingUi>>,
    mut areas: Query<
        (
            &ComputedNode,
            &UiGlobalTransform,
            &mut ScrollPosition,
            Has<drawer::DrawerScroll>,
            Has<HorizontalScroller>,
            Has<image_viewer::edit::ColorPickerScroll>,
            Has<image_viewer::edit::layers::ui::LayerScroll>,
        ),
        With<Scroller>,
    >,
) {
    if lighting
        .as_ref()
        .is_some_and(|lighting| lighting.is_menu_open())
    {
        return;
    }
    if editor.pending.is_some()
        || editor.hdri_library_open
        || editor.browser_popup
        || scroll.delta == Vec2::ZERO
    {
        return;
    }
    let Some(cursor) = window
        .cursor_position()
        .map(|pos| pos * window.scale_factor())
    else {
        return;
    };
    let color_picker = viewer
        .as_ref()
        .is_some_and(|viewer| viewer.edit.color_picker.is_some());
    if !color_picker && cursor.y / window.scale_factor() >= window.height() - FOOT {
        return;
    }
    let over_drawer = drawer.visible_height > 0.0
        && cursor.y / window.scale_factor() >= window.height() - FOOT - drawer.visible_height;
    let layer_popup = viewer
        .as_ref()
        .is_some_and(|v| v.edit.layers.menu || v.edit.layers.form.is_some());
    let layer_area = if color_picker {
        None
    } else {
        areas
            .iter()
            .filter(|(computed, transform, _, _, _, _, layer)| {
                *layer
                    && computed.size().min_element() > 0.0
                    && !(cursor - transform.affine().translation)
                        .abs()
                        .cmpgt(computed.size() * 0.5)
                        .any()
            })
            .map(|(computed, _, _, _, _, _, _)| computed.size().x * computed.size().y)
            .min_by(f32::total_cmp)
    };
    for (computed, transform, mut offset, drawer_scroll, horizontal, picker_scroll, layer_scroll) in
        &mut areas
    {
        if !color_picker
            && (layer_area
                .is_some_and(|area| !layer_scroll || computed.size().x * computed.size().y != area)
                || layer_popup && !layer_scroll)
        {
            continue;
        }
        if color_picker != picker_scroll || (!color_picker && over_drawer != drawer_scroll) {
            continue;
        }
        let center = transform.affine().translation;
        if computed.size().min_element() <= 0.0
            || (cursor - center).abs().cmpgt(computed.size() * 0.5).any()
        {
            continue;
        }
        let amount = if scroll.unit == MouseScrollUnit::Line {
            33.0
        } else {
            1.0
        };
        if horizontal {
            let delta = if scroll.delta.x != 0.0 {
                scroll.delta.x
            } else {
                scroll.delta.y
            };
            let max = ((computed.content_size().x - computed.size().x)
                * computed.inverse_scale_factor())
            .max(0.0);
            offset.x = (offset.x - delta * amount).clamp(0.0, max);
        } else {
            let max = ((computed.content_size().y - computed.size().y)
                * computed.inverse_scale_factor())
            .max(0.0);
            offset.y = (offset.y - scroll.delta.y * amount).clamp(0.0, max);
        }
    }
}

pub(super) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    interface: Res<Interface>,
    browser: Res<Browser>,
    (image_viewer, model_edit): (
        Option<Res<ImageViewer>>,
        Option<Res<model_viewer::editing::State>>,
    ),
    (focus, focus_visible): (Res<InputFocus>, Res<bevy::input_focus::InputFocusVisible>),
    (browser_ui, browser_context): (Res<drawer::BrowserUi>, Res<BrowserContext>),
    numeric: Res<NumericInput>,
    lighting: Res<LightingUi>,
    (mut outline, mut tabs, mut inspector_selection): (
        Local<OutlineState>,
        Local<scene_tabs::Tabs>,
        Local<Option<(Option<Room>, Option<Selection>)>>,
    ),
    mut scroll_positions: Query<&mut ScrollPosition>,
    mut labels: Query<(&mut Text, &mut TextColor)>,
    mut panes: Query<&mut Node, Without<Control>>,
    mut buttons: Query<(
        Entity,
        &Control,
        &Hovered,
        Has<Pressed>,
        Has<InteractionDisabled>,
        Has<AssetOpen>,
        &ComputedNode,
        &Children,
        Option<&TabIndex>,
        &mut BackgroundColor,
        &mut BackgroundGradient,
        &mut BorderColor,
        &mut Node,
    )>,
    mut fields: Query<
        (
            Entity,
            &Field,
            &mut EditableText,
            &mut BorderColor,
            &mut TextReadWriteMode,
            &ComputedNode,
            &mut TabIndex,
        ),
        Without<Control>,
    >,
    mut other_inputs: Query<
        (&ComputedNode, &mut TabIndex, &mut TextReadWriteMode),
        (With<TextInput>, Without<Field>, Without<Control>),
    >,
) {
    let model_edit = model_edit
        .as_deref()
        .filter(|_| editor.viewing_model.is_some() && editor.viewing_image.is_none());
    icons::set_glyph(
        &mut commands,
        interface.tree_chevron,
        if editor.sidebar.tree_collapsed() {
            icons::Glyph::ChevronRight
        } else {
            icons::Glyph::ChevronDown
        },
    );
    scene_tabs::refresh(
        &mut commands,
        &editor,
        &numeric,
        &interface,
        &mut tabs,
        &mut labels,
        focus.get().filter(|_| focus_visible.0),
        |entity| {
            buttons
                .get(entity)
                .is_ok_and(|(_, _, hovered, pressed, ..)| hovered.0 || pressed)
        },
    );
    let room =
        (editor.has_scene() && editor.viewing_model.is_none() && editor.viewing_image.is_none())
            .then_some(editor.room);
    let selected_document = (room, editor.selection);
    if *inspector_selection != Some(selected_document) {
        if let Ok(mut scroll) = scroll_positions.get_mut(interface.inspector) {
            scroll.y = 0.0;
        }
        *inspector_selection = Some(selected_document);
    }
    let models: Vec<_> = editor.edits.models.iter().map(|model| model.id).collect();
    let objects: Vec<_> = (0..editor.imported.len())
        .map(Selection::Imported)
        .chain(models.iter().copied().map(Selection::Model))
        .map(|target| (target, editor.label(target), editor.object_deleted(target)))
        .collect();
    let hidden: Vec<_> = (0..editor.imported.len())
        .map(Selection::Imported)
        .chain(models.iter().copied().map(Selection::Model))
        .chain(
            editor
                .edits
                .lights
                .iter()
                .map(|light| Selection::Light(light.id)),
        )
        .chain(viewport::shots(&editor).map(Selection::Camera))
        .map(|target| editor.object_hidden(target))
        .collect();
    let lights: Vec<_> = editor
        .edits
        .lights
        .iter()
        .map(|light| (light.id, light.name.clone(), light.enabled, light.kind))
        .collect();
    let hierarchy = editor
        .edits
        .hierarchy
        .as_ref()
        .map(|h| {
            h.nodes
                .iter()
                .map(|n| {
                    (
                        n.key,
                        n.entity.parent.clone(),
                        editor.label(editor.hierarchy_selection(n.key)),
                        editor.hierarchy_collapsed.contains(&(editor.room, n.key)),
                        h.overridden(n.key),
                        n.entity.model.clone(),
                        n.camera.clone(),
                        n.instance.clone(),
                        editor.object_hidden(editor.hierarchy_selection(n.key)),
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if outline.room != room
        || outline.model != editor.viewing_model
        || outline.imported != editor.imported.len()
        || outline.models != models
        || outline.hidden != hidden
        || outline.objects != objects
        || outline.lights != lights
        || outline.hierarchy != hierarchy
        || outline.hierarchy_selection != editor.selection
        || outline.hierarchy_parenting != editor.hierarchy_parenting
    {
        *outline = OutlineState {
            room,
            model: editor.viewing_model,
            imported: editor.imported.len(),
            models,
            hidden,
            objects,
            lights,
            hierarchy,
            hierarchy_selection: editor.selection,
            hierarchy_parenting: editor.hierarchy_parenting,
        };
        commands
            .entity(interface.outliner)
            .despawn_related::<Children>();
        let heading = |commands: &mut Commands, label| {
            let section = node(
                commands,
                interface.outliner,
                Node {
                    height: px(25),
                    padding: UiRect::horizontal(px(8)),
                    flex_shrink: 0.0,
                    ..row(0.0)
                },
            );
            text(commands, section, &interface.font, label, 11.0, MUTED);
        };
        if editor.viewing_model.is_some() {
            // The model document supplies its own object list after this refresh.
        } else if editor.has_scene() && editor.edits.hierarchy.is_some() {
            scene_hierarchy_ui::tree(&mut commands, interface.outliner, &interface.font, &editor);
        } else if editor.has_scene() {
            if editor.definition().is_native() {
                scene_hierarchy_ui::toolbar(&mut commands, interface.outliner, &interface.font);
                scene_tree_drag::root_row(&mut commands, interface.outliner, &interface.font);
            }
            heading(&mut commands, "ОСВЕЩЕНИЕ");
            outline_entry(
                &mut commands,
                interface.outliner,
                &interface.font,
                "Освещение сцены",
                Selection::Environment,
                None,
                true,
            );
            for light in &editor.edits.lights {
                let target = Selection::Light(light.id);
                object_outline_entry(
                    &mut commands,
                    interface.outliner,
                    &interface.font,
                    &editor,
                    target,
                );
            }
            heading(&mut commands, "КАМЕРЫ");
            for shot in viewport::shots(&editor) {
                let target = Selection::Camera(shot);
                object_outline_entry(
                    &mut commands,
                    interface.outliner,
                    &interface.font,
                    &editor,
                    target,
                );
            }
            heading(&mut commands, "ОБЪЕКТЫ СЦЕНЫ");
            for index in 0..editor.imported.len() {
                if editor.object_deleted(Selection::Imported(index)) {
                    continue;
                }
                object_outline_entry(
                    &mut commands,
                    interface.outliner,
                    &interface.font,
                    &editor,
                    Selection::Imported(index),
                );
            }
            if !editor.edits.models.is_empty() {
                if !editor.definition().is_native() {
                    heading(&mut commands, "ЭКЗЕМПЛЯРЫ КАТАЛОГА");
                }
                for model in &editor.edits.models {
                    object_outline_entry(
                        &mut commands,
                        interface.outliner,
                        &interface.font,
                        &editor,
                        Selection::Model(model.id),
                    );
                }
            }
        } else {
            heading(&mut commands, "НЕТ ОТКРЫТОЙ СЦЕНЫ");
        }
        commands
            .entity(interface.hierarchy_info)
            .despawn_related::<Children>();
        scene_hierarchy_ui::inspector(
            &mut commands,
            interface.hierarchy_info,
            &interface.font,
            &editor,
        );
    }

    let mut set_label = |entity, content: &str, color| {
        if let Ok((mut text, mut tint)) = labels.get_mut(entity) {
            if text.0 != content {
                text.0 = content.to_owned();
            }
            tint.set_if_neq(TextColor(color));
        }
    };
    set_label(
        interface.navigation_hint,
        if editor.viewing_image.is_some() {
            if image_viewer
                .as_ref()
                .is_some_and(|viewer| viewer.edit.editing(editor.viewing_image))
            {
                "Редактирование изображения · Пробел — панорама · Ctrl+S — сохранить"
            } else {
                "Просмотр изображения · Колесо — масштаб · Перетаскивание — панорама"
            }
        } else if editor.viewing_model.is_some() {
            "Модель · ПКМ + WASD — полёт · Alt+ЛКМ — орбита · Tab — режим"
        } else if !editor.has_scene() {
            "Откройте сцену в файловом менеджере"
        } else if editor.pilot {
            "ПКМ + WASD / Q E · Shift — быстрее · Esc — выйти"
        } else if editor.selection.is_some() {
            "Num . — фокус · Alt + ЛКМ — орбита · СКМ — панорама"
        } else {
            "Выберите объект в сцене или дереве · ПКМ + WASD — полёт"
        },
        MUTED,
    );
    set_label(
        interface.field_error,
        numeric.error_message().unwrap_or_default(),
        ERROR,
    );
    set_label(
        interface.dirty,
        if let Some(doc) = editor.narrative.current() {
            if doc.dirty() {
                "● Изменено"
            } else {
                "● Сохранено"
            }
        } else if editor.generation.active
            || !editor.has_scene()
            || editor.viewing_model.is_some()
            || editor.viewing_image.is_some()
        {
            ""
        } else if editor.dirty()
            || numeric.has_changes(&editor)
            || lighting.name_has_changes(&editor)
        {
            "● Изменено"
        } else {
            "● Сохранено"
        },
        if editor.narrative.current().map_or_else(
            || editor.dirty() || numeric.has_changes(&editor) || lighting.name_has_changes(&editor),
            |doc| doc.dirty(),
        ) {
            AMBER
        } else {
            GREEN
        },
    );
    let camera = editor.has_scene() && matches!(editor.selection, Some(Selection::Camera(_)));
    let filling = camera
        && matches!(editor.selection, Some(Selection::Camera(shot)) if editor.definition().cameras[shot].projection == LensKind::Orthographic);
    let perspective = if editor.pilot {
        editor.selection.map_or(String::new(), |target| {
            format!("Камера · {} · 16:9", editor.label(target))
        })
    } else if editor.view_orthographic {
        "Ортографический вид · Свет сцены".to_owned()
    } else {
        "Перспектива · Свет сцены".to_owned()
    };
    set_label(interface.perspective, &perspective, TEXT);
    set_label(
        interface.lens_label,
        if filling {
            "Высота, м"
        } else {
            "Обзор, °"
        },
        MUTED,
    );
    if let Some(model) = editor
        .viewing_model
        .and_then(|index| editor.project.catalog.get(index))
    {
        set_label(interface.selected, &model.label, TEXT);
        set_label(interface.kind, "Просмотр 3D-модели", MUTED);
    } else if !editor.has_scene() {
        set_label(interface.selected, "Сцена не открыта", MUTED);
        set_label(interface.kind, "Откройте файл сцены внизу", MUTED);
    } else if let Some(target) = editor.selection {
        let selected = editor.selected_targets();
        let title = if selected.len() > 1 {
            format!(
                "Выбрано объектов: {} · {}",
                selected.len(),
                editor.label(target)
            )
        } else {
            editor.label(target)
        };
        set_label(interface.selected, &title, TEXT);
        let kind = match target {
            Selection::Node(_) => "Группа или экземпляр вложенной сцены",
            Selection::Camera(_) => "Игровой ракурс",
            Selection::Imported(_) if !editor.field_allowed(target, Field::Position(1)) => {
                "Ограничения трансформации проекта"
            }
            Selection::Imported(_) => "Объект исходной сцены",
            Selection::Model(_) => "Экземпляр общей модели",
            Selection::Light(_) => "Источник света · настройки ниже",
            Selection::Environment => "Окружение, экспозиция и свет сцены",
        };
        let kind = if selected.len() > 1 {
            format!("Поля активного объекта · {}", editor.label(target))
        } else {
            kind.into()
        };
        set_label(interface.kind, &kind, MUTED);
    } else {
        set_label(interface.selected, "Выберите объект или камеру", MUTED);
        set_label(interface.kind, "В 3D-виде или в дереве сцены", MUTED);
    }
    let painting = model_edit.is_some_and(|state| {
        state.mode(editor.viewing_model) == crate::editor::model_document::Mode::Paint
    });
    let top = if painting
        || editor.generation.active
        || editor.viewing_image.is_some()
        || editor.narrative.active.is_some()
    {
        IMAGE_TOP
    } else {
        TOP
    };
    if let Ok(mut toolbar) = panes.get_mut(interface.toolbar) {
        toolbar.height = px(top);
    }
    if let Ok(mut body) = panes.get_mut(interface.body) {
        body.top = px(top);
    }
    for (entity, shown) in [
        (
            interface.tool_bar,
            !painting
                && !editor.generation.active
                && editor.viewing_image.is_none()
                && editor.narrative.active.is_none(),
        ),
        (
            interface.scene_image,
            !editor.generation.active
                && editor.has_scene()
                && editor.viewing_model.is_none()
                && editor.viewing_image.is_none(),
        ),
        (
            interface.scene_badge,
            !editor.generation.active
                && editor.has_scene()
                && editor.viewing_model.is_none()
                && editor.viewing_image.is_none(),
        ),
        (
            interface.empty_scene,
            !editor.generation.active
                && !editor.has_scene()
                && editor.viewing_model.is_none()
                && editor.viewing_image.is_none(),
        ),
        (
            interface.transforms,
            editor.has_scene()
                && editor.viewing_model.is_none()
                && editor
                    .selection
                    .is_some_and(|target| target != Selection::Environment),
        ),
        (interface.lens_row, camera),
        (interface.field_error, numeric.error_message().is_some()),
        (
            interface.scale_row,
            !camera && !matches!(editor.selection, Some(Selection::Light(_))),
        ),
    ] {
        if let Ok(mut pane) = panes.get_mut(entity) {
            let display = if shown { Display::Flex } else { Display::None };
            if pane.display != display {
                pane.display = display;
            }
        }
    }
    for (
        entity,
        control,
        hovered,
        pressed,
        disabled,
        asset_open,
        computed,
        children,
        tab_index,
        mut tint,
        mut background,
        mut border,
        mut node,
    ) in &mut buttons
    {
        let model_action = editor
            .viewing_model
            .filter(|_| editor.viewing_image.is_none())
            .and_then(|_| model_viewer::editing::toolbar_action(control.action));
        if let Action::Tool(tool) = control.action {
            commands.entity(entity).try_insert(AccessibleLabel(
                tool_icons::label(tool, model_action.is_some()).into(),
            ));
        }
        let active = if let Some(action) = model_action {
            model_edit.is_some_and(|state| state.active(action, editor.viewing_model))
        } else {
            match control.action {
                Action::Generation(action) => generation::active(&editor, action),
                Action::Open(room) if matches!(control.look, Look::Asset) => {
                    matches!(browser_ui.selected_entry, Some(browser::Entry::Scene { room: selected, .. }) if selected == room)
                }
                Action::Open(room) | Action::CloseTab(room) => {
                    !editor.generation.active
                        && editor.viewing_model.is_none()
                        && editor.narrative.active.is_none()
                        && editor.viewing_image.is_none()
                        && editor.has_scene()
                        && editor.room == room
                }
                Action::Select(target) => editor.is_selected(target),
                Action::Tool(tool) => editor.tool == tool,
                Action::LocalSpace => editor.local_space,
                Action::Snap => editor.snap,
                Action::Drawer => editor.drawer,
                Action::Browse(index) if asset_open => {
                    browser_ui.selected_entry == Some(browser::Entry::Folder(index))
                        || (!matches!(control.look, Look::Asset)
                            && !matches!(
                                browser_ui.selected_entry,
                                Some(browser::Entry::Folder(_))
                            )
                            && browser.current == index)
                }
                Action::Browse(index) => browser.current == index,
                Action::BrowseFile(index) => browser_ui.selected == Some(index),
                Action::Narrative(NarrativeAction::File(index)) => {
                    browser_ui.selected == Some(index)
                }
                Action::ViewImage(index) => browser_ui.selected == Some(index),
                Action::OpenImageTab(id) | Action::CloseImageTab(id) => {
                    editor.viewing_image == Some(id)
                }
                Action::ImageEdit(action) => image_viewer
                    .as_ref()
                    .is_some_and(|viewer| viewer.edit.active(action, editor.viewing_image)),
                Action::ImageView(ImageAction::Fit) => {
                    image_viewer.as_ref().is_some_and(|viewer| viewer.is_fit())
                }
                Action::ImageView(ImageAction::ActualSize) => image_viewer
                    .as_ref()
                    .is_some_and(|viewer| viewer.is_actual_size()),
                Action::ViewModel(index) if matches!(control.look, Look::Tab) => {
                    editor.viewing_model == Some(index)
                }
                Action::ViewModel(index) => browser_ui.selected_model == Some(index),
                Action::Pilot => editor.pilot,
                _ => false,
            }
        };
        let visible = match control.action {
            Action::BrowseClear => !browser.search.is_empty(),
            Action::Pilot | Action::Align => camera,
            Action::Play => play_ui::visible(&editor),
            Action::Delete => editor.deletable_targets(),
            Action::Duplicate => editor.batch_editable(),
            _ => true,
        };
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        let enabled = available(&editor, &browser, control.action)
            && model_action.is_none_or(|action| {
                model_edit.is_some_and(|state| state.available(action, editor.viewing_model))
            })
            && match control.action {
                Action::ImageEdit(action) => image_viewer
                    .as_ref()
                    .is_some_and(|viewer| viewer.edit.available(action, editor.viewing_image)),
                _ => true,
            };
        if enabled == disabled {
            if enabled {
                commands.entity(entity).try_remove::<InteractionDisabled>();
            } else {
                commands
                    .entity(entity)
                    .try_insert(InteractionDisabled)
                    .try_remove::<Pressed>();
            }
        }
        let tab = if enabled && visible && computed.size().min_element() > 0.0 {
            0
        } else {
            -1
        };
        if tab_index.is_none_or(|index| index.0 != tab) {
            // Tabs and the outliner can queue a rebuild earlier in this refresh.
            commands.entity(entity).try_insert(TabIndex(tab));
        }
        let photo_tool =
            matches!(control.look, Look::Tool) && matches!(control.action, Action::ImageEdit(_));
        let layer_row_control = matches!(
            control.action,
            Action::ImageEdit(ImageEditAction::SelectLayer(_))
                | Action::ImageEdit(ImageEditAction::Layers(
                    image_viewer::edit::layers::LayerAction::ToggleLock(_)
                ))
        );
        // Menus block activation without making the selected file/object look deselected.
        let preserve_selection = (asset_open && browser_context.is_menu())
            || (editor.scene_object_popup && active && matches!(control.look, Look::Outline));
        let visual_enabled = enabled || preserve_selection;
        let photo_options = photo_tool
            && matches!(
                control.action,
                Action::ImageEdit(ImageEditAction::PaintMenu | ImageEditAction::EraserMenu)
                    | Action::ImageEdit(ImageEditAction::Selection(
                        image_viewer::edit::selection::SelectionAction::Menu
                    ))
            );
        border.set_if_neq(BorderColor::all(if preserve_selection && active {
            FOCUS
        } else if enabled
            && focus.get() == Some(entity)
            && !matches!(control.look, Look::Tab | Look::TabClose)
        {
            FOCUS
        } else if enabled && active && photo_tool && !photo_options {
            FOCUS
        } else {
            Color::NONE
        }));
        for child in children.iter() {
            if let Ok((_, mut color)) = labels.get_mut(child) {
                color.set_if_neq(TextColor(if !visual_enabled {
                    DISABLED_TEXT
                } else if matches!(control.look, Look::Primary) {
                    ON_ACCENT
                } else if matches!(control.look, Look::Tab) {
                    if active || hovered.0 {
                        scene_tabs::LABEL
                    } else {
                        scene_tabs::INACTIVE_LABEL
                    }
                } else {
                    TEXT
                }));
            }
        }
        let color = if !visual_enabled || photo_options {
            Color::NONE
        } else if matches!(control.action, Action::EditBrowserImage(..)) {
            if hovered.0 || pressed {
                HOVER
            } else {
                PANEL
            }
        } else if layer_row_control {
            if hovered.0 || pressed {
                HOVER
            } else {
                Color::NONE
            }
        } else if matches!(control.look, Look::Tab) {
            Color::NONE
        } else if matches!(control.look, Look::TabClose) {
            if hovered.0 || pressed {
                scene_tabs::CLOSE_HOVER
            } else {
                Color::NONE
            }
        } else if matches!(control.look, Look::Tool) {
            if active {
                if photo_tool { ACTIVE } else { ACCENT }
            } else if hovered.0 || pressed {
                tool_icons::HOVER
            } else {
                tool_icons::IDLE
            }
        } else if matches!(control.look, Look::Primary) {
            if hovered.0 { FOCUS } else { ACCENT }
        } else if active {
            ACTIVE
        } else if hovered.0 || pressed {
            HOVER
        } else if matches!(control.look, Look::Asset) {
            CHROME
        } else {
            Color::NONE
        };
        tint.set_if_neq(BackgroundColor(color));
        let fill = if photo_tool || layer_row_control {
            BackgroundGradient::default()
        } else if enabled
            && (matches!(control.look, Look::Primary)
                || (active && matches!(control.look, Look::Tool)))
        {
            primary_gradient(hovered.0 || pressed)
        } else if visual_enabled
            && active
            && !hovered.0
            && !pressed
            && !matches!(control.look, Look::Tab | Look::TabClose)
        {
            active_gradient()
        } else {
            BackgroundGradient::default()
        };
        background.set_if_neq(fill);
    }
    for (entity, &field, mut text, mut border, mut mode, computed, mut tab_index) in &mut fields {
        let allowed = editor.ready
            && !editor.asset_viewer_open()
            && editor.pending.is_none()
            && editor
                .selection
                .is_some_and(|target| editor.field_allowed(target, field));
        let tab = if allowed && computed.size().min_element() > 0.0 {
            0
        } else {
            -1
        };
        if tab_index.0 != tab {
            tab_index.0 = tab;
        }
        let focused = focus.get() == Some(entity) && allowed;
        border.set_if_neq(BorderColor::all(if numeric.invalid(entity) {
            ERROR
        } else if focused {
            FOCUS
        } else if allowed {
            EDGE
        } else {
            WELL
        }));
        mode.set_if_neq(if allowed {
            TextReadWriteMode::Editable
        } else {
            TextReadWriteMode::ReadOnly
        });
        if !focused && !numeric.preserve_text(entity) {
            let value = editor
                .selection
                .and_then(|target| editor.field_value(target, field));
            let display = value.map_or_else(|| "—".into(), |value| format!("{value:.2}"));
            if text.value().to_string() != display {
                text.editor_mut().set_text(&display);
            }
        }
    }
    for (computed, mut tab_index, mut mode) in &mut other_inputs {
        let allowed = editor.pending.is_none() && computed.size().min_element() > 0.0;
        let tab = if allowed { 0 } else { -1 };
        if tab_index.0 != tab {
            tab_index.0 = tab;
        }
        mode.set_if_neq(if allowed {
            TextReadWriteMode::Editable
        } else {
            TextReadWriteMode::ReadOnly
        });
    }
}
