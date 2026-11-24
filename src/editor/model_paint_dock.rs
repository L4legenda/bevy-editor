//! Dock geometry and migration of the former independent viewport layout.
use super::*;
use bevy_scene_format::read_limited;
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

pub(super) const DIVIDER: f32 = 6.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) enum Side {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub(super) fn horizontal(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    pub(super) fn nearest(rect: Rect, point: Vec2) -> Option<Self> {
        if !rect.contains(point) {
            return None;
        }
        let point = (point - rect.center()) / rect.size();
        Some(if point.x.abs() >= point.y.abs() {
            if point.x < 0. {
                Self::Left
            } else {
                Self::Right
            }
        } else if point.y < 0. {
            Self::Top
        } else {
            Self::Bottom
        })
    }

    pub(super) fn size_from_edge(self, rect: Rect, point: Vec2) -> f32 {
        let (distance, length) = match self {
            Self::Left => (point.x - rect.min.x, rect.width()),
            Self::Right => (rect.max.x - point.x, rect.width()),
            Self::Top => (point.y - rect.min.y, rect.height()),
            Self::Bottom => (rect.max.y - point.y, rect.height()),
        };
        ((distance - DIVIDER * 0.5) / (length - DIVIDER).max(1.)).clamp(0.2, 0.8)
    }

    pub(super) fn region(self, rect: Rect, fraction: f32) -> Rect {
        let mut zone = rect;
        match self {
            Self::Left => zone.max.x = rect.min.x + rect.width() * fraction,
            Self::Right => zone.min.x = rect.max.x - rect.width() * fraction,
            Self::Top => zone.max.y = rect.min.y + rect.height() * fraction,
            Self::Bottom => zone.min.y = rect.max.y - rect.height() * fraction,
        }
        zone
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct LegacyLayout {
    pub model_side: Side,
    pub model_fraction: f32,
}

impl Default for LegacyLayout {
    fn default() -> Self {
        Self {
            model_side: Side::Left,
            model_fraction: 0.51,
        }
    }
}

impl LegacyLayout {
    pub(super) fn load(path: &Path) -> io::Result<Self> {
        let mut value: Self = serde_json::from_slice(&read_limited(path, 4096)?)?;
        value.model_fraction = value.model_fraction.clamp(0.2, 0.8);
        Ok(value)
    }
}
