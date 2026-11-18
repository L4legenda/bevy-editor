//! Layer commands and view state for the native Layers panel.
use super::*;
use photocraft_doc::{LayerContent, LayerId};
use serde_json::{Value, json};

#[path = "photo_effect_params.rs"]
mod effects;
#[path = "image_layer_forms.rs"]
mod forms;
#[path = "image_layer_ux.rs"]
pub(in crate::editor) mod ux;
pub(super) use forms::{FieldKind, Form};
#[path = "image_layers_ui.rs"]
pub(in crate::editor) mod ui;

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum LayerAction {
    Quick(&'static str),
    Group,
    FieldNumber(usize, f64),
    FieldStep(usize, f64),
    Advanced,
    Preview,
    BlendMore,
    PanelSection(PanelSection),
    Menu,
    Category(&'static str),
    Command(&'static str),
    Apply,
    Cancel,
    FieldChoice(usize, usize),
    FieldChoices(usize),
    FieldArray(usize, bool),
    FieldComplex(usize),
    FieldItem(usize, i8),
    Effects(usize),
    EffectEdit(usize, usize),
    EffectToggle(usize, usize),
    EffectRemove(usize, usize),
    Select(usize, bool, bool),
    Collapse(u64),
    Mask(usize),
    MaskPreview(bool),
    VectorMask(usize),
    FilterMask(usize),
    SmartFilter(usize, usize),
    SmartFilterVisible(usize, usize),
    Blend,
    SetBlend(photocraft_color::BlendMode),
    Fill(f32, bool),
    Lock(&'static str),
    ToggleLock(u64),
    Move(u64, u64, &'static str, bool),
    Properties,
    Copy,
    Paste,
    FilterKind(&'static str),
    FilterLabel(Option<photocraft_doc::LabelColor>),
    OnlySelected,
    OnlyVisible,
    Thumbnail(ThumbnailSize),
    ThumbnailContent,
    ResetFilter,
    SaveEffectDefault,
    ResetEffectDefault,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum PanelSection {
    Layers,
    Search,
    Settings,
    Locks,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum ThumbnailSize {
    None,
    Small,
    #[default]
    Medium,
    Large,
}
impl ThumbnailSize {
    pub fn pixels(self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::Small => 22.0,
            Self::Medium => 32.0,
            Self::Large => 48.0,
        }
    }
}

#[derive(Default)]
pub(in crate::editor) struct LayerState {
    pub advanced: bool,
    pub preview_enabled: bool,
    pub preview: Option<FormPreview>,
    pub blend_more: bool,
    pub search_open: bool,
    pub settings_open: bool,
    pub locks_open: bool,
    pub ui_revision: u64,
    pub menu: bool,
    pub category: &'static str,
    pub search: String,
    pub filter: String,
    pub form: Option<Form>,
    pub blend_menu: bool,
    pub fill_drag: bool,
    pub clipboard: Vec<photocraft_doc::Layer>,
    pub style_clipboard: Option<photocraft_engine::CopiedStyle>,
    pub filter_kind: &'static str,
    pub filter_label: Option<photocraft_doc::LabelColor>,
    pub only_selected: bool,
    pub only_visible: bool,
    pub thumbnail: ThumbnailSize,
    pub thumbnail_content: bool,
    pub libraries: Option<photo_document::LayerLibraries>,
    pub libraries_readonly: bool,
}

pub(in crate::editor) struct FormPreview {
    id: usize,
    command: &'static str,
    original: photo_document::LayerMove,
    params: Value,
}

impl LayerState {
    pub fn section_open(&self, section: PanelSection) -> bool {
        match section {
            PanelSection::Layers => !self.settings_open,
            PanelSection::Search => self.search_open,
            PanelSection::Settings => self.settings_open,
            PanelSection::Locks => self.locks_open,
        }
    }
    pub fn has_filter(&self) -> bool {
        !self.filter.is_empty()
            || !self.filter_kind.is_empty()
            || self.filter_label.is_some()
            || self.only_selected
            || self.only_visible
    }
}

pub(super) fn collapsed(layer: &photocraft_doc::Layer) -> bool {
    matches!(&layer.content,LayerContent::Group(g) if !g.expanded)
}
pub(super) fn tree_rows<'a>(
    doc: &'a PhotoDocument,
    state: &LayerState,
) -> Vec<(usize, (Vec<usize>, usize, &'a photocraft_doc::Layer))> {
    fn matches(
        layer: &photocraft_doc::Layer,
        filter: &str,
        doc: &PhotoDocument,
        state: &LayerState,
    ) -> bool {
        (layer.name.to_lowercase().contains(filter)
            && (state.filter_kind.is_empty() || layer_kind(layer) == state.filter_kind)
            && state.filter_label.is_none_or(|label| label == layer.label)
            && (!state.only_selected || doc.selected.contains(&layer.id))
            && (!state.only_visible
                || doc
                    .document
                    .path_of(layer.id)
                    .is_some_and(|p| doc.document.shown_at(&p))))
            || layer
                .children()
                .is_some_and(|children| children.iter().any(|l| matches(l, filter, doc, state)))
    }
    let filter = state.filter.to_lowercase();
    let filtering = !filter.is_empty()
        || !state.filter_kind.is_empty()
        || state.filter_label.is_some()
        || state.only_selected
        || state.only_visible;
    let mut rows = doc
        .document
        .walk()
        .into_iter()
        .enumerate()
        .filter(|(_, (path, _, layer))| {
            if !filtering {
                !(1..path.len()).any(|n| doc.document.layer_at(&path[..n]).is_some_and(collapsed))
            } else {
                matches(layer, &filter, doc, state)
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        a.1.0
            .iter()
            .zip(&b.1.0)
            .find_map(|(a, b)| (a != b).then(|| b.cmp(a)))
            .unwrap_or(a.1.0.len().cmp(&b.1.0.len()))
    });
    rows
}

pub(super) fn layer_kind(layer: &photocraft_doc::Layer) -> &'static str {
    match &layer.content {
        LayerContent::Raster(_) => "Растровый",
        LayerContent::Group(g) => {
            if g.artboard.is_some() {
                "Монтажная область"
            } else {
                "Группа"
            }
        }
        LayerContent::Adjustment(_) => "Коррекция",
        LayerContent::Fill(_) => "Заливка",
        LayerContent::Text(_) => "Текст",
        LayerContent::Shape(_) => "Фигура",
        LayerContent::Smart(_) => "Смарт-объект",
    }
}

pub(super) fn catalog() -> impl Iterator<Item = &'static photocraft_engine::CommandSpec> {
    photocraft_engine::command_specs().iter().filter(|s| {
        (s.id.starts_with("layer.")
            || s.id.starts_with("style.presets.")
            || s.id.starts_with("pattern.")
            || s.id.starts_with("gradient.presets.")
            || s.id.starts_with("shape.presets.")
            || s.id == "edit.definePattern"
            || (s.id.starts_with("filter.") && s.menu.first() == Some(&"Filter"))
            || matches!(
                s.id,
                "type.create" | "type.edit" | "type.setStyle" | "shape.create" | "shape.edit"
            ))
            && !matches!(
                s.id,
                "layer.select"
                    | "layer.setProps"
                    | "layer.layerContentOptions"
                    | "layer.layerStyle.replace"
                    | "layer.layerStyle.defaultFor"
                    | "layer.layerStyle.makeDefault"
            )
            && !s.id.ends_with(".list")
    })
}

pub(super) fn parse_color(text: &str) -> Option<[f32; 4]> {
    let text = text.trim().trim_start_matches('#');
    if text.len() == 8 {
        if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let mut color = super::color::parse_hex(&text[..6])?;
        color[3] = u8::from_str_radix(&text[6..], 16).ok()? as f32 / 255.0;
        Some(color)
    } else {
        super::color::parse_hex(text)
    }
}
pub(super) fn color_hex(color: [f32; 4]) -> String {
    let [r, g, b, a] = color.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    if a < 255 {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}")
    }
}

pub(super) fn title(id: &str) -> String {
    if let Some(kind) = id.strip_prefix("layer.newAdjustmentLayer.") {
        return match kind {
            "brightnessContrast" => "Яркость / контрастность",
            "levels" => "Уровни",
            "curves" => "Кривые",
            "hueSaturation" => "Цветовой тон / насыщенность",
            "vibrance" => "Сочность",
            "exposure" => "Экспозиция",
            "colorBalance" => "Цветовой баланс",
            "blackWhite" => "Чёрно-белое",
            "photoFilter" => "Фотофильтр",
            "channelMixer" => "Микширование каналов",
            "gradientMap" => "Карта градиента",
            "selectiveColor" => "Выборочная коррекция цвета",
            "invert" => "Инверсия",
            "posterize" => "Постеризация",
            "threshold" => "Порог",
            "colorLookup" => "Поиск цвета",
            _ => {
                return photocraft_engine::commands::find(id)
                    .map_or_else(|| kind.into(), |s| s.label.trim_end_matches('…').into());
            }
        }
        .into();
    }
    match id {
        "style.presets.apply" => "Применить сохранённый стиль",
        "style.presets.new" => "Сохранить стиль слоя",
        "style.presets.edit" => "Управление наборами стилей",
        "style.presets.reset" => "Восстановить встроенные стили",
        "pattern.presets.select" => "Выбрать узор",
        "pattern.presets.apply" => "Применить сохранённый узор",
        "pattern.presets.new" | "edit.definePattern" => "Сохранить изображение как узор",
        "pattern.presets.edit" => "Управление наборами узоров",
        "pattern.rename" => "Переименовать узор",
        "pattern.delete" => "Удалить узор из библиотеки",
        "pattern.import" => "Импортировать узоры",
        "pattern.export" => "Экспортировать узоры",
        "gradient.presets.apply" => "Заливка из сохранённого градиента",
        "gradient.presets.new" => "Сохранить градиент",
        "gradient.presets.edit" => "Управление наборами градиентов",
        "gradient.presets.reset" => "Восстановить встроенные градиенты",
        "shape.presets.place" => "Добавить сохранённую фигуру",
        "shape.presets.new" => "Сохранить фигуру",
        "shape.presets.edit" => "Управление наборами фигур",
        "shape.presets.reset" => "Восстановить встроенные фигуры",
        "layer.new.layer" => "Новый слой",
        "layer.new.group" => "Новая группа",
        "layer.groupLayers" | "layer.new.groupFromLayers" => "Сгруппировать выделенные",
        "layer.ungroupLayers" => "Разгруппировать",
        "layer.duplicate" => "Дублировать",
        "layer.delete" => "Удалить",
        "layer.renameLayer" => "Переименовать",
        "layer.mergeLayers" => "Объединить выделенные",
        "layer.mergeDown" => "Объединить с нижним",
        "layer.mergeVisible" => "Объединить видимые",
        "layer.flattenImage" => "Свести изображение",
        "layer.layerMask.revealAll" => "Добавить белую маску",
        "layer.layerMask.hideAll" => "Добавить чёрную маску",
        "layer.layerMask.delete" => "Удалить маску",
        "layer.layerMask.apply" => "Применить маску",
        "layer.layerMask.edit" => "Плотность и растушёвка маски",
        "layer.layerMask.enabled" => "Включить / выключить маску",
        "layer.createClippingMask" => "Создать обтравочную маску",
        "layer.releaseClippingMask" => "Снять обтравочную маску",
        "layer.linkLayers" => "Связать слои",
        "layer.unlinkLayers" => "Разорвать связь",
        "layer.newFillLayer.solidColor" => "Цветовая заливка",
        "layer.newFillLayer.gradient" => "Градиентная заливка",
        "layer.newFillLayer.pattern" => "Заливка узором",
        "type.create" => "Новый текстовый слой",
        "type.edit" => "Содержимое текста",
        "type.setStyle" => "Шрифт и оформление текста",
        "shape.create" => "Новая векторная фигура",
        "shape.edit" => "Параметры векторной фигуры",
        "layer.layerStyle.dropShadow" => "Тень",
        "layer.layerStyle.innerShadow" => "Внутренняя тень",
        "layer.layerStyle.outerGlow" => "Внешнее свечение",
        "layer.layerStyle.innerGlow" => "Внутреннее свечение",
        "layer.layerStyle.stroke" => "Обводка",
        "layer.layerStyle.bevelEmboss" => "Тиснение",
        "layer.layerStyle.colorOverlay" => "Наложение цвета",
        "layer.layerStyle.gradientOverlay" => "Наложение градиента",
        "layer.layerStyle.patternOverlay" => "Наложение узора",
        "layer.layerStyle.satin" => "Глянец",
        "layer.layerStyle.clear" => "Очистить стиль",
        "layer.layerStyle.copyLayerStyle" => "Копировать стиль",
        "layer.layerStyle.pasteLayerStyle" => "Вставить стиль",
        "layer.layerStyle.blendingOptions" => "Параметры наложения",
        "layer.layerStyle.globalLight" => "Глобальное освещение",
        "layer.smartObjects.convertToSmartObject" => "Преобразовать в смарт-объект",
        "layer.rasterize.layer" => "Растрировать слой",
        "layer.quickExportAsPng" => "Экспортировать слой в PNG",
        "filter.other.highPass" => "Цветовой контраст",
        "filter.blur.gaussian" | "filter.blur.gaussianBlur" => "Размытие по Гауссу",
        "filter.sharpen.unsharpMask" => "Контурная резкость",
        _ => {
            return photocraft_engine::commands::find(id).map_or_else(
                || id.to_owned(),
                |s| s.label.trim_end_matches('…').to_owned(),
            );
        }
    }
    .into()
}

pub(super) fn dispatch(state: &mut State, id: usize, action: LayerAction) {
    use LayerAction::*;
    state.gesture = None;
    // View-only disclosures keep the existing tree, inputs and scroll position alive.
    if let PanelSection(section) = action {
        if matches!(
            section,
            self::PanelSection::Layers | self::PanelSection::Settings
        ) {
            state.layers.settings_open = matches!(section, self::PanelSection::Settings);
            return;
        }
        let open = match section {
            self::PanelSection::Layers => return,
            self::PanelSection::Search => &mut state.layers.search_open,
            self::PanelSection::Settings => &mut state.layers.settings_open,
            self::PanelSection::Locks => &mut state.layers.locks_open,
        };
        *open ^= true;
        state.layers.fill_drag = false;
        return;
    }
    if matches!(
        action,
        Cancel
            | Menu
            | Category(_)
            | Command(_)
            | Properties
            | Effects(_)
            | EffectEdit(..)
            | SmartFilter(..)
    ) {
        finish_preview(state, false);
        state.layers.advanced = false;
        state.layers.preview_enabled = true;
        state.error = None;
    }
    if let FieldNumber(index, value) | FieldStep(index, value) = action {
        if let Some(form) = state.layers.form.as_mut()
            && let Some(field) = form.fields.get_mut(index)
        {
            let value = if matches!(action, FieldStep(..)) {
                field.text.parse::<f64>().unwrap_or(0.0) + value
            } else {
                value
            };
            if value.is_finite() {
                let current = field.text.parse::<f64>().unwrap_or(0.0);
                let (min, max, _) = ux::number(form.command, &field.key, current);
                let value = value.clamp(min, max);
                field.text = format!("{value:.4}")
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_owned();
                field.enabled = true;
            }
        }
        // Do not rebuild a captured slider while it is being dragged.
        return;
    }
    if matches!(action, Preview) {
        state.layers.preview_enabled ^= true;
        if !state.layers.preview_enabled {
            finish_preview(state, false);
        }
        state.layers.ui_revision += 1;
        return;
    }
    if matches!(action, Apply)
        && state.layers.preview_enabled
        && state
            .layers
            .form
            .as_ref()
            .is_some_and(|f| ux::can_preview(f.command))
    {
        preview(state, id);
        if state.error.is_none() {
            finish_preview(state, true);
            state.layers.form = None;
            state.layers.ui_revision += 1;
        }
        return;
    }
    state.layers.ui_revision = state.layers.ui_revision.wrapping_add(1);
    if !matches!(action, LayerAction::Fill(..)) {
        state.layers.fill_drag = false;
    }
    let colors = (state.color(), state.background());
    if state.layers.libraries.is_none() {
        state.layers.libraries = Some(
            match library_path()
                .filter(|p| p.exists())
                .map(|p| photo_document::LayerLibraries::load(&p))
            {
                Some(Ok(libraries)) => libraries,
                Some(Err(error)) => {
                    state.layers.libraries_readonly = true;
                    state.error = Some(format!("Не удалось прочитать наборы слоёв: {error}"));
                    photo_document::LayerLibraries::default()
                }
                None => photo_document::LayerLibraries::default(),
            },
        );
    }
    let Some(session) = state.sessions.get_mut(&id).filter(|s| s.editing) else {
        return;
    };
    let doc = &mut session.doc;
    doc.set_tool_colors(colors.0, colors.1);
    if let Some(libraries) = &state.layers.libraries {
        doc.set_libraries(libraries);
    }
    let mut call = None;
    match action {
        FieldNumber(..) | FieldStep(..) | Preview => {}
        Advanced => state.layers.advanced ^= true,
        BlendMore => state.layers.blend_more ^= true,
        Quick(command) => call = Some((command, json!({}))),
        Group => {
            call = Some((
                if doc.selected.len() > 1 {
                    "layer.groupLayers"
                } else {
                    "layer.new.group"
                },
                json!({"name":"Группа"}),
            ))
        }
        PanelSection(_) => unreachable!(),
        SaveEffectDefault => {
            if let Some(form) = &state.layers.form
                && let Some(kind) = form.command.strip_prefix("layer.layerStyle.")
            {
                match form.values() {
                    Ok(mut params) => {
                        if let Some(params) = params.as_object_mut() {
                            for key in ["layer", "layers", "document", "add", "coalesce"] {
                                params.remove(key);
                            }
                        }
                        call = Some((
                            "layer.layerStyle.makeDefault",
                            json!({"kind":kind,"params":params}),
                        ))
                    }
                    Err(error) => state.error = Some(error),
                }
            }
        }
        ResetEffectDefault => {
            if let Some(spec) = state
                .layers
                .form
                .as_ref()
                .and_then(|f| photocraft_engine::commands::find(f.command))
            {
                state.layers.form = Some(Form::new(spec, doc));
            }
        }
        FilterKind(kind) => state.layers.filter_kind = kind,
        FilterLabel(label) => state.layers.filter_label = label,
        OnlySelected => state.layers.only_selected ^= true,
        OnlyVisible => state.layers.only_visible ^= true,
        Thumbnail(size) => state.layers.thumbnail = size,
        ThumbnailContent => state.layers.thumbnail_content ^= true,
        ResetFilter => {
            state.layers.filter.clear();
            state.layers.filter_kind = "";
            state.layers.filter_label = None;
            state.layers.only_selected = false;
            state.layers.only_visible = false;
        }
        Copy => {
            state.layers.clipboard = doc.copy_layers();
            state.layers.menu = false;
        }
        Paste => {
            if let Err(error) = doc.paste_layers(&state.layers.clipboard) {
                state.error = Some(error);
            }
            state.layers.menu = false;
        }
        Menu => {
            state.layers.menu ^= true;
            state.layers.form = None;
            state.layers.blend_menu = false;
            state.layers.category = "";
        }
        Category(category) => {
            state.layers.category = category;
            state.layers.search.clear();
            state.layers.menu = true;
            state.layers.form = None;
            state.layers.blend_menu = false;
        }
        Cancel => {
            state.layers.form = None;
            state.layers.menu = false;
            state.layers.blend_menu = false;
        }
        Collapse(id) => {
            let expanded = doc.document.layer(LayerId(id)).is_some_and(collapsed);
            call = Some(("layer.setExpanded", json!({"layer":id,"expanded":expanded})));
        }
        Select(index, toggle, range) => {
            let ids = if range {
                let rows = tree_rows(doc, &state.layers);
                rows.iter()
                    .position(|(i, _)| *i == doc.active)
                    .zip(rows.iter().position(|(i, _)| *i == index))
                    .map(|(a, b)| {
                        rows[a.min(b)..=a.max(b)]
                            .iter()
                            .map(|(_, (_, _, l))| l.id)
                            .collect::<Vec<_>>()
                    })
            } else {
                None
            };
            doc.select(index, toggle, false);
            if let Some(ids) = ids {
                doc.selected = ids;
            }
        }
        Mask(index) => {
            doc.select(index, false, false);
            doc.mask_target = doc.active_layer().is_some_and(|l| l.mask.is_some());
        }
        MaskPreview(overlay) => {
            call = Some((
                "view.layerMask",
                json!({"mode":if overlay{"toggleOverlay"}else{"toggleGray"},"mask":if doc.filter_target{"filter"}else{"layer"}}),
            ))
        }
        VectorMask(index) => {
            doc.select(index, false, false);
            if let Some(spec) = photocraft_engine::commands::find("layer.vectorMask.edit") {
                state.layers.form = Some(Form::new(spec, doc));
            }
        }
        FilterMask(index) => {
            doc.select(index, false, false);
            match doc.ensure_filter_mask() {
                Ok(()) => call = Some(("view.layerMask", json!({"mask":"filter","mode":"gray"}))),
                Err(error) => state.error = Some(error),
            }
        }
        SmartFilter(index, filter) => {
            doc.select(index, false, false);
            if let Some(layer) = doc.active_layer()
                && let LayerContent::Smart(s) = &layer.content
                && let Some(value) = s.smart_filters.get(filter)
            {
                state.layers.form = Some(Form::structured(
                    "layer.smartFilter.setParams",
                    title(&value.command),
                    json!({"layer":layer.id.0,"index":filter,"params":value.params}),
                ));
            }
        }
        SmartFilterVisible(index, filter) => {
            if let Some(layer) = doc.layer(index)
                && let LayerContent::Smart(s) = &layer.content
                && let Some(value) = s.smart_filters.get(filter)
            {
                call = Some((
                    "layer.smartFilter.setVisible",
                    json!({"layer":layer.id.0,"index":filter,"visible":!value.visible}),
                ));
            }
        }
        Move(from, to, position, copy) => {
            call = Some((
                "layer.moveTo",
                json!({"layers": if doc.selected.contains(&LayerId(from)) { doc.selected.iter().map(|l| l.0).collect::<Vec<_>>() } else {vec![from]}, "target":to,"position":position,"copy":copy}),
            ))
        }
        SetBlend(blend) => {
            call = Some((
                "layer.setProps",
                json!({"layers": doc.selected.iter().map(|l| l.0).collect::<Vec<_>>(),"blend":blend.label()}),
            ));
            state.layers.blend_menu = false;
        }
        Blend => state.layers.blend_menu ^= true,
        ToggleLock(id) => {
            if let Some(layer) = doc.document.layer(LayerId(id)) {
                call = Some((
                    "layer.setProps",
                    json!({"layers":[id],"locks":{"all":!layer.locks.all}}),
                ));
            }
        }
        Lock(key) => {
            let locks = doc
                .active_id()
                .map(|id| doc.document.effective_locks(id))
                .unwrap_or_default();
            let locked = match key {
                "all" => locks.all,
                "pixels" => locks.pixels,
                "position" => locks.position,
                "transparency" => locks.transparency,
                _ => false,
            };
            call = Some((
                "layer.setProps",
                json!({"layers":doc.selected.iter().map(|l| l.0).collect::<Vec<_>>(),"locks":{key:!locked}}),
            ));
        }
        Fill(value, finished) => {
            if !state.layers.fill_drag {
                doc.checkpoint();
                state.layers.fill_drag = true;
            }
            for id in doc.selected.clone() {
                if let Some(l) = doc.document.layer_mut(id) {
                    l.fill_opacity = value.clamp(0.0, 1.0);
                }
            }
            doc.changed();
            if finished {
                state.layers.fill_drag = false;
            }
        }
        FieldChoice(index, choice) => {
            if let Some(field) = state
                .layers
                .form
                .as_mut()
                .and_then(|f| f.fields.get_mut(index))
            {
                field.choose(choice);
            }
            if let Some(form) = &mut state.layers.form {
                form.choice = None;
            }
        }
        FieldChoices(index) => {
            if let Some(form) = &mut state.layers.form {
                form.choice = if form.choice == Some(index) {
                    None
                } else {
                    Some(index)
                };
            }
        }
        FieldArray(index, add) => {
            if let Some(form) = &mut state.layers.form
                && let Err(error) = form.array(index, add)
            {
                state.error = Some(error);
            }
        }
        FieldComplex(index) => {
            if let Some(form) = &mut state.layers.form
                && let Err(error) = form.complex(index)
            {
                state.error = Some(error);
            }
        }
        FieldItem(index, action) => {
            if let Some(form) = &mut state.layers.form
                && let Err(error) = form.list_edit(index, action)
            {
                state.error = Some(error);
            }
        }
        Effects(index) => {
            doc.select(index, false, false);
            state.layers.settings_open = true;
        }
        EffectToggle(index, effect) | EffectRemove(index, effect) => {
            if let Some(layer) = doc.layer(index) {
                let mut items = layer.effects.items.clone();
                if matches!(action, EffectRemove(..)) {
                    if effect < items.len() {
                        items.remove(effect);
                    }
                } else if let Some(fx) = items.get_mut(effect) {
                    fx.set_enabled(!fx.enabled());
                }
                call = Some((
                    "layer.layerStyle.replace",
                    json!({"layer":layer.id.0,"effects":items.into_iter().map(|fx|json!({"kind":photocraft_engine::layer_style::kind_of(&fx),"params":{},"fx":fx})).collect::<Vec<_>>()}),
                ));
            }
        }
        EffectEdit(index, effect) => {
            doc.select(index, false, false);
            if let Some(layer) = doc.active_layer() {
                let effects=layer.effects.items.iter().map(|fx|json!({"kind":photocraft_engine::layer_style::kind_of(fx),"params":effects::values_of(fx,doc.document.global_light.angle),"on":fx.enabled(),"fx":serde_json::to_value(fx).unwrap_or(Value::Null)})).collect::<Vec<_>>();
                state.layers.form = Some(Form::structured(
                    "layer.layerStyle.replace",
                    "Эффекты слоя".into(),
                    json!({"layer":layer.id.0,"effects":effects}),
                ));
                if let Some(form) = state.layers.form.as_mut() {
                    form.effect = Some(effect);
                    form.title = layer
                        .effects
                        .items
                        .get(effect)
                        .map(|fx| {
                            title(&format!(
                                "layer.layerStyle.{}",
                                photocraft_engine::layer_style::kind_of(fx)
                            ))
                        })
                        .unwrap_or_else(|| "Эффект слоя".into());
                }
            }
        }
        Command(command) => {
            if let Some(spec) = photocraft_engine::commands::find(command) {
                if command == "layer.renameLayer" {
                    state.layers.settings_open = false;
                }
                let form = Form::new(spec, doc);
                if form.fields.is_empty() {
                    call = Some((command, form.params));
                } else {
                    state.layers.form = Some(form);
                }
                state.layers.menu = false;
            }
        }
        Properties => {
            if let Some(layer) = doc.active_layer() {
                if doc.mask_target
                    && doc.filter_target
                    && let LayerContent::Smart(smart) = &layer.content
                    && let Some(mask) = &smart.filter_mask
                {
                    state.layers.form = Some(Form::structured(
                        "editor.filterMaskOptions",
                        "Маска смарт-фильтров".into(),
                        json!({"layer":layer.id.0,"enabled":mask.enabled,"linked":mask.linked,"density":mask.density*100.0,"feather":mask.feather}),
                    ));
                    return;
                }
                let command = match &layer.content {
                    _ if doc.mask_target => "layer.layerMask.edit",
                    LayerContent::Adjustment(_) => "layer.setAdjustment",
                    LayerContent::Fill(photocraft_doc::Fill::Solid(_)) => {
                        "layer.fill.solidColor.set"
                    }
                    LayerContent::Fill(photocraft_doc::Fill::Gradient { .. }) => {
                        "gradient.fill.set"
                    }
                    LayerContent::Fill(photocraft_doc::Fill::Pattern { .. }) => {
                        "layer.fill.pattern.set"
                    }
                    LayerContent::Text(_) => "type.edit",
                    LayerContent::Shape(_) => "shape.edit",
                    _ => {
                        state.layers.settings_open = true;
                        return;
                    }
                };
                if let Some(spec) = photocraft_engine::commands::find(command) {
                    state.layers.form = Some(Form::new(spec, doc));
                }
            }
        }
        Apply => {
            if let Some(form) = &state.layers.form {
                match form.values() {
                    Ok(params) => call = Some((form.command, params)),
                    Err(error) => {
                        state.error = Some(error);
                        return;
                    }
                }
            }
        }
    }
    if let Some((command, params)) = call {
        if command == "layer.layerStyle.pasteLayerStyle" {
            doc.set_copied_style(state.layers.style_clipboard.clone());
        }
        match doc.layer_command(command, params) {
            Ok(value) => {
                if command == "layer.layerStyle.copyLayerStyle" {
                    state.layers.style_clipboard = doc.copied_style();
                }
                state.error = value["warnings"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" · ")
                    })
                    .filter(|s| !s.is_empty());
                if command != "layer.layerStyle.makeDefault" {
                    state.layers.form = None;
                }
                state.layers.libraries = Some(doc.libraries());
                if library_mutation(command)
                    && let Some(path) = library_path()
                {
                    if state.layers.libraries_readonly {
                        state.error=Some("Наборы доступны в этой сессии. Существующая библиотека повреждена и сохранена без изменений; используйте экспорт набора.".into());
                    } else if let Some(libraries) = &state.layers.libraries
                        && let Err(error) = libraries.save(&path)
                    {
                        state.error = Some(format!("Не удалось сохранить наборы слоёв: {error}"));
                    }
                }
            }
            Err(error) => state.error = Some(error),
        }
    }
}

pub(super) fn finish_preview(state: &mut State, accept: bool) {
    if let Some(mut preview) = state.layers.preview.take() {
        if !accept && let Some(session) = state.sessions.get_mut(&preview.id) {
            session.doc.cancel_move(&mut preview.original);
        }
    }
}

/// At most one native command per frame, always from the dialog's original snapshot.
pub(super) fn preview(state: &mut State, id: usize) {
    let Some(form) = state
        .layers
        .form
        .as_ref()
        .filter(|f| state.layers.preview_enabled && ux::can_preview(f.command))
    else {
        return;
    };
    let command = form.command;
    let params = match form.values() {
        Ok(params) => params,
        Err(error) => {
            state.error = Some(error);
            return;
        }
    };
    if state
        .layers
        .preview
        .as_ref()
        .is_some_and(|p| p.command != command || p.id != id)
    {
        finish_preview(state, false);
    }
    let Some(session) = state.sessions.get_mut(&id) else {
        return;
    };
    let preview = state.layers.preview.get_or_insert_with(|| FormPreview {
        id,
        command,
        original: session.doc.begin_move(),
        params: Value::Null,
    });
    if preview.params == params {
        return;
    }
    match session
        .doc
        .preview_layer_command(&mut preview.original, command, params.clone())
    {
        Ok(()) => {
            preview.params = params;
            state.error = None;
        }
        Err(error) => {
            preview.params = Value::Null;
            state.error = Some(error);
        }
    }
}

fn library_mutation(command: &str) -> bool {
    (command.starts_with("style.presets.")
        || command.starts_with("pattern.")
        || command.starts_with("gradient.presets.")
        || command.starts_with("shape.presets.")
        || command.starts_with("layer.layerStyle.contours.")
        || command == "layer.layerStyle.makeDefault"
        || command == "edit.definePattern")
        && !command.ends_with(".list")
        && !command.ends_with(".apply")
        && !command.to_ascii_lowercase().contains("export")
}
fn library_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("LOCALAPPDATA"))
            .map(|p| PathBuf::from(p).join("BevyEditor/image-layer-libraries.pcraft"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|p| {
            PathBuf::from(p)
                .join("Library/Application Support/BevyEditor/image-layer-libraries.pcraft")
        })
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
            .map(|p| p.join("bevy-editor/image-layer-libraries.pcraft"))
    }
}
