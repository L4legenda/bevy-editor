//! Size controls for the browser's white PNG creation dialog.
use super::*;

pub(super) struct SizeUi {
    pub root: Entity,
    pub width: Entity,
    pub height: Entity,
}

fn dimension(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    label: &str,
) -> Entity {
    let group = node(
        commands,
        parent,
        Node {
            flex_grow: 1.0,
            flex_basis: px(0),
            min_width: px(0),
            ..column(5.0)
        },
    );
    text(commands, group, handle, format!("{label}, px"), 12.0, MUTED);
    commands
        .spawn((
            TextInput,
            Number::decimal(1.0, 8192.0, 1.0, 0),
            EditableText::new("1024"),
            crate::ui_theme::text_cursor_style(),
            AccessibleLabel(format!("{label} изображения")),
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
        .id()
}

pub(super) fn setup(commands: &mut Commands, dialog: Entity, font: &Handle<Font>) -> SizeUi {
    let root = node(
        commands,
        dialog,
        Node {
            display: Display::None,
            width: percent(100),
            ..column(10.0)
        },
    );
    let fields = node(
        commands,
        root,
        Node {
            width: percent(100),
            ..row(12.0)
        },
    );
    let width = dimension(commands, fields, font, "Ширина");
    let height = dimension(commands, fields, font, "Высота");
    SizeUi {
        root,
        width,
        height,
    }
}

impl SizeUi {
    pub fn limits(&self, inputs: &Query<&EditableText>, numbers: &mut Query<&mut Number>) {
        for (entity, other) in [(self.width, self.height), (self.height, self.width)] {
            let other = inputs
                .get(other)
                .ok()
                .and_then(|text| text.value().to_string().trim().parse::<u32>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(1);
            let max = (crate::editor::photo_document::MAX_PIXELS / u64::from(other)).clamp(1, 8192)
                as f32;
            if let Ok(mut number) = numbers.get_mut(entity) {
                number.set_if_neq(Number::decimal(1.0, max, 1.0, 0));
            }
        }
    }
    pub fn set(&self, inputs: &mut Query<&mut EditableText>, width: u32, height: u32) {
        for (entity, value) in [(self.width, width), (self.height, height)] {
            if let Ok(mut input) = inputs.get_mut(entity) {
                input.editor_mut().set_text(&value.to_string());
            }
        }
    }

    pub fn signature(&self, inputs: &Query<&mut EditableText>) -> String {
        format!(
            "{}:{}",
            inputs
                .get(self.width)
                .map(|input| input.value().to_string())
                .unwrap_or_default(),
            inputs
                .get(self.height)
                .map(|input| input.value().to_string())
                .unwrap_or_default()
        )
    }

    pub fn read(&self, inputs: &Query<&mut EditableText>) -> Result<(u32, u32), String> {
        let value = |entity, label| {
            inputs
                .get(entity)
                .ok()
                .and_then(|input| input.value().to_string().trim().parse::<u32>().ok())
                .ok_or_else(|| format!("{label}: введите целое число пикселей."))
        };
        let (width, height) = (value(self.width, "Ширина")?, value(self.height, "Высота")?);
        browser_files::validate_image_size(width, height)?;
        Ok((width, height))
    }
}
