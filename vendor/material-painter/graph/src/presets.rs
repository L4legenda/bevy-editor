//! Ready-made material graphs to start from, built from the built-in nodes.

use super::{Graph, Value};

/// A starting graph: name, one-line description and the graph.
pub struct Preset {
    pub name: &'static str,
    pub description: &'static str,
    pub build: fn() -> Graph,
}

pub const PRESETS: &[Preset] = &[
    Preset { name: "Brick Wall", description: "Offset bricks with mortar, color variation and height.", build: bricks },
    Preset { name: "Rusty Metal", description: "Bare steel eaten by patches of rust.", build: rusty_metal },
    Preset { name: "Wood Planks", description: "Grained planks with gaps between them.", build: wood_planks },
    Preset { name: "Stone Tiles", description: "Irregular stones with deep joints.", build: stone_tiles },
    Preset { name: "Scratched Paint", description: "Painted metal with chips and scratches.", build: scratched_paint },
    Preset { name: "Fabric", description: "A woven cloth pattern.", build: fabric },
    Preset { name: "Blank", description: "One color into the base color output.", build: blank },
];

struct B(Graph);

impl B {
    fn node(&mut self, def: &str, x: f32, y: f32, params: &[(&str, Value)]) -> u64 {
        let id = self.0.add(def, [x, y]);
        let n = self.0.node_mut(id).expect("just added");
        for (k, v) in params {
            n.params.insert((*k).to_string(), v.clone());
        }
        id
    }
    fn link(&mut self, from: u64, to: u64, input: &str) {
        self.0.connect(from, to, input);
    }
    /// An Output node for `usage` (0 base color, 1 roughness, 2 metallic, 3 height) fed by `from`.
    fn out(&mut self, from: u64, usage: usize, x: f32, y: f32) {
        let o = self.node("output", x, y, &[("usage", Value::Enum(usage))]);
        self.link(from, o, "input");
    }
}

fn f(v: f32) -> Value {
    Value::Float(v)
}
fn i(v: i32) -> Value {
    Value::Int(v)
}
fn c(r: f32, g: f32, b: f32) -> Value {
    Value::Color([r, g, b])
}
fn e(v: usize) -> Value {
    Value::Enum(v)
}

fn graph(name: &str) -> B {
    B(Graph::new(name))
}

fn bricks() -> Graph {
    let mut g = graph("Brick Wall");
    let t = g.node("tiles", 0.0, 0.0, &[("columns", i(4)), ("rows", i(8)), ("gap", f(0.06)), ("bevel", f(0.05)), ("variation", f(0.35))]);
    let n = g.node("noise", 0.0, 260.0, &[("scale", i(12)), ("octaves", i(6))]);
    let b = g.node("blend", 220.0, 0.0, &[("mode", e(1)), ("opacity", f(0.35))]);
    g.link(n, b, "foreground");
    g.link(t, b, "background");
    let m = g.node("gradient_map", 440.0, 0.0, &[
        ("dark", c(0.25, 0.1, 0.07)),
        ("mid", c(0.55, 0.25, 0.15)),
        ("light", c(0.8, 0.5, 0.35)),
        ("mid_position", f(0.55)),
    ]);
    g.link(b, m, "input");
    g.out(m, 0, 660.0, 0.0);
    let r = g.node("levels", 220.0, 260.0, &[("out_low", f(0.6)), ("out_high", f(0.9))]);
    g.link(n, r, "input");
    g.out(r, 1, 660.0, 220.0);
    g.out(t, 3, 660.0, 440.0);
    g.0
}

fn rusty_metal() -> Graph {
    let mut g = graph("Rusty Metal");
    let n1 = g.node("noise", 0.0, 0.0, &[("scale", i(4)), ("octaves", i(7)), ("roughness", f(0.6)), ("seed", i(3))]);
    let mask = g.node("histogram_scan", 220.0, 0.0, &[("position", f(0.55)), ("contrast", f(0.7))]);
    g.link(n1, mask, "input");
    let steel = g.node("color", 220.0, 220.0, &[("color", c(0.56, 0.57, 0.58))]);
    let n2 = g.node("noise", 0.0, 440.0, &[("scale", i(24)), ("seed", i(9))]);
    let rust = g.node("gradient_map", 220.0, 440.0, &[("dark", c(0.2, 0.07, 0.03)), ("mid", c(0.45, 0.18, 0.07)), ("light", c(0.65, 0.35, 0.15))]);
    g.link(n2, rust, "input");
    let b = g.node("blend", 440.0, 0.0, &[]);
    g.link(rust, b, "foreground");
    g.link(steel, b, "background");
    g.link(mask, b, "mask");
    g.out(b, 0, 660.0, 0.0);
    let r = g.node("levels", 440.0, 260.0, &[("out_low", f(0.35)), ("out_high", f(0.9))]);
    g.link(mask, r, "input");
    g.out(r, 1, 660.0, 220.0);
    let metal = g.node("invert", 440.0, 460.0, &[]);
    g.link(mask, metal, "input");
    g.out(metal, 2, 660.0, 440.0);
    g.0
}

fn wood_planks() -> Graph {
    let mut g = graph("Wood Planks");
    let d = g.node("directional_noise", 0.0, 0.0, &[("scale", i(3)), ("stretch", i(24)), ("octaves", i(5))]);
    let m = g.node("gradient_map", 220.0, 0.0, &[("dark", c(0.25, 0.13, 0.06)), ("mid", c(0.5, 0.3, 0.15)), ("light", c(0.72, 0.5, 0.3))]);
    g.link(d, m, "input");
    let p = g.node("tiles", 0.0, 260.0, &[("columns", i(1)), ("rows", i(6)), ("offset", f(0.0)), ("gap", f(0.03)), ("bevel", f(0.02)), ("variation", f(0.25))]);
    let b = g.node("blend", 440.0, 0.0, &[("mode", e(1))]);
    g.link(p, b, "foreground");
    g.link(m, b, "background");
    g.out(b, 0, 660.0, 0.0);
    let r = g.node("levels", 220.0, 260.0, &[("out_low", f(0.45)), ("out_high", f(0.7))]);
    g.link(d, r, "input");
    g.out(r, 1, 660.0, 220.0);
    g.out(p, 3, 660.0, 440.0);
    g.0
}

fn stone_tiles() -> Graph {
    let mut g = graph("Stone Tiles");
    let cells = g.node("cells", 0.0, 0.0, &[("mode", e(2)), ("scale", i(6)), ("jitter", f(0.8))]);
    let edges = g.node("cells", 0.0, 260.0, &[("mode", e(1)), ("scale", i(6)), ("jitter", f(0.8))]);
    let n = g.node("noise", 0.0, 520.0, &[("scale", i(16))]);
    let v = g.node("blend", 220.0, 0.0, &[("mode", e(3)), ("opacity", f(0.5))]);
    g.link(n, v, "foreground");
    g.link(cells, v, "background");
    let m = g.node("gradient_map", 440.0, 0.0, &[("dark", c(0.25, 0.24, 0.23)), ("mid", c(0.45, 0.44, 0.42)), ("light", c(0.7, 0.68, 0.65))]);
    g.link(v, m, "input");
    let b = g.node("blend", 660.0, 0.0, &[("mode", e(1))]);
    g.link(edges, b, "foreground");
    g.link(m, b, "background");
    g.out(b, 0, 880.0, 0.0);
    let r = g.node("levels", 220.0, 520.0, &[("out_low", f(0.6)), ("out_high", f(0.85))]);
    g.link(n, r, "input");
    g.out(r, 1, 880.0, 220.0);
    g.out(edges, 3, 880.0, 440.0);
    g.0
}

fn scratched_paint() -> Graph {
    let mut g = graph("Scratched Paint");
    let paint = g.node("color", 0.0, 0.0, &[("color", c(0.15, 0.35, 0.6))]);
    let metal = g.node("color", 0.0, 200.0, &[("color", c(0.6, 0.6, 0.62))]);
    let sc = g.node("scratches", 0.0, 400.0, &[("count", i(150)), ("length", f(0.25)), ("width", f(1.2))]);
    let n = g.node("noise", 0.0, 660.0, &[("scale", i(5)), ("seed", i(4))]);
    let chips = g.node("histogram_scan", 220.0, 660.0, &[("position", f(0.7)), ("contrast", f(0.9))]);
    g.link(n, chips, "input");
    let wear = g.node("math", 440.0, 400.0, &[("op", e(5))]);
    g.link(sc, wear, "a");
    g.link(chips, wear, "b");
    let b = g.node("blend", 660.0, 0.0, &[]);
    g.link(metal, b, "foreground");
    g.link(paint, b, "background");
    g.link(wear, b, "mask");
    g.out(b, 0, 880.0, 0.0);
    let r = g.node("levels", 660.0, 260.0, &[("out_low", f(0.45)), ("out_high", f(0.25))]);
    g.link(wear, r, "input");
    g.out(r, 1, 880.0, 220.0);
    g.out(wear, 2, 880.0, 440.0);
    g.0
}

fn fabric() -> Graph {
    let mut g = graph("Fabric");
    let h = g.node("directional_noise", 0.0, 0.0, &[("scale", i(8)), ("stretch", i(32))]);
    let v = g.node("directional_noise", 0.0, 260.0, &[("direction", e(1)), ("scale", i(8)), ("stretch", i(32)), ("seed", i(5))]);
    let weave = g.node("checker", 0.0, 520.0, &[("count", i(64))]);
    let w = g.node("blend", 220.0, 0.0, &[]);
    g.link(h, w, "foreground");
    g.link(v, w, "background");
    g.link(weave, w, "mask");
    let m = g.node("gradient_map", 440.0, 0.0, &[("dark", c(0.15, 0.05, 0.08)), ("mid", c(0.45, 0.12, 0.18)), ("light", c(0.75, 0.3, 0.35))]);
    g.link(w, m, "input");
    g.out(m, 0, 660.0, 0.0);
    let r = g.node("levels", 440.0, 260.0, &[("out_low", f(0.75)), ("out_high", f(0.95))]);
    g.link(w, r, "input");
    g.out(r, 1, 660.0, 220.0);
    g.out(w, 3, 660.0, 440.0);
    g.0
}

fn blank() -> Graph {
    let mut g = graph("Blank");
    let col = g.node("color", 0.0, 0.0, &[("color", c(0.6, 0.6, 0.6))]);
    g.out(col, 0, 220.0, 0.0);
    g.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_use_known_nodes_and_parameters() {
        let lib = super::super::Library::load(None);
        for p in PRESETS {
            let g = (p.build)();
            assert!(g.outputs(&lib)[0].is_some(), "{} has no base color output", p.name);
            for n in &g.nodes {
                let def = lib.find(&n.def).unwrap_or_else(|| panic!("{}: unknown node {}", p.name, n.def));
                for k in n.params.keys() {
                    let param = def.params.iter().find(|q| &q.name == k).unwrap_or_else(|| panic!("{}: {} has no {k}", p.name, n.def));
                    assert_eq!(
                        std::mem::discriminant(&param.default),
                        std::mem::discriminant(&n.params[k]),
                        "{}: {}.{k} has the wrong kind",
                        p.name,
                        n.def
                    );
                }
                for input in n.links.keys() {
                    assert!(def.inputs.iter().any(|i| &i.name == input), "{}: {} has no input {input}", p.name, n.def);
                }
            }
        }
    }
}
