//! Hierarchical outliner and instance controls in the scene inspector.
use super::*;
use crate::editor::scene_hierarchy::HierarchyAction as H;

fn action(commands: &mut Commands, parent: Entity, font: &Handle<Font>, label: &str, value: H) {
    button(
        commands,
        parent,
        font,
        label,
        Action::Hierarchy(value),
        Look::Regular,
    );
}
pub(super) fn toolbar(commands: &mut Commands, parent: Entity, font: &Handle<Font>) {
    let line = node(
        commands,
        parent,
        Node {
            flex_wrap: FlexWrap::Wrap,
            ..row(3.0)
        },
    );
    action(commands, line, font, "+ Группа", H::Group);
    action(commands, line, font, "Сгруппировать", H::GroupSelection);
    icons::button(
        commands,
        line,
        icons::Glyph::Refresh,
        "Обновить вложенные сцены с диска",
        Action::Hierarchy(H::Refresh),
        Look::Regular,
    );
}
pub(super) fn tree(commands: &mut Commands, parent: Entity, font: &Handle<Font>, editor: &Editor) {
    toolbar(commands, parent, font);
    scene_tree_drag::root_row(commands, parent, font);
    outline_entry(
        commands,
        parent,
        font,
        "Освещение сцены",
        Selection::Environment,
        None,
        true,
    );
    if editor.hierarchy_parenting.is_some() {
        text(
            commands,
            parent,
            font,
            "Выберите нового родителя",
            11.0,
            FOCUS,
        );
        action(
            commands,
            parent,
            font,
            "Отменить смену родителя",
            H::CancelParent,
        );
    }
    let Some(graph) = &editor.edits.hierarchy else {
        return;
    };
    let mut children = std::collections::HashMap::<Option<&str>, Vec<_>>::new();
    for node in &graph.nodes {
        children
            .entry(node.entity.parent.as_deref())
            .or_default()
            .push(node);
    }
    let mut pending: Vec<_> = children
        .get(&None)
        .into_iter()
        .flatten()
        .rev()
        .map(|n| (*n, 0usize))
        .collect();
    while let Some((entry, depth)) = pending.pop() {
        let key = entry.key;
        let target = editor.hierarchy_selection(key);
        let has_children = children.contains_key(&Some(entry.entity.id.as_str()))
            || usize::from(entry.entity.model.is_some())
                + usize::from(entry.camera.is_some())
                + usize::from(entry.entity.components.contains_key("light"))
                > 1;
        let collapsed = editor.hierarchy_collapsed.contains(&(editor.room, key));
        let row_node = node(
            commands,
            parent,
            Node {
                width: percent(100),
                height: px(28),
                padding: UiRect::left(px((depth.min(24) * 14) as f32)),
                flex_shrink: 0.0,
                ..row(0.0)
            },
        );
        let fold = control(
            commands,
            row_node,
            Action::Hierarchy(H::Fold(key)),
            Look::Regular,
            Node {
                width: px(18),
                height: px(28),
                flex_shrink: 0.0,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        );
        if has_children {
            icons::spawn(
                commands,
                fold,
                if collapsed {
                    icons::Glyph::ChevronRight
                } else {
                    icons::Glyph::ChevronDown
                },
                11.0,
                MUTED,
            );
        }
        let name = node(
            commands,
            row_node,
            Node {
                flex_grow: 1.0,
                flex_basis: px(0),
                min_width: px(0),
                overflow: Overflow::clip(),
                ..default()
            },
        );
        let label = format!(
            "{}{}{}",
            editor.label(target),
            if entry.instance.is_some() {
                " · сцена"
            } else {
                ""
            },
            if graph.overridden(key) { " *" } else { "" }
        );
        let hidden = editor.object_hidden(target);
        let light = if let Selection::Light(id) = target {
            editor.light(id)
        } else {
            None
        };
        let item = outline_entry(
            commands,
            name,
            font,
            &label,
            target,
            light.map(|l| l.kind),
            !hidden,
        );
        commands.entity(item).observe(scene_objects::pointer_menu);
        scene_tree_drag::mark(commands, item, Some(target));
        if entry.instance.is_some() {
            control_hints::attach(
                commands,
                item,
                "Экземпляр вложенной сцены. Правки дочерних объектов сохраняются в этом экземпляре; исходная сцена доступна в инспекторе.",
            );
        } else if entry.inherited() {
            control_hints::attach(
                commands,
                item,
                "Объект вложенной сцены. Изменения сохраняются как переопределения текущего экземпляра.",
            );
        }
        icons::button(
            commands,
            row_node,
            if hidden {
                icons::Glyph::EyeOff
            } else {
                icons::Glyph::Eye
            },
            if editor.visibility_inherited(target) {
                "Скрыто родителем. Сначала покажите родителя"
            } else {
                "Видимость объекта и его дочерних объектов"
            },
            Action::ToggleVisibility(target),
            Look::Regular,
        );
        if !collapsed && !matches!(target, Selection::Camera(_)) {
            if let Some(camera) = &entry.camera
                && let Some(shot) = editor
                    .definition()
                    .cameras
                    .iter()
                    .position(|c| c.id == *camera)
            {
                component(
                    commands,
                    parent,
                    font,
                    editor,
                    Selection::Camera(shot),
                    depth + 1,
                );
            }
        }
        if !collapsed
            && !matches!(target, Selection::Light(_))
            && entry.entity.components.contains_key("light")
        {
            component(
                commands,
                parent,
                font,
                editor,
                Selection::Light(key),
                depth + 1,
            );
        }
        if !collapsed && let Some(children) = children.get(&Some(entry.entity.id.as_str())) {
            pending.extend(children.iter().rev().map(|n| (*n, depth + 1)));
        }
    }
    text(
        commands,
        parent,
        font,
        "Перетащите файл сцены в 3D-вид, чтобы добавить экземпляр",
        10.0,
        MUTED,
    );
}
fn component(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    editor: &Editor,
    target: Selection,
    depth: usize,
) {
    let row = node(
        commands,
        parent,
        Node {
            width: percent(100),
            padding: UiRect::left(px((depth.min(24) * 14 + 18) as f32)),
            ..default()
        },
    );
    let light = if let Selection::Light(id) = target {
        editor.light(id)
    } else {
        None
    };
    outline_entry(
        commands,
        row,
        font,
        &editor.label(target),
        target,
        light.map(|l| l.kind),
        true,
    );
}
pub(super) fn inspector(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    editor: &Editor,
) {
    if !editor.has_scene() || editor.asset_viewer_open() {
        return;
    }
    let Some(target) = editor.selection else {
        return;
    };
    let Some(key) = editor.hierarchy_key(target) else {
        return;
    };
    let graph = editor.edits.hierarchy.as_ref().unwrap();
    let entry = graph.node(key).unwrap();
    let parent_label = graph
        .parent(key)
        .map(|n| editor.label(editor.hierarchy_selection(n.key)))
        .unwrap_or_else(|| "Корень сцены".into());
    text(
        commands,
        parent,
        font,
        format!("Родитель: {parent_label}"),
        11.0,
        MUTED,
    );
    let line = node(
        commands,
        parent,
        Node {
            flex_wrap: FlexWrap::Wrap,
            ..row(3.0)
        },
    );
    button(
        commands,
        line,
        font,
        "Переименовать",
        Action::SceneObject(SceneObjectAction::Rename(target)),
        Look::Regular,
    );
    action(commands, line, font, "Сменить родителя", H::PickParent);
    action(commands, line, font, "В корень", H::ToRoot);
    action(commands, line, font, "Выше", H::SiblingEarlier);
    action(commands, line, font, "Ниже", H::SiblingLater);
    let source = graph.source_path(key);
    if let Some(source) = source {
        let label = text(
            commands,
            parent,
            font,
            format!("Исходная сцена: {source}"),
            11.0,
            FOCUS,
        );
        commands.entity(label).insert(TextLayout::default());
        let line = node(
            commands,
            parent,
            Node {
                flex_wrap: FlexWrap::Wrap,
                ..row(3.0)
            },
        );
        action(commands, line, font, "Открыть исходную", H::OpenSource);
        if graph.overridden(key) {
            action(
                commands,
                line,
                font,
                "Сбросить изменения",
                H::ResetOverrides,
            );
        }
        if entry.instance.is_some() && !entry.inherited() {
            action(commands, line, font, "Распаковать", H::Unpack);
        }
    }
    text(
        commands,
        parent,
        font,
        "Локальные координаты задаются относительно родителя",
        10.0,
        MUTED,
    );
}
