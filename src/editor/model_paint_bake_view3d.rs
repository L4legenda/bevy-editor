//! Display-only material for the active texture set.
use super::*;

#[derive(Default)]
pub(in crate::editor::ui::model_viewer) struct Overlay {
    material: Option<Handle<StandardMaterial>>,
}

impl Overlay {
    pub fn update(
        &mut self,
        model: usize,
        doc: &Document,
        paint: &PaintState,
        baked: &State,
        materials: &mut Assets<StandardMaterial>,
    ) -> Option<(sets::SetKey, Handle<StandardMaterial>)> {
        let session = paint.sessions.get(&model)?;
        if session.bake_live_masks && session.view != View::Two {
            return Some((session.binding?, baked.masks.material(model, doc, session)?));
        }
        let map = session.diagnostic_bake()?;
        let key = session.binding?;
        if !session.set_appearance.visibility.shown(key.0, key.1) {
            return None;
        }
        let image = baked.preview_for(model, doc, session, map)?;
        let handle = self.material.get_or_insert_with(|| {
            materials.add(StandardMaterial {
                unlit: true,
                double_sided: true,
                cull_mode: None,
                base_color_texture: Some(image.clone()),
                ..default()
            })
        });
        if materials
            .get(&*handle)
            .is_some_and(|material| material.base_color_texture.as_ref() != Some(image))
        {
            if let Some(mut material) = materials.get_mut(&*handle) {
                material.base_color_texture = Some(image.clone());
            }
        }
        Some((key, handle.clone()))
    }

    pub fn clear(&mut self) {
        self.material = None;
    }
}
