//! Reproducible codec benchmarks and an explicit build step for runtime scenes.
use bevy_scene_format::*;
use serde::{Deserialize, Serialize};
use std::{
    hint::black_box,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Serialize)]
struct Sample {
    entities: usize,
    profile: String,
    format: String,
    bytes: usize,
    median_us: f64,
    p95_us: f64,
    samples_us: Vec<f64>,
}
#[derive(Serialize, Deserialize)]
struct LegacyPose {
    position: [f32; 3],
    rotation: [f32; 4],
    scale: [f32; 3],
}
#[derive(Serialize, Deserialize)]
struct LegacyModel {
    id: u32,
    asset: String,
    pose: LegacyPose,
}
#[derive(Serialize, Deserialize)]
struct LegacyScene {
    models: Vec<LegacyModel>,
    model_names: std::collections::BTreeMap<u32, String>,
}
fn legacy_fixture(scene: &Scene) -> LegacyScene {
    LegacyScene {
        models: scene
            .entities
            .iter()
            .enumerate()
            .map(|(i, e)| LegacyModel {
                id: i as u32 + 1,
                asset: "models/chair.glb".into(),
                pose: LegacyPose {
                    position: e.transform.translation,
                    rotation: e.transform.rotation,
                    scale: e.transform.scale,
                },
            })
            .collect(),
        model_names: scene
            .entities
            .iter()
            .enumerate()
            .map(|(i, e)| (i as u32 + 1, e.name.clone().unwrap()))
            .collect(),
    }
}
fn measure(mut action: impl FnMut(), samples: usize) -> (f64, f64, Vec<f64>) {
    action();
    let mut times = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        action();
        times.push(started.elapsed().as_secs_f64() * 1e6);
    }
    times.sort_by(f64::total_cmp);
    (
        times[times.len() / 2],
        times[(times.len() * 95 / 100).min(times.len() - 1)],
        times,
    )
}
fn fixture(count: usize, components: bool) -> Scene {
    let mut scene = Scene::new("Synthetic equivalent data, not engine objects");
    scene.resources.insert(
        "model".into(),
        Resource::external("model", "models/chair.glb"),
    );
    for i in 0..count {
        let mut entity = Entity::new(format!("e{i:08}"));
        entity.model = Some("model".into());
        entity.name = Some(format!("Entity_{i}"));
        entity.transform.translation = [(i % 100) as f32, (i / 100) as f32, -((i % 7) as f32)];
        if components {
            entity.components.insert(
                "game.Interaction".into(),
                serde_json::json!({"enabled": true, "power": 9.125, "label": "Предмет"}),
            );
        }
        scene.entities.push(entity);
    }
    scene
}
fn bench(output: &Path) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let expected = Revision::read(output)?;
    // Fail before doing measurements if the destination cannot be created.
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut rows = Vec::new();
    for count in [100, 1000, 10_000, 100_000] {
        for components in [false, true] {
            let profile = if components {
                "components"
            } else {
                "transforms"
            };
            let source = fixture(count, components);
            let json = source.to_source()?;
            if count == 100 && !components {
                let directory = output.parent().unwrap_or(Path::new("."));
                let mut renamed = source.clone();
                renamed.entities[10].name = Some("Renamed independently".into());
                let mut moved = source.clone();
                moved.entities[10].transform.translation[0] += 5.0;
                for (name, data) in [
                    ("merge.base.scene", json.clone()),
                    ("merge.ours.scene", renamed.to_source()?),
                    ("merge.theirs.scene", moved.to_source()?),
                ] {
                    let path = directory.join(name);
                    let revision = Revision::read(&path)?;
                    atomic_write(&path, &data, revision)?;
                }
            }
            let resolved = resolve(&source, &mut |_: &Resource| {
                Err(Error("Unexpected load".into()))
            })?;
            let raw = cook(&resolved)?;
            let compressed = compress(&raw)?;
            let legacy = legacy_fixture(&source);
            let legacy_bytes = serde_json::to_vec_pretty(&legacy)?;
            for (format, bytes) in [
                ("legacy_typed_parse_only", &legacy_bytes),
                ("source_parse_validate", &json),
                ("packed_checked_view", &raw),
                ("lz4_checked_view", &compressed),
                ("packed_full_materialize", &raw),
                ("source_encode_validate", &json),
                ("compile_validate", &raw),
                ("lz4_encode_validate", &compressed),
            ] {
                if format == "legacy_typed_parse_only" && components {
                    continue;
                }
                let (median_us, p95_us, samples_us) = measure(
                    || match format {
                        "legacy_typed_parse_only" => {
                            black_box(
                                serde_json::from_slice::<LegacyScene>(black_box(bytes)).unwrap(),
                            );
                        }
                        "source_parse_validate" => {
                            black_box(Scene::from_slice(black_box(bytes)).unwrap());
                        }
                        "packed_checked_view" => {
                            black_box(PackedScene::open(black_box(bytes), None).unwrap());
                        }
                        "lz4_checked_view" => {
                            let decoded = decompress(black_box(bytes)).unwrap();
                            black_box(
                                PackedScene::open(black_box(decoded.as_ref()), None).unwrap(),
                            );
                        }
                        "source_encode_validate" => {
                            black_box(source.to_source().unwrap());
                        }
                        "compile_validate" => {
                            black_box(cook(&resolved).unwrap());
                        }
                        "lz4_encode_validate" => {
                            black_box(compress(&raw).unwrap());
                        }
                        _ => {
                            black_box(
                                PackedScene::open(black_box(bytes), None)
                                    .unwrap()
                                    .unpack()
                                    .unwrap(),
                            );
                        }
                    },
                    if format.contains("encode") || format == "compile_validate" {
                        5
                    } else if count == 100_000 {
                        11
                    } else {
                        21
                    },
                );
                rows.push(Sample {
                    entities: count,
                    profile: profile.into(),
                    format: format.into(),
                    bytes: bytes.len(),
                    median_us,
                    p95_us,
                    samples_us,
                });
                eprintln!(
                    "{count:>6} {profile:>10} {format:>24} {median_us:>10.1} us {size:>10} bytes",
                    size = bytes.len()
                );
            }
        }
    }
    atomic_write(output, &serde_json::to_vec_pretty(&rows)?, expected)?;
    Ok(())
}

fn load_under(root: &Path, path: &str) -> Result<Scene> {
    if !asset_path(path) {
        return Err(Error("Invalid scene resource path".into()));
    }
    let physical = root.join(path).canonicalize()?;
    if !physical.starts_with(root) {
        return Err(Error("Scene resource escapes asset root".into()));
    }
    let bytes = read_limited(&physical, MAX_SOURCE_BYTES)?;
    Scene::from_slice(&bytes)
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("bench") if args.len() == 2 => bench(Path::new(&args[1])),
        Some("cook") if (4..=5).contains(&args.len()) => {
            let root = PathBuf::from(&args[1]).canonicalize()?;
            let output = PathBuf::from(&args[3]);
            if output.extension().is_none_or(|extension| extension != "sceneb") || args.get(4).is_some_and(|flag| flag != "--lz4") { return Err("Cook output must be .sceneb; the optional flag is --lz4".into()); }
            let expected = Revision::read(&output)?;
            let source = load_under(&root, &args[2])?;
            let resolved = resolve(&source, &mut |resource: &Resource| load_under(&root, resource.path.as_deref().ok_or_else(|| Error("Scene needs a path".into()))?))?;
            let mut bytes = cook(&resolved)?;
            if args.get(4).map(String::as_str) == Some("--lz4") { bytes = compress(&bytes)?; }
            atomic_write(&output, &bytes, expected)?;
            Ok(())
        }
        Some("inspect") if args.len() == 2 => {
            let bytes = read_limited(Path::new(&args[1]), 512 * 1024 * 1024)?;
            let decoded = decompress(&bytes)?;
            let view = PackedScene::open(&decoded, None)?;
            println!("{} entities, {} resources, {} bytes", view.entity_count(), view.resource_count(), bytes.len());
            Ok(())
        }
        _ => Err("Usage: scene-format bench OUTPUT.json | cook ASSET_ROOT INPUT.scene OUTPUT.sceneb [--lz4] | inspect FILE.sceneb".into()),
    }
}
