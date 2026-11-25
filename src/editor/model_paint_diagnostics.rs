//! Throttled diagnostics for the paint workspace status bar.
use super::{Session, engine};
use crate::editor::model_document::Document;
use bevy::prelude::*;

#[derive(Default)]
pub(super) struct Meter {
    target: Option<(usize, usize, u32)>,
    elapsed: f64,
    frames: u32,
    geometry: Option<(usize, u64, usize)>,
}

impl Meter {
    pub fn sample(&mut self, target: (usize, usize, u32), seconds: f64) -> Option<f64> {
        self.elapsed += seconds;
        self.frames += 1;
        if self.target == Some(target) && self.elapsed < 0.5 {
            return None;
        }
        let ms = self.elapsed * 1000. / f64::from(self.frames);
        self.target = Some(target);
        self.elapsed = 0.;
        self.frames = 0;
        Some(ms)
    }

    pub fn triangles(&mut self, model: usize, doc: &Document) -> usize {
        if let Some((index, revision, count)) = self.geometry
            && (index, revision) == (model, doc.revision)
        {
            return count;
        }
        let count = doc
            .objects
            .iter()
            .flat_map(|object| &object.geometry.faces)
            .map(|face| face.corners.len().saturating_sub(2))
            .sum();
        self.geometry = Some((model, doc.revision, count));
        count
    }
}

pub(super) fn label(
    model: usize,
    session: &Session,
    runtime: &engine::Runtime,
    queue: &engine::gpu::Queue,
    images: &Assets<Image>,
    ms: f64,
    triangles: usize,
) -> String {
    let history = session.history_cache.bytes();
    let steps = session.undo.len()
        + session.redo.len()
        + session
            .histories
            .values()
            .map(|(undo, redo)| undo.len() + redo.len())
            .sum::<usize>();
    let gpu = runtime.paint_gpu_bytes(model, queue, images);
    const MIB: f64 = 1024. * 1024.;
    format!(
        "{ms:.1} мс · GPU рисования {:.1} МиБ · История {:.1} МиБ ({steps}) · {}² · {triangles} треуг.",
        gpu as f64 / MIB,
        history as f64 / MIB,
        session.texture_size,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_frame_average_and_refreshes_on_texture_set_change() {
        let mut meter = Meter::default();
        assert_eq!(meter.sample((4, 1, 512), 0.016), Some(16.));
        assert!(meter.sample((4, 1, 512), 0.2).is_none());
        assert_eq!(meter.sample((4, 1, 512), 0.3), Some(250.));
        assert_eq!(meter.sample((4, 2, 512), 0.020), Some(20.));
        assert_eq!(meter.sample((4, 2, 1024), 0.024), Some(24.));
    }
}
