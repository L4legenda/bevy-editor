use super::*;
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::ui_widgets::{SelectAllOnFocus, TextInput};
use material_painter_graph::{ParamKind, Value, def::Param};
use std::collections::BTreeMap;

#[cfg(test)]
#[test]
fn preview_values_follow_parameter_kind_and_range_without_editing_source() {
    let source = "// @output color\n// @param gain: float(0, 1) = 0.2\n// @param enabled: bool = false\nfn node(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(params.gain); }\n";
    let mut state = State::default();
    state.schema(source);
    state
        .edit(Edit::Number("gain".into(), 0, "0,8".into()))
        .unwrap();
    state
        .edit(Edit::Set("enabled".into(), Value::Bool(true)))
        .unwrap();
    assert!(
        state
            .edit(Edit::Number("gain".into(), 0, "NaN".into()))
            .is_err()
    );
    let narrowed = source.replace("float(0, 1)", "float(0, 0.5)");
    state.schema(&narrowed);
    let definition = def::parse(DRAFT_ID, &narrowed).unwrap();
    let mut graph = Graph::new("test");
    let id = graph.add(DRAFT_ID, [0., 0.]);
    let node = graph.node_mut(id).unwrap();
    state.values.apply(node, &definition);
    assert_eq!(node.params["gain"], Value::Float(0.5));
    assert_eq!(node.params["enabled"], Value::Bool(true));
    assert_eq!(definition.params[0].default, Value::Float(0.2));
    state.schema(&narrowed.replace("bool = false", "int(0, 10) = 3"));
    assert!(!state.values.params.contains_key("enabled"));
    state.edit(Edit::Reset).unwrap();
    assert!(state.values.params.is_empty());
}

#[derive(Clone, Default)]
pub(super) struct Values {
    params: BTreeMap<String, Value>,
    images: texture_fill::GraphImages,
}

impl Values {
    pub(super) fn apply(&self, node: &mut material_painter_graph::Node, definition: &def::NodeDef) {
        node.params = definition
            .params
            .iter()
            .filter_map(|parameter| {
                let value = self.params.get(&parameter.name)?;
                Some((
                    parameter.name.clone(),
                    normalize(&parameter.kind, value.clone())?,
                ))
            })
            .collect();
    }

    pub(super) fn images(&self) -> &texture_fill::GraphImages {
        &self.images
    }

    fn prune_images(&mut self) {
        self.images.retain(|key, _| {
            self.params.values().any(
                |value| matches!(value, Value::Image(Some(path)) if path.to_str() == Some(key)),
            )
        });
    }
}

#[derive(Default)]
pub(super) struct State {
    pub(super) values: Arc<Values>,
    parameters: Vec<Param>,
    valid: bool,
    pub(super) dirty: bool,
    menu: Option<String>,
    custom_maps: Option<Arc<Vec<custom::Recipe>>>,
}

#[derive(Clone)]
pub(super) enum Edit {
    Reset,
    Choose(String),
    Set(String, Value),
    Number(String, usize, String),
    RollSeed(String),
    PickImage(String),
}

impl State {
    pub(super) fn new_source(&mut self) {
        self.values = Arc::default();
        self.menu = None;
        self.dirty = true;
    }

    pub(super) fn schema(&mut self, source: &str) {
        let definition = (source.len() <= SOURCE_LIMIT)
            .then(|| def::parse(DRAFT_ID, source).ok())
            .flatten();
        let valid = definition.is_some();
        self.dirty |= self.valid != valid;
        self.valid = valid;
        let Some(definition) = definition else { return };
        let parameters: Vec<_> = definition
            .params
            .into_iter()
            .filter(|parameter| {
                !definition.shading
                    || !matches!(parameter.kind, ParamKind::Image | ParamKind::MeshMap)
            })
            .collect();
        if self.parameters.len() == parameters.len()
            && self
                .parameters
                .iter()
                .zip(&parameters)
                .all(|(a, b)| a.name == b.name && a.kind == b.kind && a.default == b.default)
        {
            return;
        }
        let retained: BTreeMap<_, _> = parameters
            .iter()
            .filter_map(|parameter| {
                let value = self.values.params.get(&parameter.name)?;
                Some((
                    parameter.name.clone(),
                    normalize(&parameter.kind, value.clone())?,
                ))
            })
            .collect();
        if retained != self.values.params {
            let values = Arc::make_mut(&mut self.values);
            values.params = retained;
            values.prune_images();
        }
        self.parameters = parameters;
        self.menu = None;
        self.dirty = true;
    }

    pub(super) fn visible(&self) -> bool {
        self.valid && !self.parameters.is_empty()
    }

    pub(super) fn maps(&mut self, recipes: &Arc<Vec<custom::Recipe>>) {
        if self
            .custom_maps
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, recipes))
        {
            self.custom_maps = Some(recipes.clone());
            self.dirty = true;
        }
    }

    fn value<'a>(&'a self, parameter: &'a Param) -> &'a Value {
        self.values
            .params
            .get(&parameter.name)
            .unwrap_or(&parameter.default)
    }

    fn parameter(&self, name: &str) -> Result<&Param, String> {
        self.parameters
            .iter()
            .find(|parameter| self.valid && parameter.name == name)
            .ok_or_else(|| "Параметр предпросмотра недоступен".into())
    }

    fn set(&mut self, name: String, value: Value) -> Result<(), String> {
        let parameter = self.parameter(&name)?;
        let value = normalize(&parameter.kind, value).ok_or("Несовместимый тип параметра")?;
        if self.value(parameter) != &value {
            let values = Arc::make_mut(&mut self.values);
            values.params.insert(name, value);
            values.prune_images();
        }
        self.menu = None;
        self.dirty = true;
        Ok(())
    }

    pub(super) fn image(
        &mut self,
        name: String,
        bitmap: texture_fill::Bitmap,
    ) -> Result<(), String> {
        if self.parameter(&name)?.kind != ParamKind::Image {
            return Err("Параметр изображения недоступен".into());
        }
        let key = format!("shader-preview/{}.png", bevy::asset::uuid::Uuid::new_v4());
        self.set(name, Value::Image(Some(PathBuf::from(&key))))?;
        Arc::make_mut(&mut self.values)
            .images
            .insert(key, Arc::new(bitmap));
        Ok(())
    }

    pub(super) fn edit(&mut self, edit: Edit) -> Result<(), String> {
        match edit {
            Edit::Reset => self.new_source(),
            Edit::Choose(name) => {
                self.menu = (self.menu.as_deref() != Some(&name)).then_some(name);
                self.dirty = true;
            }
            Edit::Set(name, value) => self.set(name, value)?,
            Edit::Number(name, component, text) => {
                let parameter = self.parameter(&name)?;
                let text = text.trim().replace(',', ".");
                let error = "Введите конечное число";
                let value = match self.value(parameter) {
                    Value::Float(_) => Value::Float(text.parse().map_err(|_| error)?),
                    Value::Int(_) => Value::Int(text.parse().map_err(|_| "Введите целое число")?),
                    Value::Color(rgb) if component < rgb.len() => {
                        let mut rgb = *rgb;
                        rgb[component] = text.parse().map_err(|_| error)?;
                        Value::Color(rgb)
                    }
                    _ => return Err(error.into()),
                };
                self.set(name, value)?;
            }
            Edit::RollSeed(name) => {
                let ParamKind::Int { min, max } = self.parameter(&name)?.kind else {
                    return Err("Параметр зерна недоступен".into());
                };
                let width = (i64::from(max) - i64::from(min) + 1).max(1) as u64;
                let random = bevy::asset::uuid::Uuid::new_v4().as_u128() as u64;
                self.set(
                    name,
                    Value::Int((i64::from(min) + (random % width) as i64) as i32),
                )?;
            }
            Edit::PickImage(_) => unreachable!("Проводник обрабатывается редактором WGSL"),
        }
        Ok(())
    }
}

fn normalize(kind: &ParamKind, value: Value) -> Option<Value> {
    Some(match (kind, value) {
        (ParamKind::Float { min, max }, Value::Float(v)) if v.is_finite() && min <= max => {
            Value::Float(v.clamp(*min, *max))
        }
        (ParamKind::Int { min, max }, Value::Int(v)) if min <= max => {
            Value::Int(v.clamp(*min, *max))
        }
        (ParamKind::Color, Value::Color(rgb)) if rgb.iter().all(|v| v.is_finite()) => {
            Value::Color(rgb.map(|v| v.clamp(0., 1.)))
        }
        (ParamKind::Enum(choices), Value::Enum(index)) if !choices.is_empty() => {
            Value::Enum(index.min(choices.len() - 1))
        }
        (ParamKind::Bool, value @ Value::Bool(_))
        | (ParamKind::Image, value @ Value::Image(_))
        | (ParamKind::MeshMap, value @ Value::MeshMap(_)) => value,
        _ => return None,
    })
}

pub(super) fn controls(commands: &mut Commands, panel: Entity, font: &Handle<Font>, state: &State) {
    if !state.visible() {
        return;
    }
    text(commands, panel, font, "Параметры предпросмотра", 12., TEXT);
    if !state.values.params.is_empty() {
        let actions = node(commands, panel, row(5.));
        button(commands, actions, font, "Сбросить значения", Edit::Reset);
    }
    for parameter in &state.parameters {
        let line = node(
            commands,
            panel,
            Node {
                flex_shrink: 0.,
                flex_wrap: FlexWrap::Wrap,
                ..row(5.)
            },
        );
        let label = parameter.name.replace('_', " ");
        text(commands, line, font, &label, 12., MUTED);
        let name = &parameter.name;
        match state.value(parameter) {
            Value::Float(value) => number(commands, line, font, name, None, value.to_string()),
            Value::Int(value) => {
                number(commands, line, font, name, None, value.to_string());
                if name.contains("seed") {
                    button(
                        commands,
                        line,
                        font,
                        "Новое зерно",
                        Edit::RollSeed(name.clone()),
                    );
                }
            }
            Value::Color(rgb) => {
                for (index, value) in rgb.iter().enumerate() {
                    number(commands, line, font, name, Some(index), value.to_string());
                }
            }
            Value::Bool(value) => button(
                commands,
                line,
                font,
                if *value { "Вкл." } else { "Выкл." },
                Edit::Set(name.clone(), Value::Bool(!value)),
            ),
            Value::Enum(index) => {
                if let ParamKind::Enum(choices) = &parameter.kind {
                    choices_menu(
                        commands,
                        line,
                        font,
                        state,
                        name,
                        *index,
                        choices
                            .iter()
                            .enumerate()
                            .map(|(index, label)| (index, label.as_str())),
                        Value::Enum,
                    );
                }
            }
            Value::MeshMap(index) => {
                let base =
                    material_painter_graph::MESH_MAPS
                        .iter()
                        .enumerate()
                        .map(|(i, source)| {
                            (
                                i,
                                material_graph::mesh_maps::kind(i)
                                    .map_or(*source, |map| map.label()),
                            )
                        });
                let custom = state
                    .custom_maps
                    .iter()
                    .flat_map(|recipes| recipes.iter())
                    .filter_map(|map| {
                        let value = material_painter_graph::CUSTOM_MAP_VALUE
                            .checked_add(usize::try_from(map.id).ok()?)?;
                        Some((value, map.name.as_str()))
                    });
                choices_menu(
                    commands,
                    line,
                    font,
                    state,
                    name,
                    *index,
                    base.chain(custom),
                    Value::MeshMap,
                );
            }
            Value::Image(path) => {
                if let Some(bitmap) = path
                    .as_ref()
                    .and_then(|path| state.values.images.get(path.to_str()?))
                {
                    text(commands, line, font, &bitmap.name, 11., MUTED);
                    button(
                        commands,
                        line,
                        font,
                        "Убрать",
                        Edit::Set(name.clone(), Value::Image(None)),
                    );
                }
                button(
                    commands,
                    line,
                    font,
                    "Выбрать изображение…",
                    Edit::PickImage(name.clone()),
                );
            }
        }
    }
}

fn choices_menu<'a>(
    commands: &mut Commands,
    line: Entity,
    font: &Handle<Font>,
    state: &State,
    name: &str,
    index: usize,
    choices: impl Iterator<Item = (usize, &'a str)>,
    value: impl Fn(usize) -> Value,
) {
    let choices: Vec<_> = choices.collect();
    let selected = choices
        .iter()
        .find(|(position, _)| *position == index)
        .map_or("Выбрать", |(_, label)| *label);
    button(
        commands,
        line,
        font,
        &format!("{selected} ▾"),
        Edit::Choose(name.to_owned()),
    );
    if state.menu.as_deref() == Some(name) {
        for (position, label) in choices {
            button(
                commands,
                line,
                font,
                label,
                Edit::Set(name.to_owned(), value(position)),
            );
        }
    }
}

fn button(commands: &mut Commands, parent: Entity, font: &Handle<Font>, label: &str, edit: Edit) {
    control(commands, parent, font, label, Command::Parameter(edit));
}

fn number(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    name: &str,
    component: Option<usize>,
    value: String,
) {
    let label = component.map_or_else(
        || name.to_owned(),
        |index| format!("{name} {}", ["R", "G", "B"][index]),
    );
    if let Some(index) = component {
        text(commands, parent, font, ["R", "G", "B"][index], 11., MUTED);
    }
    let name = name.to_owned();
    commands
        .spawn((
            ChildOf(parent),
            TextInput,
            EditableText::new(&value),
            SelectAllOnFocus,
            TabIndex(0),
            crate::ui_theme::text_cursor_style(),
            AccessibleLabel(format!("WGSL · {label}")),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(12.),
                ..default()
            },
            TextColor(TEXT),
            BackgroundColor(WELL),
            BorderColor::all(EDGE),
            Node {
                width: px(68.),
                height: px(26.),
                padding: UiRect::horizontal(px(5.)),
                border: UiRect::all(px(1.)),
                overflow: Overflow::clip(),
                ..default()
            },
        ))
        .observe(
            move |mut event: On<FocusedInput<KeyboardInput>>,
                  fields: Query<&EditableText>,
                  mut maker: ResMut<Maker>| {
                if !event.input.repeat
                    && event.input.state == ButtonState::Pressed
                    && matches!(event.input.key_code, KeyCode::Enter | KeyCode::NumpadEnter)
                    && let Ok(field) = fields.get(event.focused_entity)
                {
                    maker.command = Some(Command::Parameter(Edit::Number(
                        name.clone(),
                        component.unwrap_or_default(),
                        field.value().to_string(),
                    )));
                    event.propagate(false);
                }
            },
        );
}
