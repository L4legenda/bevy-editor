//! File deletion, recoverable trash and a captured-card black-hole transition.
use super::*;
use crate::editor::browser::Entry;
use bevy::{
    asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path},
    camera::RenderTarget,
    reflect::TypePath,
    render::{
        render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat},
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
    shader::ShaderRef,
    ui::IsDefaultUiCamera,
};

#[path = "browser_delete_blocked.rs"]
mod blocked;
#[cfg(test)]
#[path = "browser_delete_tests.rs"]
mod tests;
#[path = "asset_trash.rs"]
mod trash;

const DURATION: f32 = 0.88;
const MARGIN: f32 = 56.0;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct BlackHoleMaterial {
    #[uniform(0)]
    state: Vec4,
    #[texture(1)]
    #[sampler(2)]
    snapshot: Handle<Image>,
}

impl UiMaterial for BlackHoleMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("asset_black_hole.wesl"))
                .with_source("embedded"),
        )
    }
}

#[derive(Clone)]
struct Target {
    card: Entity,
    revision: u64,
    path: PathBuf,
}

struct Pending {
    id: u64,
    target: Target,
    wait: u8,
    requested: bool,
    age: f32,
    rect: Rect,
    snapshot: Option<Image>,
    effect: Option<(Entity, Handle<BlackHoleMaterial>)>,
}

struct DeletedFile {
    file: trash::TrashedFile,
    material: Handle<BlackHoleMaterial>,
}

struct Restoration {
    path: PathBuf,
    revision: u64,
    material: Handle<BlackHoleMaterial>,
    age: f32,
    card: Option<Entity>,
    effect: Option<Entity>,
}

impl Restoration {
    fn finish(self, commands: &mut Commands) {
        if let Some(card) = self.card {
            commands.entity(card).try_insert(Visibility::Inherited);
        }
        if let Some(effect) = self.effect {
            commands.entity(effect).try_despawn();
        }
    }
}

#[derive(Resource, Default)]
struct Deletion {
    selection: Option<Target>,
    menu_target: Option<Target>,
    pending: Option<Pending>,
    history: Vec<DeletedFile>,
    restoring: Option<Restoration>,
    next_id: u64,
}

#[derive(Resource)]
struct Menu {
    remove: Entity,
}

#[derive(Component)]
struct CaptureToken(u64);

pub(in crate::editor) struct BrowserDeletionPlugin;
impl Plugin for BrowserDeletionPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "asset_black_hole.wesl");
        blocked::register(app);
        app.add_plugins(UiMaterialPlugin::<BlackHoleMaterial>::default())
            .init_resource::<Deletion>()
            .add_systems(Last, empty_trash_on_exit)
            .add_systems(Startup, setup.after(crate::editor::ui::setup))
            .add_systems(
                Update,
                (
                    input
                        .after(super::input)
                        .before(crate::editor::ui::shortcuts),
                    update
                        .after(input)
                        .after(super::update)
                        .before(crate::editor::ui::refresh_browser),
                    menu.after(super::refresh).after(crate::editor::ui::refresh),
                    restore_effect.after(super::refresh),
                ),
            );
    }
}

fn empty_trash_on_exit(
    mut exits: MessageReader<AppExit>,
    editor: Res<Editor>,
    mut deletion: ResMut<Deletion>,
) {
    // WindowCloseRequested can still be cancelled in the save dialog.
    // AppExit is emitted only after that dialog has allowed the editor to exit.
    if exits.read().last().is_none() {
        return;
    }
    match trash::empty(&editor.project.asset_root) {
        Ok(()) => {
            deletion.history.clear();
            deletion.pending = None;
            deletion.restoring = None;
        }
        Err(error) => warn!("Не удалось очистить корзину при выходе: {error}"),
    }
}

fn setup(mut commands: Commands, context: Res<ContextUi>, interface: Res<Interface>) {
    let remove = button(
        &mut commands,
        context.menu,
        &interface.font,
        "Удалить файл",
        Action::BrowserDeleteFile,
        Look::Regular,
    );
    commands
        .entity(remove)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.width = percent(100);
            node.height = px(34);
            node.display = Display::None;
        });
    let icon = icons::themed(&mut commands, remove, icons::Glyph::Trash, 16.0);
    commands.entity(remove).insert_children(0, &[icon]);
    commands.insert_resource(Menu { remove });
}

fn target(
    entity: Entity,
    card: &drawer::FileCard,
    browser: &Browser,
    editor: &Editor,
) -> Option<Target> {
    if card.1 != browser.scan_revision {
        return None;
    }
    let path = match card.0 {
        Entry::Folder(_) => return None,
        Entry::Model(index) => PathBuf::from(&editor.project.catalog.get(index)?.asset),
        Entry::Scene { file, .. } | Entry::Image(file) | Entry::File(file) => {
            browser.files.get(file)?.path.clone()
        }
    };
    Some(Target {
        card: entity,
        revision: card.1,
        path,
    })
}

fn at_pointer(
    point: Vec2,
    geometry: &Geometry,
    cards: &Query<&drawer::FileCard>,
    browser: &Browser,
    editor: &Editor,
) -> Option<Target> {
    geometry
        .iter()
        .filter_map(|(entity, node, _, _)| {
            (hit(geometry, entity, point))
                .then(|| {
                    target(entity, cards.get(entity).ok()?, browser, editor)
                        .map(|target| (node.size().element_product(), target))
                })
                .flatten()
        })
        .min_by(|(a, _), (b, _)| a.total_cmp(b))
        .map(|(_, target)| target)
}

fn input(
    editor: Res<Editor>,
    browser: Res<Browser>,
    context: Res<BrowserContext>,
    interface: Res<drawer::BrowserUi>,
    drag: Res<AssetDrag>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    focus: Res<InputFocus>,
    inputs: Query<(), With<TextInput>>,
    geometry: Geometry,
    cards: Query<&drawer::FileCard>,
    mut deletion: ResMut<Deletion>,
    mut actions: MessageWriter<Action>,
) {
    if !editor.drawer
        || editor.pending.is_some()
        || editor.hdri_library_open
        || !window.focused
        || drag.active()
    {
        return;
    }
    let point = window
        .cursor_position()
        .map(|point| point * window.scale_factor());
    let inside = point.is_some_and(|point| hit(&geometry, interface.panel, point));
    if mouse.just_pressed(MouseButton::Right) && context.phase == Phase::Menu {
        deletion.menu_target = context.menu_entry.and_then(|entry| {
            target(
                entry.entity,
                cards.get(entry.entity).ok()?,
                &browser,
                &editor,
            )
        });
    }
    if mouse.just_pressed(MouseButton::Left) && context.phase == Phase::Closed {
        deletion.selection = point
            .filter(|_| inside)
            .and_then(|point| at_pointer(point, &geometry, &cards, &browser, &editor));
    }
    if focus.get().is_some_and(|entity| inputs.contains(entity)) || context.phase.dialog() {
        return;
    }
    let focused = focus
        .get()
        .and_then(|entity| target(entity, cards.get(entity).ok()?, &browser, &editor));
    if (inside || focused.is_some()) && keys.just_pressed(KeyCode::Delete) {
        // Consume the key before scene shortcuts can delete a selected 3D object.
        keys.clear_just_pressed(KeyCode::Delete);
        if deletion.pending.is_none() && deletion.restoring.is_none() {
            let selected = if context.phase == Phase::Menu {
                deletion.menu_target.clone()
            } else {
                focused.or_else(|| deletion.selection.clone())
            };
            if let Some(selected) = selected {
                deletion.menu_target = Some(selected);
                actions.write(Action::BrowserDeleteFile);
            }
        }
    }
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    if inside
        && command
        && !keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight])
        && keys.just_pressed(KeyCode::KeyZ)
        && !deletion.history.is_empty()
    {
        keys.clear_just_pressed(KeyCode::KeyZ);
        actions.write(Action::BrowserRestoreFile);
    }
}

fn menu(
    deletion: Res<Deletion>,
    context: Res<BrowserContext>,
    menu: Res<Menu>,
    mut nodes: Query<&mut Node>,
) {
    if let Ok(mut node) = nodes.get_mut(menu.remove) {
        let shown = context.file_menu()
            && deletion.menu_target.is_some()
            && deletion.pending.is_none()
            && deletion.restoring.is_none();
        node.display = if shown { Display::Flex } else { Display::None };
    }
}

fn is_open(editor: &Editor, path: &std::path::Path) -> bool {
    editor.narrative.tabs.iter().any(|tab| tab.path == path)
        || editor.image_tabs.iter().any(|tab| tab.path == path)
        || editor.model_tabs.iter().any(|&index| {
            editor
                .project
                .catalog
                .get(index)
                .is_some_and(|model| std::path::Path::new(&model.asset) == path)
        })
        || editor.open_tabs.iter().any(|&room| {
            editor.project.scenes.get(room).is_some_and(|scene| {
                std::path::Path::new(&scene.file) == path
                    || std::path::Path::new(&scene.asset) == path
            })
        })
}

fn captured(
    event: On<ScreenshotCaptured>,
    tokens: Query<&CaptureToken>,
    mut deletion: ResMut<Deletion>,
) {
    let Ok(token) = tokens.get(event.entity) else {
        return;
    };
    let Some(pending) = deletion
        .pending
        .as_mut()
        .filter(|pending| pending.id == token.0 && pending.effect.is_none())
    else {
        return;
    };
    let Ok(image) = event.image.clone().try_into_dynamic() else {
        return;
    };
    let min = pending.rect.min.max(Vec2::ZERO).as_uvec2();
    let max = pending
        .rect
        .max
        .min(Vec2::new(image.width() as f32, image.height() as f32))
        .as_uvec2();
    if max.x <= min.x || max.y <= min.y {
        return;
    }
    pending.rect = Rect::from_corners(min.as_vec2(), max.as_vec2());
    let image = image.crop_imm(min.x, min.y, max.x - min.x, max.y - min.y);
    pending.snapshot = Some(Image::from_dynamic(
        image,
        true,
        RenderAssetUsages::default(),
    ));
}

#[derive(bevy::ecs::system::SystemParam)]
struct RenderAssets<'w> {
    images: ResMut<'w, Assets<Image>>,
    materials: ResMut<'w, Assets<BlackHoleMaterial>>,
    previews: ResMut<'w, Thumbnails>,
}

fn rescan(
    editor: &mut Editor,
    browser: &mut Browser,
    ui: &mut drawer::BrowserUi,
    assets: &mut RenderAssets,
) {
    browser.refresh(&mut editor.project);
    ui.select(None);
    assets.previews.extend(editor.project.catalog.len());
}

fn effect_layout(geometry: &Geometry, panel_entity: Entity, rect: Rect) -> Option<Node> {
    let (_, panel, transform, _) = geometry.get(panel_entity).ok()?;
    let position = transform.affine().inverse().transform_point2(rect.min) + panel.size() * 0.5
        - Vec2::splat(MARGIN);
    let scale = panel.inverse_scale_factor();
    Some(Node {
        position_type: PositionType::Absolute,
        left: px(position.x * scale),
        top: px(position.y * scale),
        width: px((rect.width() + MARGIN * 2.0) * scale),
        height: px((rect.height() + MARGIN * 2.0) * scale),
        ..default()
    })
}

fn spawn_effect(
    commands: &mut Commands,
    panel: Entity,
    material: Handle<BlackHoleMaterial>,
    node: Node,
    label: &str,
) -> Entity {
    commands
        .spawn((
            MaterialNode(material),
            Pickable::IGNORE,
            GlobalZIndex(65),
            AccessibleLabel(label.into()),
            node,
            ChildOf(panel),
        ))
        .id()
}

// Run after the browser rebuilds its cards, hiding the destination before its
// first rendered frame. Layout becomes available on the following update.
fn restore_effect(
    mut commands: Commands,
    mut deletion: ResMut<Deletion>,
    editor: Res<Editor>,
    browser: Res<Browser>,
    ui: Res<drawer::BrowserUi>,
    time: Res<Time>,
    geometry: Geometry,
    cards: Query<(Entity, &drawer::FileCard)>,
    mut nodes: Query<&mut Node>,
    mut materials: ResMut<Assets<BlackHoleMaterial>>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let Some(mut restoring) = deletion.restoring.take() else {
        return;
    };
    if !editor.drawer
        || editor.pending.is_some()
        || editor.hdri_library_open
        || browser.revision != restoring.revision
    {
        restoring.finish(&mut commands);
        return;
    }
    let card = restoring.card.or_else(|| {
        cards.iter().find_map(|(entity, card)| {
            target(entity, card, &browser, &editor)
                .filter(|target| target.path == restoring.path)
                .map(|target| target.card)
        })
    });
    let Some(card) = card else {
        restoring.finish(&mut commands);
        return;
    };
    restoring.card = Some(card);
    commands.entity(card).try_insert(Visibility::Hidden);
    redraw.write(RequestRedraw);
    let Some(rect) = bounds(&geometry, card) else {
        deletion.restoring = Some(restoring);
        return;
    };
    let Some(node) = effect_layout(&geometry, ui.panel, rect) else {
        restoring.finish(&mut commands);
        return;
    };
    if let Some(effect) = restoring.effect {
        restoring.age += time.delta_secs().min(0.05);
        if let Ok(mut current) = nodes.get_mut(effect) {
            current.set_if_neq(node);
        }
    } else {
        restoring.effect = Some(spawn_effect(
            &mut commands,
            ui.panel,
            restoring.material.clone(),
            node,
            "Чёрная дыра возвращает файл",
        ));
    }
    if let Some(mut material) = materials.get_mut(&restoring.material) {
        material.state.x = 1.0 - (restoring.age / DURATION).min(1.0);
        material.state.y = rect.width();
        material.state.z = rect.height();
    } else {
        restoring.finish(&mut commands);
        return;
    }
    if restoring.age >= DURATION {
        restoring.finish(&mut commands);
    } else {
        deletion.restoring = Some(restoring);
    }
}

fn update(
    mut commands: Commands,
    mut actions: MessageReader<Action>,
    mut deletion: ResMut<Deletion>,
    mut context: ResMut<BrowserContext>,
    mut editor: ResMut<Editor>,
    mut browser: ResMut<Browser>,
    mut ui: ResMut<drawer::BrowserUi>,
    time: Res<Time>,
    geometry: Geometry,
    cameras: Query<&RenderTarget, With<IsDefaultUiCamera>>,
    mut assets: RenderAssets,
    mut redraw: MessageWriter<RequestRedraw>,
    mut focus: ResMut<InputFocus>,
    mut blocked: MessageWriter<blocked::Attempt>,
) {
    for action in actions.read() {
        match action {
            Action::BrowserDeleteFile
                if editor.drawer
                    && editor.pending.is_none()
                    && !editor.hdri_library_open
                    && deletion.pending.is_none()
                    && deletion.restoring.is_none() =>
            {
                let Some(target) = deletion.menu_target.clone() else {
                    continue;
                };
                if target.revision != browser.scan_revision || !geometry.contains(target.card) {
                    continue;
                }
                if is_open(&editor, &target.path) {
                    editor.status = format!(
                        "Нельзя удалить «{}»: сначала закройте вкладку файла",
                        target
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    );
                    blocked.write(blocked::Attempt(target));
                    context.phase = Phase::Closed;
                    editor.browser_popup = false;
                    continue;
                }
                deletion.next_id = deletion.next_id.wrapping_add(1);
                deletion.pending = Some(Pending {
                    id: deletion.next_id,
                    target,
                    wait: 2,
                    requested: false,
                    age: 0.0,
                    rect: Rect::default(),
                    snapshot: None,
                    effect: None,
                });
                context.phase = Phase::Closed;
                context.previous_focus = None;
                editor.browser_popup = false;
                focus.clear();
            }
            Action::BrowserRestoreFile if editor.pending.is_none() && !editor.hdri_library_open => {
                let Some(last) = deletion.history.last() else {
                    continue;
                };
                match trash::restore(&editor.project.asset_root, &last.file) {
                    Ok(()) => {
                        let restored = deletion.history.pop().unwrap();
                        let effect = deletion.pending.take().and_then(|pending| {
                            let (entity, material) = pending.effect?;
                            if material == restored.material {
                                commands
                                    .entity(entity)
                                    .insert(AccessibleLabel("Чёрная дыра возвращает файл".into()));
                                Some(entity)
                            } else {
                                commands.entity(entity).try_despawn();
                                None
                            }
                        });
                        if let Some(restoring) = deletion.restoring.take() {
                            restoring.finish(&mut commands);
                        }
                        rescan(&mut editor, &mut browser, &mut ui, &mut assets);
                        if let Some(parent) = restored.file.original.parent()
                            && let Some(folder) = browser
                                .folders
                                .iter()
                                .position(|folder| folder.path == parent)
                        {
                            browser.navigate(folder);
                        }
                        context.focus_after = Some(restored.file.original.clone());
                        editor.status =
                            format!("Восстановлен файл · {}", restored.file.original.display());
                        let age = assets
                            .materials
                            .get(&restored.material)
                            .map_or(0.0, |material| {
                                (1.0 - material.state.x.clamp(0.0, 1.0)) * DURATION
                            });
                        deletion.restoring = Some(Restoration {
                            path: restored.file.original,
                            revision: browser.revision,
                            material: restored.material,
                            age,
                            card: None,
                            effect,
                        });
                    }
                    Err(error) => editor.status = format!("Не удалось восстановить файл · {error}"),
                }
                context.phase = Phase::Closed;
                editor.browser_popup = false;
            }
            _ => {}
        }
    }
    let Some(mut pending) = deletion.pending.take() else {
        return;
    };
    let delta = time.delta_secs().min(0.05);
    redraw.write(RequestRedraw);
    if let Some((entity, handle)) = &pending.effect {
        pending.age += delta;
        if let Some(mut material) = assets.materials.get_mut(handle) {
            material.state.x = (pending.age / DURATION).min(1.0);
        }
        if pending.age >= DURATION {
            commands.entity(*entity).try_despawn();
            rescan(&mut editor, &mut browser, &mut ui, &mut assets);
            deletion.selection = None;
            deletion.menu_target = None;
            return;
        }
    } else if pending.target.revision != browser.scan_revision
        || !geometry.contains(pending.target.card)
    {
        editor.status = "Удаление отменено: содержимое папки изменилось".into();
        return;
    } else if pending.wait > 0 {
        pending.wait -= 1;
    } else if !pending.requested {
        let Some(rect) = bounds(&geometry, pending.target.card) else {
            return;
        };
        let Ok(render_target) = cameras.single() else {
            return;
        };
        pending.rect = rect;
        commands
            .spawn((Screenshot(render_target.clone()), CaptureToken(pending.id)))
            .observe(captured);
        pending.requested = true;
        pending.age = 0.0;
    } else {
        pending.age += delta;
        if pending.snapshot.is_some() || pending.age > 1.0 {
            let snapshot = pending.snapshot.take().unwrap_or_else(|| {
                Image::new_fill(
                    Extent3d {
                        width: 2,
                        height: 2,
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    &[24, 24, 29, 255],
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::default(),
                )
            });
            let trashed = match trash::remove(&editor.project.asset_root, &pending.target.path) {
                Ok(trashed) => {
                    editor.status = format!(
                        "Удалён файл · {} · Ctrl+Z — восстановить",
                        pending.target.path.display()
                    );
                    trashed
                }
                Err(error) => {
                    editor.status = format!("Не удалось удалить файл · {error}");
                    return;
                }
            };
            let size = pending.rect.size();
            let material = assets.materials.add(BlackHoleMaterial {
                state: Vec4::new(0.0, size.x, size.y, pending.id as f32),
                snapshot: assets.images.add(snapshot),
            });
            deletion.history.push(DeletedFile {
                file: trashed,
                material: material.clone(),
            });
            let Some(node) = effect_layout(&geometry, ui.panel, pending.rect) else {
                rescan(&mut editor, &mut browser, &mut ui, &mut assets);
                return;
            };
            let entity = spawn_effect(
                &mut commands,
                ui.panel,
                material.clone(),
                node,
                "Чёрная дыра поглощает файл",
            );
            commands
                .entity(pending.target.card)
                .try_insert(Visibility::Hidden)
                .try_remove::<(Control, drawer::FileCard, drawer::DropFolder)>();
            pending.effect = Some((entity, material));
            pending.age = 0.0;
        }
    }
    deletion.pending = Some(pending);
}
