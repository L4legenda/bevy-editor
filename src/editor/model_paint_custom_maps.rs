//! Settings and baked scalar planes shared by the baker and paint document.
use bevy::math::Vec3;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};

pub(in crate::editor) const MAX_MAPS: usize = 64;
pub(in crate::editor) const MAX_MIX_INPUTS: usize = 4;
const DOCUMENT_KEY: &str = "BEVY_editor_custom_bakes";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::editor) enum Kind {
    Flow,
    Exposure,
    Deposit,
    Contact,
    Point,
}

impl Kind {
    pub const ALL: [Self; 5] = [
        Self::Flow,
        Self::Exposure,
        Self::Deposit,
        Self::Contact,
        Self::Point,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Flow => "Потёки по поверхности",
            Self::Exposure => "Направленная открытость",
            Self::Deposit => "Ветровые отложения",
            Self::Contact => "Контакт деталей",
            Self::Point => "Расстояние от точки",
        }
    }

    pub fn has_direction(self) -> bool {
        matches!(self, Self::Flow | Self::Exposure | Self::Deposit)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Field {
    Distance,
    Azimuth,
    Elevation,
    Spread,
    Strength,
    Wander,
    X,
    Y,
    Z,
}

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Self::Distance => "Дальность",
            Self::Azimuth => "Азимут, °",
            Self::Elevation => "Высота, °",
            Self::Spread => "Разброс, °",
            Self::Strength => "Сила потёков",
            Self::Wander => "Извилистость, °",
            Self::X => "Точка X",
            Self::Y => "Точка Y",
            Self::Z => "Точка Z",
        }
    }

    pub fn is_angle(self) -> bool {
        matches!(
            self,
            Self::Azimuth | Self::Elevation | Self::Spread | Self::Wander
        )
    }

    fn bounds(self) -> (i16, i16, i16) {
        match self {
            Self::Distance => (1, 800, 5),
            Self::Azimuth => (-180, 180, 5),
            Self::Elevation => (-90, 90, 5),
            Self::Spread => (0, 90, 5),
            Self::Strength => (1, 800, 10),
            Self::Wander => (0, 90, 5),
            Self::X | Self::Y | Self::Z => (-200, 200, 10),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::editor) struct Recipe {
    pub id: u64,
    pub name: String,
    pub kind: Kind,
    pub distance: i16,
    pub azimuth: i16,
    pub elevation: i16,
    pub spread: i16,
    #[serde(default = "flow_strength")]
    pub strength: i16,
    #[serde(default = "flow_wander")]
    pub wander: i16,
    pub point: [i16; 3],
}

const fn flow_strength() -> i16 {
    200
}
const fn flow_wander() -> i16 {
    25
}

impl Recipe {
    pub fn direction(&self) -> Vec3 {
        let (sin_azimuth, cos_azimuth) = f32::from(self.azimuth).to_radians().sin_cos();
        let (sin_elevation, cos_elevation) = f32::from(self.elevation).to_radians().sin_cos();
        Vec3::new(
            cos_elevation * sin_azimuth,
            sin_elevation,
            cos_elevation * cos_azimuth,
        )
    }

    pub fn point_at(&self, center: Vec3, unit: f32) -> Vec3 {
        center + Vec3::from_array(self.point.map(f32::from)) * (unit / 100.)
    }

    pub fn set_direction(&mut self, direction: Vec3) {
        if let Some(direction) = direction.try_normalize() {
            self.elevation = direction.y.clamp(-1., 1.).asin().to_degrees().round() as i16;
            if direction.x * direction.x + direction.z * direction.z > 1e-6 {
                self.azimuth = direction.x.atan2(direction.z).to_degrees().round() as i16;
            }
        }
    }

    pub fn set_point(&mut self, point: Vec3, center: Vec3, unit: f32) {
        self.point = ((point - center) * (100. / unit)).to_array().map(|axis| {
            let (min, max, _) = Field::X.bounds();
            axis.round().clamp(f32::from(min), f32::from(max)) as i16
        });
    }

    pub fn new(kind: Kind, previous: &[Self]) -> Self {
        let base = kind.label();
        let name = if previous.iter().any(|recipe| recipe.name == base) {
            Self::numbered_name(base, previous)
        } else {
            base.to_owned()
        };
        let (distance, elevation, spread) = match kind {
            Kind::Flow => (50, 90, 12),
            Kind::Exposure => (400, 15, 30),
            Kind::Deposit => (40, 5, 25),
            Kind::Contact => (8, 90, 25),
            Kind::Point => (100, 90, 25),
        };
        Self {
            id: Self::next_id(previous),
            name,
            kind,
            distance,
            azimuth: 0,
            elevation,
            spread,
            strength: flow_strength(),
            wander: flow_wander(),
            point: [0; 3],
        }
    }

    pub fn duplicate(&self, previous: &[Self]) -> Self {
        let base = self
            .name
            .trim_end_matches(|ch: char| ch.is_ascii_digit())
            .trim_end();
        Self {
            id: Self::next_id(previous),
            name: Self::numbered_name(if base.is_empty() { &self.name } else { base }, previous),
            ..self.clone()
        }
    }

    fn next_id(previous: &[Self]) -> u64 {
        std::iter::repeat_with(|| bevy::asset::uuid::Uuid::new_v4().as_u128() as u64)
            .find(|&id| id != 0 && previous.iter().all(|recipe| recipe.id != id))
            .unwrap()
    }

    fn numbered_name(base: &str, previous: &[Self]) -> String {
        (2..)
            .map(|number| {
                let suffix = format!(" {number}");
                let mut end = base.len().min(128 - suffix.len());
                while !base.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}{suffix}", base[..end].trim_end())
            })
            .find(|name| previous.iter().all(|recipe| &recipe.name != name))
            .unwrap()
    }

    pub fn same_bake(&self, other: &Self) -> bool {
        self.id == other.id
            && self.kind == other.kind
            && self
                .fields()
                .iter()
                .all(|&field| self.value(field) == other.value(field))
    }

    pub fn fields(&self) -> &'static [Field] {
        use Field::*;
        match self.kind {
            Kind::Flow => &[Azimuth, Elevation, Spread, Distance, Strength, Wander],
            Kind::Exposure | Kind::Deposit => &[Azimuth, Elevation, Spread, Distance],
            Kind::Contact => &[Distance],
            Kind::Point => &[X, Y, Z, Distance],
        }
    }

    pub fn value(&self, field: Field) -> i16 {
        match field {
            Field::Distance => self.distance,
            Field::Azimuth => self.azimuth,
            Field::Elevation => self.elevation,
            Field::Spread => self.spread,
            Field::Strength => self.strength,
            Field::Wander => self.wander,
            Field::X => self.point[0],
            Field::Y => self.point[1],
            Field::Z => self.point[2],
        }
    }

    pub fn adjust(&mut self, field: Field, direction: i8) -> bool {
        let (min, max, step) = field.bounds();
        let value = self.value(field);
        let next = (value + step * i16::from(direction.signum())).clamp(min, max);
        if !self.fields().contains(&field) || value == next {
            return false;
        }
        match field {
            Field::Distance => self.distance = next,
            Field::Azimuth => self.azimuth = next,
            Field::Elevation => self.elevation = next,
            Field::Spread => self.spread = next,
            Field::Strength => self.strength = next,
            Field::Wander => self.wander = next,
            Field::X => self.point[0] = next,
            Field::Y => self.point[1] = next,
            Field::Z => self.point[2] = next,
        }
        true
    }
}

pub(in crate::editor) fn valid_recipes(recipes: &[Recipe]) -> bool {
    let mut ids = BTreeSet::new();
    recipes.len() <= MAX_MAPS
        && recipes.iter().all(|recipe| {
            recipe.id != 0
                && ids.insert(recipe.id)
                && !recipe.name.is_empty()
                && recipe.name.len() <= 128
                && [
                    Field::Distance,
                    Field::Azimuth,
                    Field::Elevation,
                    Field::Spread,
                    Field::Strength,
                    Field::Wander,
                    Field::X,
                    Field::Y,
                    Field::Z,
                ]
                .into_iter()
                .all(|field| {
                    let (min, max, _) = field.bounds();
                    (min..=max).contains(&recipe.value(field))
                })
        })
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::editor) struct Plane {
    pub recipe: Recipe,
    pub values: Arc<Vec<u8>>,
}

/// A viewport-sized input independent of the persistent bake resolution.
#[derive(Debug)]
pub(in crate::editor) struct LivePlane {
    pub id: u64,
    pub size: u32,
    pub values: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::editor) struct Input {
    pub id: u64,
    pub weight: i16,
}

pub(in crate::editor) fn read_document(root: &serde_json::Value) -> Vec<Recipe> {
    root["extensions"]
        .get(DOCUMENT_KEY)
        .and_then(|data| serde_json::from_value::<Vec<Recipe>>(data["maps"].clone()).ok())
        .filter(|maps| valid_recipes(maps))
        .unwrap_or_default()
}

pub(in crate::editor) fn write_document(
    root: &mut serde_json::Value,
    maps: &[Recipe],
) -> Result<(), String> {
    use serde_json::json;
    if maps.is_empty() {
        if let Some(extensions) = root
            .get_mut("extensions")
            .and_then(serde_json::Value::as_object_mut)
        {
            extensions.remove(DOCUMENT_KEY);
        }
        if let Some(used) = root
            .get_mut("extensionsUsed")
            .and_then(serde_json::Value::as_array_mut)
        {
            used.retain(|name| name.as_str() != Some(DOCUMENT_KEY));
        }
    } else {
        if root["extensionsUsed"].is_null() {
            root["extensionsUsed"] = json!([]);
        }
        let used = root["extensionsUsed"]
            .as_array_mut()
            .ok_or("Некорректные расширения GLB")?;
        if !used.iter().any(|name| name.as_str() == Some(DOCUMENT_KEY)) {
            used.push(json!(DOCUMENT_KEY));
        }
        if root["extensions"].is_null() {
            root["extensions"] = json!({});
        }
        root["extensions"]
            .as_object_mut()
            .ok_or("Некорректные расширения GLB")?
            .insert(DOCUMENT_KEY.into(), json!({"maps": maps}));
    }
    Ok(())
}
