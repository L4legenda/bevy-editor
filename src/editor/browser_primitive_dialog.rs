//! Shape-specific fields in the file browser's creation dialog.
use super::*;
use crate::editor::model_primitives::{Parameter, Parameters, Primitive};

struct Field {
    kind: Parameter,
    group: Entity,
    label: Entity,
    input: Entity,
}

pub(super) struct ParametersUi {
    pub root: Entity,
    pub preview: Entity,
    pub image: Entity,
    pub status: Entity,
    fields: Vec<Field>,
}

pub(super) fn setup(
    commands: &mut Commands,
    dialog: Entity,
    handle: &Handle<Font>,
) -> ParametersUi {
    let root = node(
        commands,
        dialog,
        Node {
            display: Display::None,
            width: percent(100),
            flex_shrink: 0.0,
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Start,
            column_gap: px(12),
            row_gap: px(12),
            ..row(16.0)
        },
    );
    let preview_group = node(
        commands,
        root,
        Node {
            flex_basis: px(300),
            flex_grow: 1.0,
            min_width: px(200),
            ..column(7.0)
        },
    );
    text(commands, preview_group, handle, "Предпросмотр", 12.0, MUTED);
    let preview = surface(
        commands,
        preview_group,
        Node {
            width: percent(100),
            height: px(254),
            flex_shrink: 0.0,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            overflow: Overflow::clip(),
            ..default()
        },
        WELL,
    );
    commands.entity(preview).insert(AccessibleLabel(
        "3D-предпросмотр: перетащите мышью для вращения, колесо для масштаба".into(),
    ));
    let image = commands
        .spawn((
            ImageNode::default(),
            Node {
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(preview),
        ))
        .id();
    text(
        commands,
        preview_group,
        handle,
        "ЛКМ · вращение   Колесо · масштаб",
        11.0,
        MUTED,
    );
    let status = text(commands, preview_group, handle, "", 11.0, MUTED);
    commands.entity(status).insert((
        TextLayout::default(),
        Node {
            width: percent(100),
            min_height: px(28),
            ..default()
        },
    ));
    let parameter_group = node(
        commands,
        root,
        Node {
            flex_basis: px(300),
            flex_grow: 1.0,
            min_width: px(200),
            ..column(10.0)
        },
    );
    text(commands, parameter_group, handle, "Параметры", 12.0, MUTED);
    let fields_root = node(
        commands,
        parameter_group,
        Node {
            width: percent(100),
            flex_wrap: FlexWrap::Wrap,
            column_gap: px(12),
            row_gap: px(12),
            ..row(0.0)
        },
    );
    let fields = Parameter::ALL
        .into_iter()
        .map(|kind| {
            let group = node(
                commands,
                fields_root,
                Node {
                    width: percent(48),
                    flex_grow: 1.0,
                    min_width: px(0),
                    ..column(5.0)
                },
            );
            let label = text(commands, group, handle, "", 12.0, MUTED);
            let input = commands
                .spawn((
                    TextInput,
                    number(kind, Primitive::Cube),
                    EditableText::new(""),
                    crate::ui_theme::text_cursor_style(),
                    AccessibleLabel(format!("Параметр фигуры: {}", kind.label(Primitive::Cube))),
                    font(handle, 14.0),
                    TextColor(TEXT),
                    TextLayout::no_wrap(),
                    TabIndex(0),
                    BackgroundColor(WELL),
                    BorderColor::all(EDGE),
                    Node {
                        width: percent(100),
                        height: px(36),
                        flex_shrink: 0.0,
                        padding: UiRect::axes(px(9), px(7)),
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(6)),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    ChildOf(group),
                ))
                .id();
            Field {
                kind,
                group,
                label,
                input,
            }
        })
        .collect();
    let hint = text(
        commands,
        parameter_group,
        handle,
        "Числа: тяните ← → · Shift — точно · Ctrl — быстрее",
        11.0,
        MUTED,
    );
    commands.entity(hint).insert(TextLayout::default());
    ParametersUi {
        root,
        preview,
        image,
        status,
        fields,
    }
}

impl ParametersUi {
    pub fn input(&self, kind: Parameter) -> Entity {
        self.fields
            .iter()
            .find(|field| field.kind == kind)
            .unwrap()
            .input
    }

    pub fn inputs(&self) -> impl Iterator<Item = Entity> + '_ {
        self.fields.iter().map(|field| field.input)
    }

    pub fn reset(
        &self,
        kind: Primitive,
        inputs: &mut Query<&mut EditableText>,
        commands: &mut Commands,
    ) {
        let defaults = kind.defaults();
        for field in &self.fields {
            commands
                .entity(field.input)
                .insert(number(field.kind, kind));
            if let Ok(mut input) = inputs.get_mut(field.input) {
                input
                    .editor_mut()
                    .set_text(&defaults.get(field.kind).to_string());
            }
        }
    }

    pub fn signature(&self, kind: Primitive, inputs: &Query<&mut EditableText>) -> String {
        kind.fields()
            .iter()
            .filter_map(|&field| {
                inputs
                    .get(self.input(field))
                    .ok()
                    .map(|input| input.value().to_string())
            })
            .collect::<Vec<_>>()
            .join(":")
    }

    pub fn read(
        &self,
        kind: Primitive,
        inputs: &Query<&EditableText>,
    ) -> Result<Parameters, String> {
        let mut parameters = kind.defaults();
        for &field in kind.fields() {
            let raw = inputs
                .get(self.input(field))
                .map(|input| input.value().to_string())
                .unwrap_or_default();
            let raw = raw.trim().replace(',', ".");
            let value = if field.integer() {
                raw.parse::<u32>().ok().map(|value| value as f32)
            } else {
                raw.parse::<f32>().ok()
            };
            let (min, max) = field.range(kind);
            let value = value
                .filter(|value| value.is_finite() && *value >= min && *value <= max)
                .ok_or_else(|| {
                    format!(
                        "{}: введите {}число от {min} до {max}.",
                        field.label(kind),
                        if field.integer() { "целое " } else { "" }
                    )
                })?;
            parameters.set(field, value);
        }
        parameters.validate(kind)?;
        Ok(parameters)
    }

    pub fn refresh(
        &self,
        kind: Option<Primitive>,
        nodes: &mut Query<&mut Node>,
        labels: &mut Query<&mut Text>,
        inputs: &Query<&EditableText>,
        numbers: &mut Query<&mut Number>,
    ) {
        for field in &self.fields {
            let visible = kind.is_some_and(|kind| kind.fields().contains(&field.kind));
            if let Ok(mut node) = nodes.get_mut(field.group) {
                node.display = if visible {
                    Display::Flex
                } else {
                    Display::None
                };
            }
            if let Some(kind) = kind
                && let Ok(mut label) = labels.get_mut(field.label)
            {
                label.set_if_neq(Text(field.kind.label(kind).into()));
            }
            if let Some(kind) = kind {
                let get = |parameter| {
                    inputs
                        .get(self.input(parameter))
                        .ok()
                        .and_then(|text| {
                            text.value()
                                .to_string()
                                .trim()
                                .replace(',', ".")
                                .parse::<f32>()
                                .ok()
                        })
                        .filter(|v| v.is_finite() && *v > 0.0)
                };
                let (mut min, mut max) = field.kind.range(kind);
                if matches!(kind, Primitive::Torus | Primitive::UvSphere) {
                    let other = match field.kind {
                        Parameter::Segments => Some(Parameter::Rings),
                        Parameter::Rings => Some(Parameter::Segments),
                        _ => None,
                    };
                    if let Some(other) = other.and_then(get) {
                        max = max
                            .min((Parameters::MAX_CELLS as f32 / other).floor())
                            .max(min);
                    }
                }
                if kind == Primitive::Torus {
                    match field.kind {
                        Parameter::MajorRadius => {
                            if let Some(other) = get(Parameter::MinorRadius) {
                                min = (other + (other * 0.000001).max(0.0001)).min(max);
                            }
                        }
                        Parameter::MinorRadius => {
                            if let Some(other) = get(Parameter::MajorRadius) {
                                max = (other - (other * 0.000001).max(0.0001)).max(min);
                            }
                        }
                        _ => {}
                    }
                }
                if let Ok(mut current) = numbers.get_mut(field.input) {
                    current.set_if_neq(number(field.kind, kind).bounds(min, max));
                }
            }
        }
    }
}

fn number(field: Parameter, kind: Primitive) -> Number {
    let (min, max) = field.range(kind);
    if field.integer() {
        Number::integer(min as u32, max as u32)
    } else {
        Number::decimal(min, max, 0.01, 4)
    }
}
