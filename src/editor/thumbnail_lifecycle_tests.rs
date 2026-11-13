//! Headless regressions for retained preview resources and hidden async reads.
use super::*;
use crate::{CameraPlacement, EditorProject};
use bevy::{
    ecs::system::SystemState,
    tasks::{IoTaskPool, TaskPool},
};

fn previews() -> Thumbnails {
    Thumbnails::new(&mut Assets::<Image>::default(), 1, 2)
}

fn job(target: Target, root: Entity, camera: Entity) -> Job {
    Job {
        target,
        asset: Handle::default(),
        root,
        instances: Vec::new(),
        models: Vec::new(),
        started: Instant::now(),
        camera: Some(camera),
        stable_frames: 0,
    }
}

#[test]
fn cancelling_a_studio_preview_retains_an_inactive_camera_and_disables_its_light() {
    let mut world = World::new();
    let root = world.spawn_empty().id();
    let instance = world.spawn(ChildOf(root)).id();
    let camera = world.spawn(Camera::default()).id();
    let light = world
        .spawn((
            DirectionalLight {
                illuminance: 2400.0,
                shadow_maps_enabled: true,
                ..default()
            },
            Visibility::Visible,
        ))
        .id();
    let mut previews = previews();
    previews.view = Some(PreviewView {
        entity: camera,
        studio: true,
    });
    previews.light = Some(light);
    previews.job = Some(job(Target::Model(0), root, camera));

    let mut state = SystemState::<Commands>::new(&mut world);
    previews.cancel(&mut state.get_mut(&mut world).unwrap());
    state.apply(&mut world);

    assert!(previews.job.is_none());
    let retained = previews.view.expect("The studio camera should be reusable");
    assert_eq!(retained.entity, camera);
    assert!(retained.studio);
    assert!(!world.get::<Camera>(camera).unwrap().is_active);
    assert!(world.get_entity(root).is_err());
    assert!(world.get_entity(instance).is_err());
    assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Hidden));
    let light = world.get::<DirectionalLight>(light).unwrap();
    assert_eq!(light.illuminance, 0.0);
    assert!(!light.shadow_maps_enabled);
}

#[test]
fn stopping_a_scene_specific_preview_removes_its_camera_and_instance_hierarchy() {
    let mut world = World::new();
    let root = world.spawn_empty().id();
    let instance = world.spawn(ChildOf(root)).id();
    let camera = world.spawn(Camera::default()).id();
    let mut previews = previews();
    previews.view = Some(PreviewView {
        entity: camera,
        studio: false,
    });

    let mut state = SystemState::<Commands>::new(&mut world);
    previews.stop(
        job(Target::Scene(0), root, camera),
        &mut state.get_mut(&mut world).unwrap(),
    );
    state.apply(&mut world);

    assert!(previews.view.is_none());
    assert!(world.get_entity(camera).is_err());
    assert!(world.get_entity(root).is_err());
    assert!(world.get_entity(instance).is_err());
}

struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "bevy-editor-thumbnail-lifecycle-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn opening_a_hidden_scene_cancels_its_pending_disk_read_and_uses_the_live_draft() {
    IoTaskPool::get_or_init(TaskPool::new);
    let scratch = Scratch::new();
    let project = EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let mut editor = Editor::new(project, Default::default());
    editor.root.clone_from(&scratch.0);
    editor.project.asset_root.clone_from(&scratch.0);
    let mut previews = previews();
    // The future never completes and never blocks a worker. This makes the
    // open-document cancellation deterministic, without filesystem races.
    previews.pending_snapshots.insert(
        1,
        scene::SnapshotRead::pending(IoTaskPool::get().spawn(std::future::pending())),
    );
    assert!(previews.snapshots[1].is_none());
    assert!(previews.warmup.is_none());

    assert!(editor.open_document(1));
    let camera = &editor.project.scenes[1].cameras[0];
    editor.edits.cameras.insert(
        camera.id.clone(),
        CameraPlacement {
            lens: 80.0,
            ..camera.placement
        },
    );

    let mut world = World::new();
    let mut state = SystemState::<Commands>::new(&mut world);
    previews.refresh_scenes(&editor, &[], &mut state.get_mut(&mut world).unwrap());
    state.apply(&mut world);

    assert!(!previews.pending_snapshots.contains_key(&1));
    assert!(!previews.completed.contains(&Target::Scene(1)));
    let snapshot = previews.snapshots[1]
        .as_ref()
        .expect("A hidden open document still needs its current draft snapshot");
    assert_eq!(snapshot.camera.lens, 80.0);
    assert!(snapshot.matches_editor(&editor));

    editor.close_document(1);
    assert!(editor.preview_edits(1).is_none());
    assert!(!snapshot.matches_editor(&editor));
}
