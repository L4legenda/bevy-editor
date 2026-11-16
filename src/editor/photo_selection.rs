//! PhotoCraft's native selection/transform commands, on the shared undo timeline.
use super::*;

impl PhotoDocument {
    pub fn preview_layer_command(
        &mut self,
        edit: &mut LayerMove,
        command: &str,
        params: serde_json::Value,
    ) -> Result<(), String> {
        self.cancel_move(edit);
        self.layer_command(command, params)?;
        edit.applied = self.revision != edit.before.revision;
        Ok(())
    }
    pub fn transform_frame(&self) -> Option<[f32; 4]> {
        if self.filter_target || !self.movable() {
            return None;
        }
        let bounds = if self.mask_target {
            let mut bounds = self.paint_surface()?.content_bounds();
            if let Some(selection) = &self.document.selection {
                bounds = bounds.intersect(&selection.content_bounds());
            }
            bounds
        } else {
            let ids: Vec<_> = if self.selected.is_empty() {
                self.active_id().into_iter().collect()
            } else {
                self.selected.clone()
            };
            if ids.iter().any(|&id| {
                photocraft_engine::commands::transform_cmds::locked_for_transform(
                    &self.document,
                    id,
                )
            }) {
                return None;
            }
            let content = photocraft_engine::commands::transform_cmds::transform_bounds_of(
                &self.document,
                &ids,
            );
            // The engine frames raster selections by their outline, including empty pixels.
            if content.is_empty() {
                content
            } else if self.active_layer().is_some_and(|layer| {
                !photocraft_engine::commands::transform_cmds::moves_whole(layer)
            }) {
                self.document
                    .selection
                    .as_ref()
                    .map_or(content, Surface::content_bounds)
            } else {
                content
            }
        };
        (!bounds.is_empty()).then_some([
            bounds.x0 as f32,
            bounds.y0 as f32,
            bounds.x1 as f32,
            bounds.y1 as f32,
        ])
    }

    pub fn preview_transform(
        &mut self,
        edit: &mut LayerMove,
        frame: [f32; 4],
        quad: [[f32; 2]; 4],
        nearest: bool,
    ) -> Result<(), String> {
        if quad
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 32768.0)
        {
            return Err("Недопустимые координаты трансформации".into());
        }
        let min = quad
            .iter()
            .fold([f32::INFINITY; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
        let max = quad.iter().fold([f32::NEG_INFINITY; 2], |a, p| {
            [a[0].max(p[0]), a[1].max(p[1])]
        });
        if (max[0] - min[0]) as f64 * (max[1] - min[1]) as f64 > MAX_PIXELS as f64 {
            return Err("Трансформация превышает 16 Мп".into());
        }
        self.cancel_move(edit);
        let unchanged = [
            [frame[0], frame[1]],
            [frame[2], frame[1]],
            [frame[2], frame[3]],
            [frame[0], frame[3]],
        ];
        if quad
            .iter()
            .flatten()
            .zip(unchanged.iter().flatten())
            .all(|(a, b)| (a - b).abs() < 0.001)
        {
            return Ok(());
        }
        self.layer_command(
            "edit.transform",
            serde_json::json!({
                "rect":frame,"quad":quad,
                "interpolation":if nearest {"nearest"} else {"bicubic"},
                "target":if self.mask_target {"mask"} else {"pixels"},
            }),
        )?;
        edit.applied = self.revision != edit.before.revision;
        Ok(())
    }
}
