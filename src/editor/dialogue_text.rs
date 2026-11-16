//! Optional paragraph formatting; plain text stays compatible with existing dialogue files.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::editor) enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justified,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(in crate::editor) struct TextFormat {
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub alignment: TextAlign,
    pub line_height: f32,
    pub color: Option<[u8; 3]>,
}

impl Default for TextFormat {
    fn default() -> Self {
        Self {
            font_size: 17.0,
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
            alignment: TextAlign::Left,
            line_height: 1.2,
            color: None,
        }
    }
}

impl TextFormat {
    /// Only the character attributes belong to a span; paragraph attributes stay on the node.
    pub fn character(&self) -> Self {
        Self {
            alignment: TextAlign::Left,
            line_height: 1.2,
            ..self.normalized()
        }
    }
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn normalized(&self) -> Self {
        let mut result = self.clone();
        result.font_size = if self.font_size.is_finite() {
            self.font_size.clamp(8.0, 72.0)
        } else {
            Self::default().font_size
        };
        result.line_height = if self.line_height.is_finite() {
            self.line_height.clamp(1.0, 3.0)
        } else {
            Self::default().line_height
        };
        result
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::editor) struct TextSpan {
    pub start: usize,
    pub end: usize,
    pub format: TextFormat,
}

pub(in crate::editor) fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Canonical, complete runs. Later spans take precedence, including malformed legacy input.
pub(in crate::editor) fn runs(text: &str, base: &TextFormat, spans: &[TextSpan]) -> Vec<TextSpan> {
    let mut points = vec![0, text.len()];
    for s in spans {
        points.extend([boundary(text, s.start), boundary(text, s.end)]);
    }
    points.sort_unstable();
    points.dedup();
    let mut out: Vec<TextSpan> = Vec::new();
    for pair in points.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start == end {
            continue;
        }
        let format = spans
            .iter()
            .rev()
            .find(|s| boundary(text, s.start) <= start && boundary(text, s.end) >= end)
            .map_or_else(|| base.character(), |s| s.format.character());
        if let Some(last) = out.last_mut()
            && last.format == format
        {
            last.end = end;
        } else {
            out.push(TextSpan { start, end, format });
        }
    }
    out
}

pub(in crate::editor) fn compact(
    text: &str,
    base: &TextFormat,
    spans: &[TextSpan],
) -> Vec<TextSpan> {
    runs(text, base, spans)
        .into_iter()
        .filter(|s| s.format != base.character())
        .collect()
}

pub(in crate::editor) fn apply(
    text: &str,
    base: &TextFormat,
    spans: &[TextSpan],
    range: std::ops::Range<usize>,
    mut change: impl FnMut(&mut TextFormat),
) -> Vec<TextSpan> {
    let start = boundary(text, range.start);
    let end = boundary(text, range.end);
    let mut out = Vec::new();
    for s in runs(text, base, spans) {
        if s.start < start {
            out.push(TextSpan {
                end: s.end.min(start),
                ..s.clone()
            });
        }
        let a = s.start.max(start);
        let b = s.end.min(end);
        if a < b {
            let mut format = s.format.clone();
            change(&mut format);
            out.push(TextSpan {
                start: a,
                end: b,
                format,
            });
        }
        if s.end > end {
            out.push(TextSpan {
                start: s.start.max(end),
                ..s
            });
        }
    }
    compact(text, base, &out)
}

/// Preserve character styles across one replacement, including typing and IME updates.
pub(in crate::editor) fn replace(
    text: &mut String,
    base: &TextFormat,
    spans: &mut Vec<TextSpan>,
    range: std::ops::Range<usize>,
    value: &str,
) {
    let start = boundary(text, range.start);
    let end = boundary(text, range.end).max(start);
    let inherit = if start == end && start > 0 {
        start - 1
    } else {
        start
    };
    let shift = value.len() as isize - (end - start) as isize;
    let previous = runs(text, base, spans);
    let style = previous
        .iter()
        .find(|s| s.start <= inherit && inherit < s.end)
        .map_or_else(|| base.character(), |s| s.format.clone());
    let mut out = Vec::new();
    for s in previous {
        if s.start < start {
            out.push(TextSpan {
                end: s.end.min(start),
                ..s.clone()
            });
        }
        if s.end > end {
            out.push(TextSpan {
                start: (s.start.max(end) as isize + shift) as usize,
                end: (s.end as isize + shift) as usize,
                format: s.format,
            });
        }
    }
    if !value.is_empty() {
        out.push(TextSpan {
            start,
            end: start + value.len(),
            format: style,
        });
    }
    text.replace_range(start..end, value);
    *spans = compact(text, base, &out);
}

pub(in crate::editor) fn remap(
    old: &str,
    new: &str,
    base: &TextFormat,
    spans: &[TextSpan],
) -> Vec<TextSpan> {
    if old == new {
        return compact(new, base, spans);
    }
    let start = boundary(
        old,
        old.bytes()
            .zip(new.bytes())
            .take_while(|(a, b)| a == b)
            .count(),
    );
    let mut suffix = old[start..]
        .bytes()
        .rev()
        .zip(new[start..].bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    let mut text = old.to_owned();
    let mut result = spans.to_vec();
    replace(
        &mut text,
        base,
        &mut result,
        start..old.len() - suffix,
        &new[start..new.len() - suffix],
    );
    result
}
