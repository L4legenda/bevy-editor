//! Hierarchical outliner and instance controls in the scene inspector.
use super::*;
use crate::editor::scene_hierarchy::HierarchyAction as H;
use bevy::ui::{UiTransform, Val2};
use std::time::Instant;

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
fn toolbar(commands: &mut Commands, parent: Entity, font: &Handle<Font>) {
    let line = node(
        commands,
        parent,
        Node {
            flex_wrap: FlexWrap::Wrap,
            ..row(3.0)
        },
    );
    action(commands, line, font, "+ Папка", H::AddFolder);
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
    if editor.definition().is_native() {
        toolbar(commands, parent, font);
        scene_tree_drag::root_row(commands, parent, font);
    }
    outline_entry(
        commands,
        parent,
        font,
        "Освещение сцены",
        Selection::Environment,
        None,
        true,
        icons::Glyph::Folder,
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
        let targets = viewport::shots(editor)
            .map(Selection::Camera)
            .chain(
                editor
                    .imported
                    .iter()
                    .enumerate()
                    .map(|(i, _)| Selection::Imported(i)),
            )
            .chain(
                editor
                    .edits
                    .models
                    .iter()
                    .map(|model| Selection::Model(model.id)),
            )
            .chain(
                editor
                    .edits
                    .lights
                    .iter()
                    .map(|light| Selection::Light(light.id)),
            );
        for target in targets.filter(|&target| !editor.object_deleted(target)) {
            object_outline_entry(commands, parent, font, editor, target);
        }
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
                border_radius: BorderRadius::all(px(6)),
                flex_shrink: 0.0,
                ..row(0.0)
            },
        );
        if let Some((room, folder, started)) = editor.folder_arrival
            && room == editor.room
            && folder == key
            && let Some((transform, fill)) = folder_frame(started.elapsed().as_secs_f32())
        {
            commands
                .entity(row_node)
                .insert((FolderArrival(started), transform, fill));
        }
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
            !hidden && light.is_none_or(|light| light.enabled),
            if entry.instance.is_some() {
                icons::Glyph::Layers
            } else if has_children && !collapsed {
                icons::Glyph::FolderOpen
            } else {
                icons::Glyph::Folder
            },
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
#[derive(Component)]
pub(in crate::editor) struct FolderArrival(Instant);

fn folder_frame(elapsed: f32) -> Option<(UiTransform, BackgroundColor)> {
    let progress = elapsed / 0.36;
    if progress >= 1.0 {
        return None;
    }
    let remaining = 1.0 - progress;
    Some((
        UiTransform::from_translation(Val2::px(10.0 * remaining.powi(3), 0.0)),
        BackgroundColor(AMBER.with_alpha(0.28 * remaining.powi(2))),
    ))
}

pub(in crate::editor) fn animate_folders(
    mut commands: Commands,
    mut rows: Query<(
        Entity,
        &FolderArrival,
        &mut UiTransform,
        &mut BackgroundColor,
    )>,
) {
    for (entity, arrival, mut transform, mut fill) in &mut rows {
        if let Some((next, color)) = folder_frame(arrival.0.elapsed().as_secs_f32()) {
            *transform = next;
            *fill = color;
        } else {
            *transform = UiTransform::default();
            *fill = BackgroundColor(Color::NONE);
            commands.entity(entity).remove::<FolderArrival>();
        }
    }
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
        icons::Glyph::Folder,
    );
}
pub(super) struct Inspector {
    relations: Entity,
    instance: Entity,
    relations_scope: Entity,
    instance_scope: Entity,
}

pub(super) fn setup_inspector(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
) -> Inspector {
    let relations = node(
        commands,
        parent,
        Node {
            display: Display::None,
            ..column(0.0)
        },
    );
    let instance = node(
        commands,
        parent,
        Node {
            display: Display::None,
            ..column(0.0)
        },
    );
    Inspector {
        relations_scope: relations,
        instance_scope: instance,
        relations: inspector::section(
            commands,
            relations,
            font,
            "Узел сцены",
            "node object hierarchy иерархия",
            false,
        )
        .body,
        instance: inspector::section(
            commands,
            instance,
            font,
            "Экземпляр сцены",
            "instance prefab исходная вложенная overrides",
            true,
        )
        .body,
    }
}

pub(super) fn inspector(
    commands: &mut Commands,
    ui: &Inspector,
    font: &Handle<Font>,
    editor: &Editor,
) {
    let target = (editor.has_scene() && !editor.asset_viewer_open() && !editor.multiple_selected())
        .then_some(editor.selection)
        .flatten();
    let hierarchy = target.and_then(|target| {
        let graph = editor.edits.hierarchy.as_deref()?;
        let key = editor.hierarchy_key(target)?;
        Some((graph, key))
    });
    let source = hierarchy.and_then(|(graph, key)| graph.source_path(key));
    let rename = target
        .filter(|&target| editor.object_editable(target) && !matches!(target, Selection::Light(_)));
    for (scope, body, show) in [
        (
            ui.relations_scope,
            ui.relations,
            hierarchy.is_some() || rename.is_some(),
        ),
        (ui.instance_scope, ui.instance, source.is_some()),
    ] {
        commands.entity(body).despawn_related::<Children>();
        commands
            .entity(scope)
            .entry::<Node>()
            .and_modify(move |mut node| {
                node.display = if show { Display::Flex } else { Display::None };
            });
    }
    let relation_actions = node(
        commands,
        ui.relations,
        Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(3),
            ..row(3.0)
        },
    );
    if let Some(target) = rename {
        let item = node(commands, relation_actions, row(0.0));
        inspector::searchable(commands, item, "переименовать имя rename name");
        button(
            commands,
            item,
            font,
            "Переименовать",
            Action::SceneObject(SceneObjectAction::Rename(target)),
            Look::Regular,
        );
    }
    let Some((graph, key)) = hierarchy else {
        return;
    };
    let parent = graph.parent(key).map(|n| editor.hierarchy_selection(n.key));
    let label = parent.map_or_else(|| "Корень сцены".into(), |target| editor.label(target));
    let label = text(
        commands,
        ui.relations,
        font,
        format!("Родитель · {label}"),
        11.0,
        MUTED,
    );
    inspector::searchable(commands, label, "родитель parent корень");
    commands.entity(label).insert(TextLayout::default());
    for (label, command, terms) in [
        ("Сменить родителя", H::PickParent, "родитель reparent"),
        ("В корень", H::ToRoot, "родитель корень root"),
        ("Выше", H::SiblingEarlier, "порядок выше order"),
        ("Ниже", H::SiblingLater, "порядок ниже order"),
    ] {
        if matches!(command, H::ToRoot) && parent.is_none() {
            continue;
        }
        let item = node(commands, relation_actions, row(0.0));
        inspector::searchable(commands, item, terms);
        action(commands, item, font, label, command);
    }
    if let Some(source) = source {
        let overridden = graph.overridden(key);
        let hint = if overridden {
            "Есть переопределения"
        } else {
            "Связано с исходной сценой"
        };
        let status = text(
            commands,
            ui.instance,
            font,
            hint,
            11.0,
            if overridden { FOCUS } else { MUTED },
        );
        inspector::searchable(commands, status, "переопределения overrides");
        let path = text(commands, ui.instance, font, source, 11.0, MUTED);
        commands.entity(path).insert(TextLayout::default());
        inspector::searchable(commands, path, &format!("источник source {source}"));
        let actions = node(
            commands,
            ui.instance,
            Node {
                flex_wrap: FlexWrap::Wrap,
                row_gap: px(3),
                ..row(3.0)
            },
        );
        let entry = graph.node(key).unwrap();
        for (show, label, command, terms) in [
            (
                true,
                "Открыть исходную",
                H::OpenSource,
                "открыть источник source open",
            ),
            (
                overridden,
                "Сбросить переопределения",
                H::ResetOverrides,
                "сбросить переопределения reset revert",
            ),
            (
                entry.instance.is_some() && !entry.inherited(),
                "Распаковать",
                H::Unpack,
                "распаковать unpack",
            ),
        ] {
            if show {
                let item = node(commands, actions, row(0.0));
                inspector::searchable(commands, item, terms);
                action(commands, item, font, label, command);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::world::CommandQueue;

    fn render_tree(editor: &Editor) -> World {
        let mut world = World::new();
        let root = world.spawn(Node::default()).id();
        let mut queue = CommandQueue::default();
        tree(
            &mut Commands::new(&mut queue, &world),
            root,
            &Handle::default(),
            editor,
        );
        queue.apply(&mut world);
        world
    }

    fn labels(world: &mut World) -> Vec<String> {
        world
            .query::<&Text>()
            .iter(world)
            .map(|text| text.0.clone())
            .collect()
    }

    #[test]
    fn scene_tree_stays_unified_when_the_first_folder_is_added() {
        let fixture = crate::editor::scene_hierarchy::tests::fixture();
        let settings = &fixture.project.scenes[0];
        let flat = crate::LevelEdits {
            scene: Some(crate::SceneSettings {
                label: settings.label.clone(),
                cameras: settings.cameras.clone(),
            }),
            models: vec![crate::PlacedModel {
                selector: None,
                uid: None,
                id: 1,
                asset: "models/block.glb".into(),
                pose: Transform::IDENTITY.into(),
                bounds: None,
            }],
            model_names: std::collections::BTreeMap::from([(1, "Тестовый объект".into())]),
            lights: vec![crate::LightPreset::Bulb.create(1)],
            ..default()
        };
        std::fs::write(
            fixture.root.join("room.scene"),
            serde_json::to_vec(&flat).unwrap(),
        )
        .unwrap();
        let mut editor = Editor::new(fixture.project.clone(), flat);
        editor.ready = true;
        let light_name = editor.edits.lights[0].name.clone();
        let common = [
            light_name.as_str(),
            "+ Папка",
            "Корень сцены",
            "Освещение сцены",
            "Главная камера",
            "Тестовый объект",
        ];
        let before = labels(&mut render_tree(&editor));
        assert!(
            common
                .iter()
                .all(|label| before.iter().any(|text| text == label))
        );

        editor.hierarchy_action(H::AddFolder);
        let folder = editor.selection.unwrap();
        assert_eq!(editor.label(folder), "Папка");
        let first = editor
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.entity.parent.is_none())
            .unwrap();
        assert_eq!(folder, Selection::Node(first.key));
        let (_, _, started) = editor.folder_arrival.unwrap();
        assert!(folder_frame(0.0).is_some());
        assert!(folder_frame(0.15).is_some());
        assert!(folder_frame(0.36).is_none());
        assert!(started <= Instant::now());

        let after = labels(&mut render_tree(&editor));
        assert!(
            common
                .iter()
                .all(|label| after.iter().any(|text| text == label))
        );
        assert!(after.iter().any(|text| text == "Папка"));
        for label in ["ОСВЕЩЕНИЕ", "КАМЕРЫ", "ОБЪЕКТЫ СЦЕНЫ", "Сгруппировать"]
        {
            assert!(!before.iter().chain(&after).any(|text| text == label));
        }
    }
}
