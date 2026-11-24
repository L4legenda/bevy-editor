//! Editable layer recipes from Material Painter's built-in smart material shelf.
use super::materials::{MATERIALS, MaterialPreset};
use super::{Blend, Generator, GeneratorKind, Layer, LayerKind, Mask, texture_fill};
use crate::editor::model_paint_data::solid;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// A smart layer's mask: a generator preset with optional overrides, so each
/// layer can be tuned the way an artist would tune it in the mask panel.
#[derive(Clone, Copy)]
struct SmartMask {
    kind: GeneratorKind,
    invert: bool,
    /// Input weights; when non-empty they replace the preset's weights.
    weights: &'static [(usize, f32)],
    level: Option<f32>,
    contrast: Option<f32>,
    scale: Option<f32>,
    noise: Option<f32>,
    edge_width: Option<f32>,
    seed: f32,
}

const fn mask(kind: GeneratorKind) -> SmartMask {
    SmartMask {
        kind,
        invert: false,
        weights: &[],
        level: None,
        contrast: None,
        scale: None,
        noise: None,
        edge_width: None,
        seed: 0.0,
    }
}

/// Mask from explicit input weights, with level and contrast.
const fn mix(
    kind: GeneratorKind,
    weights: &'static [(usize, f32)],
    level: f32,
    contrast: f32,
) -> SmartMask {
    SmartMask {
        weights,
        level: Some(level),
        contrast: Some(contrast),
        ..mask(kind)
    }
}

pub const COLOR_ONLY: [bool; 4] = [true, false, false, false];
pub const HEIGHT_ONLY: [bool; 4] = [false, false, false, true];
pub const ROUGH_ONLY: [bool; 4] = [false, true, false, false];
// (Smart layers name their channels among the first four; opacity stays off.)

struct SmartLayer {
    name: &'static str,
    material: &'static str,
    /// Overrides of the material's values (color in sRGB).
    color: Option<[f32; 3]>,
    rough: Option<f32>,
    metal: Option<f32>,
    /// Fill height; enables the height channel.
    height: Option<f32>,
    color_var: Option<f32>,
    rough_var: Option<f32>,
    height_var: Option<f32>,
    var_scale: Option<f32>,
    blend: Blend,
    opacity: f32,
    mask: Option<SmartMask>,
    /// Channels the layer writes, when not the material's own.
    channels: Option<[bool; 4]>,
    /// Per-part brightness/roughness variation.
    part_var: f32,
    seed: f32,
    /// A scanned PBR set whose images replace the layer's values.
    scan: Option<Scan>,
}

/// A photo-scanned PBR texture set in `assets/pbr/<id>` (color.jpg,
/// rough.jpg, height.jpg, optional metal.png), projected tri-planar.
#[derive(Clone, Copy)]
struct Scan {
    id: &'static str,
    /// Repeats per world unit (meshes are about two units across).
    tiling: f32,
    /// Height strength of the scan's displacement map.
    height: f32,
}

impl Scan {
    const FILES: [(usize, &'static str); 4] = [
        (0, "color.jpg"),
        (1, "rough.jpg"),
        (2, "metal.png"),
        (3, "height.jpg"),
    ];
}

pub(super) struct SmartMaterial {
    pub name: &'static str,
    layers: &'static [SmartLayer],
}

const fn layer(name: &'static str, material: &'static str) -> SmartLayer {
    SmartLayer {
        name,
        material,
        color: None,
        rough: None,
        metal: None,
        height: None,
        color_var: None,
        rough_var: None,
        height_var: None,
        var_scale: None,
        blend: Blend::Normal,
        opacity: 1.0,
        mask: None,
        channels: None,
        part_var: 0.0,
        seed: 0.0,
        scan: None,
    }
}

/// A base layer from a scanned set; `material` supplies the values used
/// where the scan has no map (metalness of non-metals) or is missing.
const fn scan(
    name: &'static str,
    material: &'static str,
    id: &'static str,
    tiling: f32,
    height: f32,
) -> SmartLayer {
    SmartLayer {
        scan: Some(Scan { id, tiling, height }),
        ..layer(name, material)
    }
}

mod gin {
    pub const EDGE: usize = 0;
    pub const CONVEX: usize = 1;
    pub const CONCAVE: usize = 2;
    pub const OCCLUSION: usize = 3;
    pub const CAVITY: usize = 4;
    pub const SKY: usize = 5;
    pub const UP: usize = 6;
    pub const THIN: usize = 7;
    pub const PART_GRADIENT: usize = 8;
    pub const WORLD_HEIGHT: usize = 9;
    pub const CREASE: usize = 11;
    pub const STREAKS: usize = 12;
    pub const NOISE: usize = 13;
    pub const OVERHANG: usize = 15;
    pub const GRAIN: usize = 16;
    pub const CELLS: usize = 17;
    pub const SCRATCHES: usize = 18;
    pub const FRONT: usize = 19;
}

use gin::*;

// Shared masks. Meshes are normalized to about two units across on import,
// so scales and edge widths here are relative to the whole object.
const GRIME: SmartMask = SmartMask {
    noise: Some(0.5),
    edge_width: Some(0.03),
    ..mix(
        GeneratorKind::Dirt,
        &[(OCCLUSION, 2.5), (CAVITY, 1.5), (CREASE, 1.5)],
        0.6,
        0.2,
    )
};
const DUST: SmartMask = SmartMask {
    noise: Some(0.8),
    scale: Some(4.0),
    ..mix(
        GeneratorKind::Dust,
        &[(SKY, 0.8), (UP, 0.6), (OCCLUSION, 0.3)],
        1.3,
        0.3,
    )
};
const POLISHED_EDGES: SmartMask = SmartMask {
    noise: Some(0.6),
    edge_width: Some(0.006),
    ..mix(
        GeneratorKind::EdgeWear,
        &[(EDGE, 1.0), (CONVEX, 0.5)],
        0.55,
        0.1,
    )
};
const SCRATCHED: SmartMask = SmartMask {
    scale: Some(2.0),
    noise: Some(0.0),
    ..mix(GeneratorKind::Clouds, &[(SCRATCHES, 1.0)], 0.22, 0.12)
};
/// Chips: edges plus blotches of noise, crisp.
const CHIPS: SmartMask = SmartMask {
    noise: Some(0.9),
    scale: Some(5.0),
    edge_width: Some(0.012),
    ..mix(
        GeneratorKind::EdgeWear,
        &[(EDGE, 1.0), (CONVEX, 0.6), (NOISE, 0.35)],
        0.72,
        0.04,
    )
};
const DUST_LAYER: SmartLayer = SmartLayer {
    color: Some([0.52, 0.48, 0.42]),
    rough: Some(0.95),
    metal: Some(0.0),
    opacity: 0.4,
    mask: Some(DUST),
    ..layer("Dust", "Concrete")
};
const GRIME_LAYER: SmartLayer = SmartLayer {
    color: Some([0.10, 0.08, 0.055]),
    rough: Some(0.9),
    metal: Some(0.0),
    opacity: 0.6,
    mask: Some(GRIME),
    ..layer("Grime", "Concrete")
};
/// Fine surface detail every smart material gets on top: roughness overlaid
/// around 0.5 (varies what's below without flattening it) and a little
/// height added, at a scale far finer than the wear and dirt masks.
const MICRO_DETAIL: [SmartLayer; 2] = [
    SmartLayer {
        rough: Some(0.5),
        rough_var: Some(0.14),
        var_scale: Some(40.0),
        channels: Some(ROUGH_ONLY),
        blend: Blend::Overlay,
        ..layer("Micro Roughness", "Concrete")
    },
    SmartLayer {
        height: Some(0.0),
        height_var: Some(0.025),
        var_scale: Some(55.0),
        channels: Some(HEIGHT_ONLY),
        blend: Blend::Add,
        ..layer("Micro Height", "Concrete")
    },
];

/// Lighter grime for scanned sets, which already carry their own dirt.
const SCAN_GRIME: SmartLayer = SmartLayer {
    color: Some([0.12, 0.09, 0.06]),
    opacity: 0.4,
    ..GRIME_LAYER
};

// Skin: shared layers. Blood under the skin shows where it's thin or creased.
const SKIN_FLUSH: SmartLayer = SmartLayer {
    color: Some([0.78, 0.40, 0.34]),
    opacity: 0.45,
    channels: Some(COLOR_ONLY),
    mask: Some(SmartMask {
        noise: Some(0.6),
        scale: Some(3.0),
        ..mix(
            GeneratorKind::Clouds,
            &[(NOISE, 0.7), (THIN, 0.6), (CONCAVE, 0.8)],
            0.55,
            0.25,
        )
    }),
    ..layer("Flush", "Skin")
};
/// Faint bluish veins.
const SKIN_VEINS: SmartLayer = SmartLayer {
    color: Some([0.52, 0.55, 0.66]),
    opacity: 0.18,
    channels: Some(COLOR_ONLY),
    mask: Some(SmartMask {
        scale: Some(1.5),
        noise: Some(0.2),
        ..mix(
            GeneratorKind::Clouds,
            &[(SCRATCHES, 1.0), (THIN, 0.4)],
            0.35,
            0.12,
        )
    }),
    ..layer("Veins", "Skin")
};
/// Pores: tiny pits, a little rougher.
const SKIN_PORES: SmartLayer = SmartLayer {
    height: Some(-0.12),
    rough: Some(0.55),
    opacity: 0.8,
    channels: Some([false, true, false, true]),
    mask: Some(SmartMask {
        scale: Some(40.0),
        noise: Some(0.0),
        invert: true,
        ..mix(GeneratorKind::Clouds, &[(CELLS, 1.0)], 0.15, 0.05)
    }),
    ..layer("Pores", "Skin")
};
/// Darker skin in folds and creases.
const SKIN_FOLDS: SmartLayer = SmartLayer {
    color: Some([0.55, 0.36, 0.29]),
    opacity: 0.4,
    channels: Some(COLOR_ONLY),
    mask: Some(SmartMask {
        noise: Some(0.3),
        edge_width: Some(0.01),
        ..mix(
            GeneratorKind::CreaseGrime,
            &[(CAVITY, 1.5), (CREASE, 1.0), (OCCLUSION, 0.8)],
            0.5,
            0.2,
        )
    }),
    ..layer("Folds", "Skin")
};
/// Oil: shinier on the high points (nose, cheekbones, forehead).
const SKIN_SHEEN: SmartLayer = SmartLayer {
    rough: Some(0.28),
    opacity: 0.7,
    channels: Some(ROUGH_ONLY),
    mask: Some(SmartMask {
        noise: Some(0.4),
        scale: Some(2.0),
        ..mix(
            GeneratorKind::Clouds,
            &[(CONVEX, 1.5), (SKY, 0.4), (NOISE, 0.3)],
            0.55,
            0.2,
        )
    }),
    ..layer("Oily Sheen", "Skin")
};

// Eyes: a sphere looking down +Z. The Front distance input (the angle from
// the front) draws the rings: iris, limbal ring, pupil, cornea.
const fn eye_ring(level: f32, contrast: f32) -> SmartMask {
    SmartMask {
        invert: true,
        noise: Some(0.0),
        ..mix(GeneratorKind::Clouds, &[(FRONT, 1.0)], level, contrast)
    }
}
const EYE_SCLERA: SmartLayer = SmartLayer {
    color: Some([0.93, 0.90, 0.86]),
    rough: Some(0.12),
    color_var: Some(0.03),
    var_scale: Some(6.0),
    ..layer("Sclera", "Porcelain")
};
/// Pinker toward the back.
const EYE_BACK: SmartLayer = SmartLayer {
    color: Some([0.90, 0.72, 0.68]),
    opacity: 0.5,
    channels: Some(COLOR_ONLY),
    mask: Some(SmartMask {
        noise: Some(0.2),
        scale: Some(3.0),
        ..mix(GeneratorKind::Clouds, &[(FRONT, 1.0)], 0.5, 0.2)
    }),
    ..layer("Pink Back", "Porcelain")
};
/// Veins, more of them toward the sides.
const EYE_VEINS: SmartLayer = SmartLayer {
    color: Some([0.72, 0.14, 0.12]),
    opacity: 0.85,
    channels: Some(COLOR_ONLY),
    mask: Some(SmartMask {
        scale: Some(1.2),
        noise: Some(0.0),
        ..mix(
            GeneratorKind::Clouds,
            &[(SCRATCHES, 1.6), (FRONT, 0.7)],
            0.5,
            0.1,
        )
    }),
    ..layer("Veins", "Porcelain")
};
/// The dark band round the iris.
const EYE_LIMBAL: SmartLayer = SmartLayer {
    color: Some([0.08, 0.09, 0.10]),
    channels: Some(COLOR_ONLY),
    mask: Some(eye_ring(0.165, 0.012)),
    ..layer("Limbal Ring", "Porcelain")
};
/// The iris: fibrous color, set a little back.
const EYE_IRIS: SmartLayer = SmartLayer {
    color: Some([0.25, 0.45, 0.62]),
    color_var: Some(0.35),
    var_scale: Some(40.0),
    height: Some(-0.15),
    channels: Some([true, false, false, true]),
    mask: Some(eye_ring(0.145, 0.008)),
    ..layer("Iris", "Porcelain")
};
/// A lighter ring round the pupil.
const EYE_INNER: SmartLayer = SmartLayer {
    color: Some([0.55, 0.50, 0.30]),
    opacity: 0.5,
    channels: Some(COLOR_ONLY),
    mask: Some(eye_ring(0.085, 0.03)),
    ..layer("Inner Iris", "Porcelain")
};
const EYE_PUPIL: SmartLayer = SmartLayer {
    color: Some([0.01, 0.01, 0.012]),
    channels: Some(COLOR_ONLY),
    mask: Some(eye_ring(0.055, 0.006)),
    ..layer("Pupil", "Porcelain")
};
/// The cornea: glassy over the iris.
const EYE_CORNEA: SmartLayer = SmartLayer {
    rough: Some(0.02),
    channels: Some(ROUGH_ONLY),
    mask: Some(eye_ring(0.2, 0.03)),
    ..layer("Cornea", "Porcelain")
};
const EYE_BLUE: [SmartLayer; 8] = [
    EYE_SCLERA, EYE_BACK, EYE_VEINS, EYE_LIMBAL, EYE_IRIS, EYE_INNER, EYE_PUPIL, EYE_CORNEA,
];
const EYE_BROWN: [SmartLayer; 8] = [
    EYE_SCLERA,
    EYE_BACK,
    EYE_VEINS,
    EYE_LIMBAL,
    SmartLayer {
        color: Some([0.33, 0.18, 0.08]),
        ..EYE_IRIS
    },
    SmartLayer {
        color: Some([0.50, 0.32, 0.12]),
        ..EYE_INNER
    },
    EYE_PUPIL,
    EYE_CORNEA,
];

pub(super) const SMART_MATERIALS: &[SmartMaterial] = &[
    SmartMaterial {
        name: "Oak Aged",
        layers: &[
            SmartLayer {
                part_var: 0.15,
                ..layer("Oak", "Oak")
            },
            SmartLayer {
                color: Some([0.17, 0.09, 0.045]),
                rough: Some(0.8),
                height: Some(-0.25),
                opacity: 0.65,
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    noise: Some(0.1),
                    ..mix(GeneratorKind::Clouds, &[(GRAIN, 1.0)], 0.45, 0.2)
                }),
                ..layer("Grain", "Oak")
            },
            SmartLayer {
                color: Some([0.52, 0.40, 0.28]),
                rough: Some(0.85),
                opacity: 0.35,
                mask: Some(SmartMask {
                    scale: Some(1.5),
                    noise: Some(0.0),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(NOISE, 0.6), (SKY, 0.6)],
                        0.6,
                        0.25,
                    )
                }),
                ..layer("Sun Bleach", "Oak")
            },
            SmartLayer {
                color: Some([0.55, 0.40, 0.26]),
                rough: Some(0.6),
                opacity: 0.6,
                mask: Some(SmartMask {
                    edge_width: Some(0.008),
                    noise: Some(0.7),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 1.0), (CONVEX, 0.5)],
                        0.6,
                        0.1,
                    )
                }),
                ..layer("Worn Edges", "Oak")
            },
            GRIME_LAYER,
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Painted Wood Chipped",
        layers: &[
            SmartLayer {
                color: Some([0.40, 0.27, 0.16]),
                ..layer("Pine", "Oak")
            },
            SmartLayer {
                color: Some([0.22, 0.13, 0.07]),
                height: Some(-0.2),
                opacity: 0.6,
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    noise: Some(0.1),
                    ..mix(GeneratorKind::Clouds, &[(GRAIN, 1.0)], 0.45, 0.2)
                }),
                ..layer("Grain", "Oak")
            },
            SmartLayer {
                color: Some([0.20, 0.30, 0.32]),
                rough: Some(0.55),
                height: Some(0.08),
                color_var: Some(0.1),
                rough_var: Some(0.2),
                part_var: 0.1,
                mask: Some(SmartMask {
                    invert: true,
                    ..CHIPS
                }),
                ..layer("Paint", "Paint Olive")
            },
            SmartLayer {
                color: Some([0.72, 0.70, 0.62]),
                rough: Some(0.9),
                opacity: 0.4,
                channels: Some([true, true, false, false]),
                mask: Some(SmartMask {
                    scale: Some(2.0),
                    noise: Some(0.4),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(SKY, 0.7), (NOISE, 0.6)],
                        0.75,
                        0.15,
                    )
                }),
                ..layer("Sun Fade", "Paint Olive")
            },
            GRIME_LAYER,
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Brass Aged",
        layers: &[
            SmartLayer {
                part_var: 0.1,
                ..layer("Brass", "Brass")
            },
            SmartLayer {
                color: Some([0.20, 0.17, 0.10]),
                rough: Some(0.8),
                metal: Some(0.3),
                opacity: 0.85,
                mask: Some(SmartMask {
                    scale: Some(4.0),
                    noise: Some(0.6),
                    edge_width: Some(0.03),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 1.5), (CAVITY, 1.5), (CREASE, 1.0), (NOISE, 1.2)],
                        0.5,
                        0.25,
                    )
                }),
                ..layer("Tarnish", "Brass")
            },
            SmartLayer {
                color: Some([0.30, 0.52, 0.42]),
                rough: Some(0.85),
                metal: Some(0.0),
                height: Some(0.05),
                opacity: 0.9,
                mask: Some(SmartMask {
                    noise: Some(0.7),
                    scale: Some(6.0),
                    ..mix(
                        GeneratorKind::CavityGrime,
                        &[(CAVITY, 2.0), (CREASE, 1.0)],
                        0.75,
                        0.12,
                    )
                }),
                ..layer("Verdigris", "Concrete")
            },
            SmartLayer {
                color: Some([0.82, 0.68, 0.44]),
                rough: Some(0.25),
                opacity: 0.6,
                mask: Some(SCRATCHED),
                ..layer("Scratches", "Brass")
            },
            SmartLayer {
                color: Some([0.88, 0.74, 0.48]),
                rough: Some(0.22),
                opacity: 0.85,
                mask: Some(POLISHED_EDGES),
                ..layer("Polished Edges", "Brass")
            },
        ],
    },
    SmartMaterial {
        name: "Gold Tarnished",
        layers: &[
            SmartLayer {
                color: Some([0.92, 0.72, 0.36]),
                rough_var: Some(0.25),
                var_scale: Some(5.0),
                ..layer("Gold", "Gold")
            },
            SmartLayer {
                color: Some([0.40, 0.28, 0.12]),
                rough: Some(0.6),
                opacity: 0.8,
                mask: Some(SmartMask {
                    noise: Some(0.5),
                    edge_width: Some(0.03),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 1.5), (CAVITY, 1.5), (NOISE, 0.8)],
                        0.55,
                        0.2,
                    )
                }),
                ..layer("Tarnish", "Gold")
            },
            SmartLayer {
                color: Some([1.0, 0.82, 0.45]),
                rough: Some(0.15),
                opacity: 0.5,
                mask: Some(SCRATCHED),
                ..layer("Scratches", "Gold")
            },
            SmartLayer {
                rough: Some(0.1),
                mask: Some(POLISHED_EDGES),
                ..layer("Polished Edges", "Gold")
            },
            SmartLayer {
                opacity: 0.6,
                ..GRIME_LAYER
            },
        ],
    },
    SmartMaterial {
        name: "Silver Tarnished",
        layers: &[
            layer("Silver", "Silver"),
            SmartLayer {
                rough: Some(0.32),
                opacity: 0.6,
                channels: Some(ROUGH_ONLY),
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    noise: Some(0.3),
                    ..mix(GeneratorKind::Clouds, &[(NOISE, 1.0)], 0.5, 0.2)
                }),
                ..layer("Smudges", "Silver")
            },
            SmartLayer {
                color: Some([0.09, 0.08, 0.07]),
                rough: Some(0.55),
                metal: Some(0.6),
                mask: Some(SmartMask {
                    noise: Some(0.45),
                    edge_width: Some(0.03),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 2.0), (CAVITY, 2.0), (CREASE, 1.0)],
                        0.5,
                        0.2,
                    )
                }),
                ..layer("Black Tarnish", "Silver")
            },
            SmartLayer {
                rough: Some(0.12),
                opacity: 0.6,
                mask: Some(SCRATCHED),
                ..layer("Scratches", "Silver")
            },
            SmartLayer {
                rough: Some(0.08),
                mask: Some(POLISHED_EDGES),
                ..layer("Polished Edges", "Silver")
            },
        ],
    },
    SmartMaterial {
        name: "Iron Forged",
        layers: &[
            SmartLayer {
                part_var: 0.12,
                ..layer("Iron", "Iron Dark")
            },
            SmartLayer {
                height: Some(-0.12),
                blend: Blend::Add,
                channels: Some(HEIGHT_ONLY),
                mask: Some(SmartMask {
                    invert: true,
                    scale: Some(1.5),
                    noise: Some(0.0),
                    ..mix(GeneratorKind::Clouds, &[(CELLS, 1.0)], 0.5, 0.35)
                }),
                ..layer("Hammer Dents", "Iron Dark")
            },
            SmartLayer {
                color: Some([0.17, 0.17, 0.19]),
                rough: Some(0.7),
                opacity: 0.8,
                mask: Some(SmartMask {
                    scale: Some(2.5),
                    noise: Some(0.3),
                    ..mix(GeneratorKind::Clouds, &[(NOISE, 1.0)], 0.55, 0.12)
                }),
                ..layer("Mill Scale", "Iron Dark")
            },
            SmartLayer {
                color: Some([0.33, 0.15, 0.06]),
                rough: Some(0.92),
                metal: Some(0.0),
                height: Some(0.12),
                opacity: 0.9,
                mask: Some(SmartMask {
                    scale: Some(8.0),
                    noise: Some(0.6),
                    edge_width: Some(0.015),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 2.0), (CAVITY, 2.0), (CREASE, 1.5), (NOISE, 0.6)],
                        0.85,
                        0.15,
                    )
                }),
                ..layer("Rust", "Rust")
            },
            SmartLayer {
                color: Some([0.58, 0.58, 0.60]),
                rough: Some(0.3),
                opacity: 0.85,
                mask: Some(POLISHED_EDGES),
                ..layer("Worn Edges", "Iron Raw")
            },
            GRIME_LAYER,
        ],
    },
    SmartMaterial {
        name: "Rusty Iron",
        layers: &[
            layer("Iron", "Iron Dark"),
            SmartLayer {
                color: Some([0.42, 0.20, 0.08]),
                height: Some(0.1),
                mask: Some(SmartMask {
                    scale: Some(2.5),
                    noise: Some(0.3),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(NOISE, 1.0), (OCCLUSION, 0.6), (EDGE, -0.4)],
                        0.55,
                        0.08,
                    )
                }),
                ..layer("Rust", "Rust")
            },
            SmartLayer {
                color: Some([0.62, 0.32, 0.12]),
                rough: Some(0.95),
                opacity: 0.7,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    scale: Some(9.0),
                    noise: Some(0.0),
                    ..mix(GeneratorKind::Clouds, &[(NOISE, 1.0)], 0.62, 0.1)
                }),
                ..layer("Bright Rust", "Rust")
            },
            SmartLayer {
                color: Some([0.16, 0.08, 0.04]),
                height: Some(-0.15),
                blend: Blend::Normal,
                opacity: 0.9,
                // Small irregular pits, clustered where the rust is heavy
                // rather than spread evenly like polka dots.
                mask: Some(SmartMask {
                    scale: Some(5.0),
                    noise: Some(0.5),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(CELLS, -1.0), (NOISE, 0.7), (OCCLUSION, 0.4)],
                        0.3,
                        0.05,
                    )
                }),
                ..layer("Pitting", "Rust")
            },
            SmartLayer {
                color: Some([0.30, 0.15, 0.07]),
                opacity: 0.85,
                mask: Some(mask(GeneratorKind::Drips)),
                ..layer("Rust Bleeding", "Rust")
            },
            SmartLayer {
                color: Some([0.17, 0.09, 0.05]),
                opacity: 0.9,
                mask: Some(mask(GeneratorKind::CavityGrime)),
                ..layer("Deep Rust", "Rust")
            },
        ],
    },
    SmartMaterial {
        name: "Steel Scratched",
        layers: &[
            layer("Steel", "Steel Brushed"),
            SmartLayer {
                rough: Some(0.5),
                opacity: 0.7,
                channels: Some(ROUGH_ONLY),
                mask: Some(SmartMask {
                    scale: Some(2.5),
                    noise: Some(0.3),
                    ..mix(GeneratorKind::Clouds, &[(NOISE, 1.0)], 0.55, 0.2)
                }),
                ..layer("Smudges", "Steel Brushed")
            },
            SmartLayer {
                color: Some([0.80, 0.80, 0.82]),
                rough: Some(0.12),
                mask: Some(SCRATCHED),
                ..layer("Scratches", "Steel Brushed")
            },
            SmartLayer {
                color: Some([0.82, 0.82, 0.84]),
                rough: Some(0.15),
                mask: Some(POLISHED_EDGES),
                ..layer("Polished Edges", "Steel Brushed")
            },
            SmartLayer {
                opacity: 0.6,
                ..GRIME_LAYER
            },
        ],
    },
    SmartMaterial {
        name: "Painted Steel Worn",
        layers: &[
            layer("Steel", "Steel Brushed"),
            SmartLayer {
                color: Some([0.36, 0.20, 0.10]),
                rough: Some(0.85),
                metal: Some(0.0),
                mask: Some(SmartMask {
                    noise: Some(0.6),
                    edge_width: Some(0.02),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 1.0), (CONVEX, 0.6), (NOISE, 0.35)],
                        0.6,
                        0.1,
                    )
                }),
                ..layer("Rust Under Chips", "Rust")
            },
            SmartLayer {
                color: Some([0.42, 0.07, 0.04]),
                height: Some(0.06),
                color_var: Some(0.1),
                rough_var: Some(0.15),
                part_var: 0.1,
                mask: Some(SmartMask {
                    invert: true,
                    ..CHIPS
                }),
                ..layer("Paint", "Paint Red")
            },
            SmartLayer {
                color: Some([0.60, 0.60, 0.62]),
                rough: Some(0.25),
                metal: Some(1.0),
                opacity: 0.8,
                mask: Some(SCRATCHED),
                ..layer("Scratches", "Steel Brushed")
            },
            GRIME_LAYER,
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Military Olive",
        layers: &[
            SmartLayer {
                part_var: 0.25,
                height: Some(0.0),
                ..layer("Olive Paint", "Paint Olive")
            },
            SmartLayer {
                color: Some([0.48, 0.48, 0.47]),
                rough: Some(0.35),
                metal: Some(1.0),
                height: Some(-0.06),
                mask: Some(CHIPS),
                ..layer("Bare Metal Chips", "Steel Brushed")
            },
            SmartLayer {
                color: Some([0.45, 0.45, 0.44]),
                rough: Some(0.3),
                metal: Some(1.0),
                opacity: 0.7,
                mask: Some(SCRATCHED),
                ..layer("Scratches", "Steel Brushed")
            },
            SmartLayer {
                color: Some([0.26, 0.21, 0.14]),
                rough: Some(0.95),
                height: Some(0.08),
                opacity: 0.9,
                mask: Some(SmartMask {
                    invert: true,
                    noise: Some(0.6),
                    scale: Some(3.0),
                    ..mix(
                        GeneratorKind::PartGradient,
                        &[(WORLD_HEIGHT, 1.0), (PART_GRADIENT, 0.3)],
                        0.3,
                        0.12,
                    )
                }),
                ..layer("Mud", "Concrete")
            },
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Copper Weathered",
        layers: &[
            layer("Copper", "Copper"),
            SmartLayer {
                color: Some([0.42, 0.22, 0.14]),
                rough: Some(0.55),
                opacity: 0.85,
                mask: Some(SmartMask {
                    scale: Some(2.0),
                    noise: Some(0.4),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(NOISE, 1.0), (OCCLUSION, 0.5)],
                        0.45,
                        0.2,
                    )
                }),
                ..layer("Oxidation", "Copper")
            },
            SmartLayer {
                color: Some([0.33, 0.50, 0.42]),
                rough: Some(0.85),
                metal: Some(0.0),
                height: Some(0.06),
                color_var: Some(0.2),
                mask: Some(SmartMask {
                    noise: Some(0.6),
                    edge_width: Some(0.03),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 2.0), (CAVITY, 1.5), (CREASE, 1.0), (NOISE, 0.5)],
                        0.85,
                        0.15,
                    )
                }),
                ..layer("Patina", "Concrete")
            },
            SmartLayer {
                color: Some([0.36, 0.52, 0.44]),
                rough: Some(0.8),
                metal: Some(0.0),
                opacity: 0.5,
                mask: Some(mask(GeneratorKind::RainStreaks)),
                ..layer("Patina Streaks", "Concrete")
            },
            SmartLayer {
                color: Some([0.98, 0.68, 0.55]),
                rough: Some(0.2),
                opacity: 0.8,
                mask: Some(POLISHED_EDGES),
                ..layer("Polished Edges", "Copper")
            },
        ],
    },
    SmartMaterial {
        name: "Leather Worn",
        layers: &[
            layer("Leather", "Leather Brown"),
            SmartLayer {
                height: Some(-0.15),
                blend: Blend::Add,
                channels: Some(HEIGHT_ONLY),
                mask: Some(SmartMask {
                    scale: Some(6.0),
                    noise: Some(0.0),
                    ..mix(GeneratorKind::Clouds, &[(CELLS, 1.0)], 0.75, 0.15)
                }),
                ..layer("Pebble Grain", "Leather Brown")
            },
            SmartLayer {
                color: Some([0.12, 0.07, 0.04]),
                opacity: 0.8,
                channels: Some(COLOR_ONLY),
                mask: Some(mask(GeneratorKind::Dirt)),
                ..layer("Darkening", "Leather Brown")
            },
            SmartLayer {
                color: Some([0.55, 0.38, 0.24]),
                rough: Some(0.45),
                mask: Some(SmartMask {
                    edge_width: Some(0.015),
                    noise: Some(0.7),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 1.0), (CONVEX, 0.6)],
                        0.55,
                        0.15,
                    )
                }),
                ..layer("Worn Edges", "Leather Brown")
            },
            SmartLayer {
                color: Some([0.50, 0.36, 0.24]),
                rough: Some(0.7),
                opacity: 0.5,
                mask: Some(SCRATCHED),
                ..layer("Scuffs", "Leather Brown")
            },
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Rubber Dusty",
        layers: &[
            layer("Rubber", "Rubber"),
            SmartLayer {
                color: Some([0.16, 0.16, 0.16]),
                rough: Some(0.6),
                mask: Some(mask(GeneratorKind::EdgeWear)),
                ..layer("Scuffs", "Rubber")
            },
            SmartLayer {
                color: Some([0.14, 0.14, 0.14]),
                rough: Some(0.7),
                opacity: 0.6,
                mask: Some(SCRATCHED),
                ..layer("Scratches", "Rubber")
            },
            SmartLayer {
                opacity: 0.6,
                ..DUST_LAYER
            },
        ],
    },
    SmartMaterial {
        name: "Concrete Weathered",
        layers: &[
            layer("Concrete", "Concrete"),
            SmartLayer {
                height: Some(-0.2),
                blend: Blend::Add,
                channels: Some(HEIGHT_ONLY),
                mask: Some(SmartMask {
                    scale: Some(6.0),
                    noise: Some(0.0),
                    ..mix(GeneratorKind::Clouds, &[(CELLS, -1.0)], -0.12, 0.05)
                }),
                ..layer("Pores", "Concrete")
            },
            SmartLayer {
                color: Some([0.3, 0.29, 0.26]),
                opacity: 0.85,
                channels: Some(COLOR_ONLY),
                mask: Some(mask(GeneratorKind::RainStreaks)),
                ..layer("Rain Marks", "Concrete")
            },
            SmartLayer {
                color: Some([0.19, 0.24, 0.10]),
                rough: Some(0.95),
                height: Some(0.1),
                opacity: 0.9,
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    noise: Some(0.5),
                    ..mix(
                        GeneratorKind::Dust,
                        &[(SKY, 0.6), (OCCLUSION, 0.6), (NOISE, 0.9)],
                        1.25,
                        0.15,
                    )
                }),
                ..layer("Moss", "Moss")
            },
            SmartLayer {
                color: Some([0.12, 0.11, 0.1]),
                opacity: 0.8,
                channels: Some(COLOR_ONLY),
                mask: Some(GRIME),
                ..layer("Grime", "Concrete")
            },
        ],
    },
    SmartMaterial {
        name: "Stone Mossy",
        layers: &[
            SmartLayer {
                part_var: 0.2,
                ..layer("Stone", "Stone Dark")
            },
            SmartLayer {
                color: Some([0.10, 0.10, 0.10]),
                height: Some(-0.35),
                mask: Some(SmartMask {
                    scale: Some(1.2),
                    noise: Some(0.25),
                    ..mix(GeneratorKind::Clouds, &[(CELLS, 1.0)], 0.88, 0.05)
                }),
                ..layer("Cracks", "Stone Dark")
            },
            SmartLayer {
                color: Some([0.45, 0.44, 0.40]),
                rough: Some(0.75),
                opacity: 0.6,
                mask: Some(SmartMask {
                    edge_width: Some(0.02),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 1.0), (CONVEX, 0.6)],
                        0.5,
                        0.2,
                    )
                }),
                ..layer("Weathered Edges", "Stone Dark")
            },
            SmartLayer {
                color: Some([0.18, 0.25, 0.09]),
                height: Some(0.15),
                height_var: Some(0.3),
                var_scale: Some(30.0),
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    noise: Some(0.6),
                    ..mix(
                        GeneratorKind::Dust,
                        &[(SKY, 0.6), (OCCLUSION, 0.6), (NOISE, 0.9)],
                        1.2,
                        0.15,
                    )
                }),
                ..layer("Moss", "Moss")
            },
            GRIME_LAYER,
        ],
    },
    SmartMaterial {
        name: "Bone Aged",
        layers: &[
            SmartLayer {
                color_var: Some(0.08),
                height_var: Some(0.1),
                var_scale: Some(12.0),
                ..layer("Bone", "Bone")
            },
            SmartLayer {
                color: Some([0.62, 0.50, 0.30]),
                opacity: 0.8,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    noise: Some(0.6),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 1.5), (CAVITY, 1.0), (NOISE, 0.6)],
                        0.45,
                        0.25,
                    )
                }),
                ..layer("Yellowing", "Bone")
            },
            SmartLayer {
                color: Some([0.30, 0.24, 0.16]),
                height: Some(-0.15),
                mask: Some(SmartMask {
                    scale: Some(2.0),
                    noise: Some(0.2),
                    ..mix(GeneratorKind::Clouds, &[(CELLS, 1.0)], 0.92, 0.03)
                }),
                ..layer("Hairline Cracks", "Bone")
            },
            SmartLayer {
                opacity: 0.85,
                ..GRIME_LAYER
            },
        ],
    },
    SmartMaterial {
        name: "Sci-Fi Panels",
        layers: &[
            SmartLayer {
                part_var: 0.15,
                ..layer("Panels", "Plastic White")
            },
            SmartLayer {
                color: Some([0.08, 0.08, 0.09]),
                rough: Some(0.7),
                mask: Some(mask(GeneratorKind::CreaseGrime)),
                ..layer("Panel Gaps", "Plastic Black")
            },
            SmartLayer {
                color: Some([0.55, 0.56, 0.58]),
                rough: Some(0.3),
                metal: Some(1.0),
                opacity: 0.8,
                mask: Some(CHIPS),
                ..layer("Chipped Edges", "Aluminium")
            },
            SmartLayer {
                color: Some([0.70, 0.70, 0.70]),
                rough: Some(0.25),
                opacity: 0.5,
                mask: Some(SCRATCHED),
                ..layer("Scratches", "Plastic White")
            },
            SmartLayer {
                opacity: 0.5,
                ..GRIME_LAYER
            },
        ],
    },
    SmartMaterial {
        name: "Candle Wax",
        layers: &[
            layer("Wax", "Wax"),
            SmartLayer {
                color: Some([1.0, 0.86, 0.6]),
                opacity: 0.9,
                channels: Some(COLOR_ONLY),
                mask: Some(mask(GeneratorKind::ThinAreas)),
                ..layer("Light Through Thin Wax", "Wax")
            },
            SmartLayer {
                color: Some([0.45, 0.36, 0.25]),
                opacity: 0.7,
                channels: Some(COLOR_ONLY),
                mask: Some(mask(GeneratorKind::CavityGrime)),
                ..layer("Soot", "Wax")
            },
        ],
    },
    SmartMaterial {
        name: "Galvanized Metal",
        layers: &[
            SmartLayer {
                part_var: 0.1,
                ..layer("Zinc", "Zinc Galvanized")
            },
            SmartLayer {
                color: Some([0.62, 0.64, 0.66]),
                rough: Some(0.3),
                opacity: 0.6,
                mask: Some(SmartMask {
                    scale: Some(1.5),
                    noise: Some(0.2),
                    ..mix(GeneratorKind::Clouds, &[(CELLS, 1.0)], 0.6, 0.06)
                }),
                ..layer("Spangle", "Zinc Galvanized")
            },
            SmartLayer {
                color: Some([0.80, 0.80, 0.78]),
                rough: Some(0.95),
                metal: Some(0.0),
                height: Some(0.04),
                opacity: 0.85,
                mask: Some(SmartMask {
                    scale: Some(5.0),
                    noise: Some(0.7),
                    edge_width: Some(0.03),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 1.5), (CAVITY, 1.5), (CREASE, 1.0), (NOISE, 0.6)],
                        0.8,
                        0.12,
                    )
                }),
                ..layer("White Rust", "Concrete")
            },
            SmartLayer {
                color: Some([0.30, 0.29, 0.27]),
                rough: Some(0.7),
                opacity: 0.6,
                channels: Some([true, true, false, false]),
                mask: Some(mask(GeneratorKind::RainStreaks)),
                ..layer("Water Streaks", "Zinc Galvanized")
            },
            SmartLayer {
                color: Some([0.88, 0.89, 0.90]),
                rough: Some(0.2),
                opacity: 0.6,
                mask: Some(SCRATCHED),
                ..layer("Scratches", "Zinc Galvanized")
            },
            SmartLayer {
                opacity: 0.6,
                ..GRIME_LAYER
            },
        ],
    },
    SmartMaterial {
        name: "Bronze Patina",
        layers: &[
            SmartLayer {
                part_var: 0.1,
                ..layer("Bronze", "Bronze")
            },
            SmartLayer {
                color: Some([0.28, 0.17, 0.09]),
                rough: Some(0.55),
                metal: Some(0.6),
                opacity: 0.9,
                mask: Some(SmartMask {
                    scale: Some(2.0),
                    noise: Some(0.4),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(NOISE, 1.0), (OCCLUSION, 0.6), (CONVEX, -0.5)],
                        0.35,
                        0.2,
                    )
                }),
                ..layer("Brown Oxide", "Bronze")
            },
            SmartLayer {
                color: Some([0.32, 0.62, 0.52]),
                rough: Some(0.9),
                metal: Some(0.0),
                height: Some(0.1),
                color_var: Some(0.2),
                var_scale: Some(20.0),
                mask: Some(SmartMask {
                    scale: Some(6.0),
                    noise: Some(0.7),
                    edge_width: Some(0.04),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 2.0), (CAVITY, 2.0), (CREASE, 1.5), (NOISE, 0.5)],
                        0.8,
                        0.12,
                    )
                }),
                ..layer("Verdigris", "Concrete")
            },
            SmartLayer {
                color: Some([0.38, 0.62, 0.54]),
                rough: Some(0.85),
                metal: Some(0.0),
                opacity: 0.55,
                mask: Some(SmartMask {
                    noise: Some(0.25),
                    ..mix(
                        GeneratorKind::RainStreaks,
                        &[
                            (SKY, 0.4),
                            (STREAKS, 1.0),
                            (OVERHANG, -0.4),
                            (OCCLUSION, 0.3),
                        ],
                        0.95,
                        0.12,
                    )
                }),
                ..layer("Verdigris Runs", "Concrete")
            },
            SmartLayer {
                color: Some([0.86, 0.66, 0.40]),
                rough: Some(0.18),
                mask: Some(SmartMask {
                    edge_width: Some(0.02),
                    noise: Some(0.4),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 0.8), (CONVEX, 1.2)],
                        0.5,
                        0.15,
                    )
                }),
                ..layer("Hand-Rubbed Highlights", "Bronze")
            },
        ],
    },
    SmartMaterial {
        name: "Car Paint Chipped",
        layers: &[
            layer("Steel", "Steel Brushed"),
            SmartLayer {
                color: Some([0.52, 0.52, 0.50]),
                rough: Some(0.7),
                metal: Some(0.0),
                height: Some(0.04),
                mask: Some(SmartMask {
                    invert: true,
                    level: Some(0.9),
                    ..CHIPS
                }),
                ..layer("Primer", "Plastic White")
            },
            SmartLayer {
                height: Some(0.08),
                part_var: 0.05,
                mask: Some(SmartMask {
                    invert: true,
                    ..CHIPS
                }),
                ..layer("Paint", "Car Paint Red")
            },
            SmartLayer {
                rough: Some(0.3),
                opacity: 0.8,
                channels: Some(ROUGH_ONLY),
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    ..SCRATCHED
                }),
                ..layer("Clear-coat Swirls", "Car Paint Red")
            },
            SmartLayer {
                color: Some([0.20, 0.10, 0.06]),
                rough: Some(0.9),
                opacity: 0.6,
                mask: Some(SmartMask {
                    invert: true,
                    noise: Some(0.6),
                    scale: Some(3.0),
                    ..mix(
                        GeneratorKind::PartGradient,
                        &[(WORLD_HEIGHT, 1.0), (PART_GRADIENT, 0.3)],
                        0.25,
                        0.12,
                    )
                }),
                ..layer("Road Grime", "Concrete")
            },
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Plastic Worn",
        layers: &[
            SmartLayer {
                color: Some([0.85, 0.62, 0.08]),
                part_var: 0.08,
                ..layer("Plastic", "Plastic Red")
            },
            SmartLayer {
                color: Some([0.88, 0.78, 0.50]),
                opacity: 0.5,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    scale: Some(2.0),
                    noise: Some(0.4),
                    ..mix(GeneratorKind::Clouds, &[(SKY, 0.8), (NOISE, 0.5)], 0.7, 0.2)
                }),
                ..layer("Sun Fade", "Plastic Red")
            },
            SmartLayer {
                rough: Some(0.15),
                opacity: 0.9,
                channels: Some(ROUGH_ONLY),
                mask: Some(POLISHED_EDGES),
                ..layer("Handled Shine", "Plastic Red")
            },
            SmartLayer {
                color: Some([0.95, 0.88, 0.70]),
                rough: Some(0.6),
                opacity: 0.7,
                mask: Some(SCRATCHED),
                ..layer("Stress Whitening", "Plastic Red")
            },
            SmartLayer {
                opacity: 0.65,
                ..GRIME_LAYER
            },
            SmartLayer {
                opacity: 0.6,
                ..DUST_LAYER
            },
        ],
    },
    SmartMaterial {
        name: "Gunmetal Blued",
        layers: &[
            SmartLayer {
                color: Some([0.16, 0.19, 0.26]),
                rough: Some(0.28),
                ..layer("Bluing", "Gunmetal")
            },
            SmartLayer {
                color: Some([0.30, 0.22, 0.24]),
                opacity: 0.45,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    scale: Some(2.0),
                    noise: Some(0.4),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(NOISE, 0.8), (CONVEX, 0.6)],
                        0.55,
                        0.2,
                    )
                }),
                ..layer("Plum Fade", "Gunmetal")
            },
            SmartLayer {
                rough: Some(0.5),
                opacity: 0.6,
                channels: Some(ROUGH_ONLY),
                mask: Some(SmartMask {
                    scale: Some(3.5),
                    noise: Some(0.3),
                    ..mix(GeneratorKind::Clouds, &[(NOISE, 1.0)], 0.58, 0.12)
                }),
                ..layer("Fingerprints", "Gunmetal")
            },
            SmartLayer {
                color: Some([0.72, 0.72, 0.74]),
                rough: Some(0.22),
                mask: Some(SmartMask {
                    edge_width: Some(0.012),
                    ..POLISHED_EDGES
                }),
                ..layer("Holster Wear", "Steel Brushed")
            },
            SmartLayer {
                color: Some([0.62, 0.63, 0.66]),
                rough: Some(0.2),
                opacity: 0.7,
                mask: Some(SCRATCHED),
                ..layer("Scratches", "Steel Brushed")
            },
            SmartLayer {
                color: Some([0.04, 0.035, 0.03]),
                rough: Some(0.45),
                opacity: 0.8,
                mask: Some(GRIME),
                ..layer("Gun Oil Grime", "Gunmetal")
            },
        ],
    },
    SmartMaterial {
        name: "Marble Polished",
        layers: &[
            SmartLayer {
                part_var: 0.06,
                ..layer("Marble", "Marble White")
            },
            SmartLayer {
                color: Some([0.70, 0.70, 0.68]),
                opacity: 0.6,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    scale: Some(1.2),
                    noise: Some(0.6),
                    ..mix(GeneratorKind::Clouds, &[(NOISE, 1.0)], 0.55, 0.25)
                }),
                ..layer("Clouding", "Marble White")
            },
            SmartLayer {
                color: Some([0.38, 0.40, 0.38]),
                opacity: 0.85,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    scale: Some(0.7),
                    noise: Some(0.5),
                    ..mix(GeneratorKind::Clouds, &[(CELLS, 1.0)], 0.9, 0.04)
                }),
                ..layer("Veins", "Marble White")
            },
            SmartLayer {
                color: Some([0.55, 0.50, 0.42]),
                opacity: 0.5,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    scale: Some(1.6),
                    noise: Some(0.5),
                    seed: 3.0,
                    ..mix(GeneratorKind::Clouds, &[(CELLS, 1.0)], 0.92, 0.03)
                }),
                ..layer("Gold Veins", "Marble White")
            },
            SmartLayer {
                rough: Some(0.45),
                opacity: 0.7,
                channels: Some(ROUGH_ONLY),
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    noise: Some(0.5),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(SKY, 0.6), (NOISE, 0.7)],
                        0.75,
                        0.15,
                    )
                }),
                ..layer("Water Etching", "Marble White")
            },
            SmartLayer {
                color: Some([0.30, 0.27, 0.22]),
                opacity: 0.6,
                channels: Some(COLOR_ONLY),
                mask: Some(GRIME),
                ..layer("Grime", "Marble White")
            },
        ],
    },
    SmartMaterial {
        name: "Brick Weathered",
        layers: &[
            SmartLayer {
                part_var: 0.3,
                ..layer("Brick", "Brick Red")
            },
            SmartLayer {
                color: Some([0.30, 0.13, 0.08]),
                opacity: 0.6,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    scale: Some(2.5),
                    noise: Some(0.3),
                    ..mix(GeneratorKind::Clouds, &[(NOISE, 1.0)], 0.6, 0.15)
                }),
                ..layer("Kiln Burn", "Brick Red")
            },
            SmartLayer {
                height: Some(-0.2),
                blend: Blend::Add,
                channels: Some(HEIGHT_ONLY),
                mask: Some(SmartMask {
                    scale: Some(5.0),
                    noise: Some(0.0),
                    ..mix(GeneratorKind::Clouds, &[(CELLS, -1.0)], -0.15, 0.05)
                }),
                ..layer("Pores", "Brick Red")
            },
            SmartLayer {
                color: Some([0.70, 0.40, 0.28]),
                height: Some(-0.15),
                opacity: 0.85,
                mask: Some(SmartMask {
                    edge_width: Some(0.02),
                    ..CHIPS
                }),
                ..layer("Chipped Edges", "Brick Red")
            },
            SmartLayer {
                color: Some([0.86, 0.84, 0.78]),
                rough: Some(0.95),
                opacity: 0.7,
                mask: Some(SmartMask {
                    invert: true,
                    noise: Some(0.7),
                    scale: Some(4.0),
                    ..mix(
                        GeneratorKind::PartGradient,
                        &[(WORLD_HEIGHT, 1.0), (OCCLUSION, -0.3)],
                        0.3,
                        0.12,
                    )
                }),
                ..layer("Efflorescence", "Concrete")
            },
            SmartLayer {
                color: Some([0.18, 0.12, 0.09]),
                opacity: 0.6,
                channels: Some(COLOR_ONLY),
                mask: Some(mask(GeneratorKind::RainStreaks)),
                ..layer("Rain Streaks", "Brick Red")
            },
            GRIME_LAYER,
        ],
    },
    SmartMaterial {
        name: "Ceramic Glazed Crazed",
        layers: &[
            SmartLayer {
                color: Some([0.86, 0.82, 0.70]),
                ..layer("Glaze", "Porcelain")
            },
            SmartLayer {
                color: Some([0.68, 0.60, 0.44]),
                opacity: 0.7,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    noise: Some(0.3),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 1.2), (CAVITY, 1.2), (CONCAVE, 0.6)],
                        0.4,
                        0.25,
                    )
                }),
                ..layer("Glaze Pooling", "Porcelain")
            },
            SmartLayer {
                color: Some([0.70, 0.45, 0.30]),
                rough: Some(0.6),
                opacity: 0.6,
                mask: Some(SmartMask {
                    edge_width: Some(0.008),
                    noise: Some(0.5),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 1.0), (CONVEX, 0.6)],
                        0.6,
                        0.15,
                    )
                }),
                ..layer("Thin Glaze on Edges", "Terracotta")
            },
            SmartLayer {
                color: Some([0.30, 0.25, 0.18]),
                height: Some(-0.08),
                opacity: 0.8,
                channels: Some([true, false, false, true]),
                mask: Some(SmartMask {
                    scale: Some(4.0),
                    noise: Some(0.1),
                    ..mix(GeneratorKind::Clouds, &[(CELLS, 1.0)], 0.93, 0.025)
                }),
                ..layer("Crazing", "Porcelain")
            },
            SmartLayer {
                opacity: 0.5,
                ..GRIME_LAYER
            },
        ],
    },
    SmartMaterial {
        name: "Chrome Pitted",
        layers: &[
            layer("Chrome", "Chrome"),
            SmartLayer {
                rough: Some(0.3),
                opacity: 0.6,
                channels: Some(ROUGH_ONLY),
                mask: Some(SmartMask {
                    scale: Some(2.0),
                    noise: Some(0.4),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(NOISE, 1.0), (OCCLUSION, 0.4)],
                        0.6,
                        0.15,
                    )
                }),
                ..layer("Haze", "Chrome")
            },
            SmartLayer {
                color: Some([0.70, 0.68, 0.62]),
                rough: Some(0.45),
                opacity: 0.8,
                mask: Some(SmartMask {
                    noise: Some(0.6),
                    edge_width: Some(0.03),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 1.5), (CAVITY, 1.5), (CREASE, 1.0), (NOISE, 0.5)],
                        0.7,
                        0.12,
                    )
                }),
                ..layer("Flaked to Nickel", "Nickel")
            },
            SmartLayer {
                color: Some([0.38, 0.18, 0.08]),
                rough: Some(0.9),
                metal: Some(0.0),
                height: Some(-0.1),
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    noise: Some(0.35),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(CELLS, -1.0), (OCCLUSION, 0.4), (NOISE, 0.3)],
                        0.0,
                        0.04,
                    )
                }),
                ..layer("Rust Pits", "Rust")
            },
            SmartLayer {
                color: Some([0.34, 0.17, 0.08]),
                opacity: 0.7,
                mask: Some(mask(GeneratorKind::Drips)),
                ..layer("Rust Bleeding", "Rust")
            },
            SmartLayer {
                opacity: 0.6,
                ..GRIME_LAYER
            },
        ],
    },
    SmartMaterial {
        name: "Canvas Dirty",
        layers: &[
            SmartLayer {
                color: Some([0.62, 0.56, 0.42]),
                part_var: 0.1,
                ..layer("Canvas", "Fabric Olive")
            },
            SmartLayer {
                color: Some([0.40, 0.34, 0.24]),
                opacity: 0.5,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    scale: Some(2.5),
                    noise: Some(0.3),
                    ..mix(GeneratorKind::Clouds, &[(NOISE, 1.0)], 0.62, 0.1)
                }),
                ..layer("Stains", "Fabric Olive")
            },
            SmartLayer {
                color: Some([0.78, 0.74, 0.62]),
                opacity: 0.5,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    edge_width: Some(0.015),
                    noise: Some(0.7),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 1.0), (CONVEX, 0.8)],
                        0.5,
                        0.15,
                    )
                }),
                ..layer("Frayed Edges", "Fabric Olive")
            },
            SmartLayer {
                color: Some([0.24, 0.19, 0.13]),
                height: Some(0.05),
                opacity: 0.85,
                mask: Some(SmartMask {
                    invert: true,
                    noise: Some(0.6),
                    scale: Some(3.0),
                    ..mix(
                        GeneratorKind::PartGradient,
                        &[(WORLD_HEIGHT, 1.0), (PART_GRADIENT, 0.3)],
                        0.3,
                        0.12,
                    )
                }),
                ..layer("Mud", "Concrete")
            },
            GRIME_LAYER,
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Bark Mossy",
        layers: &[
            SmartLayer {
                part_var: 0.1,
                ..layer("Bark", "Bark")
            },
            SmartLayer {
                color: Some([0.09, 0.07, 0.05]),
                height: Some(-0.45),
                mask: Some(SmartMask {
                    scale: Some(1.2),
                    noise: Some(0.3),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(GRAIN, 1.0), (NOISE, 0.4)],
                        0.6,
                        0.12,
                    )
                }),
                ..layer("Furrows", "Bark")
            },
            SmartLayer {
                color: Some([0.42, 0.38, 0.32]),
                opacity: 0.6,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    edge_width: Some(0.02),
                    noise: Some(0.5),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 0.6), (CONVEX, 1.0)],
                        0.45,
                        0.15,
                    )
                }),
                ..layer("Weathered Ridges", "Bark")
            },
            SmartLayer {
                color: Some([0.62, 0.66, 0.52]),
                height: Some(0.05),
                opacity: 0.8,
                mask: Some(SmartMask {
                    scale: Some(4.0),
                    noise: Some(0.4),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(CELLS, -1.0), (NOISE, 0.5)],
                        0.05,
                        0.05,
                    )
                }),
                ..layer("Lichen", "Moss")
            },
            SmartLayer {
                color: Some([0.18, 0.27, 0.08]),
                height: Some(0.15),
                height_var: Some(0.3),
                var_scale: Some(30.0),
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    noise: Some(0.6),
                    ..mix(
                        GeneratorKind::Dust,
                        &[(SKY, 0.5), (OCCLUSION, 0.6), (OVERHANG, -0.4), (NOISE, 0.9)],
                        1.0,
                        0.15,
                    )
                }),
                ..layer("Moss", "Moss")
            },
            GRIME_LAYER,
        ],
    },
    SmartMaterial {
        name: "Jade Carved",
        layers: &[
            layer("Jade", "Jade"),
            SmartLayer {
                color: Some([0.70, 0.80, 0.68]),
                opacity: 0.55,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    scale: Some(1.5),
                    noise: Some(0.6),
                    ..mix(GeneratorKind::Clouds, &[(NOISE, 1.0)], 0.6, 0.15)
                }),
                ..layer("Pale Mottling", "Jade")
            },
            SmartLayer {
                color: Some([0.10, 0.30, 0.18]),
                opacity: 0.6,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    noise: Some(0.5),
                    seed: 2.0,
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(NOISE, 1.0), (CELLS, 0.4)],
                        0.75,
                        0.12,
                    )
                }),
                ..layer("Deep Inclusions", "Jade")
            },
            SmartLayer {
                color: Some([0.55, 0.85, 0.60]),
                opacity: 0.7,
                channels: Some(COLOR_ONLY),
                mask: Some(mask(GeneratorKind::ThinAreas)),
                ..layer("Translucent Thin Areas", "Jade")
            },
            SmartLayer {
                rough: Some(0.08),
                channels: Some(ROUGH_ONLY),
                mask: Some(POLISHED_EDGES),
                ..layer("Polished Highs", "Jade")
            },
            SmartLayer {
                color: Some([0.20, 0.17, 0.10]),
                rough: Some(0.7),
                opacity: 0.7,
                mask: Some(mask(GeneratorKind::CavityGrime)),
                ..layer("Carving Dirt", "Concrete")
            },
        ],
    },
    SmartMaterial {
        name: "Walnut Varnished",
        layers: &[
            SmartLayer {
                part_var: 0.12,
                ..layer("Walnut", "Walnut")
            },
            SmartLayer {
                color: Some([0.10, 0.05, 0.025]),
                height: Some(-0.15),
                opacity: 0.7,
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    noise: Some(0.1),
                    ..mix(GeneratorKind::Clouds, &[(GRAIN, 1.0)], 0.45, 0.2)
                }),
                ..layer("Grain", "Walnut")
            },
            SmartLayer {
                rough: Some(0.12),
                channels: Some(ROUGH_ONLY),
                ..layer("Varnish", "Walnut")
            },
            SmartLayer {
                color: Some([0.40, 0.26, 0.15]),
                rough: Some(0.6),
                opacity: 0.8,
                mask: Some(SmartMask {
                    edge_width: Some(0.01),
                    noise: Some(0.7),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 1.0), (CONVEX, 0.5)],
                        0.6,
                        0.1,
                    )
                }),
                ..layer("Varnish Worn Through", "Walnut")
            },
            SmartLayer {
                rough: Some(0.35),
                opacity: 0.6,
                channels: Some(ROUGH_ONLY),
                mask: Some(SCRATCHED),
                ..layer("Fine Scratches", "Walnut")
            },
            SmartLayer {
                opacity: 0.7,
                ..GRIME_LAYER
            },
            SmartLayer {
                opacity: 0.3,
                ..DUST_LAYER
            },
        ],
    },
    SmartMaterial {
        name: "Sci-Fi Hull Dark",
        layers: &[
            SmartLayer {
                color: Some([0.22, 0.23, 0.25]),
                rough: Some(0.45),
                metal: Some(0.0),
                part_var: 0.3,
                ..layer("Hull Paint", "Gunmetal")
            },
            SmartLayer {
                color: Some([0.85, 0.42, 0.08]),
                rough: Some(0.4),
                metal: Some(0.0),
                mask: Some(SmartMask {
                    level: Some(0.78),
                    ..mask(GeneratorKind::PartRandom)
                }),
                ..layer("Accent Parts", "Paint Red")
            },
            SmartLayer {
                color: Some([0.03, 0.03, 0.035]),
                rough: Some(0.7),
                mask: Some(mask(GeneratorKind::CreaseGrime)),
                ..layer("Panel Gaps", "Plastic Black")
            },
            SmartLayer {
                color: Some([0.70, 0.71, 0.72]),
                rough: Some(0.3),
                metal: Some(1.0),
                height: Some(-0.04),
                mask: Some(CHIPS),
                ..layer("Chipped to Titanium", "Titanium")
            },
            SmartLayer {
                color: Some([0.05, 0.045, 0.04]),
                rough: Some(0.8),
                metal: Some(0.0),
                opacity: 0.85,
                mask: Some(SmartMask {
                    invert: true,
                    noise: Some(0.8),
                    scale: Some(2.5),
                    ..mix(
                        GeneratorKind::Height,
                        &[(WORLD_HEIGHT, 1.0), (OVERHANG, -0.4), (NOISE, 0.3)],
                        0.35,
                        0.15,
                    )
                }),
                ..layer("Engine Scorch", "Plastic Black")
            },
            SmartLayer {
                color: Some([0.12, 0.11, 0.10]),
                opacity: 0.6,
                channels: Some(COLOR_ONLY),
                mask: Some(mask(GeneratorKind::Drips)),
                ..layer("Leaks", "Plastic Black")
            },
            SmartLayer {
                opacity: 0.5,
                ..GRIME_LAYER
            },
        ],
    },
    SmartMaterial {
        name: "Titanium Heat Tinted",
        layers: &[
            SmartLayer {
                rough_var: Some(0.15),
                var_scale: Some(8.0),
                ..layer("Titanium", "Titanium")
            },
            SmartLayer {
                color: Some([0.86, 0.68, 0.36]),
                rough: Some(0.25),
                mask: Some(SmartMask {
                    scale: Some(1.2),
                    noise: Some(0.3),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(NOISE, 1.0), (WORLD_HEIGHT, -0.4)],
                        0.30,
                        0.06,
                    )
                }),
                ..layer("Straw Tint", "Titanium")
            },
            SmartLayer {
                color: Some([0.50, 0.30, 0.55]),
                rough: Some(0.22),
                mask: Some(SmartMask {
                    scale: Some(1.2),
                    noise: Some(0.3),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(NOISE, 1.0), (WORLD_HEIGHT, -0.4)],
                        0.38,
                        0.05,
                    )
                }),
                ..layer("Purple Tint", "Titanium")
            },
            SmartLayer {
                color: Some([0.25, 0.42, 0.82]),
                rough: Some(0.2),
                mask: Some(SmartMask {
                    scale: Some(1.2),
                    noise: Some(0.3),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(NOISE, 1.0), (WORLD_HEIGHT, -0.4)],
                        0.46,
                        0.05,
                    )
                }),
                ..layer("Blue Tint", "Titanium")
            },
            SmartLayer {
                color: Some([0.82, 0.80, 0.76]),
                rough: Some(0.18),
                opacity: 0.7,
                mask: Some(SCRATCHED),
                ..layer("Scratches", "Titanium")
            },
            SmartLayer {
                color: Some([0.84, 0.82, 0.78]),
                rough: Some(0.15),
                opacity: 0.8,
                mask: Some(POLISHED_EDGES),
                ..layer("Polished Edges", "Titanium")
            },
        ],
    },
    SmartMaterial {
        name: "Terracotta Aged",
        layers: &[
            SmartLayer {
                part_var: 0.12,
                ..layer("Terracotta", "Terracotta")
            },
            SmartLayer {
                color: Some([0.45, 0.26, 0.17]),
                rough: Some(0.9),
                opacity: 0.7,
                mask: Some(SmartMask {
                    invert: true,
                    noise: Some(0.6),
                    scale: Some(3.0),
                    ..mix(
                        GeneratorKind::PartGradient,
                        &[(WORLD_HEIGHT, 1.0)],
                        0.4,
                        0.15,
                    )
                }),
                ..layer("Water Soak", "Terracotta")
            },
            SmartLayer {
                color: Some([0.88, 0.86, 0.80]),
                rough: Some(0.95),
                height: Some(0.03),
                opacity: 0.75,
                mask: Some(SmartMask {
                    scale: Some(3.5),
                    noise: Some(0.8),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(NOISE, 0.8), (WORLD_HEIGHT, -0.5), (OCCLUSION, 0.4)],
                        0.42,
                        0.1,
                    )
                }),
                ..layer("Mineral Bloom", "Concrete")
            },
            SmartLayer {
                color: Some([0.82, 0.52, 0.36]),
                height: Some(-0.12),
                opacity: 0.85,
                mask: Some(CHIPS),
                ..layer("Chips", "Terracotta")
            },
            SmartLayer {
                color: Some([0.20, 0.28, 0.10]),
                height: Some(0.08),
                opacity: 0.85,
                mask: Some(SmartMask {
                    noise: Some(0.6),
                    scale: Some(5.0),
                    ..mix(
                        GeneratorKind::CavityGrime,
                        &[(CAVITY, 1.5), (OCCLUSION, 0.8), (NOISE, 0.4)],
                        0.65,
                        0.12,
                    )
                }),
                ..layer("Algae", "Moss")
            },
            GRIME_LAYER,
        ],
    },
    SmartMaterial {
        name: "Granite Snowy",
        layers: &[
            SmartLayer {
                part_var: 0.15,
                ..layer("Granite", "Granite")
            },
            SmartLayer {
                color: Some([0.12, 0.12, 0.12]),
                height: Some(-0.3),
                mask: Some(SmartMask {
                    scale: Some(1.0),
                    noise: Some(0.3),
                    ..mix(GeneratorKind::Clouds, &[(CELLS, 1.0)], 0.9, 0.04)
                }),
                ..layer("Fractures", "Granite")
            },
            SmartLayer {
                color: Some([0.20, 0.18, 0.16]),
                opacity: 0.7,
                channels: Some(COLOR_ONLY),
                mask: Some(mask(GeneratorKind::RainStreaks)),
                ..layer("Meltwater Streaks", "Granite")
            },
            GRIME_LAYER,
            SmartLayer {
                color: Some([0.80, 0.86, 0.92]),
                rough: Some(0.15),
                opacity: 0.7,
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    noise: Some(0.5),
                    ..mix(
                        GeneratorKind::Dust,
                        &[(SKY, 0.8), (UP, 0.9), (OVERHANG, -0.5)],
                        0.8,
                        0.05,
                    )
                }),
                ..layer("Icy Melt Line", "Ice")
            },
            SmartLayer {
                height: Some(0.3),
                height_var: Some(0.25),
                var_scale: Some(8.0),
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    noise: Some(0.5),
                    ..mix(
                        GeneratorKind::Dust,
                        &[(SKY, 0.8), (UP, 0.9), (OVERHANG, -0.5)],
                        0.95,
                        0.12,
                    )
                }),
                ..layer("Snow", "Snow")
            },
        ],
    },
    // ------------------------------------------------------------ scanned
    // Built on photo-scanned PBR sets (Poly Haven, CC0; assets/pbr): the
    // scan gives the base color, roughness, height and metalness, and
    // procedural layers add wear and dirt that follow the mesh.
    SmartMaterial {
        name: "Rusty Sheet Metal",
        layers: &[
            scan("Rusty Sheet", "Rust", "rusty_metal_02", 1.2, 0.2),
            SmartLayer {
                color: Some([0.30, 0.14, 0.06]),
                rough: Some(0.92),
                height: Some(0.05),
                opacity: 0.8,
                mask: Some(SmartMask {
                    noise: Some(0.7),
                    scale: Some(3.0),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 2.0), (CAVITY, 1.5), (STREAKS, 0.6)],
                        0.55,
                        0.15,
                    )
                }),
                ..layer("Heavy Rust", "Rust")
            },
            SCAN_GRIME,
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Diamond Plate Worn",
        layers: &[
            scan("Diamond Plate", "Steel Brushed", "metal_plate", 0.8, 0.5),
            SmartLayer {
                color: Some([0.72, 0.72, 0.73]),
                rough: Some(0.18),
                metal: Some(1.0),
                mask: Some(POLISHED_EDGES),
                ..layer("Polished Edges", "Steel Brushed")
            },
            SCAN_GRIME,
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Corrugated Iron Weathered",
        layers: &[
            scan(
                "Corrugated Iron",
                "Zinc Galvanized",
                "corrugated_iron",
                1.0,
                0.45,
            ),
            SmartLayer {
                color: Some([0.38, 0.19, 0.08]),
                rough: Some(0.9),
                metal: Some(0.0),
                opacity: 0.9,
                mask: Some(SmartMask {
                    noise: Some(0.6),
                    scale: Some(4.0),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 1.5), (STREAKS, 1.0), (CREASE, 1.0)],
                        0.6,
                        0.15,
                    )
                }),
                ..layer("Rust Runs", "Rust")
            },
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Green Paint Rusted",
        layers: &[
            scan("Green Paint", "Paint Olive", "green_metal_rust", 1.2, 0.2),
            SmartLayer {
                color: Some([0.50, 0.50, 0.50]),
                rough: Some(0.35),
                metal: Some(1.0),
                height: Some(-0.04),
                mask: Some(CHIPS),
                ..layer("Bare Metal Chips", "Steel Brushed")
            },
            SCAN_GRIME,
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Blue Painted Plate",
        layers: &[
            scan(
                "Blue Paint",
                "Car Paint Blue",
                "blue_metal_plate",
                1.0,
                0.25,
            ),
            SmartLayer {
                color: Some([0.55, 0.55, 0.56]),
                rough: Some(0.3),
                metal: Some(1.0),
                height: Some(-0.03),
                mask: Some(SmartMask {
                    edge_width: Some(0.008),
                    ..CHIPS
                }),
                ..layer("Worn Edges", "Steel Brushed")
            },
            SmartLayer {
                color: Some([0.6, 0.6, 0.62]),
                rough: Some(0.3),
                metal: Some(1.0),
                opacity: 0.6,
                mask: Some(SCRATCHED),
                ..layer("Scratches", "Steel Brushed")
            },
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Worn Table Wood",
        layers: &[
            scan("Table Wood", "Walnut", "wood_table_worn", 0.9, 0.2),
            SmartLayer {
                color: Some([0.62, 0.45, 0.28]),
                rough: Some(0.7),
                opacity: 0.6,
                mask: Some(SmartMask {
                    edge_width: Some(0.01),
                    noise: Some(0.6),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 1.0), (CONVEX, 0.6)],
                        0.6,
                        0.1,
                    )
                }),
                ..layer("Worn Edges", "Oak")
            },
            SCAN_GRIME,
        ],
    },
    SmartMaterial {
        name: "Oak Veneer",
        layers: &[
            scan("Oak Veneer", "Oak", "oak_veneer_01", 1.0, 0.08),
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Weathered Planks",
        layers: &[
            scan("Planks", "Pine", "weathered_brown_planks", 0.7, 0.35),
            SmartLayer {
                color: Some([0.62, 0.60, 0.55]),
                rough: Some(0.95),
                opacity: 0.45,
                mask: Some(SmartMask {
                    scale: Some(1.5),
                    noise: Some(0.2),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(SKY, 0.8), (NOISE, 0.6)],
                        0.6,
                        0.25,
                    )
                }),
                ..layer("Sun Bleach", "Pine")
            },
            SmartLayer {
                color: Some([0.20, 0.27, 0.08]),
                rough: Some(0.95),
                height: Some(0.06),
                opacity: 0.85,
                mask: Some(SmartMask {
                    noise: Some(0.8),
                    scale: Some(5.0),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 1.5), (CAVITY, 1.0), (UP, 0.5)],
                        0.65,
                        0.12,
                    )
                }),
                ..layer("Moss", "Fabric Olive")
            },
        ],
    },
    SmartMaterial {
        name: "Dark Wood Varnished",
        layers: &[
            scan("Dark Wood", "Walnut", "dark_wood", 0.8, 0.1),
            SmartLayer {
                rough: Some(0.18),
                opacity: 0.55,
                channels: Some(ROUGH_ONLY),
                ..layer("Varnish", "Walnut")
            },
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Plywood",
        layers: &[
            scan("Plywood", "Pine", "plywood", 0.8, 0.12),
            SmartLayer {
                color: Some([0.28, 0.19, 0.10]),
                rough: Some(0.85),
                opacity: 0.5,
                mask: Some(SmartMask {
                    edge_width: Some(0.012),
                    noise: Some(0.4),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 1.0), (CONVEX, 0.4)],
                        0.55,
                        0.1,
                    )
                }),
                ..layer("Dark Edges", "Walnut")
            },
            SCAN_GRIME,
        ],
    },
    SmartMaterial {
        name: "Leather Brown Scan",
        layers: &[
            scan("Leather", "Leather Brown", "brown_leather", 2.0, 0.25),
            SmartLayer {
                color: Some([0.55, 0.36, 0.20]),
                rough: Some(0.45),
                opacity: 0.6,
                mask: Some(SmartMask {
                    edge_width: Some(0.012),
                    noise: Some(0.5),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 1.0), (CONVEX, 0.8)],
                        0.55,
                        0.12,
                    )
                }),
                ..layer("Polished Wear", "Leather Brown")
            },
            SCAN_GRIME,
        ],
    },
    SmartMaterial {
        name: "Denim",
        layers: &[
            scan("Denim", "Denim", "denim_fabric", 2.5, 0.2),
            SmartLayer {
                color: Some([0.62, 0.68, 0.76]),
                rough: Some(0.95),
                opacity: 0.55,
                mask: Some(SmartMask {
                    edge_width: Some(0.02),
                    noise: Some(0.6),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 1.0), (CONVEX, 0.8)],
                        0.5,
                        0.15,
                    )
                }),
                ..layer("Faded Edges", "Denim")
            },
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Linen",
        layers: &[
            scan("Linen", "Fabric Olive", "rough_linen", 2.0, 0.45),
            SCAN_GRIME,
        ],
    },
    SmartMaterial {
        name: "Concrete Worn",
        layers: &[
            scan("Concrete", "Concrete", "concrete_floor_worn_001", 0.8, 0.25),
            SmartLayer {
                color: Some([0.70, 0.69, 0.66]),
                rough: Some(0.9),
                opacity: 0.5,
                mask: Some(SmartMask {
                    edge_width: Some(0.015),
                    noise: Some(0.8),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 1.0), (CONVEX, 0.5), (NOISE, 0.3)],
                        0.6,
                        0.1,
                    )
                }),
                ..layer("Chipped Edges", "Concrete")
            },
            SCAN_GRIME,
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Red Brick",
        layers: &[
            scan("Bricks", "Brick Red", "red_brick", 0.6, 0.4),
            SCAN_GRIME,
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Castle Brick",
        layers: &[
            scan("Bricks", "Brick Red", "castle_brick_02_red", 0.6, 0.4),
            SmartLayer {
                color: Some([0.18, 0.26, 0.08]),
                rough: Some(0.95),
                height: Some(0.05),
                opacity: 0.8,
                mask: Some(SmartMask {
                    noise: Some(0.8),
                    scale: Some(5.0),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(CAVITY, 1.5), (OCCLUSION, 1.0), (SKY, 0.6)],
                        0.62,
                        0.12,
                    )
                }),
                ..layer("Moss", "Fabric Olive")
            },
            SCAN_GRIME,
        ],
    },
    SmartMaterial {
        name: "Rock Face",
        layers: &[
            scan("Rock", "Stone Dark", "rock_face_03", 0.5, 0.45),
            SmartLayer {
                color: Some([0.22, 0.30, 0.09]),
                rough: Some(0.95),
                height: Some(0.06),
                opacity: 0.9,
                mask: Some(SmartMask {
                    noise: Some(0.8),
                    scale: Some(4.0),
                    ..mix(
                        GeneratorKind::Dust,
                        &[(UP, 1.0), (SKY, 0.6), (CAVITY, 0.4)],
                        0.9,
                        0.2,
                    )
                }),
                ..layer("Moss On Top", "Fabric Olive")
            },
        ],
    },
    SmartMaterial {
        name: "Rustic Stone Wall",
        layers: &[
            scan(
                "Stone Wall",
                "Stone Dark",
                "rustic_stone_wall_02",
                0.5,
                0.45,
            ),
            SCAN_GRIME,
            DUST_LAYER,
        ],
    },
    SmartMaterial {
        name: "Tree Bark",
        layers: &[
            scan("Bark", "Bark", "bark_brown_02", 0.8, 0.5),
            SmartLayer {
                color: Some([0.20, 0.28, 0.08]),
                rough: Some(0.95),
                opacity: 0.7,
                mask: Some(SmartMask {
                    noise: Some(0.8),
                    scale: Some(4.0),
                    ..mix(GeneratorKind::Dirt, &[(CAVITY, 1.5), (UP, 0.6)], 0.65, 0.15)
                }),
                ..layer("Moss", "Fabric Olive")
            },
        ],
    },
    // ---------------------------------------------------------------- skin
    SmartMaterial {
        name: "Skin Realistic",
        layers: &[
            SmartLayer {
                color: Some([0.80, 0.58, 0.47]),
                rough: Some(0.45),
                color_var: Some(0.05),
                var_scale: Some(12.0),
                ..layer("Skin", "Skin")
            },
            SKIN_FLUSH,
            SKIN_VEINS,
            SmartLayer {
                color: Some([0.50, 0.31, 0.20]),
                opacity: 0.5,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    scale: Some(8.0),
                    noise: Some(0.0),
                    invert: true,
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(CELLS, 1.0), (NOISE, -0.5)],
                        -0.12,
                        0.03,
                    )
                }),
                ..layer("Freckles", "Skin")
            },
            SKIN_PORES,
            SKIN_FOLDS,
            SKIN_SHEEN,
        ],
    },
    SmartMaterial {
        name: "Skin Dark",
        layers: &[
            SmartLayer {
                color: Some([0.45, 0.30, 0.22]),
                rough: Some(0.5),
                color_var: Some(0.06),
                var_scale: Some(12.0),
                ..layer("Skin", "Skin")
            },
            SmartLayer {
                color: Some([0.44, 0.24, 0.18]),
                opacity: 0.3,
                ..SKIN_FLUSH
            },
            SKIN_PORES,
            SmartLayer {
                color: Some([0.20, 0.12, 0.08]),
                ..SKIN_FOLDS
            },
            SmartLayer {
                rough: Some(0.34),
                opacity: 0.6,
                ..SKIN_SHEEN
            },
        ],
    },
    SmartMaterial {
        name: "Skin Stylized",
        layers: &[
            SmartLayer {
                color: Some([0.95, 0.74, 0.62]),
                rough: Some(0.55),
                ..layer("Skin", "Skin")
            },
            // Painted-on blush on the high points.
            SmartLayer {
                color: Some([0.96, 0.56, 0.50]),
                opacity: 0.45,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    noise: Some(0.0),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(CONVEX, 1.2), (NOISE, 0.6)],
                        0.55,
                        0.3,
                    )
                }),
                ..layer("Blush", "Skin")
            },
            // Warm, soft shadow color in the hollows.
            SmartLayer {
                color: Some([0.78, 0.46, 0.40]),
                opacity: 0.5,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    noise: Some(0.0),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 1.5), (CAVITY, 0.8)],
                        0.45,
                        0.3,
                    )
                }),
                ..layer("Warm Shadows", "Skin")
            },
        ],
    },
    // ---------------------------------------------------------------- organic
    SmartMaterial {
        name: "Flesh Raw",
        layers: &[
            SmartLayer {
                color: Some([0.55, 0.12, 0.11]),
                rough: Some(0.25),
                color_var: Some(0.15),
                var_scale: Some(6.0),
                part_var: 0.1,
                ..layer("Muscle", "Skin")
            },
            // Muscle fibers running along the part.
            SmartLayer {
                color: Some([0.38, 0.06, 0.06]),
                height: Some(-0.12),
                opacity: 0.35,
                channels: Some([true, false, false, true]),
                mask: Some(SmartMask {
                    scale: Some(3.0),
                    noise: Some(0.1),
                    ..mix(GeneratorKind::Clouds, &[(GRAIN, 1.0)], 0.45, 0.2)
                }),
                ..layer("Fibers", "Skin")
            },
            // Fat marbling.
            SmartLayer {
                color: Some([0.92, 0.84, 0.74]),
                rough: Some(0.4),
                height: Some(0.08),
                opacity: 0.85,
                mask: Some(SmartMask {
                    scale: Some(4.0),
                    noise: Some(0.3),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(NOISE, 1.0), (STREAKS, 0.3)],
                        0.78,
                        0.04,
                    )
                }),
                ..layer("Fat", "Wax")
            },
            // Dark blood pooled in the hollows.
            SmartLayer {
                color: Some([0.25, 0.02, 0.02]),
                rough: Some(0.1),
                opacity: 0.7,
                mask: Some(SmartMask {
                    noise: Some(0.5),
                    edge_width: Some(0.02),
                    ..mix(
                        GeneratorKind::CavityGrime,
                        &[(CAVITY, 2.0), (OCCLUSION, 1.0)],
                        0.55,
                        0.15,
                    )
                }),
                ..layer("Blood", "Skin")
            },
            // Wet: glossy wherever it isn't tucked away.
            SmartLayer {
                rough: Some(0.08),
                channels: Some(ROUGH_ONLY),
                opacity: 0.6,
                mask: Some(SmartMask {
                    noise: Some(0.6),
                    scale: Some(5.0),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(SKY, 0.6), (CONVEX, 0.8), (NOISE, 0.5)],
                        0.5,
                        0.2,
                    )
                }),
                ..layer("Wet", "Skin")
            },
        ],
    },
    SmartMaterial {
        name: "Reptile Scales",
        layers: &[
            SmartLayer {
                color: Some([0.25, 0.32, 0.15]),
                rough: Some(0.45),
                color_var: Some(0.2),
                var_scale: Some(5.0),
                part_var: 0.1,
                ..layer("Skin", "Skin")
            },
            // Domed scales: highest at each scale's center.
            SmartLayer {
                height: Some(0.35),
                channels: Some(HEIGHT_ONLY),
                mask: Some(SmartMask {
                    scale: Some(3.5),
                    noise: Some(0.0),
                    seed: 3.0,
                    ..mix(GeneratorKind::Clouds, &[(CELLS, -1.0)], -0.35, 0.35)
                }),
                ..layer("Scales", "Skin")
            },
            // Dark skin between the scales.
            SmartLayer {
                color: Some([0.08, 0.10, 0.05]),
                rough: Some(0.7),
                opacity: 0.85,
                mask: Some(SmartMask {
                    scale: Some(3.5),
                    noise: Some(0.0),
                    seed: 3.0,
                    ..mix(GeneratorKind::Clouds, &[(CELLS, 1.0)], 0.6, 0.06)
                }),
                ..layer("Gaps", "Skin")
            },
            // Pale belly, underneath.
            SmartLayer {
                color: Some([0.70, 0.66, 0.45]),
                opacity: 0.9,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    noise: Some(0.5),
                    scale: Some(3.0),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(OVERHANG, 1.4), (NOISE, 0.3)],
                        0.55,
                        0.15,
                    )
                }),
                ..layer("Belly", "Skin")
            },
            // Blotches along the back.
            SmartLayer {
                color: Some([0.12, 0.18, 0.08]),
                opacity: 0.7,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    noise: Some(0.4),
                    scale: Some(2.0),
                    ..mix(GeneratorKind::Clouds, &[(NOISE, 1.0), (UP, 0.5)], 0.7, 0.06)
                }),
                ..layer("Blotches", "Skin")
            },
            SmartLayer {
                rough: Some(0.25),
                channels: Some(ROUGH_ONLY),
                opacity: 0.6,
                mask: Some(SmartMask {
                    noise: Some(0.3),
                    ..mix(GeneratorKind::Clouds, &[(CONVEX, 1.5)], 0.4, 0.2)
                }),
                ..layer("Sheen", "Skin")
            },
        ],
    },
    SmartMaterial {
        name: "Fungus Spotted",
        layers: &[
            SmartLayer {
                color: Some([0.70, 0.10, 0.06]),
                rough: Some(0.45),
                color_var: Some(0.12),
                var_scale: Some(3.0),
                ..layer("Cap", "Skin")
            },
            // Cream gills and stem underneath.
            SmartLayer {
                color: Some([0.88, 0.82, 0.68]),
                rough: Some(0.7),
                mask: Some(SmartMask {
                    noise: Some(0.3),
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(OVERHANG, 1.5), (THIN, 0.4)],
                        0.35,
                        0.12,
                    )
                }),
                ..layer("Gills", "Wax")
            },
            // Raised white warts on top.
            SmartLayer {
                color: Some([0.94, 0.91, 0.84]),
                rough: Some(0.8),
                height: Some(0.2),
                mask: Some(SmartMask {
                    scale: Some(2.0),
                    noise: Some(0.2),
                    invert: true,
                    ..mix(
                        GeneratorKind::Clouds,
                        &[(CELLS, 1.0), (UP, -0.2)],
                        0.06,
                        0.03,
                    )
                }),
                ..layer("Spots", "Wax")
            },
            SmartLayer {
                color: Some([0.30, 0.20, 0.10]),
                opacity: 0.4,
                mask: Some(SmartMask {
                    noise: Some(0.6),
                    ..mix(
                        GeneratorKind::Dirt,
                        &[(OCCLUSION, 1.5), (CAVITY, 1.0)],
                        0.6,
                        0.2,
                    )
                }),
                ..layer("Soil", "Concrete")
            },
        ],
    },
    SmartMaterial {
        name: "Insect Chitin",
        layers: &[
            SmartLayer {
                color: Some([0.07, 0.06, 0.05]),
                rough: Some(0.18),
                color_var: Some(0.15),
                var_scale: Some(4.0),
                part_var: 0.15,
                ..layer("Shell", "Plastic Black")
            },
            // Amber where the shell thins at the edges.
            SmartLayer {
                color: Some([0.45, 0.25, 0.08]),
                opacity: 0.8,
                channels: Some(COLOR_ONLY),
                mask: Some(SmartMask {
                    noise: Some(0.4),
                    edge_width: Some(0.03),
                    ..mix(
                        GeneratorKind::EdgeWear,
                        &[(EDGE, 1.0), (THIN, 0.8)],
                        0.5,
                        0.25,
                    )
                }),
                ..layer("Amber Edges", "Plastic Black")
            },
            // Fine pits.
            SmartLayer {
                height: Some(-0.1),
                rough: Some(0.35),
                channels: Some([false, true, false, true]),
                mask: Some(SmartMask {
                    scale: Some(25.0),
                    noise: Some(0.0),
                    invert: true,
                    ..mix(GeneratorKind::Clouds, &[(CELLS, 1.0)], 0.12, 0.04)
                }),
                ..layer("Pits", "Plastic Black")
            },
            SmartLayer {
                opacity: 0.5,
                ..DUST_LAYER
            },
        ],
    },
    // ---------------------------------------------------------------- eyes
    SmartMaterial {
        name: "Eyeball Blue",
        layers: &EYE_BLUE,
    },
    SmartMaterial {
        name: "Eyeball Brown",
        layers: &EYE_BROWN,
    },
];

impl SmartMaterial {
    pub fn kind(&self) -> super::MaterialType {
        use super::MaterialType;
        match MaterialType::of(self.name, None) {
            MaterialType::Other => self.layers[0].material().kind(),
            kind => kind,
        }
    }

    pub fn len(&self) -> usize {
        self.layers.len() + MICRO_DETAIL.len() + 1
    }

    pub fn preview(&self) -> ([u8; 3], f32, f32) {
        let base = &self.layers[0];
        let material = base.material();
        (
            base.color.unwrap_or(material.color).map(pack),
            base.rough.unwrap_or(material.roughness),
            base.metal.unwrap_or(material.metallic),
        )
    }

    pub fn prepare(&self, size: u32) -> Result<Vec<Layer>, String> {
        let mut planes = std::collections::HashMap::new();
        let mut plane = |rgba| {
            planes
                .entry(rgba)
                .or_insert_with(|| solid(size, rgba))
                .clone()
        };
        let scans = self
            .layers
            .iter()
            .any(|layer| layer.scan.is_some())
            .then(scan_root)
            .transpose()?;
        let mut layers = self
            .layers
            .iter()
            .chain(&MICRO_DETAIL)
            .map(|recipe| recipe.build(scans.as_deref(), &mut plane))
            .collect::<Result<Vec<_>, _>>()?;
        for layer in &mut layers {
            layer.depth = 1;
        }
        layers.push(Layer::new(self.name.to_owned(), LayerKind::Folder));
        Ok(layers)
    }
}

impl SmartLayer {
    fn material(&self) -> &'static MaterialPreset {
        MATERIALS
            .iter()
            .find(|preset| preset.name == self.material)
            .expect("built-in smart material references a built-in base")
    }

    fn build(
        &self,
        scans: Option<&Path>,
        plane: &mut impl FnMut([u8; 4]) -> Arc<Vec<u8>>,
    ) -> Result<Layer, String> {
        let base = self.material();
        let [r, g, b] = self.color.unwrap_or(base.color).map(pack);
        let roughness = pack(self.rough.unwrap_or(base.roughness));
        let metallic = pack(self.metal.unwrap_or(base.metallic));
        let height = pack(0.5 + self.height.unwrap_or(0.) * 0.5);
        let mut variation = texture_fill::Variation {
            color: self.color_var.unwrap_or(base.variation.color),
            roughness: self.rough_var.unwrap_or(base.variation.roughness),
            height: self.height_var.unwrap_or(base.variation.height),
            scale: self.var_scale.unwrap_or(base.variation.scale),
            parts: self.part_var,
            seed: self.seed,
        };
        let mut edit = Layer::new(self.name.to_owned(), LayerKind::Fill);
        edit.opacity = self.opacity * 100.;
        edit.blend = if self.blend == Blend::Add {
            Blend::AddHeight
        } else {
            self.blend
        };
        let enabled = self.channels.unwrap_or([
            true,
            true,
            true,
            self.height.is_some()
                || self.height_var.is_some()
                || variation.height > 0.
                || self.scan.is_some(),
        ]);
        for (channel, rgb) in [[r, g, b], [roughness; 3], [metallic; 3], [height; 3]]
            .into_iter()
            .enumerate()
        {
            if enabled[channel] {
                edit.channels[channel] = Some(plane([rgb[0], rgb[1], rgb[2], 255]));
            }
        }
        // Variations must not allocate channels that the recipe did not enable.
        if !enabled[0] {
            variation.color = 0.;
        }
        if !enabled[1] {
            variation.roughness = 0.;
        }
        if !enabled[3] {
            variation.height = 0.;
        }
        debug_assert!(variation.parts == 0. || (enabled[0] && enabled[1]));
        let mut fill = texture_fill::Fill::default();
        if let Some(scan) = self.scan {
            let root = scans
                .expect("scan root was discovered for scanned materials")
                .join(scan.id);
            for (channel, name) in Scan::FILES {
                let path = root.join(name);
                if channel == 2 && !path.is_file() {
                    continue;
                }
                let bitmap = texture_fill::Bitmap::open(&path)
                    .map_err(|error| format!("{} / {name} · {error}", scan.id))?;
                fill.images[channel] = Some(Arc::new(bitmap));
            }
            fill.settings.projection = texture_fill::Projection::Triplanar;
            fill.settings.tiling = scan.tiling;
            fill.settings.height_strength = scan.height;
        } else {
            fill.settings.variation = variation;
        }
        if fill.images.iter().any(Option::is_some) || fill.settings.variation != Default::default()
        {
            edit.texture_fill = Some(Arc::new(fill));
        }
        if let Some(recipe) = self.mask {
            let mut mask = Mask::new(plane([255, 0, 0, 255]), Vec::new());
            mask.affine = true;
            mask.generator = Some(recipe.generator());
            edit.mask = Some(Arc::new(mask));
        }
        Ok(edit)
    }
}

impl SmartMask {
    fn generator(self) -> Generator {
        let mut generator = Generator::new(self.kind);
        generator.invert = self.invert;
        for (value, replacement) in generator.values.iter_mut().zip([
            self.level.map(percent),
            self.contrast.map(percent),
            self.scale,
            self.noise.map(percent),
            Some(self.seed),
        ]) {
            if let Some(replacement) = replacement {
                *value = replacement;
            }
        }
        if let Some(width) = self.edge_width {
            generator.edge_width = (width * 1000.).round() as u16;
        }
        if !self.weights.is_empty() {
            let mut weights = generator.weights().map(|_| 0);
            for &(index, weight) in self.weights {
                weights[index] = percent(weight).round() as i16;
            }
            generator.weights = Some(weights);
        }
        generator
    }
}

fn scan_root() -> Result<PathBuf, String> {
    let relative = Path::new("assets/paint/pbr");
    let roots = std::env::current_dir()
        .ok()
        .into_iter()
        .chain(std::env::current_exe().ok().into_iter().flat_map(|exe| {
            exe.ancestors()
                .skip(1)
                .take(5)
                .map(Path::to_owned)
                .collect::<Vec<_>>()
        }))
        .chain(std::iter::once(PathBuf::from(env!("CARGO_MANIFEST_DIR"))));
    roots
        .map(|root| root.join(relative))
        .find(|root| root.is_dir())
        .ok_or_else(|| "Папка сканов умных материалов отсутствует".into())
}

fn percent(value: f32) -> f32 {
    value * 100.
}

fn pack(value: f32) -> u8 {
    (value.clamp(0., 1.) * 255.).round() as u8
}

#[cfg(test)]
mod tests {
    use super::super::{PaintAction, Session};
    use super::*;
    use crate::editor::model_paint_data::{Stack, Surface};

    #[test]
    fn paint_smart_material_recipes_load_and_insert_as_one_undoable_group() {
        let mut masks = 0;
        let mut scans = 0;
        let scan_root = scan_root().unwrap();
        let mut names = std::collections::HashSet::new();
        for recipe in SMART_MATERIALS {
            assert!(names.insert(recipe.name), "{}", recipe.name);
            for layer in recipe.layers.iter().chain(&MICRO_DETAIL) {
                if let Some(scan) = layer.scan {
                    scans += 1;
                    for (_, map) in Scan::FILES.into_iter().filter(|(ch, _)| *ch != 2) {
                        assert!(
                            scan_root.join(scan.id).join(map).is_file(),
                            "{} / {map}",
                            scan.id
                        );
                    }
                } else {
                    let built = layer.build(None, &mut |rgba| solid(4, rgba)).unwrap();
                    if let Some(fill) = built.texture_fill {
                        assert!(fill.settings.valid(), "{} / {}", recipe.name, layer.name);
                        if let Some(enabled) = layer.channels {
                            for (channel, varies) in [0, 1, 3].into_iter().zip([
                                fill.settings.variation.color != 0.
                                    || fill.settings.variation.parts != 0.,
                                fill.settings.variation.roughness != 0.
                                    || fill.settings.variation.parts != 0.,
                                fill.settings.variation.height != 0.,
                            ]) {
                                assert!(
                                    !varies || enabled[channel],
                                    "{} / {}",
                                    recipe.name,
                                    layer.name
                                );
                            }
                        }
                    }
                }
                if let Some(mask) = layer.mask {
                    masks += 1;
                    assert!(
                        mask.generator().valid(),
                        "{} / {} · {:?}",
                        recipe.name,
                        layer.name,
                        mask.generator()
                    );
                }
            }
        }
        assert_eq!(SMART_MATERIALS.len(), 63);
        assert_eq!(scans, 19);
        assert!(masks > 100);
        let recipe = SMART_MATERIALS
            .iter()
            .find(|item| item.layers.iter().any(|layer| layer.scan.is_some()))
            .unwrap();
        let layers = recipe.prepare(4).unwrap();
        let fill = layers[0].texture_fill.as_ref().unwrap();
        assert_eq!(
            fill.settings.projection,
            texture_fill::Projection::Triplanar
        );
        let scan = recipe.layers[0].scan.unwrap();
        assert_eq!(fill.settings.height_strength, scan.height);
        assert_eq!(fill.images[3].as_ref().unwrap().name, "height.jpg");

        let mut session = Session {
            stack: Stack {
                layers: vec![Layer::new("Parent".into(), LayerKind::Folder)],
                selected: 0,
            },
            texture_size: 4,
            ..Session::default()
        };
        let before = session.stack.clone();
        session.insert_smart_material(layers, session.insertion());
        assert!(session.stack.valid_hierarchy());
        assert_eq!(
            session.stack.layers[session.stack.selected].name,
            recipe.name
        );
        assert_eq!(
            session.stack.block(session.stack.selected).len(),
            recipe.len()
        );
        assert_eq!(session.stack.layers[session.stack.selected].depth, 1);
        assert_eq!(session.stack.layers[0].depth, 2);
        session.edit(PaintAction::Undo);
        assert_eq!(session.stack, before);
    }

    #[test]
    fn paint_smart_material_neutral_height_does_not_raise_the_surface() {
        let mut layer = Layer::new("Relief".into(), LayerKind::Fill);
        layer.blend = Blend::AddHeight;
        layer.channels[3] = Some(solid(1, [128, 128, 128, 255]));
        let mut surface = Surface {
            size: 1,
            base: std::array::from_fn(|_| solid(1, [160, 160, 160, 255])),
            stack: Stack {
                layers: vec![layer],
                selected: 0,
            },
            material: serde_json::Value::Null,
            baked: None,
        };
        assert_eq!(&surface.composite(3)[..4], &[160, 160, 160, 255]);
        surface.stack.layers[0].channels[3] = Some(solid(1, [90, 90, 90, 255]));
        assert_eq!(&surface.composite(3)[..4], &[122, 122, 122, 255]);
    }
}
