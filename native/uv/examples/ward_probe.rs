#[path = "../src/tree.rs"]
mod tree;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&args[1]).unwrap()).unwrap();
    let vertices: Vec<[f64; 3]> = serde_json::from_value(request["positions"].clone()).unwrap();
    let faces: Vec<[usize; 3]> = serde_json::from_value(request["triangles"].clone()).unwrap();
    let (vertices, faces) = tree::prepare(&vertices, &faces);
    let nodes = tree::build(&vertices, &faces);
    std::fs::write(
        &args[2],
        serde_json::to_vec(
            &serde_json::json!({"positions":vertices,"triangles":faces,"tree":nodes}),
        )
        .unwrap(),
    )
    .unwrap();
}
