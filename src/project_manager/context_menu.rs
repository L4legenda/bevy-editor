//! Project actions belong to the card under the context click.
use super::*;
use bevy::ui::CalculatedClip;

const WIDTH: f32 = 224.0;
const HEIGHT: f32 = 148.0;

#[derive(Component)]
pub(in crate::project_manager) struct MenuItem;
#[derive(Component)]
pub(in crate::project_manager) struct MenuPanel;

type Geometry<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static ComputedNode,
        &'static UiGlobalTransform,
        Option<&'static CalculatedClip>,
    ),
>;

fn contains(
    node: &ComputedNode,
    transform: &UiGlobalTransform,
    clip: Option<&CalculatedClip>,
    point: Vec2,
) -> bool {
    let local = transform.affine().inverse().transform_point2(point);
    node.size().min_element() > 0.0
        && local.abs().cmple(node.size() * 0.5).all()
        && clip.is_none_or(|clip| clip.contains_point(point))
}

pub(in crate::project_manager) fn input(
    manager: Res<Manager>,
    picker: Res<Picker>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    wheel: Res<AccumulatedMouseScroll>,
    geometry: Geometry,
    controls: Query<&Control>,
    panels: Query<(), With<MenuPanel>>,
    mut actions: MessageWriter<Action>,
) {
    if !window.focused || manager.form.is_some() || picker.busy() || wheel.delta != Vec2::ZERO {
        if manager.context.is_some() {
            actions.write(Action::DismissContext);
        }
        return;
    }
    let keyboard = keys.just_pressed(KeyCode::ContextMenu)
        || (keys.just_pressed(KeyCode::F10)
            && keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]));
    if keyboard {
        for (entity, node, transform, _) in &geometry {
            if let Ok(Control {
                action: Action::Select(file),
                ..
            }) = controls.get(entity)
                && manager.selected.as_ref() == Some(file)
            {
                let position = (transform.affine().translation
                    + Vec2::new(-node.size().x * 0.5 + 20.0, 16.0))
                    / window.scale_factor();
                actions.write(Action::Context(file.clone(), position));
                return;
            }
        }
    }
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let point = cursor * window.scale_factor();
    let inside_menu = geometry.iter().any(|(entity, node, transform, clip)| {
        panels.contains(entity) && contains(node, transform, clip, point)
    });
    if mouse.just_pressed(MouseButton::Right) && !inside_menu {
        for (entity, node, transform, clip) in &geometry {
            if let Ok(Control {
                action: Action::Select(file),
                ..
            }) = controls.get(entity)
                && contains(node, transform, clip, point)
            {
                actions.write(Action::Context(file.clone(), cursor));
                return;
            }
        }
        if manager.context.is_some() {
            actions.write(Action::DismissContext);
        }
    } else if mouse.just_pressed(MouseButton::Left) && manager.context.is_some() && !inside_menu {
        actions.write(Action::DismissContext);
    }
}

pub(in crate::project_manager) fn refresh(
    mut commands: Commands,
    manager: Res<Manager>,
    mut ui: ResMut<Interface>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut panels: Query<&mut Node, With<MenuPanel>>,
    mut focus: ResMut<InputFocus>,
) {
    if ui.context_state != manager.context {
        if let Some(overlay) = ui.context_root.take() {
            commands.entity(overlay).despawn();
        }
        ui.context_state.clone_from(&manager.context);
        if let Some(context) = &manager.context {
            let overlay = commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: percent(100),
                        height: percent(100),
                        ..default()
                    },
                    GlobalZIndex(150),
                    TabGroup::modal(),
                    ChildOf(ui.root),
                ))
                .id();
            ui.context_root = Some(overlay);
            let panel = surface(
                &mut commands,
                overlay,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(context
                        .position
                        .x
                        .clamp(8.0, (window.width() - WIDTH - 8.0).max(8.0))),
                    top: px(context
                        .position
                        .y
                        .clamp(8.0, (window.height() - HEIGHT - 20.0).max(8.0))),
                    width: px(WIDTH),
                    height: px(HEIGHT),
                    padding: UiRect::all(px(6)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(8)),
                    ..column(2.0)
                },
                CHROME,
            );
            commands
                .entity(panel)
                .insert((MenuPanel, effects::Reveal::panel()));
            let favorite = manager
                .recent
                .projects
                .iter()
                .find(|project| project.file == context.file)
                .is_some_and(|project| project.favorite);
            for (index, (label, action)) in [
                ("Открыть", Action::Open(context.file.clone())),
                (
                    if favorite {
                        "Убрать из избранного"
                    } else {
                        "В избранное"
                    },
                    Action::Favorite,
                ),
                ("Показать папку", Action::Reveal),
                ("Убрать из списка", Action::Forget),
            ]
            .into_iter()
            .enumerate()
            {
                let (item, label) =
                    button(&mut commands, panel, &ui.font, label, action, false, false);
                commands.entity(item).insert((
                    MenuItem,
                    Node {
                        width: percent(100),
                        height: px(32),
                        min_height: px(32),
                        flex_shrink: 0.0,
                        padding: UiRect::axes(px(9), px(5)),
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(5)),
                        ..row(6.0)
                    },
                ));
                commands
                    .entity(label)
                    .insert(TextFont::from_font_size(14.0).with_font(ui.font.clone()));
                if index == 0 {
                    focus.set(item, FocusCause::Auto);
                }
            }
        }
    }
    if let Some(context) = &manager.context {
        for mut panel in &mut panels {
            panel.left = px(context
                .position
                .x
                .clamp(8.0, (window.width() - WIDTH - 8.0).max(8.0)));
            panel.top = px(context
                .position
                .y
                .clamp(8.0, (window.height() - HEIGHT - 20.0).max(8.0)));
        }
    }
}
