//! File-card drag gestures and their overlay. Filesystem writes stay in actions.
use super::*;
use bevy::ui::CalculatedClip;
use std::path::Path;

const THRESHOLD: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DragPayload {
    pub entry: Entry,
    pub scan_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DropTarget {
    Folder(usize),
    Scene(Vec2),
}

#[derive(Clone, Copy)]
struct Ghost {
    root: Entity,
    label: Entity,
    hint: Entity,
}

#[derive(Resource, Default)]
pub(crate) struct AssetDrag {
    pub payload: Option<DragPayload>,
    pub dragging: bool,
    pub target: Option<DropTarget>,
    pub generation: u64,
    pub cancelled_this_frame: bool,
    origin: Option<Vec2>,
    suppress_click: bool,
    ghost: Option<Ghost>,
}

impl AssetDrag {
    pub fn active(&self) -> bool {
        self.dragging
    }

    // Activation observers run in PreUpdate, before release polling in Update.
    // Retain suppression through the release so a drag never becomes a click.
    pub fn suppress_activation(&self) -> bool {
        self.dragging || self.suppress_click
    }

    /// Also covers a fast release whose final movement arrives in this frame's
    /// PreUpdate, before the Update gesture system has seen that position.
    pub fn suppress_activation_at(&self, cursor: Option<Vec2>) -> bool {
        self.suppress_activation()
            || (self.payload.is_some()
                && self
                    .origin
                    .zip(cursor)
                    .is_some_and(|(origin, point)| origin.distance(point) >= THRESHOLD))
    }

    fn begin(&mut self, entry: Entry, scan_revision: u64, origin: Vec2) {
        self.payload = Some(DragPayload {
            entry,
            scan_revision,
        });
        self.origin = Some(origin);
        self.target = None;
        self.dragging = false;
        self.suppress_click = false;
        self.generation = self.generation.wrapping_add(1);
    }

    fn move_pointer(&mut self, cursor: Vec2) {
        if self.payload.is_some()
            && self
                .origin
                .is_some_and(|origin| origin.distance(cursor) >= THRESHOLD)
        {
            self.dragging = true;
            self.suppress_click = true;
        }
    }

    fn finish(&mut self) -> Option<(DragPayload, DropTarget)> {
        let drop = self.payload.zip(self.target).filter(|_| self.dragging);
        self.payload = None;
        self.origin = None;
        self.target = None;
        self.dragging = false;
        drop
    }

    fn cancel(&mut self) {
        self.cancelled_this_frame |= self.payload.is_some();
        self.suppress_click |= self.payload.is_some();
        self.finish();
    }
}

#[derive(Component)]
pub(crate) struct DropFolder(pub usize, pub u64);

fn hit(
    node: &ComputedNode,
    transform: &UiGlobalTransform,
    clip: Option<&CalculatedClip>,
    cursor: Vec2,
) -> bool {
    !node.is_empty()
        && node.contains_point(*transform, cursor)
        && clip.is_none_or(|clip| clip.contains_point(cursor))
}

fn folder_move_allowed(source: &Path, target: &Path) -> bool {
    !source.as_os_str().is_empty() && !target.starts_with(source) && source.parent() != Some(target)
}

fn destination_allowed(
    payload: DragPayload,
    folder: usize,
    browser: &Browser,
    editor: &Editor,
) -> bool {
    let Some(target) = browser.folders.get(folder) else {
        return false;
    };
    match payload.entry {
        Entry::Folder(index) => browser
            .folders
            .get(index)
            .is_some_and(|source| folder_move_allowed(&source.path, &target.path)),
        Entry::Model(index) => {
            editor.project.catalog.get(index).is_some_and(|model| {
                Path::new(&model.asset).parent() != Some(target.path.as_path())
            })
        }
        Entry::Scene { file: index, .. } | Entry::Image(index) | Entry::File(index) => browser
            .files
            .get(index)
            .is_some_and(|file| file.folder != folder),
    }
}

fn can_place(entry: Entry, browser: &Browser, editor: &Editor) -> bool {
    editor.has_scene()
        && editor.ready
        && browser
            .model_asset(&editor.project.catalog, entry)
            .is_some()
}

fn scene_target_contains(
    editor: &Editor,
    window: &Window,
    motion: &DrawerMotion,
    cursor: Vec2,
) -> bool {
    if !scene_input_rect(window, motion).contains(cursor) {
        return false;
    }
    if !editor.pilot
        && matches!(editor.selection, Some(Selection::Camera(_)))
        && camera_preview_rect(window, motion).contains(cursor)
    {
        return false;
    }
    if editor.pilot {
        let render = viewport_rect(window);
        let scale = window.scale_factor();
        let frame =
            viewport::frame_viewport((render.size() * scale).round().max(Vec2::ONE).as_uvec2());
        let min = render.min + frame.physical_position.as_vec2() / scale;
        return Rect::from_corners(min, min + frame.physical_size.as_vec2() / scale)
            .contains(cursor);
    }
    true
}

pub(crate) fn pointer(
    editor: Res<Editor>,
    browser: Res<Browser>,
    motion: Res<DrawerMotion>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut drag: ResMut<AssetDrag>,
    mut focus: Option<ResMut<InputFocus>>,
    cards: Query<(
        &FileCard,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&CalculatedClip>,
    )>,
    folders: Query<(
        &DropFolder,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&CalculatedClip>,
    )>,
    mut actions: MessageWriter<Action>,
    lighting: Option<Res<LightingUi>>,
) {
    drag.cancelled_this_frame = false;
    if mouse.just_pressed(MouseButton::Left)
        || (!mouse.pressed(MouseButton::Left) && !mouse.just_released(MouseButton::Left))
    {
        drag.suppress_click = false;
    }
    if !window.focused
        || lighting
            .as_ref()
            .is_some_and(|lighting| lighting.blocks_scene_input())
        || !editor.drawer
        || editor.pending.is_some()
        || motion.resizing
        || keys.just_pressed(KeyCode::Escape)
        || drag
            .payload
            .is_some_and(|payload| payload.scan_revision != browser.scan_revision)
    {
        drag.cancel();
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        if mouse.just_released(MouseButton::Left) {
            drag.cancel();
        }
        return;
    };
    let physical = cursor * window.scale_factor();
    if mouse.just_pressed(MouseButton::Left) {
        let over_panel = cursor.y >= window.height() - FOOT - motion.visible_height
            && cursor.y < window.height() - FOOT;
        if over_panel {
            let source = cards
                .iter()
                .filter(|(card, node, transform, clip)| {
                    card.1 == browser.scan_revision && hit(node, transform, *clip, physical)
                })
                .min_by(|(_, a, _, _), (_, b, _, _)| {
                    (a.size().x * a.size().y).total_cmp(&(b.size().x * b.size().y))
                })
                .map(|(card, _, _, _)| card.0);
            if let Some(entry) = source {
                drag.begin(entry, browser.scan_revision, cursor);
            }
        }
    }
    if drag.payload.is_none() {
        return;
    }
    if mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left) {
        drag.move_pointer(cursor);
    }
    if drag.dragging {
        if let Some(focus) = focus.as_mut() {
            focus.clear();
        }
        let Some(payload) = drag.payload else {
            return;
        };
        let folder = folders
            .iter()
            .filter(|(folder, node, transform, clip)| {
                folder.1 == browser.scan_revision
                    && destination_allowed(payload, folder.0, &browser, &editor)
                    && hit(node, transform, *clip, physical)
            })
            .min_by(|(_, a, _, _), (_, b, _, _)| {
                (a.size().x * a.size().y).total_cmp(&(b.size().x * b.size().y))
            })
            .map(|(folder, _, _, _)| folder.0);
        drag.target = folder.map(DropTarget::Folder).or_else(|| {
            (can_place(payload.entry, &browser, &editor)
                && scene_target_contains(&editor, &window, &motion, cursor))
            .then_some(DropTarget::Scene(cursor))
        });
    }
    if mouse.just_released(MouseButton::Left) || !mouse.pressed(MouseButton::Left) {
        if let Some((payload, target)) = drag.finish() {
            match target {
                DropTarget::Folder(folder) => {
                    actions.write(Action::MoveAsset(payload.entry, folder));
                }
                DropTarget::Scene(position) => {
                    actions.write(Action::DropAsset(payload.entry, position));
                }
            }
        }
    }
}

fn entry_label(payload: DragPayload, browser: &Browser, editor: &Editor) -> String {
    match payload.entry {
        Entry::Folder(index) => browser
            .folders
            .get(index)
            .map(|folder| folder.label.clone()),
        Entry::Model(index) => editor.project.catalog.get(index).map(|item| {
            item.asset
                .rsplit('/')
                .next()
                .unwrap_or(&item.asset)
                .to_owned()
        }),
        Entry::Scene { file: index, .. } | Entry::Image(index) | Entry::File(index) => {
            browser.files.get(index).map(|file| file.filename.clone())
        }
    }
    .unwrap_or_else(|| "Файл".into())
}

pub(crate) fn visuals(
    mut commands: Commands,
    mut drag: ResMut<AssetDrag>,
    browser: Res<Browser>,
    editor: Res<Editor>,
    interface: Res<BrowserUi>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<(&mut Text, &mut TextColor)>,
    mut targets: Query<(
        &DropFolder,
        &mut BorderColor,
        Option<&mut BackgroundColor>,
        Has<Control>,
    )>,
) {
    for (folder, mut border, background, control) in &mut targets {
        let highlighted = drag.dragging
            && folder.1 == browser.scan_revision
            && drag.target == Some(DropTarget::Folder(folder.0));
        if highlighted || !control {
            border.set_if_neq(BorderColor::all(if highlighted {
                BLUE
            } else {
                Color::NONE
            }));
        }
        if let Some(mut background) = background {
            if highlighted {
                background.set_if_neq(BackgroundColor(ACTIVE));
            } else if !control {
                background.set_if_neq(BackgroundColor(Color::NONE));
            }
        }
    }
    let Some(payload) = drag.payload.filter(|_| drag.dragging) else {
        if let Some(ghost) = drag.ghost.take() {
            commands.entity(ghost.root).despawn();
        }
        return;
    };
    let caption = entry_label(payload, &browser, &editor);
    let (hint, color) = match drag.target {
        Some(DropTarget::Folder(folder)) => (
            format!("Переместить в {}", browser.folders[folder].label),
            GREEN,
        ),
        Some(DropTarget::Scene(_)) => ("Добавить в сцену".into(), BLUE),
        None => ("Здесь нельзя разместить".into(), MUTED),
    };
    let cursor = window.cursor_position().unwrap_or(Vec2::ZERO);
    let left = px((cursor.x + 16.0).clamp(8.0, (window.width() - 288.0).max(8.0)));
    let top = px((cursor.y + 16.0).clamp(TOP, (window.height() - FOOT - 64.0).max(TOP)));
    let ghost = if let Some(ghost) = drag.ghost {
        ghost
    } else {
        let root = surface(
            &mut commands,
            interface.overlay,
            Node {
                position_type: PositionType::Absolute,
                left,
                top,
                max_width: px(280),
                padding: UiRect::axes(px(11), px(8)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(6)),
                overflow: Overflow::clip(),
                ..column(4.0)
            },
            CHROME,
        );
        commands
            .entity(root)
            .insert((ZIndex(100), Pickable::IGNORE));
        let label = text(&mut commands, root, &interface.font, &caption, 12.0, TEXT);
        let hint_label = text(&mut commands, root, &interface.font, &hint, 11.0, color);
        let ghost = Ghost {
            root,
            label,
            hint: hint_label,
        };
        drag.ghost = Some(ghost);
        ghost
    };
    if let Ok(mut node) = nodes.get_mut(ghost.root) {
        node.left = left;
        node.top = top;
    }
    for (entity, value, color) in [(ghost.label, caption, TEXT), (ghost.hint, hint, color)] {
        if let Ok((mut label, mut tint)) = labels.get_mut(entity) {
            if label.0 != value {
                label.0 = value;
            }
            tint.set_if_neq(TextColor(color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_becomes_a_drag_only_after_the_threshold_and_suppresses_activation() {
        let mut drag = AssetDrag::default();
        drag.begin(Entry::Model(0), 4, Vec2::ZERO);
        drag.move_pointer(Vec2::new(3.0, 3.0));
        assert!(!drag.active());
        assert!(!drag.suppress_activation());
        assert!(!drag.suppress_activation_at(Some(Vec2::new(3.0, 3.0))));
        assert!(drag.suppress_activation_at(Some(Vec2::new(6.0, 0.0))));
        drag.move_pointer(Vec2::new(6.0, 0.0));
        assert!(drag.active());
        assert!(drag.suppress_activation());
        drag.target = Some(DropTarget::Folder(2));
        assert_eq!(
            drag.finish(),
            Some((
                DragPayload {
                    entry: Entry::Model(0),
                    scan_revision: 4
                },
                DropTarget::Folder(2)
            ))
        );
        assert!(drag.suppress_activation());
        assert!(drag.payload.is_none());
    }

    #[test]
    fn cancel_discards_a_pending_drop_and_keeps_release_suppressed() {
        let mut drag = AssetDrag::default();
        drag.begin(Entry::Image(0), 8, Vec2::ZERO);
        drag.move_pointer(Vec2::splat(20.0));
        drag.target = Some(DropTarget::Folder(3));
        drag.cancel();
        assert!(!drag.active());
        assert!(drag.finish().is_none());
        assert!(drag.suppress_activation());
    }

    #[test]
    fn folder_targets_reject_self_descendants_and_noop_moves() {
        let source = Path::new("models/props");
        assert!(!folder_move_allowed(source, source));
        assert!(!folder_move_allowed(
            source,
            Path::new("models/props/chairs")
        ));
        assert!(!folder_move_allowed(source, Path::new("models")));
        assert!(folder_move_allowed(source, Path::new("models/props_extra")));
        assert!(folder_move_allowed(source, Path::new("")));
        assert!(!folder_move_allowed(Path::new(""), Path::new("models")));
    }

    #[test]
    fn scene_and_generic_glbs_are_placeable_only_with_a_ready_open_scene() {
        let mut project = crate::project::EditorProject::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"),
        )
        .unwrap();
        let browser = Browser::new(&mut project);
        let file = browser
            .files
            .iter()
            .position(|file| file.path == Path::new(&project.scenes[0].asset))
            .unwrap();
        let mut editor = Editor::new(project, crate::data::LevelEdits::default());
        let scene = Entry::Scene { file, room: 0 };
        assert!(!can_place(scene, &browser, &editor));
        editor.ready = true;
        assert!(can_place(scene, &browser, &editor));
        assert!(can_place(Entry::File(file), &browser, &editor));
        assert!(!can_place(Entry::Image(file), &browser, &editor));
        editor.open_tabs.clear();
        assert!(!can_place(scene, &browser, &editor));
    }

    #[test]
    fn camera_preview_and_pilot_letterbox_bars_are_not_drop_targets() {
        let project = crate::project::EditorProject::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"),
        )
        .unwrap();
        let mut editor = Editor::new(project, crate::data::LevelEdits::default());
        editor.selection = Some(Selection::Camera(0));
        let window = Window {
            resolution: (1280, 760).into(),
            ..default()
        };
        let motion = DrawerMotion::default();
        assert!(!scene_target_contains(
            &editor,
            &window,
            &motion,
            camera_preview_rect(&window, &motion).center()
        ));
        editor.pilot = true;
        let render = viewport_rect(&window);
        assert!(!scene_target_contains(
            &editor,
            &window,
            &motion,
            render.min + Vec2::new(100.0, 5.0)
        ));
        assert!(scene_target_contains(
            &editor,
            &window,
            &motion,
            render.center()
        ));
    }

    #[test]
    fn pointer_system_emits_a_scene_drop_instead_of_a_card_click() {
        use bevy::ecs::message::MessageCursor;
        let mut project = crate::project::EditorProject::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"),
        )
        .unwrap();
        let browser = Browser::new(&mut project);
        let mut editor = Editor::new(project, crate::data::LevelEdits::default());
        editor.drawer = true;
        editor.ready = true;
        let mut motion = DrawerMotion::default();
        motion.advance(true, DURATION, 350.0);
        let mut window = Window {
            resolution: (1280, 760).into(),
            focused: true,
            ..default()
        };
        window.set_cursor_position(Some(Vec2::new(400.0, 600.0)));
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(editor)
            .insert_resource(browser)
            .insert_resource(motion)
            .init_resource::<AssetDrag>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<Action>()
            .add_systems(Update, pointer);
        let window_entity = app.world_mut().spawn((window, PrimaryWindow)).id();
        app.world_mut().spawn((
            FileCard(Entry::Model(0), 0),
            ComputedNode {
                size: Vec2::splat(100.0),
                ..default()
            },
            UiGlobalTransform::from_xy(400.0, 600.0),
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert!(!app.world().resource::<AssetDrag>().active());
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear_just_pressed(MouseButton::Left);
        app.world_mut()
            .get_mut::<Window>(window_entity)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(500.0, 300.0)));
        app.update();
        assert!(app.world().resource::<AssetDrag>().active());
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        let mut cursor = MessageCursor::<Action>::default();
        let drops: Vec<_> = cursor
            .read(app.world().resource::<Messages<Action>>())
            .filter_map(|action| {
                if let Action::DropAsset(entry, point) = action {
                    Some((*entry, *point))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(drops, vec![(Entry::Model(0), Vec2::new(500.0, 300.0))]);
        assert!(!app.world().resource::<AssetDrag>().active());
        assert!(app.world().resource::<AssetDrag>().suppress_activation());
    }
}
