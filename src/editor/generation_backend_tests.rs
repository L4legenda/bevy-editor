use super::*;
use crate::editor::{
    model_document::{MaterialSlot, Object, glb::Glb},
    model_primitives::Primitive,
};
use bevy::prelude::*;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "bevy-hunyuan-backend-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if let (Ok(path), Ok(parent)) = (self.0.canonicalize(), std::env::temp_dir().canonicalize())
            && path.parent() == Some(parent.as_path())
            && path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("bevy-hunyuan-backend-"))
        {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn model(path: &Path) -> ResultModel {
    fs::create_dir_all(path).unwrap();
    let object = Object {
        id: 0,
        source_node: None,
        name: "Generated".into(),
        geometry: Primitive::Cube
            .geometry(&Primitive::Cube.defaults())
            .unwrap()
            .into(),
        transform: Transform::IDENTITY,
        materials: vec![MaterialSlot {
            authored: None,
            paint: None,
            material: default(),
            source: serde_json::json!({"material":0}),
        }],
        hidden: false,
    };
    fs::write(path.join("mesh.glb"), Glb::new_asset(&[object]).unwrap()).unwrap();
    fs::write(path.join("mesh.obj"), b"v 0 0 0\n").unwrap();
    ResultModel {
        directory: path.to_owned(),
        name: "Generated".into(),
        seed: 12345,
        steps: 30,
        guidance: 5.0,
        resolution: 380,
        vertices: 8,
        faces: 12,
        raw_faces: 12,
        seconds: 1.0,
        obj: true,
    }
}

#[test]
fn generation_backend_request_validates_names_numbers_and_snapshots_seed() {
    let mut draft = Workspace::default();
    let request = Request::from_draft(&draft, "model".into()).unwrap();
    draft.steps = 50;
    assert_eq!(request.steps, 30);
    assert_eq!(request.seed, 12345);
    draft.name = "../escape".into();
    assert!(Request::from_draft(&draft, "model".into()).is_err());
    draft.name = "Generated".into();
    draft.guidance = f32::NAN;
    assert!(Request::from_draft(&draft, "model".into()).is_err());
    draft.guidance = 5.0;
    draft.random_seed = true;
    let random = Request::from_draft(&draft, "model".into()).unwrap();
    assert_ne!(random.seed, 12345);
}

#[test]
fn generation_backend_import_is_collision_safe_and_copies_only_model_outputs() {
    let scratch = Scratch::new();
    let result = model(&scratch.0.join("system-job"));
    let project = scratch.0.join("project");
    fs::create_dir(&project).unwrap();
    let first = import(&result, &project).unwrap();
    assert_eq!(first, PathBuf::from("models/generated/Generated.glb"));
    assert_eq!(
        fs::read(project.join(&first)).unwrap(),
        fs::read(result.glb()).unwrap()
    );
    assert!(project.join("models/generated/Generated.obj").is_file());
    let second = import(&result, &project).unwrap();
    assert_eq!(second, PathBuf::from("models/generated/Generated (2).glb"));
    fs::write(
        project.join("models/generated/Generated (3).obj"),
        b"preserve",
    )
    .unwrap();
    let fourth = import(&result, &project).unwrap();
    assert!(fourth.ends_with("Generated (4).glb"));
    assert_eq!(
        fs::read(project.join("models/generated/Generated (3).obj")).unwrap(),
        b"preserve"
    );
    assert!(!project.join("models/generated/Generated (3).glb").exists());
    assert_eq!(
        fs::read_dir(project.join("models/generated"))
            .unwrap()
            .count(),
        7
    );
    let mut invalid = result.clone();
    invalid.name = "CON".into();
    assert!(import(&invalid, &project).is_err());
    fs::remove_file(result.directory.join("mesh.obj")).unwrap();
    assert!(import(&result, &project).is_err());
}

#[test]
fn generation_backend_progress_ignores_library_logs_and_reports_actual_steps() {
    let progress = Progress::default();
    progress.event("a third-party log");
    progress.event("BEVY_HUNYUAN {broken");
    assert!(progress.snapshot().stage.is_empty());
    progress
        .event(r#"BEVY_HUNYUAN {"event":"progress","stage":"diffusion","current":7,"total":30}"#);
    let state = progress.snapshot();
    assert_eq!((state.current, state.total), (7, 30));
    assert_eq!(state.stage, "diffusion");
    progress.event(r#"BEVY_HUNYUAN {"event":"error","message":"CUDA unavailable"}"#);
    assert_eq!(
        progress.snapshot().error.as_deref(),
        Some("CUDA unavailable")
    );
}
