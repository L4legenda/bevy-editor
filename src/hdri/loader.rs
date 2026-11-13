//! Decode and prefilter downloaded panoramas on Bevy's asset worker threads.
use bevy::{
    asset::{AssetLoader, LoadContext, RenderAssetUsages, io::Reader},
    image::ImageSampler,
    prelude::*,
    render::render_resource::{
        Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
    },
};
use std::{
    f32::consts::{PI, TAU},
    io::{self, Cursor},
};

#[derive(Asset, TypePath)]
pub(super) struct PanoramaMaps {
    pub diffuse: Handle<Image>,
    pub specular: Handle<Image>,
    pub skybox: Handle<Image>,
}

#[derive(Default, TypePath)]
pub(super) struct PanoramaLoader;

impl AssetLoader for PanoramaLoader {
    type Asset = PanoramaMaps;
    type Settings = ();
    type Error = io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        context: &mut LoadContext<'_>,
    ) -> io::Result<PanoramaMaps> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let extension = context
            .path()
            .path()
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("hdr");
        let source = decode(&bytes, extension)?;
        let sky_size = (source.width / 4).clamp(16, 1024).next_power_of_two();
        let (diffuse, specular, skybox) = prepare(source, sky_size, 256, 64);
        Ok(PanoramaMaps {
            diffuse: context.add_labeled_asset("diffuse", diffuse),
            specular: context.add_labeled_asset("specular", specular),
            skybox: context.add_labeled_asset("skybox", skybox),
        })
    }

    fn extensions(&self) -> &[&str] {
        &["hdr", "exr"]
    }
}

#[cfg(feature = "editor")]
pub(crate) fn validate_panorama(bytes: &[u8], extension: &str) -> io::Result<()> {
    decode(bytes, extension).map(|_| ())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn decode(bytes: &[u8], extension: &str) -> io::Result<Panorama> {
    let format = match extension {
        "hdr" => image::ImageFormat::Hdr,
        "exr" => image::ImageFormat::OpenExr,
        _ => return Err(invalid("Нужна панорама HDR или EXR")),
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(2048);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| invalid(error.to_string()))?
        .into_rgb32f();
    let (width, height) = image.dimensions();
    if width != 2 * height || height < 8 {
        return Err(invalid(
            "Панорама должна иметь пропорции 2:1 и разрешение до 4K",
        ));
    }
    let mut pixels = image.into_raw();
    for value in &mut pixels {
        *value = if value.is_finite() {
            value.clamp(0.0, 60_000.0)
        } else {
            0.0
        };
    }
    Ok(Panorama {
        width,
        height,
        pixels,
    })
}

struct Panorama {
    width: u32,
    height: u32,
    pixels: Vec<f32>,
}

impl Panorama {
    fn pixel(&self, x: u32, y: u32) -> Vec3 {
        let index = ((y * self.width + x) * 3) as usize;
        Vec3::from_slice(&self.pixels[index..index + 3])
    }

    fn sample(&self, uv: Vec2) -> Vec3 {
        let x = uv.x * self.width as f32 - 0.5;
        let y = (uv.y * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let x0 = x.floor() as i32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).rem_euclid(self.width as i32) as u32;
        let x0_wrapped = x0.rem_euclid(self.width as i32) as u32;
        let y1 = (y0 + 1).min(self.height - 1);
        self.pixel(x0_wrapped, y0)
            .lerp(self.pixel(x1, y0), x - x0 as f32)
            .lerp(
                self.pixel(x0_wrapped, y1)
                    .lerp(self.pixel(x1, y1), x - x0 as f32),
                y - y0 as f32,
            )
    }

    fn half_size(&self) -> Self {
        let (width, height) = (self.width / 2, self.height / 2);
        let mut pixels = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(
                    &((self.pixel(x * 2, y * 2)
                        + self.pixel(x * 2 + 1, y * 2)
                        + self.pixel(x * 2, y * 2 + 1)
                        + self.pixel(x * 2 + 1, y * 2 + 1))
                        * 0.25)
                        .to_array(),
                );
            }
        }
        Self {
            width,
            height,
            pixels,
        }
    }
}

fn uv(direction: Vec3) -> Vec2 {
    Vec2::new(
        direction.z.atan2(direction.x) / TAU + 0.5,
        direction.y.clamp(-1.0, 1.0).acos() / PI,
    )
}

fn cube_direction(face: u32, x: u32, y: u32, size: u32) -> Vec3 {
    let u = (x as f32 + 0.5) * 2.0 / size as f32 - 1.0;
    let v = (y as f32 + 0.5) * 2.0 / size as f32 - 1.0;
    match face {
        0 => Vec3::new(1.0, -v, -u),
        1 => Vec3::new(-1.0, -v, u),
        2 => Vec3::new(u, 1.0, v),
        3 => Vec3::new(u, -1.0, -v),
        4 => Vec3::new(u, -v, 1.0),
        _ => Vec3::new(-u, -v, -1.0),
    }
    .normalize()
}

fn pack(color: Vec3, bytes: &mut Vec<u8>) {
    for value in [color.x, color.y, color.z, 1.0] {
        bytes.extend_from_slice(
            &half::f16::from_f32(value.clamp(0.0, 60_000.0))
                .to_bits()
                .to_le_bytes(),
        );
    }
}

fn cube(size: u32, mips: Vec<Vec<u8>>) -> Image {
    let levels = mips.len() as u32;
    // Bevy's default image upload order is layer-major, then mip-major.
    let mut data = Vec::new();
    for face in 0..6 {
        for (level, bytes) in mips.iter().enumerate() {
            let edge = (size >> level).max(1);
            let length = (edge * edge * 8) as usize;
            data.extend_from_slice(&bytes[face * length..(face + 1) * length]);
        }
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::all(),
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image.sampler = ImageSampler::linear();
    image
}

fn sh_basis(d: Vec3) -> [f32; 9] {
    [
        0.2820948,
        0.48860252 * d.x,
        0.48860252 * d.y,
        0.48860252 * d.z,
        1.0925485 * d.x * d.y,
        1.0925485 * d.y * d.z,
        0.31539157 * (3.0 * d.z * d.z - 1.0),
        1.0925485 * d.x * d.z,
        0.54627424 * (d.x * d.x - d.y * d.y),
    ]
}

fn irradiance(source: &Panorama) -> [Vec3; 9] {
    let mut coefficients = [Vec3::ZERO; 9];
    for y in 0..source.height {
        let theta = (y as f32 + 0.5) * PI / source.height as f32;
        let weight = theta.sin() * TAU / source.width as f32 * PI / source.height as f32;
        for x in 0..source.width {
            let phi = ((x as f32 + 0.5) / source.width as f32 - 0.5) * TAU;
            let direction = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            let color = source.pixel(x, y) * weight;
            for (coefficient, basis) in coefficients.iter_mut().zip(sh_basis(direction)) {
                *coefficient += color * basis;
            }
        }
    }
    let bands = [
        1.0,
        2.0 / 3.0,
        2.0 / 3.0,
        2.0 / 3.0,
        0.25,
        0.25,
        0.25,
        0.25,
        0.25,
    ];
    std::array::from_fn(|i| coefficients[i] * bands[i])
}

fn face_bytes(size: u32, color: impl Fn(u32, Vec3) -> Vec3 + Sync) -> Vec<u8> {
    std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..6)
            .map(|face| {
                let color = &color;
                scope.spawn(move || {
                    let mut bytes = Vec::with_capacity((size * size * 8) as usize);
                    for y in 0..size {
                        for x in 0..size {
                            pack(color(face, cube_direction(face, x, y, size)), &mut bytes);
                        }
                    }
                    bytes
                })
            })
            .collect();
        jobs.into_iter()
            .flat_map(|job| job.join().expect("HDRI filter worker panicked"))
            .collect()
    })
}

fn prepare(source: Panorama, sky_size: u32, spec_size: u32, samples: u32) -> (Image, Image, Image) {
    let mut mips = vec![source];
    while mips.last().unwrap().height >= 2 {
        mips.push(mips.last().unwrap().half_size());
    }
    let projection = mips.iter().find(|image| image.width <= 512).unwrap();
    let coefficients = irradiance(projection);
    let diffuse = face_bytes(32, |_, direction| {
        coefficients
            .iter()
            .zip(sh_basis(direction))
            .fold(Vec3::ZERO, |sum, (c, b)| sum + *c * b)
            .max(Vec3::ZERO)
    });
    let skybox = face_bytes(sky_size, |_, direction| mips[0].sample(uv(direction)));
    let mut specular = vec![face_bytes(spec_size, |_, direction| {
        mips[0].sample(uv(direction))
    })];
    let levels = spec_size.ilog2() + 1;
    for level in 1..levels {
        let roughness = level as f32 / (levels - 1) as f32;
        let alpha2 = roughness.powi(4);
        let points: Vec<_> = (0..samples)
            .map(|index| {
                let xi = (index as f32 + 0.5) / samples as f32;
                let azimuth = index.reverse_bits() as f32 * (1.0 / 4294967296.0) * TAU;
                let cos_h = ((1.0 - xi) / (1.0 + (alpha2 - 1.0) * xi)).sqrt();
                let sin_h = (1.0 - cos_h * cos_h).max(0.0).sqrt();
                let nl = (2.0 * cos_h * cos_h - 1.0).max(0.0);
                let pdf = alpha2 / (4.0 * PI * (cos_h * cos_h * (alpha2 - 1.0) + 1.0).powi(2));
                (
                    Vec3::new(sin_h * azimuth.cos(), sin_h * azimuth.sin(), cos_h),
                    nl,
                    pdf,
                )
            })
            .filter(|(_, weight, _)| *weight > 0.0)
            .collect();
        let weight_sum: f32 = points.iter().map(|(_, weight, _)| *weight).sum();
        specular.push(face_bytes(spec_size >> level, |_, normal| {
            let up = if normal.y.abs() > 0.99 {
                Vec3::X
            } else {
                Vec3::Y
            };
            let tangent = up.cross(normal).normalize();
            let bitangent = normal.cross(tangent);
            let mut sum = Vec3::ZERO;
            for (h, weight, pdf) in &points {
                let half = tangent * h.x + bitangent * h.y + normal * h.z;
                let light = half * (2.0 * h.z) - normal;
                let area = 2.0 * PI * PI / (mips[0].width * mips[0].height) as f32
                    * (1.0 - light.y * light.y).max(0.0).sqrt().max(1e-4);
                let lod = (0.5 * (1.0 / (samples as f32 * pdf * area)).log2())
                    .clamp(0.0, (mips.len() - 1) as f32);
                let first = lod.floor() as usize;
                let coordinate = uv(light);
                let color = mips[first].sample(coordinate).lerp(
                    mips[(first + 1).min(mips.len() - 1)].sample(coordinate),
                    lod.fract(),
                );
                sum += color * *weight;
            }
            sum / weight_sum
        }));
    }
    (
        cube(32, vec![diffuse]),
        cube(spec_size, specular),
        cube(sky_size, vec![skybox]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constant_panorama_keeps_radiance_and_cubemap_mip_layout() {
        let source = Panorama {
            width: 64,
            height: 32,
            pixels: vec![1.0; 64 * 32 * 3],
        };
        let (diffuse, specular, skybox) = prepare(source, 16, 16, 32);
        assert_eq!(specular.texture_descriptor.mip_level_count, 5);
        assert_eq!(skybox.texture_descriptor.size.depth_or_array_layers, 6);
        for image in [diffuse, specular, skybox] {
            for pixel in image.data.unwrap().chunks_exact(8) {
                for channel in pixel[..6].chunks_exact(2) {
                    let value =
                        half::f16::from_bits(u16::from_le_bytes([channel[0], channel[1]])).to_f32();
                    assert!(
                        (value - 1.0).abs() < 0.01,
                        "Constant radiance changed to {value}"
                    );
                }
            }
        }
    }
}
