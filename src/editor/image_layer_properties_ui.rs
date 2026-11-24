//! Contextual layer properties, with one clear way to edit each existing effect.
use super::*;

fn heading(commands: &mut Commands, parent: Entity, font: &Handle<Font>, label: &str) {
    let row = node(
        commands,
        parent,
        Node {
            width: percent(100),
            margin: UiRect::top(px(10)),
            padding: UiRect::top(px(8)),
            border: UiRect::top(px(1)),
            ..row(4.0)
        },
    );
    commands.entity(row).insert(BorderColor::all(EDGE));
    text(commands, row, font, label, 12.0, TEXT);
}

pub(super) fn build(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    doc: &PhotoDocument,
    _state: &LayerState,
) {
    let Some(layer) = doc.active_layer() else {
        return;
    };
    let selected = doc.active;
    let kind = match layer_kind(layer) {
        "Растровый" => "Слой рисунка",
        other => other,
    };
    text(commands, parent, font, kind, 11.0, MUTED);
    if doc.selected.len() > 1 {
        text(
            commands,
            parent,
            font,
            &format!(
                "Выбрано слоёв: {}. Общие настройки применяются ко всем.",
                doc.selected.len()
            ),
            11.0,
            MUTED,
        );
    }
    if matches!(layer.content, LayerContent::Adjustment(_)) && !doc.mask_target {
        let detail = photocraft_engine::inspect::layer_detail(layer);
        let name = detail["adjustment"]["kind"].as_str().map_or_else(
            || "Коррекция".into(),
            |kind| title(&format!("layer.newAdjustmentLayer.{kind}")),
        );
        compact(
            commands,
            parent,
            font,
            &format!("Настроить: {name}"),
            LayerAction::Properties,
        );
    } else if !matches!(
        layer.content,
        LayerContent::Raster(_) | LayerContent::Group(_)
    ) || doc.mask_target
    {
        compact(
            commands,
            parent,
            font,
            if doc.mask_target {
                "Настроить маску…"
            } else {
                "Настроить содержимое…"
            },
            LayerAction::Properties,
        );
    }
    heading(commands, parent, font, "Маска · скрыть часть рисунка");
    if let Some(mask) = &layer.mask {
        let options = node(
            commands,
            parent,
            Node {
                width: percent(100),
                flex_wrap: FlexWrap::Wrap,
                ..row(4.0)
            },
        );
        for (label, action) in [
            ("Рисунок", LayerAction::Select(selected, false, false)),
            ("Рисовать по маске", LayerAction::Mask(selected)),
            ("Настроить", LayerAction::Command("layer.layerMask.edit")),
            (
                if mask.enabled {
                    "Выключить"
                } else {
                    "Включить"
                },
                LayerAction::Quick("layer.layerMask.enabled"),
            ),
        ] {
            compact(commands, options, font, label, action);
        }
        let hint = text(
            commands,
            parent,
            font,
            if doc.mask_target {
                "Сейчас выбрана маска"
            } else {
                "Сейчас выбран рисунок"
            },
            11.0,
            FOCUS,
        );
        ux::help(
            commands,
            hint,
            "На маске чёрная кисть скрывает изображение, белая — возвращает. Чтобы снова рисовать цветом, нажмите «Рисунок».",
        );
    } else {
        compact(
            commands,
            parent,
            font,
            "Добавить маску",
            LayerAction::Category("Добавить маску"),
        );
    }
    heading(commands, parent, font, "Эффекты");
    for (index, effect) in layer.effects.items.iter().enumerate() {
        let row = node(
            commands,
            parent,
            Node {
                width: percent(100),
                ..row(3.0)
            },
        );
        let toggle = icons::button(
            commands,
            row,
            if effect.enabled() {
                icons::Glyph::Eye
            } else {
                icons::Glyph::EyeOff
            },
            "Включить / выключить эффект",
            act(LayerAction::EffectToggle(selected, index)),
            Look::Regular,
        );
        commands.entity(toggle).entry::<Node>().and_modify(|mut n| {
            n.width = px(26);
            n.height = px(28);
            n.padding = UiRect::ZERO;
        });
        ux::help(
            commands,
            toggle,
            "Временно скрыть эффект для сравнения. Его настройки сохраняются. Нажмите ещё раз, чтобы вернуть.",
        );
        let name = title(&format!(
            "layer.layerStyle.{}",
            photocraft_engine::layer_style::kind_of(effect)
        ));
        let edit = compact(
            commands,
            row,
            font,
            &name,
            LayerAction::EffectEdit(selected, index),
        );
        commands.entity(edit).entry::<Node>().and_modify(|mut n| {
            n.flex_grow = 1.0;
            n.min_width = px(0);
        });
        ux::help(
            commands,
            edit,
            format!(
                "{name} · открыть настройки. Ползунки сразу показывают результат на холсте; «Отмена» вернёт прежний вид."
            ),
        );
        let remove = icons::button(
            commands,
            row,
            icons::Glyph::Close,
            "Удалить эффект",
            act(LayerAction::EffectRemove(selected, index)),
            Look::Regular,
        );
        commands.entity(remove).entry::<Node>().and_modify(|mut n| {
            n.width = px(25);
            n.height = px(28);
            n.padding = UiRect::ZERO;
        });
        ux::help(
            commands,
            remove,
            "Удалить только этот эффект. Сам рисунок и другие эффекты сохранятся. Ctrl+Z отменяет удаление.",
        );
    }
    compact(
        commands,
        parent,
        font,
        "Добавить эффект…",
        LayerAction::Category("Эффекты"),
    );
    heading(commands, parent, font, "Свет и цвет");
    let extra = node(
        commands,
        parent,
        Node {
            width: percent(100),
            flex_wrap: FlexWrap::Wrap,
            ..row(4.0)
        },
    );
    compact(
        commands,
        extra,
        font,
        "Добавить коррекцию…",
        LayerAction::Category("Коррекция"),
    );
    compact(
        commands,
        extra,
        font,
        "Параметры смешивания…",
        LayerAction::Command("layer.layerStyle.blendingOptions"),
    );
}
