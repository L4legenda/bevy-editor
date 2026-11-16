//! Optional real-render diagnostic; source cache files remain untouched.
use super::*;
use crate::editor::{
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::fixture,
};
use bevy::render::render_resource::TextureUsages;

#[test]
#[ignore = "requires GPU and BEVY_DIAGNOSTIC_JOB containing a completed Hunyuan cache directory"]
fn gpu_generation_surface_diagnostic_raw_reduced_lit_and_unlit() {
    let source =
        PathBuf::from(std::env::var_os("BEVY_DIAGNOSTIC_JOB").expect("Set BEVY_DIAGNOSTIC_JOB"));
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/generation-surface-qa");
    std::fs::create_dir_all(&output).unwrap();
    let (mut app, _, _) = fixture();
    for _ in 0..35 {
        frame(&mut app);
    }
    app.world_mut().resource_mut::<Editor>().open_generation();
    app.world_mut()
        .insert_resource(installation::State::installed_fixture());
    app.world_mut().insert_resource(State::ready_fixture());
    let mut baseline = None;
    let mut cases = vec![
        ("reduced", source.join("mesh.glb")),
        ("raw", source.join("mesh-raw.glb")),
    ];
    for (name, file) in [
        ("200k", "less-reduced/mesh.glb"),
        ("sharp", "sharp-normals/mesh.glb"),
        ("flat", "flat-normals/mesh.glb"),
    ] {
        let path = output.join(file);
        if path.is_file() {
            cases.push((name, path));
        }
    }
    for (name, file) in cases {
        let folder = output.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::copy(file, folder.join("mesh.glb")).unwrap();
        let mut result: backend::ResultModel =
            serde_json::from_slice(&std::fs::read(source.join("result.json")).unwrap()).unwrap();
        result.directory = folder.clone();
        app.world_mut().resource_mut::<State>().result = Some(result);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            frame(&mut app);
            let p = app.world().resource::<Preview>();
            if p.ready && p.requested.as_ref() == Some(&folder.join("mesh.glb")) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Preview decode timed out: {}",
                app.world().resource::<State>().error
            );
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        assert!(app.world().resource::<Preview>().ready);
        let (target, material) = {
            let mut p = app.world_mut().resource_mut::<Preview>();
            let (center, radius) = *baseline.get_or_insert((p.center, p.radius));
            p.center = center;
            p.radius = radius;
            p.distance = radius * 2.15;
            p.pitch = 0.05;
            (p.image.clone(), p.material.clone())
        };
        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .get_mut(&target)
            .unwrap()
            .texture_descriptor
            .usage |= TextureUsages::COPY_SRC;
        for (side, angle) in [
            ("front", 0.0),
            ("back", std::f32::consts::PI),
            ("oblique", 0.4),
        ] {
            app.world_mut().resource_mut::<Preview>().yaw = angle;
            for unlit in [false, true] {
                app.world_mut()
                    .resource_mut::<Assets<StandardMaterial>>()
                    .get_mut(&material)
                    .unwrap()
                    .unlit = unlit;
                for _ in 0..8 {
                    frame(&mut app);
                }
                let path = output.join(format!(
                    "{name}-{side}-{}.png",
                    if unlit { "unlit" } else { "lit" }
                ));
                capture_when(&mut app, &target, &path, |image| {
                    image.pixels().filter(|p| p.0[0] > 80).count() > 1000
                });
                eprintln!("{}", path.display());
            }
        }
    }
}
