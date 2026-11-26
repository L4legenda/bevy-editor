use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use bevy::{
    anti_alias::taa::TemporalAntiAliasing,
    asset::AssetEvent,
    camera::{
        ClearColorConfig, Exposure, Hdr, RenderTarget, primitives::Aabb, visibility::RenderLayers,
    },
    light::{CascadeShadowConfigBuilder, EnvironmentMapLight},
    pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceTransmission},
    prelude::*,
    render::{
        MainWorld,
        render_asset::RenderAssets,
        render_resource::{PipelineCache, TextureFormat},
        texture::GpuImage,
        view::Tonemapping,
    },
};

use super::{
    Editor,
    browser::{Browser, Entry},
    load_error,
    model_source::ModelSource,
    viewport,
};

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "asset_loading_tests.rs"]
mod asset_loading_tests;
#[path = "thumbnail_framing.rs"]
pub(super) mod framing;
#[cfg(test)]
#[path = "thumbnail_lifecycle_tests.rs"]
mod lifecycle_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "thumbnail_performance.rs"]
mod performance;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "thumbnail_render_tests.rs"]
mod render_tests;
#[path = "preview_scene.rs"]
pub(super) mod scene;

// Render above the card's logical size so edges stay crisp on Retina displays.
const WIDTH: u32 = 384;
const HEIGHT: u32 = 240;
const LAYER: usize = 2;
const SETTLE: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Target {
    Model(usize),
    Scene(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PreviewState {
    Loading,
    Ready,
    Failed,
}

impl Target {
    fn from_entry(entry: Entry) -> Option<Self> {
        match entry {
            Entry::Model(index) => Some(Self::Model(index)),
            Entry::Scene { room, .. } => Some(Self::Scene(room)),
            _ => None,
        }
    }
}

/// Paths outlive the offscreen entities so a texture edit can refresh a cached card.
struct ThumbnailFiles {
    paths: HashSet<String>,
    source: String,
    direction: Option<[f32; 3]>,
    pending: Option<bevy::tasks::Task<HashSet<String>>>,
    changes: HashSet<String>,
}

impl ThumbnailFiles {
    fn changed(&mut self, changed: &HashSet<String>) -> bool {
        if let Some(task) = &mut self.pending {
            self.changes.extend(changed.iter().cloned());
            if let Some(paths) = bevy::tasks::block_on(bevy::tasks::poll_once(task)) {
                self.pending = None;
                self.paths.extend(paths);
                return !self.paths.is_disjoint(&std::mem::take(&mut self.changes));
            }
        }
        !self.paths.is_disjoint(changed)
    }
}

#[derive(Resource)]
pub(super) struct Thumbnails {
    pub(super) images: Vec<Handle<Image>>,
    pub(super) scene_images: Vec<Handle<Image>>,
    completed: HashSet<Target>,
    failed: HashSet<Target>,
    snapshots: Vec<Option<scene::SceneSnapshot>>,
    changed: Vec<Instant>,
    scan_revision: Option<u64>,
    browser_revision: Option<u64>,
    placeholder: Handle<Image>,
    recent: std::collections::VecDeque<Target>,
    pending_snapshots: HashMap<usize, scene::SnapshotRead>,
    warmup: Option<Target>,
    warmed_up: bool,
    view: Option<PreviewView>,
    light: Option<Entity>,
    model_environment: Option<crate::SceneLighting>,
    changed_assets: HashSet<String>,
    asset_versions: HashMap<String, (u64, std::time::SystemTime)>,
    files: HashMap<Target, ThumbnailFiles>,
    job: Option<Job>,
}

#[derive(Clone, Copy)]
struct PreviewView {
    entity: Entity,
    studio: bool,
}

struct Job {
    target: Target,
    asset: Option<ModelSource>,
    // Roots retain only this job's assets until its captured image is ready.
    root: Entity,
    instances: Vec<Entity>,
    models: Vec<Handle<WorldAsset>>,
    started: Instant,
    camera: Option<Entity>,
    stable_frames: u8,
}

impl Job {
    fn despawn(self, commands: &mut Commands, retained_camera: Option<Entity>) {
        if let Some(camera) = self.camera {
            if Some(camera) == retained_camera {
                commands
                    .entity(camera)
                    .entry::<Camera>()
                    .and_modify(|mut camera| {
                        camera.is_active = false;
                    });
            } else {
                commands.entity(camera).try_despawn();
            }
        }
        commands.entity(self.root).try_despawn();
    }
}

impl Thumbnails {
    pub(super) fn new(images: &mut Assets<Image>, models: usize, scenes: usize) -> Self {
        let placeholder = images.add(Image::transparent());
        Self {
            images: vec![placeholder.clone(); models],
            scene_images: vec![placeholder.clone(); scenes],
            completed: HashSet::new(),
            failed: HashSet::new(),
            snapshots: vec![None; scenes],
            changed: vec![Instant::now(); scenes],
            scan_revision: None,
            browser_revision: None,
            placeholder,
            recent: Default::default(),
            pending_snapshots: HashMap::new(),
            warmup: None,
            warmed_up: false,
            view: None,
            light: None,
            model_environment: None,
            changed_assets: HashSet::new(),
            asset_versions: HashMap::new(),
            files: default(),
            job: None,
        }
    }

    pub(super) fn extend(&mut self, count: usize) {
        self.images.resize(count, self.placeholder.clone());
    }

    fn allocate(&mut self, target: Target, visible: &[Entry], images: &mut Assets<Image>) {
        const CAPACITY: usize = 64;
        if self.image(target) == self.placeholder {
            let texture = target_texture(images);
            match target {
                Target::Model(i) => self.images[i] = texture,
                Target::Scene(i) => self.scene_images[i] = texture,
            }
        }
        self.recent.retain(|&t| t != target);
        self.recent.push_back(target);
        while self.recent.len() > CAPACITY {
            let Some(index) = self.recent.iter().position(|&t| {
                t != target && !visible.iter().any(|&e| Target::from_entry(e) == Some(t))
            }) else {
                break;
            };
            let evicted = self.recent.remove(index).unwrap();
            match evicted {
                Target::Model(i) => self.images[i] = self.placeholder.clone(),
                Target::Scene(i) => {
                    self.scene_images[i] = self.placeholder.clone();
                    self.snapshots[i] = None;
                }
            }
            self.files.remove(&evicted);
            self.completed.remove(&evicted);
            self.failed.remove(&evicted);
        }
    }

    pub(super) fn invalidate_asset(&mut self, path: &str) {
        self.changed_assets.insert(path.replace('\\', "/"));
    }

    fn remember_asset(&mut self, editor: &Editor, path: &str) {
        if let Some(version) = asset_version(editor, path) {
            self.asset_versions
                .entry(asset_key(editor, path))
                .or_insert(version);
        }
    }

    fn asset_modified(&mut self, editor: &Editor, path: &str) -> bool {
        let Some(version) = asset_version(editor, path) else {
            return true;
        };
        // Loading a GLTF root again also replaces its live Scene0 subasset.
        // That emits Modified even when the file itself has not changed.
        self.asset_versions
            .insert(asset_key(editor, path), version)
            .is_none_or(|previous| previous != version)
    }

    fn remember_files(&mut self, editor: &Editor, target: Target, asset: &str) {
        let mut models = Vec::new();
        if !asset.is_empty() {
            models.push(asset.to_owned());
        }
        let (source, direction) = match target {
            Target::Model(i) => (
                asset.to_owned(),
                Some(editor.project.catalog[i].preview_direction),
            ),
            Target::Scene(i) => {
                if let Some(snapshot) = &self.snapshots[i] {
                    models.extend(
                        snapshot
                            .edits
                            .models
                            .iter()
                            .map(|model| model.asset.clone()),
                    );
                }
                (editor.project.scenes[i].file.clone(), None)
            }
        };
        let mut paths: HashSet<_> = models.iter().map(|path| asset_key(editor, path)).collect();
        paths.insert(asset_key(editor, &source));
        for model in &models {
            self.remember_asset(editor, model);
        }
        let root = editor.root.clone();
        let pending = bevy::tasks::IoTaskPool::get().spawn(async move {
            let mut paths = HashSet::new();
            for model in models {
                let path = crate::asset_paths::resolve_asset_path(&root, &model).unwrap_or(model);
                if let Ok(inputs) =
                    super::file_moves::model_inputs(&root, std::path::Path::new(&path))
                {
                    for input in inputs {
                        let path = input.to_string_lossy().replace('\\', "/");
                        paths.insert(
                            crate::asset_paths::original_asset_path(&root, &path).unwrap_or(path),
                        );
                    }
                }
            }
            paths
        });
        self.files.insert(
            target,
            ThumbnailFiles {
                source,
                direction,
                paths,
                pending: Some(pending),
                changes: default(),
            },
        );
    }

    fn refresh_models(
        &mut self,
        editor: &Editor,
        environment: &crate::SceneLighting,
        commands: &mut Commands,
    ) {
        let lighting_changed = self.model_environment.as_ref() != Some(environment);
        self.model_environment = Some(environment.clone());
        let changed = std::mem::take(&mut self.changed_assets);
        let changed: HashSet<_> = changed.iter().map(|path| asset_key(editor, path)).collect();
        let mut invalid = HashSet::new();
        for (&target, files) in &mut self.files {
            if files.changed(&changed) || (lighting_changed && matches!(target, Target::Model(_))) {
                invalid.insert(target);
            }
        }
        for (i, scene) in editor.project.scenes.iter().enumerate() {
            if !changed.is_empty() && changed.contains(&asset_key(editor, &scene.file)) {
                invalid.insert(Target::Scene(i));
                self.pending_snapshots.remove(&i);
                if let Some(snapshot) = self.snapshots.get_mut(i) {
                    *snapshot = None;
                }
            }
        }
        if self
            .job
            .as_ref()
            .is_some_and(|job| invalid.contains(&job.target))
        {
            self.cancel(commands);
        }
        for target in invalid {
            self.completed.remove(&target);
            self.failed.remove(&target);
            if let Target::Scene(i) = target
                && let Some(stamp) = self.changed.get_mut(i)
            {
                *stamp = Instant::now();
            }
        }
    }

    pub(super) fn state(&self, entry: Entry) -> PreviewState {
        match Target::from_entry(entry) {
            Some(target) if self.failed.contains(&target) => PreviewState::Failed,
            Some(target) if !self.completed.contains(&target) => PreviewState::Loading,
            _ => PreviewState::Ready,
        }
    }

    fn image(&self, target: Target) -> Handle<Image> {
        match target {
            Target::Model(index) => self.images[index].clone(),
            Target::Scene(room) => self.scene_images[room].clone(),
        }
    }

    fn cancel(&mut self, commands: &mut Commands) {
        if let Some(job) = self.job.take() {
            self.stop(job, commands);
        }
    }

    fn stop(&mut self, job: Job, commands: &mut Commands) {
        if self.view.is_some_and(|view| !view.studio) {
            self.view = None;
        }
        job.despawn(commands, self.view.map(|view| view.entity));
        if let Some(light) = self.light {
            commands
                .entity(light)
                .insert(Visibility::Hidden)
                .entry::<DirectionalLight>()
                .and_modify(|mut light| {
                    light.illuminance = 0.0;
                    light.shadow_maps_enabled = false;
                });
        }
    }

    fn complete(&mut self, target: Target) {
        self.completed.insert(target);
        self.failed.remove(&target);
        if self.warmup == Some(target) {
            self.warmup = None;
            self.warmed_up = true;
        }
    }

    fn fail(&mut self, target: Target) {
        self.complete(target);
        self.failed.insert(target);
    }

    fn accept_snapshot(
        &mut self,
        editor: &Editor,
        room: usize,
        result: std::io::Result<scene::SceneSnapshot>,
    ) {
        match result {
            Ok(snapshot) if snapshot.matches_editor(editor) => {
                self.snapshots[room] = Some(snapshot)
            }
            Ok(_) => {} // The document was opened/edited while its disk read was running.
            Err(error) => {
                self.snapshots[room] = None;
                self.fail(Target::Scene(room));
                warn!(
                    "Scene thumbnail for {}: {error}",
                    editor.project.scenes[room].label
                );
            }
        }
    }

    fn refresh_scenes(&mut self, editor: &Editor, visible: &[Entry], commands: &mut Commands) {
        let finished: Vec<_> = self
            .pending_snapshots
            .iter_mut()
            .filter_map(|(&room, read)| read.poll().map(|result| (room, result)))
            .collect();
        for (room, result) in finished {
            self.pending_snapshots.remove(&room);
            if editor.preview_edits(room).is_none() {
                self.accept_snapshot(editor, room, result);
            }
        }
        // Track already-cached documents while hidden too: an edit/save/close
        // must not resurrect an older disk snapshot when returning to the folder.
        let rooms: HashSet<_> = visible
            .iter()
            .filter_map(|entry| match *entry {
                Entry::Scene { room, .. } => Some(room),
                _ => None,
            })
            .chain(self.warmup.and_then(|target| match target {
                Target::Scene(room) => Some(room),
                Target::Model(_) => None,
            }))
            .chain(self.pending_snapshots.keys().copied())
            .chain(
                self.snapshots
                    .iter()
                    .enumerate()
                    .filter_map(|(room, snapshot)| snapshot.as_ref().map(|_| room)),
            )
            .collect();
        for room in rooms {
            let current = self.snapshots[room].as_ref();
            // Failed snapshots retry on a filesystem refresh, rather than logging every frame.
            if current.is_none() && self.completed.contains(&Target::Scene(room)) {
                continue;
            }
            if current.is_some_and(|snapshot| snapshot.matches_editor(editor)) {
                continue;
            }
            if self.pending_snapshots.contains_key(&room) {
                if editor.preview_edits(room).is_none() {
                    continue;
                }
                self.pending_snapshots.remove(&room);
            }
            if self
                .job
                .as_ref()
                .is_some_and(|job| job.target == Target::Scene(room))
            {
                self.cancel(commands);
            }
            self.completed.remove(&Target::Scene(room));
            self.failed.remove(&Target::Scene(room));
            self.changed[room] = Instant::now();
            self.snapshots[room] = None;
            // Bound concurrent disk reads. Open documents need no worker or I/O.
            if editor.preview_edits(room).is_none() && self.pending_snapshots.len() >= 2 {
                continue;
            }
            match scene::SceneSnapshot::request(editor, room) {
                Ok(mut read) => match read.poll() {
                    Some(result) => self.accept_snapshot(editor, room, result),
                    None => {
                        self.pending_snapshots.insert(room, read);
                    }
                },
                Err(error) => self.accept_snapshot(editor, room, Err(error)),
            }
        }
    }
}

fn target_texture(images: &mut Assets<Image>) -> Handle<Image> {
    images.add(Image::new_target_texture(
        WIDTH,
        HEIGHT,
        TextureFormat::Rgba8Unorm,
        Some(TextureFormat::Rgba8UnormSrgb),
    ))
}

fn next_missing(
    entries: &[Entry],
    completed: &HashSet<Target>,
    ready: impl Fn(Target) -> bool,
) -> Option<Target> {
    entries
        .iter()
        .filter_map(|&entry| Target::from_entry(entry))
        .find(|target| !completed.contains(target) && ready(*target))
}

fn asset_key(editor: &Editor, path: &str) -> String {
    crate::asset_paths::original_asset_path(&editor.root, path)
        .unwrap_or_else(|_| path.replace('\\', "/"))
}

fn asset_version(editor: &Editor, path: &str) -> Option<(u64, std::time::SystemTime)> {
    let path = crate::asset_paths::resolve_asset_path(&editor.root, path).ok()?;
    let metadata = std::fs::metadata(editor.root.join(path)).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

/// A saved/reloaded model invalidates both its catalog card and dependent scenes.
pub(super) fn watch_assets(
    mut gltfs: MessageReader<AssetEvent<Gltf>>,
    mut worlds: MessageReader<AssetEvent<WorldAsset>>,
    server: Res<AssetServer>,
    editor: Res<Editor>,
    mut previews: ResMut<Thumbnails>,
) {
    for event in gltfs.read() {
        if let AssetEvent::Modified { id } = event
            && let Some(path) = server.get_path(*id)
            && previews.asset_modified(&editor, &path.path().to_string_lossy())
        {
            previews.invalidate_asset(&path.path().to_string_lossy());
        }
    }
    for event in worlds.read() {
        if let AssetEvent::Modified { id } = event
            && let Some(path) = server.get_path(*id)
            && previews.asset_modified(&editor, &path.path().to_string_lossy())
        {
            previews.invalidate_asset(&path.path().to_string_lossy());
        }
    }
}

pub(super) fn update(
    mut commands: Commands,
    editor: Res<Editor>,
    browser: Res<Browser>,
    window: Res<super::ui::GridWindow>,
    mut cards: Query<(&super::ui::PreviewCard, &mut ImageNode)>,
    mut previews: ResMut<Thumbnails>,
    assets: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    prepared: Query<(), With<scene::Prepared>>,
    subjects: Query<(), With<scene::Subject>>,
    children: Query<&Children>,
    meshes: Query<(&Aabb, &GlobalTransform)>,
    (mesh_materials, materials): (
        Query<&MeshMaterial3d<StandardMaterial>>,
        Res<Assets<StandardMaterial>>,
    ),
    mut backdrop_assets: (
        ResMut<Assets<Mesh>>,
        ResMut<Assets<super::viewport_background::BackgroundMaterial>>,
    ),
    mut images: ResMut<Assets<Image>>,
) {
    let model_environment = if editor.has_scene() {
        editor.environment().clone()
    } else {
        crate::SceneLighting::editor_default()
    };
    previews.refresh_models(&editor, &model_environment, &mut commands);
    let scene_count = editor.project.scenes.len();
    let placeholder = previews.placeholder.clone();
    previews
        .scene_images
        .resize(scene_count, placeholder.clone());
    previews.snapshots.resize_with(scene_count, || None);
    previews.changed.resize_with(scene_count, Instant::now);
    if previews.scan_revision != Some(browser.scan_revision) {
        // A rescan can reassign catalog slots. Keep every card whose source
        // still occupies its slot. Actual content changes arrive from the registry.
        let remapped: Vec<_> = previews
            .files
            .iter()
            .filter_map(|(&target, files)| {
                let unchanged = match target {
                    Target::Model(i) => editor.project.catalog.get(i).is_some_and(|model| {
                        model.asset == files.source
                            && Some(model.preview_direction) == files.direction
                    }),
                    Target::Scene(i) => editor
                        .project
                        .scenes
                        .get(i)
                        .is_some_and(|scene| scene.file == files.source),
                };
                (!unchanged).then_some(target)
            })
            .collect();
        for target in remapped {
            if previews
                .job
                .as_ref()
                .is_some_and(|job| job.target == target)
            {
                previews.cancel(&mut commands);
            }
            previews.files.remove(&target);
            previews.completed.remove(&target);
            previews.failed.remove(&target);
            previews.recent.retain(|&cached| cached != target);
            match target {
                Target::Model(i) => {
                    if let Some(image) = previews.images.get_mut(i) {
                        *image = placeholder.clone();
                    }
                }
                Target::Scene(i) => {
                    if let Some(image) = previews.scene_images.get_mut(i) {
                        *image = placeholder.clone();
                    }
                    if let Some(snapshot) = previews.snapshots.get_mut(i) {
                        *snapshot = None;
                    }
                    previews.pending_snapshots.remove(&i);
                }
            }
        }
        previews.browser_revision = None;
        previews.scan_revision = Some(browser.scan_revision);
    }
    let changed_folder = previews.browser_revision != Some(window.revision);
    let visible = if editor.drawer { window.visible() } else { &[] };
    if changed_folder {
        previews.browser_revision = Some(window.revision);
        // Protect recently viewed thumbnails even when they were already cached.
        for entry in visible {
            if let Some(target) = Target::from_entry(*entry)
                && let Some(i) = previews.recent.iter().position(|&t| t == target)
            {
                previews.recent.remove(i);
                previews.recent.push_back(target);
            }
        }
    }
    for (card, mut image) in &mut cards {
        if let Some(target) = Target::from_entry(card.0) {
            let handle = match target {
                Target::Model(i) => previews.images.get(i),
                Target::Scene(i) => previews.scene_images.get(i),
            };
            if let Some(handle) = handle
                && image.image != *handle
            {
                image.image = handle.clone();
            }
        }
    }
    // Warm the same calibrated environment used by the active scene.
    let start_warmup = !previews.warmed_up && previews.warmup.is_none() && editor.has_scene();
    if start_warmup {
        previews.warmup = Some(if editor.project.catalog.is_empty() {
            Target::Scene(editor.room)
        } else {
            Target::Model(0)
        });
    }
    if editor.is_changed()
        || changed_folder
        || start_warmup
        || !previews.pending_snapshots.is_empty()
    {
        previews.refresh_scenes(&editor, visible, &mut commands);
    }
    let Some(mut job) = previews.job.take() else {
        let ready = |target| match target {
            Target::Model(_) => true,
            Target::Scene(room) => {
                previews.snapshots[room].is_some() && previews.changed[room].elapsed() >= SETTLE
            }
        };
        let next = next_missing(visible, &previews.completed, ready).or_else(|| {
            previews
                .warmup
                .filter(|target| !previews.completed.contains(target) && ready(*target))
        });
        if let Some(target) = next {
            previews.allocate(target, visible, &mut images);
            let path = match target {
                Target::Model(index) => &editor.project.catalog[index].asset,
                Target::Scene(room) => &editor.project.scenes[room].asset,
            };
            previews.remember_files(&editor, target, path);
            let asset = (!path.is_empty()).then(|| ModelSource::load(&assets, path, true));
            let root = commands
                .spawn((
                    Transform::IDENTITY,
                    Visibility::Hidden,
                    RenderLayers::layer(LAYER),
                ))
                .id();
            previews.job = Some(Job {
                target,
                asset,
                root,
                instances: Vec::new(),
                models: Vec::new(),
                started: Instant::now(),
                camera: None,
                stable_frames: 0,
            });
        }
        return;
    };

    // Leaving a folder or editing a snapshot never lets an obsolete render finish its job.
    if previews.warmup != Some(job.target)
        && !visible
            .iter()
            .any(|&entry| Target::from_entry(entry) == Some(job.target))
    {
        previews.stop(job, &mut commands);
        return;
    }
    let world = if job.instances.is_empty() {
        job.asset
            .as_ref()
            .map_or(Ok(None), |source| source.world(&assets, &gltfs))
    } else {
        Ok(None)
    };
    let error = world
        .as_ref()
        .err()
        .cloned()
        .or_else(|| {
            job.models
                .iter()
                .find_map(|model| load_error(&assets, model))
        })
        .or_else(|| {
            (job.started.elapsed() >= Duration::from_secs(20)).then(|| "preview timed out".into())
        });
    if let Some(error) = error {
        warn!("Thumbnail for {:?}: {error}", job.target);
        previews.fail(job.target);
        previews.stop(job, &mut commands);
        return;
    }

    if job.instances.is_empty() {
        let native = match job.target {
            Target::Scene(room) => previews.snapshots[room]
                .as_ref()
                .filter(|snapshot| snapshot.asset.is_empty()),
            _ => None,
        };
        if let Some(snapshot) = native {
            let (root, spatial) = scene::spawn_native(&mut commands, snapshot, job.root);
            job.instances.push(root);
            for model in &snapshot.edits.models {
                let root = crate::runtime::spawn_model(&mut commands, &assets, model);
                commands
                    .entity(root)
                    .insert(ChildOf(job.root))
                    .observe(scene::prepare);
                if let Some(parent) = spatial.get(&model.id) {
                    commands
                        .entity(root)
                        .insert((Transform::IDENTITY, ChildOf(*parent)));
                }
                job.instances.push(root);
                job.models.push(crate::runtime::load_model(&assets, model));
            }
        } else if let Ok(Some(world)) = world {
            let mut instance = commands.spawn((WorldAssetRoot(world), ChildOf(job.root)));
            if matches!(job.target, Target::Model(_)) {
                instance.insert(scene::CatalogModel);
            }
            if let Target::Scene(room) = job.target
                && let Some(snapshot) = &previews.snapshots[room]
            {
                if let Some(setup) = editor.project.scene_setup {
                    let logical =
                        crate::asset_paths::original_asset_path(&editor.root, &snapshot.asset)
                            .unwrap_or_else(|_| snapshot.asset.clone());
                    setup(&mut instance, &assets, &logical);
                }
                instance.insert(snapshot.clone());
            }
            job.instances.push(instance.observe(scene::prepare).id());
            if let Target::Scene(room) = job.target
                && let Some(snapshot) = &previews.snapshots[room]
            {
                for model in &snapshot.edits.models {
                    let root = crate::runtime::spawn_model(&mut commands, &assets, model);
                    commands
                        .entity(root)
                        .insert(ChildOf(job.root))
                        .observe(scene::prepare);
                    job.instances.push(root);
                    job.models.push(crate::runtime::load_model(&assets, model));
                }
            }
        }
    } else if job.camera.is_some() {
        if job.stable_frames >= 5 {
            if let Some(files) = previews.files.get_mut(&job.target) {
                use bevy::asset::VisitAssetDependencies;
                for entity in children.iter_descendants(job.root) {
                    if let Ok(handle) = mesh_materials.get(entity)
                        && let Some(material) = materials.get(&handle.0)
                    {
                        material.visit_dependencies(&mut |id| {
                            if let Some(path) = assets.get_path(id) {
                                files
                                    .paths
                                    .insert(asset_key(&editor, &path.path().to_string_lossy()));
                            }
                        });
                    }
                }
            }
            previews.complete(job.target);
            previews.stop(job, &mut commands);
            return;
        }
    } else if job.instances.iter().all(|&root| prepared.contains(root))
        && job
            .models
            .iter()
            .all(|model| assets.is_loaded_with_dependencies(model.id()))
    {
        // The gameplay camera often includes a whole room. Frame its editable
        // subjects instead, while retaining the floor and surroundings in the render.
        let bounds = framing::bounds(job.root, &subjects, &children, &meshes);
        let direction = match job.target {
            Target::Scene(room) => previews.snapshots[room]
                .as_ref()
                .map(|snapshot| *snapshot.camera.pose.transform().back()),
            Target::Model(index) => {
                Some(Vec3::from(editor.project.catalog[index].preview_direction))
            }
        };
        let view = bounds
            .zip(direction)
            .map(|(bounds, direction)| {
                framing::fit(bounds, direction, WIDTH as f32 / HEIGHT as f32)
            })
            .or_else(|| match job.target {
                Target::Scene(room) => previews.snapshots[room].as_ref().map(|snapshot| {
                    (
                        snapshot.camera.pose.transform(),
                        viewport::projection(snapshot.projection, snapshot.camera.lens),
                    )
                }),
                Target::Model(_) => None,
            });
        if let Some((view, projection)) = view {
            let snapshot = match job.target {
                Target::Scene(room) => previews.snapshots.get(room).and_then(Option::as_ref),
                Target::Model(_) => None,
            };
            let host_lighting = matches!(job.target, Target::Scene(_))
                && (editor.project.scene_setup.is_some() || editor.project.camera_setup.is_some());
            let environment = if matches!(job.target, Target::Model(_)) {
                Some(model_environment.clone())
            } else {
                snapshot.and_then(|snapshot| {
                    snapshot
                        .edits
                        .lighting
                        .clone()
                        .or_else(|| (!host_lighting).then(crate::SceneLighting::editor_default))
                })
            };
            let custom_lights = snapshot.is_some_and(|snapshot| !snapshot.edits.lights.is_empty());
            let contacts = snapshot.is_some_and(|snapshot| {
                snapshot.edits.lights.iter().any(|light| {
                    light.enabled
                        && light.contact_shadows
                        && light.kind != crate::lighting::LightKind::Rect
                })
            });
            let studio_fill = environment.is_none() && !custom_lights && !host_lighting;
            let fill = DirectionalLight {
                illuminance: if studio_fill { 2400.0 } else { 0.0 },
                color: Color::srgb(1.0, 0.94, 0.86),
                shadow_maps_enabled: studio_fill,
                ..default()
            };
            // Light from the camera's upper left, giving every face a distinct value.
            let light_direction = view.back() * 0.35 - view.right() * 0.85 + Vec3::Y * 1.4;
            let light_pose =
                Transform::from_translation(light_direction).looking_at(Vec3::ZERO, Vec3::Y);
            let shadow_distance = bounds
                .map_or(30.0, |bounds| {
                    view.translation.distance(Vec3::from(bounds.center))
                        + Vec3::from(bounds.half_size).length() * 4.0
                })
                .max(0.1);
            let shadow = CascadeShadowConfigBuilder {
                num_cascades: 1,
                minimum_distance: 0.0,
                maximum_distance: shadow_distance,
                ..default()
            }
            .build();
            if let Some(light) = previews.light {
                commands.entity(light).insert((
                    light_pose,
                    fill,
                    shadow,
                    if studio_fill {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    },
                ));
            } else {
                previews.light = Some(
                    commands
                        .spawn((
                            fill,
                            light_pose,
                            shadow,
                            RenderLayers::layer(LAYER),
                            if studio_fill {
                                Visibility::Inherited
                            } else {
                                Visibility::Hidden
                            },
                        ))
                        .id(),
                );
            }
            if matches!(job.target, Target::Model(_))
                && (!model_environment.has_hdri() || !model_environment.hdri_background)
            {
                let backdrop = super::viewport_background::spawn(
                    &mut commands,
                    &mut backdrop_assets.0,
                    &mut backdrop_assets.1,
                    LAYER,
                );
                commands
                    .entity(backdrop)
                    .insert((Visibility::Visible, view, ChildOf(job.root)));
            }
            commands.entity(job.root).insert(Visibility::Inherited);
            let reusable = previews.view.filter(|view| view.studio && !host_lighting);
            let mut camera = if let Some(view) = reusable {
                commands.entity(view.entity)
            } else {
                if let Some(previous) = previews.view.take() {
                    commands.entity(previous.entity).try_despawn();
                }
                commands.spawn((
                    Camera3d::default(),
                    Hdr,
                    Exposure::BLENDER,
                    Tonemapping::AgX,
                    ScreenSpaceTransmission::default(),
                    AmbientLight {
                        color: Color::srgb(0.83, 0.89, 1.0),
                        brightness: 120.0,
                        ..default()
                    },
                ))
            };
            if let Target::Scene(room) = job.target
                && let Some(setup) = editor.project.camera_setup
            {
                let definition = &editor.project.scenes[room];
                let path = if definition.is_native() {
                    &definition.file
                } else {
                    &definition.asset
                };
                let logical = crate::asset_paths::original_asset_path(&editor.root, path)
                    .unwrap_or_else(|_| path.clone());
                setup(&mut camera, &logical);
            }
            if let Some(environment) = &environment {
                crate::lighting::apply_camera_lighting(&mut camera, environment);
            } else if contacts {
                camera.insert(bevy::pbr::ContactShadows::default());
            }
            job.camera = Some(
                camera
                    .insert((
                        Camera {
                            order: -3,
                            clear_color: ClearColorConfig::Custom(Color::srgb(0.065, 0.078, 0.11)),
                            ..default()
                        },
                        RenderTarget::Image(previews.image(job.target).into()),
                        RenderLayers::layer(LAYER),
                        projection,
                        view,
                    ))
                    .remove::<(TemporalAntiAliasing, ScreenSpaceAmbientOcclusion)>()
                    .id(),
            );
            previews.view = job.camera.map(|entity| PreviewView {
                entity,
                studio: !host_lighting,
            });
            job.started = Instant::now();
        }
    }
    previews.job = Some(job);
}

pub(super) fn track_frame(
    mut world: ResMut<MainWorld>,
    cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
) {
    let environment_ready = world
        .get_resource::<Thumbnails>()
        .and_then(|previews| previews.job.as_ref())
        .and_then(|job| job.camera)
        .is_some_and(|camera| {
            if world
                .get::<crate::hdri::CameraEnvironment>(camera)
                .is_none()
            {
                return true;
            }
            world
                .get::<EnvironmentMapLight>(camera)
                .is_some_and(|environment| {
                    images.get(&environment.diffuse_map).is_some()
                        && images.get(&environment.specular_map).is_some()
                })
        });
    if let Some(mut previews) = world.get_resource_mut::<Thumbnails>()
        && let Some(job) = &mut previews.job
        && job.camera.is_some()
    {
        job.stable_frames = if environment_ready && cache.waiting_pipelines().next().is_none() {
            job.stable_frames.saturating_add(1)
        } else {
            0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_missing_previews_in_the_displayed_folder_are_requested() {
        let entries = [Entry::Folder(4), Entry::Model(650), Entry::Model(23)];
        let mut completed = HashSet::from([Target::Model(23)]);
        assert_eq!(
            next_missing(&entries, &completed, |_| true),
            Some(Target::Model(650))
        );
        completed.insert(Target::Model(650));
        assert_eq!(next_missing(&entries, &completed, |_| true), None);
        assert_eq!(
            next_missing(&[Entry::Model(44)], &completed, |_| true),
            Some(Target::Model(44))
        );
        assert_eq!(
            next_missing(&[Entry::Folder(8)], &completed, |_| true),
            None
        );
    }

    #[test]
    fn scene_source_and_document_share_a_preview_but_scene_variants_do_not() {
        let entries = [
            Entry::Scene { file: 10, room: 2 },
            Entry::Scene { file: 11, room: 2 },
            Entry::Scene { file: 10, room: 3 },
            Entry::Model(2),
        ];
        let completed = HashSet::from([Target::Scene(2)]);
        assert_eq!(
            next_missing(&entries, &completed, |_| true),
            Some(Target::Scene(3))
        );
        let completed = HashSet::from([Target::Scene(2), Target::Scene(3)]);
        assert_eq!(
            next_missing(&entries, &completed, |_| true),
            Some(Target::Model(2))
        );
        assert_eq!(
            next_missing(&entries, &HashSet::new(), |target| target
                == Target::Model(2)),
            Some(Target::Model(2))
        );
    }

    #[test]
    fn changing_a_scene_cancels_its_old_render_and_invalidates_its_cached_image() {
        use crate::{CameraPlacement, EditorProject, LevelEdits};
        use bevy::ecs::system::SystemState;

        let project = EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        let mut editor = Editor::new(project, LevelEdits::default());
        let mut images = Assets::<Image>::default();
        let mut previews = Thumbnails::new(
            &mut images,
            editor.project.catalog.len(),
            editor.project.scenes.len(),
        );
        previews.snapshots[0] = Some(scene::SceneSnapshot::read(&editor, 0).unwrap());
        previews.completed.insert(Target::Scene(0));
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let camera = world.spawn_empty().id();
        previews.job = Some(Job {
            target: Target::Scene(0),
            asset: None,
            root,
            instances: Vec::new(),
            models: Vec::new(),
            started: Instant::now(),
            camera: Some(camera),
            stable_frames: 4,
        });
        editor.edits.cameras.insert(
            "main".into(),
            CameraPlacement {
                lens: 60.0,
                ..editor.camera(0)
            },
        );
        let mut state = SystemState::<Commands>::new(&mut world);
        // The cached scene must also track edits made while its folder is hidden.
        previews.refresh_scenes(&editor, &[], &mut state.get_mut(&mut world).unwrap());
        state.apply(&mut world);

        assert!(previews.job.is_none());
        assert!(!previews.completed.contains(&Target::Scene(0)));
        assert_eq!(previews.snapshots[0].as_ref().unwrap().camera.lens, 60.0);
        assert!(world.get_entity(root).is_err());
        assert!(world.get_entity(camera).is_err());
    }
}
