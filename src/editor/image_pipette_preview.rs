//! Read-only colour under the sampling hotspot; reuse the actual picker's compositor.
use super::*;
use crate::editor::photo_document::PhotoDocument;

const SIDE: f32 = 28.0;

#[derive(Clone, Copy)]
pub(in crate::editor) struct Sample {
    pub position: Vec2,
    pub color: Color,
}

#[derive(Default)]
pub(in crate::editor) struct Sampling {
    key: Option<(photocraft_doc::DocId, u64, IVec2)>,
    color: Color,
}
impl Sampling {
    pub fn sample(
        &mut self,
        doc: &PhotoDocument,
        cursor: Vec2,
        image: Rect,
        bounds: Rect,
    ) -> Option<Sample> {
        if !image.contains(cursor) || !bounds.contains(cursor) {
            return None;
        }
        let point = (cursor - image.min) / image.size()
            * Vec2::new(doc.size().width as f32, doc.size().height as f32);
        if !point.is_finite()
            || !point.cmpge(Vec2::ZERO).all()
            || !point
                .cmplt(Vec2::new(doc.size().width as f32, doc.size().height as f32))
                .all()
        {
            return None;
        }
        let pixel = point.floor().as_ivec2();
        let key = (doc.document.id, doc.revision, pixel);
        if self.key != Some(key) {
            let composite = doc.composite_region(photocraft_geom::Rect::new(
                pixel.x,
                pixel.y,
                pixel.x + 1,
                pixel.y + 1,
            ));
            let p = composite.pixels.get(..4)?;
            // Picking changes the RGB tool colour. Match that exact click result.
            self.color = Color::srgb_u8(p[0], p[1], p[2]);
            self.key = Some(key);
        }
        let mut position = cursor + Vec2::splat(14.0);
        if position.x + SIDE > bounds.max.x - 3.0 {
            position.x = cursor.x - SIDE - 14.0;
        }
        if position.y + SIDE > bounds.max.y - 3.0 {
            position.y = cursor.y - SIDE - 14.0;
        }
        position = position.clamp(
            bounds.min + Vec2::splat(3.0),
            (bounds.max - Vec2::splat(SIDE + 3.0)).max(bounds.min + Vec2::splat(3.0)),
        );
        Some(Sample {
            position,
            color: self.color,
        })
    }
}

pub(in crate::editor) struct Preview {
    root: Entity,
    fill: Entity,
}
impl Preview {
    pub fn setup(commands: &mut Commands, canvas: Entity) -> Self {
        let root = surface(
            commands,
            canvas,
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                width: px(SIDE),
                height: px(SIDE),
                padding: UiRect::all(px(1)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
                ..default()
            },
            Color::WHITE,
        );
        commands.entity(root).insert((
            ZIndex(9),
            Pickable::IGNORE,
            BorderColor::all(Color::BLACK),
            AccessibleLabel("Цвет под пипеткой".into()),
            BoxShadow(vec![ShadowStyle {
                color: Color::srgba(0.0, 0.0, 0.0, 0.35),
                x_offset: px(0),
                y_offset: px(2),
                spread_radius: px(0),
                blur_radius: px(6),
            }]),
        ));
        let fill = surface(
            commands,
            root,
            Node {
                width: percent(100),
                height: percent(100),
                border_radius: BorderRadius::all(px(2)),
                ..default()
            },
            Color::BLACK,
        );
        commands.entity(fill).insert((
            Pickable::IGNORE,
            AccessibleLabel("Образец цвета пипетки".into()),
        ));
        Self { root, fill }
    }
    pub fn refresh(
        &self,
        nodes: &mut Query<&mut Node>,
        colors: &mut Query<&mut BackgroundColor>,
        sample: Option<Sample>,
    ) {
        if let Ok(mut node) = nodes.get_mut(self.root) {
            let display = if sample.is_some() {
                Display::Flex
            } else {
                Display::None
            };
            if node.display != display {
                node.display = display;
            }
            if let Some(sample) = sample {
                node.left = px(sample.position.x);
                node.top = px(sample.position.y);
            }
        }
        if let Some(sample) = sample
            && let Ok(mut color) = colors.get_mut(self.fill)
        {
            color.set_if_neq(BackgroundColor(sample.color));
        }
    }
}
