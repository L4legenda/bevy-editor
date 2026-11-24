//! Numeric editing of a selected PhotoCraft model reference.
use super::*;
use crate::editor::image_reference::ReferenceRender;
use photocraft_doc::{DocId, LayerId};

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
#[derive(Component)]
pub(in crate::editor) struct PanelPart(bool);

#[derive(Component)]
pub(in crate::editor) struct Disclosure;

pub(super) fn setup(commands: &mut Commands, panel: Entity, font: &Handle<Font>) {
    let root = node(
        commands,
        panel,
        Node {
            display: Display::None,
            width: percent(100),
            flex_shrink: 0.0,
            ..column(3.0)
        },
    );
    commands.entity(root).insert(PanelPart(false));
    let toggle = button(
        commands,
        root,
        font,
        "Трансформация 3D",
        Action::ImageEdit(edit::EditAction::ModelPanel),
        Look::Regular,
    );
    let arrow = icons::themed(commands, toggle, icons::Glyph::ChevronDown, 14.0);
    commands.entity(arrow).insert(Disclosure);
    let rows = node(commands, root, column(2.0));
    commands.entity(rows).insert(PanelPart(true));
    let header = node(
        commands,
        rows,
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
            rows,
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
    collapsed: Option<bool>,
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
    arrow: Query<Entity, With<Disclosure>>,
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
    for (part, mut node) in &mut parts {
        let display = if selection.is_some() && (!part.0 || !viewer.edit.model_panel_closed) {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    let target = selection.map(|(t, _)| t);
    if state.target != target {
        if focus.get().is_some_and(|e| fields.contains(e)) {
            focus.clear();
        }
        *state = Fields {
            target,
            ..default()
        };
    }
    let Some(((tab, _, layer), saved)) = selection else {
        return;
    };
    if state.collapsed != Some(viewer.edit.model_panel_closed) {
        if let Ok(arrow) = arrow.single() {
            icons::set_glyph(
                &mut commands,
                arrow,
                if viewer.edit.model_panel_closed {
                    icons::Glyph::ChevronRight
                } else {
                    icons::Glyph::ChevronDown
                },
            );
        }
        state.collapsed = Some(viewer.edit.model_panel_closed);
    }
    let mut pose = render.pending_pose(tab, layer).unwrap_or(saved);
    let active = focus
        .get()
        .filter(|e| fields.contains(*e) && !viewer.edit.model_panel_closed);
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
                        let doc = viewer.edit.document(Some(tab)).unwrap();
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
    let display = viewer.model_drag.as_ref().map_or(pose, |drag| drag.preview);
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
