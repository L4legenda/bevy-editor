//! Optional GPU acceptance fixture for scene-folder first visits and cache reuse.
#![cfg(not(target_arch = "wasm32"))]

use std::{
    collections::HashSet,
    path::Path,
    time::{Duration, Instant},
};

use bevy::prelude::*;

use super::{SETTLE, Target, Thumbnails};
use crate::editor::{
    Editor,
    browser::{Browser, Entry},
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::fixture,
};

struct Sample {
    elapsed: Duration,
    stage: String,
}

fn stage(app: &App) -> String {
    let previews = app.world().resource::<Thumbnails>();
    match previews.job.as_ref() {
        None => "idle".into(),
        Some(job) if job.instances.is_empty() => format!("load {:?}", job.target),
        Some(job) if job.camera.is_none() => format!("prepare {:?}", job.target),
        Some(job) => format!("render {:?}, stable={}", job.target, job.stable_frames),
    }
}

fn measured_frame(app: &mut App, samples: &mut Vec<Sample>) {
    let before = stage(app);
    let started = Instant::now();
    frame(app);
    let elapsed = started.elapsed();
    samples.push(Sample {
        elapsed,
        stage: format!("{before} -> {}", stage(app)),
    });
}

fn report(label: &str, samples: &[Sample]) {
    let mut sorted: Vec<_> = samples.iter().collect();
    sorted.sort_unstable_by_key(|sample| sample.elapsed);
    let millis = |sample: &Sample| sample.elapsed.as_secs_f64() * 1000.0;
    let percentile = |percent: usize| millis(sorted[(sorted.len() - 1) * percent / 100]);
    println!(
        "THUMB_PERF {label}: count={} p50={:.2}ms p95={:.2}ms max={:.2}ms total={:.2}ms",
        samples.len(),
        percentile(50),
        percentile(95),
        millis(sorted[sorted.len() - 1]),
        samples.iter().map(millis).sum::<f64>(),
    );
    for sample in sorted.iter().rev().take(5) {
        println!(
            "THUMB_PERF {label} slow {:.2}ms: {}",
            millis(sample),
            sample.stage
        );
    }
}

fn completed(app: &App, scenes: &HashSet<Target>) -> usize {
    app.world()
        .resource::<Thumbnails>()
        .completed
        .intersection(scenes)
        .count()
}

#[test]
#[ignore = "requires GPU adapter; reports real editor scene-folder latency"]
fn gpu_scene_folder_preview_latency() {
    let (mut app, _, canvas) = fixture();
    let (folder, scenes) = {
        let editor = app.world().resource::<Editor>();
        let browser = app.world().resource::<Browser>();
        let folder = browser
            .folders
            .iter()
            .position(|folder| folder.path == Path::new("scenes"))
            .expect("The bundled example must have a scenes folder");
        let scenes = browser
            .entries(&editor.project.catalog)
            .iter()
            .filter_map(|entry| match entry {
                Entry::Scene { room, .. } => Some(Target::Scene(*room)),
                _ => None,
            })
            .collect::<HashSet<_>>();
        (folder, scenes)
    };
    // The root lists folders rather than their children; gather the scene IDs
    // from the project before changing browser state for the measured visit.
    let scenes = if scenes.is_empty() {
        (0..app.world().resource::<Editor>().project.scenes.len())
            .map(Target::Scene)
            .collect()
    } else {
        scenes
    };
    assert_eq!(scenes.len(), 2, "Expected the two bundled example scenes");

    let mut baseline = Vec::new();
    for _ in 0..32 {
        measured_frame(&mut app, &mut baseline);
    }
    report("baseline", &baseline);

    let prewarmed: HashSet<_> = app
        .world()
        .resource::<Thumbnails>()
        .completed
        .intersection(&scenes)
        .copied()
        .collect();
    println!("THUMB_PERF prewarmed={}", prewarmed.len());
    let studio_view = app
        .world()
        .resource::<Thumbnails>()
        .view
        .expect("Studio camera must be prepared before browsing")
        .entity;
    assert!(app.world().resource::<Thumbnails>().warmed_up);
    assert!(!app.world().get::<Camera>(studio_view).unwrap().is_active);

    app.world_mut().resource_mut::<Editor>().drawer = true;
    assert!(app.world_mut().resource_mut::<Browser>().navigate(folder));
    let started = Instant::now();
    let mut first_visit = Vec::new();
    let mut rendered = prewarmed;
    for _ in 0..1200 {
        // Exclude the intentional edit-debounce delay from rendering latency.
        app.world_mut()
            .resource_mut::<Thumbnails>()
            .changed
            .fill(Instant::now() - SETTLE);
        measured_frame(&mut app, &mut first_visit);
        if let Some(job) = &app.world().resource::<Thumbnails>().job
            && job.camera.is_some()
        {
            rendered.insert(job.target);
        }
        if completed(&app, &scenes) == scenes.len() {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "Scene thumbnails did not finish within two minutes"
        );
    }
    report("first_visit", &first_visit);
    println!(
        "THUMB_PERF first_visit completed={}/{} rendered={}",
        completed(&app, &scenes),
        scenes.len(),
        rendered.len()
    );
    assert_eq!(
        completed(&app, &scenes),
        scenes.len(),
        "Both previews must complete"
    );
    assert!(
        scenes.is_subset(&rendered),
        "Timed-out/error placeholders must not count as rendered previews"
    );
    assert_eq!(
        app.world().resource::<Thumbnails>().view.unwrap().entity,
        studio_view,
        "Plain scene thumbnails must reuse the prepared studio camera"
    );
    assert!(!app.world().get::<Camera>(studio_view).unwrap().is_active);

    assert!(app.world_mut().resource_mut::<Browser>().navigate(0));
    frame(&mut app);
    assert!(app.world_mut().resource_mut::<Browser>().navigate(folder));
    let mut revisit = Vec::new();
    for _ in 0..32 {
        measured_frame(&mut app, &mut revisit);
    }
    report("cached_revisit", &revisit);
    assert_eq!(completed(&app, &scenes), scenes.len());
    assert!(
        app.world().resource::<Thumbnails>().job.is_none(),
        "A cached revisit must not start another thumbnail render"
    );

    let output = std::env::temp_dir().join(format!(
        "bevy-editor-thumbnail-performance-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&output).unwrap();
    capture_when(&mut app, &canvas, &output.join("scenes-folder.png"), |_| {
        true
    });
    println!(
        "THUMB_PERF screenshot: {}",
        output.join("scenes-folder.png").display()
    );
}
