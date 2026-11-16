//! Material Paint workspace state. Rendering and mesh editing stay isolated.
use super::*;
use crate::editor::model_document::Mode;
pub(in crate::editor) use crate::editor::model_paint_data::LayerKind;
use crate::editor::model_paint_data::{Layer, Stack, Surface};
use std::collections::HashMap;
#[path = "model_paint_engine.rs"]
pub(super) mod engine;

#[path = "model_paint_ui.rs"]
mod ui;
pub(super) use ui::{refresh, setup};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum PaintTool {
    Brush,
    Eraser,
    Fill,
    Picker,
    Path,
}
impl PaintTool {
    pub fn label(self) -> &'static str {
        match self {
            Self::Brush => "Кисть",
            Self::Eraser => "Ластик",
            Self::Fill => "Заливка полигонов",
            Self::Picker => "Пипетка",
            Self::Path => "Контур",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Panel {
    Layers,
    Maps,
    Sets,
    Preview,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum View {
    Split,
    Three,
    Two,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Category {
    Materials,
    Smart,
    Brushes,
    Textures,
    Generators,
    Graphs,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Parameter {
    Size,
    Flow,
    Opacity,
    Hardness,
    Spacing,
    Roughness,
    Metallic,
    LayerOpacity,
}
impl Parameter {
    fn label(self) -> &'static str {
        match self {
            Self::Size => "Размер",
            Self::Flow => "Поток, %",
            Self::Opacity => "Сила, %",
            Self::Hardness => "Жёсткость, %",
            Self::Spacing => "Шаг, %",
            Self::Roughness => "Шероховатость, %",
            Self::Metallic => "Металличность, %",
            Self::LayerOpacity => "Непрозрачность, %",
        }
    }
    fn bounds(self) -> (f64, f64) {
        match self {
            Self::Size => (1., 512.),
            Self::Spacing => (1., 100.),
            _ => (0., 100.),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum PaintAction {
    Tool(PaintTool),
    Panel(Panel),
    View(View),
    Category(Category),
    Channel(usize),
    Add(LayerKind),
    Select(usize),
    Visible(usize),
    Remove,
    Duplicate,
    Move(i8),
    Undo,
    Redo,
    Set(usize),
    Color(usize),
    Preset(usize),
    Reset,
}
impl PaintAction {
    pub(in crate::editor) fn hint(self) -> &'static str {
        match self {
            Self::Tool(PaintTool::Brush) => {
                "B · Кисть. Размер, поток и жёсткость находятся в свойствах."
            }
            Self::Tool(PaintTool::Eraser) => "E · Ластик.",
            Self::Tool(PaintTool::Picker) => "I · Пипетка материала.",
            Self::Tool(PaintTool::Fill) => "Выбрать инструмент заливки полигонов.",
            Self::Tool(PaintTool::Path) => "Выбрать инструмент контура.",
            Self::Add(_) => "Добавить слой в стек выбранного набора текстур.",
            Self::Remove => "Удалить выбранный слой.",
            Self::Duplicate => "Создать копию выбранного слоя.",
            Self::Undo => "Ctrl+Z · Отменить изменение стека слоёв.",
            Self::Redo => "Ctrl+Shift+Z · Повторить изменение стека слоёв.",
            Self::Visible(_) => "Включить или выключить слой.",
            Self::Move(_) => "Изменить порядок слоя.",
            Self::Select(_) => "Выбрать слой и показать его свойства.",
            Self::Set(_) => "Выбрать объект и набор текстур модели.",
            Self::Channel(_) => "Выбрать канал материала.",
            Self::Category(_) => "Открыть категорию библиотеки.",
            Self::Color(_) => "Выбрать цвет кисти.",
            Self::Preset(_) => "Выбрать параметры материала.",
            Self::View(_) => "Изменить расположение 3D- и 2D-вида.",
            Self::Panel(_) => "Открыть панель рабочего пространства рисования.",
            Self::Reset => "Восстановить расположение панелей: 3D и UV рядом, библиотека снизу.",
        }
    }
}
struct Session {
    stack: Stack,
    undo: Vec<Stack>,
    redo: Vec<Stack>,
    tool: PaintTool,
    panel: Panel,
    view: View,
    category: Category,
    channel: usize,
    set: usize,
    color: usize,
    preset: usize,
    size: f32,
    flow: f32,
    opacity: f32,
    hardness: f32,
    spacing: f32,
    roughness: f32,
    metallic: f32,
    revision: u64,
    binding: Option<(usize, usize)>,
    histories: HashMap<(usize, usize), (Vec<Stack>, Vec<Stack>)>,
    busy: bool,
    texture_size: u32,
    picked: Option<[u8; 3]>,
    queued: std::collections::VecDeque<PaintAction>,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            stack: Stack::default(),
            undo: vec![],
            redo: vec![],
            tool: PaintTool::Brush,
            panel: Panel::Layers,
            view: View::Split,
            category: Category::Materials,
            channel: 0,
            set: 0,
            color: 0,
            preset: 0,
            size: 70.,
            flow: 100.,
            opacity: 100.,
            hardness: 55.,
            spacing: 8.,
            roughness: 55.,
            metallic: 0.,
            revision: 0,
            binding: None,
            histories: HashMap::new(),
            busy: false,
            texture_size: 1024,
            picked: None,
            queued: Default::default(),
        }
    }
}
impl Session {
    fn available(&self, action: PaintAction) -> bool {
        if matches!(
            action,
            PaintAction::Tool(PaintTool::Path) | PaintAction::Add(LayerKind::Folder)
        ) {
            return false;
        }
        if self.busy
            && matches!(
                action,
                PaintAction::Add(_)
                    | PaintAction::Select(_)
                    | PaintAction::Visible(_)
                    | PaintAction::Remove
                    | PaintAction::Duplicate
                    | PaintAction::Move(_)
                    | PaintAction::Undo
                    | PaintAction::Redo
                    | PaintAction::Set(_)
            )
        {
            return false;
        }
        match action {
            PaintAction::Undo => !self.undo.is_empty(),
            PaintAction::Redo => !self.redo.is_empty(),
            PaintAction::Remove | PaintAction::Duplicate | PaintAction::Move(_) => {
                !self.stack.layers.is_empty()
            }
            _ => true,
        }
    }
    fn value(&self, parameter: Parameter) -> f32 {
        match parameter {
            Parameter::Size => self.size,
            Parameter::Flow => self.flow,
            Parameter::Opacity => self.opacity,
            Parameter::Hardness => self.hardness,
            Parameter::Spacing => self.spacing,
            Parameter::Roughness => self.roughness,
            Parameter::Metallic => self.metallic,
            Parameter::LayerOpacity => self
                .stack
                .layers
                .get(self.stack.selected)
                .map_or(100., |layer| layer.opacity),
        }
    }
    fn set_value(&mut self, parameter: Parameter, value: f32) {
        if self.value(parameter) == value {
            return;
        }
        if parameter == Parameter::LayerOpacity {
            if self.busy {
                return;
            }
            self.checkpoint();
        }
        let fill = self
            .stack
            .layers
            .get(self.stack.selected)
            .is_some_and(|l| l.kind == LayerKind::Fill);
        if fill && matches!(parameter, Parameter::Roughness | Parameter::Metallic) {
            if self.busy {
                return;
            }
            self.checkpoint();
        }
        match parameter {
            Parameter::Size => self.size = value,
            Parameter::Flow => self.flow = value,
            Parameter::Opacity => self.opacity = value,
            Parameter::Hardness => self.hardness = value,
            Parameter::Spacing => self.spacing = value,
            Parameter::Roughness => self.roughness = value,
            Parameter::Metallic => self.metallic = value,
            Parameter::LayerOpacity => {
                if let Some(layer) = self.stack.layers.get_mut(self.stack.selected) {
                    layer.opacity = value;
                }
            }
        }
        self.revision += 1;
        if fill && matches!(parameter, Parameter::Roughness | Parameter::Metallic) {
            self.fill_values();
        }
    }
    fn edit(&mut self, action: PaintAction) {
        if !self.available(action) {
            return;
        }
        let edit = matches!(
            action,
            PaintAction::Add(_)
                | PaintAction::Visible(_)
                | PaintAction::Remove
                | PaintAction::Duplicate
                | PaintAction::Move(_)
        );
        if edit {
            self.checkpoint();
        }
        let fill = self
            .stack
            .layers
            .get(self.stack.selected)
            .is_some_and(|l| l.kind == LayerKind::Fill)
            && matches!(action, PaintAction::Color(_) | PaintAction::Preset(_));
        if fill {
            if self.busy {
                return;
            }
            self.checkpoint();
        }
        match action {
            PaintAction::Tool(value) => self.tool = value,
            PaintAction::Panel(value) => self.panel = value,
            PaintAction::View(value) => self.view = value,
            PaintAction::Category(value) => self.category = value,
            PaintAction::Channel(value) => self.channel = value.min(7),
            PaintAction::Select(i) => {
                self.stack.selected = i.min(self.stack.layers.len().saturating_sub(1))
            }
            PaintAction::Add(kind) => {
                let name = match kind {
                    LayerKind::Paint => "Слой",
                    LayerKind::Fill => "Заливка",
                    LayerKind::Folder => "Группа",
                };
                let mut layer = Layer::new(format!("{name} {}", self.stack.layers.len() + 1), kind);
                if kind == LayerKind::Fill {
                    let color = self.brush_color();
                    layer.channels[0] = Some(crate::editor::model_paint_data::solid(
                        self.texture_size,
                        [color[0], color[1], color[2], 255],
                    ));
                    let r = (self.roughness * 2.55).round() as u8;
                    let m = (self.metallic * 2.55).round() as u8;
                    layer.channels[1] = Some(crate::editor::model_paint_data::solid(
                        self.texture_size,
                        [r, r, r, 255],
                    ));
                    layer.channels[2] = Some(crate::editor::model_paint_data::solid(
                        self.texture_size,
                        [m, m, m, 255],
                    ));
                }
                self.stack.layers.push(layer);
                self.stack.selected = self.stack.layers.len() - 1;
            }
            PaintAction::Visible(i) => {
                if let Some(layer) = self.stack.layers.get_mut(i) {
                    layer.visible = !layer.visible;
                }
            }
            PaintAction::Remove => {
                if !self.stack.layers.is_empty() {
                    self.stack.layers.remove(self.stack.selected);
                    self.stack.selected = self
                        .stack
                        .selected
                        .min(self.stack.layers.len().saturating_sub(1));
                }
            }
            PaintAction::Duplicate => {
                if let Some(mut layer) = self.stack.layers.get(self.stack.selected).cloned() {
                    layer.name.push_str(" — копия");
                    self.stack.layers.push(layer);
                    self.stack.selected = self.stack.layers.len() - 1;
                }
            }
            PaintAction::Move(direction) => {
                if !self.stack.layers.is_empty() {
                    let to = (self.stack.selected as isize + direction as isize)
                        .clamp(0, self.stack.layers.len() as isize - 1)
                        as usize;
                    self.stack.layers.swap(to, self.stack.selected);
                    self.stack.selected = to;
                }
            }
            PaintAction::Undo => {
                if let Some(stack) = self.undo.pop() {
                    self.redo.push(self.stack.clone());
                    self.stack = stack;
                }
            }
            PaintAction::Redo => {
                if let Some(stack) = self.redo.pop() {
                    self.undo.push(self.stack.clone());
                    self.stack = stack;
                }
            }
            PaintAction::Set(value) => self.set = value,
            PaintAction::Color(value) => {
                self.color = value.min(COLORS.len() - 1);
                self.picked = None;
            }
            PaintAction::Preset(value) => {
                self.preset = value.min(PRESETS.len() - 1);
                self.roughness = PRESETS[self.preset].2;
                self.metallic = PRESETS[self.preset].3;
                self.picked = Some(PRESETS[self.preset].1);
            }
            PaintAction::Reset => {
                self.view = View::Split;
                self.panel = Panel::Layers;
                self.category = Category::Materials;
            }
        }
        self.revision += 1;
        if fill {
            self.fill_values();
        }
    }
    fn fill_values(&mut self) {
        let color = self.brush_color();
        let size = self.texture_size;
        let Some(layer) = self.stack.layers.get_mut(self.stack.selected) else {
            return;
        };
        layer.channels[0] = Some(crate::editor::model_paint_data::solid(
            size,
            [color[0], color[1], color[2], 255],
        ));
        for (c, v) in [(1, self.roughness), (2, self.metallic)] {
            let v = (v * 2.55).round() as u8;
            layer.channels[c] = Some(crate::editor::model_paint_data::solid(size, [v, v, v, 255]));
        }
    }
    fn checkpoint(&mut self) {
        if self.undo.len() >= 32 {
            self.undo.remove(0);
        }
        self.undo.push(self.stack.clone());
        self.redo.clear();
    }
    fn brush_color(&self) -> [u8; 3] {
        self.picked.unwrap_or(COLORS[self.color])
    }
    fn active(&self, action: PaintAction) -> bool {
        match action {
            PaintAction::Tool(value) => self.tool == value,
            PaintAction::Panel(value) => self.panel == value,
            PaintAction::View(value) => self.view == value,
            PaintAction::Category(value) => self.category == value,
            PaintAction::Channel(value) => self.channel == value,
            PaintAction::Select(value) => self.stack.selected == value,
            PaintAction::Set(value) => self.set == value,
            PaintAction::Color(value) => self.color == value,
            PaintAction::Preset(value) => self.preset == value,
            _ => false,
        }
    }
}
#[derive(Resource, Default)]
pub(in crate::editor) struct PaintState {
    sessions: HashMap<usize, Session>,
}
const CHANNELS: [&str; 8] = [
    "Цвет",
    "Шероховатость",
    "Металличность",
    "Высота",
    "Непрозрачность",
    "Свечение",
    "Нормали",
    "Блики",
];
const COLORS: [[u8; 3]; 8] = [
    [224, 186, 122],
    [224, 224, 230],
    [46, 48, 58],
    [164, 61, 68],
    [67, 118, 166],
    [80, 128, 104],
    [135, 96, 166],
    [193, 130, 84],
];
const PRESETS: [(&str, [u8; 3], f32, f32); 6] = [
    ("Золото", [209, 165, 77], 24., 100.),
    ("Сталь", [155, 166, 183], 35., 100.),
    ("Медь", [190, 113, 77], 38., 100.),
    ("Керамика", [222, 218, 207], 18., 0.),
    ("Матовый пластик", [78, 95, 126], 65., 0.),
    ("Резина", [38, 41, 48], 87., 0.),
];

pub(super) fn actions(
    mut messages: MessageReader<Action>,
    editor: Res<Editor>,
    mut paint: ResMut<PaintState>,
    mut editing: ResMut<editing::State>,
) {
    let Some(index) = editor.viewing_model else {
        return;
    };
    if editor.pending.is_some()
        || editor.viewing_image.is_some()
        || !editing
            .documents
            .get(&index)
            .is_some_and(|doc| doc.mode == Mode::Paint)
    {
        return;
    }
    let session = paint.sessions.entry(index).or_default();
    for action in messages.read() {
        if let Action::ModelView(ModelViewAction::Paint(action)) = *action {
            if !session.available(action) {
                if matches!(action, PaintAction::Undo | PaintAction::Redo) {
                    session.queued.push_back(action);
                }
                continue;
            }
            if let PaintAction::Set(set) = action {
                if let Some(doc) = editing.documents.get_mut(&index) {
                    let object = doc
                        .objects
                        .iter()
                        .flat_map(|object| {
                            (0..object.materials.len().max(1)).map(move |_| object.id)
                        })
                        .nth(set);
                    if let Some(id) = object {
                        doc.select_object(id, false);
                    }
                }
            }
            session.edit(action);
        }
    }
}

#[cfg(test)]
#[path = "model_paint_tests.rs"]
mod tests;
