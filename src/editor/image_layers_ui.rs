//! Native Bevy tree, command browser and property sheet for image layers.
use super::ux;
use super::*;
#[path = "image_layer_drag.rs"]
pub(in crate::editor) mod drag_ui;
#[path = "image_layer_form_ui.rs"]
mod form_ui;
#[path = "image_layer_properties_ui.rs"]
mod properties_ui;
use crate::editor::image_texture::TextureUploads;
use bevy::picking::events::PointerClick;
use bevy::ui_widgets::{
    Activate, Button, Slider, SliderRange, SliderValue, TrackClick, ValueChange,
};
pub(in crate::editor) use form_ui::sync as sync_form;

#[derive(Resource)]
pub(in crate::editor) struct LayerUi {
    host: Entity,
    stack: Entity,
    basic: Entity,
    details: Entity,
    lock_controls: Entity,
    search_body: Entity,
    search_input: Entity,
    name: Entity,
    clear_filter: Entity,
    settings: Disclosure,
    locks: Disclosure,
    shown_sections: (bool, bool, bool),
    heading: Entity,
    popup: Entity,
    signature: String,
    thumbnail_revision: Option<(photocraft_doc::DocId, u64, photocraft_geom::Size)>,
    popup_signature: String,
    thumbnails: HashMap<(u64, bool, bool), (Option<photocraft_doc::Layer>, Handle<Image>)>,
    results: Option<Entity>,
    popup_scroll: Option<Entity>,
    popup_context: String,
    search_signature: String,
    last_click: Option<(u64, std::time::Instant)>,
}

struct Disclosure {
    button: Entity,
    body: Entity,
    chevron: Entity,
    label: Entity,
}

fn disclosure(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    title: &str,
    section: PanelSection,
) -> Disclosure {
    let button = control(
        commands,
        parent,
        act(LayerAction::PanelSection(section)),
        Look::Regular,
        Node {
            width: percent(100),
            height: px(27),
            padding: UiRect::horizontal(px(4)),
            ..row(5.0)
        },
    );
    commands
        .entity(button)
        .insert(AccessibleLabel(title.into()));
    let chevron = icons::themed(commands, button, icons::Glyph::ChevronRight, 12.0);
    let label = text(commands, button, font, title, 12.0, TEXT);
    let body = node(
        commands,
        parent,
        Node {
            display: Display::None,
            width: percent(100),
            padding: UiRect::new(px(8), px(0), px(2), px(6)),
            ..column(5.0)
        },
    );
    Disclosure {
        button,
        body,
        chevron,
        label,
    }
}
#[derive(Component)]
struct LayerRow {
    id: u64,
    index: usize,
}
#[derive(Component)]
pub(in crate::editor) struct FormInput(usize);
#[derive(Component)]
pub(in crate::editor) struct LayerSearch;
#[derive(Component)]
pub(in crate::editor) struct CommandSearch;
#[derive(Component)]
pub(in crate::editor) struct LayerScroll;
#[derive(Component)]
struct FillReadout(Entity);
#[derive(Component)]
pub(in crate::editor) struct LayerThumbnail(u64, bool, bool);

// Painting changes document pixels, not the tree's widgets. Only metadata displayed
// by the panel belongs here; thumbnail textures have their own revision below.
fn panel_signature(doc: &PhotoDocument) -> String {
    use std::fmt::Write;
    let mut signature = format!(
        "{:?}:{:?}:{}:{}",
        doc.document.id,
        doc.size(),
        doc.document.name,
        doc.editing_smart()
    );
    for (_, depth, layer) in doc.document.walk() {
        write!(
            signature,
            "{:?}{:?}",
            (
                depth,
                layer.id,
                &layer.name,
                layer.visible,
                layer.blend,
                layer.fill_opacity,
                layer.label,
                layer.link_group,
                layer.clipped
            ),
            (
                &layer.locks,
                collapsed(layer),
                layer_kind(layer),
                layer.mask.is_some(),
                layer.vector_mask.is_some(),
                !layer.effects.items.is_empty()
            )
        )
        .unwrap();
        if let LayerContent::Smart(smart) = &layer.content {
            write!(signature, "{:?}", smart.filter_mask.is_some()).unwrap();
            for filter in &smart.smart_filters {
                write!(signature, "{:?}", (&filter.command, filter.visible)).unwrap();
            }
        }
    }
    signature
}

fn scroll_node(commands: &mut Commands, parent: Entity, layout: Node) -> Entity {
    let entity = node(commands, parent, layout);
    commands
        .entity(entity)
        .insert((Scroller, ScrollPosition::default(), LayerScroll));
    entity
}

fn act(action: LayerAction) -> Action {
    Action::ImageEdit(EditAction::Layers(action))
}
fn compact(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    action: LayerAction,
) -> Entity {
    let button = button(commands, parent, font, label, act(action), Look::Regular);
    commands.entity(button).entry::<Node>().and_modify(|mut n| {
        n.height = px(25);
        n.min_width = px(0);
        n.padding = UiRect::axes(px(6), px(3));
    });
    ux::help(commands, button, ux::action_help(action));
    button
}

pub(in crate::editor) fn setup(
    commands: &mut Commands,
    container: Entity,
    root: Entity,
    font: &Handle<Font>,
    heading: Entity,
    name: Entity,
    opacity: Entity,
) {
    let tabs = node(
        commands,
        container,
        Node {
            width: percent(100),
            ..row(4.0)
        },
    );
    for (label, section) in [
        ("Слои", PanelSection::Layers),
        ("Свойства", PanelSection::Settings),
    ] {
        let tab = compact(
            commands,
            tabs,
            font,
            label,
            LayerAction::PanelSection(section),
        );
        commands.entity(tab).entry::<Node>().and_modify(|mut n| {
            n.flex_grow = 1.0;
            n.justify_content = JustifyContent::Center;
            n.height = px(31);
        });
    }
    let search_body = node(
        commands,
        container,
        Node {
            display: Display::None,
            width: percent(100),
            ..column(4.0)
        },
    );
    text(commands, search_body, font, "Поиск по имени", 10.0, MUTED);
    let filter = super::super::interface::input(commands, search_body, font, "Найти слой");
    commands.entity(filter).insert(LayerSearch);
    let search_actions = node(commands, search_body, row(4.0));
    compact(
        commands,
        search_actions,
        font,
        "Фильтры…",
        LayerAction::Category("Вид панели"),
    );
    let clear_filter = compact(
        commands,
        search_actions,
        font,
        "Сбросить",
        LayerAction::ResetFilter,
    );
    text(
        commands,
        search_body,
        font,
        "Ctrl — несколько · Shift — диапазон",
        10.0,
        MUTED,
    );
    let basic = node(
        commands,
        container,
        Node {
            width: percent(100),
            ..column(4.0)
        },
    );
    commands.entity(opacity).insert(ChildOf(container));
    let stack = node(
        commands,
        container,
        Node {
            width: percent(100),
            flex_grow: 1.0,
            min_height: px(0),
            ..column(5.0)
        },
    );
    let host = scroll_node(
        commands,
        stack,
        Node {
            width: percent(100),
            flex_grow: 1.0,
            min_height: px(80),
            overflow: Overflow::scroll_y(),
            ..column(2.0)
        },
    );
    commands.entity(host).insert(drag_ui::LayerList);
    drag_ui::setup(commands, root, font);
    let footer = node(
        commands,
        stack,
        Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            padding: UiRect::vertical(px(5)),
            border: UiRect::top(px(1)),
            ..row(2.0)
        },
    );
    commands.entity(footer).insert(BorderColor::all(EDGE));
    let add = icons::button(
        commands,
        footer,
        icons::Glyph::Plus,
        "Новый слой",
        Action::ImageEdit(EditAction::AddLayer),
        Look::Regular,
    );
    commands.entity(add).entry::<Node>().and_modify(|mut n| {
        n.height = px(30);
        n.padding = UiRect::ZERO;
    });
    ux::help(
        commands,
        add,
        "Создать прозрачный слой над выбранным. Рисуйте на новом слое, чтобы не менять исходное изображение.",
    );
    for (glyph, label, action) in [
        (icons::Glyph::Folder, "Сгруппировать", LayerAction::Group),
        (
            icons::Glyph::Mask,
            "Добавить маску",
            LayerAction::Quick("layer.layerMask.revealAll"),
        ),
        (
            icons::Glyph::Trash,
            "Удалить слой",
            LayerAction::Quick("layer.delete"),
        ),
    ] {
        let e = icons::button(commands, footer, glyph, label, act(action), Look::Regular);
        commands.entity(e).entry::<Node>().and_modify(|mut n| {
            n.width = px(30);
            n.min_width = px(24);
            n.flex_shrink = 1.0;
            n.height = px(30);
            n.padding = UiRect::ZERO;
        });
        ux::help(commands, e, ux::action_help(action));
    }
    let settings = disclosure(
        commands,
        container,
        font,
        "Параметры слоя",
        PanelSection::Settings,
    );
    commands
        .entity(settings.button)
        .remove::<(Control, Button, TabIndex)>()
        .entry::<Node>()
        .and_modify(|mut n| n.display = Display::None);
    commands
        .entity(settings.body)
        .entry::<Node>()
        .and_modify(|mut n| {
            n.padding = UiRect::ZERO;
            n.flex_grow = 1.0;
            n.min_height = px(0);
            n.overflow = Overflow::scroll_y();
        });
    commands
        .entity(settings.body)
        .insert((Scroller, LayerScroll));
    text(commands, settings.body, font, "Название", 11.0, MUTED);
    commands.entity(name).insert(ChildOf(settings.body));
    let details = node(
        commands,
        settings.body,
        Node {
            width: percent(100),
            ..column(4.0)
        },
    );
    let locks = disclosure(
        commands,
        settings.body,
        font,
        "Защита от случайных изменений",
        PanelSection::Locks,
    );
    let lock_controls = node(
        commands,
        locks.body,
        Node {
            width: percent(100),
            ..column(4.0)
        },
    );
    let popup = node(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
    );
    commands.entity(popup).insert((
        GlobalZIndex(500),
        TabGroup::modal(),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.12)),
    ));
    commands.insert_resource(LayerUi {
        host,
        stack,
        basic,
        details,
        lock_controls,
        search_body,
        search_input: filter,
        name,
        clear_filter,
        settings,
        locks,
        shown_sections: (false, false, false),
        heading,
        popup,
        signature: String::new(),
        thumbnail_revision: None,
        popup_signature: String::new(),
        thumbnails: HashMap::new(),
        last_click: None,
        results: None,
        popup_scroll: None,
        popup_context: String::new(),
        search_signature: String::new(),
    });
}

fn select(
    event: On<PointerClick>,
    rows: Query<&LayerRow>,
    keys: Res<ButtonInput<KeyCode>>,
    mut actions: MessageWriter<Action>,
    viewer: Res<ImageViewer>,
    mut ui: ResMut<LayerUi>,
    drag: Res<drag_ui::LayerDrag>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    if drag.suppresses(window.cursor_position()) {
        ui.last_click = None;
        return;
    }
    let Ok(row) = rows.get(event.entity) else {
        return;
    };
    if event.button == PointerButton::Secondary {
        if !viewer
            .edit
            .document(viewer.tab)
            .is_some_and(|doc| doc.selected.contains(&LayerId(row.id)))
        {
            actions.write(act(LayerAction::Select(row.index, false, false)));
        }
        actions.write(act(LayerAction::Menu));
    } else if event.button == PointerButton::Primary
        && !keys.any_pressed([
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::ShiftLeft,
            KeyCode::ShiftRight,
        ])
    {
        if ui
            .last_click
            .is_some_and(|(id, time)| id == row.id && time.elapsed().as_millis() < 400)
        {
            ui.last_click = None;
            actions.write(act(LayerAction::Command("layer.renameLayer")));
        } else {
            ui.last_click = Some((row.id, std::time::Instant::now()));
        }
    }
    let _ = keys;
}
fn activate_row(
    event: On<Activate>,
    rows: Query<&LayerRow>,
    keys: Res<ButtonInput<KeyCode>>,
    mut actions: MessageWriter<Action>,
    drag: Res<drag_ui::LayerDrag>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    if drag.suppresses(window.cursor_position()) {
        return;
    }
    if let Ok(row) = rows.get(event.entity) {
        actions.write(act(LayerAction::Select(
            row.index,
            keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]),
            keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        )));
    }
}
fn fill(
    event: On<ValueChange<f32>>,
    mut actions: MessageWriter<Action>,
    mut commands: Commands,
    children: Query<&Children>,
    mut thumbs: Query<&mut Node, With<bevy::ui_widgets::SliderThumb>>,
    readouts: Query<&FillReadout>,
    mut labels: Query<&mut Text>,
) {
    commands
        .entity(event.source)
        .insert(SliderValue(event.value));
    if let Ok(children) = children.get(event.source) {
        for child in children {
            if let Ok(mut thumb) = thumbs.get_mut(*child) {
                thumb.left = percent(event.value * 94.0);
            }
        }
    }
    if let Ok(readout) = readouts.get(event.source)
        && let Ok(mut text) = labels.get_mut(readout.0)
    {
        **text = format!("{:.0}%", event.value * 100.0);
    }
    actions.write(act(LayerAction::Fill(event.value, event.is_final)));
}

pub(in crate::editor) fn input(
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    mut viewer: ResMut<ImageViewer>,
    inputs: Query<(
        Entity,
        &EditableText,
        Option<&FormInput>,
        Has<LayerSearch>,
        Has<CommandSearch>,
    )>,
    mut focus: ResMut<InputFocus>,
    mut actions: MessageWriter<Action>,
    mut colour_was_open: Local<bool>,
) {
    // The colour picker closes earlier in the schedule. Consume the same Escape
    // here too, so one key press cannot also dismiss its parent property sheet.
    let colour_open = viewer.edit.color_picker.is_some();
    let colour_handled = colour_open || *colour_was_open;
    *colour_was_open = colour_open;
    if !viewer.edit.editing(editor.viewing_image) || colour_handled {
        return;
    }
    for (entity, input, field, filter, search) in &inputs {
        if focus.get() != Some(entity) {
            continue;
        }
        let value = input.value().to_string();
        if let Some(field) = field
            && let Some(form) = &mut viewer.edit.layers.form
            && let Some(field) = form.fields.get_mut(field.0)
        {
            if field.text != value {
                field.text = value.clone();
                field.enabled = true;
            }
        }
        if filter {
            viewer.edit.layers.filter = value.clone();
        }
        if search {
            viewer.edit.layers.search = value;
        }
    }
    if keys.just_pressed(KeyCode::Escape)
        && (viewer.edit.layers.menu
            || viewer.edit.layers.form.is_some()
            || viewer.edit.layers.blend_menu)
    {
        actions.write(act(LayerAction::Cancel));
        focus.clear();
    }
    if keys.just_pressed(KeyCode::Enter)
        && (keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight])
            || viewer.edit.layers.form.as_ref().is_some_and(|f| {
                matches!(
                    f.command,
                    "layer.renameLayer" | "layer.new.layer" | "layer.new.group"
                )
            }))
        && viewer.edit.layers.form.is_some()
    {
        actions.write(act(LayerAction::Apply));
        focus.clear();
    }
}

pub(in crate::editor) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    viewer: Res<ImageViewer>,
    interface: Res<Interface>,
    mut ui: ResMut<LayerUi>,
    mut images: ResMut<Assets<Image>>,
    scroll_positions: Query<&ScrollPosition>,
    mut filters: Query<(Entity, &mut EditableText), With<LayerSearch>>,
    mut focus: ResMut<InputFocus>,
    mut titles: Query<&mut Text>,
    mut uploads: ResMut<TextureUploads>,
    thumbnail_nodes: Query<&LayerThumbnail>,
    mut nodes: Query<&mut Node>,
) {
    let state = &viewer.edit.layers;
    for (entity, mut input) in &mut filters {
        if focus.get() != Some(entity) && input.value().to_string() != state.filter {
            input.editor_mut().set_text(&state.filter);
        }
    }
    let doc = viewer.edit.document(editor.viewing_image);
    if let Ok(mut title) = titles.get_mut(ui.heading) {
        title.set_if_neq(Text(if state.has_filter() {
            format!(
                "Слои · {}/{}",
                doc.map_or(0, |d| tree_rows(d, state).len()),
                doc.map_or(0, |d| d.document.layer_count())
            )
        } else {
            format!("Слои · {}", doc.map_or(0, |d| d.document.layer_count()))
        }));
    }
    let editing = viewer.edit.editing(editor.viewing_image);
    let selected = doc.and_then(PhotoDocument::active_layer);
    for (entity, show) in [
        (
            ui.search_body,
            editing && state.search_open && !state.settings_open,
        ),
        (ui.stack, editing && !state.settings_open),
        (
            ui.settings.body,
            editing && selected.is_some() && state.settings_open,
        ),
        (
            ui.locks.body,
            editing && selected.is_some() && state.locks_open,
        ),
        (ui.clear_filter, state.has_filter()),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            let display = if show { Display::Flex } else { Display::None };
            if node.display != display {
                node.display = display;
            }
        }
    }
    if (!state.search_open && focus.get() == Some(ui.search_input))
        || (!state.settings_open && focus.get() == Some(ui.name))
    {
        focus.clear();
    }
    let sections = (state.search_open, state.settings_open, state.locks_open);
    if sections != ui.shown_sections {
        for (entity, open) in [
            (ui.settings.chevron, state.settings_open),
            (ui.locks.chevron, state.locks_open),
        ] {
            icons::set_glyph(
                &mut commands,
                entity,
                if open {
                    icons::Glyph::ChevronDown
                } else {
                    icons::Glyph::ChevronRight
                },
            );
        }
        ui.shown_sections = sections;
    }
    let lock_count = selected.map_or(0, |layer| {
        [
            layer.locks.all,
            layer.locks.transparency,
            layer.locks.pixels,
            layer.locks.position,
        ]
        .into_iter()
        .filter(|value| *value)
        .count()
    });
    if let Ok(mut title) = titles.get_mut(ui.locks.label) {
        title.set_if_neq(Text(if lock_count == 0 {
            "Защита слоя".into()
        } else {
            format!("Защита слоя · {lock_count}")
        }));
    }
    if let Ok(mut title) = titles.get_mut(ui.settings.label) {
        title.set_if_neq(Text(
            selected
                .filter(|layer| layer.fill_opacity != 1.0)
                .map_or_else(
                    || "Параметры слоя".into(),
                    |layer| format!("Параметры · заливка {:.0}%", layer.fill_opacity * 100.0),
                ),
        ));
    }
    let thumbnail_revision = doc
        .filter(|_| editing)
        .map(|doc| (doc.document.id, doc.revision, doc.size()));
    if ui.thumbnail_revision.map(|(id, _, size)| (id, size))
        != thumbnail_revision.map(|(id, _, size)| (id, size))
    {
        ui.thumbnails.clear();
    }
    let signature = format!(
        "{:?}:{}:{:?}:{}:{:?}:{}:{}",
        editor.viewing_image,
        doc.map(panel_signature).unwrap_or_default(),
        doc.map(|d| (
            &d.selected,
            d.active,
            d.mask_target,
            d.filter_target,
            d.view_revision
        )),
        state.filter,
        state.ui_revision,
        state.blend_menu,
        editing
    );
    let rebuild = signature != ui.signature && !state.fill_drag;
    if rebuild {
        for host in [ui.host, ui.basic, ui.details, ui.lock_controls] {
            commands.entity(host).despawn_related::<Children>();
        }
        if editing && let Some(doc) = doc {
            panel(
                &mut commands,
                ui.host,
                ui.basic,
                ui.details,
                ui.lock_controls,
                &interface.font,
                doc,
                state,
                &mut ui.thumbnails,
                &mut images,
                &mut uploads,
            );
        }
        ui.signature = signature;
    }
    if !rebuild
        && thumbnail_revision != ui.thumbnail_revision
        && editing
        && let Some(doc) = doc
    {
        for &LayerThumbnail(id, mask, content) in &thumbnail_nodes {
            if let Some(layer) = doc.document.layer(LayerId(id)) {
                layer_thumbnail(
                    doc,
                    layer,
                    mask,
                    content,
                    &mut ui.thumbnails,
                    &mut images,
                    &mut uploads,
                );
            }
        }
    }
    ui.thumbnail_revision = thumbnail_revision;
    let popup_signature = format!(
        "{}:{}:{}:{:?}",
        editing, state.ui_revision, state.menu, viewer.edit.error
    );
    if popup_signature != ui.popup_signature {
        let context = format!(
            "{:?}:{:?}:{}",
            editor.viewing_image,
            state
                .form
                .as_ref()
                .map(|f| (f.command, f.params.get("layer"))),
            state.category
        );
        let previous = if ui.popup_context == context {
            ui.popup_scroll
                .and_then(|e| scroll_positions.get(e).ok())
                .map(|p| ScrollPosition(p.0))
        } else {
            None
        };
        ui.popup_context = context;
        commands.entity(ui.popup).despawn_related::<Children>();
        let show = editing && (state.menu || state.form.is_some());
        commands
            .entity(ui.popup)
            .entry::<Node>()
            .and_modify(move |mut n| n.display = if show { Display::Flex } else { Display::None });
        if show {
            let (results, scroll) = popup(
                &mut commands,
                ui.popup,
                &interface.font,
                state,
                viewer.edit.error.as_deref(),
                &mut focus,
            );
            ui.results = results;
            ui.popup_scroll = scroll;
            if let (Some(scroll), Some(previous)) = (scroll, previous) {
                commands.entity(scroll).insert(previous);
            }
        } else {
            ui.results = None;
            ui.popup_scroll = None;
        }
        ui.popup_signature = popup_signature;
    }
    let search_signature = format!("{}:{}", ui.popup_signature, state.search);
    if ui.search_signature != search_signature {
        if let Some(results) = ui.results {
            commands.entity(results).despawn_related::<Children>();
            command_results(&mut commands, results, &interface.font, state);
        }
        ui.search_signature = search_signature;
    }
}

fn panel(
    commands: &mut Commands,
    host: Entity,
    basic: Entity,
    details: Entity,
    lock_controls: Entity,
    font: &Handle<Font>,
    doc: &PhotoDocument,
    state: &LayerState,
    thumbs: &mut HashMap<(u64, bool, bool), (Option<photocraft_doc::Layer>, Handle<Image>)>,
    images: &mut Assets<Image>,
    uploads: &mut TextureUploads,
) {
    let tree_host = host;
    let host = basic;
    if doc.editing_smart() {
        text(
            commands,
            host,
            font,
            &format!("Содержимое: {}", doc.document.name),
            12.0,
            TEXT,
        );
        compact(
            commands,
            host,
            font,
            "← Готово · вернуться к изображению",
            LayerAction::Command("layer.smartObjects.saveContents"),
        );
    }
    if let Some(layer) = doc.active_layer() {
        text(
            commands,
            host,
            font,
            "Смешивание с нижними слоями",
            10.0,
            MUTED,
        );
        let properties = node(
            commands,
            host,
            Node {
                width: percent(100),
                flex_wrap: FlexWrap::Wrap,
                ..row(3.0)
            },
        );
        dropdown(
            commands,
            properties,
            font,
            ux::blend(layer.blend),
            LayerAction::Blend,
        );
        if doc.mask_target {
            compact(
                commands,
                properties,
                font,
                "Маска",
                LayerAction::MaskPreview(false),
            );
            compact(
                commands,
                properties,
                font,
                "Наложение",
                LayerAction::MaskPreview(true),
            );
        }
        let locks = node(
            commands,
            lock_controls,
            Node {
                width: percent(100),
                flex_wrap: FlexWrap::Wrap,
                ..row(3.0)
            },
        );
        for (key, title, on) in [
            ("all", "Всё", layer.locks.all),
            ("transparency", "Прозрачность", layer.locks.transparency),
            ("pixels", "Пиксели", layer.locks.pixels),
            ("position", "Положение", layer.locks.position),
        ] {
            compact(
                commands,
                locks,
                font,
                &format!("{} {title}", if on { "●" } else { "○" }),
                LayerAction::Lock(key),
            );
        }
        let fill_header = node(
            commands,
            details,
            Node {
                width: percent(100),
                justify_content: JustifyContent::SpaceBetween,
                ..row(3.0)
            },
        );
        text(commands, fill_header, font, "Заливка", 11.0, MUTED);
        let fill_readout = text(
            commands,
            fill_header,
            font,
            &format!("{:.0}%", layer.fill_opacity * 100.0),
            11.0,
            TEXT,
        );
        let slider = commands
            .spawn((
                Slider {
                    track_click: TrackClick::Snap,
                    ..default()
                },
                SliderRange::new(0.0, 1.0),
                SliderValue(layer.fill_opacity),
                Node {
                    width: percent(100),
                    height: px(18),
                    flex_shrink: 0.0,
                    ..default()
                },
                BackgroundColor(Color::NONE),
                ChildOf(details),
            ))
            .observe(fill)
            .id();
        commands.entity(slider).insert(FillReadout(fill_readout));
        ux::help(commands, slider, ux::field_help("fillOpacity"));
        let track = surface(
            commands,
            slider,
            Node {
                position_type: PositionType::Absolute,
                left: px(5),
                right: px(5),
                top: px(7),
                height: px(4),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            EDGE,
        );
        commands.entity(track).insert(Pickable::IGNORE);
        let thumb = surface(
            commands,
            slider,
            Node {
                position_type: PositionType::Absolute,
                left: percent(layer.fill_opacity * 94.0),
                width: px(14),
                height: px(14),
                top: px(2),
                border_radius: BorderRadius::MAX,
                border: UiRect::all(px(2)),
                ..default()
            },
            WELL,
        );
        commands.entity(thumb).insert((
            bevy::ui_widgets::SliderThumb,
            Pickable::IGNORE,
            BorderColor::all(FOCUS),
        ));
        properties_ui::build(commands, details, font, doc, state);
        if state.blend_menu {
            let choices = scroll_node(
                commands,
                host,
                Node {
                    width: percent(100),
                    max_height: px(180),
                    overflow: Overflow::scroll_y(),
                    ..column(2.0)
                },
            );
            if layer.is_group() {
                compact(
                    commands,
                    choices,
                    font,
                    ux::blend(photocraft_color::BlendMode::PassThrough),
                    LayerAction::SetBlend(photocraft_color::BlendMode::PassThrough),
                );
            }
            for mode in photocraft_color::BlendMode::layer_modes().filter(|m| {
                state.blend_more
                    || matches!(
                        m,
                        photocraft_color::BlendMode::Normal
                            | photocraft_color::BlendMode::Multiply
                            | photocraft_color::BlendMode::Screen
                            | photocraft_color::BlendMode::Overlay
                            | photocraft_color::BlendMode::SoftLight
                            | photocraft_color::BlendMode::Color
                    )
            }) {
                compact(
                    commands,
                    choices,
                    font,
                    ux::blend(mode),
                    LayerAction::SetBlend(mode),
                );
            }
            compact(
                commands,
                choices,
                font,
                if state.blend_more {
                    "Только основные режимы"
                } else {
                    "Все режимы смешивания…"
                },
                LayerAction::BlendMore,
            );
        }
    }
    let host = tree_host;
    let rows = tree_rows(doc, state);
    let no_matches = rows.is_empty();
    for (index, (_path, depth, layer)) in rows {
        // Filter controls belong to their layer's moving block as well.
        let host = node(
            commands,
            host,
            Node {
                width: percent(100),
                flex_shrink: 0.0,
                ..column(2.0)
            },
        );
        commands.entity(host).insert((
            drag_ui::DropRow {
                id: layer.id.0,
                depth,
                group: layer.is_group(),
            },
            drag_ui::Slide::default(),
            UiTransform::default(),
            Visibility::Inherited,
        ));
        let selected = doc.selected.contains(&layer.id);
        let row_entity = surface(
            commands,
            host,
            Node {
                width: percent(100),
                height: px((state.thumbnail.pixels() + 10.0).max(32.0)),
                min_height: px((state.thumbnail.pixels() + 10.0).max(32.0)),
                padding: UiRect::left(px(depth.min(6) as f32 * 10.0)),
                border: UiRect::all(px(if doc.active == index { 1 } else { 0 })),
                border_radius: BorderRadius::all(px(5)),
                ..row(3.0)
            },
            if selected { ACTIVE } else { WELL },
        );
        commands.entity(row_entity).insert(BorderColor::all(FOCUS));
        if layer.label != photocraft_doc::LabelColor::None {
            let stripe = surface(
                commands,
                row_entity,
                Node {
                    width: px(3),
                    height: percent(78),
                    flex_shrink: 0.0,
                    border_radius: BorderRadius::all(px(2)),
                    ..default()
                },
                label_color(layer.label),
            );
            commands.entity(stripe).insert(Pickable::IGNORE);
        }
        let eye = icons::button(
            commands,
            row_entity,
            if layer.visible {
                icons::Glyph::Eye
            } else {
                icons::Glyph::EyeOff
            },
            "Показать / скрыть слой",
            Action::ImageEdit(EditAction::Visible(index)),
            Look::Regular,
        );
        commands.entity(eye).entry::<Node>().and_modify(|mut n| {
            n.width = px(24);
            n.height = px(30);
        });
        if layer.is_group() {
            let disclosure = icons::button(
                commands,
                row_entity,
                if collapsed(layer) {
                    icons::Glyph::ChevronRight
                } else {
                    icons::Glyph::ChevronDown
                },
                "Развернуть / свернуть группу",
                act(LayerAction::Collapse(layer.id.0)),
                Look::Regular,
            );
            commands
                .entity(disclosure)
                .entry::<Node>()
                .and_modify(|mut n| {
                    n.width = px(16);
                    n.height = px(26);
                    n.padding = UiRect::ZERO;
                });
        }
        let select = node(
            commands,
            row_entity,
            Node {
                flex_grow: 1.0,
                min_width: px(0),
                height: percent(100),
                ..row(5.0)
            },
        );
        commands
            .entity(select)
            .insert((
                LayerRow {
                    id: layer.id.0,
                    index,
                },
                Hovered::default(),
                Button,
                TabIndex(0),
                Control {
                    action: Action::ImageEdit(EditAction::SelectLayer(index)),
                    look: Look::Regular,
                },
                AccessibleLabel(layer.name.clone()),
                BackgroundColor(Color::NONE),
                BackgroundGradient::default(),
                BorderColor::all(EDGE),
            ))
            .observe(activate_row)
            .observe(select_row)
            .observe(drag_ui::press);
        ux::help(
            commands,
            select,
            format!(
                "{} · выберите слой, чтобы рисовать на нём. Двойной щелчок — переименовать. Перетаскивание — изменить порядок; Ctrl — выбрать несколько слоёв.",
                layer.name
            ),
        );
        if !layer.is_group() && state.thumbnail != ThumbnailSize::None {
            let handle = layer_thumbnail(
                doc,
                layer,
                false,
                state.thumbnail_content,
                thumbs,
                images,
                uploads,
            );
            commands.spawn((
                ImageNode::new(handle),
                LayerThumbnail(layer.id.0, false, state.thumbnail_content),
                Node {
                    width: px(state.thumbnail.pixels()),
                    height: px(state.thumbnail.pixels()),
                    flex_shrink: 0.0,
                    ..default()
                },
                Pickable::IGNORE,
                ChildOf(select),
            ));
        } else if layer.is_group() {
            icons::themed(commands, select, icons::Glyph::Folder, 22.0);
        }
        let label = node(
            commands,
            select,
            Node {
                flex_grow: 1.0,
                min_width: px(0),
                overflow: Overflow::clip(),
                ..column(1.0)
            },
        );
        commands.entity(label).insert(Pickable::IGNORE);
        text(
            commands,
            label,
            font,
            &format!("{}{}", if layer.clipped { "↳ " } else { "" }, layer.name),
            12.0,
            TEXT,
        );
        text(
            commands,
            label,
            font,
            &format!(
                "{}{}{}",
                if layer_kind(layer) == "Растровый" {
                    "Рисунок"
                } else {
                    layer_kind(layer)
                },
                if layer.locks.all {
                    " · блокировка"
                } else {
                    ""
                },
                if layer.link_group.is_some() {
                    " · связь"
                } else {
                    ""
                }
            ),
            9.0,
            MUTED,
        );
        if layer.mask.is_some() {
            let handle = layer_thumbnail(doc, layer, true, false, thumbs, images, uploads);
            let mask = control(
                commands,
                row_entity,
                Action::ImageEdit(EditAction::Layers(LayerAction::Mask(index))),
                Look::Tool,
                Node {
                    width: px(32),
                    height: px(34),
                    padding: UiRect::all(px(2)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(3)),
                    ..default()
                },
            );
            commands.entity(mask).insert((
                AccessibleLabel("Маска слоя · рисовать по маске".into()),
                BorderColor::all(if doc.active == index && doc.mask_target {
                    FOCUS
                } else {
                    EDGE
                }),
            ));
            commands.spawn((
                ImageNode::new(handle),
                LayerThumbnail(layer.id.0, true, false),
                Node {
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
                Pickable::IGNORE,
                ChildOf(mask),
            ));
        }
        if layer.vector_mask.is_some() {
            let mask = icons::button(
                commands,
                row_entity,
                icons::Glyph::Rectangle,
                "Векторная маска",
                act(LayerAction::VectorMask(index)),
                Look::Regular,
            );
            commands.entity(mask).entry::<Node>().and_modify(|mut n| {
                n.width = px(26);
                n.height = px(28);
                n.padding = UiRect::ZERO;
            });
        }
        if !layer.effects.items.is_empty() {
            compact(
                commands,
                row_entity,
                font,
                "fx",
                LayerAction::Effects(index),
            );
        }
        let lock = icons::button(
            commands,
            row_entity,
            icons::Glyph::Lock,
            if layer.locks.all {
                "Разблокировать слой"
            } else {
                "Заблокировать слой"
            },
            act(LayerAction::ToggleLock(layer.id.0)),
            Look::Regular,
        );
        commands.entity(lock).entry::<Node>().and_modify(|mut n| {
            n.width = px(26);
            n.height = px(28);
            n.padding = UiRect::ZERO;
        });
        ux::help(
            commands,
            lock,
            ux::action_help(LayerAction::ToggleLock(layer.id.0)),
        );
        if let LayerContent::Smart(s) = &layer.content {
            for (filter, value) in s.smart_filters.iter().enumerate().rev() {
                let filter_row = node(
                    commands,
                    host,
                    Node {
                        width: percent(100),
                        padding: UiRect::left(px((depth.min(6) + 1) as f32 * 10.0)),
                        ..row(3.0)
                    },
                );
                compact(
                    commands,
                    filter_row,
                    font,
                    if value.visible { "●" } else { "○" },
                    LayerAction::SmartFilterVisible(index, filter),
                );
                compact(
                    commands,
                    filter_row,
                    font,
                    &title(&value.command),
                    LayerAction::SmartFilter(index, filter),
                );
            }
            if !s.smart_filters.is_empty() {
                compact(
                    commands,
                    host,
                    font,
                    if s.filter_mask.is_some() {
                        "Маска смарт-фильтров"
                    } else {
                        "Добавить маску фильтров"
                    },
                    LayerAction::FilterMask(index),
                );
            }
        }
    }
    drag_ui::extent(commands, host);
    if doc.document.layer_count() == 0 {
        text(
            commands,
            host,
            font,
            "Нет слоёв. Нажмите +, чтобы добавить.",
            12.0,
            MUTED,
        );
    } else if no_matches {
        text(commands, host, font, "Слои не найдены", 12.0, MUTED);
    }
    thumbs.retain(|(id, _, _), _| doc.document.layer(LayerId(*id)).is_some());
}
// The helper name differs from the local selection node variable.
fn select_row(
    event: On<PointerClick>,
    rows: Query<&LayerRow>,
    keys: Res<ButtonInput<KeyCode>>,
    actions: MessageWriter<Action>,
    viewer: Res<ImageViewer>,
    ui: ResMut<LayerUi>,
    drag: Res<drag_ui::LayerDrag>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    select(event, rows, keys, actions, viewer, ui, drag, window);
}

fn popup(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    state: &LayerState,
    error: Option<&str>,
    focus: &mut InputFocus,
) -> (Option<Entity>, Option<Entity>) {
    let sheet = surface(
        commands,
        parent,
        Node {
            width: px(if state.form.is_some() { 420 } else { 380 }),
            position_type: PositionType::Absolute,
            right: px(16),
            top: px(105),
            max_width: percent(92),
            max_height: percent(86),
            padding: UiRect::all(px(16)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(10)),
            ..column(10.0)
        },
        PANEL,
    );
    commands.entity(sheet).insert(BorderColor::all(EDGE));
    let header = node(
        commands,
        sheet,
        Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            ..row(8.0)
        },
    );
    text(
        commands,
        header,
        font,
        state.form.as_ref().map_or(
            if state.category.is_empty() {
                "Действия со слоем"
            } else {
                state.category
            },
            |f| f.title.as_str(),
        ),
        16.0,
        TEXT,
    );
    compact(commands, header, font, "Закрыть · Esc", LayerAction::Cancel);
    if let Some(form) = &state.form {
        let scroll = form_ui::build(commands, sheet, font, state, focus);
        if let Some(error) = error {
            text(commands, sheet, font, error, 11.0, ERROR);
        }
        if state.advanced
            && form
                .command
                .strip_prefix("layer.layerStyle.")
                .is_some_and(|kind| {
                    photocraft_engine::layer_style::effect_from_params(kind, &json!({})).is_some()
                })
        {
            let defaults = node(commands, sheet, row(6.0));
            compact(
                commands,
                defaults,
                font,
                "Сделать стандартными",
                LayerAction::SaveEffectDefault,
            );
            compact(
                commands,
                defaults,
                font,
                "Вернуть значения",
                LayerAction::ResetEffectDefault,
            );
        }
        let footer = node(
            commands,
            sheet,
            Node {
                width: percent(100),
                justify_content: JustifyContent::SpaceBetween,
                ..row(5.0)
            },
        );
        if ux::can_preview(form.command) {
            compact(
                commands,
                footer,
                font,
                if state.preview_enabled {
                    "Предпросмотр: вкл."
                } else {
                    "Предпросмотр: выкл."
                },
                LayerAction::Preview,
            );
        }
        let actions = node(commands, footer, row(4.0));
        compact(commands, actions, font, "Отмена", LayerAction::Cancel);
        let apply = button(
            commands,
            actions,
            font,
            "Применить",
            act(LayerAction::Apply),
            Look::Primary,
        );
        ux::help(commands, apply, ux::action_help(LayerAction::Apply));
        return (None, Some(scroll));
    } else {
        if state.category == "Дополнительно" {
            text(
                commands,
                sheet,
                font,
                "Найти дополнительный инструмент",
                11.0,
                MUTED,
            );
            let search =
                super::super::interface::input(commands, sheet, font, "Найти операцию со слоями");
            commands
                .entity(search)
                .insert((CommandSearch, EditableText::new(&state.search)));
        } else if !state.category.is_empty() {
            compact(
                commands,
                sheet,
                font,
                "Назад к действиям",
                LayerAction::Category(""),
            );
        }
        let scroll = scroll_node(
            commands,
            sheet,
            Node {
                width: percent(100),
                max_height: px(490),
                overflow: Overflow::scroll_y(),
                ..column(3.0)
            },
        );
        return (Some(scroll), Some(scroll));
    }
}

fn command_results(
    commands: &mut Commands,
    scroll: Entity,
    font: &Handle<Font>,
    state: &LayerState,
) {
    if state.category == "Вид панели" && state.search.is_empty() {
        panel_settings(commands, scroll, font, state);
        return;
    }
    if state.category != "Дополнительно" && state.category != "Вид панели" {
        for (label, action) in ux::menu(state.category) {
            let button = compact(commands, scroll, font, label, action);
            commands.entity(button).entry::<Node>().and_modify(|mut n| {
                n.width = percent(100);
                n.height = px(33);
            });
        }
        return;
    }
    let search = state.search.to_lowercase();
    if search.is_empty() {
        compact(
            commands,
            scroll,
            font,
            "Копировать выделенные слои",
            LayerAction::Copy,
        );
        compact(commands, scroll, font, "Вставить слои", LayerAction::Paste);
    }
    for spec in catalog() {
        let label = title(spec.id);
        if !label
            .chars()
            .any(|c| ('а'..='я').contains(&c) || ('А'..='Я').contains(&c))
        {
            continue;
        }
        if !search.is_empty()
            && !label.to_lowercase().contains(&search)
            && !spec.label.to_lowercase().contains(&search)
        {
            continue;
        }
        compact(
            commands,
            scroll,
            font,
            &label,
            LayerAction::Command(spec.id),
        );
    }
}

fn layer_thumbnail(
    doc: &PhotoDocument,
    layer: &photocraft_doc::Layer,
    mask: bool,
    content: bool,
    cache: &mut HashMap<(u64, bool, bool), (Option<photocraft_doc::Layer>, Handle<Image>)>,
    images: &mut Assets<Image>,
    uploads: &mut TextureUploads,
) -> Handle<Image> {
    let handle = cache
        .entry((layer.id.0, mask, content))
        .or_insert_with(|| (None, Handle::default()));
    if handle.0.as_ref() != Some(layer) {
        let bitmap = if mask {
            let longest = doc.size().width.max(doc.size().height).max(1);
            let (w, h) = (
                (doc.size().width * 32 / longest).max(1),
                (doc.size().height * 32 / longest).max(1),
            );
            let mut image = photocraft_raster::Rgba8Image::new(w, h);
            if let Some(mask) = &layer.mask {
                for (index, pixel) in image.pixels.chunks_exact_mut(4).enumerate() {
                    let x = (index as u32 % w) * doc.size().width / w;
                    let y = (index as u32 / w) * doc.size().height / h;
                    let value = (mask
                        .surface
                        .sample_channel(x as i32, y as i32, 0)
                        .clamp(0.0, 1.0)
                        * 255.0)
                        .round() as u8;
                    pixel.copy_from_slice(&[value, value, value, 255]);
                }
            }
            image
        } else {
            let mut preview = doc.document.clone();
            preview.layers = vec![layer.clone()];
            let layer = &mut preview.layers[0];
            layer.visible = true;
            layer.clipped = false;
            layer.mask = None;
            layer.vector_mask = None;
            layer.effects = Default::default();
            if content {
                let rect =
                    photocraft_compose::layer_bounds(&preview.layers[0], doc.document.bounds());
                if rect.is_empty() {
                    photocraft_compose::thumbnail(&preview, 64)
                } else {
                    let side = rect.width().max(rect.height()).max(1);
                    photocraft_compose::render_reduced_rect(
                        &preview,
                        rect,
                        (rect.width() * 64 / side).max(1),
                        (rect.height() * 64 / side).max(1),
                    )
                    .to_rgba8()
                }
            } else {
                photocraft_compose::thumbnail(&preview, 64)
            }
        };
        let mut texture = image_previews::texture_image(DecodedImage {
            size: UVec2::new(bitmap.width, bitmap.height),
            original: UVec2::new(bitmap.width, bitmap.height),
            rgba: bitmap.pixels,
        });
        texture.sampler = bevy::image::ImageSampler::linear();
        // Keep the GPU image and its bindings alive while a brush updates its pixels.
        // A full asset replacement is only necessary when the thumbnail changes size.
        let size = UVec2::new(bitmap.width, bitmap.height);
        if handle.0.is_none() {
            // Handle::default() is Bevy's shared white UI image, not an empty slot.
            handle.1 = images.add(texture);
        } else if let Some(image) = images.get_mut_untracked(&handle.1)
            && image.size() == size
        {
            let rgba = texture.data.unwrap_or_default();
            image.data = Some(rgba.clone());
            uploads.patch(handle.1.id(), UVec2::ZERO, size, rgba);
        } else if let Some(mut image) = images.get_mut(&handle.1) {
            uploads.reset(handle.1.id());
            *image = texture;
        } else {
            handle.1 = images.add(texture);
        }
        handle.0 = Some(layer.clone());
    }
    handle.1.clone()
}

fn panel_settings(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    state: &LayerState,
) {
    text(commands, parent, font, "Показывать слои", 12.0, TEXT);
    compact(
        commands,
        parent,
        font,
        if state.only_selected {
            "● Только выделенные"
        } else {
            "○ Только выделенные"
        },
        LayerAction::OnlySelected,
    );
    compact(
        commands,
        parent,
        font,
        if state.only_visible {
            "● Только видимые"
        } else {
            "○ Только видимые"
        },
        LayerAction::OnlyVisible,
    );
    let kinds = node(
        commands,
        parent,
        Node {
            width: percent(100),
            flex_wrap: FlexWrap::Wrap,
            ..row(4.0)
        },
    );
    for kind in [
        "",
        "Растровый",
        "Группа",
        "Коррекция",
        "Заливка",
        "Текст",
        "Фигура",
        "Смарт-объект",
        "Монтажная область",
    ] {
        compact(
            commands,
            kinds,
            font,
            &format!(
                "{} {}",
                if state.filter_kind == kind {
                    "●"
                } else {
                    "○"
                },
                if kind.is_empty() {
                    "Все типы"
                } else {
                    kind
                }
            ),
            LayerAction::FilterKind(kind),
        );
    }
    text(commands, parent, font, "Цветовая метка", 12.0, TEXT);
    let labels = node(
        commands,
        parent,
        Node {
            width: percent(100),
            flex_wrap: FlexWrap::Wrap,
            ..row(4.0)
        },
    );
    compact(
        commands,
        labels,
        font,
        "Все",
        LayerAction::FilterLabel(None),
    );
    for label in photocraft_doc::LabelColor::ALL {
        let button = compact(
            commands,
            labels,
            font,
            &format!(
                "{} {}",
                if state.filter_label == Some(label) {
                    "●"
                } else {
                    "○"
                },
                ux::choice(&label.label().to_lowercase())
            ),
            LayerAction::FilterLabel(Some(label)),
        );
        commands
            .entity(button)
            .insert(BorderColor::all(label_color(label)));
    }
    compact(
        commands,
        parent,
        font,
        "Сбросить фильтры",
        LayerAction::ResetFilter,
    );
    text(commands, parent, font, "Размер миниатюр", 12.0, TEXT);
    let sizes = node(commands, parent, row(4.0));
    for (label, size) in [
        ("Нет", ThumbnailSize::None),
        ("Мелкие", ThumbnailSize::Small),
        ("Средние", ThumbnailSize::Medium),
        ("Крупные", ThumbnailSize::Large),
    ] {
        compact(
            commands,
            sizes,
            font,
            &format!(
                "{} {label}",
                if state.thumbnail == size {
                    "●"
                } else {
                    "○"
                }
            ),
            LayerAction::Thumbnail(size),
        );
    }
    compact(
        commands,
        parent,
        font,
        if state.thumbnail_content {
            "● Миниатюра по содержимому"
        } else {
            "○ Миниатюра по холсту"
        },
        LayerAction::ThumbnailContent,
    );
}
fn label_color(label: photocraft_doc::LabelColor) -> Color {
    use photocraft_doc::LabelColor::*;
    let rgb = match label {
        None => [0.4, 0.4, 0.4],
        Red => [0.9, 0.27, 0.27],
        Orange => [0.95, 0.52, 0.2],
        Yellow => [0.96, 0.81, 0.25],
        Green => [0.35, 0.76, 0.4],
        Seafoam => [0.35, 0.8, 0.72],
        Blue => [0.3, 0.55, 0.96],
        Indigo => [0.42, 0.4, 0.86],
        Magenta => [0.81, 0.3, 0.68],
        Fuchsia => [0.96, 0.36, 0.6],
        Violet => [0.65, 0.4, 0.88],
        Gray => [0.65, 0.65, 0.65],
    };
    Color::srgb(rgb[0], rgb[1], rgb[2])
}

fn dropdown(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    action: LayerAction,
) -> Entity {
    let button = compact(commands, parent, font, label, action);
    commands.entity(button).entry::<Node>().and_modify(|mut n| {
        n.max_width = percent(100);
        n.flex_shrink = 1.0;
    });
    icons::themed(commands, button, icons::Glyph::ChevronDown, 12.0);
    button
}
