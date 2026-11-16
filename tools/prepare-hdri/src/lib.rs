//! Offline preparation of the ten bundled CC0 environments, entirely in Rust.
//! Full-resolution SH irradiance/pi and GGX sampling preserve the previous baker;
//! the editor's faster downloaded-panorama loader is a separate runtime path.
pub mod ktx;
use std::{
    f64::consts::{PI, TAU},
    io,
    path::Path,
};

pub const SOURCES: [&str; 10] = [
    "studio_small_09",
    "brown_photostudio_02",
    "christmas_photo_studio_07",
    "empty_warehouse_01",
    "lebombo",
    "kloofendal_48d_partly_cloudy_puresky",
    "venice_sunset",
    "industrial_sunset_02_puresky",
    "phalzer_forest_01",
    "moonless_golf",
];
pub type Color = [f64; 3];
fn add(a: Color, b: Color) -> Color {
    std::array::from_fn(|i| a[i] + b[i])
}
fn scale(a: Color, s: f64) -> Color {
    a.map(|v| v * s)
}
fn cross(a: Color, b: Color) -> Color {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(a: Color) -> Color {
    let len = a.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-20);
    scale(a, 1. / len)
}
fn lerp(a: Color, b: Color, t: f64) -> Color {
    std::array::from_fn(|i| a[i] * (1. - t) + b[i] * t)
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub struct Panorama {
    pub width: u32,
    pub height: u32,
    pixels: Vec<[f32; 3]>,
}
impl Panorama {
    pub fn new(width: u32, height: u32, pixels: Vec<[f32; 3]>) -> io::Result<Self> {
        if width == 0
            || height == 0
            || u64::from(width) * u64::from(height) != pixels.len() as u64
            || pixels.iter().flatten().any(|v| !v.is_finite() || *v < 0.)
        {
            return Err(invalid("Invalid HDR panorama dimensions or radiance"));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        let mut reader = image::ImageReader::open(path)?;
        reader.set_format(image::ImageFormat::Hdr);
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|e| invalid(format!("{}: {e}", path.display())))?
            .into_rgb32f();
        let (width, height) = image.dimensions();
        Self::new(
            width,
            height,
            image
                .into_raw()
                .chunks_exact(3)
                .map(|p| [p[0], p[1], p[2]])
                .collect(),
        )
    }
    fn pixel(&self, x: u32, y: u32) -> Color {
        self.pixels[(y * self.width + x) as usize].map(f64::from)
    }
    pub fn sample(&self, direction: Color) -> Color {
        let u = direction[2].atan2(direction[0]) / TAU + 0.5;
        let v = direction[1].clamp(-1., 1.).acos() / PI;
        self.sample_uv(u, v)
    }
    fn sample_uv(&self, u: f64, v: f64) -> Color {
        let x = u * self.width as f64 - 0.5;
        let y = (v * self.height as f64 - 0.5).clamp(0., (self.height - 1) as f64);
        let x0 = x.floor() as i64;
        let y0 = y.floor() as u32;
        let a = x0.rem_euclid(self.width as i64) as u32;
        let b = (x0 + 1).rem_euclid(self.width as i64) as u32;
        let y1 = (y0 + 1).min(self.height - 1);
        lerp(
            lerp(self.pixel(a, y0), self.pixel(b, y0), x - x0 as f64),
            lerp(self.pixel(a, y1), self.pixel(b, y1), x - x0 as f64),
            y - y0 as f64,
        )
    }
    fn half_size(&self) -> Self {
        let (width, height) = (self.width / 2, self.height / 2);
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                // Preserve the source mip pyramid's float32 box filter, including odd-edge cropping.
                let p = [
                    self.pixels[((y * 2) * self.width + x * 2) as usize],
                    self.pixels[((y * 2) * self.width + x * 2 + 1) as usize],
                    self.pixels[((y * 2 + 1) * self.width + x * 2) as usize],
                    self.pixels[((y * 2 + 1) * self.width + x * 2 + 1) as usize],
                ];
                pixels.push(std::array::from_fn(|i| {
                    (p[0][i] + p[1][i] + p[2][i] + p[3][i]) * 0.25
                }));
            }
        }
        Self {
            width,
            height,
            pixels,
        }
    }
}

/// KTX/Vulkan face order: +X, -X, +Y, -Y, +Z, -Z.
pub fn cube_direction(face: u32, x: u32, y: u32, size: u32) -> Color {
    let u = (x as f32 + 0.5) * (2. / size as f32) - 1.;
    let v = (y as f32 + 0.5) * (2. / size as f32) - 1.;
    let p = match face {
        0 => [1., -v, -u],
        1 => [-1., -v, u],
        2 => [u, 1., v],
        3 => [u, -1., -v],
        4 => [u, -v, 1.],
        _ => [-u, -v, -1.],
    };
    let len = p.iter().map(|v| v * v).sum::<f32>().sqrt();
    p.map(|v| (v / len) as f64)
}

pub struct CubeMip {
    pub size: u32,
    pub pixels: Vec<Color>,
}
fn face_colors(size: u32, color: impl Fn(Color) -> Color + Sync) -> CubeMip {
    let pixels = std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..6)
            .map(|face| {
                let color = &color;
                scope.spawn(move || {
                    let mut pixels = Vec::with_capacity((size * size) as usize);
                    for y in 0..size {
                        for x in 0..size {
                            pixels.push(color(cube_direction(face, x, y, size)));
                        }
                    }
                    pixels
                })
            })
            .collect();
        jobs.into_iter()
            .flat_map(|job| job.join().expect("HDRI face worker panicked"))
            .collect()
    });
    CubeMip { size, pixels }
}
fn sh_basis([x, y, z]: Color) -> [f64; 9] {
    [
        0.2820947918,
        0.4886025119 * x,
        0.4886025119 * y,
        0.4886025119 * z,
        1.0925484306 * x * y,
        1.0925484306 * y * z,
        0.3153915653 * (3. * z * z - 1.),
        1.0925484306 * x * z,
        0.5462742153 * (x * x - y * y),
    ]
}
pub fn diffuse_cube(source: &Panorama) -> CubeMip {
    let mut coefficients = [[0.; 3]; 9];
    for y in 0..source.height {
        let theta = (y as f64 + 0.5) / source.height as f64 * PI;
        let area = theta.sin() * (TAU / source.width as f64) * (PI / source.height as f64);
        for x in 0..source.width {
            let phi = ((x as f64 + 0.5) / source.width as f64 - 0.5) * TAU;
            let basis = sh_basis([
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            ]);
            let pixel = source.pixel(x, y);
            for i in 0..9 {
                for c in 0..3 {
                    coefficients[i][c] += pixel[c] * basis[i] * area;
                }
            }
        }
    }
    // Cosine convolution divided by pi: Bevy expects irradiance/pi.
    let bands = [1., 2. / 3., 2. / 3., 2. / 3., 0.25, 0.25, 0.25, 0.25, 0.25];
    for (i, c) in coefficients.iter_mut().enumerate() {
        *c = scale(*c, bands[i]);
    }
    face_colors(32, |direction| {
        let basis = sh_basis(direction);
        let mut sum = [0.; 3];
        for i in 0..9 {
            sum = add(sum, scale(coefficients[i], basis[i]));
        }
        sum.map(|v| v.max(0.))
    })
}

pub fn validate_options(size: u32, samples: u32) -> io::Result<()> {
    if !size.is_power_of_two() || size > 4096 {
        return Err(invalid("--size must be a power of two between 1 and 4096"));
    }
    if samples == 0 {
        return Err(invalid("--samples must be greater than zero"));
    }
    Ok(())
}
pub fn specular_mips(source: Panorama, size: u32, samples: u32) -> io::Result<Vec<CubeMip>> {
    validate_options(size, samples)?;
    let mut sources = vec![source];
    while sources
        .last()
        .unwrap()
        .width
        .min(sources.last().unwrap().height)
        >= 2
    {
        sources.push(sources.last().unwrap().half_size());
    }
    let mut result = vec![face_colors(size, |direction| sources[0].sample(direction))];
    let levels = size.ilog2() + 1;
    for level in 1..levels {
        let roughness = level as f64 / (levels - 1) as f64;
        let alpha2 = roughness.powi(4);
        let points: Vec<_> = (0..samples)
            .filter_map(|i| {
                let xi = (i as f64 + 0.5) / samples as f64;
                let azimuth =
                    (i.reverse_bits() as f32 * (1. / 4294967296.) * std::f32::consts::TAU) as f64;
                let cos_h = ((1. - xi) / (1. + (alpha2 - 1.) * xi)).sqrt();
                let sin_h = (1. - cos_h * cos_h).max(0.).sqrt();
                let weight = (2. * cos_h * cos_h - 1.).max(0.);
                let pdf = alpha2 / (4. * PI * (cos_h * cos_h * (alpha2 - 1.) + 1.).powi(2));
                (weight > 0.).then_some((
                    [sin_h * azimuth.cos(), sin_h * azimuth.sin(), cos_h],
                    weight,
                    pdf,
                ))
            })
            .collect();
        let weight_sum: f64 = points.iter().map(|p| p.1).sum();
        if weight_sum == 0. {
            return Err(invalid("GGX sampling produced no valid directions"));
        }
        result.push(face_colors(size >> level, |normal| {
            let up = if normal[1].abs() > 0.99 {
                [1., 0., 0.]
            } else {
                [0., 1., 0.]
            };
            let tangent = normalize(cross(up, normal));
            let bitangent = cross(normal, tangent);
            let mut sum = [0.; 3];
            for &(h, weight, pdf) in &points {
                let half = add(
                    add(scale(tangent, h[0]), scale(bitangent, h[1])),
                    scale(normal, h[2]),
                );
                let light = add(scale(half, 2. * h[2]), scale(normal, -1.));
                let area = 2. * PI * PI / (sources[0].width as f64 * sources[0].height as f64)
                    * (1. - light[1] * light[1]).max(0.).sqrt().max(1e-4);
                let lod = (0.5 * (1. / (samples as f64 * pdf * area)).log2())
                    .clamp(0., (sources.len() - 1) as f64);
                let first = lod.floor() as usize;
                let color = lerp(
                    sources[first].sample(light),
                    sources[(first + 1).min(sources.len() - 1)].sample(light),
                    lod.fract(),
                );
                sum = add(sum, scale(color, weight));
            }
            // The previous baker's GGX output was stored as float32 before packing.
            scale(sum, 1. / weight_sum).map(|v| v as f32 as f64)
        }));
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
