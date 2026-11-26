//! Material Painter's portable JSON export recipes.
use super::{ExportDirectory, Padding, Samples, bake::BakeMap, safe_stem, templates};
use crate::editor::model_paint_data::Surface;
use serde::{Deserialize, Serialize};
use std::{io, path::Path};
use templates::Component;

pub(super) const MAX_TEMPLATES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::editor::ui::model_viewer::paint) struct Recipe {
    pub name: String,
    pub maps: Vec<Output>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::editor::ui::model_viewer::paint) struct Output {
    pub name: String,
    pub layout: Layout,
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub force_16: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::editor) enum Layout {
    Grayscale,
    #[serde(rename = "RGB")]
    Rgb,
    #[serde(rename = "RGBA")]
    Rgba,
}

impl Layout {
    pub(in crate::editor::ui::model_viewer::paint) fn len(self) -> usize {
        match self {
            Self::Grayscale => 1,
            Self::Rgb => 3,
            Self::Rgba => 4,
        }
    }

    pub(in crate::editor::ui::model_viewer::paint) const ALL: [Self; 3] =
        [Self::Grayscale, Self::Rgb, Self::Rgba];
    pub(in crate::editor::ui::model_viewer::paint) fn label(self) -> &'static str {
        match self {
            Self::Grayscale => "Серый",
            Self::Rgb => "RGB",
            Self::Rgba => "RGBA",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::editor::ui::model_viewer::paint) struct Channel {
    pub src: Source,
    #[serde(default)]
    pub comp: u8,
    #[serde(default)]
    pub invert: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::editor) enum Source {
    BaseColor,
    Diffuse,
    Specular,
    SpecularLevel,
    Roughness,
    Glossiness,
    Metallic,
    Height,
    Opacity,
    Emission,
    NormalGl,
    NormalDx,
    #[serde(rename = "mesh:AmbientOcclusion")]
    Occlusion,
    #[serde(rename = "mesh:Curvature")]
    Curvature,
    #[serde(rename = "mesh:Cavity")]
    Cavity,
    #[serde(rename = "mesh:Thickness")]
    Thickness,
    #[serde(rename = "mesh:SkyExposure")]
    Sky,
    #[serde(rename = "mesh:EdgeDistance")]
    EdgeDistance,
    #[serde(rename = "mesh:CreaseDistance")]
    CreaseDistance,
    #[serde(rename = "mesh:PartRandom")]
    PartRandom,
    #[serde(rename = "mesh:PartGradient")]
    PartGradient,
    #[serde(rename = "mesh:BentNormal")]
    BentNormal,
    #[serde(rename = "mesh:Normal")]
    ProjectedNormal,
    #[serde(rename = "mesh:Height")]
    ProjectedHeight,
    #[serde(rename = "mesh:ID")]
    PartId,
    WorldNormal,
    Position,
    Zero,
    One,
}

impl Source {
    pub(in crate::editor::ui::model_viewer::paint) const ALL: &[(Self, &str)] = &[
        (Self::BaseColor, "Базовый цвет"),
        (Self::Diffuse, "Diffuse"),
        (Self::Specular, "Specular"),
        (Self::SpecularLevel, "Сила блика"),
        (Self::Roughness, "Шероховатость"),
        (Self::Glossiness, "Гладкость"),
        (Self::Metallic, "Металличность"),
        (Self::Height, "Высота"),
        (Self::Opacity, "Непрозрачность"),
        (Self::Emission, "Свечение"),
        (Self::NormalGl, "Нормаль OpenGL"),
        (Self::NormalDx, "Нормаль DirectX"),
        (Self::Occlusion, "AO"),
        (Self::Curvature, "Кривизна"),
        (Self::Cavity, "Впадины"),
        (Self::Thickness, "Толщина"),
        (Self::Sky, "Открытость небу"),
        (Self::EdgeDistance, "Расстояние до края"),
        (Self::CreaseDistance, "Расстояние до складки"),
        (Self::PartRandom, "Разброс по деталям"),
        (Self::PartGradient, "Градиент детали"),
        (Self::BentNormal, "Нормаль видимости"),
        (Self::ProjectedNormal, "Нормаль high-poly"),
        (Self::ProjectedHeight, "Высота high-poly"),
        (Self::PartId, "ID детали"),
        (Self::WorldNormal, "Мировая нормаль"),
        (Self::Position, "Позиция"),
        (Self::Zero, "Чёрный (0)"),
        (Self::One, "Белый (1)"),
    ];

    pub(in crate::editor::ui::model_viewer::paint) fn label(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(source, _)| *source == self)
            .unwrap()
            .1
    }

    pub(in crate::editor::ui::model_viewer::paint) fn has_components(self) -> bool {
        self.color() || self.normal() || matches!(self, Self::PartId | Self::Position)
    }
    fn component(self, component: u8) -> Component {
        use Component::{Baked, Layer, Normal, Smoothness};
        let component = usize::from(component.min(2));
        match self {
            Self::BaseColor => Layer(0, component),
            Self::Diffuse => Component::Diffuse(component),
            Self::Specular => Component::Specular(component),
            Self::SpecularLevel => Layer(7, 0),
            Self::Roughness => Layer(1, 0),
            Self::Glossiness => Smoothness,
            Self::Metallic => Layer(2, 0),
            Self::Height => Layer(3, 0),
            Self::Opacity => Layer(4, 0),
            Self::Emission => Layer(5, component),
            Self::NormalGl => Normal(component, false),
            Self::NormalDx => Normal(component, component == 1),
            Self::Occlusion => Baked(BakeMap::Occlusion, 0),
            Self::Curvature => Baked(BakeMap::Curvature, 0),
            Self::Cavity => Baked(BakeMap::Cavity, 0),
            Self::Thickness => Baked(BakeMap::Thickness, 0),
            Self::Sky => Baked(BakeMap::Sky, 0),
            Self::EdgeDistance => Baked(BakeMap::EdgeDistance, 0),
            Self::CreaseDistance => Baked(BakeMap::CreaseDistance, 0),
            Self::PartRandom => Baked(BakeMap::PartRandom, 0),
            Self::PartGradient => Baked(BakeMap::PartGradient, 0),
            Self::BentNormal => Baked(BakeMap::BentNormal, component),
            Self::ProjectedNormal => Baked(BakeMap::ProjectedNormal, component),
            Self::ProjectedHeight => Baked(BakeMap::ProjectedHeight, 0),
            Self::PartId => Baked(BakeMap::PartId, component),
            Self::WorldNormal => Baked(BakeMap::WorldNormal, component),
            Self::Position => Baked(BakeMap::Position, component),
            Self::Zero => Component::Zero,
            Self::One => Component::One,
        }
    }

    fn color(self) -> bool {
        matches!(
            self,
            Self::BaseColor | Self::Diffuse | Self::Specular | Self::Emission
        )
    }

    fn normal(self) -> bool {
        matches!(
            self,
            Self::NormalGl
                | Self::NormalDx
                | Self::WorldNormal
                | Self::BentNormal
                | Self::ProjectedNormal
        )
    }
}

pub(super) fn load(path: &Path) -> io::Result<Vec<Recipe>> {
    let bytes = bevy_scene_format::read_limited(path, 1024 * 1024)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let recipes = if value.is_array() {
        serde_json::from_value::<Vec<Recipe>>(value)?
    } else {
        vec![serde_json::from_value::<Recipe>(value)?]
    };
    if recipes.is_empty() || recipes.len() > MAX_TEMPLATES {
        return Err(io::Error::other(
            "Файл должен содержать от 1 до 64 шаблонов",
        ));
    }
    for recipe in &recipes {
        recipe.validate()?;
    }
    Ok(recipes)
}

impl Recipe {
    pub(super) fn from_builtin(template: templates::Template) -> Option<Self> {
        use Source::*;
        use templates::Template;
        let maps = match template {
            Template::Custom(_) => return None,
            Template::Separate => vec![
                Output::source("$mesh_BaseColor", Layout::Rgba, BaseColor),
                Output::source("$mesh_Opacity", Layout::Grayscale, Opacity),
                Output::source("$mesh_Roughness", Layout::Grayscale, Roughness),
                Output::source("$mesh_Metallic", Layout::Grayscale, Metallic),
                Output::source("$mesh_SpecularLevel", Layout::Grayscale, SpecularLevel),
                Output::source("$mesh_Height", Layout::Grayscale, Height),
                Output::source("$mesh_Normal", Layout::Rgb, NormalGl),
                Output::source("$mesh_Emissive", Layout::Rgb, Emission),
            ],
            Template::Baked => [
                ("AmbientOcclusion", Occlusion),
                ("Curvature", Curvature),
                ("Cavity", Cavity),
                ("Thickness", Thickness),
                ("SkyExposure", Sky),
                ("EdgeDistance", EdgeDistance),
                ("CreaseDistance", CreaseDistance),
                ("PartRandom", PartRandom),
                ("PartGradient", PartGradient),
                ("BentNormal", BentNormal),
                ("Normal", ProjectedNormal),
                ("Height", ProjectedHeight),
                ("ID", PartId),
                ("WorldSpaceNormal", WorldNormal),
                ("Position", Position),
            ]
            .into_iter()
            .map(|(name, source)| {
                Output::source(
                    &format!("$mesh_{name}"),
                    if source.has_components() {
                        Layout::Rgb
                    } else {
                        Layout::Grayscale
                    },
                    source,
                )
            })
            .collect(),
            _ => {
                let prefix = if template == Template::Unreal {
                    "T_$mesh"
                } else {
                    "$mesh"
                };
                template
                    .maps()
                    .iter()
                    .map(|map| {
                        let layout = match map.components.len() {
                            1 => Layout::Grayscale,
                            3 => Layout::Rgb,
                            _ => Layout::Rgba,
                        };
                        let directx = map
                            .components
                            .iter()
                            .any(|c| matches!(c, Component::Normal(1, true)));
                        let channels = map
                            .components
                            .iter()
                            .map(|component| {
                                let (src, comp) = match *component {
                                    Component::Layer(0, i) => (BaseColor, i),
                                    Component::Layer(1, _) => (Roughness, 0),
                                    Component::Layer(2, _) => (Metallic, 0),
                                    Component::Layer(3, _) => (Height, 0),
                                    Component::Layer(4, _) => (Opacity, 0),
                                    Component::Layer(5, i) => (Emission, i),
                                    Component::Layer(7, _) => (SpecularLevel, 0),
                                    Component::Normal(i, _) => {
                                        (if directx { NormalDx } else { NormalGl }, i)
                                    }
                                    Component::Smoothness => (Glossiness, 0),
                                    Component::Diffuse(i) => (Diffuse, i),
                                    Component::Specular(i) => (Specular, i),
                                    Component::Baked(BakeMap::Occlusion, _) => (Occlusion, 0),
                                    Component::Zero => (Zero, 0),
                                    Component::One => (One, 0),
                                    _ => unreachable!(
                                        "unknown component in the built-in export profile"
                                    ),
                                };
                                Channel {
                                    src,
                                    comp: comp as u8,
                                    invert: false,
                                }
                            })
                            .collect();
                        Output {
                            name: format!("{prefix}_{}", map.name),
                            layout,
                            channels,
                            force_16: matches!(map.components, [Component::Layer(3, _)]),
                        }
                    })
                    .collect()
            }
        };
        Some(Self {
            name: template.label().to_owned(),
            maps,
        })
    }

    pub(super) fn write_json(&self, folder: &Path) -> io::Result<std::path::PathBuf> {
        use bevy_scene_format::{Revision, atomic_write};
        self.validate()?;
        let stem = safe_stem(&self.name);
        let bytes = serde_json::to_vec_pretty(&[self])?;
        for index in 1.. {
            let name = if index == 1 {
                format!("{stem}_export.json")
            } else {
                format!("{stem}_export_{index}.json")
            };
            let path = folder.join(name);
            if path.exists() {
                continue;
            }
            match atomic_write(&path, &bytes, Revision::Missing) {
                Ok(_) => return Ok(path),
                Err(_) if path.exists() => continue,
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }

    pub(super) fn apply_edit(
        &mut self,
        output: usize,
        channel: usize,
        edit: Edit,
        text: Option<&str>,
    ) -> io::Result<()> {
        let invalid = || io::Error::other("Карта или канал не найдены");
        match edit {
            Edit::RenameTemplate => self.name = text.ok_or_else(invalid)?.trim().to_owned(),
            Edit::Add => {
                let mut index = self.maps.len() + 1;
                while self
                    .maps
                    .iter()
                    .any(|map| map.name == format!("$mesh_Map{index}"))
                {
                    index += 1;
                }
                self.maps.push(Output::source(
                    &format!("$mesh_Map{index}"),
                    Layout::Grayscale,
                    Source::Roughness,
                ));
            }
            Edit::Remove => {
                if self.maps.len() <= 1 || output >= self.maps.len() {
                    return Err(invalid());
                }
                self.maps.remove(output);
            }
            Edit::Move(up) => {
                let offset = if up { -1 } else { 1 };
                if output >= self.maps.len() {
                    return Err(invalid());
                }
                let target = output
                    .checked_add_signed(offset)
                    .filter(|&index| index < self.maps.len())
                    .ok_or_else(invalid)?;
                self.maps.swap(output, target);
            }
            _ => {
                let map = self.maps.get_mut(output).ok_or_else(invalid)?;
                match edit {
                    Edit::RenameOutput => map.name = text.ok_or_else(invalid)?.trim().to_owned(),
                    Edit::SetLayout(layout) => {
                        for index in map.channels.len()..layout.len() {
                            map.channels.push(Channel {
                                src: if index == 3 {
                                    Source::One
                                } else {
                                    Source::Zero
                                },
                                comp: 0,
                                invert: false,
                            });
                        }
                        map.layout = layout;
                    }
                    Edit::ToggleDepth => map.force_16 = !map.force_16,
                    _ => {
                        let ch = map.channels[..map.layout.len()]
                            .get_mut(channel)
                            .ok_or_else(invalid)?;
                        match edit {
                            Edit::SetSource(source) => {
                                ch.src = source;
                                if !source.has_components() {
                                    ch.comp = 0;
                                }
                            }
                            Edit::SetComponent(component) if component <= 2 => ch.comp = component,
                            Edit::ToggleInvert => ch.invert = !ch.invert,
                            _ => return Err(invalid()),
                        }
                    }
                }
            }
        }
        self.validate()
    }

    pub(super) fn validate(&self) -> io::Result<()> {
        if self.name.trim().is_empty()
            || self.name.chars().count() > 100
            || self.name.chars().any(char::is_control)
            || self.maps.is_empty()
            || self.maps.len() > 32
        {
            return Err(io::Error::other(
                "Шаблон должен иметь имя и от 1 до 32 выходных карт",
            ));
        }
        let mut names = std::collections::HashSet::new();
        for map in &self.maps {
            if map.name.trim().is_empty()
                || map.name.chars().count() > 100
                || map.name.chars().any(char::is_control)
                || map.name.contains(['/', '\\', ':'])
                || !(map.layout.len()..=4).contains(&map.channels.len())
                || map.channels.iter().any(|channel| channel.comp > 2)
            {
                return Err(io::Error::other(format!(
                    "Недопустимое имя или каналы карты «{}»",
                    map.name
                )));
            }
            let check = ["$mesh", "$set", "$project", "$udim"]
                .into_iter()
                .fold(map.name.clone(), |name, token| name.replace(token, ""));
            if check.contains('$') {
                return Err(io::Error::other(format!(
                    "Неизвестная переменная имени «{}»",
                    map.name
                )));
            }
            if !names.insert(map.name.trim().to_lowercase()) {
                return Err(io::Error::other(format!(
                    "Имя карты «{}» повторяется",
                    map.name
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Edit {
    RenameTemplate,
    RenameOutput,
    Add,
    Remove,
    Move(bool),
    SetLayout(Layout),
    ToggleDepth,
    SetSource(Source),
    SetComponent(u8),
    ToggleInvert,
}

impl Output {
    fn source(name: &str, layout: Layout, source: Source) -> Self {
        let channels = (0..layout.len())
            .map(|index| Channel {
                src: if index == 3 { Source::Opacity } else { source },
                comp: if index < 3 && source.has_components() {
                    index as u8
                } else {
                    0
                },
                invert: false,
            })
            .collect();
        Self {
            name: name.into(),
            layout,
            channels,
            force_16: source == Source::ProjectedHeight,
        }
    }
}

pub(super) struct Names<'a> {
    pub mesh: &'a str,
    pub project: &'a str,
    pub set: &'a str,
    pub tile: Option<u32>,
    pub multiple_sets: bool,
}

impl Names<'_> {
    pub(super) fn stem(&self, pattern: &str) -> String {
        let name = if self.multiple_sets && !pattern.contains("$set") {
            if pattern.contains("$mesh") {
                pattern.replace("$mesh", "$mesh_$set")
            } else {
                format!("$set_{pattern}")
            }
        } else {
            pattern.to_owned()
        };
        let mut name = name
            .replace("$mesh", &safe_stem(self.mesh))
            .replace("$set", self.set)
            .replace("$project", &safe_stem(self.project));
        if let Some(tile) = self.tile {
            if name.contains("$udim") {
                name = name.replace("$udim", &tile.to_string());
            } else {
                name.push_str(&format!(".{tile}"));
            }
        } else {
            name = name.replace("$udim", "");
        }
        let name: String = name
            .chars()
            .map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c })
            .collect();
        let name = name.trim_matches([' ', '.']);
        if name.is_empty() {
            "Texture".into()
        } else {
            name.into()
        }
    }
}

pub(super) fn write(
    recipe: &Recipe,
    output: &mut ExportDirectory,
    names: Names<'_>,
    surface: &Surface,
    baked: Option<&super::bake::Maps>,
    sources: &[u32],
    padding: Padding,
) -> io::Result<()> {
    for map in &recipe.maps {
        let channels = &map.channels[..map.layout.len()];
        let components: Vec<_> = channels
            .iter()
            .map(|channel| channel.src.component(channel.comp))
            .collect();
        let flag = |apply: fn(&Channel) -> bool| {
            channels
                .iter()
                .enumerate()
                .fold(0u8, |mask, (index, channel)| {
                    mask | if apply(channel) { 1 << index } else { 0 }
                })
        };
        let plan = templates::Map {
            name: &map.name,
            components: &components,
            skip_if_default: None,
            inverted: flag(|channel| channel.invert),
        };
        let normal = channels.len() >= 3
            && channels[0].src.normal()
            && channels[..3].iter().enumerate().all(|(index, channel)| {
                channel.src == channels[0].src && usize::from(channel.comp) == index
            });
        let samples = Samples::Packed {
            srgb: flag(|channel| channel.src.color()),
            normal,
            force_16: map.force_16,
        };
        templates::write_map(
            output,
            names.stem(&map.name),
            plan,
            surface,
            baked,
            sources,
            padding,
            samples,
            false,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use templates::Template;

    #[test]
    fn export_recipe_copies_builtins_and_round_trips_an_edited_template() {
        use Edit::*;
        for profile in Template::ALL {
            Recipe::from_builtin(profile).unwrap().validate().unwrap();
        }
        let mut recipe = Recipe::from_builtin(Template::Unreal).unwrap();
        assert_eq!(recipe.maps[1].channels[1].src, Source::NormalDx);
        assert!(!recipe.maps[1].channels[1].invert);
        let untouched = recipe.clone();
        recipe
            .apply_edit(0, 0, RenameTemplate, Some("Карты корабля"))
            .unwrap();
        recipe
            .apply_edit(2, 0, RenameOutput, Some("$project_$mesh_$udim_ORM"))
            .unwrap();
        recipe
            .apply_edit(2, 0, SetLayout(Layout::Rgba), None)
            .unwrap();
        recipe
            .apply_edit(2, 3, SetSource(Source::BaseColor), None)
            .unwrap();
        recipe.apply_edit(2, 3, SetComponent(2), None).unwrap();
        recipe.apply_edit(2, 3, ToggleInvert, None).unwrap();
        recipe.apply_edit(2, 3, ToggleDepth, None).unwrap();
        recipe.apply_edit(2, 0, Move(true), None).unwrap();
        recipe.apply_edit(2, 0, Remove, None).unwrap();
        recipe.apply_edit(0, 0, Add, None).unwrap();
        assert_eq!(recipe.maps.len(), untouched.maps.len());
        assert_eq!(
            recipe.maps[1].channels[3],
            Channel {
                src: Source::BaseColor,
                comp: 2,
                invert: true
            }
        );
        assert!(recipe.maps[1].force_16);
        assert_eq!(recipe.maps[1].layout, Layout::Rgba);

        let root = std::env::temp_dir().join(format!(
            "bevy-export-recipe-edit-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        let output = recipe.write_json(&root).unwrap();
        let next = recipe.write_json(&root).unwrap();
        assert_ne!(output, next);
        assert_eq!(load(&output).unwrap(), vec![recipe]);
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&next).unwrap()).unwrap();
        assert_eq!(raw[0]["maps"][1]["layout"], "RGBA");
        assert_eq!(
            raw[0]["maps"][1]["channels"][3],
            serde_json::json!({"src": "BaseColor", "comp": 2, "invert": true})
        );
        assert_eq!(raw[0]["maps"][1]["force_16"], true);
        std::fs::remove_dir_all(root).unwrap();
    }
}
