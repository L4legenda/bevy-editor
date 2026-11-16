use super::*;
use bevy::ui::InteractionDisabled;

#[derive(Component)]
pub(in crate::editor) struct ToolLabel(TextAction);
#[derive(Component)]
pub(in crate::editor) struct BodyStyle;
#[derive(Component, Default)]
pub(in crate::editor) struct NativeGlyphs(pub Vec<bevy::text::PositionedGlyph>);

// Retain the native geometry for editing, but submit ink from the rich display
// only. Transparent text can still submit colour-font glyphs to the UI renderer.
pub(in crate::editor) fn hide_native_ink(
    mut bodies: Query<(
        &EditableText,
        &mut bevy::text::TextLayoutInfo,
        &mut NativeGlyphs,
    )>,
) {
    for (input, mut layout, mut saved) in &mut bodies {
        if !layout.glyphs.is_empty() {
            saved.0 = std::mem::take(&mut layout.glyphs);
        } else if input.editor.raw_text().is_empty() && !saved.0.is_empty() {
            saved.0.clear();
        }
    }
}
#[derive(Component)]
pub(in crate::editor) struct DecorationLayer {
    body: Entity,
    key: String,
    text_key: String,
    text: Option<Entity>,
    selection: Option<Entity>,
    caret: Option<Entity>,
    spans: Vec<Entity>,
}

pub(in crate::editor) fn setup(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    fonts: &mut Assets<Font>,
) {
    let root = surface(
        commands,
        parent,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        Color::srgba(0.008, 0.012, 0.023, 0.89),
    );
    commands.entity(root).insert((
        ZIndex(80),
        TabGroup::modal(),
        AccessibleLabel("Редактор текста реплики".into()),
    ));
    let panel = surface(
        commands,
        root,
        Node {
            width: px(1100),
            max_width: percent(96),
            height: percent(92),
            max_height: px(850),
            min_height: px(0),
            padding: UiRect::all(px(20)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(16)),
            ..column(12.0)
        },
        PANEL,
    );
    commands
        .entity(panel)
        .insert(BorderColor::all(FOCUS.with_alpha(0.65)));
    let color_ui =
        crate::editor::ui::image_viewer::edit::picker_ui::setup_text(commands, parent, font);
    commands.insert_resource(State {
        root,
        panel,
        session: None,
        built: None,
        body: None,
        find_row: None,
        confirm_row: None,
        preview_root: None,
        restore: None,
        color_ui,
        bold: fonts.add(Font::from_bytes(
            include_bytes!("../../assets/font/LiberationSans-Bold.ttf").to_vec(),
        )),
        italic: fonts.add(Font::from_bytes(
            include_bytes!("../../assets/font/LiberationSans-Italic.ttf").to_vec(),
        )),
        bold_italic: fonts.add(Font::from_bytes(
            include_bytes!("../../assets/font/LiberationSans-BoldItalic.ttf").to_vec(),
        )),
    });
}

fn toolbar(commands: &mut Commands, parent: Entity) -> Entity {
    node(
        commands,
        parent,
        Node {
            width: percent(100),
            flex_shrink: 0.0,
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(7),
            ..row(8.0)
        },
    )
}
fn group(commands: &mut Commands, parent: Entity) -> Entity {
    node(
        commands,
        parent,
        Node {
            flex_shrink: 0.0,
            ..row(4.0)
        },
    )
}

fn tool(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    hint: &str,
    action: TextAction,
) -> Entity {
    let button = control(
        commands,
        parent,
        Action::Narrative(NarrativeAction::Text(action)),
        if action == TextAction::Apply {
            Look::Primary
        } else {
            Look::Regular
        },
        Node {
            height: px(30),
            flex_shrink: 0.0,
            padding: UiRect::horizontal(px(9)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..row(4.0)
        },
    );
    commands
        .entity(button)
        .insert((Tool(action), AccessibleLabel(hint.into())));
    if matches!(action, TextAction::Undo | TextAction::Redo) {
        icons::spawn(
            commands,
            button,
            if action == TextAction::Undo {
                icons::Glyph::Undo
            } else {
                icons::Glyph::Redo
            },
            14.0,
            TEXT,
        );
    } else {
        let label = text(commands, button, font, label, 12.0, TEXT);
        commands.entity(label).insert(ToolLabel(action));
    }
    button
}

fn input(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    kind: Input,
    value: &str,
    label: &str,
    width: f32,
) -> Entity {
    let entity = commands
        .spawn((
            TextInput,
            kind,
            EditableText::new(value),
            crate::ui_theme::text_cursor_style(),
            AccessibleLabel(label.into()),
            TabIndex(0),
            super::super::font(font, 14.0),
            TextColor(TEXT),
            TextLayout::no_wrap(),
            BackgroundColor(WELL),
            BorderColor::all(EDGE),
            Node {
                width: px(width),
                min_width: px(0),
                height: px(30),
                padding: UiRect::axes(px(7), px(5)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
            ChildOf(parent),
        ))
        .id();
    if kind == Input::Size {
        commands
            .entity(entity)
            .insert(bevy::ui_widgets::SelectAllOnFocus);
    }
    entity
}

fn build(commands: &mut Commands, state: &mut State, ui: &NarrativeUi) {
    let session = state.session.as_ref().unwrap();
    let font = &ui.font;
    let header = toolbar(commands, state.panel);
    let title = node(
        commands,
        header,
        Node {
            flex_grow: 1.0,
            ..column(4.0)
        },
    );
    text(
        commands,
        title,
        font,
        format!("РЕДАКТОР ТЕКСТА · НОДА #{}", session.node),
        13.0,
        FOCUS,
    );
    text(
        commands,
        title,
        font,
        "Выделите текст, чтобы изменить размер, цвет и начертание",
        11.0,
        MUTED,
    );
    tool(
        commands,
        header,
        font,
        "Закрыть · Esc",
        "Закрыть редактор текста",
        TextAction::Close,
    );

    let edits = toolbar(commands, state.panel);
    let history = group(commands, edits);
    tool(
        commands,
        history,
        font,
        "↶",
        "Отменить изменение текста · Ctrl+Z",
        TextAction::Undo,
    );
    tool(
        commands,
        history,
        font,
        "↷",
        "Вернуть изменение текста · Ctrl+Shift+Z",
        TextAction::Redo,
    );
    tool(
        commands,
        edits,
        font,
        "Выделить всё",
        "Выделить весь текст · Ctrl+A",
        TextAction::SelectAll,
    );
    tool(
        commands,
        edits,
        font,
        "Вырезать",
        "Вырезать выделенный текст · Ctrl+X",
        TextAction::Cut,
    );
    tool(
        commands,
        edits,
        font,
        "Копировать",
        "Копировать выделенный текст · Ctrl+C",
        TextAction::Copy,
    );
    tool(
        commands,
        edits,
        font,
        "Вставить",
        "Вставить текст · Ctrl+V",
        TextAction::Paste,
    );
    tool(
        commands,
        edits,
        font,
        "Поиск и замена",
        "Поиск и замена · Ctrl+F",
        TextAction::Find,
    );
    tool(
        commands,
        edits,
        font,
        "Предпросмотр",
        "Переключить предпросмотр текста",
        TextAction::Preview,
    );

    let format = toolbar(commands, state.panel);
    let sizes = group(commands, format);
    text(commands, sizes, font, "Размер", 11.0, MUTED);
    tool(
        commands,
        sizes,
        font,
        "A−",
        "Уменьшить размер шрифта",
        TextAction::Size(-1),
    );
    input(
        commands,
        sizes,
        font,
        Input::Size,
        &session.size_text,
        "Размер шрифта",
        46.0,
    );
    tool(
        commands,
        sizes,
        font,
        "A+",
        "Увеличить размер шрифта",
        TextAction::Size(1),
    );
    let faces = group(commands, format);
    for (label, hint, action) in [
        ("B", "Полужирный · Ctrl+B", TextAction::Bold),
        ("I", "Курсив · Ctrl+I", TextAction::Italic),
        ("U", "Подчёркивание · Ctrl+U", TextAction::Underline),
        ("S", "Зачёркивание", TextAction::Strike),
    ] {
        tool(commands, faces, font, label, hint, action);
    }
    let align = group(commands, format);
    for (label, alignment) in [
        ("Слева", TextAlign::Left),
        ("Центр", TextAlign::Center),
        ("Справа", TextAlign::Right),
        ("По ширине", TextAlign::Justified),
    ] {
        tool(
            commands,
            align,
            font,
            label,
            &format!("Выравнивание: {label}"),
            TextAction::Align(alignment),
        );
    }
    tool(
        commands,
        format,
        font,
        "Интервал 1,2",
        "Изменить межстрочный интервал",
        TextAction::Spacing,
    );
    let colors = group(commands, format);
    let picker = tool(
        commands,
        colors,
        font,
        "Цвет текста ▾",
        "Выбрать цвет выделенного текста",
        TextAction::ColorMenu,
    );
    let sample = surface(
        commands,
        picker,
        Node {
            width: px(14),
            height: px(14),
            border_radius: BorderRadius::all(px(3)),
            ..default()
        },
        TEXT,
    );
    commands
        .entity(sample)
        .insert((ColorSample, Pickable::IGNORE));
    let extras = toolbar(commands, state.panel);
    tool(
        commands,
        extras,
        font,
        "• Список",
        "Маркированный список",
        TextAction::Bullets,
    );
    tool(
        commands,
        extras,
        font,
        "1. Список",
        "Нумерованный список",
        TextAction::Numbered,
    );
    tool(
        commands,
        extras,
        font,
        "Убрать лишние пробелы",
        "Убрать пробелы в конце строк",
        TextAction::Clean,
    );
    tool(
        commands,
        extras,
        font,
        "Сбросить оформление",
        "Сбросить оформление текста",
        TextAction::Reset,
    );

    let find = toolbar(commands, state.panel);
    commands
        .entity(find)
        .entry::<Node>()
        .and_modify(|mut n| n.display = Display::None);
    input(
        commands,
        find,
        font,
        Input::Find,
        "",
        "Найти в тексте",
        136.0,
    );
    input(
        commands,
        find,
        font,
        Input::Replace,
        "",
        "Заменить на",
        136.0,
    );
    tool(
        commands,
        find,
        font,
        "↑",
        "Предыдущее совпадение · Shift+F3",
        TextAction::Next(true),
    );
    tool(
        commands,
        find,
        font,
        "↓",
        "Следующее совпадение · F3",
        TextAction::Next(false),
    );
    tool(
        commands,
        find,
        font,
        "Aa",
        "Учитывать регистр при поиске",
        TextAction::MatchCase,
    );
    tool(
        commands,
        find,
        font,
        "Заменить",
        "Заменить совпадение",
        TextAction::Replace,
    );
    tool(
        commands,
        find,
        font,
        "Заменить все",
        "Заменить все совпадения",
        TextAction::ReplaceAll,
    );
    let count = text(commands, find, font, "", 11.0, MUTED);
    commands.entity(count).insert(FindSummary);
    tool(
        commands,
        find,
        font,
        "×",
        "Закрыть поиск и замену",
        TextAction::HideFind,
    );
    state.find_row = Some(find);

    let area = node(
        commands,
        state.panel,
        Node {
            width: percent(100),
            flex_grow: 1.0,
            min_height: px(0),
            ..column(0.0)
        },
    );
    let mut editable = EditableText::new(&session.draft.text);
    editable.allow_newlines = true;
    editable.visible_lines = None;
    editable.cursor_width = 1.5 / session.draft.format.font_size;
    let body = commands
        .spawn((
            TextInput,
            Input::Body,
            BodyStyle,
            editable,
            bevy::text::TextCursorStyle {
                color: Color::NONE,
                selection_color: Color::NONE,
                unfocused_selection_color: Color::NONE,
                selected_text_color: None,
                ..default()
            },
            AccessibleLabel("Текст в большом редакторе".into()),
            TabIndex(0),
            formatted_font(state, font, &session.draft.format),
            TextColor(Color::NONE),
            alignment(&session.draft.format),
            LineHeight::RelativeToFont(session.draft.format.line_height),
            BackgroundColor(WELL),
            BorderColor::all(FOCUS.with_alpha(0.3)),
            Node {
                width: percent(100),
                flex_grow: 1.0,
                min_height: px(0),
                padding: UiRect::all(px(20)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(10)),
                overflow: Overflow::clip(),
                ..default()
            },
            ChildOf(area),
        ))
        .id();
    commands.entity(body).insert(NativeGlyphs::default());
    state.body = Some(body);
    let layer = node(
        commands,
        body,
        Node {
            position_type: PositionType::Absolute,
            left: px(21),
            right: px(21),
            top: px(21),
            bottom: px(21),
            overflow: Overflow::clip(),
            ..default()
        },
    );
    commands.entity(layer).insert((
        DecorationLayer {
            body,
            key: String::new(),
            text_key: String::new(),
            text: None,
            selection: None,
            caret: None,
            spans: Vec::new(),
        },
        Pickable::IGNORE,
    ));
    let preview = surface(
        commands,
        area,
        Node {
            display: Display::None,
            width: percent(100),
            flex_grow: 1.0,
            min_height: px(0),
            padding: UiRect::all(px(20)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(10)),
            overflow: Overflow::scroll_y(),
            ..column(0.0)
        },
        WELL,
    );
    commands.entity(preview).insert(Scroller);
    let value = render(
        commands,
        preview,
        font,
        state,
        &session.draft.text,
        &session.draft.format,
        &session.draft.spans,
    );
    commands.entity(value).insert(PreviewText);
    state.preview_root = Some(preview);

    let notice = text(commands, state.panel, font, "", 11.0, ERROR);
    commands.entity(notice).insert(Notice);
    let confirm = toolbar(commands, state.panel);
    commands
        .entity(confirm)
        .entry::<Node>()
        .and_modify(|mut n| n.display = Display::None);
    text(
        commands,
        confirm,
        font,
        "Есть изменения в тексте.",
        12.0,
        FOCUS,
    );
    tool(
        commands,
        confirm,
        font,
        "Продолжить редактирование",
        "Продолжить редактирование текста",
        TextAction::KeepEditing,
    );
    tool(
        commands,
        confirm,
        font,
        "Отменить изменения",
        "Отменить изменения в редакторе текста",
        TextAction::Discard,
    );
    state.confirm_row = Some(confirm);
    let footer = toolbar(commands, state.panel);
    let counter = text(commands, footer, font, "", 11.0, MUTED);
    commands.entity(counter).insert((
        Summary,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            ..default()
        },
    ));
    tool(
        commands,
        footer,
        font,
        "Отмена",
        "Отмена редактирования текста",
        TextAction::Cancel,
    );
    tool(
        commands,
        footer,
        font,
        "Применить · Ctrl+Enter",
        "Применить текст реплики",
        TextAction::Apply,
    );
    commands.queue(move |world: &mut World| {
        world
            .resource_mut::<InputFocus>()
            .set(body, bevy::input_focus::FocusCause::Auto);
    });
}

pub(in crate::editor) fn refresh(
    mut commands: Commands,
    mut state: ResMut<State>,
    ui: Res<NarrativeUi>,
    mut nodes: Query<&mut Node>,
    mut fields: Query<(Entity, &Input, &mut EditableText)>,
    mut fonts: ResMut<FontCx>,
    mut layouts: ResMut<LayoutCx>,
    assets: Res<Assets<Font>>,
    tools: Query<(Entity, &Tool)>,
) {
    if let Ok(mut node) = nodes.get_mut(state.root) {
        node.display = if state.is_open() {
            Display::Flex
        } else {
            Display::None
        };
    }
    let Some(session) = state.session.as_ref() else {
        state.built = None;
        if let Some((document, node)) = state.restore.take() {
            commands.queue(move |world: &mut World| {
                if world.resource::<Editor>().narrative.active != Some(document) {
                    return;
                }
                let entity = world
                    .query::<(Entity, &DraftField, &nodes::Owner)>()
                    .iter(world)
                    .find(|(_, field, owner)| **field == DraftField::Text && owner.0 == node)
                    .map(|(e, _, _)| e);
                if let Some(entity) = entity {
                    world
                        .resource_mut::<InputFocus>()
                        .set(entity, bevy::input_focus::FocusCause::Auto);
                }
            });
        }
        return;
    };
    let key = (session.document, session.node);
    if state.built != Some(key) {
        commands.entity(state.panel).despawn_related::<Children>();
        build(&mut commands, &mut state, &ui);
        state.built = Some(key);
        return;
    }
    let find = state.find_row;
    let confirm = state.confirm_row;
    let body = state.body;
    let preview = state.preview_root;
    let faces = [
        ui.font.clone(),
        state.bold.clone(),
        state.italic.clone(),
        state.bold_italic.clone(),
    ];
    let session = state.session.as_mut().unwrap();
    for (entity, tool) in &tools {
        if tool.0 == TextAction::Cancel {
            if let Ok(mut node) = nodes.get_mut(entity) {
                node.display = if session.confirm {
                    Display::None
                } else {
                    Display::Flex
                };
            }
        }
    }
    for (entity, visible) in [
        (find, session.find_open),
        (confirm, session.confirm),
        (body, !session.preview),
        (preview, session.preview),
    ] {
        if let Some(entity) = entity {
            if let Ok(mut node) = nodes.get_mut(entity) {
                node.display = if visible {
                    Display::Flex
                } else {
                    Display::None
                };
            }
        }
    }
    if session.rewrite_fields || session.format_fields {
        let size = session.draft.selected_format().font_size;
        session.size_text = if !session.draft.selection.is_empty()
            && !session.draft.all_selected(|f| f.font_size == size)
        {
            "—".into()
        } else {
            size.to_string()
        };
        for (_, kind, mut field) in &mut fields {
            let value = match kind {
                Input::Body if session.rewrite_fields => &session.draft.text,
                Input::Size => &session.size_text,
                _ => continue,
            };
            if field.value().to_string() != *value {
                field.pending_edits.clear();
                field.editor.set_text(value);
            }
        }
    }
    for (_, kind, mut field) in &mut fields {
        if *kind == Input::Body {
            apply_native_styles(&mut field, &session.draft, &assets, &faces);
            let width = 1.5 / session.draft.format.font_size;
            if field.cursor_width != width {
                field.cursor_width = width;
            }
            if session.rewrite_fields {
                let range = &session.draft.selection;
                field
                    .editor
                    .driver(&mut fonts, &mut layouts.0)
                    .select_byte_range(range.start, range.end);
                reveal(&mut field);
            }
        }
    }
    session.rewrite_fields = false;
    session.format_fields = false;
}

// EditableText provides native editing, IME, selection and hit testing. Both the
// editor and this rich display use the same Parley runs, so mixed sizes wrap alike.
#[derive(Component)]
pub(in crate::editor) struct ColorSample;

fn apply_native_styles(
    input: &mut EditableText,
    draft: &Draft,
    fonts: &Assets<Font>,
    faces: &[Handle<Font>; 4],
) {
    let raw = input.editor.raw_text();
    let spans = rich::remap(&draft.text, raw, &draft.format, &draft.spans);
    let mut styles = Vec::new();
    for run in rich::runs(raw, &draft.format, &spans) {
        let f = &run.format;
        let range = run.start..run.end;
        let index = usize::from(f.bold) + 2 * usize::from(f.italic);
        if let Some(font) = fonts.get(&faces[index]) {
            styles.push((
                range.clone(),
                parley::StyleProperty::FontFamily(parley::FontFamily::Single(
                    parley::FontFamilyName::Named(std::borrow::Cow::Owned(font.alias.to_string())),
                )),
            ));
        }
        styles.extend([
            (range.clone(), parley::StyleProperty::FontSize(f.font_size)),
            (
                range.clone(),
                parley::StyleProperty::FontWeight(if f.bold {
                    parley::FontWeight::BOLD
                } else {
                    parley::FontWeight::NORMAL
                }),
            ),
            (
                range.clone(),
                parley::StyleProperty::FontStyle(if f.italic {
                    parley::FontStyle::Italic
                } else {
                    parley::FontStyle::Normal
                }),
            ),
        ]);
    }
    input.editor.set_ranged_styles(styles);
}

pub(in crate::editor) fn decorate(
    mut commands: Commands,
    state: Res<State>,
    ui: Res<NarrativeUi>,
    inputs: Query<(&EditableText, &bevy::text::TextLayoutInfo)>,
    mut layers: Query<(Entity, &mut DecorationLayer)>,
    previews: Query<Entity, With<PreviewText>>,
    mut preview_key: Local<String>,
    focus: Res<InputFocus>,
    time: Res<Time>,
    mut samples: Query<&mut BackgroundColor, With<ColorSample>>,
    mut span_values: Query<(
        &mut bevy::text::TextSpan,
        &mut TextFont,
        &mut TextColor,
        &mut LineHeight,
        Has<Underline>,
        Has<Strikethrough>,
    )>,
) {
    let Some(session) = state.session.as_ref() else {
        return;
    };
    for mut sample in &mut samples {
        sample.set_if_neq(BackgroundColor(color(&session.draft.selected_format())));
    }
    let next_preview = format!(
        "{:?}:{:?}:{:?}:{:?}",
        session.draft.text, session.draft.format, session.draft.spans, state.built
    );
    if *preview_key != next_preview {
        for entity in &previews {
            commands.entity(entity).despawn_related::<Children>();
            render_spans(
                &mut commands,
                entity,
                &ui.font,
                &state,
                &session.draft.text,
                &session.draft.format,
                &session.draft.spans,
            );
        }
        *preview_key = next_preview;
    }
    for (entity, mut layer) in &mut layers {
        let Ok((input, info)) = inputs.get(layer.body) else {
            continue;
        };
        let blink = focus.get() == Some(layer.body) && time.elapsed_secs() % 1.1 < 0.65;
        let scale = input.editor.get_scale().max(0.01);
        let offset = input.viewport.offset;
        let selection = if let Some(e) = layer.selection {
            e
        } else {
            let e = node(
                &mut commands,
                entity,
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
            );
            commands.entity(e).insert(Pickable::IGNORE);
            layer.selection = Some(e);
            e
        };
        let raw = input.editor.raw_text();
        let spans = rich::remap(
            &session.draft.text,
            raw,
            &session.draft.format,
            &session.draft.spans,
        );
        let text_key = format!("{:?}:{:?}:{:?}", raw, session.draft.format, spans);
        let text = if let Some(e) = layer.text {
            e
        } else {
            let (e, spans) = render_parts(
                &mut commands,
                entity,
                &ui.font,
                &state,
                raw,
                &session.draft.format,
                &spans,
            );
            layer.text = Some(e);
            layer.spans = spans;
            layer.text_key = text_key.clone();
            e
        };
        if layer.text_key != text_key {
            let runs = rich::runs(raw, &session.draft.format, &spans);
            if layer.spans.len() != runs.len() {
                commands.entity(text).despawn_related::<Children>();
                layer.spans = render_spans(
                    &mut commands,
                    text,
                    &ui.font,
                    &state,
                    raw,
                    &session.draft.format,
                    &spans,
                );
            } else {
                for (&entity, run) in layer.spans.iter().zip(&runs) {
                    if let Ok((mut value, mut face, mut tint, mut height, underline, strike)) =
                        span_values.get_mut(entity)
                    {
                        if value.0 != raw[run.start..run.end] {
                            value.0 = raw[run.start..run.end].to_owned();
                        }
                        face.set_if_neq(formatted_font(&state, &ui.font, &run.format));
                        tint.set_if_neq(TextColor(color(&run.format)));
                        height.set_if_neq(LineHeight::RelativeToFont(
                            session.draft.format.line_height,
                        ));
                        if underline != run.format.underline || strike != run.format.strikethrough {
                            decorations(&mut commands, entity, &run.format);
                        }
                    }
                }
            }
            let face = formatted_font(&state, &ui.font, &session.draft.format);
            let tint = TextColor(color(&session.draft.format));
            let layout = alignment(&session.draft.format);
            let height = LineHeight::RelativeToFont(session.draft.format.line_height);
            commands
                .entity(text)
                .entry::<TextFont>()
                .and_modify(move |mut v| {
                    v.set_if_neq(face);
                });
            commands
                .entity(text)
                .entry::<TextColor>()
                .and_modify(move |mut v| {
                    v.set_if_neq(tint);
                });
            commands
                .entity(text)
                .entry::<LineHeight>()
                .and_modify(move |mut v| {
                    v.set_if_neq(height);
                });
            commands
                .entity(text)
                .entry::<TextLayout>()
                .and_modify(move |mut v| {
                    if v.justify != layout.justify || v.linebreak != layout.linebreak {
                        *v = layout;
                    }
                });
            layer.text_key = text_key;
        }
        commands.entity(text).insert((
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                left: px(-offset.x / scale),
                top: px(-offset.y / scale),
                width: px(input.viewport.size.x / scale),
                ..default()
            },
        ));
        let key = format!(
            "{:?}:{:?}:{:?}:{:?}:{}",
            input.editor.generation(),
            input.viewport,
            info.size,
            session.draft.selection,
            focus.get() == Some(layer.body)
        );
        let rect = |commands: &mut Commands, parent: Entity, bounds: Rect, tint: Color| {
            commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px((bounds.min.x - offset.x) / scale),
                        top: px((bounds.min.y - offset.y) / scale),
                        width: px(bounds.width() / scale),
                        height: px(bounds.height() / scale),
                        ..default()
                    },
                    BackgroundColor(tint),
                    Pickable::IGNORE,
                    ChildOf(parent),
                ))
                .id()
        };
        if layer.key != key {
            layer.key = key;
            commands.entity(selection).despawn_related::<Children>();
            if focus.get() != Some(layer.body) && !session.draft.selection.is_empty() {
                if let Some(layout) = input.editor.try_layout() {
                    use parley::{
                        editing::{Cursor, Selection},
                        layout::Affinity,
                    };
                    let range = &session.draft.selection;
                    let selected = Selection::new(
                        Cursor::from_byte_index(layout, range.start, Affinity::Downstream),
                        Cursor::from_byte_index(layout, range.end, Affinity::Downstream),
                    );
                    selected.geometry_with(layout, |b, _| {
                        rect(
                            &mut commands,
                            selection,
                            Rect::from_corners(
                                Vec2::new(b.x0 as f32, b.y0 as f32),
                                Vec2::new(b.x1 as f32, b.y1 as f32),
                            ),
                            FOCUS.with_alpha(0.22),
                        );
                    });
                }
            } else {
                for bounds in &info.selection_rects {
                    rect(&mut commands, selection, *bounds, FOCUS.with_alpha(0.22));
                }
            }
            for bounds in &info.preedit_underline_rects {
                rect(&mut commands, selection, *bounds, TEXT);
            }
        }
        // Blinking changes only this rectangle. Rebuilding shaped text here caused
        // transient missing glyphs after reopening and unnecessary atlas churn.
        let caret = if let Some(e) = layer.caret {
            e
        } else {
            let e = rect(&mut commands, entity, Rect::default(), TEXT);
            layer.caret = Some(e);
            e
        };
        let mut cursor = Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            ..default()
        };
        if blink && let Some(c) = input.editor.cursor_geometry(1.5 * scale) {
            cursor.display = Display::Flex;
            cursor.left = px((c.x0 as f32 - offset.x) / scale);
            cursor.top = px((c.y0 as f32 - offset.y) / scale);
            cursor.width = px(((c.x1 - c.x0) as f32 / scale).max(1.));
            cursor.height = px((c.y1 - c.y0) as f32 / scale);
        }
        commands.entity(caret).insert(cursor);
    }
}

pub(in crate::editor) fn style(
    mut commands: Commands,
    state: Res<State>,
    ui: Res<NarrativeUi>,
    mut values: Query<(
        &mut Text,
        Option<&Summary>,
        Option<&FindSummary>,
        Option<&Notice>,
        Option<&PreviewText>,
        Option<&ToolLabel>,
    )>,
    mut font_fields: Query<
        (
            Entity,
            &mut TextFont,
            &mut TextColor,
            &mut TextLayout,
            &mut LineHeight,
            Has<Underline>,
            Has<Strikethrough>,
        ),
        Or<(With<BodyStyle>, With<PreviewText>)>,
    >,
    mut controls: Query<(Entity, &Tool, &mut Control, Has<InteractionDisabled>)>,
    font_inputs: Query<(Entity, &Input, Has<InteractionDisabled>)>,
) {
    let Some(session) = state.session.as_ref() else {
        return;
    };
    let found = matches(&session.draft.text, &session.search, session.match_case);
    for (mut value, summary, search, notice, preview, label) in &mut values {
        let content = if summary.is_some() {
            format!(
                "Символы: {} · Слова: {} · Строки: {}",
                session.draft.text.chars().count(),
                session.draft.text.split_whitespace().count(),
                session.draft.text.split('\n').count()
            )
        } else if search.is_some() {
            format!("Найдено: {}", found.len())
        } else if notice.is_some() {
            if !session.size_valid {
                "Размер шрифта: от 8 до 72.".into()
            } else {
                session.error.clone()
            }
        } else if preview.is_some() {
            continue;
        } else if let Some(label) = label {
            match label.0 {
                TextAction::ColorMenu => format!("Цвет {}", hex(&session.draft.selected_format())),
                TextAction::Spacing => format!(
                    "Интервал {}",
                    session
                        .draft
                        .format
                        .line_height
                        .to_string()
                        .replace('.', ",")
                ),
                TextAction::Preview => {
                    if session.preview {
                        "Редактировать".into()
                    } else {
                        "Предпросмотр".into()
                    }
                }
                _ => continue,
            }
        } else {
            continue;
        };
        if value.0 != content {
            value.0 = content;
        }
    }
    for (entity, mut face, mut tint, mut layout, mut height, underline, strike) in &mut font_fields
    {
        face.set_if_neq(formatted_font(&state, &ui.font, &session.draft.format));
        tint.set_if_neq(TextColor(if state.body == Some(entity) {
            Color::NONE
        } else {
            color(&session.draft.format)
        }));
        let next_layout = alignment(&session.draft.format);
        if layout.justify != next_layout.justify || layout.linebreak != next_layout.linebreak {
            *layout = next_layout;
        }
        height.set_if_neq(LineHeight::RelativeToFont(session.draft.format.line_height));
        if underline != session.draft.format.underline
            || strike != session.draft.format.strikethrough
        {
            commands
                .entity(entity)
                .remove::<(Underline, Strikethrough)>();
        }
    }
    for (entity, kind, disabled) in &font_inputs {
        if *kind == Input::Size && disabled != session.draft.selection.is_empty() {
            if disabled {
                commands.entity(entity).remove::<InteractionDisabled>();
            } else {
                commands.entity(entity).insert(InteractionDisabled);
            }
        }
    }
    for (entity, tool, mut control, disabled) in &mut controls {
        let selected = match tool.0 {
            TextAction::Bold => session.draft.all_selected(|f| f.bold),
            TextAction::Italic => session.draft.all_selected(|f| f.italic),
            TextAction::Underline => session.draft.all_selected(|f| f.underline),
            TextAction::Strike => session.draft.all_selected(|f| f.strikethrough),
            TextAction::Align(value) => value == session.draft.format.alignment,
            TextAction::MatchCase => session.match_case,
            TextAction::Find => session.find_open,
            TextAction::Preview => session.preview,
            TextAction::Apply => true,
            _ => false,
        };
        let next_look = if selected {
            Look::Primary
        } else {
            Look::Regular
        };
        if std::mem::discriminant(&control.look) != std::mem::discriminant(&next_look) {
            control.look = next_look;
        }
        let should_disable = match tool.0 {
            TextAction::Undo => session.undo.is_empty(),
            TextAction::Redo => session.redo.is_empty(),
            TextAction::Cut
            | TextAction::Copy
            | TextAction::Size(_)
            | TextAction::Bold
            | TextAction::Italic
            | TextAction::Underline
            | TextAction::Strike
            | TextAction::ColorMenu
            | TextAction::Reset => session.draft.selection.is_empty(),
            TextAction::Next(_) | TextAction::Replace | TextAction::ReplaceAll => found.is_empty(),
            TextAction::Apply => !session.size_valid,
            _ => false,
        };
        if should_disable != disabled {
            if should_disable {
                commands.entity(entity).insert(InteractionDisabled);
            } else {
                commands.entity(entity).remove::<InteractionDisabled>();
            }
        }
    }
}
