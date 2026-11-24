use super::*;
use crate::editor::dialogue::conditions::{Catalog, Operator, Rule, Value, Variable};
use std::collections::BTreeMap;
#[path = "narrative_variables_ui.rs"]
mod variable_ui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Edit {
    Open(u32, Option<usize>),
    Variables,
    Select(u64),
    New,
    ClearSearch,
    ToggleTypeMenu,
    Type(bool),
    Flag(bool),
    Back,
    Close,
    Leave,
    Discard,
    Stay,
    Confirm,
    CancelConfirm,
    NestedNew,
    CancelNested,
    PreviewPanel,
    SaveVariable,
    RemoveVariable,
    Mode(bool),
    Add(u64),
    Variable(usize, u64),
    Operator(usize, Operator),
    FlagRule(usize, bool),
    Remove(usize),
    Apply,
    TestFlag(u64),
    ResetTests,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    Search,
    Name,
    Default,
    Rule(usize),
    Test(u64),
}

struct VariableDraft {
    id: Option<u64>,
    name: String,
    initial_name: String,
    integer: bool,
    type_menu_open: bool,
    flag: bool,
    number: String,
    invalid: Option<Field>,
}

impl VariableDraft {
    fn new(name: String) -> Self {
        Self {
            id: None,
            initial_name: name.clone(),
            name,
            integer: false,
            type_menu_open: false,
            flag: false,
            number: "0".into(),
            invalid: None,
        }
    }

    fn existing(id: u64, variable: &Variable) -> Self {
        let mut draft = Self::new(variable.name.clone());
        draft.id = Some(id);
        match variable.default {
            Value::Flag(value) => draft.flag = value,
            Value::Number(value) => {
                draft.integer = true;
                draft.number = value.to_string();
            }
        }
        draft
    }

    fn value(&self) -> Result<Value, Field> {
        if self.integer {
            self.number
                .trim()
                .parse()
                .map(Value::Number)
                .map_err(|_| Field::Default)
        } else {
            Ok(Value::Flag(self.flag))
        }
    }

    fn to_variable(&self) -> Result<Variable, Field> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(Field::Name);
        }
        Ok(Variable {
            name: name.to_owned(),
            default: self.value()?,
        })
    }

    fn error(&self, field: Field) -> Option<&'static str> {
        if self.invalid != Some(field) {
            return None;
        }
        match field {
            Field::Name if self.name.trim().is_empty() => Some("Укажите название переменной"),
            Field::Default if self.value().is_err() => Some("Введите целое число"),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    pub catalog: Catalog,
    pub anchor: Option<(u32, Option<usize>)>,
    pub revision: u64,
    root: Option<std::path::PathBuf>,
    load_error: Option<String>,
    error: Option<String>,
    draft: Condition,
    rule_inputs: BTreeMap<usize, String>,
    variable: Option<VariableDraft>,
    tests: BTreeMap<u64, String>,
    chooser: Option<Option<usize>>,
    operators: Option<usize>,
    search: String,
    cursor: usize,
    mark_cursor: bool,
    nested: bool,
    leaving: Option<Edit>,
    confirm: Option<Edit>,
    preview_open: bool,
    pub(super) focus: Option<Field>,
    pub(super) invalid_test: Option<u64>,
    submitted: bool,
    pub(super) exit: Option<NarrativeAction>,
    resume: Option<NarrativeAction>,
}

impl State {
    pub(super) fn load(&mut self, root: &std::path::Path) {
        if self.root.as_deref() == Some(root) {
            return;
        }
        *self = Self::default();
        self.root = Some(root.to_owned());
        match Catalog::read(root) {
            Ok(catalog) => self.catalog = catalog,
            Err(error) => {
                self.load_error = Some(format!("Не удалось прочитать переменные: {error}"))
            }
        }
    }

    pub fn values(&self) -> BTreeMap<u64, Value> {
        self.catalog
            .variables
            .iter()
            .map(|(&id, var)| {
                let value = self
                    .tests
                    .get(&id)
                    .and_then(|input| match var.default {
                        Value::Number(_) => input.trim().parse().ok().map(Value::Number),
                        Value::Flag(_) => input.trim().parse().ok().map(Value::Flag),
                    })
                    .unwrap_or(var.default);
                (id, value)
            })
            .collect()
    }

    pub fn input(&mut self, field: Field, value: String) {
        match field {
            Field::Search => {
                if self.search != value {
                    self.search = value;
                    self.cursor = 0;
                    self.mark_cursor = !self.search.is_empty();
                }
            }
            Field::Name => {
                if let Some(draft) = &mut self.variable {
                    draft.name = value;
                }
            }
            Field::Default => {
                if let Some(draft) = &mut self.variable {
                    draft.number = value;
                }
            }
            Field::Rule(i) => {
                self.rule_inputs.insert(i, value);
            }
            Field::Test(id) => {
                self.tests.insert(id, value);
            }
        }
    }

    fn new_variable(&mut self, use_search: bool) {
        let name = if use_search {
            self.search.trim().to_owned()
        } else {
            String::new()
        };
        self.variable = Some(VariableDraft::new(name));
        self.focus = Some(Field::Name);
    }

    fn select_variable(&mut self, id: u64) {
        if let Some(variable) = self.catalog.variables.get(&id) {
            self.variable = Some(VariableDraft::existing(id, variable));
            self.cursor = self
                .results()
                .iter()
                .position(|&key| key == id)
                .unwrap_or(0);
            self.mark_cursor = true;
            self.focus = Some(Field::Name);
        }
    }

    fn show_library(&mut self, saved: Option<u64>) {
        self.variable = None;
        let mut results = self.results();
        if let Some(id) = saved {
            if !results.contains(&id) {
                self.search.clear();
                results = self.results();
            }
            self.cursor = results.iter().position(|&key| key == id).unwrap_or(0);
            self.mark_cursor = true;
        } else {
            self.cursor = self.cursor.min(results.len().saturating_sub(1));
        }
        self.focus = Some(Field::Search);
    }

    fn cancel_nested_variable(&mut self) {
        self.variable = None;
        self.nested = false;
        self.focus = Some(Field::Search);
    }

    pub(super) fn creating_variable(&self, picker: Option<Picker>) -> bool {
        (picker == Some(Picker::Variables) || picker == Some(Picker::Condition) && self.nested)
            && self
                .variable
                .as_ref()
                .is_some_and(|draft| draft.id.is_none())
    }

    pub(super) fn variable_title(&self, picker: Option<Picker>) -> Option<&'static str> {
        if picker != Some(Picker::Variables) && !self.creating_variable(picker) {
            return None;
        }
        Some(match self.confirm {
            Some(Edit::RemoveVariable) => "Удалить переменную?",
            Some(_) => "Изменить тип переменной?",
            None if self.creating_variable(picker) => "Новая переменная",
            None if self.variable.is_some() => "Свойства переменной",
            None => "Переменные диалогов",
        })
    }
    fn dirty(&self) -> bool {
        let Some(draft) = &self.variable else {
            return false;
        };
        let Some(saved) = draft.id.and_then(|id| self.catalog.variables.get(&id)) else {
            return draft.name.trim() != draft.initial_name || draft.integer || draft.flag;
        };
        draft.name.trim() != saved.name || draft.value() != Ok(saved.default)
    }

    fn apply_variable(&mut self, target: Option<usize>, id: u64) {
        let Some(var) = self.catalog.variables.get(&id) else {
            return;
        };
        let rule = Rule {
            variable: id,
            operator: Operator::Eq,
            value: var.default,
        };
        if let Some(i) = target {
            if let Some(old) = self.draft.rules.get_mut(i) {
                *old = rule;
            }
            self.rule_inputs.remove(&i);
        } else {
            self.draft.rules.push(rule);
        }
        self.chooser = None;
        self.nested = false;
        self.variable = None;
        self.operators = None;
    }

    fn results(&self) -> Vec<u64> {
        self.matching_variables(&self.search)
    }

    fn matching_variables(&self, search: &str) -> Vec<u64> {
        let search = search.to_lowercase().replace('ё', "е");
        let mut rows: Vec<_> = self
            .catalog
            .variables
            .iter()
            .filter(|(_, var)| {
                let name = var.name.to_lowercase().replace('ё', "е");
                search.split_whitespace().all(|word| name.contains(word))
            })
            .collect();
        rows.sort_by_cached_key(|(id, var)| (var.name.to_lowercase(), **id));
        rows.into_iter().map(|(&id, _)| id).collect()
    }

    pub(super) fn reveal_invalid_test(&mut self, id: u64) {
        self.preview_open = true;
        self.focus = Some(Field::Test(id));
        self.revision += 1;
    }

    pub(super) fn invalid_tests(&self) -> Option<u64> {
        self.tests.iter().find_map(|(&id, value)| {
            (self
                .catalog
                .variables
                .get(&id)
                .is_some_and(|v| matches!(v.default, Value::Number(_)))
                && value.trim().parse::<i64>().is_err())
            .then_some(id)
        })
    }
}

fn condition_mut(node: &mut DialogueNode, answer: Option<usize>) -> Option<&mut Condition> {
    match answer {
        Some(index) => Some(&mut node.choices.get_mut(index)?.condition),
        None if matches!(node.kind, NodeKind::Start | NodeKind::Line) => Some(&mut node.condition),
        None => None,
    }
}

pub(super) fn handle(editor: &mut Editor, ui: &mut NarrativeUi, edit: Edit) {
    let state = &mut ui.conditions;
    state.load(&editor.root);
    if !matches!(ui.picker, Some(Picker::Variables | Picker::Condition))
        && !matches!(
            edit,
            Edit::Open(..)
                | Edit::Variables
                | Edit::PreviewPanel
                | Edit::TestFlag(_)
                | Edit::ResetTests
        )
    {
        return;
    }
    // Opening or dismissing the picker keeps the form and its text inputs mounted.
    if let Some(draft) = &mut state.variable {
        if edit == Edit::ToggleTypeMenu {
            draft.type_menu_open = !draft.type_menu_open;
            return;
        }
        if std::mem::take(&mut draft.type_menu_open)
            && (edit == Edit::Close || edit == Edit::Type(draft.integer))
        {
            return;
        }
    }
    let confirmed = edit == Edit::Confirm;
    let edit = if confirmed {
        let Some(edit) = state.confirm.take() else {
            return;
        };
        edit
    } else {
        edit
    };
    state.error = None;
    state.revision += 1;
    if edit == Edit::Close && (state.confirm.take().is_some() || state.leaving.take().is_some()) {
        state.exit = None;
        return;
    }
    if ui.picker == Some(Picker::Variables)
        && matches!(
            edit,
            Edit::Select(_) | Edit::New | Edit::Close | Edit::Leave
        )
        && state.dirty()
    {
        state.leaving = Some(edit);
        return;
    }
    match edit {
        Edit::Discard if ui.picker == Some(Picker::Variables) => {
            state.variable = None;
            if let Some(next) = state.leaving.take() {
                handle(editor, ui, next);
            }
        }
        Edit::Stay => {
            state.leaving = None;
            state.exit = None;
            state.focus = Some(Field::Name);
        }
        Edit::Leave if ui.picker == Some(Picker::Variables) => {
            ui.picker = None;
            state.resume = state.exit.take();
        }
        Edit::CancelConfirm => state.confirm = None,
        Edit::Close if ui.picker == Some(Picker::Condition) => {
            if state.nested {
                state.cancel_nested_variable();
            } else if state.chooser.take().is_some() || state.operators.take().is_some() {
                state.focus = None;
            } else {
                ui.picker = None;
            }
        }
        Edit::Close if ui.picker == Some(Picker::Variables) => {
            if state.confirm.take().is_none() && state.leaving.take().is_none() {
                ui.picker = None;
            }
        }
        Edit::Back if ui.picker == Some(Picker::Variables) => state.show_library(None),
        Edit::NestedNew if ui.picker == Some(Picker::Condition) && state.chooser.is_some() => {
            state.new_variable(true);
            state.nested = true;
        }
        Edit::CancelNested if ui.picker == Some(Picker::Condition) => {
            state.cancel_nested_variable();
        }
        Edit::Open(id, answer) if ui.picker.is_none() && !ui.preview => {
            let Some(doc) = editor.narrative.current() else {
                return;
            };
            let Content::Dialogue(d) = &doc.content else {
                return;
            };
            let Some(n) = d.node(id) else { return };
            let condition = match answer {
                Some(i) => n.choices.get(i).map(|c| &c.condition),
                None if matches!(n.kind, NodeKind::Start | NodeKind::Line) => Some(&n.condition),
                None => None,
            };
            let Some(condition) = condition else { return };
            state.anchor = Some((id, answer));
            state.draft = condition.clone();
            state.rule_inputs.clear();
            state.submitted = false;
            state.chooser = None;
            state.operators = None;
            state.nested = false;
            state.focus = None;
            ui.picker = Some(Picker::Condition);
            ui.linking = None;
        }
        Edit::Variables if !ui.preview && ui.picker.is_none() => {
            state.variable = None;
            state.nested = false;
            state.leaving = None;
            state.confirm = None;
            state.exit = None;
            state.search.clear();
            state.cursor = 0;
            state.mark_cursor = false;
            if state.catalog.variables.is_empty() && state.load_error.is_none() {
                state.new_variable(false);
            } else {
                state.focus = Some(Field::Search);
            }
            ui.picker = Some(Picker::Variables);
            ui.linking = None;
        }
        Edit::Select(id) if ui.picker == Some(Picker::Variables) => state.select_variable(id),
        Edit::New if ui.picker == Some(Picker::Variables) => {
            state.new_variable(!state.search.is_empty() && state.results().is_empty());
        }
        Edit::ClearSearch => {
            state.input(Field::Search, String::new());
            state.focus = Some(Field::Search);
        }
        Edit::Type(number) if ui.picker == Some(Picker::Variables) || state.nested => {
            if let Some(draft) = &mut state.variable {
                draft.integer = number;
                state.focus = Some(if number && !draft.name.trim().is_empty() {
                    Field::Default
                } else {
                    Field::Name
                });
            }
        }
        Edit::Flag(value) if ui.picker == Some(Picker::Variables) || state.nested => {
            if let Some(draft) = &mut state.variable {
                draft.flag = value;
                state.focus = Some(Field::Name);
            }
        }
        Edit::SaveVariable | Edit::RemoveVariable
            if ui.picker == Some(Picker::Variables) || state.nested =>
        {
            if state.load_error.is_some() {
                return;
            }
            let Some(draft) = &mut state.variable else {
                return;
            };
            let editing = draft.id;
            let replacement = if edit == Edit::RemoveVariable {
                None
            } else {
                match draft.to_variable() {
                    Ok(variable) => Some(variable),
                    Err(field) => {
                        draft.invalid = Some(field);
                        state.leaving = None;
                        state.exit = None;
                        state.focus = Some(field);
                        return;
                    }
                }
            };
            let previous_type = editing
                .and_then(|id| state.catalog.variables.get(&id))
                .map(|variable| std::mem::discriminant(&variable.default));
            let new_type = replacement
                .as_ref()
                .map(|variable| std::mem::discriminant(&variable.default));
            let changes_type = previous_type.is_some() && previous_type != new_type;
            if !confirmed && changes_type {
                state.confirm = Some(edit);
                return;
            }
            let mut catalog = state.catalog.clone();
            let saved_id = if let Some(variable) = replacement {
                let id = if let Some(id) = editing {
                    id
                } else {
                    let largest = catalog.variables.keys().next_back().copied().unwrap_or(0);
                    let Some(id) = largest.max(catalog.next_id).checked_add(1) else {
                        return;
                    };
                    catalog.next_id = id;
                    id
                };
                catalog.variables.insert(id, variable);
                Some(id)
            } else {
                let Some(id) = editing else { return };
                catalog.variables.remove(&id);
                None
            };
            match catalog.save(&editor.root) {
                Ok(()) => {
                    if let Some(id) = editing
                        && changes_type
                    {
                        state.tests.remove(&id);
                    }
                    state.catalog = catalog;
                    if state.nested
                        && let Some(id) = saved_id
                        && let Some(target) = state.chooser
                    {
                        state.apply_variable(target, id);
                    } else {
                        state.show_library(saved_id);
                    }
                    ui.generation += 1;
                    if let Some(next) = state.leaving.take() {
                        handle(editor, ui, next);
                    }
                }
                Err(error) => {
                    state.error = Some(format!("Не удалось сохранить переменные: {error}"))
                }
            }
        }
        Edit::PreviewPanel if ui.preview => state.preview_open = !state.preview_open,
        Edit::TestFlag(id) if ui.preview => {
            if let Some(Value::Flag(value)) = state.values().get(&id) {
                state.tests.insert(id, (!value).to_string());
            }
        }
        Edit::ResetTests if ui.preview => {
            state.tests.clear();
            state.invalid_test = None;
        }
        _ if ui.picker == Some(Picker::Condition) => match edit {
            Edit::Mode(any) => state.draft.any = any,
            Edit::Add(u64::MAX) | Edit::Variable(_, u64::MAX) => {
                state.chooser = Some(if let Edit::Variable(i, _) = edit {
                    Some(i)
                } else {
                    None
                });
                state.search.clear();
                state.cursor = 0;
                state.mark_cursor = false;
                state.focus = Some(Field::Search);
                state.operators = None;
                if state.catalog.variables.is_empty() && state.load_error.is_none() {
                    state.new_variable(false);
                    state.nested = true;
                }
            }
            Edit::Add(id) => state.apply_variable(None, id),
            Edit::Variable(i, id) => state.apply_variable(Some(i), id),
            Edit::Operator(i, operator) => {
                if state.operators == Some(i) {
                    if let Some(rule) = state.draft.rules.get_mut(i) {
                        rule.operator = operator;
                    }
                    state.operators = None;
                } else {
                    state.operators = Some(i);
                }
            }
            Edit::FlagRule(i, value) => {
                if let Some(rule) = state.draft.rules.get_mut(i)
                    && matches!(rule.value, Value::Flag(_))
                {
                    rule.value = Value::Flag(value);
                }
            }
            Edit::Remove(i) if i < state.draft.rules.len() => {
                state.draft.rules.remove(i);
                state.rule_inputs = std::mem::take(&mut state.rule_inputs)
                    .into_iter()
                    .filter_map(|(index, value)| {
                        (index != i).then(|| (index - usize::from(index > i), value))
                    })
                    .collect();
                state.chooser = None;
                state.operators = None;
            }
            Edit::Apply => {
                state.submitted = true;
                let mut draft = state.draft.clone();
                for (&i, input) in &state.rule_inputs {
                    if let Some(rule) = draft.rules.get_mut(i) {
                        let Ok(value) = input.trim().parse() else {
                            state.focus = Some(Field::Rule(i));
                            return;
                        };
                        rule.value = Value::Number(value);
                    }
                }
                if draft.is_empty() {
                    draft.any = false;
                }
                let Some((id, answer)) = state.anchor else {
                    return;
                };
                let Some(doc) = editor.narrative.current_mut() else {
                    return;
                };
                let mut content = doc.content.clone();
                let Content::Dialogue(d) = &mut content else {
                    return;
                };
                let Some(n) = d.node_mut(id) else { return };
                let Some(condition) = condition_mut(n, answer) else {
                    return;
                };
                *condition = draft;
                if !n.has_otherwise() {
                    n.otherwise = None;
                }
                if doc.content != content {
                    doc.checkpoint();
                    doc.content = content;
                    ui.generation += 1;
                }
                ui.picker = None;
            }
            _ => {}
        },
        _ => {}
    }
}

fn button(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: impl Into<String>,
    edit: Edit,
) -> Entity {
    act(
        commands,
        parent,
        font,
        label,
        NarrativeAction::Conditions(edit),
        false,
    )
}

pub(super) fn trigger(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    id: u32,
    answer: Option<usize>,
    condition: &Condition,
    catalog: &Catalog,
) {
    let count = condition.rules.len();
    let invalid = condition.rules.iter().any(|rule| {
        catalog.variables.get(&rule.variable).is_none_or(|var| {
            std::mem::discriminant(&var.default) != std::mem::discriminant(&rule.value)
        })
    });
    let summary = condition.rules.first().map(|rule| {
        let name = catalog
            .variables
            .get(&rule.variable)
            .map(|v| v.name.as_str())
            .unwrap_or("Переменная отсутствует");
        format!(
            "{name} {} {}{}",
            rule.operator.label(),
            rule.value.label(),
            if count > 1 {
                format!(" · ещё {}", count - 1)
            } else {
                String::new()
            }
        )
    });
    let label = answer.map_or_else(
        || format!("Условие ноды #{id}"),
        |i| format!("Условие ответа {} ноды #{id}", i + 1),
    );
    let action = NarrativeAction::Conditions(Edit::Open(id, answer));
    let entity = if answer.is_some() {
        icons::button(
            commands,
            parent,
            icons::Glyph::Check,
            &label,
            Action::Narrative(action),
            if count == 0 {
                Look::Regular
            } else {
                Look::Primary
            },
        )
    } else {
        let caption = summary.as_deref().map_or_else(
            || "Условие".to_owned(),
            |description| {
                if description.chars().count() > 21 {
                    format!(
                        "{}… · {count}",
                        description.chars().take(17).collect::<String>()
                    )
                } else {
                    description.to_owned()
                }
            },
        );
        let entity = act(
            commands,
            parent,
            font,
            if invalid {
                format!("{caption} !")
            } else {
                caption
            },
            action,
            count != 0,
        );
        commands
            .entity(entity)
            .entry::<Node>()
            .and_modify(|mut node| node.max_width = px(152));
        entity
    };
    let help = summary.unwrap_or_else(|| label.clone());
    commands.entity(entity).insert((
        Anchor(id, answer),
        AccessibleLabel(label.clone()),
        control_hints::Help(if invalid {
            format!("Проверьте условие · {help}")
        } else {
            help
        }),
    ));
}

#[derive(Component)]
pub(super) struct Anchor(pub u32, pub Option<usize>);

#[derive(Component)]
pub(super) struct Input(Field);

#[derive(Component)]
pub(super) struct TypeMenu {
    trigger: Entity,
    chevron: Entity,
    options: [Entity; 2],
}

pub(super) fn update_type_menu(
    mut commands: Commands,
    mut ui: ResMut<NarrativeUi>,
    mut menus: Query<(Entity, &TypeMenu, &mut Node)>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut focus: ResMut<InputFocus>,
) {
    if !matches!(ui.picker, Some(Picker::Variables | Picker::Condition)) {
        return;
    }
    let Ok((popup, menu, mut layout)) = menus.single_mut() else {
        return;
    };
    let Some(draft) = &mut ui.conditions.variable else {
        return;
    };
    let visible = layout.display != Display::None;
    let on_trigger = focus.get() == Some(menu.trigger);
    let on_option = focus
        .get()
        .is_some_and(|entity| menu.options.contains(&entity));
    let down = keys.just_pressed(KeyCode::ArrowDown);
    let up = keys.just_pressed(KeyCode::ArrowUp);

    if visible && draft.type_menu_open {
        let outside_press = mouse.any_just_pressed([MouseButton::Left, MouseButton::Right])
            && window.physical_cursor_position().is_some_and(|point| {
                ![menu.trigger, popup].into_iter().any(|entity| {
                    geometry
                        .get(entity)
                        .is_ok_and(|(n, t)| n.contains_point(*t, point))
                })
            });
        if !window.focused || outside_press || (focus.get().is_some() && !on_option && !on_trigger)
        {
            draft.type_menu_open = false;
        } else if down || up {
            focus.set(
                menu.options[usize::from(down)],
                bevy::input_focus::FocusCause::Navigated,
            );
        }
    } else if !visible && on_trigger && (down || up) {
        draft.type_menu_open = true;
    }

    if visible == draft.type_menu_open {
        return;
    }
    let open = draft.type_menu_open;
    commands
        .entity(menu.trigger)
        .entry::<bevy::a11y::AccessibilityNode>()
        .and_modify(move |mut node| node.set_expanded(open));
    if draft.type_menu_open {
        layout.display = Display::Flex;
        icons::set_glyph(&mut commands, menu.chevron, icons::Glyph::ChevronUp);
        focus.set(
            menu.options[usize::from(draft.integer)],
            bevy::input_focus::FocusCause::Navigated,
        );
    } else {
        layout.display = Display::None;
        icons::set_glyph(&mut commands, menu.chevron, icons::Glyph::ChevronDown);
        if on_option || focus.get().is_none() {
            if window.focused {
                focus.set(menu.trigger, bevy::input_focus::FocusCause::Auto);
            } else {
                focus.clear();
            }
        }
    }
}

#[derive(Component)]
pub(super) struct SearchResult(u64);

#[derive(Component)]
pub(super) enum SearchPart {
    Placeholder,
    Clear,
    Headings,
    Empty,
    Cursor(u64),
}

#[derive(Component, Default)]
pub(super) struct ResultList {
    reveal: Option<Entity>,
}

#[derive(Component)]
pub(super) enum StatusLabel {
    Test(u64),
    Number(usize),
    Variable(Field),
    VariableExample,
}

pub(super) fn refresh_status(
    ui: Res<NarrativeUi>,
    mut labels: Query<(&StatusLabel, &mut Text, Option<&mut Node>)>,
) {
    if labels.is_empty() {
        return;
    }
    let state = &ui.conditions;
    let values = ui.preview.then(|| state.values());
    for (label, mut text, node) in &mut labels {
        let caption = match label {
            StatusLabel::Test(id) => {
                let Some(var) = state.catalog.variables.get(id) else {
                    continue;
                };
                if state.tests.get(id).is_some_and(|value| {
                    matches!(var.default, Value::Number(_)) && value.trim().parse::<i64>().is_err()
                }) {
                    "Введите целое число".into()
                } else if values
                    .as_ref()
                    .and_then(|values| values.get(id))
                    .is_some_and(|value| *value != var.default)
                {
                    format!("Изменено · по умолчанию {}", var.default.label())
                } else {
                    String::new()
                }
            }
            StatusLabel::Number(index)
                if state.submitted
                    && state
                        .rule_inputs
                        .get(index)
                        .is_some_and(|value| value.trim().parse::<i64>().is_err()) =>
            {
                "Введите целое число.".into()
            }
            StatusLabel::Number(_) => String::new(),
            StatusLabel::Variable(field) => state
                .variable
                .as_ref()
                .and_then(|draft| draft.error(*field))
                .unwrap_or_default()
                .into(),
            StatusLabel::VariableExample => state
                .variable
                .as_ref()
                .filter(|draft| draft.name.is_empty())
                .map_or("", |draft| {
                    if draft.integer {
                        "Например, «Уровень доверия»"
                    } else {
                        "Например, «Маяк найден»"
                    }
                })
                .into(),
        };
        if matches!(
            label,
            StatusLabel::Variable(_) | StatusLabel::VariableExample
        ) && let Some(mut node) = node
        {
            let display = if caption.is_empty() {
                Display::None
            } else {
                Display::Flex
            };
            if node.display != display {
                node.display = display;
            }
        }
        if text.0 != caption {
            text.0 = caption;
        }
    }
}

pub(super) fn field_borders(
    ui: Res<NarrativeUi>,
    focus: Res<InputFocus>,
    mut fields: Query<(Entity, &Input, &mut BorderColor)>,
) {
    if !matches!(ui.picker, Some(Picker::Variables | Picker::Condition)) {
        return;
    }
    for (entity, input, mut border) in &mut fields {
        if !matches!(input.0, Field::Name | Field::Default | Field::Search) {
            continue;
        }
        let invalid = ui
            .conditions
            .variable
            .as_ref()
            .is_some_and(|draft| draft.error(input.0).is_some());
        let tint = if invalid {
            ERROR
        } else if focus.get() == Some(entity) {
            FOCUS.with_alpha(0.55)
        } else {
            EDGE
        };
        border.set_if_neq(BorderColor::all(tint));
    }
}

pub(super) fn filter_results(
    ui: Res<NarrativeUi>,
    mut lists: Query<(Entity, &mut ResultList, &mut ScrollPosition)>,
    mut rows: Query<(Entity, &SearchResult, &mut Node)>,
    mut parts: Query<(&SearchPart, &mut Node), Without<SearchResult>>,
    mut previous: Local<Option<(Entity, String, usize, bool)>>,
) {
    let Ok((list, mut view, mut scroll)) = lists.single_mut() else {
        return;
    };
    let state = &ui.conditions;
    if previous.as_ref().is_some_and(|(id, query, index, mark)| {
        *id == list
            && query == &state.search
            && *index == state.cursor
            && *mark == state.mark_cursor
    }) {
        return;
    }
    *previous = Some((list, state.search.clone(), state.cursor, state.mark_cursor));
    let results = state.results();
    let marked = results
        .get(state.cursor)
        .copied()
        .filter(|_| state.mark_cursor);
    let visible: std::collections::HashSet<_> = results.iter().copied().collect();
    view.reveal = None;
    for (entity, row, mut node) in &mut rows {
        node.display = if visible.contains(&row.0) {
            Display::Flex
        } else {
            Display::None
        };
        if marked == Some(row.0) {
            view.reveal = Some(entity);
        }
    }
    if marked.is_none() {
        scroll.0.y = 0.;
    }
    for (part, mut node) in &mut parts {
        let show = match part {
            SearchPart::Placeholder => state.search.is_empty(),
            SearchPart::Clear => !state.search.is_empty(),
            SearchPart::Empty => results.is_empty(),
            SearchPart::Headings => !results.is_empty(),
            SearchPart::Cursor(id) => marked == Some(*id),
        };
        node.display = if show { Display::Flex } else { Display::None };
    }
}

pub(super) fn reveal_result(
    mut lists: Query<(
        &mut ResultList,
        &mut ScrollPosition,
        &ComputedNode,
        &UiGlobalTransform,
    )>,
    rows: Query<(&ComputedNode, &UiGlobalTransform), With<SearchResult>>,
) {
    for (mut view, mut scroll, list, origin) in &mut lists {
        let Some(entity) = view.reveal.take() else {
            continue;
        };
        let Ok((item, at)) = rows.get(entity) else {
            continue;
        };
        let top =
            at.translation.y - item.size().y * 0.5 - (origin.translation.y - list.size().y * 0.5);
        let overflow = top + item.size().y - list.size().y;
        let delta = if top < 0. { top } else { overflow.max(0.) };
        scroll.0.y = (scroll.0.y + delta * list.inverse_scale_factor()).max(0.);
    }
}

pub(super) fn keyboard(
    mut ui: ResMut<NarrativeUi>,
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<InputFocus>,
    inputs: Query<(&Input, &EditableText)>,
    mut actions: MessageWriter<Action>,
) {
    if let Some(action) = ui.conditions.resume.take() {
        actions.write(Action::Narrative(action));
    }
    let Some((input, editable)) = focus.get().and_then(|entity| inputs.get(entity).ok()) else {
        return;
    };
    if !matches!(ui.picker, Some(Picker::Condition | Picker::Variables)) || editable.is_composing()
    {
        return;
    }
    let submit = keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]);
    let picker = ui.picker;
    let state = &mut ui.conditions;
    if matches!(input.0, Field::Name | Field::Default)
        && submit
        && state.confirm.is_none()
        && state.leaving.is_none()
        && (picker == Some(Picker::Variables) || state.nested)
    {
        actions.write(Action::Narrative(NarrativeAction::Conditions(
            Edit::SaveVariable,
        )));
        return;
    }
    if input.0 != Field::Search {
        return;
    }
    if !submit && !keys.any_just_pressed([KeyCode::ArrowDown, KeyCode::ArrowUp]) {
        return;
    }
    let results = state.results();
    if keys.just_pressed(KeyCode::ArrowDown) {
        if state.mark_cursor {
            state.cursor = (state.cursor + 1).min(results.len().saturating_sub(1));
        }
    } else if keys.just_pressed(KeyCode::ArrowUp) {
        state.cursor = state.cursor.saturating_sub(1);
    } else if submit {
        let Some(&id) = results.get(state.cursor) else {
            return;
        };
        let action = if picker == Some(Picker::Variables) {
            Edit::Select(id)
        } else {
            state
                .chooser
                .flatten()
                .map_or(Edit::Add(id), |row| Edit::Variable(row, id))
        };
        actions.write(Action::Narrative(NarrativeAction::Conditions(action)));
        return;
    }
    state.mark_cursor = true;
}

fn input(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    value: &str,
    field: Field,
    focus: Option<Field>,
) -> Entity {
    let entity = input_field(
        commands,
        parent,
        font,
        label,
        value,
        DraftField::Condition(field),
        false,
    );
    commands.entity(entity).insert(Input(field));
    if focus == Some(field) {
        commands.queue(move |world: &mut World| {
            world
                .resource_mut::<InputFocus>()
                .set(entity, bevy::input_focus::FocusCause::Auto);
        });
    }
    entity
}

fn scroll(commands: &mut Commands, parent: Entity) -> Entity {
    let panel = node(
        commands,
        parent,
        Node {
            width: percent(100),
            min_height: px(0),
            max_height: px(240),
            flex_shrink: 1.,
            overflow: Overflow::scroll_y(),
            ..column(7.)
        },
    );
    commands.entity(panel).insert(Scroller);
    panel
}

fn wrap() -> Node {
    Node {
        flex_wrap: FlexWrap::Wrap,
        row_gap: px(6),
        ..row(6.)
    }
}

fn toggle(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    action: Edit,
    selected: bool,
) {
    act(
        commands,
        parent,
        font,
        label,
        NarrativeAction::Conditions(action),
        selected,
    );
}

fn variable_error(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    draft: &VariableDraft,
    field: Field,
) {
    let error = draft.error(field);
    let entity = text(
        commands,
        parent,
        font,
        error.unwrap_or_default(),
        11.,
        AMBER,
    );
    commands.entity(entity).insert((
        StatusLabel::Variable(field),
        Node {
            display: if error.is_some() {
                Display::Flex
            } else {
                Display::None
            },
            ..default()
        },
    ));
}

pub(super) fn panel(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    ui: &NarrativeUi,
) {
    let state = &ui.conditions;
    if let Some(error) = state.error.as_ref().or(state.load_error.as_ref()) {
        wrapped(commands, parent, font, error, 13., AMBER);
    }
    if let Some(pending) = state.confirm {
        let removing = pending == Edit::RemoveVariable;
        let name = state
            .variable
            .as_ref()
            .and_then(|draft| draft.id)
            .and_then(|id| state.catalog.variables.get(&id))
            .map_or("переменной", |var| var.name.as_str());
        wrapped(
            commands,
            parent,
            font,
            if removing {
                format!("Условия с переменной «{name}» перестанут выполняться.")
            } else {
                format!("Условия с прежним типом переменной «{name}» потребуется обновить.")
            },
            14.,
            TEXT,
        );
        let controls = node(commands, parent, wrap());
        button(commands, controls, font, "Отмена", Edit::CancelConfirm);
        button(
            commands,
            controls,
            font,
            if removing {
                "Удалить"
            } else {
                "Изменить тип"
            },
            Edit::Confirm,
        );
        return;
    }
    if ui.picker == Some(Picker::Variables) {
        if state.leaving.is_some() {
            let creating = state.creating_variable(ui.picker);
            wrapped(
                commands,
                parent,
                font,
                if creating {
                    "Создать переменную перед выходом?"
                } else {
                    "Сохранить изменения переменной?"
                },
                13.,
                TEXT,
            );
            let controls = node(commands, parent, wrap());
            button(
                commands,
                controls,
                font,
                if creating {
                    "Создать"
                } else {
                    "Сохранить переменную"
                },
                Edit::SaveVariable,
            );
            button(
                commands,
                controls,
                font,
                "Отменить изменения",
                Edit::Discard,
            );
            button(
                commands,
                controls,
                font,
                "Продолжить редактирование",
                Edit::Stay,
            );
            return;
        }
        if state.variable.is_some() {
            variable_ui::form(commands, parent, font, state);
        } else if state.catalog.variables.is_empty() {
            if state.load_error.is_none() {
                wrapped(
                    commands,
                    parent,
                    font,
                    "В проекте пока нет переменных.",
                    14.,
                    MUTED,
                );
                let actions = node(commands, parent, row(0.));
                toggle(
                    commands,
                    actions,
                    font,
                    "Создать переменную",
                    Edit::New,
                    true,
                );
            }
        } else {
            variable_ui::search_list(commands, parent, font, state, true);
        }
        return;
    }
    if state.creating_variable(ui.picker) {
        variable_ui::form(commands, parent, font, state);
        return;
    }
    if state.chooser.is_some() {
        variable_ui::search_list(commands, parent, font, state, false);
        text(
            commands,
            parent,
            font,
            "↑ ↓  выбрать · Enter  применить",
            10.,
            MUTED,
        );
        let footer = node(commands, parent, wrap());
        button(
            commands,
            footer,
            font,
            "Создать переменную",
            Edit::NestedNew,
        );
        button(commands, footer, font, "Назад к условию", Edit::Close);
        return;
    }
    if state.draft.rules.len() > 1 {
        let modes = node(commands, parent, wrap());
        toggle(
            commands,
            modes,
            font,
            "Все условия",
            Edit::Mode(false),
            !state.draft.any,
        );
        toggle(
            commands,
            modes,
            font,
            "Хотя бы одно",
            Edit::Mode(true),
            state.draft.any,
        );
    }
    let list = scroll(commands, parent);
    if state.draft.rules.is_empty() {
        wrapped(
            commands,
            list,
            font,
            "Без проверок условие всегда выполняется.",
            12.,
            MUTED,
        );
    }
    for (i, rule) in state.draft.rules.iter().enumerate() {
        let section = node(commands, list, column(4.));
        let row = node(commands, section, wrap());
        let variable = state.catalog.variables.get(&rule.variable);
        let valid = variable.is_some_and(|var| {
            std::mem::discriminant(&var.default) == std::mem::discriminant(&rule.value)
        });
        let label = variable.map_or_else(
            || format!("Переменная #{}", rule.variable),
            |var| var.name.clone(),
        );
        let variable_field = node(
            commands,
            row,
            Node {
                min_width: px(110),
                max_width: percent(100),
                flex_grow: 1.,
                ..column(2.)
            },
        );
        text(commands, variable_field, font, "Переменная", 10., MUTED);
        let selector = button(
            commands,
            variable_field,
            font,
            label,
            Edit::Variable(i, u64::MAX),
        );
        commands
            .entity(selector)
            .entry::<Node>()
            .and_modify(|mut node| node.max_width = px(200));
        let compare = node(commands, row, column(2.));
        text(commands, compare, font, "Сравнение", 10., MUTED);
        button(
            commands,
            compare,
            font,
            rule.operator.label(),
            Edit::Operator(i, rule.operator),
        );
        let values = node(
            commands,
            row,
            Node {
                min_width: px(82),
                ..column(2.)
            },
        );
        text(commands, values, font, "Значение", 10., MUTED);
        match rule.value {
            Value::Flag(current) => {
                let choices = node(commands, values, wrap());
                for (value, label) in [(true, "Да"), (false, "Нет")] {
                    toggle(
                        commands,
                        choices,
                        font,
                        label,
                        Edit::FlagRule(i, value),
                        current == value,
                    );
                }
            }
            Value::Number(value) => {
                let value = state
                    .rule_inputs
                    .get(&i)
                    .cloned()
                    .unwrap_or_else(|| value.to_string());
                input(
                    commands,
                    values,
                    font,
                    "Значение условия",
                    &value,
                    Field::Rule(i),
                    state.focus,
                );
            }
        }
        let remove = button(commands, row, font, "×", Edit::Remove(i));
        commands
            .entity(remove)
            .insert(AccessibleLabel(format!("Убрать проверку {}", i + 1)));
        if !valid {
            wrapped(
                commands,
                section,
                font,
                "Переменная удалена или сменила тип. Выберите переменную заново.",
                11.,
                AMBER,
            );
        }
        if matches!(rule.value, Value::Number(_)) {
            let status = text(commands, section, font, "", 11., AMBER);
            commands.entity(status).insert(StatusLabel::Number(i));
        }
        if state.operators == Some(i) {
            let controls = node(commands, section, wrap());
            for operator in Operator::ALL {
                if matches!(rule.value, Value::Number(_))
                    || matches!(operator, Operator::Eq | Operator::Ne)
                {
                    toggle(
                        commands,
                        controls,
                        font,
                        operator.label(),
                        Edit::Operator(i, operator),
                        rule.operator == operator,
                    );
                }
            }
        }
    }
    let footer = node(commands, parent, wrap());
    button(commands, footer, font, "+ Проверка", Edit::Add(u64::MAX));
    toggle(commands, footer, font, "Применить", Edit::Apply, true);
}

pub(super) fn preview(commands: &mut Commands, parent: Entity, font: &Handle<Font>, state: &State) {
    if state.catalog.variables.is_empty() {
        return;
    }
    let values = state.values();
    let changed = values
        .iter()
        .filter(|(id, value)| state.catalog.variables[*id].default != **value)
        .count();
    let arrow = if state.preview_open { "−" } else { "+" };
    let title = if changed > 0 {
        format!("{arrow} Тестовые значения · {changed} изменено")
    } else {
        format!("{arrow} Тестовые значения")
    };
    button(commands, parent, font, title, Edit::PreviewPanel);
    if !state.preview_open {
        return;
    }
    text(
        commands,
        parent,
        font,
        "Начните диалог заново, чтобы применить значения.",
        11.,
        MUTED,
    );
    let list = scroll(commands, parent);
    for (&id, var) in &state.catalog.variables {
        let row = node(commands, list, wrap());
        let changed = values[&id] != var.default;
        text(commands, row, font, &var.name, 12., TEXT);
        match values[&id] {
            value @ Value::Flag(_) => toggle(
                commands,
                row,
                font,
                &value.label(),
                Edit::TestFlag(id),
                changed,
            ),
            Value::Number(_) => {
                let value = state
                    .tests
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| var.default.label());
                let field = node(
                    commands,
                    row,
                    Node {
                        width: px(110),
                        ..column(2.)
                    },
                );
                input(
                    commands,
                    field,
                    font,
                    &var.name,
                    &value,
                    Field::Test(id),
                    state.focus,
                );
            }
        }
        let status = text(commands, row, font, "", 11., FOCUS);
        commands.entity(status).insert(StatusLabel::Test(id));
    }
    button(
        commands,
        parent,
        font,
        "Сбросить тестовые значения",
        Edit::ResetTests,
    );
}
