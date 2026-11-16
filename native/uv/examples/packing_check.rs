#[path = "../tests/support/packing.rs"]
mod packing;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let read = |p: &str| serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
    let report = packing::check(&read(&args[1]), &read(&args[2]));
    let text = serde_json::to_string_pretty(&report).unwrap();
    println!("{text}");
    if args.len() > 3 {
        std::fs::write(&args[3], text).unwrap();
    }
}
