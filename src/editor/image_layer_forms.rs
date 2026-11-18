//! Structured, native controls for PhotoCraft's layer command parameters.
use super::*;

#[derive(Clone, Debug)]
pub(in crate::editor) enum FieldKind {
    Text,
    Color,
    Number,
    Choice(Vec<Value>),
    List(Value),
    Compound(Value),
    Item { array: String, index: usize },
}

#[derive(Clone, Debug)]
pub(in crate::editor) struct Field {
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
    pub text: String,
    pub enabled: bool,
}
impl Field {
    pub fn choose(&mut self, choice: usize) {
        if let FieldKind::Choice(choices) = &self.kind
            && let Some(value) = choices.get(choice)
        {
            self.text = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            self.enabled = true;
        }
    }
    fn value(&self) -> Result<Value, String> {
        match &self.kind {
            FieldKind::Number => self
                .text
                .trim()
                .replace(',', ".")
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .map(|v| {
                    if v.fract() == 0.0 && v >= 0.0 && v < (u64::MAX as f64) {
                        json!(v as u64)
                    } else if v.fract() == 0.0 && v >= (i64::MIN as f64) && v < 0.0 {
                        json!(v as i64)
                    } else {
                        json!(v)
                    }
                })
                .ok_or_else(|| format!("{}: введите число", self.label)),
            FieldKind::Choice(choices) => choices
                .iter()
                .find(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string())
                        == self.text
                })
                .cloned()
                .ok_or_else(|| format!("{}: выберите значение", self.label)),
            FieldKind::Text => Ok(Value::String(self.text.trim().to_owned())),
            FieldKind::Color => parse_color(&self.text)
                .map(|_| json!(format!("#{}", self.text.trim().trim_start_matches('#'))))
                .ok_or_else(|| {
                    format!(
                        "{}: введите цвет #RRGGBB или выберите его в палитре",
                        self.label
                    )
                }),
            FieldKind::List(_) | FieldKind::Compound(_) | FieldKind::Item { .. } => Ok(Value::Null),
        }
    }
}

pub(in crate::editor) struct Form {
    pub command: &'static str,
    pub title: String,
    pub fields: Vec<Field>,
    pub params: Value,
    pub choice: Option<usize>,
    pub effect: Option<usize>,
    structured: bool,
    initial: Value,
}

fn label(key: &str) -> String {
    match key {
        "name" => "Название",
        "color" => "Цвет",
        "from" => "Начальный цвет",
        "to" => "Конечный цвет",
        "opacity" => "Непрозрачность, %",
        "fillOpacity" => "Заливка, %",
        "blend" | "mode" => "Режим смешивания",
        "size" => "Размер, px",
        "angle" => "Угол, °",
        "distance" => "Смещение, px",
        "spread" => "Размах, %",
        "density" => "Плотность, %",
        "feather" => "Растушёвка, px",
        "enabled" => "Включено",
        "linked" => "Связано со слоем",
        "path" => "Путь к файлу",
        "scale" => "Масштаб, %",
        "add" => "Добавить ещё один эффект",
        "reverse" => "Обратить",
        "useGlobalLight" => "Глобальное освещение",
        "position" => "Положение",
        "target" => "Куда переместить",
        "width" => "Ширина, px",
        "height" => "Высота, px",
        "depth" => "Глубина",
        "soften" => "Смягчение, px",
        "altitude" => "Высота света, °",
        "noise" => "Шум, %",
        "technique" => "Метод",
        "style" => "Стиль",
        "brightness" => "Яркость",
        "contrast" => "Контрастность",
        "hue" => "Тон",
        "saturation" => "Насыщенность",
        "lightness" => "Светлота",
        "exposure" => "Экспозиция",
        "offset" => "Смещение",
        "gamma" => "Гамма",
        "vibrance" => "Сочность",
        "preserveLuminosity" => "Сохранять яркость",
        "colorize" => "Тонирование одним цветом",
        "inputBlack" => "Чёрная точка на входе",
        "inputWhite" => "Белая точка на входе",
        "outputBlack" => "Самые тёмные участки",
        "outputWhite" => "Самые светлые участки",
        "amount" => "Сила эффекта",
        "threshold" => "Порог",
        "radius" => "Радиус, пикс.",
        "channel" => "Цветовой канал",
        "clip" => "Только для нижнего слоя",
        "dither" => "Сглаживать переходы цветов",
        "align" => "Привязать к границам слоя",
        "on" => "Эффект включён",
        "knots" | "points" => "Точки кривой",
        "midpoints" => "Средние точки переходов",
        "channels" => "Цветовые каналы",
        "red" | "reds" => "Красные",
        "green" | "greens" => "Зелёные",
        "blue" | "blues" => "Синие",
        "yellow" | "yellows" => "Жёлтые",
        "cyan" | "cyans" => "Голубые",
        "magenta" | "magentas" => "Пурпурные",
        "blacks" => "Чёрные",
        "whites" => "Белые",
        "neutrals" => "Нейтральные",
        "shadows" => "Тени",
        "midtones" => "Средние тона",
        "highlights" => "Светлые участки",
        "x" => "По горизонтали, пикс.",
        "y" => "По вертикали, пикс.",
        "fontFamily" | "font" => "Шрифт",
        "fontSize" => "Размер шрифта",
        "alignment" => "Выравнивание",
        "leading" => "Межстрочное расстояние",
        "tracking" => "Межбуквенное расстояние",
        "samples" => "Качество",
        "smooth" => "Сглаживание",
        "blendIf" => "Смешивать по яркости",
        "knockout" => "Выбивание нижних слоёв",
        "contour" => "Контур",
        "antiAlias" | "antialias" => "Сглаживание",
        "knocksOut" => "Слой скрывает тень",
        "choke" => "Стягивание, %",
        "range" => "Диапазон, %",
        "source" => "Источник",
        "overprint" => "Наложение обводки",
        "method" => "Метод интерполяции",
        "smoothness" => "Плавность, %",
        "stops" => "Цветовые точки градиента",
        "opacityStops" | "transparency" => "Точки непрозрачности",
        "pattern" => "Узор",
        "texture" => "Текстура",
        "link" => "Связать со слоем",
        "phaseX" => "Смещение по X, px",
        "phaseY" => "Смещение по Y, px",
        "preset" => "Набор",
        "group" => "Группа набора",
        "includeEffects" => "Сохранить эффекты",
        "includeBlending" => "Сохранить параметры наложения",
        "append" => "Добавить к существующим",
        "text" => "Текст",
        "action" => "Действие",
        "rect" | "bounds" => "Границы",
        "invert" => "Инвертировать",
        "highlightMode" => "Режим светов",
        "highlightColor" => "Цвет светов",
        "highlightOpacity" => "Непрозрачность светов, %",
        "shadowMode" => "Режим теней",
        "shadowColor" => "Цвет теней",
        "shadowOpacity" => "Непрозрачность теней, %",
        _ => {
            return if let Ok(index) = key.parse::<usize>() {
                format!("Значение {}", index + 1)
            } else {
                "Дополнительная настройка".into()
            };
        }
    }
    .into()
}

// A delimiter inside a colour tuple, object, enum or explanatory parenthesis isn't a field.
fn split_top(text: &str) -> Vec<&str> {
    let mut quote = false;
    let mut depth = 0_i32;
    let mut start = 0;
    let mut fields = Vec::new();
    for (i, c) in text.char_indices() {
        if c == '"' {
            quote = !quote;
        }
        if quote {
            continue;
        }
        match c {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                fields.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    fields.push(&text[start..]);
    fields
}

// A numeric field may serialize 100.0 as 100; that is not an authored change.
fn same_parameter(a: &Value, b: &Value) -> bool {
    if a == b {
        return true;
    }
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_parameter(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| same_parameter(a, b)))
        }
        _ => false,
    }
}

impl Form {
    pub fn new(spec: &'static photocraft_engine::CommandSpec, doc: &PhotoDocument) -> Self {
        let mut params = json!({});
        if spec.id.starts_with("layer.newFillLayer.") || spec.id == "shape.create" {
            params["name"] = json!(title(spec.id));
        }
        let mut existing = json!({});
        if let Some(layer) = doc.active_layer() {
            let info = photocraft_engine::inspect::layer_detail(layer);
            if spec.id == "layer.setAdjustment" {
                existing = info["adjustment"]["params"].clone();
                existing["kind"] = info["adjustment"]["kind"].clone();
            }
            if spec.id == "layer.layerMask.edit"
                && let Some(mask) = &layer.mask
            {
                existing = json!({"density": mask.density * 100.0,"feather":mask.feather});
            }
            if spec.id == "layer.renameLayer" {
                existing = json!({"name": layer.name});
                // Inline editing remains attached to this row even if selection changes.
                params["layer"] = json!(layer.id.0);
            }
            // A dialog is pinned to the layer it opened for; a later selection cannot redirect it.
            if !spec.params.contains("no layer: every selected")
                && !spec.id.starts_with("layer.new")
                && doc.selected.len() <= 1
                && spec.params.contains("\"layer\":id")
            {
                params["layer"] = json!(layer.id.0);
            }
        }
        if let Some(kind) = spec.id.strip_prefix("layer.newAdjustmentLayer.")
            && let Ok(adjustment) =
                photocraft_engine::adjust_params::default_for(kind, doc.document.mode)
        {
            let mut form = Self::structured(
                spec.id,
                title(spec.id),
                photocraft_engine::adjust_params::to_dialog_params(&adjustment),
            );
            form.params["name"] = json!(title(spec.id));
            return form;
        }
        if spec.id == "layer.setAdjustment" {
            let mut form = Self::structured(spec.id, title(spec.id), existing);
            form.params["layer"] = json!(doc.active_id().map(|id| id.0));
            return form;
        }
        if spec.id == "layer.fill.solidColor.set"
            && let Some(layer) = doc.active_layer()
            && let LayerContent::Fill(photocraft_doc::Fill::Solid(color)) = &layer.content
        {
            let [r, g, b, a] = color.to_rgba8();
            existing = json!({"color":format!("#{r:02x}{g:02x}{b:02x}{a:02x}")});
        }
        if spec.id == "layer.fill.pattern.set"
            && let Some(layer) = doc.active_layer()
            && let LayerContent::Fill(photocraft_doc::Fill::Pattern {
                name,
                scale,
                angle,
                link,
                phase,
                ..
            }) = &layer.content
        {
            existing = json!({"pattern":name,"scale":scale*100.0,"angle":angle,"link":link,"phaseX":phase.0,"phaseY":phase.1});
        }
        if spec.id == "type.edit"
            && let Some(layer) = doc.active_layer()
            && let LayerContent::Text(text) = &layer.content
        {
            existing = json!({"text":text.text,"name":layer.name,"antialias":format!("{:?}",text.antialias).to_lowercase()});
        }
        if spec.id == "type.create" {
            existing = json!({"text":"Текст","x":doc.size().width as f32*0.1,"y":doc.size().height as f32*0.5});
        }
        if spec.id == "gradient.presets.apply" {
            existing = json!({"layer":true});
        }
        if spec.id == "gradient.fill.set"
            && let Some(layer) = doc.active_layer()
            && let LayerContent::Fill(fill @ photocraft_doc::Fill::Gradient { .. }) = &layer.content
        {
            let mut params = photocraft_engine::commands::gradient_fill_cmds::describe_fill(
                fill,
                doc.document.bounds(),
            );
            params["layer"] = json!(layer.id.0);
            return Self::structured(spec.id, "Градиентная заливка".into(), params);
        }
        if spec.id == "layer.vectorMask.edit"
            && let Some(layer) = doc.active_layer()
            && let Some(mask) = &layer.vector_mask
        {
            return Self::structured(
                spec.id,
                "Векторная маска".into(),
                json!({"layer":layer.id.0,"enabled":mask.enabled,"linked":mask.linked,"density":mask.density*100.0,"feather":mask.feather,"path":serde_json::to_value(&mask.path).unwrap_or(Value::Null)}),
            );
        }
        if let Some(kind) = spec.id.strip_prefix("layer.layerStyle.") {
            let defaults = doc.effect_defaults(kind);
            if defaults.as_object().is_some_and(|o| !o.is_empty()) {
                existing = defaults;
                if existing["useGlobalLight"].as_bool() == Some(true) {
                    existing["angle"] = json!(doc.document.global_light.angle);
                }
            }
        }
        let description = spec.params.trim();
        let object = description
            .find('{')
            .and_then(|a| {
                let mut depth = 0;
                let mut quote = false;
                for (b, c) in description[a..].char_indices() {
                    if c == '"' {
                        quote = !quote;
                    }
                    if quote {
                        continue;
                    }
                    if c == '{' {
                        depth += 1;
                    }
                    if c == '}' {
                        depth -= 1;
                        if depth == 0 {
                            return Some(&description[a + 1..a + b]);
                        }
                    }
                }
                None
            })
            .unwrap_or("");
        let mut fields = Vec::new();
        for field in split_top(object) {
            let Some((key, desc)) = field.trim().split_once(':') else {
                continue;
            };
            let key = key.trim().trim_matches('"');
            if (matches!(key, "layer" | "layers" | "document" | "coalesce")
                && !(key == "layer" && desc.trim().starts_with("bool")))
                || key.contains(' ')
            {
                continue;
            }
            let desc = desc.trim();
            let default = desc
                .rsplit_once('=')
                .map(|(_, v)| v.trim().trim_matches('"'));
            let enum_text = desc
                .strip_prefix('"')
                .and_then(|s| s.split_once('"').map(|(s, _)| s));
            let choices = if desc.starts_with("bool") {
                Some(vec![json!(false), json!(true)])
            } else if let Some(en) =
                enum_text.filter(|s| s.contains('|') && !s.contains('[') && !s.contains('#'))
            {
                Some(en.split('|').map(|s| json!(s)).collect())
            } else if let Some(options) = doc.library_options(key, spec.id) {
                Some(options)
            } else if key == "blend" {
                Some(
                    photocraft_color::BlendMode::layer_modes()
                        .map(|m| json!(m.label()))
                        .collect(),
                )
            } else if key == "target" && desc.starts_with("id") {
                Some(
                    doc.document
                        .walk()
                        .iter()
                        .map(|(_, _, l)| json!(l.id.0))
                        .collect(),
                )
            } else {
                None
            };
            let numeric = desc.starts_with(|c: char| c.is_ascii_digit() || c == '-')
                || [
                    "px", "deg", "f32", "i32", "uint", "int", "number", "0..", "1..",
                ]
                .iter()
                .any(|p| desc.starts_with(p));
            let compound = if desc.starts_with('[') || desc.starts_with('{') {
                Some(compound_default(key, desc, doc))
            } else {
                None
            };
            let mut kind = if let Some(value) = compound {
                FieldKind::Compound(value)
            } else if let Some(c) = choices {
                FieldKind::Choice(c)
            } else if desc.contains("#rrggbb") {
                FieldKind::Color
            } else if numeric {
                FieldKind::Number
            } else {
                FieldKind::Text
            };
            let value = existing.get(key);
            let mut text = value
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string())
                })
                .or_else(|| default.map(str::to_owned))
                .unwrap_or_else(|| match &kind {
                    FieldKind::Choice(c) => c
                        .first()
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| v.to_string())
                        })
                        .unwrap_or_default(),
                    FieldKind::Number => "0".into(),
                    _ if ["color", "from", "to"].contains(&key) => {
                        if key == "to" {
                            "#ffffff".into()
                        } else {
                            "#000000".into()
                        }
                    }
                    _ => String::new(),
                });
            if matches!(kind, FieldKind::Number) && text.parse::<f64>().is_err() {
                text = default
                    .and_then(|d| d.split_whitespace().next())
                    .filter(|d| d.parse::<f64>().is_ok())
                    .unwrap_or("0")
                    .into();
            }
            if let FieldKind::Choice(choices) = &mut kind {
                if let Some(value) = value
                    && !choices.contains(value)
                {
                    choices.push(value.clone());
                }
                if value.is_none()
                    && !choices.iter().any(|v| {
                        v.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string())
                            == text
                    })
                {
                    text = choices
                        .first()
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| v.to_string())
                        })
                        .unwrap_or_default();
                }
            }
            fields.push(Field {
                key: key.into(),
                label: label(key),
                kind,
                text,
                enabled: value.is_some_and(|v| v.as_str() != Some(""))
                    || key == "path"
                    || (!desc.contains('?') && !desc.contains('=')),
            });
        }
        if let Some(defaults) = existing.as_object() {
            for (key, value) in defaults {
                if fields.iter().any(|f| f.key == *key) || key == "kind" {
                    continue;
                }
                let kind = if value.is_boolean() {
                    FieldKind::Choice(vec![json!(false), json!(true)])
                } else if value.is_number() {
                    FieldKind::Number
                } else if value.is_object() || value.is_array() {
                    FieldKind::Compound(value.clone())
                } else {
                    FieldKind::Text
                };
                fields.push(Field {
                    key: key.clone(),
                    label: label(key),
                    kind,
                    text: value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string()),
                    enabled: !value.is_array() && !value.is_object(),
                });
            }
        }
        let mut form = Self {
            command: spec.id,
            title: title(spec.id),
            fields,
            initial: params.clone(),
            params,
            choice: None,
            effect: None,
            structured: false,
        };
        if spec.id == "layer.layerStyle.stroke" {
            form.fields.retain(|f| f.key != "strokePaint");
            form.fields.insert(
                0,
                stroke_choice(
                    "strokePaint",
                    if existing.get("from").is_some() || existing.get("stops").is_some() {
                        "gradient"
                    } else {
                        "solid"
                    },
                ),
            );
            if let Some(style) = form.fields.iter_mut().find(|f| f.key == "style") {
                *style = stroke_choice("style", existing["style"].as_str().unwrap_or("linear"));
            }
            if existing.get("angle").is_none()
                && let Some(angle) = form.fields.iter_mut().find(|f| f.key == "angle")
            {
                angle.text = "90".into();
            }
        }
        form
    }
    pub fn structured(command: &'static str, title: String, params: Value) -> Self {
        let mut form = Self {
            command,
            title,
            fields: Vec::new(),
            initial: params.clone(),
            params,
            choice: None,
            effect: None,
            structured: true,
        };
        form.rebuild();
        form
    }
    pub fn complex(&mut self, index: usize) -> Result<(), String> {
        let Some(field) = self.fields.get(index).cloned() else {
            return Ok(());
        };
        let FieldKind::Compound(template) = field.kind else {
            return Ok(());
        };
        self.params = self.raw_values()?;
        self.params[&field.key] = template;
        self.structured = true;
        self.rebuild();
        Ok(())
    }
    fn rebuild(&mut self) {
        fn visit(value: &Value, path: &str, fields: &mut Vec<Field>) {
            let key = path.rsplit('/').next().unwrap_or_default();
            if matches!(
                key,
                "layer" | "document" | "kind" | "unsupported" | "psdKey" | "fx"
            ) {
                return;
            }
            match value {
                Value::Object(object) => {
                    for (key, value) in object {
                        visit(value, &format!("{path}/{key}"), fields);
                    }
                }
                Value::Array(array) => {
                    let list = array.is_empty()
                        || array.iter().any(|v| v.is_object() || v.is_array())
                        || array.len() > 6;
                    if list {
                        fields.push(Field {
                            key: path.into(),
                            label: format!("{} · {}", label(key), array.len()),
                            kind: FieldKind::List(array.last().cloned().unwrap_or_else(
                                || match key {
                                    "opacityStops" | "transparency" => json!([0, 100]),
                                    "midpoints" => json!(0.5),
                                    "knots" => json!([0, 0]),
                                    _ => json!(0),
                                },
                            )),
                            text: String::new(),
                            enabled: true,
                        });
                    }
                    for (index, value) in array.iter().enumerate() {
                        if list {
                            fields.push(Field {
                                key: format!("{path}/{index}"),
                                label: format!("{} {}", label(key), index + 1),
                                kind: FieldKind::Item {
                                    array: path.into(),
                                    index,
                                },
                                text: String::new(),
                                enabled: true,
                            });
                        }
                        visit(value, &format!("{path}/{index}"), fields);
                    }
                }
                Value::Null => {}
                _ => {
                    let kind = if value.is_boolean() {
                        FieldKind::Choice(vec![json!(false), json!(true)])
                    } else if value.is_number() {
                        FieldKind::Number
                    } else if value.as_str().is_some_and(|s| s.starts_with('#')) {
                        FieldKind::Color
                    } else {
                        FieldKind::Text
                    };
                    let label = path
                        .split('/')
                        .filter(|p| !p.is_empty() && !matches!(*p, "params" | "fx" | "effects"))
                        .map(label)
                        .collect::<Vec<_>>()
                        .join(" · ");
                    fields.push(Field {
                        key: path.into(),
                        label,
                        kind,
                        text: value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| value.to_string()),
                        enabled: true,
                    });
                }
            }
        }
        self.fields.clear();
        visit(&self.params, "", &mut self.fields);
        // Place the paint switch before the settings it controls.
        let mut paints = Vec::new();
        self.fields.retain(|field| {
            if field.key.ends_with("/strokePaint") && is_stroke_param(&self.params, &field.key) {
                paints.push(field.clone());
                false
            } else {
                true
            }
        });
        for paint in paints {
            let prefix = paint.key.strip_suffix("strokePaint").unwrap();
            if let Some(index) = self
                .fields
                .iter()
                .position(|field| field.key.starts_with(prefix))
            {
                self.fields.insert(index, paint);
            }
        }
        for field in &mut self.fields {
            if is_stroke_param(&self.params, &field.key) {
                let key = field.key.rsplit('/').next().unwrap_or_default();
                if matches!(key, "strokePaint" | "style") {
                    let mut choice = stroke_choice(key, &field.text);
                    choice.key = field.key.clone();
                    *field = choice;
                }
            }
            if let Some(rest) = field.key.split("/stops/").nth(1) {
                let mut parts = rest.split('/');
                if let Some(index) = parts.next().and_then(|s| s.parse::<usize>().ok()) {
                    field.label = match parts.next() {
                        Some("0") => format!("Положение цвета {}", index + 1),
                        Some("1") => format!("Цвет {}", index + 1),
                        _ => format!("Цветовая точка {}", index + 1),
                    };
                }
            }
            if let Some(index) = field
                .key
                .strip_prefix("/effects/")
                .and_then(|p| p.split('/').next())
                .and_then(|p| p.parse::<usize>().ok())
            {
                let kind = self.params["effects"][index]["kind"].as_str().unwrap_or("");
                field.label = format!(
                    "{} · {}",
                    title(&format!("layer.layerStyle.{kind}")),
                    label(field.key.rsplit('/').next().unwrap_or_default())
                );
            }
            if matches!(field.kind, FieldKind::Text) {
                let key = field.key.rsplit('/').next().unwrap_or_default();
                let values: &[&str] = match key {
                    "mode" | "blend" | "highlightMode" | "shadowMode" => &[],
                    "style" if self.command.contains("gradient") => {
                        &["linear", "radial", "angle", "reflected", "diamond"]
                    }
                    "position" => &["inside", "center", "outside"],
                    "technique" => &["softer", "precise"],
                    "source" => &["edge", "center"],
                    "method" => &["classic", "linear", "perceptual"],
                    _ => continue,
                };
                let mut choices = if values.is_empty() {
                    photocraft_color::BlendMode::layer_modes()
                        .map(|m| json!(m.label()))
                        .collect::<Vec<_>>()
                } else {
                    values.iter().map(|v| json!(v)).collect()
                };
                if !choices.contains(&json!(field.text)) {
                    choices.push(json!(field.text));
                }
                field.kind = FieldKind::Choice(choices);
            }
        }
    }
    pub fn array(&mut self, index: usize, add: bool) -> Result<(), String> {
        let Some(field) = self.fields.get(index).cloned() else {
            return Ok(());
        };
        let FieldKind::List(item) = field.kind else {
            return Ok(());
        };
        self.params = self.raw_values()?;
        if let Some(array) = self
            .params
            .pointer_mut(&field.key)
            .and_then(Value::as_array_mut)
        {
            if add && array.len() < 256 {
                array.push(item);
            } else if !add {
                array.pop();
            }
        }
        self.rebuild();
        Ok(())
    }
    pub fn list_edit(&mut self, index: usize, action: i8) -> Result<(), String> {
        let Some(field) = self.fields.get(index) else {
            return Ok(());
        };
        let FieldKind::Item { array, index } = &field.kind else {
            return Ok(());
        };
        let (array, index) = (array.clone(), *index);
        self.params = self.raw_values()?;
        if let Some(values) = self
            .params
            .pointer_mut(&array)
            .and_then(Value::as_array_mut)
            && index < values.len()
        {
            match action {
                0 => {
                    values.remove(index);
                }
                -1 if index > 0 => values.swap(index, index - 1),
                1 if index + 1 < values.len() => values.swap(index, index + 1),
                _ => {}
            }
        }
        self.rebuild();
        Ok(())
    }
    fn raw_values(&self) -> Result<Value, String> {
        let mut value = self.params.clone();
        for field in self.fields.iter().filter(|f| f.enabled) {
            if matches!(
                field.kind,
                FieldKind::List(_) | FieldKind::Compound(_) | FieldKind::Item { .. }
            ) {
                continue;
            }
            if self.structured {
                if let Some(target) = value.pointer_mut(&field.key) {
                    *target = field.value()?;
                }
            } else {
                value[&field.key] = field.value()?;
            }
        }
        Ok(value)
    }
    pub fn values(&self) -> Result<Value, String> {
        let mut value = self.raw_values()?;
        if self.command == "layer.layerStyle.stroke" {
            let gradient = self
                .fields
                .iter()
                .any(|f| f.key == "strokePaint" && f.text == "gradient");
            let params = value.as_object_mut().unwrap();
            params.remove("strokePaint");
            if gradient {
                params.remove("color");
                for key in ["from", "to", "angle", "style"] {
                    if let Some(field) = self.fields.iter().find(|f| f.key == key) {
                        params.insert(key.into(), field.value()?);
                    }
                }
            } else {
                for key in ["from", "to", "style", "angle", "stops", "opacityStops"] {
                    params.remove(key);
                }
            }
        }
        if self.command == "layer.layerStyle.replace"
            && let Some(entries) = value["effects"].as_array_mut()
        {
            for entry in entries {
                let original = self.initial["effects"]
                    .as_array()
                    .and_then(|entries| entries.iter().find(|e| e["fx"] == entry["fx"]));
                if let Some(params) = entry.get_mut("params").and_then(Value::as_object_mut) {
                    params.retain(|key, value| {
                        !original
                            .and_then(|e| e["params"].get(key))
                            .is_some_and(|old| same_parameter(old, value))
                    });
                }
                if entry["kind"] == "stroke" {
                    update_stroke_snapshot(entry)?;
                }
                if let Some(on) = entry.get("on").and_then(Value::as_bool) {
                    let mut effect: photocraft_doc::Effect =
                        serde_json::from_value(entry["fx"].clone()).map_err(|e| e.to_string())?;
                    effect.set_enabled(on);
                    entry["fx"] = serde_json::to_value(effect).map_err(|e| e.to_string())?;
                    entry.as_object_mut().unwrap().remove("on");
                }
            }
        }
        Ok(value)
    }

    pub(super) fn is_stroke_field(&self, key: &str) -> bool {
        self.command == "layer.layerStyle.stroke" || is_stroke_param(&self.params, key)
    }

    pub(super) fn stroke_field_visible(&self, key: &str) -> bool {
        if !self.is_stroke_field(key) {
            return true;
        }
        let (prefix, name) = key.rsplit_once('/').unwrap_or(("", key));
        let gradient = self.fields.iter().any(|f| {
            let (field_prefix, name) = f.key.rsplit_once('/').unwrap_or(("", &f.key));
            prefix == field_prefix && name == "strokePaint" && f.text == "gradient"
        });
        match name {
            "color" => !gradient,
            "from" | "to" | "angle" | "style" | "method" | "smoothness" => gradient,
            _ => true,
        }
    }
}

fn is_stroke_param(params: &Value, key: &str) -> bool {
    key.strip_prefix("/effects/")
        .and_then(|key| key.split_once("/params/"))
        .and_then(|(index, _)| index.parse::<usize>().ok())
        .is_some_and(|index| params["effects"][index]["kind"] == "stroke")
}

fn stroke_choice(key: &str, value: &str) -> Field {
    let (label, choices): (_, &[&str]) = if key == "strokePaint" {
        ("Заливка обводки", &["solid", "gradient"])
    } else {
        (
            "Форма градиента",
            &["linear", "radial", "angle", "reflected", "diamond"],
        )
    };
    Field {
        key: key.into(),
        label: label.into(),
        kind: FieldKind::Choice(choices.iter().map(|v| json!(v)).collect()),
        text: value.into(),
        enabled: true,
    }
}

fn update_stroke_snapshot(entry: &mut Value) -> Result<(), String> {
    use photocraft_doc::{Effect, FxPaint};
    let params = &mut entry["params"];
    let Some(changes) = params.as_object_mut() else {
        return Ok(());
    };
    let mode = changes.remove("strokePaint");
    let paint: serde_json::Map<String, Value> = ["color", "from", "to", "style", "angle"]
        .into_iter()
        .filter_map(|key| changes.remove(key).map(|v| (key.into(), v)))
        .collect();
    if mode.is_none() && paint.is_empty() {
        return Ok(());
    }
    let mut effect: Effect =
        serde_json::from_value(entry["fx"].clone()).map_err(|e| e.to_string())?;
    let Effect::Stroke(stroke) = &mut effect else {
        return Ok(());
    };
    let gradient = mode
        .as_ref()
        .map_or(matches!(stroke.paint, FxPaint::Gradient(_)), |mode| {
            mode == "gradient"
        });
    let color = |key: &str| {
        paint
            .get(key)?
            .as_str()
            .and_then(parse_color)
            .map(|[r, g, b, a]| photocraft_doc::Color::rgba(r, g, b, a))
    };
    if gradient {
        if !matches!(stroke.paint, FxPaint::Gradient(_)) {
            stroke.paint = FxPaint::Gradient(photocraft_doc::effects::Gradient::default());
        }
        if let FxPaint::Gradient(gradient) = &mut stroke.paint {
            if let Some((_, target)) = gradient.stops.first_mut()
                && let Some(color) = color("from")
            {
                *target = color;
            }
            if let Some((_, target)) = gradient.stops.last_mut()
                && let Some(color) = color("to")
            {
                *target = color;
            }
            if let Some(angle) = paint.get("angle").and_then(Value::as_f64) {
                gradient.angle = angle as f32;
            }
            if let Some(style) = paint.get("style").and_then(Value::as_str) {
                gradient.style = photocraft_engine::layer_style::gradient_style(style);
            }
        }
    } else if mode.is_some() || color("color").is_some() {
        stroke.paint = FxPaint::Color(color("color").unwrap_or(photocraft_doc::Color::BLACK));
    }
    entry["fx"] = serde_json::to_value(effect).map_err(|e| e.to_string())?;
    Ok(())
}

fn compound_default(key: &str, desc: &str, doc: &PhotoDocument) -> Value {
    let (w, h) = (doc.size().width, doc.size().height);
    match key {
        "rect" | "bounds" => json!([0, 0, w, h]),
        "matrix" => json!([1, 0, 0, 1, 0, 0]),
        "channels" => json!([true, true, true]),
        "blendIf" => json!({"channel":"gray","thisLayer":[0,255],"underlying":[0,255]}),
        "path" => {
            json!({"subpaths":[{"closed":true,"op":"combine","knots":[[0,0],[w,0],[w,h],[0,h]]}],"fillRule":"nonzero","inverted":false})
        }
        "stops" => json!([[0, "#000000"], [1, "#ffffff"]]),
        "opacityStops" | "transparency" => json!([[0, 100], [1, 100]]),
        "points" => json!([[0, 0], [255, 255]]),
        "locks" => {
            json!({"all":false,"pixels":false,"transparency":false,"position":false,"artboard":false})
        }
        _ if desc.starts_with('{') => {
            let end = desc.rfind('}').unwrap_or(desc.len());
            let mut object = serde_json::Map::new();
            for field in split_top(&desc[1..end]) {
                if let Some((key, desc)) = field.split_once(':') {
                    let key = key.trim().trim_matches('"');
                    object.insert(
                        key.into(),
                        if desc.trim().starts_with("bool") {
                            json!(false)
                        } else if desc.trim().starts_with('"') {
                            json!(
                                desc.trim()
                                    .trim_matches('"')
                                    .split('|')
                                    .next()
                                    .unwrap_or_default()
                            )
                        } else {
                            json!(0)
                        },
                    );
                }
            }
            Value::Object(object)
        }
        _ => {
            let end = desc.find(']').unwrap_or(desc.len());
            let count = split_top(&desc[1..end]).len().clamp(2, 16);
            Value::Array((0..count).map(|_| json!(0)).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stroke_paint_switch_hides_inactive_settings_and_keeps_existing_gradient_stops() {
        use photocraft_doc::{Effect, FxPaint, effects::GradientStyle};
        let effect = photocraft_engine::layer_style::effect_from_params(
            "stroke",
            &json!({"stops":[[0,"#ff0000"],[0.4,"#00ff00"],[1,"#0000ff"]],"style":"radial"}),
        )
        .unwrap();
        let snapshot = serde_json::to_value(&effect).unwrap();
        let mut form = Form::structured(
            "layer.layerStyle.replace",
            "Обводка".into(),
            json!({"effects":[{"kind":"stroke","params":effects::values_of(&effect, 120.0),"fx":snapshot}]}),
        );
        let key = |field| format!("/effects/0/params/{field}");
        assert!(!form.stroke_field_visible(&key("color")));
        assert!(form.stroke_field_visible(&key("style")));
        assert_eq!(form.values().unwrap()["effects"][0]["fx"], snapshot);
        form.fields
            .iter_mut()
            .find(|f| f.key == key("style"))
            .unwrap()
            .choose(4);
        let changed = form.values().unwrap();
        let Effect::Stroke(stroke) =
            serde_json::from_value(changed["effects"][0]["fx"].clone()).unwrap()
        else {
            panic!("expected stroke")
        };
        let FxPaint::Gradient(gradient) = stroke.paint else {
            panic!("expected gradient")
        };
        assert_eq!(gradient.style, GradientStyle::Diamond);
        assert_eq!(gradient.stops.len(), 3);
        assert_eq!(gradient.stops[1].1.to_rgba8(), [0, 255, 0, 255]);
        form.fields
            .iter_mut()
            .find(|f| f.key == key("strokePaint"))
            .unwrap()
            .choose(0);
        assert!(form.stroke_field_visible(&key("color")));
        assert!(!form.stroke_field_visible(&key("from")));
        let changed = form.values().unwrap();
        let Effect::Stroke(stroke) =
            serde_json::from_value(changed["effects"][0]["fx"].clone()).unwrap()
        else {
            panic!("expected stroke")
        };
        assert!(matches!(stroke.paint, FxPaint::Color(_)));
    }
    #[test]
    fn command_dialog_keeps_multi_selection_and_preselects_required_presets() {
        let path = std::env::temp_dir().join(format!(
            "bevy-layer-form-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        image::RgbaImage::from_pixel(4, 4, image::Rgba([30, 60, 90, 255]))
            .save(&path)
            .unwrap();
        let mut doc = PhotoDocument::load(&path).unwrap();
        doc.add_layer();
        doc.select(0, false, false);
        doc.select(1, true, false);
        let form = Form::new(
            photocraft_engine::commands::find("layer.duplicate").unwrap(),
            &doc,
        );
        assert!(form.values().unwrap().get("layer").is_none());
        doc.layer_command(form.command, form.values().unwrap())
            .unwrap();
        assert_eq!(doc.document.layer_count(), 4);
        let preset = Form::new(
            photocraft_engine::commands::find("style.presets.apply").unwrap(),
            &doc,
        );
        assert!(
            preset.values().unwrap()["preset"].is_string(),
            "A required preset is usable without enabling an optional-field toggle"
        );
        doc.layer_command(preset.command, preset.values().unwrap())
            .unwrap();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("pcraft"));
    }
    #[test]
    fn numeric_fields_keep_integer_command_parameters_and_reject_nonfinite_values() {
        let mut field = Field {
            key: "index".into(),
            label: "Индекс".into(),
            kind: FieldKind::Number,
            text: "2".into(),
            enabled: true,
        };
        assert_eq!(field.value().unwrap().as_u64(), Some(2));
        field.text = "0,75".into();
        assert_eq!(field.value().unwrap().as_f64(), Some(0.75));
        for invalid in ["NaN", "inf", ""] {
            field.text = invalid.into();
            assert!(field.value().is_err());
        }
    }
    #[test]
    fn style_dialog_preserves_imported_precision_and_only_sends_changed_properties() {
        let mut effect = photocraft_doc::Effect::default_drop_shadow();
        if let photocraft_doc::Effect::DropShadow(shadow) = &mut effect {
            shadow.common.opacity = 0.7537;
        }
        let snapshot = serde_json::to_value(&effect).unwrap();
        let mut form = Form::structured(
            "layer.layerStyle.replace",
            "Эффекты".into(),
            json!({"layer":1,"effects":[{"kind":"dropShadow","on":true,"params":effects::values_of(&effect,120.0),"fx":snapshot}]}),
        );
        let unchanged = form.values().unwrap();
        assert_eq!(unchanged["effects"][0]["params"], json!({}));
        assert_eq!(unchanged["effects"][0]["fx"], snapshot);
        assert!(!form.fields.iter().any(|f| f.key.contains("/fx/")));
        form.fields
            .iter_mut()
            .find(|f| f.key == "/effects/0/params/size")
            .unwrap()
            .text = "17".into();
        let changed = form.values().unwrap();
        assert_eq!(changed["effects"][0]["params"], json!({"size":17}));
        assert_eq!(changed["effects"][0]["fx"], snapshot);
    }
    #[test]
    fn effect_reordering_keeps_each_instances_original_snapshot() {
        let a = photocraft_doc::Effect::default_drop_shadow();
        let b =
            photocraft_engine::layer_style::effect_from_params("colorOverlay", &json!({})).unwrap();
        let entries=[a,b].into_iter().map(|fx|json!({"kind":photocraft_engine::layer_style::kind_of(&fx),"on":true,"params":effects::values_of(&fx,120.0),"fx":serde_json::to_value(fx).unwrap()})).collect::<Vec<_>>();
        let mut form = Form::structured(
            "layer.layerStyle.replace",
            "Эффекты".into(),
            json!({"layer":1,"effects":entries}),
        );
        let item = form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::Item { index: 1, .. }))
            .unwrap();
        form.list_edit(item, -1).unwrap();
        let result = form.values().unwrap();
        assert_eq!(result["effects"][0]["kind"], "colorOverlay");
        assert_eq!(result["effects"][0]["params"], json!({}));
        assert_eq!(result["effects"][1]["params"], json!({}));
    }
}
