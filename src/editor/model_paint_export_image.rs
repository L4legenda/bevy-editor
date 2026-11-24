use crate::editor::model_paint_data::CompositeSample;
use bevy::prelude::{LinearRgba, Srgba, Vec3};
use image::{
    DynamicImage, ExtendedColorType, ImageBuffer, ImageEncoder, ImageFormat, ImageResult, Rgb,
    Rgba, codecs::jpeg::JpegEncoder, codecs::png::PngEncoder, imageops,
};
use std::io::{Seek, Write};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::editor) enum Format {
    #[default]
    Png,
    Tga,
    Jpeg,
    Tiff,
    Exr,
}

impl Format {
    pub(in crate::editor) const ALL: [Self; 5] =
        [Self::Png, Self::Tga, Self::Jpeg, Self::Tiff, Self::Exr];

    pub(in crate::editor) fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Tga => "TGA",
            Self::Jpeg => "JPEG",
            Self::Tiff => "TIFF",
            Self::Exr => "OpenEXR",
        }
    }

    pub(in crate::editor) fn depths(self) -> &'static [Depth] {
        match self {
            Self::Png | Self::Tiff => &[Depth::U8, Depth::U16],
            Self::Tga | Self::Jpeg => &[Depth::U8],
            Self::Exr => &[Depth::Float],
        }
    }

    pub(super) fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Tga => "tga",
            Self::Jpeg => "jpg",
            Self::Tiff => "tif",
            Self::Exr => "exr",
        }
    }

    fn image_format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Tga => ImageFormat::Tga,
            Self::Jpeg => ImageFormat::Jpeg,
            Self::Tiff => ImageFormat::Tiff,
            Self::Exr => ImageFormat::OpenExr,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::editor) enum Depth {
    #[default]
    U8,
    U16,
    Float,
}

impl Depth {
    pub(in crate::editor) fn label(self) -> &'static str {
        match self {
            Self::U8 => "8 бит",
            Self::U16 => "16 бит",
            Self::Float => "32 бита float",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(in crate::editor) struct Options {
    pub format: Format,
    pub depth: Depth,
    pub size: Option<u32>,
    pub normal: NormalOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(in crate::editor) struct NormalOptions {
    pub height_strength: f32,
    pub use_baked: bool,
}

impl Default for NormalOptions {
    fn default() -> Self {
        Self {
            height_strength: 1.,
            use_baked: true,
        }
    }
}

impl Options {
    pub(in crate::editor) fn set_format(&mut self, format: Format) {
        self.format = format;
        if !format.depths().contains(&self.depth) {
            self.depth = format.depths()[0];
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Samples {
    #[default]
    Data,
    Color,
    Normal,
    Height,
    Packed {
        srgb: u8,
        normal: bool,
        force_16: bool,
    },
}

pub(super) fn write(
    writer: &mut (impl Write + Seek),
    size: u32,
    pixels: Vec<u8>,
    channels: usize,
    samples: Samples,
    options: Options,
) -> ImageResult<()> {
    let depth = output_depth(samples, options);
    let out_size = options.size.unwrap_or(size);
    if depth == Depth::U8 && size == out_size {
        return write_u8(writer, size, pixels, channels, options.format);
    }
    if depth == Depth::U16 && size == out_size {
        let values = pixels
            .into_iter()
            .map(|value| u16::from(value) * 257)
            .collect();
        return write_u16(
            writer,
            size,
            values,
            channels,
            options.format.image_format(),
        );
    }
    write_resampled(writer, size, pixels, channels, samples, options, depth)
}

pub(super) fn output_depth(samples: Samples, options: Options) -> Depth {
    let force_16 = matches!(
        samples,
        Samples::Height | Samples::Packed { force_16: true, .. }
    );
    if force_16 && options.format.depths().contains(&Depth::U16) {
        Depth::U16
    } else if options.format.depths().contains(&options.depth) {
        options.depth
    } else {
        options.format.depths()[0]
    }
}

pub(super) fn write_float(
    writer: &mut (impl Write + Seek),
    size: u32,
    pixels: Vec<f32>,
    channels: usize,
    samples: Samples,
    options: Options,
) -> ImageResult<()> {
    let depth = output_depth(samples, options);
    if depth == Depth::U16 && options.size.unwrap_or(size) == size {
        let values = pixels.into_iter().map(to_u16).collect();
        return write_u16(
            writer,
            size,
            values,
            channels,
            options.format.image_format(),
        );
    }
    write_resampled(writer, size, pixels, channels, samples, options, depth)
}

fn to_u16(value: f32) -> u16 {
    (value.clamp(0., 1.) * 65535.).round() as u16
}

fn write_resampled<T: CompositeSample>(
    writer: &mut (impl Write + Seek),
    size: u32,
    pixels: Vec<T>,
    channels: usize,
    samples: Samples,
    options: Options,
    depth: Depth,
) -> ImageResult<()> {
    let out_size = options.size.unwrap_or(size);
    let srgb = if options.format == Format::Exr {
        match samples {
            Samples::Color => 0b0111,
            Samples::Packed { srgb, .. } if channels == 1 && srgb & 1 != 0 => 0b0111,
            Samples::Packed { srgb, .. } => srgb,
            _ => 0,
        }
    } else {
        0
    };
    let mut rgba = ImageBuffer::from_vec(
        size,
        size,
        pixels
            .chunks_exact(channels)
            .flat_map(|pixel| {
                let v = |i: usize| pixel[i].unit();
                let (r, g, b) = if channels == 1 {
                    (v(0), v(0), v(0))
                } else {
                    (v(0), v(1), v(2))
                };
                let a = if channels == 4 { v(3) } else { 1. };
                if srgb == 0b0111 {
                    let c = LinearRgba::from(Srgba::new(r, g, b, a));
                    [c.red, c.green, c.blue, a]
                } else {
                    let mut values = [r, g, b, a];
                    for (channel, value) in values.iter_mut().enumerate() {
                        if srgb & (1 << channel) != 0 {
                            *value = LinearRgba::from(Srgba::new(*value, 0., 0., 1.)).red;
                        }
                    }
                    values
                }
            })
            .collect(),
    )
    .expect("one RGBA texel per input texel");
    drop(pixels);
    if out_size != size {
        let filter = if out_size < size {
            imageops::FilterType::Triangle
        } else {
            imageops::FilterType::CatmullRom
        };
        rgba = imageops::resize(&rgba, out_size, out_size, filter);
        if matches!(
            samples,
            Samples::Normal | Samples::Packed { normal: true, .. }
        ) {
            for Rgba([r, g, b, a]) in rgba.pixels_mut() {
                if *a == 0. && [*r, *g, *b] == [0.; 3] {
                    continue;
                }
                let normal = (Vec3::new(*r, *g, *b) * 2. - Vec3::ONE).normalize_or(Vec3::Z);
                [*r, *g, *b] = (normal * 0.5 + Vec3::splat(0.5)).to_array();
            }
        }
    }
    let image_format = options.format.image_format();
    match depth {
        Depth::U8 => {
            let bytes = rgba
                .pixels()
                .flat_map(|pixel| {
                    pixel
                        .0
                        .map(|value| (value.clamp(0., 1.) * 255.).round() as u8)
                        .into_iter()
                        .take(channels)
                })
                .collect();
            write_u8(writer, out_size, bytes, channels, options.format)
        }
        Depth::U16 => {
            let values = rgba
                .pixels()
                .flat_map(|pixel| pixel.0.map(to_u16).into_iter().take(channels))
                .collect();
            write_u16(writer, out_size, values, channels, image_format)
        }
        Depth::Float => {
            if channels == 4 {
                DynamicImage::ImageRgba32F(rgba).write_to(writer, image_format)
            } else {
                DynamicImage::ImageRgb32F(ImageBuffer::from_fn(out_size, out_size, |x, y| {
                    let [r, g, b, _] = rgba.get_pixel(x, y).0;
                    Rgb([r, g, b])
                }))
                .write_to(writer, image_format)
            }
        }
    }
}

fn write_u16(
    writer: &mut (impl Write + Seek),
    size: u32,
    values: Vec<u16>,
    channels: usize,
    format: ImageFormat,
) -> ImageResult<()> {
    match channels {
        1 => DynamicImage::ImageLuma16(ImageBuffer::from_raw(size, size, values).unwrap()),
        3 => DynamicImage::ImageRgb16(ImageBuffer::from_raw(size, size, values).unwrap()),
        4 => DynamicImage::ImageRgba16(ImageBuffer::from_raw(size, size, values).unwrap()),
        _ => unreachable!(),
    }
    .write_to(writer, format)
}

fn write_u8(
    writer: &mut (impl Write + Seek),
    size: u32,
    mut bytes: Vec<u8>,
    mut channels: usize,
    format: Format,
) -> ImageResult<()> {
    if format == Format::Jpeg && channels == 4 {
        let mut index = 0;
        bytes.retain(|_| {
            index += 1;
            index % 4 != 0
        });
        channels = 3;
    }
    let color = match channels {
        1 => ExtendedColorType::L8,
        3 => ExtendedColorType::Rgb8,
        4 => ExtendedColorType::Rgba8,
        _ => unreachable!(),
    };
    match format {
        Format::Png => PngEncoder::new(writer).write_image(&bytes, size, size, color),
        Format::Jpeg => JpegEncoder::new_with_quality(writer, 95).encode(&bytes, size, size, color),
        _ => match channels {
            1 => DynamicImage::ImageLuma8(ImageBuffer::from_raw(size, size, bytes).unwrap()),
            3 => DynamicImage::ImageRgb8(ImageBuffer::from_raw(size, size, bytes).unwrap()),
            4 => DynamicImage::ImageRgba8(ImageBuffer::from_raw(size, size, bytes).unwrap()),
            _ => unreachable!(),
        }
        .write_to(writer, format.image_format()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use std::io::Cursor;

    fn encode(
        format: Format,
        depth: Depth,
        samples: Samples,
        channels: usize,
        pixel: &[u8],
    ) -> DynamicImage {
        let mut file = Cursor::new(Vec::new());
        write(
            &mut file,
            2,
            pixel.repeat(4),
            channels,
            samples,
            Options {
                format,
                depth,
                ..Options::default()
            },
        )
        .unwrap();
        image::load_from_memory_with_format(&file.into_inner(), format.image_format()).unwrap()
    }

    #[test]
    fn export_file_round_trips_all_formats_and_supported_depths() {
        for format in Format::ALL {
            for &depth in format.depths() {
                let rgba = encode(format, depth, Samples::Color, 4, &[128, 64, 32, 96]);
                assert_eq!(rgba.dimensions(), (2, 2));
                match depth {
                    Depth::U8 if format == Format::Jpeg => {
                        assert!(!rgba.color().has_alpha());
                        let [r, g, b, _] = rgba.get_pixel(0, 0).0;
                        for (actual, expected) in [r, g, b].into_iter().zip([128, 64, 32]) {
                            assert!(actual.abs_diff(expected) <= 3);
                        }
                    }
                    Depth::U8 => assert_eq!(rgba.get_pixel(0, 0).0, [128, 64, 32, 96]),
                    Depth::U16 => {
                        assert_eq!(rgba.color(), image::ColorType::Rgba16);
                        assert_eq!(
                            rgba.to_rgba16().get_pixel(0, 0).0,
                            [128u16, 64, 32, 96].map(|v| v * 257)
                        );
                    }
                    Depth::Float => {
                        assert_eq!(rgba.color(), image::ColorType::Rgba32F);
                        let [r, _, _, a] = rgba.to_rgba32f().get_pixel(0, 0).0;
                        assert!((r - 0.21586).abs() < 0.00001);
                        assert!((a - 96. / 255.).abs() < 0.00001);
                    }
                }
                let gray = encode(format, depth, Samples::Data, 1, &[128]);
                if depth == Depth::Float {
                    assert_eq!(gray.to_rgb32f().get_pixel(0, 0).0, [128. / 255.; 3]);
                } else {
                    assert_eq!(gray.to_luma8().get_pixel(0, 0).0, [128]);
                }
            }
        }
        let height = encode(Format::Png, Depth::U8, Samples::Height, 1, &[77]);
        assert_eq!(height.color(), image::ColorType::L16);
    }

    #[test]
    fn export_packed_exr_linearizes_only_color_components() {
        let packed = encode(
            Format::Exr,
            Depth::Float,
            Samples::Packed {
                srgb: 0b1001,
                normal: false,
                force_16: false,
            },
            4,
            &[128; 4],
        )
        .into_rgba32f();
        let [r, g, b, a] = packed.get_pixel(0, 0).0;
        assert!((r - 0.21586).abs() < 0.00001);
        assert_eq!([g, b], [128. / 255.; 2]);
        assert_eq!(r, a);
    }

    #[test]
    fn export_file_resizes_scalar_data_and_renormalizes_normals() {
        let mut file = Cursor::new(Vec::new());
        let options = Options {
            size: Some(1),
            ..Options::default()
        };
        write(
            &mut file,
            2,
            vec![0, 255, 255, 0],
            1,
            Samples::Data,
            options,
        )
        .unwrap();
        let gray = image::load_from_memory(&file.into_inner())
            .unwrap()
            .into_luma8();
        assert_eq!(gray.dimensions(), (1, 1));
        assert_eq!(gray.get_pixel(0, 0).0, [128]);
        let mut file = Cursor::new(Vec::new());
        let pixels = [[255, 128, 128], [128, 128, 255]].concat().repeat(2);
        write(&mut file, 2, pixels, 3, Samples::Normal, options).unwrap();
        let normal = image::load_from_memory(&file.into_inner())
            .unwrap()
            .into_rgb8();
        let normal = Vec3::from_array(normal.get_pixel(0, 0).0.map(|v| v as f32 / 127.5 - 1.));
        assert!((normal.length() - 1.).abs() < 0.01);
        assert!((normal.x - normal.z).abs() < 0.01);
    }
}
