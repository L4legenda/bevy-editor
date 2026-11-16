#[path = "support/packing.rs"]
mod packing;
use serde_json::{Value, json};
fn run(request: &Value) -> Value {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "uv-contour-{}-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("input.json");
    let output = dir.join("output.json");
    std::fs::write(&input, serde_json::to_vec(request).unwrap()).unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_bevy-uv-native"))
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&result.stderr).contains("Contour packing fallback"),
        "ordinary islands must pack by contour"
    );
    serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap()
}
#[test]
fn concave_islands_keep_shape_density_and_seams_without_overlap() {
    // Small squares must fit inside the empty quadrant of an L-shaped island.
    // A bounding-rectangle fallback reserves this space and cannot reach 65%.
    let shape = [[0., 0.], [4., 0.], [4., 1.], [1., 1.], [1., 4.], [0., 4.]];
    let base = [[0usize, 1, 3], [1, 2, 3], [0, 3, 5], [3, 4, 5]];
    let mut positions = Vec::new();
    let mut faces = Vec::new();
    let mut uv = Vec::new();
    for _ in 0..1 {
        let offset = positions.len();
        positions.extend(shape.map(|p| [p[0], p[1], 0.]));
        for f in base {
            faces.push(f.map(|v| v + offset));
            uv.push(f.map(|v| shape[v]));
        }
    }
    let square = [[0., 0.], [1.3, 0.], [1.3, 1.3], [0., 1.3]];
    for _ in 0..4 {
        let offset = positions.len();
        positions.extend(square.map(|p| [p[0], p[1], 0.]));
        for f in [[0usize, 1, 2], [0, 2, 3]] {
            faces.push(f.map(|v| v + offset));
            uv.push(f.map(|v| square[v]));
        }
    }
    let request = json!({"positions":positions,"triangles":faces,"existing_uv":uv,"resolution":1024,"padding":8,"threshold":1.25});
    let output = run(&request);
    let check = packing::check(&request, &output);
    assert!(
        check["uv_area_after"].as_f64().unwrap() > 0.65,
        "concave packing remains too sparse: {check}"
    );
}
#[test]
fn sliver_connectors_and_tiny_islands_are_reserved_and_never_split() {
    let uv = [
        [0., 0.],
        [1., 0.],
        [1., 1.],
        [0., 1.],
        [1.5, 1e-10],
        [0., 0.],
        [0.4, 0.],
        [0.4, 0.2],
        [0., 0.2],
        [0., 0.],
        [0.0001, 0.],
        [0.0001, 0.1],
        [0., 0.1],
    ];
    let faces = [
        [0usize, 1, 2],
        [0, 2, 3],
        [0, 4, 1],
        [5, 6, 7],
        [5, 7, 8],
        [9, 10, 11],
        [9, 11, 12],
    ];
    let positions = uv.map(|p| [p[0], p[1], 0.]);
    let existing = faces.map(|f| f.map(|i| uv[i]));
    let request = json!({"positions":positions,"triangles":faces,"existing_uv":existing,"resolution":1024,"padding":8,"threshold":1.25});
    packing::check(&request, &run(&request));
}

#[test]
fn small_island_far_from_origin_keeps_its_shape() {
    // Exact float32 coordinates: translation must not degrade rotation precision.
    let uv = [
        [0., 0.],
        [1., 0.],
        [1., 1.],
        [0., 1.],
        [10000., 10000.],
        [10000.125, 10000.],
        [10000.125, 10000.001953125],
        [10000., 10000.001953125],
    ];
    let faces = [[0usize, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]];
    let request = json!({"positions":uv.map(|p|[p[0],p[1],0.]),
        "triangles":faces,"existing_uv":faces.map(|f|f.map(|i|uv[i])),
        "resolution":2048,"padding":8,"threshold":1.25});
    packing::check(&request, &run(&request));
}
