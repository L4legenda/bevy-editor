//! Property controls: sliders, colour swatches and translated choices, not a parameter dump.
use super::super::ux;
use super::*;
use bevy::ui_widgets::SliderThumb;

#[derive(Component)]
pub(in crate::editor) struct NumberSlider {
    index: usize,
    readout: Entity,
    thumb: Entity,
    step: f64,
}

fn caption(form: &Form, field: &super::super::forms::Field) -> String {
    if field.key.rsplit('/').next() == Some("size") {
        let kind = form
            .effect
            .and_then(|index| form.params["effects"][index]["kind"].as_str())
            .unwrap_or_else(|| form.command.rsplit('.').next().unwrap_or_default());
        match kind {
            "stroke" => return "Толщина обводки, пикс.".into(),
            "dropShadow" | "innerShadow" | "outerGlow" | "innerGlow" => {
                return "Размытие края, пикс.".into();
            }
            _ => {}
        }
    }
    if form.effect.is_some() {
        field
            .label
            .split_once(" · ")
            .map_or(field.label.as_str(), |(_, label)| label)
            .into()
    } else {
        field.label.clone()
    }
}

fn number_change(
    event: On<ValueChange<f32>>,
    sliders: Query<&NumberSlider>,
    mut actions: MessageWriter<Action>,
) {
    if let Ok(slider) = sliders.get(event.source) {
        let value = (f64::from(event.value) / slider.step).round() * slider.step;
        actions.write(act(LayerAction::FieldNumber(slider.index, value)));
    }
}

pub(in crate::editor) fn sync(
    mut commands: Commands,
    viewer: Res<ImageViewer>,
    sliders: Query<(Entity, &NumberSlider, &SliderValue, &SliderRange)>,
    mut labels: Query<&mut Text>,
    mut nodes: Query<&mut Node>,
) {
    let Some(form) = viewer.edit.layers.form.as_ref() else {
        return;
    };
    for (entity, slider, current, range) in &sliders {
        if let Some(value) = form
            .fields
            .get(slider.index)
            .and_then(|f| f.text.parse::<f32>().ok())
        {
            if current.0 != value {
                commands.entity(entity).insert(SliderValue(value));
            }
            if let Ok(mut text) = labels.get_mut(slider.readout) {
                text.set_if_neq(Text(if slider.step >= 1.0 {
                    format!("{value:.0}")
                } else {
                    format!("{value:.2}")
                }));
            }
            if let Ok(mut thumb) = nodes.get_mut(slider.thumb) {
                thumb.left = percent(
                    ((value - range.start()) / range.span().max(0.0001)).clamp(0.0, 1.0) * 94.0,
                );
            }
        }
    }
}

fn numeric(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    form: &Form,
    index: usize,
) {
    let field = &form.fields[index];
    let caption = caption(form, field);
    let current = field.text.parse::<f64>().unwrap_or(0.0);
    let (min, max, step) = ux::number(form.command, &field.key, current);
    let header = node(
        commands,
        parent,
        Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            ..row(4.0)
        },
    );
    let label = text(commands, header, font, &caption, 12.0, TEXT);
    ux::help(commands, label, ux::field_help(&field.key));
    let buttons = node(commands, header, row(2.0));
    compact(
        commands,
        buttons,
        font,
        "−",
        LayerAction::FieldStep(index, -step),
    );
    let readout = text(commands, buttons, font, &field.text, 12.0, TEXT);
    commands.entity(readout).insert(Node {
        width: px(45),
        justify_content: JustifyContent::Center,
        ..default()
    });
    compact(
        commands,
        buttons,
        font,
        "+",
        LayerAction::FieldStep(index, step),
    );
    let slider = commands
        .spawn((
            Slider {
                track_click: TrackClick::Snap,
                ..default()
            },
            SliderRange::new(min as f32, max as f32),
            bevy::ui_widgets::SliderStep(step as f32),
            SliderValue(current as f32),
            TabIndex(0),
            AccessibleLabel(caption),
            Node {
                width: percent(100),
                height: px(24),
                flex_shrink: 0.0,
                ..default()
            },
            ChildOf(parent),
        ))
        .observe(number_change)
        .id();
    ux::help(commands, slider, ux::field_help(&field.key));
    let track = surface(
        commands,
        slider,
        Node {
            position_type: PositionType::Absolute,
            left: px(7),
            right: px(7),
            top: px(10),
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
            left: percent(((current - min) / (max - min).max(0.001)) as f32 * 94.0),
            top: px(5),
            width: px(14),
            height: px(14),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        WELL,
    );
    commands
        .entity(thumb)
        .insert((SliderThumb, Pickable::IGNORE, BorderColor::all(FOCUS)));
    commands.entity(slider).insert(NumberSlider {
        index,
        readout,
        thumb,
        step,
    });
}

pub(super) fn build(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    state: &LayerState,
    focus: &mut InputFocus,
) -> Entity {
    let form = state.form.as_ref().unwrap();
    let scroll = scroll_node(
        commands,
        parent,
        Node {
            width: percent(100),
            min_height: px(0),
            max_height: px(520),
            overflow: Overflow::scroll_y(),
            ..column(12.0)
        },
    );
    let has_primary = form
        .fields
        .iter()
        .any(|f| ux::primary(form.command, &f.key));
    let mut hidden = 0;
    for (index, field) in form.fields.iter().enumerate() {
        let caption = caption(form, field);
        if form
            .effect
            .is_some_and(|effect| !field.key.starts_with(&format!("/effects/{effect}/")))
        {
            continue;
        }
        if !state.advanced && has_primary && !ux::primary(form.command, &field.key) {
            hidden += 1;
            continue;
        }
        let group = node(
            commands,
            scroll,
            Node {
                width: percent(100),
                ..column(4.0)
            },
        );
        if matches!(field.kind, FieldKind::Number) {
            numeric(commands, group, font, form, index);
            continue;
        }
        if let FieldKind::Choice(choices) = &field.kind {
            if choices.iter().all(Value::is_boolean) {
                let checked = field.text == "true";
                let button = compact(
                    commands,
                    group,
                    font,
                    &caption,
                    LayerAction::FieldChoice(index, if checked { 0 } else { 1 }),
                );
                commands
                    .entity(button)
                    .entry::<Node>()
                    .and_modify(|mut n| n.width = percent(100));
                let icon = icons::themed(
                    commands,
                    button,
                    if checked {
                        icons::Glyph::Check
                    } else {
                        icons::Glyph::Rectangle
                    },
                    16.0,
                );
                commands.entity(button).insert_children(0, &[icon]);
                ux::help(commands, button, ux::field_help(&field.key));
            } else {
                text(commands, group, font, &caption, 12.0, TEXT);
                if choices.len() <= 5 {
                    let row = node(
                        commands,
                        group,
                        Node {
                            width: percent(100),
                            flex_wrap: FlexWrap::Wrap,
                            ..row(4.0)
                        },
                    );
                    for (choice, value) in choices.iter().enumerate() {
                        let label = ux::choice(
                            &value
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| value.to_string()),
                        );
                        compact(
                            commands,
                            row,
                            font,
                            &label,
                            LayerAction::FieldChoice(index, choice),
                        );
                    }
                } else {
                    let choose = dropdown(
                        commands,
                        group,
                        font,
                        &ux::choice(&field.text),
                        LayerAction::FieldChoices(index),
                    );
                    ux::help(commands, choose, ux::field_help(&field.key));
                    if form.choice == Some(index) {
                        let options = scroll_node(
                            commands,
                            group,
                            Node {
                                width: percent(100),
                                max_height: px(180),
                                overflow: Overflow::scroll_y(),
                                ..column(2.0)
                            },
                        );
                        for (choice, value) in choices.iter().enumerate() {
                            let raw = value
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| value.to_string());
                            compact(
                                commands,
                                options,
                                font,
                                &ux::choice(&raw),
                                LayerAction::FieldChoice(index, choice),
                            );
                        }
                    }
                }
            }
        } else if matches!(field.kind, FieldKind::Color) {
            text(commands, group, font, &caption, 12.0, TEXT);
            let choose = button(
                commands,
                group,
                font,
                "Выбрать цвет…",
                Action::ImageEdit(EditAction::ColorPicker(ColorTarget::LayerField(index))),
                Look::Regular,
            );
            let color = parse_color(&field.text).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let swatch = surface(
                commands,
                choose,
                Node {
                    width: px(28),
                    height: px(22),
                    flex_shrink: 0.0,
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
                Color::srgba(color[0], color[1], color[2], color[3]),
            );
            commands
                .entity(swatch)
                .insert((BorderColor::all(MUTED), Pickable::IGNORE));
            commands.entity(choose).insert_children(0, &[swatch]);
            ux::help(commands, choose, ux::field_help(&field.key));
        } else if let FieldKind::Item { .. } = field.kind {
            let header = node(
                commands,
                group,
                Node {
                    width: percent(100),
                    justify_content: JustifyContent::SpaceBetween,
                    ..row(4.0)
                },
            );
            text(commands, header, font, &caption, 12.0, FOCUS);
            let actions = node(commands, header, row(2.0));
            for (label, op) in [("↑", -1), ("↓", 1), ("Удалить", 0)] {
                compact(
                    commands,
                    actions,
                    font,
                    label,
                    LayerAction::FieldItem(index, op),
                );
            }
        } else if matches!(field.kind, FieldKind::Compound(_)) {
            compact(
                commands,
                group,
                font,
                &format!("Настроить: {caption}"),
                LayerAction::FieldComplex(index),
            );
        } else if matches!(field.kind, FieldKind::List(_)) {
            text(commands, group, font, &caption, 12.0, TEXT);
            compact(
                commands,
                group,
                font,
                "Добавить точку",
                LayerAction::FieldArray(index, true),
            );
        } else {
            text(commands, group, font, &caption, 12.0, TEXT);
            let input = super::super::super::interface::input(commands, group, font, &caption);
            let mut value = EditableText::new(&field.text);
            if form.command == "layer.renameLayer" {
                value.queue_edit(bevy::text::TextEdit::SelectAll);
                focus.set(input, bevy::input_focus::FocusCause::Auto);
            }
            commands.entity(input).insert((FormInput(index), value));
            ux::help(commands, input, ux::field_help(&field.key));
        }
    }
    if hidden > 0 || state.advanced {
        compact(
            commands,
            scroll,
            font,
            if state.advanced {
                "Скрыть дополнительные настройки"
            } else {
                "Дополнительные настройки"
            },
            LayerAction::Advanced,
        );
    }
    scroll
}
