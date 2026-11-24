//! WGSL editor presentation using Bevy's native editable glyphs and input.
use super::*;
use bevy::ui_widgets::TextInput;
use bevy::{
    input_focus::tab_navigation::TabIndex,
    picking::events::PointerScroll,
    text::{
        ComputedTextBlock, EditableText, FontCx, LayoutCx, LetterSpacing, LineBreak, LineHeight,
        RemSize, TextBackgroundColor, TextBounds, TextBrush, TextEdit, TextElement, TextPipeline,
        TextSpan,
    },
};
use std::{fmt::Write, ops::Range};

const ROW_HEIGHT: f32 = 18.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
enum Ink {
    Plain,
    Comment,
    Tag,
    Keyword,
    Type,
    Builtin,
    Number,
    Parameter,
}

const COLORS: [Color; 8] = [
    TEXT,
    Color::srgb(0.53, 0.66, 0.53),
    Color::srgb(0.91, 0.75, 0.39),
    Color::srgb(0.79, 0.55, 0.88),
    Color::srgb(0.40, 0.81, 0.76),
    Color::srgb(0.48, 0.70, 0.98),
    Color::srgb(0.94, 0.66, 0.46),
    Color::srgb(0.95, 0.80, 0.52),
];

fn word(word: &str) -> Ink {
    match word {
        "fn" | "let" | "var" | "const" | "if" | "else" | "for" | "loop" | "while" | "return"
        | "switch" | "case" | "default" | "break" | "continue" | "struct" | "true" | "false"
        | "alias" | "override" | "enable" | "requires" | "diagnostic" | "const_assert"
        | "discard" | "continuing" => Ink::Keyword,
        "f16" | "f32" | "i32" | "u32" | "bool" | "vec2" | "vec3" | "vec4" | "mat2x2" | "mat2x3"
        | "mat2x4" | "mat3x2" | "mat3x3" | "mat3x4" | "mat4x2" | "mat4x3" | "mat4x4" | "array"
        | "texture_2d" | "sampler" | "ptr" | "atomic" | "Shade" | "Surface" => Ink::Type,
        "sin" | "cos" | "tan" | "atan2" | "abs" | "floor" | "ceil" | "fract" | "round" | "min"
        | "max" | "clamp" | "mix" | "step" | "smoothstep" | "length" | "distance" | "dot"
        | "cross" | "normalize" | "pow" | "exp" | "log" | "sqrt" | "select" | "radians"
        | "degrees" | "sign" | "textureSampleLevel" | "textureLoad" | "perlin" | "fbm2"
        | "value_noise" | "cells" | "rotate2" | "px" | "res" | "luma" | "hash1" | "hash2"
        | "wrap_cell" | "grad_dot" | "saturate" | "trunc" | "inverseSqrt" => Ink::Builtin,
        _ if word == "params" || word.starts_with("in_") || word.starts_with("img_") => {
            Ink::Parameter
        }
        _ => Ink::Plain,
    }
}

fn identifier(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn block_comment(source: &str) -> usize {
    let mut depth = 1;
    let mut end = 2;
    while let Some(i) = source[end..].find(['/', '*']) {
        end += i;
        if source[end..].starts_with("/*") {
            depth += 1;
            end += 2;
        } else if source[end..].starts_with("*/") {
            depth -= 1;
            end += 2;
            if depth == 0 {
                return end;
            }
        } else {
            end += 1;
        }
    }
    source.len()
}

fn styled_ranges(source: &str) -> Vec<(Range<usize>, Ink)> {
    let mut spans = Vec::new();
    let mut start = 0;
    while start < source.len() {
        let rest = &source[start..];
        let first = rest.chars().next().unwrap();
        let (width, ink) = if rest.starts_with("//") {
            let width = rest.find('\n').unwrap_or(rest.len());
            let comment = &rest[..width];
            if let Some(tag) = comment.find('@') {
                let end = comment[tag..]
                    .find(char::is_whitespace)
                    .map_or(width, |i| tag + i);
                spans.push((start..start + tag, Ink::Comment));
                spans.push((start + tag..start + end, Ink::Tag));
                start += end;
                (width - end, Ink::Comment)
            } else {
                (width, Ink::Comment)
            }
        } else if rest.starts_with("/*") {
            (block_comment(rest), Ink::Comment)
        } else if first.is_ascii_digit()
            || (first == '.' && rest.as_bytes().get(1).is_some_and(u8::is_ascii_digit))
        {
            let width = rest
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '.')
                .unwrap_or(rest.len());
            (width, Ink::Number)
        } else if identifier(first) {
            let width = rest.find(|c| !identifier(c)).unwrap_or(rest.len());
            (width, word(&rest[..width]))
        } else if first == '@' {
            let width = rest[1..]
                .find(|c| !identifier(c))
                .map_or(rest.len(), |i| i + 1);
            (width, Ink::Tag)
        } else {
            (first.len_utf8(), Ink::Plain)
        };
        if width > 0 && ink != Ink::Plain {
            spans.push((start..start + width, ink));
        }
        start += width;
    }
    spans
}

pub(super) fn error_line(message: &str) -> Option<usize> {
    let (_, location) = message.split_once("line ")?;
    let end = location
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(location.len());
    location[..end].parse().ok().filter(|&line| line > 0)
}

#[derive(Resource)]
pub(in crate::editor::ui::model_viewer) struct CodeUi {
    source: Entity,
    palette: [Entity; COLORS.len()],
    gutter: Entity,
    marker: Entity,
    suffix: Entity,
    text: String,
    lines: usize,
    visible: Option<(usize, usize, Option<usize>)>,
}

pub(super) fn setup(
    commands: &mut Commands,
    parent: Entity,
    initial: &str,
    face: Handle<Font>,
) -> Entity {
    let font = TextFont {
        font: face.into(),
        font_size: FontSize::Px(13.),
        ..default()
    };
    let gutter_clip = surface(
        commands,
        parent,
        Node {
            width: px(43),
            height: percent(100),
            flex_shrink: 0.,
            overflow: Overflow::clip(),
            ..default()
        },
        CHROME,
    );
    commands.entity(gutter_clip).insert(Pickable::IGNORE);
    let gutter = commands
        .spawn((
            Text::new(""),
            TextColor(MUTED),
            font.clone(),
            LineHeight::Px(ROW_HEIGHT),
            TextLayout::no_wrap().with_justify(bevy::text::Justify::Right),
            Node {
                width: percent(100),
                right: px(5),
                position_type: PositionType::Absolute,
                ..default()
            },
            ChildOf(gutter_clip),
            Pickable::IGNORE,
        ))
        .id();
    let marker = commands
        .spawn((
            TextSpan::new(""),
            font.clone(),
            TextColor(Color::WHITE),
            TextBackgroundColor(Color::srgb(0.42, 0.14, 0.16)),
            LineHeight::Px(ROW_HEIGHT),
            ChildOf(gutter),
        ))
        .id();
    let suffix = commands
        .spawn((
            TextSpan::new(""),
            font.clone(),
            TextColor(MUTED),
            LineHeight::Px(ROW_HEIGHT),
            ChildOf(gutter),
        ))
        .id();
    let mut input = EditableText::new(initial);
    input.allow_newlines = true;
    input.visible_lines = None;
    let source = commands
        .spawn((
            TextInput,
            input,
            TabIndex(0),
            ComputedTextBlock::default(),
            crate::ui_theme::text_cursor_style(),
            TextLayout::no_wrap(),
            font,
            LineHeight::Px(ROW_HEIGHT),
            TextColor(TEXT),
            AccessibleLabel("Исходный код WGSL".into()),
            Node {
                height: percent(100),
                min_height: px(0),
                min_width: px(0),
                flex_basis: px(0),
                flex_grow: 1.,
                ..default()
            },
            ChildOf(parent),
        ))
        .id();
    commands.entity(parent).observe(
        move |mut event: On<PointerScroll>, mut inputs: Query<&mut EditableText>| {
            if let Ok(mut input) = inputs.get_mut(source) {
                let unit = if event.unit == bevy::input::mouse::MouseScrollUnit::Line {
                    ROW_HEIGHT * 3.
                } else {
                    1.
                };
                let delta = Vec2::new(-event.x, -event.y) * (input.editor.get_scale() * unit);
                input.queue_edit(TextEdit::ScrollBy(delta));
                event.propagate(false);
            }
        },
    );
    let palette = std::array::from_fn(|i| {
        if i == Ink::Plain as usize {
            source
        } else {
            commands.spawn((TextColor(COLORS[i]), ChildOf(source))).id()
        }
    });
    commands.insert_resource(CodeUi {
        source,
        palette,
        gutter,
        marker,
        suffix,
        text: String::new(),
        lines: 1,
        visible: None,
    });
    source
}

pub(in crate::editor::ui::model_viewer) fn decorate(
    mut ui: ResMut<CodeUi>,
    maker: Res<ShaderMaker>,
    mut code: Query<(
        &mut EditableText,
        &mut ComputedTextBlock,
        &TextFont,
        &ComputedNode,
    )>,
    mut text: Query<&mut Text>,
    mut spans: Query<&mut TextSpan>,
    mut nodes: Query<&mut Node>,
    mut pipeline: ResMut<TextPipeline>,
    fonts: Res<Assets<Font>>,
    mut font_cx: ResMut<FontCx>,
    mut layout_cx: ResMut<LayoutCx>,
    rem: Res<RemSize>,
) {
    let Ok((mut input, mut computed, font, node)) = code.get_mut(ui.source) else {
        return;
    };
    // EditableText shapes the actual source, while the stock UI renderer resolves its
    // glyph brush indices through this section table. No duplicate editor text is drawn.
    if computed.entities().len() != COLORS.len() {
        let sections = ui.palette.into_iter().zip(COLORS).map(|(entity, color)| {
            (
                entity,
                0,
                TextElement::Text {
                    text: "",
                    font,
                    color,
                    line_height: LineHeight::Px(ROW_HEIGHT),
                    letter_spacing: LetterSpacing::default(),
                },
            )
        });
        if let Err(error) = pipeline.update_buffer(
            &fonts,
            sections,
            LineBreak::NoWrap,
            bevy::text::Justify::Left,
            TextBounds::UNBOUNDED,
            1.,
            &mut computed,
            &mut font_cx,
            &mut layout_cx,
            Vec2::ZERO,
            *rem,
        ) {
            warn!("Could not set WGSL syntax palette: {error}");
        }
    }
    if ui.text != input.editor.raw_text() {
        ui.text.clear();
        ui.text.push_str(input.editor.raw_text());
        ui.lines = ui.text.bytes().filter(|&c| c == b'\n').count() + 1;
        let styles = styled_ranges(&ui.text)
            .into_iter()
            .map(|(range, ink)| {
                (
                    range,
                    parley::StyleProperty::Brush(TextBrush::new(ink as u32, font.font_smoothing)),
                )
            })
            .collect();
        input.editor_mut().set_ranged_styles(styles);
    }
    let offset = input.viewport.offset.y / input.editor.get_scale().max(0.01);
    let first = ((offset / ROW_HEIGHT).floor() as usize + 1).min(ui.lines);
    let end =
        (first + (node.size().y * node.inverse_scale_factor() / ROW_HEIGHT).ceil() as usize + 1)
            .min(ui.lines);
    let error = maker
        .error_line()
        .filter(|line| (first..=end).contains(line));
    if ui.visible != Some((first, end, error)) {
        let mut before = String::new();
        let mut after = String::new();
        for line in first..=end {
            if Some(line) == error {
                continue;
            }
            let target = if error.is_some_and(|error| line > error) {
                &mut after
            } else {
                &mut before
            };
            let _ = writeln!(target, "{line}");
        }
        if let Ok(mut text) = text.get_mut(ui.gutter) {
            text.0 = before;
        }
        if let Ok(mut span) = spans.get_mut(ui.marker) {
            span.0 = error.map_or_else(String::new, |line| format!("{line}\n"));
        }
        if let Ok(mut span) = spans.get_mut(ui.suffix) {
            span.0 = after;
        }
        ui.visible = Some((first, end, error));
    }
    if let Ok(mut node) = nodes.get_mut(ui.gutter) {
        let top = px((first - 1) as f32 * ROW_HEIGHT - offset);
        if node.top != top {
            node.top = top;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wgsl_lexer_keeps_utf8_offsets_and_distinguishes_comments() {
        let code = "// @name Узел\nfn node(uv: vec2<f32>) { /* let /* 0 */ x */ return sin(in_noise(uv) + .5); }";
        let spans: Vec<_> = styled_ranges(code)
            .into_iter()
            .map(|(range, ink)| (&code[range], ink))
            .collect();
        assert_eq!(
            spans,
            [
                ("// ", Ink::Comment),
                ("@name", Ink::Tag),
                (" Узел", Ink::Comment),
                ("fn", Ink::Keyword),
                ("vec2", Ink::Type),
                ("f32", Ink::Type),
                ("/* let /* 0 */ x */", Ink::Comment),
                ("return", Ink::Keyword),
                ("sin", Ink::Builtin),
                ("in_noise", Ink::Parameter),
                (".5", Ink::Number),
            ]
        );
        assert_eq!(error_line("error at line 12:8"), Some(12));
        assert_eq!(error_line("generated wgsl:100:8"), None);
    }
}
