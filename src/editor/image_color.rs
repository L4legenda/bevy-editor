//! Transactional tool colours. A picker draft does not edit the image or the active brush.
use bevy::color::{Hsva, Srgba};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum ColorTarget {
    Foreground,
    Background,
    LayerField(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum ColorComponent {
    Hue,
    Saturation,
    Value,
    Red,
    Green,
    Blue,
}

impl ColorComponent {
    pub fn max(self) -> f32 {
        match self {
            Self::Hue => 360.0,
            Self::Saturation | Self::Value => 100.0,
            _ => 255.0,
        }
    }
}

pub(in crate::editor) struct Picker {
    pub target: ColorTarget,
    pub original: [f32; 4],
    pub draft: [f32; 4],
    pub hsv: Hsva,
    pub error: Option<String>,
}

impl Picker {
    pub fn new(target: ColorTarget, color: [f32; 4]) -> Self {
        let mut picker = Self {
            target,
            original: color,
            draft: color,
            hsv: Hsva::hsv(0.0, 0.0, 0.0),
            error: None,
        };
        picker.set_color(color);
        picker
    }
    pub fn set_color(&mut self, color: [f32; 4]) {
        self.draft = color.map(|channel| {
            if channel.is_finite() {
                channel.clamp(0.0, 1.0)
            } else {
                0.0
            }
        });
        self.draft[3] = 1.0;
        let hsv = Hsva::from(Srgba::new(self.draft[0], self.draft[1], self.draft[2], 1.0));
        self.hsv = Hsva {
            hue: if hsv.saturation > 0.00001 {
                hsv.hue
            } else {
                self.hsv.hue
            },
            ..hsv
        };
        self.error = None;
    }
    fn update_rgb(&mut self) {
        let rgb = Srgba::from(self.hsv);
        self.draft = [rgb.red, rgb.green, rgb.blue, 1.0];
        self.error = None;
    }
    pub fn sv(&mut self, saturation: f32, value: f32) {
        if !saturation.is_finite() || !value.is_finite() {
            return;
        }
        self.hsv.saturation = saturation.clamp(0.0, 1.0);
        self.hsv.value = value.clamp(0.0, 1.0);
        self.update_rgb();
    }
    pub fn component(&self, channel: ColorComponent) -> f32 {
        match channel {
            ColorComponent::Hue => self.hsv.hue,
            ColorComponent::Saturation => self.hsv.saturation * 100.0,
            ColorComponent::Value => self.hsv.value * 100.0,
            ColorComponent::Red => self.draft[0] * 255.0,
            ColorComponent::Green => self.draft[1] * 255.0,
            ColorComponent::Blue => self.draft[2] * 255.0,
        }
    }
    pub fn set_component(&mut self, channel: ColorComponent, value: f32) {
        if !value.is_finite() {
            return;
        }
        let value = value.clamp(0.0, channel.max());
        match channel {
            ColorComponent::Hue => self.hsv.hue = value,
            ColorComponent::Saturation => self.hsv.saturation = value / 100.0,
            ColorComponent::Value => self.hsv.value = value / 100.0,
            ColorComponent::Red | ColorComponent::Green | ColorComponent::Blue => {
                let index = match channel {
                    ColorComponent::Red => 0,
                    ColorComponent::Green => 1,
                    _ => 2,
                };
                let mut color = self.draft;
                color[index] = value / 255.0;
                self.set_color(color);
                return;
            }
        }
        self.update_rgb();
    }
    pub fn hex(&self) -> String {
        hex(self.draft)
    }
}

pub(super) fn hex(color: [f32; 4]) -> String {
    let rgb = color.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

pub(in crate::editor) fn parse_hex(text: &str) -> Option<[f32; 4]> {
    let text = text.trim();
    let text = text.strip_prefix('#').unwrap_or(text);
    if !matches!(text.len(), 3 | 6) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let expanded = if text.len() == 3 {
        text.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        text.to_owned()
    };
    let rgb = u32::from_str_radix(&expanded, 16).ok()?;
    Some([
        ((rgb >> 16) & 255) as f32 / 255.0,
        ((rgb >> 8) & 255) as f32 / 255.0,
        (rgb & 255) as f32 / 255.0,
        1.0,
    ])
}

pub(in crate::editor) const PRESETS: [[f32; 4]; 7] = [
    [0.0, 0.0, 0.0, 1.0],
    [1.0; 4],
    [0.95, 0.24, 0.26, 1.0],
    [1.0, 0.65, 0.2, 1.0],
    [0.28, 0.75, 0.62, 1.0],
    [0.25, 0.55, 1.0, 1.0],
    [0.65, 0.4, 0.95, 1.0],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_picker_hex_accepts_short_and_full_values_and_rejects_invalid_input() {
        assert_eq!(hex(parse_hex(" #f80 ").unwrap()), "FF8800");
        assert_eq!(hex(parse_hex("0aB9Ff").unwrap()), "0AB9FF");
        for text in [
            "", "12", "1234", "1234567", "##123", "#GG0000", "＋12345", "红色",
        ] {
            assert!(parse_hex(text).is_none(), "{text:?}");
        }
    }

    #[test]
    fn color_picker_hsv_rgb_stay_in_sync_and_preserve_hue_through_grey() {
        let mut picker = Picker::new(ColorTarget::Foreground, [1.0, 0.0, 0.0, 1.0]);
        picker.set_component(ColorComponent::Hue, 240.0);
        assert_eq!(picker.hex(), "0000FF");
        picker.sv(0.0, 0.5);
        assert_eq!(picker.hex(), "808080");
        assert_eq!(picker.hsv.hue, 240.0);
        picker.set_color([0.0, 0.0, 0.0, 1.0]);
        assert_eq!(picker.hsv.hue, 240.0);
        picker.sv(1.0, 1.0);
        assert_eq!(picker.hex(), "0000FF");
        picker.set_component(ColorComponent::Green, 255.0);
        assert_eq!(picker.hex(), "00FFFF");
        assert!((picker.component(ColorComponent::Hue) - 180.0).abs() < 0.01);
        let before = picker.draft;
        picker.set_component(ColorComponent::Red, f32::NAN);
        picker.sv(f32::INFINITY, 0.0);
        assert_eq!(picker.draft, before);
    }
}
