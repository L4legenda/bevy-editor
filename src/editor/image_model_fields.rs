//! Slide-out transform controls for a selected model on the image canvas.
use super::*;
use crate::editor::image_reference::ReferenceRender;
use bevy::window::RequestRedraw;
use photocraft_doc::{DocId, LayerId};

const TOP: f32 = 10.0;
const TAB_WIDTH: f32 = 30.0;
const PANEL_INSET: f32 = TAB_WIDTH + 14.0;
const PANEL_WIDTH: f32 = 284.0;
const DURATION: f32 = 0.18;

#[derive(Clone, Copy)]
enum ComponentRow {
    Position,
    Rotation,
    Scale,
}

impl ComponentRow {
    fn values(self, pose: crate::Pose) -> [f32; 3] {
        match self {
            Self::Position => pose.position,
            Self::Scale => pose.scale,
            Self::Rotation => crate::editor::euler_degrees(pose.transform().rotation).to_array(),
        }
    }
    fn change(self, mut pose: crate::Pose, axis: usize, text: &str) -> Option<crate::Pose> {
        let n: f32 = text.trim().replace(',', ".").parse().ok()?;
        if !n.is_finite() {
            return None;
        }
        match self {
            Self::Position => pose.position[axis] = n,
            Self::Scale => pose.scale[axis] = n,
            Self::Rotation => {
                let mut angles = self.values(pose);
                angles[axis] = n.rem_euclid(360.0);
                let [x, y, z] = angles.map(f32::to_radians);
                pose.rotation = Quat::from_euler(EulerRot::YXZ, y, x, z).to_array();
            }
        }
        pose.valid().then_some(pose)
    }
}
#[derive(Component)]
pub(in crate::editor) struct PoseField(ComponentRow, usize);
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum PanelPart {
    Tab,
    Reveal,
    Card,
}

#[derive(Component)]
pub(in crate::editor) struct Disclosure;

pub(super) fn setup(commands: &mut Commands, canvas: Entity, font: &Handle<Font>) {
    let tab = control(
        commands,
        canvas,
        Action::ImageEdit(edit::EditAction::ModelPanel),
        Look::Tool,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            right: px(8),
            top: px(TOP),
            width: px(TAB_WIDTH),
            height: px(32),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            justify_content: JustifyContent::Center,
            ..row(0.0)
        },
    );
    commands.entity(tab).insert((
        PanelPart::Tab,
        CanvasControl,
        ZIndex(17),
        AccessibleLabel("Панель трансформации 3D · N".into()),
    ));
    let arrow = icons::themed(commands, tab, icons::Glyph::ChevronLeft, 15.0);
    commands.entity(arrow).insert(Disclosure);

    let reveal = node(
        commands,
        canvas,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            right: px(PANEL_INSET),
            top: px(TOP),
            width: px(0),
            overflow: Overflow::clip(),
            ..default()
        },
    );
    commands
        .entity(reveal)
        .insert((PanelPart::Reveal, CanvasControl, ZIndex(16)));
    let card = surface(
        commands,
        reveal,
        Node {
            width: px(PANEL_WIDTH),
            flex_shrink: 0.0,
            padding: UiRect::all(px(12)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            overflow: Overflow::scroll_y(),
            ..column(7.0)
        },
        CHROME,
    );
    commands.entity(card).insert((PanelPart::Card, Scroller));
    text(commands, card, font, "Трансформация 3D", 13.0, TEXT);

    let header = node(
        commands,
        card,
        Node {
            width: percent(100),
            ..row(3.0)
        },
    );
    node(
        commands,
        header,
        Node {
            width: px(54),
            flex_shrink: 0.0,
            ..default()
        },
    );
    for (axis, color) in ["X", "Y", "Z"]
        .into_iter()
        .zip(crate::editor::viewport::AXIS_COLORS)
    {
        let title = text(commands, header, font, axis, 11.0, color);
        commands.entity(title).insert(Node {
            width: px(0),
            flex_grow: 1.0,
            ..default()
        });
    }
    for (label, kind) in [
        ("Позиция", ComponentRow::Position),
        ("Поворот °", ComponentRow::Rotation),
        ("Масштаб", ComponentRow::Scale),
    ] {
        let line = node(
            commands,
            card,
            Node {
                width: percent(100),
                ..row(3.0)
            },
        );
        let caption = text(commands, line, font, label, 11.0, MUTED);
        commands.entity(caption).insert(Node {
            width: px(54),
            flex_shrink: 0.0,
            ..default()
        });
        for (i, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
            let input = edit::interface::input(commands, line, font, &format!("{label} {axis}"));
            commands
                .entity(input)
                .insert(PoseField(kind, i))
                .entry::<Node>()
                .and_modify(|mut n| {
                    n.width = px(0);
                    n.min_width = px(0);
                    n.flex_grow = 1.0;
                    n.padding = UiRect::axes(px(2), px(4));
                });
        }
    }
}

type Target = (usize, DocId, LayerId);
struct Draft {
    entity: Entity,
    original: String,
}
#[derive(Default)]
pub(in crate::editor) struct Fields {
    target: Option<Target>,
    displayed: Option<crate::Pose>,
    open: bool,
    progress: f32,
    draft: Option<Draft>,
    invalid: std::collections::HashSet<Entity>,
}

pub(in crate::editor) fn update(
    editor: Res<Editor>,
    mut viewer: ResMut<ImageViewer>,
    mut render: ResMut<ReferenceRender>,
    mut state: Local<Fields>,
    mut focus: ResMut<InputFocus>,
    keys: Res<ButtonInput<KeyCode>>,
    mut fields: Query<(Entity, &PoseField, &mut EditableText, &mut BorderColor)>,
    mut parts: Query<(&PanelPart, &mut Node)>,
    canvas: Query<&ComputedNode, With<ImageCanvas>>,
    arrow: Query<Entity, With<Disclosure>>,
    time: Res<Time>,
    mut redraw: MessageWriter<RequestRedraw>,
    mut commands: Commands,
) {
    let selection = editor.viewing_image.and_then(|tab| {
        if !viewer.edit.editing(Some(tab)) || editor.pending.is_some() {
            return None;
        }
        let doc = viewer.edit.document(Some(tab))?;
        let pose = viewer.edit.model_reference(Some(tab))?.pose;
        Some(((tab, doc.document.id, doc.active_id()?), pose))
    });
    let target = selection.map(|(t, _)| t);
    if state.target != target {
        if focus.get().is_some_and(|e| fields.contains(e)) {
            focus.clear();
        }
        state.target = target;
        state.displayed = None;
        state.draft = None;
        state.invalid.clear();
    }
    let open = selection.is_some() && viewer.edit.model_panel_open;
    let previous = state.progress;
    state.progress = if selection.is_some() {
        let direction = if open { 1.0 } else { -1.0 };
        (previous + direction * time.delta_secs().min(0.05) / DURATION).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if state.progress != previous || state.progress != f32::from(open) {
        redraw.write(RequestRedraw);
    }
    let canvas_size = canvas
        .single()
        .map_or(Vec2::ZERO, |node| node.size() * node.inverse_scale_factor());
    let width = (canvas_size.x - PANEL_INSET - 8.0).clamp(0.0, PANEL_WIDTH);
    let progress = state.progress;
    let revealed = width * progress * progress * (3.0 - 2.0 * progress);
    for (part, mut node) in &mut parts {
        let visible = match part {
            PanelPart::Tab => selection.is_some(),
            PanelPart::Reveal => {
                let width = px(revealed);
                if node.width != width {
                    node.width = width;
                }
                revealed > 0.0
            }
            PanelPart::Card => {
                let width = px(width);
                let max_height = px((canvas_size.y - 2.0 * TOP).max(0.0));
                if node.width != width || node.max_height != max_height {
                    node.width = width;
                    node.max_height = max_height;
                }
                true
            }
        };
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    if state.open != open {
        if let Ok(arrow) = arrow.single() {
            icons::set_glyph(
                &mut commands,
                arrow,
                if open {
                    icons::Glyph::ChevronRight
                } else {
                    icons::Glyph::ChevronLeft
                },
            );
        }
        state.open = open;
    }
    let Some(((tab, _, _), saved)) = selection else {
        return;
    };
    let mut pose = saved;
    if !open && focus.get().is_some_and(|e| fields.contains(e)) {
        focus.clear();
    }
    let active = focus.get().filter(|e| fields.contains(*e));
    let cancel = keys.just_pressed(KeyCode::Escape);
    let submit = keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter])
        || (keys.just_pressed(KeyCode::KeyS)
            && keys.any_pressed([
                KeyCode::ControlLeft,
                KeyCode::ControlRight,
                KeyCode::SuperLeft,
                KeyCode::SuperRight,
            ]));
    let previous_focus = state.draft.as_ref().map(|d| d.entity);
    if state
        .draft
        .as_ref()
        .is_some_and(|d| Some(d.entity) != active || cancel || submit)
    {
        let draft = state.draft.take().unwrap();
        if let Ok((entity, field, value, _)) = fields.get_mut(draft.entity) {
            if cancel {
                state.invalid.remove(&entity);
            } else if value.value() != draft.original.as_str() || state.invalid.contains(&entity) {
                if let Some(next) = field.0.change(pose, field.1, &value.value().to_string()) {
                    state.invalid.remove(&entity);
                    if next != pose {
                        let doc = viewer.edit.document_mut(tab).unwrap();
                        match render.transform_selected(tab, doc, next) {
                            Ok(()) => pose = next,
                            Err(error) => viewer.edit.error = Some(error),
                        }
                    }
                } else {
                    state.invalid.insert(entity);
                }
            }
        }
        if (cancel || submit)
            && active == Some(draft.entity)
            && (cancel || !state.invalid.contains(&draft.entity))
        {
            focus.clear();
        }
    }
    let display = viewer
        .model_drag
        .as_ref()
        .filter(|_| !viewer.edit.model_posing)
        .map_or(pose, |drag| drag.preview);
    for (entity, field, mut input, mut border) in &mut fields {
        let focused = focus.get() == Some(entity);
        let invalid = state.invalid.contains(&entity);
        if !focused
            && !invalid
            && (state.displayed != Some(display) || previous_focus == Some(entity))
        {
            let value = format!("{:.4}", field.0.values(display)[field.1]);
            let value = value.trim_end_matches('0').trim_end_matches('.');
            if input.value() != value {
                input.editor_mut().set_text(value);
            }
        }
        if focused && state.draft.is_none() {
            state.draft = Some(Draft {
                entity,
                original: input.value().to_string(),
            });
            input.queue_edit(bevy::text::TextEdit::SelectAll);
        }
        border.set_if_neq(BorderColor::all(if invalid {
            Color::srgb(1.0, 0.34, 0.25)
        } else if focused {
            FOCUS
        } else {
            EDGE
        }));
    }
    state.displayed = Some(display);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_fields_use_scene_angles_and_validate_before_committing() {
        let start = Transform::IDENTITY.into();
        let moved = ComponentRow::Position.change(start, 1, "-2,5").unwrap();
        assert_eq!(moved.position, [0., -2.5, 0.]);
        for invalid in ["NaN", "inf", "100001", ""] {
            assert!(ComponentRow::Position.change(start, 0, invalid).is_none());
        }
        for invalid in ["0", "0.00001", "-1001"] {
            assert!(ComponentRow::Scale.change(start, 0, invalid).is_none());
        }
        let scaled = ComponentRow::Scale.change(moved, 1, "-1,25").unwrap();
        assert_eq!(scaled.scale, [1., -1.25, 1.]);
        let mut rotated = scaled;
        for (axis, degrees) in ["17", "-28", "33"].iter().enumerate() {
            rotated = ComponentRow::Rotation
                .change(rotated, axis, degrees)
                .unwrap();
        }
        assert!(
            Vec3::from(ComponentRow::Rotation.values(rotated))
                .abs_diff_eq(Vec3::new(17., -28., 33.), 1e-4)
        );
        assert_eq!(rotated.scale, scaled.scale);
        assert_eq!(rotated.position, moved.position);
    }
}
