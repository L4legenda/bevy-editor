//! A scene thumbnail owns a snapshot and never changes the editor's open document.
use std::io;

use bevy::{
    camera::visibility::RenderLayers,
    light::RectLight,
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, poll_once},
    transform::helper::TransformHelper,
    world_serialization::WorldInstanceReady,
};

use crate::{
    CameraPlacement,
    editor::{Editor, RoomEdits},
    project::{LensKind, SceneDefinition},
    runtime,
};

#[derive(Clone, Component, PartialEq)]
pub(in crate::editor) struct SceneSnapshot {
    pub(super) room: usize,
    pub(super) asset: String,
    pub(super) file: String,
    pub(super) edits: RoomEdits,
    pub(super) camera: CameraPlacement,
    pub(super) projection: LensKind,
    groups: Vec<String>,
    excluded: Vec<String>,
    camera_id: String,
    base_camera: CameraPlacement,
    open_document: bool,
}

/// A closed document is read on the I/O pool; an open document is already in memory.
pub(in crate::editor) struct SnapshotRead {
    state: SnapshotReadState,
}

enum SnapshotReadState {
    Ready(SceneSnapshot),
    Pending(Task<io::Result<SceneSnapshot>>),
    Complete,
}

impl SnapshotRead {
    #[cfg(test)]
    pub(super) fn pending(task: Task<io::Result<SceneSnapshot>>) -> Self {
        Self {
            state: SnapshotReadState::Pending(task),
        }
    }

    /// Poll without waiting for disk work. A completed result is returned once.
    pub(super) fn poll(&mut self) -> Option<io::Result<SceneSnapshot>> {
        let result = match &mut self.state {
            SnapshotReadState::Ready(_) => {
                let SnapshotReadState::Ready(snapshot) =
                    std::mem::replace(&mut self.state, SnapshotReadState::Complete)
                else {
                    return None;
                };
                return Some(Ok(snapshot));
            }
            SnapshotReadState::Pending(task) => block_on(poll_once(task)),
            SnapshotReadState::Complete => None,
        };
        if result.is_some() {
            self.state = SnapshotReadState::Complete;
        }
        result
    }
}

impl SceneSnapshot {
    #[cfg(test)]
    pub(super) fn read(editor: &Editor, room: usize) -> io::Result<Self> {
        let definition =
            editor.project.scenes.get(room).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Unknown preview scene")
            })?;
        Self::check_camera(definition)?;
        let open_edits = editor.preview_edits(room);
        let edits = match open_edits {
            Some(edits) => edits.clone(),
            None => editor.project.read_scene(room)?,
        };
        Self::from_edits(definition, room, edits, open_edits.is_some())
    }

    pub(super) fn request(editor: &Editor, room: usize) -> io::Result<SnapshotRead> {
        let definition =
            editor.project.scenes.get(room).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Unknown preview scene")
            })?;
        Self::check_camera(definition)?;
        if let Some(edits) = editor.preview_edits(room) {
            return Ok(SnapshotRead {
                state: SnapshotReadState::Ready(Self::from_edits(
                    definition,
                    room,
                    edits.clone(),
                    true,
                )?),
            });
        }

        // Own only the scene configuration and path needed for this read.
        // No live editor state or whole-project clone crosses into the worker.
        let definition = definition.clone();
        let root = editor.project.asset_root.clone();
        Ok(SnapshotRead {
            state: SnapshotReadState::Pending(IoTaskPool::get().spawn(async move {
                let edits = RoomEdits::read(&root, &definition.file)?;
                definition.validate_edits(&edits)?;
                Self::from_edits(&definition, room, edits, false)
            })),
        })
    }

    fn check_camera(definition: &SceneDefinition) -> io::Result<&crate::CameraDefinition> {
        definition.cameras.first().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Scene has no preview camera")
        })
    }

    fn from_edits(
        definition: &SceneDefinition,
        room: usize,
        edits: RoomEdits,
        open_document: bool,
    ) -> io::Result<Self> {
        let camera = Self::check_camera(definition)?;
        Ok(Self {
            room,
            asset: definition.asset.clone(),
            file: definition.file.clone(),
            camera: edits
                .cameras
                .get(&camera.id)
                .copied()
                .unwrap_or(camera.placement),
            projection: camera.projection,
            edits,
            groups: definition.groups.clone(),
            excluded: definition.excluded_nodes.clone(),
            camera_id: camera.id.clone(),
            base_camera: camera.placement,
            open_document,
        })
    }

    /// Disk changes are invalidated by the browser's scan generation.
    pub(super) fn matches_editor(&self, editor: &Editor) -> bool {
        let Some(definition) = editor.project.scenes.get(self.room) else {
            return false;
        };
        let Some(camera) = definition.cameras.first() else {
            return false;
        };
        self.asset == definition.asset
            && self.file == definition.file
            && self.groups == definition.groups
            && self.excluded == definition.excluded_nodes
            && self.camera_id == camera.id
            && self.base_camera == camera.placement
            && self.projection == camera.projection
            && match editor.preview_edits(self.room) {
                Some(edits) => self.edits == *edits,
                None => !self.open_document,
            }
    }
}

#[derive(Component)]
pub(in crate::editor) struct Prepared;

/// Editable content to frame, without allowing large host-owned floors or
/// backdrops to make the actual scene subjects tiny in their thumbnail.
#[derive(Component)]
pub(in crate::editor) struct Subject;

pub(super) fn spawn_native(
    commands: &mut Commands,
    snapshot: &SceneSnapshot,
    parent: Entity,
) -> Entity {
    let root = commands
        .spawn((
            snapshot.clone(),
            Prepared,
            Transform::IDENTITY,
            Visibility::Inherited,
            RenderLayers::layer(super::LAYER),
            ChildOf(parent),
        ))
        .id();
    for light in &snapshot.edits.lights {
        let mut light = light.clone();
        if snapshot
            .edits
            .lighting
            .as_ref()
            .is_some_and(|settings| !settings.shadows)
        {
            light.shadows = false;
            light.contact_shadows = false;
        }
        let entity = crate::lighting::spawn_light(commands, &light);
        commands
            .entity(entity)
            .insert((ChildOf(root), RenderLayers::layer(super::LAYER)));
    }
    root
}

pub(super) fn prepare(
    ready: On<WorldInstanceReady>,
    snapshots: Query<&SceneSnapshot>,
    children: Query<&Children>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    mut cameras: Query<&mut Camera>,
    lights: Query<(
        Option<&PointLight>,
        Option<&SpotLight>,
        Option<&DirectionalLight>,
        Option<&RectLight>,
    )>,
    mut transforms: ParamSet<(TransformHelper, Query<&mut Transform>)>,
    mut commands: Commands,
) {
    for entity in std::iter::once(ready.entity).chain(children.iter_descendants(ready.entity)) {
        commands
            .entity(entity)
            .insert(RenderLayers::layer(super::LAYER));
        if let Ok(mut camera) = cameras.get_mut(entity) {
            camera.is_active = false;
        }
    }

    if let Ok(snapshot) = snapshots.get(ready.entity) {
        if let Some(environment) = &snapshot.edits.lighting {
            for entity in
                std::iter::once(ready.entity).chain(children.iter_descendants(ready.entity))
            {
                if let Ok((point, spot, directional, rect)) = lights.get(entity) {
                    crate::lighting::apply_imported_light(
                        &mut commands.entity(entity),
                        environment,
                        point,
                        spot,
                        directional,
                        rect,
                    );
                }
            }
        }
        let mut placements: Vec<_> = runtime::imported_nodes(
            ready.entity,
            &snapshot.groups,
            &snapshot.excluded,
            &children,
            &names,
        )
        .filter_map(|(entity, name)| {
            commands.entity(entity).insert(Subject);
            snapshot
                .edits
                .nodes
                .get(name)
                .map(|edit| (entity, edit.pose))
        })
        .collect();
        // An editable placement can contain another editable group. Apply its
        // ancestor first so each saved world pose uses the final parent transform.
        placements.sort_by_key(|(entity, _)| parents.iter_ancestors(*entity).count());
        for (entity, pose) in placements {
            let Ok(parent) = parents.get(entity) else {
                warn!("Scene thumbnail placement {entity} has no parent");
                return;
            };
            let Ok(parent_world) = transforms.p0().compute_global_transform(parent.parent()) else {
                warn!("Scene thumbnail placement {entity} has an invalid parent transform");
                return;
            };
            let mut locals = transforms.p1();
            let Ok(mut local) = locals.get_mut(entity) else {
                warn!("Scene thumbnail placement {entity} has no transform");
                return;
            };
            *local = GlobalTransform::from(pose.transform()).reparented_to(&parent_world);
        }
        if let Ok(root_world) = transforms.p0().compute_global_transform(ready.entity) {
            for light in &snapshot.edits.lights {
                let mut light = light.clone();
                if snapshot
                    .edits
                    .lighting
                    .as_ref()
                    .is_some_and(|environment| !environment.shadows)
                {
                    light.shadows = false;
                    light.contact_shadows = false;
                }
                let local =
                    GlobalTransform::from(light.pose.transform()).reparented_to(&root_world);
                let entity = crate::lighting::spawn_light(&mut commands, &light);
                commands.entity(entity).insert((
                    local,
                    ChildOf(ready.entity),
                    RenderLayers::layer(super::LAYER),
                ));
            }
        }
    } else {
        // A regular catalog model, including a model placed into a scene
        // snapshot, is itself a subject regardless of its internal GLB names.
        commands.entity(ready.entity).insert(Subject);
    }
    commands.entity(ready.entity).insert(Prepared);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EditorProject, NodeEdit, Pose, editor::Selection};
    use bevy::{ecs::system::SystemState, world_serialization::WorldInstanceSpawner};

    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "bevy-editor-scene-preview-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn editor() -> Editor {
        let project = EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        Editor::new(project, RoomEdits::default())
    }

    fn lens_edits(editor: &Editor, room: usize, lens: f32) -> RoomEdits {
        let camera = &editor.project.scenes[room].cameras[0];
        let mut edits = RoomEdits::default();
        edits.cameras.insert(
            camera.id.clone(),
            CameraPlacement {
                lens,
                ..camera.placement
            },
        );
        edits
    }

    fn finish_read(read: &mut SnapshotRead) -> io::Result<SceneSnapshot> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(result) = read.poll() {
                return result;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Scene snapshot I/O task did not finish within five seconds"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn open_snapshot_requests_use_memory_without_reading_invalid_disk_data() {
        let scratch = Scratch::new();
        let mut editor = editor();
        editor.root.clone_from(&scratch.0);
        editor.project.asset_root.clone_from(&scratch.0);
        editor.edits = lens_edits(&editor, 0, 60.0);
        let path = scratch.0.join(&editor.project.scenes[0].file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"invalid JSON").unwrap();

        let mut read = SceneSnapshot::request(&editor, 0).unwrap();
        let snapshot = read
            .poll()
            .expect("Open snapshots must be immediate")
            .unwrap();
        assert!(snapshot == SceneSnapshot::read(&editor, 0).unwrap());
        assert_eq!(snapshot.camera.lens, 60.0);
        assert!(snapshot.matches_editor(&editor));
        assert!(read.poll().is_none());
    }

    #[test]
    fn closed_snapshot_requests_match_sync_reads_and_validate_camera_keys() {
        IoTaskPool::get_or_init(bevy::tasks::TaskPool::new);
        let scratch = Scratch::new();
        let mut editor = editor();
        editor.root.clone_from(&scratch.0);
        editor.project.asset_root.clone_from(&scratch.0);
        let edits = lens_edits(&editor, 1, 80.0);
        edits
            .write(&scratch.0, &editor.project.scenes[1].file)
            .unwrap();
        let expected = SceneSnapshot::read(&editor, 1).unwrap();

        let mut read = SceneSnapshot::request(&editor, 1).unwrap();
        let snapshot = finish_read(&mut read).unwrap();
        assert!(snapshot == expected);
        assert_eq!(snapshot.camera.lens, 80.0);
        assert!(snapshot.matches_editor(&editor));
        assert!(read.poll().is_none());

        // A well-formed DTO can still refer to a camera absent from the scene.
        // Worker reads must retain the same scene-specific validation as read_scene.
        let mut invalid: RoomEdits = RoomEdits::default();
        invalid.cameras.insert("unknown".into(), snapshot.camera);
        invalid
            .write(&scratch.0, &editor.project.scenes[1].file)
            .unwrap();
        let mut read = SceneSnapshot::request(&editor, 1).unwrap();
        let error = finish_read(&mut read)
            .err()
            .expect("Unknown camera accepted");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(read.poll().is_none());
    }

    #[test]
    fn snapshots_use_active_inactive_or_disk_edits_without_switching_documents() {
        let scratch = Scratch::new();
        let mut editor = editor();
        editor.root.clone_from(&scratch.0);
        editor.project.asset_root.clone_from(&scratch.0);
        editor.edits = lens_edits(&editor, 0, 60.0);
        editor.ready = true;
        editor.selection = Some(Selection::Camera(0));
        editor.undo.push(RoomEdits::default());
        let disk = lens_edits(&editor, 1, 80.0);
        disk.write(&scratch.0, &editor.project.scenes[1].file)
            .unwrap();

        let active = SceneSnapshot::read(&editor, 0).unwrap();
        let unopened = SceneSnapshot::read(&editor, 1).unwrap();
        assert_eq!(active.camera.lens, 60.0);
        assert_eq!(unopened.camera.lens, 80.0);
        assert_eq!(editor.room, 0);
        assert_eq!(editor.open_tabs, [0]);
        assert_eq!(editor.selection, Some(Selection::Camera(0)));
        assert!(editor.ready);
        assert_eq!(editor.undo.len(), 1);
        assert!(active.matches_editor(&editor));
        assert!(unopened.matches_editor(&editor));

        assert!(editor.open_document(1));
        editor.edits = lens_edits(&editor, 1, 90.0);
        let inactive = SceneSnapshot::read(&editor, 0).unwrap();
        assert_eq!(inactive.camera.lens, 60.0);
        assert_eq!(SceneSnapshot::read(&editor, 1).unwrap().camera.lens, 90.0);
        assert!(active.matches_editor(&editor));
        assert!(!unopened.matches_editor(&editor));
        assert_eq!(editor.room, 1);

        editor.close_document(0);
        assert!(!inactive.matches_editor(&editor));
        let closed = SceneSnapshot::read(&editor, 0).unwrap();
        assert_eq!(closed.camera.lens, 45.0);
        assert!(closed.edits.cameras.is_empty());
    }

    #[test]
    fn camera_defaults_and_scene_configuration_invalidate_an_overridden_snapshot() {
        let mut editor = editor();
        editor.edits = lens_edits(&editor, 0, 60.0);
        let snapshot = SceneSnapshot::read(&editor, 0).unwrap();
        assert!(snapshot.matches_editor(&editor));
        editor.project.scenes[0].cameras[0].placement.lens = 50.0;
        assert!(!snapshot.matches_editor(&editor));
        let snapshot = SceneSnapshot::read(&editor, 0).unwrap();
        assert_eq!(snapshot.camera.lens, 60.0);
        editor.project.scenes[0].cameras[0].projection = LensKind::Orthographic;
        assert!(!snapshot.matches_editor(&editor));
        let snapshot = SceneSnapshot::read(&editor, 0).unwrap();
        editor.project.scenes[0].asset = "scenes/moved.glb".into();
        assert!(!snapshot.matches_editor(&editor));
        let snapshot = SceneSnapshot::read(&editor, 0).unwrap();
        editor.project.scenes[0]
            .excluded_nodes
            .push("Hidden".into());
        assert!(!snapshot.matches_editor(&editor));
    }

    fn trigger_ready(app: &mut App, root: Entity) {
        let mut assets = Assets::<WorldAsset>::default();
        let handle = assets.add(WorldAsset::new(World::new()));
        app.insert_resource(assets);
        let instance_id = WorldInstanceSpawner::default()
            .spawn_sync(app.world_mut(), handle.id())
            .unwrap();
        app.world_mut().trigger(WorldInstanceReady {
            entity: root,
            instance_id,
        });
        app.update();
    }

    fn world_pose(app: &mut App, entity: Entity) -> Transform {
        let mut state = SystemState::<TransformHelper>::new(app.world_mut());
        state
            .get(app.world())
            .unwrap()
            .compute_global_transform(entity)
            .unwrap()
            .compute_transform()
    }

    fn assert_pose(actual: Transform, expected: Pose) {
        let expected = expected.transform();
        assert!(actual.translation.abs_diff_eq(expected.translation, 1e-4));
        assert!(actual.rotation.abs_diff_eq(expected.rotation, 1e-4));
        assert!(actual.scale.abs_diff_eq(expected.scale, 1e-4));
    }

    #[test]
    fn preparation_applies_nested_world_edits_excludes_nodes_and_isolates_scene_components() {
        let mut editor = editor();
        editor.project.scenes[0].groups.push("NestedProps".into());
        editor.project.scenes[0]
            .excluded_nodes
            .push("Excluded".into());
        let parent_pose: Pose = Transform::from_xyz(2.0, 3.0, -4.0)
            .with_rotation(Quat::from_rotation_y(0.3))
            .with_scale(Vec3::splat(1.5))
            .into();
        let child_pose: Pose = Transform::from_xyz(-5.0, 6.0, 7.0)
            .with_rotation(Quat::from_rotation_y(-0.6))
            .with_scale(Vec3::splat(0.75))
            .into();
        for (name, pose) in [
            ("Parent", parent_pose),
            ("Child", child_pose),
            ("Excluded", child_pose),
        ] {
            editor.edits.nodes.insert(
                name.into(),
                NodeEdit {
                    original: Transform::IDENTITY.into(),
                    pose,
                },
            );
        }
        let snapshot = SceneSnapshot::read(&editor, 0).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let root = app
            .world_mut()
            .spawn((snapshot, Transform::from_xyz(-4.0, 1.0, 0.0)))
            .observe(prepare)
            .id();
        let group = app
            .world_mut()
            .spawn((
                Name::new("Props"),
                Transform::from_xyz(3.0, 0.0, -2.0)
                    .with_rotation(Quat::from_rotation_y(0.7))
                    .with_scale(Vec3::splat(2.0)),
                ChildOf(root),
            ))
            .id();
        let parent = app
            .world_mut()
            .spawn((Name::new("Parent"), Transform::IDENTITY, ChildOf(group)))
            .id();
        let nested = app
            .world_mut()
            .spawn((
                Name::new("NestedProps"),
                Transform::from_xyz(1.0, 2.0, 3.0),
                ChildOf(parent),
            ))
            .id();
        let child = app
            .world_mut()
            .spawn((
                Name::new("Child"),
                Transform::IDENTITY,
                Mesh3d(Handle::default()),
                ChildOf(nested),
            ))
            .id();
        let excluded_pose = Transform::from_xyz(8.0, 9.0, 10.0);
        let excluded = app
            .world_mut()
            .spawn((Name::new("Excluded"), excluded_pose, ChildOf(group)))
            .id();
        let unedited = app
            .world_mut()
            .spawn((
                Name::new("Unedited"),
                Transform::from_xyz(1.0, 0.0, 0.0),
                ChildOf(group),
            ))
            .id();
        let camera = app
            .world_mut()
            .spawn((Camera::default(), Transform::IDENTITY, ChildOf(root)))
            .id();
        let light = app
            .world_mut()
            .spawn((DirectionalLight::default(), ChildOf(root)))
            .id();
        let main_camera = app
            .world_mut()
            .spawn((Camera::default(), RenderLayers::layer(0)))
            .id();

        // GlobalTransform still contains its required-component default here;
        // preparation must use live local transforms rather than stale globals.
        trigger_ready(&mut app, root);
        assert!(app.world().get::<Prepared>(root).is_some());
        assert_pose(world_pose(&mut app, parent), parent_pose);
        assert_pose(world_pose(&mut app, child), child_pose);
        assert_eq!(
            *app.world().get::<Transform>(excluded).unwrap(),
            excluded_pose
        );
        for entity in [
            root, group, parent, nested, child, excluded, unedited, camera, light,
        ] {
            assert_eq!(
                app.world().get::<RenderLayers>(entity),
                Some(&RenderLayers::layer(super::super::LAYER))
            );
        }
        assert!(!app.world().get::<Camera>(camera).unwrap().is_active);
        assert!(app.world().get::<Camera>(main_camera).unwrap().is_active);
        for entity in [parent, child, unedited] {
            assert!(app.world().get::<Subject>(entity).is_some());
        }
        for entity in [root, group, nested, excluded, camera, light, main_camera] {
            assert!(app.world().get::<Subject>(entity).is_none());
        }
        assert_eq!(
            app.world().get::<RenderLayers>(main_camera),
            Some(&RenderLayers::layer(0))
        );
    }

    #[test]
    fn added_model_roots_are_prepared_and_isolated_without_a_scene_snapshot() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let root = app
            .world_mut()
            .spawn(Transform::IDENTITY)
            .observe(prepare)
            .id();
        let camera = app
            .world_mut()
            .spawn((Camera::default(), ChildOf(root)))
            .id();
        trigger_ready(&mut app, root);
        assert!(app.world().get::<Prepared>(root).is_some());
        assert!(app.world().get::<Subject>(root).is_some());
        assert!(app.world().get::<Subject>(camera).is_none());
        assert!(!app.world().get::<Camera>(camera).unwrap().is_active);
        for entity in [root, camera] {
            assert_eq!(
                app.world().get::<RenderLayers>(entity),
                Some(&RenderLayers::layer(super::super::LAYER))
            );
        }
    }
}
