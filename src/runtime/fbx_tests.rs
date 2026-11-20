use super::*;
use bevy::{
    asset::LoadState,
    transform::TransformPlugin,
    world_serialization::{WorldInstanceReady, WorldSerializationPlugin},
};
use std::time::{Duration, Instant};

const SOURCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fbx");

#[derive(Component)]
struct Ready;

fn app_at(root: &Path) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin {
            file_path: root.to_string_lossy().into_owned(),
            ..default()
        },
        TransformPlugin,
        WorldSerializationPlugin,
        FbxPlugin,
    ))
    .init_asset::<Mesh>()
    .init_asset::<Image>()
    .init_asset::<StandardMaterial>()
    .register_type::<Name>()
    .register_type::<Transform>()
    .register_type::<GlobalTransform>()
    .register_type::<Visibility>()
    .register_type::<InheritedVisibility>()
    .register_type::<ViewVisibility>()
    .register_type::<Mesh3d>()
    .register_type::<MeshMaterial3d<StandardMaterial>>()
    .register_type::<Aabb>();
    app
}

fn spawn(app: &mut App, file: &str) -> Entity {
    let server = app.world().resource::<AssetServer>().clone();
    let handle = crate::runtime::load_model(
        &server,
        &crate::PlacedModel {
            id: 1,
            asset: file.into(),
            pose: Transform::IDENTITY.into(),
            bounds: None,
        },
    );
    let entity = app
        .world_mut()
        .spawn(WorldAssetRoot(handle.clone()))
        .observe(|ready: On<WorldInstanceReady>, mut commands: Commands| {
            commands.entity(ready.entity).insert(Ready);
        })
        .id();
    let start = Instant::now();
    loop {
        app.update();
        if app.world().get::<Ready>(entity).is_some() {
            assert!(server.is_loaded_with_dependencies(handle.id()));
            return entity;
        }
        if let Some(LoadState::Failed(error)) = server.get_load_state(handle.id()) {
            panic!("Failed to load {file}: {error}");
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "Timed out loading {file}"
        );
        std::thread::yield_now();
    }
}

#[test]
fn ascii_fbx_spawns_and_triangulates_a_centimeter_mesh_without_normals() {
    let mut app = app_at(Path::new(SOURCE));
    spawn(&mut app, "panel_ascii.fbx");
    let world = app.world_mut();
    let (mesh, bounds, global) = world
        .query::<(&Mesh3d, &Aabb, &GlobalTransform)>()
        .single(world)
        .unwrap();
    let mesh = world.resource::<Assets<Mesh>>().get(&mesh.0).unwrap();
    assert_eq!(mesh.indices().unwrap().len(), 6);
    assert!(mesh.contains_attribute(Mesh::ATTRIBUTE_NORMAL));
    let center = global.transform_point(bounds.center.into());
    let extent = global.affine().matrix3.abs() * bounds.half_extents;
    assert!(
        center.abs_diff_eq(Vec3::new(0.75, 1.0, 0.0), 1e-4),
        "{center:?}"
    );
    assert!(Vec3::from(extent).abs_diff_eq(Vec3::new(0.5, 0.5, 0.0), 1e-4));
}

#[test]
fn binary_fbx_preserves_hierarchy_material_slots_and_both_texture_sources() {
    for file in ["props_external.FBX", "embedded/props_embedded.fbx"] {
        let mut app = app_at(Path::new(SOURCE));
        spawn(&mut app, file);
        let world = app.world_mut();
        let mut meshes = world.query::<(
            &ChildOf,
            &Mesh3d,
            &MeshMaterial3d<StandardMaterial>,
            &Aabb,
            &GlobalTransform,
        )>();
        let parts: Vec<_> = meshes.iter(world).collect();
        assert_eq!(
            parts.len(),
            3,
            "{file}: two crate slots and one marker slot"
        );
        let materials = world.resource::<Assets<StandardMaterial>>();
        let images = world.resource::<Assets<Image>>();
        let mut textured_parts = 0;
        for (parent, mesh, material, bounds, global) in parts {
            let object = parent.parent();
            let name = world.get::<Name>(object).unwrap().as_str();
            let group = world.get::<ChildOf>(object).unwrap().parent();
            assert_eq!(world.get::<Name>(group).unwrap().as_str(), "FBX Fixture");
            let material = materials.get(&material.0).unwrap();
            let mesh = world.resource::<Assets<Mesh>>().get(&mesh.0).unwrap();
            assert!(mesh.contains_attribute(Mesh::ATTRIBUTE_UV_0));
            if let Some(texture) = &material.base_color_texture {
                let texture = images.get(texture).unwrap();
                assert_eq!((texture.width(), texture.height()), (4, 4));
                assert_eq!(name, "Crate");
                textured_parts += 1;
            }
            if name == "Marker" {
                let extent = global.affine().matrix3.abs() * bounds.half_extents;
                assert!((extent.y - 0.7).abs() < 1e-4, "{extent:?}");
            }
        }
        assert_eq!(textured_parts, 1, "{file}");
    }
}

#[test]
fn fbx_reports_an_invalid_file_and_can_render_geometry_with_a_missing_texture() {
    let dir = std::env::temp_dir().join(format!(
        "bevy-fbx-missing-{}",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        Path::new(SOURCE).join("props_external.FBX"),
        dir.join("model.fbx"),
    )
    .unwrap();
    std::fs::write(dir.join("broken.fbx"), b"truncated asset").unwrap();
    let mut app = app_at(&dir);
    spawn(&mut app, "model.fbx");
    let world = app.world_mut();
    assert_eq!(world.query::<&Mesh3d>().iter(world).count(), 3);
    let server = world.resource::<AssetServer>().clone();
    let broken: Handle<WorldAsset> = server.load("broken.fbx");
    let start = Instant::now();
    loop {
        app.update();
        if let Some(LoadState::Failed(error)) = server.get_load_state(broken.id()) {
            assert!(error.to_string().contains("FBX"), "{error}");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::yield_now();
    }
    drop(app);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn texture_paths_cannot_leave_the_asset_root() {
    let source = Path::new("models/props.FBX");
    assert_eq!(
        texture_path(source, "..\\textures\\checker.png"),
        Some(PathBuf::from("textures/checker.png"))
    );
    for path in [
        "../../elsewhere.png",
        "/private/outside.png",
        "C:\\textures\\outside.png",
        "https://example.com/file.png",
    ] {
        assert_eq!(texture_path(source, path), None, "{path}");
    }
}
