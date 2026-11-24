use super::{
    Depth, ExportDirectory, Padding, Samples, bake, image_output, material_normal,
    material_normal_rows, pad,
};
use crate::editor::model_paint_data::{CHANNEL_COUNT, CompositeSample, Surface};
use bake::BakeMap;
use bevy::prelude::{LinearRgba, Srgba, URect, UVec2};
use std::io;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::editor) enum Template {
    #[default]
    Separate,
    Unreal,
    UnityUrp,
    UnityHdrp,
    UnityStandard,
    Godot,
    Gltf,
    Roblox,
    Blender,
    MetallicRoughness,
    SpecularGlossiness,
    Baked,
    Custom(usize),
}

impl Template {
    pub(in crate::editor) const ALL: [Self; 12] = [
        Self::Separate,
        Self::Unreal,
        Self::UnityUrp,
        Self::UnityHdrp,
        Self::UnityStandard,
        Self::Godot,
        Self::Gltf,
        Self::Roblox,
        Self::Blender,
        Self::MetallicRoughness,
        Self::SpecularGlossiness,
        Self::Baked,
    ];

    pub(in crate::editor) fn label(self) -> &'static str {
        match self {
            Self::Separate => "Все карты",
            Self::Unreal => "Unreal Engine",
            Self::UnityUrp => "Unity URP",
            Self::UnityHdrp => "Unity HDRP",
            Self::UnityStandard => "Unity Standard",
            Self::Godot => "Godot 4",
            Self::Gltf => "glTF",
            Self::Roblox => "Roblox",
            Self::Blender => "Blender",
            Self::MetallicRoughness => "Metallic/Roughness",
            Self::SpecularGlossiness => "Specular/Glossiness",
            Self::Baked => "Карты запекания",
            Self::Custom(_) => "Пользовательский шаблон",
        }
    }

    pub(super) fn maps(self) -> &'static [Map<'static>] {
        match self {
            Self::Separate | Self::Baked | Self::Custom(_) => &[],
            Self::Unreal => {
                const {
                    &[
                        BASE,
                        NORMAL_DX,
                        packed("OcclusionRoughnessMetallic", ORM),
                        EMISSIVE,
                        SPEC_LEVEL,
                    ]
                }
            }
            Self::UnityUrp => {
                const {
                    &[
                        BASE.named("BaseMap"),
                        NORMAL,
                        packed("MaskMap", &[METAL, AO, Zero, Smoothness]),
                        HEIGHT,
                        EMISSIVE.named("Emission"),
                    ]
                }
            }
            Self::UnityHdrp => {
                const {
                    &[
                        BASE.named("BaseMap"),
                        NORMAL,
                        packed("MaskMap", &[METAL, AO, One, Smoothness]),
                        HEIGHT,
                        EMISSIVE,
                    ]
                }
            }
            Self::UnityStandard => {
                const {
                    &[
                        BASE.named("Albedo"),
                        packed("MetallicSmoothness", &[METAL, METAL, METAL, Smoothness]),
                        NORMAL,
                        packed("Occlusion", &[AO]),
                        HEIGHT,
                        EMISSIVE.named("Emission"),
                    ]
                }
            }
            Self::Godot => {
                const {
                    &[
                        BASE.named("Albedo"),
                        packed("ORM", ORM),
                        NORMAL,
                        HEIGHT,
                        EMISSIVE.named("Emission"),
                    ]
                }
            }
            Self::Gltf => {
                const {
                    &[
                        BASE.named("baseColor"),
                        packed("occlusionRoughnessMetallic", ORM),
                        NORMAL.named("normal"),
                        EMISSIVE.named("emissive"),
                    ]
                }
            }
            Self::Roblox => {
                const {
                    &[
                        BASE.named("Color"),
                        NORMAL,
                        ROUGHNESS,
                        METALLIC.named("Metalness"),
                    ]
                }
            }
            Self::Blender => {
                const {
                    &[
                        BASE,
                        ROUGHNESS,
                        METALLIC,
                        NORMAL,
                        HEIGHT,
                        EMISSIVE.named("Emission"),
                        SPEC_LEVEL,
                    ]
                }
            }
            Self::MetallicRoughness => {
                const {
                    &[
                        BASE,
                        NORMAL,
                        ROUGHNESS,
                        METALLIC,
                        packed("AmbientOcclusion", &[AO]),
                        HEIGHT,
                        EMISSIVE,
                        SPEC_LEVEL,
                    ]
                }
            }
            Self::SpecularGlossiness => {
                const {
                    &[
                        packed("Diffuse", &[Diffuse(0), Diffuse(1), Diffuse(2), OPACITY]),
                        packed("Specular", &[Specular(0), Specular(1), Specular(2)]),
                        packed("Glossiness", &[Smoothness]),
                        NORMAL,
                        packed("AmbientOcclusion", &[AO]),
                        HEIGHT,
                        EMISSIVE,
                    ]
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Component {
    Layer(usize, usize),
    Normal(usize, bool),
    Baked(BakeMap, usize),
    Smoothness,
    Diffuse(usize),
    Specular(usize),
    Zero,
    One,
}
use self::Component::*;

#[derive(Clone, Copy)]
pub(super) struct Map<'a> {
    pub name: &'a str,
    pub components: &'a [Component],
    pub skip_if_default: Option<(usize, u8)>,
    pub inverted: u8,
}

impl Map<'static> {
    const fn named(self, name: &'static str) -> Self {
        Self { name, ..self }
    }
}

const fn packed(name: &'static str, components: &'static [Component]) -> Map<'static> {
    Map {
        name,
        components,
        skip_if_default: None,
        inverted: 0,
    }
}

const AO: Component = Baked(BakeMap::Occlusion, 0);
const ROUGH: Component = Layer(1, 0);
const METAL: Component = Layer(2, 0);
const OPACITY: Component = Layer(4, 0);
const ORM: &[Component] = &[AO, ROUGH, METAL];
const BASE: Map = packed(
    "BaseColor",
    &[Layer(0, 0), Layer(0, 1), Layer(0, 2), OPACITY],
);
const ROUGHNESS: Map = packed("Roughness", &[ROUGH]);
const METALLIC: Map = packed("Metallic", &[METAL]);
const HEIGHT: Map = packed("Height", &[Layer(3, 0)]);
const NORMAL: Map = packed(
    "Normal",
    &[Normal(0, false), Normal(1, false), Normal(2, false)],
);
const NORMAL_DX: Map = packed(
    "Normal",
    &[Normal(0, false), Normal(1, true), Normal(2, false)],
);
const EMISSIVE: Map = Map {
    skip_if_default: Some((5, 0)),
    ..packed("Emissive", &[Layer(5, 0), Layer(5, 1), Layer(5, 2)])
};
const SPEC_LEVEL: Map = Map {
    skip_if_default: Some((7, 128)),
    ..packed("Specular", &[Layer(7, 0)])
};

pub(super) fn write(
    output: &mut ExportDirectory,
    stem: &str,
    baked: Option<&bake::Maps>,
    surface: &Surface,
    suffix: &str,
    sources: &[u32],
    padding: Padding,
    template: Template,
) -> io::Result<()> {
    let separate = [
        BASE,
        packed("Opacity", &[OPACITY]),
        ROUGHNESS,
        METALLIC,
        packed("SpecularLevel", &[Layer(7, 0)]),
        HEIGHT,
        NORMAL,
        packed("Emissive", &[Layer(5, 0), Layer(5, 1), Layer(5, 2)]),
    ];
    let maps = if template == Template::Separate {
        &separate[..]
    } else {
        template.maps()
    };
    for map in maps {
        let samples = match map.components {
            [Normal(..), ..] => Samples::Normal,
            [Layer(3, 0)] if template != Template::Separate => Samples::Height,
            [Layer(0 | 5, _), ..] | [Diffuse(_), ..] | [Specular(_), ..] => Samples::Color,
            _ => Samples::Data,
        };
        write_map(
            output,
            format!("{stem}_{name}{suffix}", name = map.name),
            *map,
            surface,
            baked,
            sources,
            padding,
            samples,
            template != Template::Separate,
        )?;
    }
    Ok(())
}

pub(super) fn write_map(
    output: &mut ExportDirectory,
    name: String,
    map: Map<'_>,
    surface: &Surface,
    baked: Option<&bake::Maps>,
    sources: &[u32],
    padding: Padding,
    samples: Samples,
    allow_opaque_rgb: bool,
) -> io::Result<()> {
    let normal_options = output.options.normal;
    if image_output::output_depth(samples, output.options) == Depth::U8 {
        if let Some(mut pixels) = evaluate(
            map,
            surface,
            baked,
            padding,
            allow_opaque_rgb,
            surface.size,
            |_| {
                material_normal(
                    surface,
                    surface.composite(6),
                    &surface.composite(3),
                    baked,
                    normal_options,
                )
            },
        ) {
            let channels = pixels.len() / surface.size.pow(2) as usize;
            pad(&mut pixels, channels, sources, padding, false);
            output.image(name, surface.size, pixels, channels, samples)?;
        }
    } else if let Some(mut pixels) =
        evaluate(map, surface, baked, padding, allow_opaque_rgb, 64, |rect| {
            material_normal_rows(surface, rect, baked, normal_options)
        })
    {
        let channels = pixels.len() / surface.size.pow(2) as usize;
        pad(&mut pixels, channels, sources, padding, false);
        output.float_image(name, surface.size, pixels, channels, samples)?;
    }
    Ok(())
}

fn evaluate<T: CompositeSample>(
    map: Map<'_>,
    surface: &Surface,
    baked: Option<&bake::Maps>,
    padding: Padding,
    allow_opaque_rgb: bool,
    stripe_height: u32,
    read_normal: impl Fn(URect) -> Vec<T>,
) -> Option<Vec<T>> {
    let mut required = [false; CHANNEL_COUNT];
    let mut baked_maps = Vec::new();
    for &component in map.components {
        let layers: &[usize] = match &component {
            Layer(layer, _) => std::slice::from_ref(layer),
            Smoothness => &[1],
            Diffuse(_) => &[0, 2],
            Specular(_) => &[0, 2, 7],
            Baked(map, _) => {
                if let Some(baked) = baked.filter(|baked| baked.has_map(*map))
                    && !baked_maps.iter().any(|(kind, _)| kind == map)
                {
                    baked_maps.push((*map, baked.rgba(*map)));
                }
                &[]
            }
            _ => &[],
        };
        for &layer in layers {
            required[layer] = true;
        }
    }
    if let Some((layer, _)) = map.skip_if_default {
        required[layer] = true;
    }
    let has_normal = map
        .components
        .iter()
        .any(|component| matches!(component, Normal(..)));
    let can_drop_alpha = allow_opaque_rgb
        && map.components.len() == 4
        && padding != Padding::Transparent
        && map.inverted & 8 == 0
        && matches!(map.components.last(), Some(Layer(4, 0)));
    let components: Vec<_> = map
        .components
        .iter()
        .map(|&component| {
            let plane = match component {
                Baked(kind, _) => baked_maps
                    .iter()
                    .find(|(map, _)| *map == kind)
                    .map(|(_, pixels)| pixels.as_slice()),
                _ => None,
            };
            (component, plane)
        })
        .collect();
    let size = surface.size as usize;
    let side = baked.map_or(1, |baked| baked.size as usize);
    let count = components.len();
    let mut pixels = Vec::with_capacity(size * size * count);
    let mut default_only = map.skip_if_default.is_some();
    let mut opaque = can_drop_alpha;
    for first in (0..surface.size).step_by(stripe_height as usize) {
        let rect = URect {
            min: UVec2::new(0, first),
            max: UVec2::new(surface.size, (first + stripe_height).min(surface.size)),
        };
        let layers: [Option<Vec<T>>; CHANNEL_COUNT] = std::array::from_fn(|layer| {
            required[layer].then(|| surface.composite_samples(layer, rect))
        });
        let normal = has_normal.then(|| read_normal(rect));
        if let Some((layer, default)) = map.skip_if_default.filter(|_| default_only) {
            default_only = layers[layer]
                .as_ref()
                .unwrap()
                .chunks_exact(4)
                .all(|rgba| rgba[..3] == [T::from_byte(default); 3]);
        }
        if opaque {
            opaque = layers[4]
                .as_ref()
                .unwrap()
                .chunks_exact(4)
                .all(|rgba| rgba[0] == T::from_byte(255));
        }
        for local in 0..size * rect.height() as usize {
            let pixel = first as usize * size + local;
            let i = local * 4;
            let value = |layer: usize, channel| layers[layer].as_ref().unwrap()[i + channel];
            for (output_channel, &(component, baked_pixels)) in components.iter().enumerate() {
                let sample = match component {
                    Layer(layer, channel) => value(layer, channel),
                    Normal(channel, invert) => {
                        let sample = normal.as_ref().unwrap()[i + channel];
                        if invert {
                            T::from_unit(1. - sample.unit())
                        } else {
                            sample
                        }
                    }
                    Baked(kind, channel) => {
                        let i = ((pixel / size * side / size) * side
                            + (pixel % size * side / size))
                            * 4;
                        T::from_byte(
                            baked_pixels
                                .filter(|pixels| pixels[i + 3] != 0)
                                .map_or_else(
                                    || default_baked(kind, channel),
                                    |pixels| pixels[i + channel],
                                ),
                        )
                    }
                    Smoothness => T::from_unit(1. - value(1, 0).unit()),
                    Diffuse(channel) | Specular(channel) => {
                        let base =
                            LinearRgba::from(Srgba::new(value(0, channel).unit(), 0., 0., 1.)).red;
                        let metal = value(2, 0).unit();
                        let linear = if matches!(component, Specular(_)) {
                            let f0 = 0.08 * value(7, 0).unit();
                            f0 + (base - f0) * metal
                        } else {
                            base * (1. - metal)
                        };
                        T::from_unit(Srgba::from(LinearRgba::new(linear, 0., 0., 1.)).red)
                    }
                    Zero => T::from_byte(0),
                    One => T::from_byte(255),
                };
                pixels.push(if map.inverted & (1 << output_channel) != 0 {
                    T::from_unit(1. - sample.unit())
                } else {
                    sample
                });
            }
        }
    }
    if default_only {
        return None;
    }
    if opaque {
        for pixel in 1..size * size {
            pixels.copy_within(pixel * 4..pixel * 4 + 3, pixel * 3);
        }
        pixels.truncate(size * size * 3);
    }
    Some(pixels)
}

fn default_baked(map: BakeMap, channel: usize) -> u8 {
    match map {
        BakeMap::Occlusion | BakeMap::Sky => 255,
        BakeMap::ProjectedNormal | BakeMap::WorldNormal | BakeMap::BentNormal => {
            [128, 128, 255][channel]
        }
        BakeMap::ProjectedHeight => 128,
        _ => 0,
    }
}
