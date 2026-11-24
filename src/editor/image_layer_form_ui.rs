//! Property controls: sliders, colour swatches and translated choices, not a parameter dump.
use super::super::ux;
use super::*;
use bevy::ui_widgets::SliderThumb;
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

#[derive(Asset, TypePath, AsBindGroup, Clone, PartialEq)]
pub(in crate::editor) struct ParameterMaterial {
    // Current and neutral positions, spectrum style, pixel density.
    #[uniform(0)]
    state: Vec4,
    #[uniform(1)]
    low: Vec4,
    #[uniform(2)]
    high: Vec4,
}

impl UiMaterial for ParameterMaterial {
    fn fragment_shader() -> ShaderRef {
        AssetPath::from_path_buf(embedded_path!("image_layer_parameter.wesl"))
            .with_source("embedded")
            .into()
    }
}

pub(super) fn install(app: &mut App) {
    embedded_asset!(app, "image_layer_parameter.wesl");
    app.add_plugins(UiMaterialPlugin::<ParameterMaterial>::default());
}

const THUMB: f32 = 14.0;

#[derive(Component)]
pub(in crate::editor) struct NumberSlider {
    index: usize,
    readout: Entity,
    thumb: Entity,
    track: Entity,
    direction: Option<Entity>,
    neutral: f32,
    spectrum: u8,
    endpoints: [Vec4; 2],
    step: f64,
}

fn caption(form: &Form, field: &super::super::forms::Field) -> String {
    if let Some((_, axis)) = ux::balance_axis(form, &field.key) {
        return ["Голубой ↔ красный", "Пурпурный ↔ зелёный", "Жёлтый ↔ синий"][axis].into();
    }
    if form.adjustment() == Some("levels") {
        match field.key.as_str() {
            "/inBlack" => return "Чёрная точка".into(),
            "/inWhite" => return "Белая точка".into(),
            "/outBlack" => return "Выходной чёрный".into(),
            "/outWhite" => return "Выходной белый".into(),
            _ => {}
        }
    }
    if field.key.rsplit('/').next() == Some("angle")
        && (form.command.ends_with("Shadow")
            || form.command.ends_with("bevelEmboss")
            || form.effect.is_some_and(|i| {
                matches!(
                    form.params["effects"][i]["kind"].as_str(),
                    Some("dropShadow" | "innerShadow" | "bevelEmboss")
                )
            }))
    {
        return "Направление света, °".into();
    }
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
    sliders: Query<(
        Entity,
        &NumberSlider,
        &SliderValue,
        &SliderRange,
        &ComputedNode,
    )>,
    tracks: Query<&MaterialNode<ParameterMaterial>>,
    materials: Option<ResMut<Assets<ParameterMaterial>>>,
    mut labels: Query<&mut Text>,
    mut nodes: Query<&mut Node>,
    mut transforms: Query<&mut UiTransform>,
) {
    let Some(form) = viewer.edit.layers.form.as_ref() else {
        return;
    };
    let mut materials = materials;
    for (entity, slider, current, range, computed) in &sliders {
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
            if let Some(direction) = slider.direction
                && let Ok(mut transform) = transforms.get_mut(direction)
            {
                transform.set_if_neq(UiTransform::from_rotation(Rot2::radians(
                    (90.0 - value).to_radians(),
                )));
            }
            let normalized =
                |v: f32| ((v - range.start()) / range.span().max(0.0001)).clamp(0.0, 1.0);
            let position = normalized(value);
            if let Ok(mut thumb) = nodes.get_mut(slider.thumb) {
                let travel = (computed.size().x * computed.inverse_scale_factor() - THUMB).max(0.0);
                let left = px(position * travel);
                if thumb.left != left {
                    thumb.left = left;
                }
            }
            if let Some(materials) = materials.as_deref_mut() {
                let data = ParameterMaterial {
                    state: Vec4::new(
                        position,
                        normalized(slider.neutral),
                        f32::from(slider.spectrum),
                        computed.inverse_scale_factor().recip(),
                    ),
                    low: slider.endpoints[0],
                    high: slider.endpoints[1],
                };
                if let Ok(handle) = tracks.get(slider.track) {
                    if materials
                        .get(&handle.0)
                        .is_some_and(|material| *material != data)
                        && let Some(mut material) = materials.get_mut(&handle.0)
                    {
                        *material = data;
                    }
                } else {
                    commands
                        .entity(slider.track)
                        .insert(MaterialNode(materials.add(data)));
                }
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
    let (min, max, step) = ux::number(form, index).expect("numeric field is present");
    let key = field.key.rsplit('/').next().unwrap_or_default();
    let neutral = if key == "gamma" {
        1.0
    } else if min < 0.0 && max > 0.0 {
        0.0
    } else {
        min
    };
    let (spectrum, colors) = if let Some((_, axis)) = ux::balance_axis(form, &field.key) {
        let pairs = [
            [Color::srgb(0.26, 0.72, 0.78), Color::srgb(0.92, 0.31, 0.34)],
            [Color::srgb(0.79, 0.42, 0.77), Color::srgb(0.43, 0.77, 0.51)],
            [Color::srgb(0.94, 0.76, 0.38), Color::srgb(0.43, 0.57, 0.92)],
        ];
        (1, pairs[axis])
    } else if key == "hue" {
        (2, [EDGE, FOCUS])
    } else if matches!(
        key,
        "brightness"
            | "lightness"
            | "exposure"
            | "gamma"
            | "inBlack"
            | "inWhite"
            | "outBlack"
            | "outWhite"
    ) {
        (
            1,
            [Color::srgb(0.25, 0.25, 0.29), Color::srgb(0.75, 0.76, 0.80)],
        )
    } else {
        (0, [EDGE, FOCUS])
    };
    let header = node(
        commands,
        parent,
        Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            ..row(4.0)
        },
    );
    let label_group = node(commands, header, row(6.0));
    let direction = (caption == "Направление света, °").then(|| {
        let arrow = icons::spawn(commands, label_group, icons::Glyph::ArrowUp, 15.0, FOCUS);
        commands.entity(arrow).insert(UiTransform::default());
        arrow
    });
    let label = text(commands, label_group, font, &caption, 12.0, TEXT);
    ux::help(commands, label, ux::field_help(&field.key));
    let buttons = node(commands, header, row(2.0));
    compact(
        commands,
        buttons,
        font,
        "−",
        LayerAction::FieldStep(index, -step),
    );
    let readout_parent = if neutral > min && neutral < max {
        let reset = compact(
            commands,
            buttons,
            font,
            "",
            LayerAction::FieldNumber(index, neutral),
        );
        commands
            .entity(reset)
            .insert(AccessibleLabel(format!("Сбросить: {caption}")));
        ux::help(
            commands,
            reset,
            format!("Вернуть нейтральное значение ({neutral})."),
        );
        reset
    } else {
        buttons
    };
    let readout = text(commands, readout_parent, font, &field.text, 12.0, TEXT);
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
            left: px(THUMB * 0.5),
            right: px(THUMB * 0.5),
            top: px(6),
            height: px(12),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        Color::NONE,
    );
    commands.entity(track).insert(Pickable::IGNORE);
    let thumb = surface(
        commands,
        slider,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(5),
            width: px(THUMB),
            height: px(THUMB),
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
        track,
        direction,
        neutral: neutral as f32,
        spectrum,
        endpoints: colors.map(|color| Vec4::from_array(color.to_linear().to_f32_array())),
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
    if form.adjustment() == Some("colorBalance") {
        let tabs = node(
            commands,
            scroll,
            Node {
                width: percent(100),
                ..row(4.0)
            },
        );
        for (tone, (_, label)) in ux::TONES.iter().enumerate() {
            let tab = button(
                commands,
                tabs,
                font,
                *label,
                act(LayerAction::BalanceTone(tone)),
                Look::Tool,
            );
            commands.entity(tab).entry::<Node>().and_modify(|mut n| {
                n.flex_grow = 1.0;
                n.min_width = px(0);
            });
        }
    }
    let has_primary = form.fields.iter().any(|f| ux::primary(form, &f.key));
    let mut hidden = 0;
    for (index, field) in form.fields.iter().enumerate() {
        if !form.stroke_field_visible(&field.key) {
            continue;
        }
        if ux::balance_axis(form, &field.key).is_some_and(|(tone, _)| tone != state.balance_tone) {
            continue;
        }
        let caption = caption(form, field);
        if form
            .effect
            .is_some_and(|effect| !field.key.starts_with(&format!("/effects/{effect}/")))
        {
            continue;
        }
        if !state.advanced && has_primary && !ux::primary(form, &field.key) {
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
                if choices.len() <= 5
                    && !(form.is_stroke_field(&field.key)
                        && field.key.rsplit('/').next() == Some("style"))
                {
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
                        let mut label = ux::choice(
                            &value
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| value.to_string()),
                        );
                        if field.key.rsplit('/').next() == Some("strokePaint") && value == "solid" {
                            label = "Цвет".into();
                        }
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
            let choose = control(
                commands,
                group,
                Action::ImageEdit(EditAction::ColorPicker(ColorTarget::LayerField(index))),
                Look::Regular,
                Node {
                    width: percent(100),
                    height: px(32),
                    padding: UiRect::horizontal(px(7)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(6)),
                    ..row(7.0)
                },
            );
            commands
                .entity(choose)
                .insert(AccessibleLabel(format!("Выбрать цвет: {caption}")));
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
            let value = text(
                commands,
                choose,
                font,
                field.text.to_ascii_uppercase(),
                12.0,
                TEXT,
            );
            commands
                .entity(value)
                .entry::<Node>()
                .and_modify(|mut n| n.flex_grow = 1.0);
            icons::themed(commands, choose, icons::Glyph::ChevronDown, 14.0);
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
