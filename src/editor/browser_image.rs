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
    text(commands, group, handle, label, 12.0, TEXT);
    commands
        .spawn((
            TextInput,
            EditableText::new("1024"),
            TextCursorStyle::default(),
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
    let presets = node(
        commands,
        root,
        Node {
            width: percent(100),
            column_gap: px(6),
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(6),
            ..default()
        },
    );
    for (w, h) in [(512, 512), (1024, 1024), (1920, 1080)] {
        let button = button(
            commands,
            presets,
            font,
            format!("{w} × {h}"),
            Action::BrowserImageSize(w, h),
            Look::Regular,
        );
        commands
            .entity(button)
            .entry::<Node>()
            .and_modify(|mut node| {
                node.flex_grow = 1.0;
                node.justify_content = JustifyContent::Center;
                node.padding = UiRect::axes(px(6), px(7));
            });
    }
    let format = node(
        commands,
        root,
        Node {
            column_gap: px(8),
            align_items: AlignItems::Center,
            ..default()
        },
    );
    commands.spawn((
        Node {
            width: px(18),
            height: px(18),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(3)),
            ..default()
        },
        BackgroundColor(Color::WHITE),
        BorderColor::all(EDGE),
        Pickable::IGNORE,
        ChildOf(format),
    ));
    text(commands, format, font, "PNG · Белый фон", 12.0, TEXT);
    let limits = text(
        commands,
        root,
        font,
        "Размер в пикселях · До 8192 px по стороне и 16 Мп",
        11.0,
        MUTED,
    );
    commands.entity(limits).insert(TextLayout::default());
    SizeUi {
        root,
        width,
        height,
    }
}

impl SizeUi {
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
