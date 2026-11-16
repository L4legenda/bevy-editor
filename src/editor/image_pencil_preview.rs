//! The pencil cursor uses the native dab coverage, cached once per size/shape.
use super::*;
use photocraft_paint::{BrushContext, BrushSettings, Stroke, StrokePoint};

struct Mask {
    rect: photocraft_geom::Rect,
    center: Vec2,
    pixels: Vec<u8>,
}
impl Mask {
    fn new(size: u16, square: bool) -> Option<Self> {
        let brush = BrushSettings {
            size: f32::from(size),
            aliased: true,
            square,
            hardness: 1.0,
            ..Default::default()
        };
        let dab = photocraft_paint::dabs(&Stroke {
            brush: brush.clone(),
            points: vec![StrokePoint::new(0.0, 0.0, 1.0)],
        })
        .into_iter()
        .next()?;
        let footprint = BrushContext::new(&brush).footprint(&dab, 0);
        let (x, y) = photocraft_paint::grid_center(0.0, 0.0, f32::from(size));
        Some(Self {
            rect: footprint.rect,
            center: Vec2::new(x, y),
            pixels: footprint
                .cov
                .into_iter()
                .map(|c| if c > 0.5 { 255 } else { 0 })
                .collect(),
        })
    }
}

#[derive(Default)]
pub(in crate::editor) struct Cache {
    key: Option<(u16, bool)>,
    image: Option<Handle<Image>>,
    mask: Option<Mask>,
}
impl Cache {
    fn prepare(&mut self, size: u16, square: bool, images: &mut Assets<Image>) -> bool {
        if self.key == Some((size, square)) {
            return true;
        }
        let Some(mut mask) = Mask::new(size, square) else {
            return false;
        };
        let mut texture = Image::new(
            Extent3d {
                width: mask.rect.width(),
                height: mask.rect.height(),
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            std::mem::take(&mut mask.pixels),
            TextureFormat::R8Unorm,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        texture.sampler = bevy::image::ImageSampler::nearest();
        if let Some(mut old) = self.image.as_ref().and_then(|h| images.get_mut(h)) {
            *old = texture;
        } else {
            self.image = Some(images.add(texture));
        }
        self.mask = Some(mask);
        self.key = Some((size, square));
        true
    }
}

pub(in crate::editor) fn refresh(
    viewer: Res<ImageViewer>,
    editor: Res<Editor>,
    window: Single<&Window, With<PrimaryWindow>>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<CanvasMaterial>>,
    mut cache: Local<Cache>,
) {
    let Some(mut material) = materials.get_mut(&super::super::canvas_material::HANDLE) else {
        return;
    };
    material.pencil_rect = Vec4::ZERO;
    if viewer.edit.tool != PhotoTool::Pencil
        || !viewer.edit.editing(editor.viewing_image)
        || !viewer.cursor_owned
        || viewer.brush_cursor.is_none()
    {
        return;
    }
    let Some(position) = viewer.brush_cursor_position else {
        return;
    };
    let Some(dimensions) = viewer.dimensions else {
        return;
    };
    let rect = canvas_rect(&canvas, &window);
    let image = viewer.image_rect(dimensions, rect);
    let point = viewer
        .edit
        .snap_point((position - image.min) / image.size() * dimensions.as_vec2());
    let size = viewer.edit.size.round().clamp(1.0, 256.0) as u16;
    let square = viewer.edit.pencil_shape == super::super::edit::PencilShape::Square;
    if !cache.prepare(size, square, &mut images) {
        return;
    }
    let mask = cache.mask.as_ref().unwrap();
    let origin = point - mask.center + Vec2::new(mask.rect.x0 as f32, mask.rect.y0 as f32);
    material.pencil_mask = cache.image.clone();
    material.pencil_rect = Vec4::new(
        origin.x,
        origin.y,
        mask.rect.width() as f32,
        mask.rect.height() as f32,
    );
    let [r, g, b, a] = viewer.edit.color();
    material.pencil_color = Vec4::from_array(
        Color::srgba(r, g, b, (a * viewer.edit.opacity).clamp(0.35, 0.85))
            .to_linear()
            .to_f32_array(),
    );
}

#[cfg(test)]
#[path = "image_pencil_preview_tests.rs"]
mod tests;
