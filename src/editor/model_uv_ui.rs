use super::*;
use bevy::{render::render_resource::TextureFormat, ui::InteractionDisabled};

#[derive(Component)]
pub(super) struct UvButton(pub UvAction);
#[derive(Component)]
pub(super) struct UvCanvas;
#[derive(Resource)]
pub(super) struct UvUi {
    pub pane: Entity,
    pub canvas: Entity,
    pub image: Handle<Image>,
    pub inspector: Entity,
    toolbar: Entity,
    decision: Entity,
    accept: Entity,
    stats: Entity,
    status: Entity,
    empty: Entity,
    caption: Entity,
    progress: Entity,
    fill: Entity,
    activity: Entity,
    resolution: Entity,
    padding: Entity,
    detect_udim: Entity,
    auto: Entity,
    cancel: Entity,
    selection: Entity,
}
fn control(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    action: UvAction,
    primary: bool,
) -> Entity {
    let e = button(
        commands,
        parent,
        font,
        label,
        Action::ModelView(ModelViewAction::Uv(action)),
        if primary {
            Look::Primary
        } else {
            Look::Regular
        },
    );
    commands
        .entity(e)
        .insert((UvButton(action), AccessibleLabel(format!("UV: {label}"))));
    e
}
fn wrapping(commands: &mut Commands, parent: Entity) -> Entity {
    node(
        commands,
        parent,
        Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(5),
            ..row(5.)
        },
    )
}
fn paragraph(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    size: f32,
    color: Color,
) -> Entity {
    let e = text(commands, parent, font, label, size, color);
    commands.entity(e).insert(TextLayout::default());
    e
}
pub(super) fn setup(
    mut commands: Commands,
    interface: Res<Interface>,
    viewer: Res<ViewerUi>,
    mut images: ResMut<Assets<Image>>,
) {
    let toolbar = node(
        &mut commands,
        viewer.toolbar,
        Node {
            display: Display::None,
            ..row(5.)
        },
    );
    let auto = control(
        &mut commands,
        toolbar,
        &interface.font,
        "Авторазвёртка",
        UvAction::Unwrap,
        true,
    );
    let cancel = control(
        &mut commands,
        toolbar,
        &interface.font,
        "Отменить расчёт",
        UvAction::Cancel,
        false,
    );
    let pane = surface(
        &mut commands,
        viewer.content,
        Node {
            display: Display::None,
            width: percent(0),
            height: percent(100),
            min_width: px(0),
            flex_shrink: 0.,
            border: UiRect::left(px(1)),
            padding: UiRect {
                top: px(10),
                bottom: px(6),
                left: px(10),
                right: px(10),
            },
            ..column(7.)
        },
        Color::srgb(0.035, 0.045, 0.065),
    );
    commands.entity(pane).insert((
        BorderColor::all(EDGE),
        AccessibleLabel("Редактор UV-развёртки".into()),
    ));
    let header = wrapping(&mut commands, pane);
    text(
        &mut commands,
        header,
        &interface.font,
        "UV-КАРТА",
        12.,
        FOCUS,
    );
    control(
        &mut commands,
        header,
        &interface.font,
        "Вписать · F",
        UvAction::Fit,
        false,
    );
    control(
        &mut commands,
        header,
        &interface.font,
        "До / после",
        UvAction::Compare,
        false,
    );
    let caption = text(
        &mut commands,
        pane,
        &interface.font,
        "0–1 · 2048 × 2048",
        11.,
        MUTED,
    );
    let tools = wrapping(&mut commands, pane);
    for (label, action) in [
        ("Все · A", UvAction::SelectAll),
        ("Снять", UvAction::Deselect),
        ("90°", UvAction::Rotate),
        ("−", UvAction::ScaleDown),
        ("+", UvAction::ScaleUp),
        ("Уплотнить", UvAction::Pack),
    ] {
        control(&mut commands, tools, &interface.font, label, action, false);
    }
    let decision = wrapping(&mut commands, pane);
    let accept = control(
        &mut commands,
        decision,
        &interface.font,
        "Применить UV",
        UvAction::Accept,
        true,
    );
    control(
        &mut commands,
        decision,
        &interface.font,
        "Отклонить",
        UvAction::Dismiss,
        false,
    );
    let canvas = surface(
        &mut commands,
        pane,
        Node {
            width: percent(100),
            flex_grow: 1.,
            min_height: px(80),
            overflow: Overflow::clip(),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            ..default()
        },
        WELL,
    );
    commands.entity(canvas).insert((
        UvCanvas,
        BorderColor::all(EDGE),
        AccessibleLabel("UV-карта: выбор и перемещение островов".into()),
    ));
    let image = images.add(Image::new_target_texture(
        512,
        512,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    commands.spawn((
        ImageNode::new(image.clone()),
        Pickable::IGNORE,
        Node {
            width: percent(100),
            height: percent(100),
            ..default()
        },
        ChildOf(canvas),
    ));
    let empty = paragraph(
        &mut commands,
        canvas,
        &interface.font,
        "UV-карты пока нет\nНажмите «Авторазвёртка»",
        15.,
        MUTED,
    );
    commands.entity(empty).insert(Node {
        position_type: PositionType::Absolute,
        left: percent(8),
        right: percent(8),
        top: percent(42),
        ..default()
    });
    let selection = text(
        &mut commands,
        pane,
        &interface.font,
        "ЛКМ — остров · Shift — добавить",
        11.,
        MUTED,
    );
    let help = paragraph(
        &mut commands,
        pane,
        &interface.font,
        "Перетащите остров для перемещения · Колесо — масштаб\nСКМ / Пробел + ЛКМ — панорама · F — вписать",
        11.,
        MUTED,
    );
    commands.entity(help).insert(Pickable::IGNORE);
    let inspector = node(
        &mut commands,
        interface.inspector,
        Node {
            display: Display::None,
            width: percent(100),
            ..column(8.)
        },
    );
    let unwrap = inspector_section(
        &mut commands,
        inspector,
        &interface.font,
        "Развёртка",
        "PartUV Geometric автоматически выбирает швы и создаёт UV-острова. Нативный движок Rust + C++ входит в редактор и работает локально на CPU. Дополнительная установка не нужна.",
    );
    let unwrap = node(&mut commands, unwrap, column(5.0));
    inspector::searchable(
        &mut commands,
        unwrap,
        "развёртка авторазвёртка быстрый предпросмотр качество unwrap xatlas",
    );
    let preview_button = control(
        &mut commands,
        unwrap,
        &interface.font,
        "Быстрый предпросмотр",
        UvAction::Preview,
        false,
    );
    control_hints::attach(
        &mut commands,
        preview_button,
        "xatlas быстро строит UV. Осмотрите их и нажмите «Применить UV», чтобы добавить в историю модели.",
    );
    let choices = wrapping(&mut commands, unwrap);
    for (index, label) in ["Точнее", "Баланс", "Меньше швов"].into_iter().enumerate()
    {
        let e = control(
            &mut commands,
            choices,
            &interface.font,
            label,
            UvAction::Quality(index),
            false,
        );
        control_hints::attach(
            &mut commands,
            e,
            [
                "Меньше растяжения поверхности, больше швов. Подходит для заметных деталей и крупных планов.",
                "Баланс между растяжением и количеством швов. Рекомендуемый вариант для первой развёртки.",
                "Меньше разрезов и более крупные острова. Допускает большее растяжение поверхности.",
            ][index],
        );
    }
    let atlas = inspector_section(
        &mut commands,
        inspector,
        &interface.font,
        "Атлас",
        "Параметры следующей развёртки или упаковки. Размер задаёт целевое разрешение, отступ разделяет острова. Готовая UV-карта нормализована в диапазон 0–1.",
    );
    let size_hint = "Размер целевой текстуры: 1024², 2048² или 4096². Нажмите значение для переключения. Используется при следующей развёртке или упаковке; текстура автоматически не создаётся.";
    let options = property_row(&mut commands, atlas, &interface.font, "Размер", size_hint);
    let resolution = control(
        &mut commands,
        options,
        &interface.font,
        "2048²",
        UvAction::Resolution,
        false,
    );
    control_hints::attach(&mut commands, resolution, size_hint);
    let padding_hint = "Зазор между UV-островами в пикселях: 4, 8 или 16. Нажмите значение для переключения. Увеличьте его, если текстура растекается на соседние острова.";
    let options = property_row(
        &mut commands,
        atlas,
        &interface.font,
        "Отступ",
        padding_hint,
    );
    let padding = control(
        &mut commands,
        options,
        &interface.font,
        "8 px",
        UvAction::Padding,
        false,
    );
    control_hints::attach(&mut commands, padding, padding_hint);
    for e in [resolution, padding] {
        commands.entity(e).entry::<Node>().and_modify(|mut n| {
            n.min_width = px(82);
            n.justify_content = JustifyContent::Center;
        });
    }
    let stats = paragraph(
        &mut commands,
        atlas,
        &interface.font,
        "Выберите объект",
        12.,
        MUTED,
    );
    control_hints::attach(
        &mut commands,
        stats,
        "Число UV-островов и сумма площадей их треугольников в атласе. Это не проверка перекрытий: при наложенных островах площадь может превышать 100%. Грани за пределами 0–1 отмечаются предупреждением.",
    );
    let import = inspector_section(
        &mut commands,
        inspector,
        &interface.font,
        "Импорт модели",
        "Параметры открытия внешних моделей.",
    );
    let options = property_row(
        &mut commands,
        import,
        &interface.font,
        "Распознавать UDIM",
        UDIM_IMPORT_HINT,
    );
    let detect_udim = control(
        &mut commands,
        options,
        &interface.font,
        "Вкл.",
        UvAction::DetectUdim,
        false,
    );
    commands
        .entity(detect_udim)
        .insert(AccessibleLabel("Распознавать UDIM при импорте".into()));
    control_hints::attach(&mut commands, detect_udim, UDIM_IMPORT_HINT);
    let preview = inspector_section(
        &mut commands,
        inspector,
        &interface.font,
        "Просмотр",
        "Проверка UV на модели. Шахматная текстура и подсветка швов меняют только предпросмотр; исходные материалы сохраняются.",
    );
    let preview = wrapping(&mut commands, preview);
    inspector::searchable(
        &mut commands,
        preview,
        "шахматка швы растяжение checker seams stretch",
    );
    control(
        &mut commands,
        preview,
        &interface.font,
        "Шахматка",
        UvAction::Checker,
        false,
    );
    control(
        &mut commands,
        preview,
        &interface.font,
        "Швы",
        UvAction::Seams,
        false,
    );
    control(
        &mut commands,
        preview,
        &interface.font,
        "Растяжение",
        UvAction::Heatmap,
        false,
    );
    let seams = inspector_section(
        &mut commands,
        inspector,
        &interface.font,
        "Размещение швов",
        "Авторазвёртка учитывает границы материалов, жёсткие изгибы и отмеченные области.",
    );
    let seam_tools = wrapping(&mut commands, seams);
    inspector::searchable(
        &mut commands,
        seam_tools,
        "беречь швы предложить остров очистить защиту seam",
    );
    for (label, action, hint) in [
        (
            "Беречь от швов",
            UvAction::Brush,
            "Кисть по поверхности. ЛКМ отмечает область, Shift + ЛКМ стирает. Разрезы предпочтительны по её границе.",
        ),
        (
            "Предложить остров",
            UvAction::Suggest,
            "DA Wand предложит связную область вокруг грани, по которой вы щёлкнете. Результат можно принять или отклонить.",
        ),
        (
            "Очистить защиту",
            UvAction::ClearMask,
            "Снять всю защиту швов с активного объекта.",
        ),
    ] {
        let button = control(
            &mut commands,
            seam_tools,
            &interface.font,
            label,
            action,
            false,
        );
        control_hints::attach(&mut commands, button, hint);
    }
    let changes = inspector_section(
        &mut commands,
        inspector,
        &interface.font,
        "Изменения",
        "UV изменяются в документе. Ctrl+Z отменяет последнее действие, Ctrl+Shift+Z возвращает его. Ctrl+S сохраняет основной UV-канал в исходный GLB.",
    );
    let save = wrapping(&mut commands, changes);
    inspector::searchable(
        &mut commands,
        save,
        "сохранить отменить повторить save undo redo",
    );
    for (label, action) in [
        ("Сохранить", EditAction::Save),
        ("↶", EditAction::Undo),
        ("↷", EditAction::Redo),
    ] {
        let e = if action == EditAction::Save {
            button(
                &mut commands,
                save,
                &interface.font,
                label,
                Action::ModelView(ModelViewAction::Edit(action)),
                Look::Regular,
            )
        } else {
            icons::button(
                &mut commands,
                save,
                if action == EditAction::Undo {
                    icons::Glyph::Undo
                } else {
                    icons::Glyph::Redo
                },
                if action == EditAction::Undo {
                    "Отменить"
                } else {
                    "Повторить"
                },
                Action::ModelView(ModelViewAction::Edit(action)),
                Look::Regular,
            )
        };
        commands.entity(e).insert((
            controls::EditButton(action),
            AccessibleLabel(format!(
                "UV: {}",
                match action {
                    EditAction::Undo => "Отменить",
                    EditAction::Redo => "Повторить",
                    _ => label,
                }
            )),
        ));
        control_hints::attach(
            &mut commands,
            e,
            match action {
                EditAction::Save => {
                    "Ctrl+S · Сохранить UV в исходный GLB. Геометрия, материалы и второй UV-канал сохраняются."
                }
                EditAction::Undo => {
                    "Ctrl+Z · Отменить последнее изменение UV. Вся авторазвёртка или перетаскивание острова отменяются одним шагом."
                }
                _ => "Ctrl+Shift+Z / Ctrl+Y · Вернуть отменённое изменение UV.",
            },
        );
    }
    let progress = node(
        &mut commands,
        unwrap,
        Node {
            display: Display::None,
            ..column(7.)
        },
    );
    let track = surface(
        &mut commands,
        progress,
        Node {
            width: percent(100),
            height: px(5),
            overflow: Overflow::clip(),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        WELL,
    );
    let fill = surface(
        &mut commands,
        track,
        Node {
            height: percent(100),
            width: percent(0),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        FOCUS,
    );
    let activity = text(&mut commands, progress, &interface.font, "", 12., MUTED);
    let status = paragraph(
        &mut commands,
        changes,
        &interface.font,
        "Нет изменений",
        12.,
        MUTED,
    );
    control_hints::attach(
        &mut commands,
        status,
        "Изменения UV сохраняются только по команде Ctrl+S. Ошибки расчёта не меняют исходную модель.",
    );
    commands.insert_resource(UvUi {
        pane,
        canvas,
        image: image.clone(),
        inspector,
        toolbar,
        decision,
        accept,
        stats,
        status,
        empty,
        caption,
        progress,
        fill,
        activity,
        resolution,
        padding,
        detect_udim,
        auto,
        cancel,
        selection,
    });
    view::setup(&mut commands, image);
}

fn property_row(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    title: &str,
    hint: &str,
) -> Entity {
    let line = node(
        commands,
        parent,
        Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            ..row(8.)
        },
    );
    inspector::searchable(commands, line, title);
    let title = text(commands, line, font, title, 12., MUTED);
    control_hints::attach(commands, title, hint);
    line
}

fn label(
    entity: Entity,
    value: &str,
    children: &Query<&Children>,
    texts: &mut Query<(&mut Text, &mut TextColor)>,
) {
    if let Ok(children) = children.get(entity) {
        for child in children.iter() {
            if let Ok((mut text, _)) = texts.get_mut(child) {
                if text.0 != value {
                    text.0 = value.into();
                }
            }
        }
    }
}
pub(super) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    editing: Res<State>,
    uv: Res<UvState>,
    ui: Res<UvUi>,
    interface: Res<Interface>,
    viewer: Res<ViewerUi>,
    time: Res<Time>,
    mut nodes: Query<&mut Node>,
    mut texts: Query<(&mut Text, &mut TextColor)>,
    children: Query<&Children>,
    controls: Query<(Entity, &UvButton)>,
    mut backgrounds: Query<&mut BackgroundColor>,
) {
    let doc = editor.viewing_model.and_then(|i| editing.documents.get(&i));
    let showing = doc.is_some_and(|d| d.mode == Mode::Uv)
        && editor.pending.is_none()
        && editor.viewing_image.is_none();
    for (entity, visible) in [
        (ui.pane, showing || uv.split > 0.002),
        (ui.inspector, showing),
        (ui.toolbar, showing),
        (ui.decision, showing && uv.proposal.is_some()),
        (ui.cancel, showing && (uv.job.is_some() || uv.relocation.is_some())),
        (ui.progress, showing && uv.job.is_some()),
        (ui.empty, showing && uv.atlas.area < 1e-9),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.display = if visible {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    if let Ok(mut pane) = nodes.get_mut(ui.pane) {
        pane.width = percent(uv.split * 49.);
    }
    if let Ok(mut toolbar) = nodes.get_mut(viewer.toolbar) {
        toolbar.right = if showing {
            percent(uv.split * 49. + 1.)
        } else {
            px(140)
        };
    }
    if !showing {
        return;
    }
    for (entity, control) in &controls {
        let available = uv.available(control.0, editor.viewing_model, &editing);
        if available {
            commands.entity(entity).try_remove::<InteractionDisabled>();
        } else {
            commands.entity(entity).try_insert(InteractionDisabled);
        }
        if uv.active(control.0) {
            commands.entity(entity).try_insert((
                BackgroundColor(ACTIVE),
                BorderColor::all(FOCUS.with_alpha(0.65)),
            ));
        }
    }
    label(
        ui.accept,
        if matches!(
            uv.proposal.as_ref().map(|p| &p.result),
            Some(Proposed::Island(_))
        ) {
            "Принять область"
        } else {
            "Применить UV"
        },
        &children,
        &mut texts,
    );
    if let Ok((mut text, _)) = texts.get_mut(ui.stats) {
        text.0 = doc.and_then(|d| d.active()).map_or_else(
            || "Выберите объект модели".into(),
            |_o| {
                let mut stats = format!(
                    "{} островов · UV {:0.1}%{}",
                    uv.atlas.count,
                    uv.atlas.area * 100.,
                    if uv.atlas.outside > 0 {
                        format!("\nВне атласа: {} граней", uv.atlas.outside)
                    } else {
                        String::new()
                    }
                );
                if uv.heatmap
                    && let Some(r) = uv.atlas.angular_p95
                {
                    stats.push_str(&format!(
                        "\nИскажение P95 · {}\nБирюзовый 1× · жёлтый 1,5× · красный ≥2×",
                        if r.is_finite() {
                            format!("{r:.2}×")
                        } else {
                            "∞".into()
                        }
                    ));
                }
                stats
            },
        );
    }
    if let Ok((mut text, mut color)) = texts.get_mut(ui.status) {
        let content = if !uv.error.is_empty() {
            &uv.error
        } else if !uv.status.is_empty() && uv.job.is_none() {
            &uv.status
        } else if doc.is_some_and(|d| d.dirty()) {
            "Есть изменения"
        } else {
            "Нет изменений"
        };
        text.0 = content.into();
        color.0 = if !uv.error.is_empty() {
            ERROR
        } else if uv.reveal > 0.01 {
            FOCUS
        } else {
            MUTED
        };
    }
    if let Ok((mut text, _)) = texts.get_mut(ui.caption) {
        text.0 = format!(
            "{} · {} × {} · отступ {} px",
            if matches!(
                uv.proposal.as_ref().map(|p| &p.result),
                Some(Proposed::Layout(_))
            ) && !uv.comparing
            {
                "Предпросмотр xatlas"
            } else if uv.comparing {
                "Исходная UV"
            } else if doc.and_then(|d| d.active()).is_some_and(|o| o.udim_tiles().next().is_some()) {
                "Атлас UDIM"
            } else {
                "Атлас 0–1"
            },
            uv.resolution,
            uv.resolution,
            uv.padding
        );
    }
    if let Ok((mut text, _)) = texts.get_mut(ui.selection) {
        text.0 = if let Some(Proposal {
            result: Proposed::Island(mask),
            ..
        }) = &uv.proposal
        {
            format!(
                "DA Wand · {} треугольников · фиолетовая область",
                mask.len()
            )
        } else if uv.tool == Tool::Wand {
            "DA Wand · щёлкните по грани в 3D".into()
        } else if uv.tool == Tool::Brush {
            "Защита швов · рисуйте в 3D · Shift — стереть".into()
        } else if uv.selected.is_empty() {
            "ЛКМ — остров · Shift — добавить".into()
        } else if uv.drag.as_ref().is_some_and(|drag| drag.tile_delta.is_some()) {
            "Отпустите, чтобы перенести острова и их рисование на другую плитку".into()
        } else {
            format!(
                "Выделено островов: {} · перетащите для перемещения",
                uv.selected.len()
            )
        };
    }
    if let Ok((mut text, _)) = texts.get_mut(ui.activity) {
        text.0 = format!("{} · {:0.0} с", uv.status, uv.elapsed);
    }
    if let Ok(mut fill) = nodes.get_mut(ui.fill) {
        fill.width = percent(if uv.progress < 0.05 {
            20.
        } else {
            uv.progress * 100.
        });
    }
    if let Ok(mut background) = backgrounds.get_mut(ui.fill) {
        background.0 = FOCUS.with_alpha(0.65 + 0.35 * (time.elapsed_secs() * 3.).sin().abs());
    }
    label(
        ui.resolution,
        &format!("{}²", uv.resolution),
        &children,
        &mut texts,
    );
    label(
        ui.padding,
        &format!("{} px", uv.padding),
        &children,
        &mut texts,
    );
    label(
        ui.detect_udim,
        if uv.detect_udim_on_import() { "Вкл." } else { "Выкл." },
        &children,
        &mut texts,
    );
    label(
        ui.auto,
        if uv
            .job
            .as_ref()
            .is_some_and(|j| j.action == UvAction::Unwrap)
        {
            "Разворачиваем…"
        } else {
            "Авторазвёртка"
        },
        &children,
        &mut texts,
    );
    if let Ok((mut text, _)) = texts.get_mut(interface.kind) {
        text.0 = "UV-развёртка".into();
    }
}
