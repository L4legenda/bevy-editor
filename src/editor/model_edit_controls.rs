//! Model-mode selector and inspector. Numeric edits are live transactions.
use super::*;
use bevy::{
    text::EditableText,
    ui::InteractionDisabled,
    ui_widgets::{SelectAllOnFocus, TextInput},
};

#[derive(Component)]
pub(in crate::editor) struct EditButton(pub(in crate::editor) EditAction);
#[derive(Component, Clone, Copy, PartialEq)]
pub(in crate::editor) enum Field {
    Transform(usize),
    Amount,
    Name,
}
#[derive(Component)]
pub(in crate::editor) struct Group(bool);
struct Draft {
    index: usize,
    entity: Entity,
    field: Field,
    before: Snapshot,
    text: String,
    error: Option<String>,
}
#[derive(Resource)]
pub(in crate::editor) struct Ui {
    toolbar: Entity,
    panel: Entity,
    menu: Entity,
    add: Entity,
    stats: Entity,
    list: Option<Entity>,
    stamp: Option<(usize, u64)>,
    bound: Option<usize>,
    draft: Option<Draft>,
}

fn action(a: EditAction) -> Action {
    Action::ModelView(ModelViewAction::Edit(a))
}
fn control(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    a: EditAction,
) -> Entity {
    let entity = button(commands, parent, font, label, action(a), Look::Regular);
    commands
        .entity(entity)
        .insert((EditButton(a), AccessibleLabel(format!("Модель: {label}"))));
    entity
}

pub(in crate::editor::ui) fn geometry_toolbar(commands: &mut Commands, parent: Entity) {
    let section = group(commands, parent);
    commands.entity(section).insert(Group(true));
    commands
        .entity(section)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.display = Display::None;
        });
    let action = EditAction::Operate(Operation::Extrude);
    let button = icons::button(
        commands,
        section,
        icons::Glyph::Extrude,
        "Экструдировать · E",
        self::action(action),
        Look::Regular,
    );
    commands.entity(button).insert(EditButton(action));
}

fn entry(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    field: Field,
    label: &str,
) -> Entity {
    let row = node(
        commands,
        parent,
        Node {
            width: percent(100),
            ..row(5.0)
        },
    );
    text(commands, row, font, label, 12.0, MUTED);
    let entity = commands
        .spawn((
            field,
            TextInput,
            EditableText::new(""),
            crate::ui_theme::text_cursor_style(),
            SelectAllOnFocus,
            AccessibleLabel(format!("Модель: {label}")),
            TabIndex(0),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(13.0),
                ..default()
            },
            TextColor(TEXT),
            Node {
                min_width: px(30),
                flex_grow: 1.0,
                height: px(28),
                padding: UiRect::axes(px(7), px(5)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(5)),
                ..default()
            },
            BackgroundColor(WELL),
            BorderColor::all(EDGE),
            ChildOf(row),
        ))
        .id();
    let number = match field {
        Field::Name => None,
        Field::Amount => Some(crate::editor::ui::Number::decimal(0.0001, 1.0e6, 0.01, 4)),
        Field::Transform(index) => Some(match index / 3 {
            0 => crate::editor::ui::Number::decimal(-1.0e6, 1.0e6, 0.01, 4),
            1 => crate::editor::ui::Number::decimal(-1.0e6, 1.0e6, 0.5, 4),
            _ => crate::editor::ui::Number::decimal(-1.0e6, 1.0e6, 0.01, 4).nonzero(0.0001),
        }),
    };
    if let Some(number) = number {
        commands.entity(entity).insert(number.finish());
    }
    entity
}

pub(in crate::editor::ui::model_viewer) fn setup(
    mut commands: Commands,
    interface: Res<Interface>,
    viewer: Res<ViewerUi>,
) {
    let toolbar = viewer.toolbar;
    commands.entity(toolbar).insert(ZIndex(30));
    let selector = surface(
        &mut commands,
        toolbar,
        Node {
            flex_shrink: 0.0,
            border_radius: BorderRadius::all(px(6)),
            ..column(3.0)
        },
        PANEL,
    );
    let select = control(
        &mut commands,
        selector,
        &interface.font,
        "Объект",
        EditAction::Menu,
    );
    commands
        .entity(select)
        .insert(AccessibleLabel("Режим модели".into()));
    icons::themed(&mut commands, select, icons::Glyph::ChevronDown, 12.0);
    let menu = surface(
        &mut commands,
        selector,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(36),
            width: px(185),
            padding: UiRect::all(px(6)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(7)),
            ..column(4.0)
        },
        PANEL,
    );
    commands.entity(menu).insert(GlobalZIndex(65));
    control(
        &mut commands,
        menu,
        &interface.font,
        "Объект",
        EditAction::Mode(Mode::Object),
    );
    control(
        &mut commands,
        menu,
        &interface.font,
        "Редактирование",
        EditAction::Mode(Mode::Edit),
    );
    control(
        &mut commands,
        menu,
        &interface.font,
        "UV-развёртка",
        EditAction::Mode(Mode::Uv),
    );
    control(
        &mut commands,
        menu,
        &interface.font,
        "Рисование",
        EditAction::Mode(Mode::Paint),
    );
    let elements = surface(
        &mut commands,
        toolbar,
        Node {
            padding: UiRect::all(px(2)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            flex_shrink: 0.0,
            ..row(1.0)
        },
        PANEL,
    );
    commands.entity(elements).insert(Group(true));
    for (label, kind, glyph) in [
        ("Вершины · 1", Element::Vertex, icons::Glyph::SelectVertex),
        ("Рёбра · 2", Element::Edge, icons::Glyph::SelectEdge),
        ("Грани · 3", Element::Face, icons::Glyph::SelectFace),
    ] {
        let action = EditAction::Element(kind);
        let button = super::super::control(
            &mut commands,
            elements,
            self::action(action),
            Look::Regular,
            Node {
                width: px(30),
                height: px(28),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
        );
        commands.entity(button).insert((
            EditButton(action),
            AccessibleLabel(format!("Модель: {label}")),
        ));
        icons::themed(&mut commands, button, glyph, 20.0);
    }
    let view_options = surface(
        &mut commands,
        toolbar,
        Node {
            padding: UiRect::all(px(2)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            flex_shrink: 0.0,
            ..row(0.0)
        },
        PANEL,
    );
    let xray = icons::button(
        &mut commands,
        view_options,
        icons::Glyph::Xray,
        "Модель: X-Ray · Рентген · Alt+Z",
        action(EditAction::Xray),
        Look::Regular,
    );
    commands.entity(xray).insert(EditButton(EditAction::Xray));
    control(
        &mut commands,
        view_options,
        &interface.font,
        "Сетка",
        EditAction::Wireframe,
    );
    let panel = node(
        &mut commands,
        interface.inspector,
        Node {
            display: Display::None,
            width: percent(100),
            row_gap: px(6),
            ..column(6.0)
        },
    );
    text(&mut commands, panel, &interface.font, "МОДЕЛЬ", 11.0, MUTED);
    let stats = text(&mut commands, panel, &interface.font, "", 12.0, MUTED);
    commands.entity(stats).insert(TextLayout::default());
    let commands_row = node(
        &mut commands,
        panel,
        Node {
            flex_wrap: FlexWrap::Wrap,
            ..row(3.0)
        },
    );
    for (label, a) in [
        ("Сохранить", EditAction::Save),
        ("Отмена", EditAction::Undo),
        ("Повтор", EditAction::Redo),
        ("Добавить…", EditAction::AddMenu),
    ] {
        control(&mut commands, commands_row, &interface.font, label, a);
    }
    let add = surface(
        &mut commands,
        panel,
        Node {
            display: Display::None,
            padding: UiRect::all(px(5)),
            ..column(3.0)
        },
        WELL,
    );
    for (i, label) in ["Куб", "Сфера", "Плоскость", "Цилиндр"]
        .into_iter()
        .enumerate()
    {
        control(
            &mut commands,
            add,
            &interface.font,
            label,
            EditAction::Add(i),
        );
    }
    entry(&mut commands, panel, &interface.font, Field::Name, "Имя");
    for (group, label) in [
        (0, "ПОЗИЦИЯ · МИРОВАЯ"),
        (1, "ВРАЩЕНИЕ · ГРАДУСЫ"),
        (2, "МАСШТАБ"),
    ] {
        let pane = node(&mut commands, panel, column(3.0));
        if group > 0 {
            commands.entity(pane).insert(Group(false));
        }
        text(&mut commands, pane, &interface.font, label, 11.0, MUTED);
        for (axis, name) in ["X", "Y", "Z"].into_iter().enumerate() {
            entry(
                &mut commands,
                pane,
                &interface.font,
                Field::Transform(group * 3 + axis),
                name,
            );
        }
    }
    let settings = node(
        &mut commands,
        panel,
        Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(3),
            ..row(3.0)
        },
    );
    for (label, a) in [
        ("Пропорции", EditAction::Proportional),
        ("Связанные", EditAction::Connected),
    ] {
        control(&mut commands, settings, &interface.font, label, a);
    }
    let edit = node(&mut commands, panel, column(6.0));
    commands.entity(edit).insert(Group(true));
    entry(
        &mut commands,
        edit,
        &interface.font,
        Field::Amount,
        "Шаг операции",
    );
    for (title, buttons) in [
        (
            "ГЕОМЕТРИЯ",
            vec![
                ("Выдавить · E", Operation::Extrude),
                ("Inset · I", Operation::Inset),
                ("Подразделить", Operation::Subdivide),
                ("Loop cut · Ctrl+R", Operation::LoopCut),
                ("Фаска вершин", Operation::BevelVertices),
                ("Треугольники", Operation::Triangulate),
                ("В четырёхугольники · Alt+J", Operation::Quads),
                ("Растворить рёбра", Operation::Dissolve),
                ("Заполнить · F", Operation::Fill),
                ("Объединить в центре · M", Operation::MergeCenter),
                ("По расстоянию · Alt+M", Operation::MergeDistance),
                ("Сгладить вершины", Operation::Smooth),
                ("Отделить", Operation::Separate),
            ],
        ),
        (
            "НОРМАЛИ И UV",
            vec![
                ("Пересчитать нормали", Operation::RecalculateNormals),
                ("Развернуть нормали", Operation::FlipNormals),
                ("UV по граням куба", Operation::ProjectUv(3)),
                ("UV сферой", Operation::SphericalUv),
            ],
        ),
    ] {
        text(&mut commands, edit, &interface.font, title, 11.0, MUTED);
        let group = node(
            &mut commands,
            edit,
            Node {
                flex_wrap: FlexWrap::Wrap,
                row_gap: px(3),
                ..row(3.0)
            },
        );
        for (label, op) in buttons {
            control(
                &mut commands,
                group,
                &interface.font,
                label,
                EditAction::Operate(op),
            );
        }
    }
    let select = node(
        &mut commands,
        panel,
        Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(3),
            ..row(3.0)
        },
    );
    for (label, a) in [
        ("Все · A", EditAction::SelectAll),
        ("Снять", EditAction::Deselect),
        ("Инверсия", EditAction::Invert),
        ("Связанные · Ctrl+L", EditAction::Linked),
        ("Расширить", EditAction::Grow),
        ("Сузить", EditAction::Shrink),
        ("Рамка · B", EditAction::BoxSelect),
        ("Нож · K", EditAction::Knife),
    ] {
        control(&mut commands, select, &interface.font, label, a);
    }
    text(
        &mut commands,
        panel,
        &interface.font,
        "ОПЕРАЦИИ",
        11.0,
        MUTED,
    );
    let group = node(
        &mut commands,
        panel,
        Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(3),
            ..row(3.0)
        },
    );
    for (label, op) in [
        ("Дублировать", Operation::Duplicate),
        ("Удалить", Operation::Delete),
        ("Только грани", Operation::DeleteFaces),
        ("Гладкое", Operation::ShadeSmooth),
        ("Плоское", Operation::ShadeFlat),
        ("Зеркало X", Operation::Mirror(0)),
        ("Зеркало Y", Operation::Mirror(1)),
        ("Зеркало Z", Operation::Mirror(2)),
        ("Объединить объекты", Operation::Join),
        ("Применить R/S", Operation::ApplyTransform),
        ("Центр по геометрии", Operation::OriginToGeometry),
    ] {
        control(
            &mut commands,
            group,
            &interface.font,
            label,
            EditAction::Operate(op),
        );
    }
    let row = node(
        &mut commands,
        panel,
        Node {
            flex_wrap: FlexWrap::Wrap,
            ..row(3.0)
        },
    );
    control(
        &mut commands,
        row,
        &interface.font,
        "Скрыть · H",
        EditAction::Hide,
    );
    control(
        &mut commands,
        row,
        &interface.font,
        "Показать · Alt+H",
        EditAction::Reveal,
    );
    commands.insert_resource(Ui {
        toolbar,
        panel,
        menu,
        add,
        stats,
        list: None,
        stamp: None,
        bound: None,
        draft: None,
    });
}

fn value(doc: &Document, field: Field) -> String {
    match field {
        Field::Name => doc.active().map_or(String::new(), |o| o.name.clone()),
        Field::Amount => format!("{:.4}", doc.amount),
        Field::Transform(index) => {
            let v = if doc.mode == Mode::Edit {
                doc.pivot().map_or(0.0, |v| v[index % 3])
            } else {
                doc.active().map_or(0.0, |o| match index / 3 {
                    0 => o.transform.translation[index % 3],
                    1 => {
                        let (x, y, z) = o.transform.rotation.to_euler(EulerRot::XYZ);
                        [x, y, z][index % 3].to_degrees()
                    }
                    _ => o.transform.scale[index % 3],
                })
            };
            format!("{v:.4}")
        }
    }
}

pub(in crate::editor) fn fields(
    mut state: ResMut<State>,
    mut ui: ResMut<Ui>,
    mut editor: ResMut<Editor>,
    mut focus: ResMut<InputFocus>,
    keys: Res<ButtonInput<KeyCode>>,
    mut inputs: Query<(Entity, &Field, &mut EditableText)>,
) {
    if let Some(mut draft) = ui.draft.take() {
        let cancel = keys.just_pressed(KeyCode::Escape) && editor.pending.is_none();
        let finish = focus.get() != Some(draft.entity)
            || keys.just_pressed(KeyCode::Enter)
            || ui.bound != editor.viewing_model;
        if let Some(doc) = state.documents.get_mut(&draft.index) {
            if cancel {
                doc.restore(draft.before);
                if let Ok((_, _, mut text)) = inputs.get_mut(draft.entity) {
                    text.editor_mut().set_text(&value(doc, draft.field));
                }
                focus.clear();
            } else {
                if let Ok((_, _, text)) = inputs.get(draft.entity) {
                    let typed = text.value().to_string();
                    if typed != draft.text {
                        draft.text = typed.clone();
                        doc.objects = draft.before.objects.clone();
                        let result = (|| {
                            if draft.field == Field::Name {
                                let name = typed.trim();
                                if name.is_empty() || name.len() > 256 {
                                    return Err("Имя должно содержать от 1 до 256 символов".into());
                                }
                                doc.active_mut().ok_or("Нет активного объекта")?.name = name.into();
                            } else {
                                let number = typed
                                    .trim()
                                    .replace(',', ".")
                                    .parse::<f32>()
                                    .ok()
                                    .filter(|v| v.is_finite())
                                    .ok_or("Введите корректное число")?;
                                match draft.field {
                                    Field::Amount => {
                                        if number <= 0.0 {
                                            return Err("Шаг должен быть больше нуля".into());
                                        }
                                        doc.amount = number;
                                    }
                                    Field::Transform(index) => {
                                        if doc.mode == Mode::Object {
                                            let o = doc.active_mut().ok_or("Нет объекта")?;
                                            match index / 3 {
                                                0 => o.transform.translation[index % 3] = number,
                                                1 => {
                                                    let (x, y, z) = o
                                                        .transform
                                                        .rotation
                                                        .to_euler(EulerRot::XYZ);
                                                    let mut e = [x, y, z];
                                                    e[index % 3] = number.to_radians();
                                                    o.transform.rotation = Quat::from_euler(
                                                        EulerRot::XYZ,
                                                        e[0],
                                                        e[1],
                                                        e[2],
                                                    );
                                                }
                                                _ => {
                                                    if number.abs() < 0.0001 {
                                                        return Err(
                                                            "Масштаб не может быть нулевым".into(),
                                                        );
                                                    }
                                                    o.transform.scale[index % 3] = number;
                                                }
                                            }
                                        } else {
                                            let pivot =
                                                doc.pivot().ok_or("Нет выделенных вершин")?;
                                            let mut delta = Vec3::ZERO;
                                            delta[index % 3] = number - pivot[index % 3];
                                            doc.transform(
                                                &draft.before,
                                                Mat4::from_translation(delta),
                                                None,
                                            );
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            doc.validate()
                        })();
                        draft.error = result.err();
                        doc.revision += 1;
                    }
                }
                if finish && draft.error.is_none() {
                    doc.checkpoint(draft.before);
                    if keys.just_pressed(KeyCode::Enter) {
                        focus.clear();
                    }
                } else {
                    if finish && editor.pending.is_none() {
                        focus.set(draft.entity, bevy::input_focus::FocusCause::Auto);
                    }
                    ui.draft = Some(draft);
                }
            }
        }
    }
    if ui.draft.is_none() && editor.pending.is_none() {
        if let Some(index) = ui.bound.filter(|i| Some(*i) == editor.viewing_model) {
            if let Some(doc) = state.documents.get(&index) {
                if let Some(entity) = focus.get() {
                    if let Ok((_, field, text)) = inputs.get(entity) {
                        let typed = text.value().to_string();
                        let expected = value(doc, *field);
                        if typed != expected {
                            ui.draft = Some(Draft {
                                index,
                                entity,
                                field: *field,
                                before: doc.snapshot(),
                                text: expected,
                                error: None,
                            });
                        }
                    }
                }
            }
        }
    }
    state.drafts = ui.draft.as_ref().map(|d| d.index).into_iter().collect();
    let previous_error = state.input_error.take();
    state.input_error = ui.draft.as_ref().and_then(|d| d.error.clone());
    if let Some(error) = &state.input_error {
        state.status = error.clone();
    } else if previous_error.is_some() {
        state.status.clear();
    }
    for (&index, doc) in &state.documents {
        if doc.dirty() || state.drafts.contains(&index) {
            editor.dirty_models.insert(index);
        } else {
            editor.dirty_models.remove(&index);
        }
    }
}

pub(in crate::editor) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    state: Res<State>,
    interface: Res<Interface>,
    mut ui: ResMut<Ui>,
    focus: Res<InputFocus>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<(&mut Text, &mut TextColor)>,
    mut inputs: Query<(Entity, &Field, &mut EditableText, &mut BorderColor)>,
    controls: Query<(Entity, &EditButton, &Children)>,
    entities: Query<()>,
) {
    let showing = editor.viewing_model.is_some()
        && editor.viewing_image.is_none()
        && editor.pending.is_none();
    let index = editor.viewing_model;
    let doc = index.and_then(|i| state.documents.get(&i));
    let editing = doc.is_some_and(|d| d.mode == Mode::Edit);
    for (entity, show) in [
        (
            ui.panel,
            showing && !doc.is_some_and(|d| matches!(d.mode, Mode::Uv | Mode::Paint)),
        ),
        (ui.toolbar, showing),
        (ui.menu, showing && state.menu),
        (ui.add, showing && state.add_menu),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.display = if show { Display::Flex } else { Display::None };
        }
    }
    for (entity, control, children) in &controls {
        let selected = state.active(control.0, index);
        let available = state.available(control.0, index) && editor.pending.is_none();
        if available {
            commands.entity(entity).try_remove::<InteractionDisabled>();
        } else {
            commands.entity(entity).try_insert(InteractionDisabled);
        }
        commands.entity(entity).try_insert((
            BackgroundColor(if selected { ACTIVE } else { Color::NONE }),
            BorderColor::all(if selected {
                FOCUS.with_alpha(0.7)
            } else {
                EDGE.with_alpha(0.3)
            }),
        ));
        for child in children.iter() {
            if let Ok((mut text, mut color)) = labels.get_mut(child) {
                if control.0 == EditAction::Menu {
                    text.0 = if doc.is_some_and(|d| d.mode == Mode::Paint) {
                        "Рисование"
                    } else if doc.is_some_and(|d| d.mode == Mode::Uv) {
                        "UV-развёртка"
                    } else if editing {
                        "Редактирование"
                    } else {
                        "Объект"
                    }
                    .into();
                }
                color.0 = if !available {
                    DISABLED_TEXT
                } else if selected {
                    FOCUS
                } else {
                    TEXT
                };
            }
        }
    }

    if let Ok((mut text, _)) = labels.get_mut(ui.stats) {
        text.0 = if let Some(doc) = doc {
            let vertices: usize = doc.objects.iter().map(|o| o.geometry.positions.len()).sum();
            let faces: usize = doc.objects.iter().map(|o| o.geometry.faces.len()).sum();
            let chosen = state.selection_label(index).unwrap_or_default();
            format!(
                "{} объектов · {vertices} вершин · {faces} граней{}{}",
                doc.objects.len(),
                if editing {
                    format!("\n{chosen}")
                } else {
                    String::new()
                },
                if doc.dirty() {
                    " · изменено"
                } else {
                    ""
                }
            )
        } else {
            index
                .and_then(|i| state.errors.get(&i))
                .cloned()
                .unwrap_or("Загрузка…".into())
        };
    }
    if let Some(doc) = doc {
        for (entity, field, mut text, mut border) in &mut inputs {
            let draft = ui.draft.as_ref().is_some_and(|d| d.entity == entity);
            if !draft && focus.get() != Some(entity) {
                let content = value(doc, *field);
                if text.value().to_string() != content {
                    text.editor_mut().set_text(&content);
                }
            }
            *border = BorderColor::all(if draft && state.input_error.is_some() {
                ERROR
            } else if focus.get() == Some(entity) {
                FOCUS
            } else {
                EDGE
            });
        }
        if let Ok((mut text, _)) = labels.get_mut(interface.kind) {
            text.0 = if editing {
                "Редактирование сетки"
            } else {
                "Объект · редактирование модели"
            }
            .into();
        }
        if let Some(object) = doc.active() {
            if let Ok((mut text, _)) = labels.get_mut(interface.selected) {
                text.0 = object.name.clone();
            }
        }
    }
    // These panes are deliberately independent of the scene inspector's controls.

    let stamp = doc.zip(index).map(|(doc, index)| (index, doc.ui_revision));
    if ui.stamp != stamp || ui.list.is_some_and(|e| !entities.contains(e)) || ui.list.is_none() {
        if let Some(old) = ui.list.take() {
            commands.entity(old).try_despawn();
        }
        ui.stamp = stamp;
        if let Some(doc) = doc.filter(|_| showing) {
            let list = node(&mut commands, interface.outliner, column(3.0));
            text(
                &mut commands,
                list,
                &interface.font,
                "ОБЪЕКТЫ МОДЕЛИ",
                11.0,
                MUTED,
            );
            for object in &doc.objects {
                let label = format!("{}{}", if object.hidden { "◌ " } else { "" }, object.name);
                control(
                    &mut commands,
                    list,
                    &interface.font,
                    &label,
                    EditAction::SelectObject(object.id),
                );
            }
            ui.list = Some(list);
        }
    }
    ui.bound = index;
}

pub(in crate::editor) fn groups(
    editor: Res<Editor>,
    state: Res<State>,
    mut groups: Query<(&Group, &mut Node)>,
) {
    let editing = editor.viewing_model.is_some()
        && editor.viewing_image.is_none()
        && editor.pending.is_none()
        && state.mode(editor.viewing_model) == Mode::Edit;
    for (group, mut node) in &mut groups {
        node.display = if group.0 == editing {
            Display::Flex
        } else {
            Display::None
        };
    }
}
