//! Sample imported material textures once in the UV0 layout used by the paint engine.
use bevy::{
    image::{ImageAddressMode, ImageSampler},
    math::Affine2,
    mesh::UvChannel,
    prelude::*,
    render::render_resource::TextureFormat,
};
use serde_json::Value;

use super::projection::weights;
use crate::editor::model_mesh::Geometry;

pub(super) struct Layout<'a> {
    mesh: &'a Geometry,
    slot: usize,
    size: u32,
    uv_offset: Vec2,
    uv1: Option<Vec<Option<Vec2>>>,
}

impl<'a> Layout<'a> {
    pub fn new(mesh: &'a Geometry, slot: usize, size: u32, uv_offset: Vec2) -> Self {
        Self {
            mesh,
            slot,
            size,
            uv_offset,
            uv1: None,
        }
    }

    pub fn pixels(
        &mut self,
        image: Option<Image>,
        texture_info: &Value,
        fallback_channel: UvChannel,
        fallback_transform: Affine2,
        default: [u8; 4],
    ) -> Vec<u8> {
        let pixel_count = self.size.pow(2) as usize;
        let Some(image) = image else {
            return default.repeat(pixel_count);
        };
        let modes = match &image.sampler {
            ImageSampler::Descriptor(sampler) => [sampler.address_mode_u, sampler.address_mode_v],
            ImageSampler::Default => [ImageAddressMode::ClampToEdge; 2],
        };
        let pixels = if image.texture_descriptor.format == TextureFormat::Rgba8Unorm {
            // Bevy's dynamic conversion skips linear RGBA8. Preserve raw
            // channel values, including the alpha used by glTF Specular.
            let (width, height) = (image.width(), image.height());
            image
                .data
                .and_then(|bytes| image::RgbaImage::from_raw(width, height, bytes))
        } else {
            image
                .try_into_dynamic()
                .ok()
                .map(image::DynamicImage::into_rgba8)
        };
        let Some(image) = pixels else {
            return default.repeat(pixel_count);
        };
        let extension = &texture_info["extensions"]["KHR_texture_transform"];
        let uv1 = extension["texCoord"]
            .as_u64()
            .or_else(|| texture_info["texCoord"].as_u64())
            .map_or(fallback_channel == UvChannel::Uv1, |channel| channel == 1);
        let vector = |key: &str, default: Vec2| {
            let value = &extension[key];
            match (value[0].as_f64(), value[1].as_f64()) {
                (Some(x), Some(y)) => Vec2::new(x as f32, y as f32),
                _ => default,
            }
        };
        let transform = if extension.is_object() {
            Affine2::from_scale_angle_translation(
                vector("scale", Vec2::ONE),
                extension["rotation"].as_f64().unwrap_or(0.) as f32,
                vector("offset", Vec2::ZERO),
            )
        } else if texture_info.is_object() {
            Affine2::IDENTITY
        } else {
            fallback_transform
        };
        if !uv1 && transform == Affine2::IDENTITY {
            return image::imageops::resize(
                &image,
                self.size,
                self.size,
                image::imageops::FilterType::Triangle,
            )
            .into_raw();
        }

        let coordinates = uv1.then(|| {
            self.uv1
                .get_or_insert_with(|| raster_uv1(self.mesh, self.slot, self.size, self.uv_offset))
        });
        let mut out = default.repeat(pixel_count);
        for (index, pixel) in out.chunks_exact_mut(4).enumerate() {
            let x = index as u32 % self.size;
            let y = index as u32 / self.size;
            let uv = match &coordinates {
                Some(raster) => {
                    let Some(uv) = raster[index] else { continue };
                    uv
                }
                None => (Vec2::new(x as f32, y as f32) + Vec2::splat(0.5)) / self.size as f32,
            };
            pixel.copy_from_slice(&sample(
                &image,
                transform.transform_point2(uv),
                modes,
                default,
            ));
        }
        out
    }
}

fn raster_uv1(mesh: &Geometry, slot: usize, size: u32, uv_offset: Vec2) -> Vec<Option<Vec2>> {
    let mut result = vec![None; size.pow(2) as usize];
    let edge = Vec2::splat(size as f32);
    for face in mesh.faces.iter().filter(|face| face.material == slot) {
        for triangle in mesh.triangles(face) {
            let corners = triangle.map(|i| &face.corners[i]);
            let uv = corners.map(|corner| (corner.uv - uv_offset) * edge);
            let min = uv[0]
                .min(uv[1])
                .min(uv[2])
                .floor()
                .clamp(Vec2::ZERO, edge)
                .as_uvec2();
            let max = uv[0]
                .max(uv[1])
                .max(uv[2])
                .ceil()
                .clamp(Vec2::ZERO, edge)
                .as_uvec2();
            for y in min.y..max.y {
                for x in min.x..max.x {
                    let index = (y * size + x) as usize;
                    if result[index].is_some() {
                        continue;
                    }
                    let position = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    if let Some(w) = weights(position, uv)
                        && w.min_element() >= -1e-5
                    {
                        result[index] = Some(
                            corners[0].uv1 * w.x + corners[1].uv1 * w.y + corners[2].uv1 * w.z,
                        );
                    }
                }
            }
        }
    }
    result
}

fn sample(
    image: &image::RgbaImage,
    uv: Vec2,
    modes: [ImageAddressMode; 2],
    border: [u8; 4],
) -> [u8; 4] {
    fn address(index: i32, length: i32, mode: ImageAddressMode) -> Option<u32> {
        let index = match mode {
            ImageAddressMode::ClampToEdge => index.clamp(0, length - 1),
            ImageAddressMode::Repeat => index.rem_euclid(length),
            ImageAddressMode::MirrorRepeat => {
                let at = index.rem_euclid(2 * length);
                at.min(2 * length - 1 - at)
            }
            ImageAddressMode::ClampToBorder if !(0..length).contains(&index) => return None,
            ImageAddressMode::ClampToBorder => index,
        };
        Some(index as u32)
    }
    let dimensions = Vec2::new(image.width() as f32, image.height() as f32);
    let point = uv * dimensions - Vec2::splat(0.5);
    let base = point.floor();
    let weight = point - base;
    let fetch = |dx, dy| {
        let x = address(
            (base.x as i32).saturating_add(dx),
            image.width() as i32,
            modes[0],
        );
        let y = address(
            (base.y as i32).saturating_add(dy),
            image.height() as i32,
            modes[1],
        );
        x.zip(y).map_or(border, |(x, y)| image.get_pixel(x, y).0)
    };
    let a = fetch(0, 0);
    let b = fetch(1, 0);
    let c = fetch(0, 1);
    let d = fetch(1, 1);
    std::array::from_fn(|i| {
        let mix = |a: u8, b: u8| f32::from(a) * (1. - weight.x) + f32::from(b) * weight.x;
        (mix(a[i], b[i]) * (1. - weight.y) + mix(c[i], d[i]) * weight.y).round() as u8
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_mesh::{Corner, Face};
    use bevy::asset::RenderAssetUsages;
    use serde_json::json;

    #[test]
    fn imported_paint_map_uses_uv1_with_texture_transform_override() {
        let mut geometry = Geometry::default();
        let uv = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y];
        geometry.positions.extend(uv.map(|p| p.extend(0.)));
        geometry.faces.push(Face {
            material: 1,
            smooth: false,
            corners: uv
                .into_iter()
                .enumerate()
                .map(|(index, p)| {
                    let mut corner = Corner::new(index);
                    corner.uv = p;
                    corner.uv1 = Vec2::new(1. - p.x, p.y);
                    corner
                })
                .collect(),
        });
        let image = Image::from_dynamic(
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(4, 4, |x, y| {
                image::Rgba([0, 0, 0, ((x + y * 4) * 16) as u8])
            })),
            false,
            RenderAssetUsages::MAIN_WORLD,
        );
        let transform = json!({
            "texCoord": 0,
            "extensions": {"KHR_texture_transform": {
                "texCoord": 1,
                "rotation": std::f32::consts::FRAC_PI_2,
                "offset": [1.0, 0.0]
            }}
        });
        let mut layout = Layout::new(&geometry, 1, 4, Vec2::ZERO);
        let map = layout.pixels(
            Some(image.clone()),
            &transform,
            UvChannel::Uv0,
            Affine2::IDENTITY,
            [0; 4],
        );
        assert_eq!(map[3], 240);
        assert_eq!(map[7], 176);
        assert_eq!(map[16 + 3], 224);
        assert_eq!(map[63], 0);
        let plain = layout.pixels(
            Some(image),
            &json!({"texCoord": 1}),
            UvChannel::Uv0,
            Affine2::IDENTITY,
            [0; 4],
        );
        assert_eq!(plain[3], 48);
        assert_eq!(plain[63], 192);
    }
}
