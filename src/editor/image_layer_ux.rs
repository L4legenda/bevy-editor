//! Plain-language layer labels, useful defaults and progressive property disclosure.
use super::*;
pub(super) use crate::editor::ui::control_hints::Help;
use photocraft_color::BlendMode;

pub(in crate::editor) fn help(
    commands: &mut Commands,
    entity: Entity,
    explanation: impl Into<String>,
) {
    commands
        .entity(entity)
        .insert((Help(explanation.into()), Hovered::default()));
}

pub(in crate::editor) fn action_help(action: LayerAction) -> String {
    match action {
        LayerAction::PanelSection(PanelSection::Layers) => "Список слоёв. Верхние слои перекрывают нижние. Перетащите строку, чтобы изменить порядок.".into(),
        LayerAction::PanelSection(PanelSection::Settings) => "Свойства выбранного слоя: название, заливка, маски, эффекты и защита от изменений.".into(),
        LayerAction::Properties => "Открыть настройки содержимого выбранного слоя: цвет, градиент, текст или коррекцию. У обычного слоя откроются его свойства.".into(),
        LayerAction::Group => "Собрать выделенные слои в папку. Если выбран один слой, создаётся пустая группа. Слои можно перетаскивать внутрь неё.".into(),
        LayerAction::Lock("all") => "Защитить слой от любых изменений. Нажмите ещё раз, чтобы продолжить редактирование.".into(),
        LayerAction::ToggleLock(_) => "Защитить этот слой от любых изменений. Нажмите ещё раз, чтобы снять блокировку.".into(),
        LayerAction::Lock("transparency") => "Рисовать только по уже закрашенным пикселям, не выходя за их края. Удобно для перекрашивания рисунка.".into(),
        LayerAction::Lock("pixels") => "Защитить рисунок от кисти и ластика. Перемещение и настройки слоя остаются доступны.".into(),
        LayerAction::Lock(_) => "Защитить положение слоя, чтобы случайно не сдвинуть рисунок.".into(),
        LayerAction::FieldNumber(..)|LayerAction::FieldStep(..) => "Двигайте ползунок для быстрого изменения. Кнопки − и + позволяют менять значение небольшими шагами.".into(),
        LayerAction::Preview => "Показывать изменения на холсте до применения. Выключите для сравнения с исходным состоянием. «Отмена» полностью вернёт исходный результат.".into(),
        LayerAction::Advanced => "Редкие параметры для точной настройки. Основных настроек обычно достаточно; скрытые параметры сохраняют своё значение.".into(),
        LayerAction::Apply => "Сохранить результат настройки. Вся операция отменяется одним нажатием Ctrl+Z.".into(),
        LayerAction::Cancel => "Закрыть окно. Неприменённый предпросмотр будет отменён.".into(),
        LayerAction::Quick(command)|LayerAction::Command(command) => match command {
            "layer.duplicate" => "Создать точную копию выбранных слоёв над оригиналом. Оригинал остаётся на месте. Ctrl+J.".into(),
            "layer.delete" => "Удалить выбранные слои. Если удалили случайно, нажмите Ctrl+Z.".into(),
            "layer.layerMask.revealAll" => "Добавить маску, чтобы скрывать части рисунка без удаления. Выберите её белый образец: чёрная кисть скрывает, белая возвращает изображение.".into(),
            "layer.arrange.bringForward" => "Поднять слой на одну позицию: его рисунок будет перекрывать больше нижних слоёв.".into(),
            "layer.arrange.sendBackward" => "Опустить слой на одну позицию: верхние слои смогут перекрывать его рисунок.".into(),
            "layer.mergeDown" => "Объединить выбранный слой с ближайшим нижним. После этого они редактируются как одно изображение. Ctrl+Z вернёт отдельные слои.".into(),
            "layer.layerMask.apply" => "Превратить действие маски в прозрачность пикселей и убрать маску. Для временного сравнения лучше выключить маску.".into(),
            "layer.layerMask.delete" => "Убрать маску и вернуть скрытые ею части рисунка. Сам рисунок не удаляется.".into(),
            _ => format!("{} · открыть настройки или выполнить действие над выбранным слоем. Изменения можно отменить через Ctrl+Z.",title(command)),
        },
        LayerAction::SetBlend(mode) => blend_help(mode).into(),
        LayerAction::Blend => "Как слой смешивается с нижними. «Обычный» подходит для большинства задач. В меню есть пояснения к каждому режиму.".into(),
        LayerAction::Category("Эффекты") => "Добавить тень, обводку или свечение. Эффекты не меняют сам рисунок, их можно перенастроить или выключить.".into(),
        LayerAction::Category("Коррекция") => "Изменить свет и цвет отдельным слоем. Исходный рисунок сохранится; действие коррекции можно ослабить непрозрачностью.".into(),
        LayerAction::Menu => "Частые действия с выбранным слоем. Полный список редких операций доступен в дополнительных инструментах.".into(),
        _ => "Настройки слоёв. Верхние слои перекрывают нижние; глаз скрывает слой без удаления, Ctrl+Z отменяет изменение.".into(),
    }
}

pub(super) fn blend(mode: BlendMode) -> &'static str {
    use BlendMode::*;
    match mode {
        PassThrough => "Пропускать через группу",
        Normal => "Обычный",
        Dissolve => "Растворение",
        Darken => "Затемнение",
        Multiply => "Умножение",
        ColorBurn => "Затемнение основы",
        LinearBurn => "Линейный затемнитель",
        DarkerColor => "Темнее",
        Lighten => "Осветление",
        Screen => "Экран",
        ColorDodge => "Осветление основы",
        LinearDodge => "Линейный осветлитель",
        LighterColor => "Светлее",
        Overlay => "Перекрытие",
        SoftLight => "Мягкий свет",
        HardLight => "Жёсткий свет",
        VividLight => "Яркий свет",
        LinearLight => "Линейный свет",
        PinLight => "Точечный свет",
        HardMix => "Жёсткое смешивание",
        Difference => "Разница",
        Exclusion => "Исключение",
        Subtract => "Вычитание",
        Divide => "Деление",
        Hue => "Цветовой тон",
        Saturation => "Насыщенность",
        Color => "Цветность",
        Luminosity => "Яркость",
        Reflect => "Отражение",
        Glow => "Свечение",
        Negation => "Отрицание",
        Xor => "Исключающее смешивание",
        PaintNetColorBurn => "Затемнение основы · дополнительное",
        PaintNetColorDodge => "Осветление основы · дополнительное",
    }
}
pub(super) fn blend_help(mode: BlendMode) -> &'static str {
    use BlendMode::*;
    match mode {
        Normal => {
            "Обычный слой: его цвет перекрывает нижние слои. Для большей прозрачности уменьшите непрозрачность."
        }
        PassThrough => {
            "Слои внутри группы смешиваются с изображением за пределами группы. Выберите «Обычный», чтобы смешивать группу отдельно."
        }
        Multiply | Darken | ColorBurn | LinearBurn | DarkerColor => {
            "Затемняет нижнее изображение. «Умножение» удобно для теней и рисунка поверх бумаги: белые участки почти не влияют на результат."
        }
        Screen | Lighten | ColorDodge | LinearDodge | LighterColor => {
            "Осветляет нижнее изображение. «Экран» удобен для бликов и свечения: чёрные участки почти не влияют на результат."
        }
        Overlay | SoftLight | HardLight | VividLight | LinearLight | PinLight => {
            "Меняет свет и контраст нижних слоёв. Начните с «Мягкого света» и небольшой непрозрачности для деликатного эффекта."
        }
        Hue | Saturation | Color | Luminosity => {
            "Заменяет только выбранную составляющую цвета нижних слоёв. «Цветность» позволяет перекрасить рисунок, сохранив его свет и тени."
        }
        _ => {
            "Специальное смешивание цветов с нижними слоями. Если результат непривычный, вернитесь к «Обычному». Изменение можно отменить через Ctrl+Z."
        }
    }
}

pub(super) fn choice(value: &str) -> String {
    if let Some(mode) = BlendMode::layer_modes()
        .chain([BlendMode::PassThrough])
        .find(|m| m.label() == value)
    {
        return blend(mode).into();
    }
    match value {
        "true" => "Да",
        "false" => "Нет",
        "none" => "Нет",
        "normal" => "Обычный",
        "linear" => "Линейный",
        "radial" => "Круговой",
        "angle" => "Угловой",
        "reflected" => "Зеркальный",
        "diamond" => "Ромб",
        "inside" => "Внутри",
        "outside" => "Снаружи",
        "center" => "По центру",
        "edge" => "От края",
        "smooth" => "Плавный",
        "precise" => "Точный",
        "softer" => "Мягкий",
        "source" => "Источник",
        "innerBevel" => "Внутренний скос",
        "outerBevel" => "Внешний скос",
        "emboss" => "Тиснение",
        "pillowEmboss" => "Контурное тиснение",
        "strokeEmboss" => "Тиснение обводки",
        "up" => "Вверх",
        "down" => "Вниз",
        "above" => "Выше",
        "below" => "Ниже",
        "into" => "Внутрь группы",
        "nearest" => "Чёткие пиксели",
        "bilinear" => "Плавное",
        "bicubic" => "Плавное, высокое качество",
        "solid" => "Сплошной цвет",
        "gradient" => "Градиент",
        "pattern" => "Узор",
        "rgb" => "Все цвета",
        "red" => "Красный",
        "green" => "Зелёный",
        "blue" => "Синий",
        "cyan" => "Голубой",
        "magenta" => "Пурпурный",
        "yellow" => "Жёлтый",
        "black" => "Чёрный",
        "orange" => "Оранжевый",
        "seafoam" => "Мятный",
        "indigo" => "Индиго",
        "violet" => "Фиолетовый",
        "fuchsia" => "Розовый",
        "gray" | "grey" => "Серый",
        "master" => "Все цвета",
        "reds" => "Красные",
        "yellows" => "Жёлтые",
        "greens" => "Зелёные",
        "cyans" => "Голубые",
        "blues" => "Синие",
        "magentas" => "Пурпурные",
        "shadows" => "Тени",
        "midtones" => "Средние тона",
        "highlights" => "Светлые участки",
        "relative" => "Относительно",
        "absolute" => "Абсолютно",
        "perceptual" => "На глаз",
        "classic" => "Классический",
        "sharp" => "Чёткий",
        "crisp" => "Резкий",
        "strong" => "Выразительный",
        "left" => "Слева",
        "right" => "Справа",
        "top" => "Сверху",
        "bottom" => "Снизу",
        "horizontal" => "Горизонтально",
        "vertical" => "Вертикально",
        "auto" => "Автоматически",
        "reset" => "Сбросить",
        "replace" => "Заменить",
        "append" => "Добавить",
        "delete" => "Удалить",
        "rename" => "Переименовать",
        _ => return value.into(), // User-created preset/layer names are preserved.
    }
    .into()
}

pub(super) fn field_help(key: &str) -> &'static str {
    let key = key.rsplit('/').next().unwrap_or(key);
    match key {
        "name" => {
            "Название помогает найти слой. Например, «Контур», «Тени» или «Фон». На рисунок оно не влияет."
        }
        "opacity" | "highlightOpacity" | "shadowOpacity" => {
            "Насколько заметен результат: 100% — полностью, 0% — невидим. Двигайте ползунок; кнопки − и + меняют значение небольшими шагами."
        }
        "fillOpacity" => {
            "Прозрачность самого рисунка без изменения его тени, обводки и других эффектов. Для прозрачности всего слоя используйте «Непрозрачность»."
        }
        "size" | "radius" | "blur" => {
            "Размер эффекта в пикселях. У тени и свечения увеличивает размытие, у обводки — толщину. Небольшие значения дают чёткий край."
        }
        "distance" => {
            "Как далеко тень отстоит от рисунка. При 0 тень остаётся прямо под ним; большее значение создаёт ощущение высоты."
        }
        "angle" => {
            "Направление света или градиента в градусах. Поверните ползунок, чтобы изменить направление эффекта."
        }
        "spread" | "choke" => {
            "Плотность края перед размытием. Увеличьте, чтобы тень или свечение стали гуще и отчётливее."
        }
        "color" | "from" | "to" | "highlightColor" | "shadowColor" => {
            "Нажмите образец, чтобы выбрать цвет в палитре. Писать код цвета не требуется. Подтвердите выбор кнопкой ОК."
        }
        "blend" | "mode" | "highlightMode" | "shadowMode" => {
            "Как цвет соединяется с изображением под ним. «Обычный» перекрывает, «Умножение» затемняет, «Экран» осветляет."
        }
        "feather" => {
            "Смягчает край маски. При 0 край чёткий; при большем значении скрытые участки плавно переходят в видимые."
        }
        "density" => {
            "Сила маски. При 100% чёрные участки полностью скрывают слой. Уменьшите значение, чтобы вернуть часть скрытого рисунка."
        }
        "linked" | "link" => {
            "Если включено, маска или заливка перемещается вместе со слоем. Выключите, чтобы менять её положение отдельно."
        }
        "enabled" | "on" => {
            "Временно включает или выключает результат, не удаляя настройки. Удобно сравнивать до и после."
        }
        "useGlobalLight" => {
            "Использовать одно направление света для теней и тиснения во всём изображении. Выключите для отдельного направления этого эффекта."
        }
        "brightness" => "Осветляет или затемняет изображение. Ноль сохраняет исходную яркость.",
        "contrast" => {
            "Увеличивает разницу между светлыми и тёмными участками. Отрицательное значение делает изображение мягче."
        }
        "hue" => {
            "Сдвигает цвета по цветовому кругу. Например, красный можно превратить в зелёный, сохранив свет и тени."
        }
        "saturation" | "vibrance" => {
            "Интенсивность цвета. Уменьшите для приглушённых цветов, увеличьте для более ярких."
        }
        "lightness" => "Сдвигает цвет к белому или чёрному. Ноль сохраняет исходную светлоту.",
        "exposure" => {
            "Меняет освещённость изображения. Положительные значения осветляют, отрицательные затемняют."
        }
        "gamma" => {
            "Меняет преимущественно средние тона, сохраняя крайние светлые и тёмные участки. 1 — без изменения."
        }
        "scale" => "Размер градиента или узора. 100% — исходный масштаб; 200% — вдвое крупнее.",
        "reverse" | "invert" => {
            "Меняет местами светлые и тёмные части маски или направление цветов градиента."
        }
        "noise" => {
            "Добавляет зерно к эффекту. 0% даёт гладкий результат, небольшое значение помогает имитировать бумагу или плёнку."
        }
        "position" => {
            "Где проходит обводка относительно края рисунка: внутри, снаружи или по центру."
        }
        "preserveLuminosity" => {
            "Сохранять яркость при изменении цвета, чтобы рисунок не становился неожиданно светлее или темнее."
        }
        _ => {
            "Настройка выбранного слоя или эффекта. Измените её ползунком или выберите готовый вариант. «Отмена» вернёт состояние до открытия окна."
        }
    }
}

pub(super) fn primary(command: &str, key: &str) -> bool {
    let last = key.rsplit('/').next().unwrap_or(key);
    if key.contains("/stops/") {
        return last == "1";
    }
    if key.starts_with("/effects/") {
        return matches!(
            last,
            "on" | "color" | "opacity" | "size" | "distance" | "angle"
        );
    }
    if command.ends_with("blendingOptions") {
        return matches!(last, "blend" | "opacity" | "fillOpacity");
    }
    matches!(
        last,
        "name"
            | "text"
            | "color"
            | "from"
            | "to"
            | "opacity"
            | "size"
            | "angle"
            | "distance"
            | "position"
            | "density"
            | "feather"
            | "enabled"
            | "brightness"
            | "contrast"
            | "hue"
            | "saturation"
            | "lightness"
            | "exposure"
            | "gamma"
            | "vibrance"
            | "scale"
            | "reverse"
            | "pattern"
            | "preset"
            | "blend"
            | "mode"
            | "style"
    )
}

/// Ranges describe each engine parameter's units; never reinterpret stored values.
pub(super) fn number(command: &str, key: &str, current: f64) -> (f64, f64, f64) {
    if key.contains("/midpoints/")
        || ((key.contains("/stops/")
            || key.contains("/transparency/")
            || key.contains("/opacityStops/"))
            && key.ends_with("/0"))
    {
        return (0.0, 1.0, 0.01);
    }
    let key = key.rsplit('/').next().unwrap_or(key);
    let (min, max, step): (f64, f64, f64) = match key {
        "opacity" | "fillOpacity" | "density" | "spread" | "choke" | "noise" | "range"
        | "smoothness" | "highlightOpacity" | "shadowOpacity" => (0.0, 100.0, 1.0),
        "brightness" | "contrast" | "saturation" | "lightness" | "vibrance" => (-100.0, 100.0, 1.0),
        "hue" | "angle" => (-180.0, 180.0, 1.0),
        "altitude" => (0.0, 90.0, 1.0),
        "gamma" => (0.01, 9.99, 0.01),
        "exposure" => (-10.0, 10.0, 0.1),
        "offset" if command.contains("xposure") => (-0.5, 0.5, 0.01),
        "scale" | "depth" => (1.0, 1000.0, 1.0),
        "size" | "distance" | "feather" | "soften" | "radius" => (0.0, 250.0, 1.0),
        "x" | "y" | "phaseX" | "phaseY" => (-8192.0, 8192.0, 1.0),
        "width" | "height" => (1.0, 8192.0, 1.0),
        _ => (0.0, 255.0, 1.0),
    };
    (min.min(current), max.max(current), step)
}

pub(super) fn can_preview(command: &str) -> bool {
    command.starts_with("layer.newAdjustmentLayer.")
        || command == "layer.setAdjustment"
        || command.starts_with("layer.newFillLayer.")
        || command.starts_with("layer.fill.")
        || command == "gradient.fill.set"
        || command == "layer.layerMask.edit"
        || command == "layer.layerStyle.replace"
        || command
            .strip_prefix("layer.layerStyle.")
            .is_some_and(|kind| {
                photocraft_engine::layer_style::effect_from_params(kind, &json!({})).is_some()
            })
}

pub(in crate::editor) fn enabled(action: LayerAction, doc: &PhotoDocument) -> bool {
    let layer = doc.active_layer();
    match action {
        LayerAction::Quick(command) | LayerAction::Command(command) => {
            if command.starts_with("layer.new")
                || command == "type.create"
                || command == "shape.create"
            {
                return doc.document.layer_count() < photo_document::MAX_LAYERS;
            }
            let Some(layer) = layer else { return false };
            match command {
                "layer.layerMask.revealAll" | "layer.layerMask.hideAll" => layer.mask.is_none(),
                "layer.layerMask.edit"
                | "layer.layerMask.delete"
                | "layer.layerMask.apply"
                | "layer.layerMask.enabled" => layer.mask.is_some(),
                "layer.arrange.bringForward" | "layer.arrange.sendBackward" | "layer.mergeDown" => {
                    let Some(path) = doc.document.path_of(layer.id) else {
                        return false;
                    };
                    let Some(index) = path.last().copied() else {
                        return false;
                    };
                    let siblings = if path.len() == 1 {
                        Some(doc.document.layers.as_slice())
                    } else {
                        doc.document
                            .layer_at(&path[..path.len() - 1])
                            .and_then(|l| l.children())
                    };
                    if command == "layer.arrange.bringForward" {
                        siblings.is_some_and(|s| index + 1 < s.len())
                    } else {
                        index > 0
                    }
                }
                _ => true,
            }
        }
        LayerAction::Group => doc.document.layer_count() < photo_document::MAX_LAYERS,
        _ => true,
    }
}

pub(super) fn menu(category: &str) -> Vec<(&'static str, LayerAction)> {
    use LayerAction::{Category, Command, Quick};
    match category {
        "Создать" => vec![
            ("Прозрачный слой для рисования", Quick("layer.new.layer")),
            ("Группа слоёв", LayerAction::Group),
            (
                "Одноцветная заливка…",
                Command("layer.newFillLayer.solidColor"),
            ),
            ("Градиент…", Command("layer.newFillLayer.gradient")),
            ("Коррекция света и цвета…", Category("Коррекция")),
            ("Текст…", Command("type.create")),
            ("Фигура…", Command("shape.create")),
        ],
        "Эффекты" => vec![
            ("Тень", Command("layer.layerStyle.dropShadow")),
            ("Обводка", Command("layer.layerStyle.stroke")),
            ("Наложение цвета", Command("layer.layerStyle.colorOverlay")),
            ("Внешнее свечение", Command("layer.layerStyle.outerGlow")),
            ("Внутренняя тень", Command("layer.layerStyle.innerShadow")),
            ("Внутреннее свечение", Command("layer.layerStyle.innerGlow")),
            ("Градиент", Command("layer.layerStyle.gradientOverlay")),
            ("Тиснение", Command("layer.layerStyle.bevelEmboss")),
            ("Узор", Command("layer.layerStyle.patternOverlay")),
            ("Глянец", Command("layer.layerStyle.satin")),
        ],
        "Коррекция" => vec![
            (
                "Яркость и контрастность",
                Command("layer.newAdjustmentLayer.brightnessContrast"),
            ),
            (
                "Цветовой тон и насыщенность",
                Command("layer.newAdjustmentLayer.hueSaturation"),
            ),
            (
                "Сочность цветов",
                Command("layer.newAdjustmentLayer.vibrance"),
            ),
            ("Экспозиция", Command("layer.newAdjustmentLayer.exposure")),
            (
                "Чёрно-белое изображение",
                Command("layer.newAdjustmentLayer.blackWhite"),
            ),
            (
                "Цветовой баланс",
                Command("layer.newAdjustmentLayer.colorBalance"),
            ),
            (
                "Фотофильтр",
                Command("layer.newAdjustmentLayer.photoFilter"),
            ),
            ("Уровни", Command("layer.newAdjustmentLayer.levels")),
            ("Кривые", Command("layer.newAdjustmentLayer.curves")),
        ],
        "Маски" => vec![
            (
                "Добавить маску: всё видно",
                Quick("layer.layerMask.revealAll"),
            ),
            (
                "Добавить маску: всё скрыто",
                Quick("layer.layerMask.hideAll"),
            ),
            ("Смягчить край маски…", Command("layer.layerMask.edit")),
            (
                "Убрать маску и показать рисунок",
                Quick("layer.layerMask.delete"),
            ),
            ("Применить маску к пикселям", Quick("layer.layerMask.apply")),
            (
                "Рисовать только внутри нижнего слоя",
                Quick("layer.createClippingMask"),
            ),
            (
                "Снять привязку к нижнему слою",
                Quick("layer.releaseClippingMask"),
            ),
        ],
        "Заливки" => vec![
            ("Один цвет…", Command("layer.newFillLayer.solidColor")),
            ("Градиент…", Command("layer.newFillLayer.gradient")),
            ("Узор…", Command("layer.newFillLayer.pattern")),
        ],
        _ => vec![
            ("Переименовать", Command("layer.renameLayer")),
            ("Дублировать", Quick("layer.duplicate")),
            ("Объединить с нижним", Quick("layer.mergeDown")),
            ("Создать…", Category("Создать")),
            ("Добавить эффект…", Category("Эффекты")),
            ("Маска и скрытие частей…", Category("Маски")),
            ("Коррекция света и цвета…", Category("Коррекция")),
            ("Вид панели…", Category("Вид панели")),
            ("Дополнительные инструменты…", Category("Дополнительно")),
            ("Удалить слой", Quick("layer.delete")),
        ],
    }
}
