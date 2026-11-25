//! Portable copies of scene environments and a shared inspector picker.
use super::{
    model_viewer::shading::{Mode, Shading},
    *,
};
use bevy_scene_format::{Revision, atomic_write, read_limited};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io, path::Path};

const DIRECTORY: &str = "environments";
const PREFERENCES: &str = ".bevy-editor/model-environment.json";
const MAX_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preset {
    version: u32,
    name: String,
    lighting: crate::SceneLighting,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Source {
    #[default]
    Preview,
    Scene,
    Preset(String),
}

#[derive(Resource, Default)]
pub(in crate::editor) struct Library {
    entries: Vec<(String, Preset)>,
    revision: u64,
    model_source: Source,
    previous_source: Option<Source>,
    model_environment: Option<(crate::SceneLighting, bool)>,
    scene_source: Option<SceneSource>,
    open: Option<List>,
}

struct SceneSource {
    room: usize,
    preset: String,
    applied: crate::SceneLighting,
    previous: Option<crate::SceneLighting>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum List {
    Scene,
    Model,
    Hdri,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum Command {
    Toggle(List),
    Save,
    KeepScene,
    ApplyScene(usize, u64),
    UsePreview,
    UseScene,
    ApplyModel(usize, u64),
    Compare,
}

fn key(scene: &crate::project::SceneDefinition) -> String {
    let mut hash = Sha256::new();
    if let Some(uid) = &scene.uid {
        hash.update(b"uid:");
        hash.update(uid);
    } else {
        hash.update(b"path:");
        hash.update(&scene.file);
    }
    format!("scene-{:x}.env.json", hash.finalize())
}

fn write(root: &Path, id: &str, name: &str, lighting: &crate::SceneLighting) -> io::Result<()> {
    lighting.validate()?;
    let path = root.join(DIRECTORY).join(id);
    let bytes = serde_json::to_vec_pretty(&Preset {
        version: 1,
        name: name.chars().filter(|c| !c.is_control()).take(160).collect(),
        lighting: lighting.clone(),
    })?;
    if read_limited(&path, MAX_BYTES).is_ok_and(|old| old == bytes) {
        return Ok(());
    }
    atomic_write(&path, &bytes, Revision::read(&path)?)?;
    Ok(())
}

pub(in crate::editor) fn save_scene(editor: &Editor) -> io::Result<()> {
    let scene = editor.definition();
    write(
        &editor.root,
        &key(scene),
        &scene.label,
        editor.environment(),
    )
}

impl Library {
    fn scan(&mut self, root: &Path) {
        let mut entries = Vec::new();
        if let Ok(files) = fs::read_dir(root.join(DIRECTORY)) {
            for file in files.filter_map(Result::ok) {
                let name = file.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".env.json")
                    || !file.file_type().is_ok_and(|kind| kind.is_file())
                {
                    continue;
                }
                let preset = read_limited(&file.path(), MAX_BYTES)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Preset>(&bytes).ok());
                if let Some(mut preset) = preset.filter(|p| {
                    p.version == 1
                        && p.name.chars().count() <= 160
                        && !p.name.chars().any(char::is_control)
                        && p.lighting.validate().is_ok()
                }) {
                    if preset.name.trim().is_empty() {
                        preset.name = "Без названия".into();
                    }
                    entries.push((name, preset));
                } else {
                    warn!("Не удалось прочитать окружение: {}", file.path().display());
                }
            }
        }
        entries.sort_by(|a, b| {
            a.1.name
                .to_lowercase()
                .cmp(&b.1.name.to_lowercase())
                .then(a.0.cmp(&b.0))
        });
        if self.entries != entries {
            self.entries = entries;
            self.revision = self.revision.wrapping_add(1);
        }
        self.resolve_model(root);
    }

    fn get(&self, id: &str) -> Option<&Preset> {
        self.entries
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, preset)| preset)
    }

    fn entry(&self, index: usize, revision: u64) -> Option<&(String, Preset)> {
        (self.revision == revision)
            .then(|| self.entries.get(index))
            .flatten()
    }

    pub(in crate::editor) fn model_lighting(&self) -> Option<&crate::SceneLighting> {
        self.model_environment
            .as_ref()
            .map(|(lighting, _)| lighting)
    }

    fn resolve_model(&mut self, root: &Path) {
        self.model_environment = match &self.model_source {
            Source::Preset(id) => self.get(id).map(|preset| resolved(&preset.lighting, root)),
            _ => None,
        };
    }

    fn active_scene_source(&self, editor: &Editor) -> Option<&SceneSource> {
        self.scene_source
            .as_ref()
            .filter(|source| source.room == editor.room && &source.applied == editor.environment())
    }

    fn scene_saved(&self, editor: &Editor) -> bool {
        editor.has_scene()
            && self.get(&key(editor.definition())).is_some_and(|preset| {
                preset.name == editor.definition().label && &preset.lighting == editor.environment()
            })
    }

    pub(super) fn enabled(&self, editor: &Editor, command: Command) -> bool {
        match command {
            Command::Save => !self.scene_saved(editor),
            Command::Compare => self.previous_source.is_some(),
            _ => true,
        }
    }

    fn model_label(&self, source: &Source, editor: &Editor) -> String {
        match source {
            Source::Scene if editor.has_scene() => {
                format!("Сцена · {}", editor.definition().label)
            }
            Source::Preset(id) => self.get(id).map_or_else(
                || "Настройки просмотра".into(),
                |preset| preset.name.clone(),
            ),
            _ => "Настройки просмотра".into(),
        }
    }

    fn set_model_source(
        &mut self,
        source: Source,
        root: &Path,
        shading: &mut Shading,
    ) -> io::Result<()> {
        let changed = self.model_source != source;
        if changed {
            self.previous_source = Some(std::mem::replace(&mut self.model_source, source));
        }
        self.resolve_model(root);
        shading.mode = Mode::Rendered;
        shading.preview_lighting = self.model_source != Source::Scene;
        if changed {
            let path = root.join(PREFERENCES);
            atomic_write(
                &path,
                &serde_json::to_vec(&self.model_source)?,
                Revision::read(&path)?,
            )?;
        }
        Ok(())
    }
}

fn resolved(lighting: &crate::SceneLighting, root: &Path) -> (crate::SceneLighting, bool) {
    let mut lighting = lighting.clone();
    let mut missing = false;
    if let Some(asset) = &mut lighting.hdri_asset {
        if let Ok(index) = crate::asset_registry::index(root) {
            asset.asset = index.resolve(&asset.asset, asset.uid.as_deref());
        }
        if !root.join(&asset.asset).is_file() {
            lighting.hdri_asset = None;
            missing = true;
        }
    }
    (lighting, missing)
}

pub(in crate::editor) fn load(
    editor: Res<Editor>,
    mut library: ResMut<Library>,
    mut shading: ResMut<Shading>,
) {
    library.scan(&editor.root);
    if let Some(source) = read_limited(&editor.root.join(PREFERENCES), MAX_BYTES)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Source>(&bytes).ok())
        && !matches!(&source, Source::Preset(id) if library.get(id).is_none())
    {
        if source != Source::Preview {
            shading.mode = Mode::Rendered;
            shading.preview_lighting = source != Source::Scene;
        }
        library.model_source = source;
        library.resolve_model(&editor.root);
    }
}

pub(in crate::editor) fn actions(
    mut events: MessageReader<Action>,
    mut editor: ResMut<Editor>,
    mut library: ResMut<Library>,
    mut shading: ResMut<Shading>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    pickers: Query<&Picker>,
    geometry: Query<(
        &ComputedNode,
        &UiGlobalTransform,
        Option<&bevy::ui::CalculatedClip>,
    )>,
) {
    if let Some(open) = library.open {
        let outside = mouse.any_just_pressed([MouseButton::Left, MouseButton::Right])
            && window.cursor_position().is_some_and(|cursor| {
                !pickers.iter().any(|ui| {
                    ui.list == open
                        && [ui.bar, ui.options].into_iter().any(|entity| {
                            geometry.get(entity).is_ok_and(|(node, transform, clip)| {
                                let point = cursor / node.inverse_scale_factor();
                                !node.is_empty()
                                    && node.contains_point(*transform, point)
                                    && clip.is_none_or(|clip| clip.contains_point(point))
                            })
                        })
                })
            });
        if outside
            || keys.just_pressed(KeyCode::Escape)
            || !available(&editor, Command::Toggle(open))
        {
            library.open = None;
        }
    }
    for &event in events.read() {
        match event {
            Action::Save | Action::SaveAndContinue | Action::Play => {
                library.scan(&editor.root);
            }
            Action::Lighting(super::super::lighting::LightingAction::SetEnvironment(_))
            | Action::UseDownloadedHdri(_) => {
                if library.open == Some(List::Hdri) {
                    library.open = None;
                }
            }
            Action::Open(_)
            | Action::ViewModel(_)
            | Action::OpenImageTab(_)
            | Action::ViewImage(_) => {
                library.open = None;
            }
            Action::EnvironmentLibrary(command)
                if available(&editor, command) && library.enabled(&editor, command) =>
            {
                match command {
                    Command::Toggle(which) => {
                        if which != List::Hdri && library.open != Some(which) {
                            library.scan(&editor.root);
                        }
                        library.open = (library.open != Some(which)).then_some(which);
                    }
                    Command::Save => {
                        editor.status = match save_scene(&editor) {
                            Ok(()) => {
                                library.scan(&editor.root);
                                format!("Окружение сохранено · {}", editor.definition().label)
                            }
                            Err(error) => format!("Не удалось сохранить окружение · {error}"),
                        };
                    }
                    Command::KeepScene => {
                        if let Some(previous) = library
                            .active_scene_source(&editor)
                            .map(|source| source.previous.clone())
                        {
                            let before = editor.edits.clone();
                            editor.edits.lighting = previous;
                            editor.finish_edit(before);
                            library.scene_source = None;
                        }
                        library.open = None;
                    }
                    Command::ApplyScene(index, revision) => {
                        if let Some((id, preset)) = library.entry(index, revision).cloned() {
                            let before = editor.edits.clone();
                            let previous = library.active_scene_source(&editor).map_or_else(
                                || before.lighting.clone(),
                                |source| source.previous.clone(),
                            );
                            let (settings, missing) = resolved(&preset.lighting, &editor.root);
                            editor.edits.lighting = Some(settings.clone());
                            editor.finish_edit(before);
                            library.scene_source = Some(SceneSource {
                                room: editor.room,
                                preset: id,
                                applied: settings,
                                previous,
                            });
                            library.open = None;
                            editor.status = if missing {
                                format!("{} · HDRI не найден, применено без панорамы", preset.name)
                            } else {
                                format!("Окружение · {}", preset.name)
                            };
                        }
                    }
                    other => {
                        let source = match other {
                            Command::UsePreview => Some(Source::Preview),
                            Command::UseScene => Some(Source::Scene),
                            Command::ApplyModel(index, revision) => library
                                .entry(index, revision)
                                .map(|(id, _)| Source::Preset(id.clone())),
                            Command::Compare => library.previous_source.clone(),
                            _ => None,
                        };
                        if let Some(source) = source {
                            library.open = None;
                            if let Err(error) =
                                library.set_model_source(source, &editor.root, &mut shading)
                            {
                                editor.status =
                                    format!("Не удалось сохранить выбор окружения · {error}");
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

pub(super) fn available(editor: &Editor, action: Command) -> bool {
    if editor.pending.is_some()
        || editor.viewing_image.is_some()
        || editor.hdri_library_open
        || editor.generation.active
        || editor.narrative.active.is_some()
        || editor.browser_popup
        || editor.scene_object_popup
    {
        return false;
    }
    match action {
        Command::Toggle(List::Model)
        | Command::UsePreview
        | Command::ApplyModel(..)
        | Command::Compare => editor.viewing_model.is_some(),
        Command::UseScene => editor.viewing_model.is_some() && editor.has_scene() && editor.ready,
        _ => {
            editor.viewing_model.is_none()
                && editor.has_scene()
                && editor.ready
                && editor.selection == Some(Selection::Environment)
                && !editor.multiple_selected()
        }
    }
}

#[derive(Component)]
pub(in crate::editor) struct Options;

#[derive(Component)]
pub(in crate::editor) struct Picker {
    list: List,
    bar: Entity,
    caption: Entity,
    options: Entity,
    list_key: Option<(u64, Option<String>, Option<String>)>,
    save_caption: Option<Entity>,
    compare: Option<Entity>,
    warning: Option<Entity>,
}

fn act(action: Command) -> Action {
    Action::EnvironmentLibrary(action)
}

pub(super) fn picker(
    commands: &mut Commands,
    parent: Entity,
    overlay: Entity,
    font: &Handle<Font>,
    list: List,
) -> Entity {
    let root = node(commands, parent, column(4.0));
    let bar = node(commands, root, row(4.0));
    let label = match list {
        List::Scene => "Своё окружение",
        List::Model => "Настройки просмотра",
        List::Hdri => "Без панорамы",
    };
    let toggle = control(
        commands,
        bar,
        act(Command::Toggle(list)),
        Look::Regular,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            height: px(30),
            padding: UiRect::horizontal(px(8)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            overflow: Overflow::clip(),
            ..row(4.0)
        },
    );
    commands.entity(toggle).insert(AccessibleLabel(
        match list {
            List::Scene => "Окружения проекта",
            List::Model => "Выбрать окружение модели",
            List::Hdri => "Панорама HDRI",
        }
        .into(),
    ));
    let caption = text(commands, toggle, font, label, 12.0, TEXT);
    commands.entity(caption).insert((
        TextLayout::no_wrap(),
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            ..default()
        },
    ));
    icons::themed(commands, toggle, icons::Glyph::ChevronDown, 12.0);
    let (save_caption, compare) = match list {
        List::Scene => {
            let save = control(
                commands,
                bar,
                act(Command::Save),
                Look::Regular,
                Node {
                    width: px(131),
                    height: px(30),
                    flex_shrink: 0.0,
                    justify_content: JustifyContent::Center,
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(5)),
                    ..row(0.0)
                },
            );
            commands.entity(save).insert((
                AccessibleLabel("Сохранить окружение".into()),
                super::lighting_ui::Help("Окружение также сохраняется вместе со сценой".into()),
            ));
            let saved = text(commands, save, font, "Сохранить окружение", 11.0, TEXT);
            commands.entity(saved).insert(TextLayout::no_wrap());
            (Some(saved), None)
        }
        List::Model => {
            let swap = inspector::icon_button(
                commands,
                bar,
                icons::Glyph::Swap,
                "Переключить два последних окружения",
                act(Command::Compare),
            );
            (None, Some(swap))
        }
        List::Hdri => (None, None),
    };
    let warning = (list == List::Model).then(|| {
        let label = text(
            commands,
            root,
            font,
            "Панорама HDRI не найдена",
            11.0,
            MUTED,
        );
        commands
            .entity(label)
            .entry::<Node>()
            .and_modify(|mut n| n.display = Display::None);
        label
    });
    let options = surface(
        commands,
        overlay,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            max_height: px(190),
            overflow: Overflow::scroll_y(),
            padding: UiRect::all(px(4)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            ..column(2.0)
        },
        PANEL,
    );
    commands.entity(options).insert((
        Scroller,
        Options,
        ScrollPosition::default(),
        BorderColor::all(EDGE),
        ZIndex(85),
    ));
    commands.entity(root).insert(Picker {
        list,
        bar,
        caption,
        options,
        list_key: None,
        save_caption,
        compare,
        warning,
    });
    options
}

fn option(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    action: Command,
) -> Entity {
    let button = button(commands, parent, font, label, act(action), Look::Outline);
    commands
        .entity(button)
        .insert(AccessibleLabel(label.into()));
    button
}

pub(in crate::editor) fn refresh(
    mut commands: Commands,
    library: Res<Library>,
    editor: Res<Editor>,
    interface: Res<Interface>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut pickers: Query<&mut Picker>,
    geometry: Query<(
        &ComputedNode,
        &UiGlobalTransform,
        Option<&bevy::ui::CalculatedClip>,
    )>,
    mut nodes: Query<&mut Node>,
    mut texts: Query<&mut Text>,
    mut hints: Query<&mut super::lighting_ui::Help>,
) {
    for mut picker in &mut pickers {
        let anchor = geometry
            .get(picker.bar)
            .ok()
            .and_then(|(node, transform, clip)| {
                let center = transform.affine().translation;
                (!node.is_empty() && clip.is_none_or(|clip| clip.contains_point(center))).then(
                    || {
                        let half = node.size() * (node.inverse_scale_factor() * 0.5);
                        let center = center * node.inverse_scale_factor();
                        Rect::from_corners(center - half, center + half)
                    },
                )
            });
        let open = library.open == Some(picker.list) && anchor.is_some();
        if let Ok(mut node) = nodes.get_mut(picker.options) {
            let display = if open { Display::Flex } else { Display::None };
            if node.display != display {
                node.display = display;
            }
            if let Some(anchor) = anchor.filter(|_| open) {
                let below = (window.height() - FOOT - anchor.max.y - 8.0).max(0.0);
                let above = (anchor.min.y - TOP - 8.0).max(0.0);
                let open_below = below >= 190.0 || below >= above;
                node.left = px(anchor.min.x);
                node.width = px(anchor.width());
                node.top = if open_below {
                    px(anchor.max.y + 4.0)
                } else {
                    Val::Auto
                };
                node.bottom = if open_below {
                    Val::Auto
                } else {
                    px(window.height() - anchor.min.y + 4.0)
                };
                node.max_height = px(if open_below { below } else { above }.min(190.0));
            }
        }
        if let Some(entity) = picker.warning
            && let Ok(mut node) = nodes.get_mut(entity)
        {
            let missing = library
                .model_environment
                .as_ref()
                .is_some_and(|(_, missing)| *missing);
            let display = if missing {
                Display::Flex
            } else {
                Display::None
            };
            if node.display != display {
                node.display = display;
            }
        }
        if let Some(entity) = picker.save_caption
            && let Ok(mut label) = texts.get_mut(entity)
        {
            let caption = if library.scene_saved(&editor) {
                "Сохранено"
            } else {
                "Сохранить окружение"
            };
            if label.0 != caption {
                label.0 = caption.into();
            }
        }
        if let Some(entity) = picker.compare {
            let hint = library.previous_source.as_ref().map_or_else(
                || "Выберите два окружения для сравнения".to_owned(),
                |source| format!("Вернуть «{}»", library.model_label(source, &editor)),
            );
            if let Ok(mut help) = hints.get_mut(entity) {
                if help.0 != hint {
                    help.0 = hint.into();
                }
            } else {
                commands
                    .entity(entity)
                    .insert(super::lighting_ui::Help(hint.into()));
            }
        }
        let caption = match picker.list {
            List::Scene => library
                .active_scene_source(&editor)
                .and_then(|source| library.get(&source.preset))
                .map_or_else(
                    || "Своё окружение".into(),
                    |p| format!("Копия · {}", p.name),
                ),
            List::Model => library.model_label(&library.model_source, &editor),
            List::Hdri => editor
                .environment()
                .hdri_asset
                .as_ref()
                .map(|a| a.name.as_str())
                .or_else(|| {
                    editor
                        .environment()
                        .hdri
                        .map(crate::EnvironmentPreset::label)
                })
                .unwrap_or("Без панорамы")
                .to_owned(),
        };
        if let Ok(mut label) = texts.get_mut(picker.caption)
            && label.0 != caption
        {
            label.0 = caption;
        }
        if !open || picker.list == List::Hdri {
            continue;
        }
        let scene_key =
            (picker.list == List::Scene && editor.has_scene()).then(|| key(editor.definition()));
        let current_room = editor
            .has_scene()
            .then(|| editor.definition().label.clone());
        let list_key = (library.revision, current_room, scene_key);
        if picker.list_key.as_ref() == Some(&list_key) {
            continue;
        }
        let target = picker.options;
        commands.entity(target).despawn_related::<Children>();
        commands.entity(target).insert(ScrollPosition::default());
        let font = &interface.font;
        match picker.list {
            List::Scene => {
                option(
                    &mut commands,
                    target,
                    font,
                    "Своё окружение",
                    Command::KeepScene,
                );
            }
            List::Model => {
                option(
                    &mut commands,
                    target,
                    font,
                    "Настройки просмотра",
                    Command::UsePreview,
                );
                if let Some(room) = &list_key.1 {
                    option(
                        &mut commands,
                        target,
                        font,
                        &format!("Сцена · {room}"),
                        Command::UseScene,
                    );
                }
            }
            List::Hdri => unreachable!(),
        }
        if picker.list == List::Scene
            && library
                .entries
                .iter()
                .any(|(id, _)| list_key.2.as_ref() != Some(id))
        {
            text(
                &mut commands,
                target,
                font,
                "Скопировать из сцены",
                11.0,
                MUTED,
            );
        }
        for (index, (id, preset)) in library.entries.iter().enumerate() {
            if list_key.2.as_ref() == Some(id) {
                continue;
            }
            let command = if picker.list == List::Scene {
                Command::ApplyScene(index, library.revision)
            } else {
                Command::ApplyModel(index, library.revision)
            };
            let button = option(&mut commands, target, font, &preset.name, command);
            commands
                .entity(button)
                .insert(AccessibleLabel(format!("Окружение: {}", preset.name)));
        }
        if library
            .entries
            .iter()
            .all(|(id, _)| list_key.2.as_ref() == Some(id))
        {
            text(
                &mut commands,
                target,
                font,
                "Сохраните окружение в сцене",
                11.0,
                MUTED,
            );
        }
        picker.list_key = Some(list_key);
    }
}
