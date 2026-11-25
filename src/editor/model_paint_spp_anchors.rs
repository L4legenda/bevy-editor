//! Resolve SPP anchor references to the imported stack state at the recorded action.
use super::{CHANNELS, LayerInfo, SourceInfo};
use std::{collections::HashMap, sync::Arc};
use subcraft_spp::document::{ActionKind, LevelsInfo};

type Plane = Arc<Vec<u8>>;

pub(super) enum Anchor {
    Mask(Plane),
    Content([Option<Plane>; CHANNELS.len()]),
}

#[derive(Default)]
pub(super) struct Anchors(HashMap<i64, Option<Anchor>>);

impl Anchors {
    pub fn into_planes(self) -> impl Iterator<Item = Plane> {
        self.0.into_values().flatten().flat_map(|anchor| match anchor {
            Anchor::Mask(plane) => {
                let mut planes = [const { None }; CHANNELS.len()];
                planes[0] = Some(plane);
                planes
            }
            Anchor::Content(planes) => planes,
        }).flatten()
    }

    pub fn requested(layers: &[LayerInfo]) -> Self {
        let mut result = Self::default();
        result.collect(layers);
        result
    }

    fn collect(&mut self, layers: &[LayerInfo]) {
        for layer in layers {
            let actions = layer
                .content
                .iter()
                .chain(&layer.mask)
                .filter(|action| action.enabled);
            let sources = actions
                .filter_map(|action| match &action.kind {
                    ActionKind::Fill { sources, .. } => Some(sources),
                    ActionKind::Paint {
                        brush: Some(brush), ..
                    } => Some(&brush.sources),
                    _ => None,
                })
                .flatten();
            for source in sources.chain(&layer.sources) {
                if let SourceInfo::Reference { anchor, .. } = source {
                    self.0.entry(*anchor).or_default();
                }
            }
            self.collect(&layer.children);
        }
    }

    pub fn pending(&self, uid: i64) -> bool {
        self.0.get(&uid).is_some_and(Option::is_none)
    }

    pub fn store(&mut self, uid: i64, anchor: Anchor) {
        *self.0.get_mut(&uid).unwrap() = Some(anchor);
    }

    pub fn channel(&self, uid: i64, channel: usize) -> Option<(Plane, bool)> {
        match self.0.get(&uid)?.as_ref()? {
            Anchor::Mask(plane) => Some((plane.clone(), true)),
            Anchor::Content(planes) => planes[channel].clone().map(|plane| (plane, false)),
        }
    }

    pub fn mask_source(&self, uid: i64, channels: u64) -> Option<(Plane, bool)> {
        match self.0.get(&uid)?.as_ref()? {
            Anchor::Mask(plane) => Some((plane.clone(), true)),
            Anchor::Content(planes) => {
                let mut matching = CHANNELS
                    .iter()
                    .zip(planes)
                    .filter_map(|((bit, ..), value)| {
                        (channels & bit != 0).then_some(value.as_ref()).flatten()
                    });
                if let Some(plane) = matching.next() {
                    return matching.next().is_none().then(|| (plane.clone(), false));
                }
                // Mask references may store a zero channel mask. A single material channel
                // has an unambiguous source in that case.
                if channels == 0 {
                    let mut planes = planes.iter().flatten();
                    let plane = planes.next()?;
                    return planes.next().is_none().then(|| (plane.clone(), false));
                }
                None
            }
        }
    }
}

pub(super) fn level(value: f32, levels: Option<&LevelsInfo>, channel: usize) -> f32 {
    let Some(levels) = levels else { return value };
    let [min, max, gamma, low, high] = [
        levels.in_min[channel],
        levels.in_max[channel],
        levels.gamma[channel],
        levels.out_min[channel],
        levels.out_max[channel],
    ];
    if [min, max, gamma, low, high].iter().any(|v| !v.is_finite()) {
        return value;
    }
    let v = ((value - min) / (max - min).max(0.0001)).clamp(0., 1.);
    (low + (high - low) * v.powf(1. / gamma.max(0.001))).clamp(0., 1.)
}
