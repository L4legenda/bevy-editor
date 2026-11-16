use super::*;

#[derive(Resource)]
pub(in crate::editor) struct PlayUi {
    button: Entity,
    icon: Entity,
    label: Entity,
    error_panel: Entity,
    error_text: Entity,
}

pub(super) fn visible(editor: &Editor) -> bool {
    editor.has_scene() && !editor.asset_viewer_open()
}

pub(super) fn setup(commands: &mut Commands, bar: Entity, root: Entity, font: &Handle<Font>) {
    let button = control(
        commands,
        bar,
        Action::Play,
        Look::Regular,
        Node {
            height: px(29),
            padding: UiRect::horizontal(px(10)),
            flex_shrink: 0.0,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..row(7.0)
        },
    );
    commands
        .entity(button)
        .insert(AccessibleLabel("Запустить открытый уровень (F5)".into()));
    let icon = icons::spawn(
        commands,
        button,
        icons::Glyph::Play,
        16.0,
        Color::srgb(0.45, 0.86, 0.40),
    );
    let label = text(commands, button, font, "Запустить", 13.0, TEXT);
    let error_panel = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: px(86),
            right: px(14),
            max_width: px(480),
            padding: UiRect::all(px(12)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(7)),
            ..column(6.0)
        },
        WELL,
    );
    commands
        .entity(error_panel)
        .insert((GlobalZIndex(1400), Pickable::IGNORE));
    let error_text = text(
        commands,
        error_panel,
        font,
        "",
        13.0,
        Color::srgb(1.0, 0.48, 0.42),
    );
    commands.entity(error_text).insert(TextLayout::default());
    commands.insert_resource(PlayUi {
        button,
        icon,
        label,
        error_panel,
        error_text,
    });
}

pub(in crate::editor) fn update(
    mut commands: Commands,
    editor: Res<Editor>,
    ui: Res<PlayUi>,
    mut texts: Query<&mut Text>,
    mut nodes: Query<&mut Node>,
    labels: Query<&AccessibleLabel>,
) {
    let running = editor.play.running();
    if let Ok(label) = labels.get(ui.button) {
        let value = if running {
            "Остановить игру (F5)"
        } else {
            "Запустить открытый уровень (F5)"
        };
        if label.0 != value {
            commands
                .entity(ui.button)
                .insert(AccessibleLabel(value.into()));
        }
    }
    if let Ok(mut label) = texts.get_mut(ui.label) {
        let value = if running {
            "Остановить"
        } else {
            "Запустить"
        };
        if label.0 != value {
            label.0 = value.into();
        }
    }
    icons::set_glyph(
        &mut commands,
        ui.icon,
        if running {
            icons::Glyph::Stop
        } else {
            icons::Glyph::Play
        },
    );
    icons::set_tint(
        &mut commands,
        ui.icon,
        if running {
            Color::srgb(1.0, 0.44, 0.37)
        } else {
            Color::srgb(0.45, 0.86, 0.40)
        },
    );
    if let Ok(mut panel) = nodes.get_mut(ui.error_panel) {
        panel.display = if visible(&editor) && editor.play.error.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok(mut label) = texts.get_mut(ui.error_text) {
        let value = editor.play.error.as_deref().unwrap_or("");
        if label.0 != value {
            label.0 = value.into();
        }
    }
}
