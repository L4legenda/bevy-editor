//! Scene transform fields and property reverts.
use super::*;
use crate::editor::euler_degrees;
use icons::Glyph;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum TransformProperty {
    Position,
    Rotation,
    Scale,
    Lens,
}

impl TransformProperty {
    pub(in crate::editor) fn restore(self, mut current: Transform, saved: Transform) -> Transform {
        match self {
            Self::Position => current.translation = saved.translation,
            Self::Rotation => current.rotation = saved.rotation,
            Self::Scale => current.scale = saved.scale,
            Self::Lens => {}
        }
        current
    }

    fn field(self, axis: usize) -> Field {
        match self {
            Self::Position => Field::Position(axis),
            Self::Rotation => Field::Rotation(axis),
            Self::Scale => Field::Scale(axis),
            Self::Lens => Field::Lens,
        }
    }

    fn default_pose(editor: &Editor, target: Selection) -> Option<(Transform, Transform)> {
        let graph = editor.edits.hierarchy.as_deref();
        let key = editor.hierarchy_key(target);
        let node = key.and_then(|key| graph?.node(key));
        let current = node.map_or_else(|| editor.pose(target), |node| Some(node.local()))?;
        let original = key
            .and_then(|key| graph?.inherited_pose(key))
            .or_else(|| match target {
                Selection::Camera(shot) => {
                    if let Some(node) = node {
                        let reset = node.entity.components.get("camera")?.get("reset")?;
                        serde_json::from_value::<crate::CameraPlacement>(reset.clone())
                            .ok()
                            .map(|c| c.pose)
                    } else {
                        Some(editor.default_camera(shot).pose)
                    }
                }
                Selection::Imported(index) => editor.imported.get(index).map(|node| node.original),
                Selection::Light(id) => editor.default_light(id).map(|light| light.pose),
                Selection::Node(_) | Selection::Model(_) => Some(Transform::IDENTITY.into()),
                Selection::Environment => None,
            })?;
        Some((current.transform(), original.transform()))
    }

    fn reverted_pose(self, editor: &Editor, target: Selection) -> Option<Transform> {
        let (current, original) = Self::default_pose(editor, target)?;
        let allowed = [0, 1, 2].map(|axis| editor.field_allowed(target, self.field(axis)));
        let merge = |old: Vec3, value: Vec3| {
            Vec3::from_array(std::array::from_fn(|axis| {
                if allowed[axis] {
                    value[axis]
                } else {
                    old[axis]
                }
            }))
        };
        let mut reverted = current;
        let changed = match self {
            Self::Position => {
                reverted.translation = merge(current.translation, original.translation);
                !current.translation.abs_diff_eq(reverted.translation, 1e-5)
            }
            Self::Scale => {
                reverted.scale = merge(current.scale, original.scale);
                !current.scale.abs_diff_eq(reverted.scale, 1e-5)
            }
            Self::Rotation => {
                reverted.rotation = if allowed.into_iter().all(|axis| axis) {
                    original.rotation
                } else {
                    let degrees = merge(
                        euler_degrees(current.rotation),
                        euler_degrees(original.rotation),
                    );
                    Quat::from_euler(
                        EulerRot::YXZ,
                        degrees.y.to_radians(),
                        degrees.x.to_radians(),
                        degrees.z.to_radians(),
                    )
                };
                !current.rotation.abs_diff_eq(reverted.rotation, 1e-5)
                    && !current.rotation.abs_diff_eq(-reverted.rotation, 1e-5)
            }
            Self::Lens => false,
        };
        changed.then_some(reverted)
    }

    fn reverted_lens(editor: &Editor, shot: usize) -> Option<f32> {
        let original = if let Some(key) = editor.hierarchy_key(Selection::Camera(shot)) {
            let graph = editor.edits.hierarchy.as_deref()?;
            graph.inherited_lens(key).or_else(|| {
                let reset = graph
                    .node(key)?
                    .entity
                    .components
                    .get("camera")?
                    .get("reset")?;
                Some(reset.get("lens")?.as_f64()? as f32)
            })?
        } else {
            editor.default_camera(shot).lens
        };
        ((editor.camera(shot).lens - original).abs() > 1e-5).then_some(original)
    }

    pub(in crate::editor) fn can_revert(self, editor: &Editor) -> bool {
        if matches!(self, Self::Lens) {
            return !editor.multiple_selected()
                && matches!(editor.selection,
                Some(Selection::Camera(shot)) if Self::reverted_lens(editor, shot).is_some());
        }
        editor
            .inspector_targets(self.field(0))
            .any(|target| self.reverted_pose(editor, target).is_some())
    }

    pub(in crate::editor) fn revert(self, editor: &mut Editor) {
        if matches!(self, Self::Lens) {
            if !editor.multiple_selected()
                && let Some(target @ Selection::Camera(shot)) = editor.selection
                && let Some(lens) = Self::reverted_lens(editor, shot)
            {
                let mut placement = editor.camera(shot);
                placement.lens = lens;
                if let Some(key) = editor.hierarchy_key(target) {
                    placement.pose = editor
                        .edits
                        .hierarchy
                        .as_ref()
                        .unwrap()
                        .node(key)
                        .unwrap()
                        .local();
                    editor.edit_hierarchy(|graph| graph.set_camera_local(key, placement));
                } else {
                    editor.set_camera(shot, placement);
                }
            }
            return;
        }
        let poses: Vec<_> = editor
            .inspector_targets(self.field(0))
            .filter_map(|target| Some((target, self.reverted_pose(editor, target)?)))
            .collect();
        if poses.is_empty() {
            return;
        }
        if let Some(locals) = poses
            .iter()
            .map(|(target, pose)| Some((editor.hierarchy_key(*target)?, (*pose).into())))
            .collect::<Option<Vec<_>>>()
        {
            editor.edit_hierarchy(|graph| graph.set_locals(&locals));
        } else {
            for (target, pose) in poses {
                if let Some(key) = editor.hierarchy_key(target) {
                    editor.edit_hierarchy(|graph| graph.set_local(key, pose.into()));
                } else {
                    editor.set_pose(target, pose);
                }
            }
        }
    }
}

pub(super) struct Ui {
    pub transforms: Entity,
    pub rotation_scope: Entity,
    pub scale_scope: Entity,
    pub camera_scope: Entity,
    pub lens_label: Entity,
    pub space_label: Entity,
}

pub(super) fn setup(commands: &mut Commands, parent: Entity, font: &Handle<Font>) -> Ui {
    let transforms = node(commands, parent, column(0.0));
    let transform = inspector::section(
        commands,
        transforms,
        font,
        "Трансформация",
        "transform trs",
        true,
    );
    let space = control(
        commands,
        transform.header,
        Action::LocalSpace,
        Look::Regular,
        Node {
            height: px(23),
            min_width: px(68),
            justify_content: JustifyContent::Center,
            border_radius: BorderRadius::all(px(4)),
            ..row(0.0)
        },
    );
    commands
        .entity(space)
        .insert(AccessibleLabel("Пространство координат инспектора".into()));
    control_hints::attach(
        commands,
        space,
        "Мир или координаты относительно родителя · переключает также оси манипулятора",
    );
    let space_label = text(commands, space, font, "Мир", 11.0, MUTED);
    vector(
        commands,
        transform.body,
        font,
        TransformProperty::Position,
        "Позиция",
        "м",
        "position location translation перемещение",
    );
    let rotation_scope = node(commands, transform.body, column(0.0));
    vector(
        commands,
        rotation_scope,
        font,
        TransformProperty::Rotation,
        "Поворот",
        "°",
        "rotation euler угол вращение",
    );
    let scale_scope = node(commands, transform.body, column(0.0));
    vector(
        commands,
        scale_scope,
        font,
        TransformProperty::Scale,
        "Масштаб",
        "",
        "scale размер",
    );

    let camera_scope = node(commands, parent, column(0.0));
    let camera = inspector::section(
        commands,
        camera_scope,
        font,
        "Камера",
        "camera projection проекция",
        true,
    );
    let lens = node(commands, camera.body, column(2.0));
    inspector::searchable(
        commands,
        lens,
        "объектив обзор высота fov lens field of view ортографическая",
    );
    let lens_label = caption(
        commands,
        lens,
        font,
        TransformProperty::Lens,
        "Обзор, °",
        "",
    );
    number(
        commands,
        lens,
        font,
        Field::Lens,
        "Объектив камеры".to_owned(),
    );
    let buttons = node(
        commands,
        camera.body,
        Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(3),
            ..row(3.0)
        },
    );
    for (label, action, terms) in [
        (
            "Управление камерой",
            Action::Pilot,
            "управление камерой пилот pilot",
        ),
        ("Из вида", Action::Align, "из вида align выровнять"),
    ] {
        let item = node(commands, buttons, row(0.0));
        inspector::searchable(commands, item, terms);
        button(commands, item, font, label, action, Look::Regular);
    }
    Ui {
        transforms,
        rotation_scope,
        scale_scope,
        camera_scope,
        lens_label,
        space_label,
    }
}

fn caption(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    property: TransformProperty,
    label: &str,
    unit: &str,
) -> Entity {
    let line = node(
        commands,
        parent,
        Node {
            height: px(20),
            ..row(5.0)
        },
    );
    let title = text(commands, line, font, label, 12.0, TEXT);
    control_hints::attach(
        commands,
        title,
        "Щелчок по числу — ввод. Потяните влево или вправо. Shift уменьшает шаг.",
    );
    if !unit.is_empty() {
        text(commands, line, font, unit, 11.0, MUTED);
    }
    node(
        commands,
        line,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            ..default()
        },
    );
    let label = match property {
        TransformProperty::Position => "Вернуть исходную позицию",
        TransformProperty::Rotation => "Вернуть исходный поворот",
        TransformProperty::Scale => "Вернуть исходный масштаб",
        TransformProperty::Lens => "Вернуть исходный объектив",
    };
    let reset = inspector::icon_button(
        commands,
        line,
        Glyph::Undo,
        label,
        Action::RevertProperty(property),
    );
    control_hints::attach(commands, reset, label);
    title
}

fn vector(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    property: TransformProperty,
    label: &str,
    unit: &str,
    terms: &str,
) {
    let property_row = node(commands, parent, column(2.0));
    inspector::searchable(commands, property_row, &format!("{label} {terms}"));
    caption(commands, property_row, font, property, label, unit);
    let line = inspector::vector_row(commands, property_row);
    for (axis, name) in ["X", "Y", "Z"].into_iter().enumerate() {
        let cell = inspector::axis_cell(commands, line, font, axis);
        number(
            commands,
            cell,
            font,
            property.field(axis),
            format!("{label} {name}"),
        );
    }
}

fn number(
    commands: &mut Commands,
    parent: Entity,
    font_handle: &Handle<Font>,
    field: Field,
    label: String,
) {
    commands.spawn((
        field,
        Number::scene(field),
        TextInput,
        bevy::ui_widgets::SelectAllOnFocus,
        AccessibleLabel(label),
        EditableText::new("0.00"),
        crate::ui_theme::text_cursor_style(),
        font(font_handle, 12.0),
        TextColor(TEXT),
        TextLayout::no_wrap(),
        TabIndex(0),
        BackgroundColor(WELL),
        BorderColor::all(EDGE),
        ChildOf(parent),
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            height: px(26),
            padding: UiRect::axes(px(5), px(3)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
            overflow: Overflow::clip(),
            ..default()
        },
    ));
}
