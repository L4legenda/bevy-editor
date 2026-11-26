//! Rotation handles in logical canvas pixels. Drawing and picking use the same front arcs.
use super::*;
use resvg::tiny_skia as sk;
use std::f32::consts::{FRAC_PI_2, TAU};

pub(super) const ANGLE_STEP: f32 = TAU / 64.0;
const SIDE: f32 = 2.0 * (FREE_RADIUS + 4.0);
const LABEL_RADIUS: f32 = 9.0;

pub(super) fn segments(frame: &Frame, axis: usize) -> Vec<(Vec2, Vec2)> {
    let (u, v, start, count, radius) = if axis == FREE {
        (Vec2::X, Vec2::Y, 0.0, 64, FREE_RADIUS)
    } else {
        let (u, v) = (AXES[(axis + 1) % 3], AXES[(axis + 2) % 3]);
        let eye = *frame.view.back();
        let depth = Vec2::new(eye.dot(u), eye.dot(v));
        // Face-on, the whole ring is the silhouette. Otherwise expose its near half.
        let (start, count) = if depth.length_squared() < 1e-6 {
            (0.0, 64)
        } else {
            (depth.y.atan2(depth.x) - FRAC_PI_2, 32)
        };
        (frame.project(u), frame.project(v), start, count, RADIUS)
    };
    let point = |i: usize| {
        let (sin, cos) = (start + i as f32 * ANGLE_STEP).sin_cos();
        frame.center + (u * cos + v * sin) * radius
    };
    let mut a = point(0);
    (1..=count)
        .map(|i| {
            let b = point(i);
            (std::mem::replace(&mut a, b), b)
        })
        .collect()
}

// The three diagonals of the camera-facing octant stay on different arcs,
// including when two rings are viewed edge-on and cross at the pivot.
fn label_radial(frame: &Frame, axis: usize) -> Vec3 {
    let toward_eye = |i: usize| {
        if frame.view.back()[i] < 0.0 {
            -AXES[i]
        } else {
            AXES[i]
        }
    };
    (toward_eye((axis + 1) % 3) + toward_eye((axis + 2) % 3)) * std::f32::consts::FRAC_1_SQRT_2
}

pub(super) fn label_hit(frame: &Frame, cursor: Vec2) -> Option<(usize, Vec2, f32)> {
    (0..3).find_map(|axis| {
        let radial = label_radial(frame, axis);
        let center = frame.center + frame.project(radial) * RADIUS;
        if cursor.distance_squared(center) > (LABEL_RADIUS + 1.0).powi(2) {
            return None;
        }
        let tangent = frame.project(AXES[axis].cross(radial));
        Some((
            axis,
            tangent.normalize_or_zero(),
            (RADIUS * tangent.length()).recip().min(0.04),
        ))
    })
}

fn raster(frame: &Frame, highlight: Option<usize>, dragging: bool, side: u32) -> Image {
    let mut pixmap = sk::Pixmap::new(side, side).unwrap();
    let scale = sk::Transform::from_scale(side as f32 / SIDE, side as f32 / SIDE);
    for axis in (0..=FREE)
        .filter(|axis| Some(*axis) != highlight)
        .chain(highlight)
    {
        let lines = segments(frame, axis);
        let local = |point: Vec2| point - frame.center + Vec2::splat(SIDE * 0.5);
        let start = local(lines[0].0);
        let mut path = sk::PathBuilder::new();
        path.move_to(start.x, start.y);
        for (_, b) in &lines {
            let p = local(*b);
            path.line_to(p.x, p.y);
        }
        if lines.len() == 64 {
            path.close();
        }
        let path = path.finish().unwrap();
        let active = Some(axis) == highlight;
        let width = if active {
            3.4
        } else if axis == FREE {
            1.4
        } else {
            2.6
        };
        let color = if active {
            HIGHLIGHT
        } else if axis == FREE {
            Color::srgb(0.86, 0.88, 0.91)
        } else {
            ROTATION_COLORS[axis]
        };
        let alpha = if active {
            1.0
        } else if dragging {
            0.32
        } else if axis == FREE {
            0.78
        } else {
            1.0
        };
        for (width, color) in [
            (width + 1.8, Color::srgba(0.04, 0.05, 0.07, alpha * 0.8)),
            (width, color.with_alpha(alpha)),
        ] {
            let c = color.to_srgba().to_u8_array();
            let mut paint = sk::Paint::default();
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
            let stroke = sk::Stroke {
                width,
                line_join: sk::LineJoin::Round,
                line_cap: sk::LineCap::Round,
                ..Default::default()
            };
            pixmap.stroke_path(&path, &paint, &stroke, scale, None);
        }
    }
    let mut rgba = Vec::with_capacity(pixmap.data().len());
    for p in pixmap.pixels() {
        let p = p.demultiply();
        rgba.extend_from_slice(&[p.red(), p.green(), p.blue(), p.alpha()]);
    }
    let size = UVec2::splat(side);
    let mut image = image_previews::texture_image(DecodedImage {
        size,
        original: size,
        rgba,
    });
    image.sampler = ImageSampler::linear();
    image
}

pub(super) struct Overlay {
    sprite: Entity,
    labels: [Entity; 3],
    texture: Option<Handle<Image>>,
    appearance: Option<(Quat, Option<usize>, bool, u32)>,
    center: Option<Vec2>,
}

impl Overlay {
    pub(super) fn setup(commands: &mut Commands, canvas: Entity, font: &Handle<Font>) -> Self {
        let hidden = Node {
            position_type: PositionType::Absolute,
            display: Display::None,
            ..default()
        };
        let sprite = commands
            .spawn((
                Node {
                    width: px(SIDE),
                    height: px(SIDE),
                    ..hidden.clone()
                },
                Pickable::IGNORE,
                ZIndex(12),
                ChildOf(canvas),
            ))
            .id();
        let labels = std::array::from_fn(|axis| {
            let label = commands
                .spawn((
                    Node {
                        width: px(LABEL_RADIUS * 2.0),
                        height: px(LABEL_RADIUS * 2.0),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border_radius: BorderRadius::MAX,
                        border: UiRect::all(px(1.5)),
                        ..hidden.clone()
                    },
                    BackgroundColor(Color::srgb(0.09, 0.10, 0.13)),
                    BorderColor::all(ROTATION_COLORS[axis]),
                    Pickable::IGNORE,
                    ZIndex(13),
                    ChildOf(canvas),
                ))
                .id();
            text(
                commands,
                label,
                font,
                ["X", "Y", "Z"][axis],
                12.0,
                Color::WHITE,
            );
            label
        });
        Self {
            sprite,
            labels,
            texture: None,
            appearance: None,
            center: None,
        }
    }

    pub(super) fn refresh(
        &mut self,
        frame: Option<&Frame>,
        viewport_min: Vec2,
        highlight: Option<usize>,
        dragging: bool,
        density: f32,
        images: &mut Assets<Image>,
        nodes: &mut Query<&mut Node>,
        commands: &mut Commands,
    ) {
        let Some(frame) = frame else {
            if self.center.take().is_some() {
                for id in std::iter::once(self.sprite).chain(self.labels) {
                    if let Ok(mut node) = nodes.get_mut(id) {
                        node.display = Display::None;
                    }
                }
            }
            return;
        };
        let side = (SIDE * density.clamp(1.0, 4.0)).ceil() as u32;
        let appearance = (frame.view.rotation, highlight, dragging, side);
        let center = frame.center - viewport_min;
        if self.appearance == Some(appearance) && self.center == Some(center) {
            return;
        }
        if self.appearance != Some(appearance) {
            let bitmap = raster(frame, highlight, dragging, side);
            if let Some(mut image) = self.texture.as_ref().and_then(|h| images.get_mut(h)) {
                *image = bitmap;
            } else {
                let texture = images.add(bitmap);
                commands
                    .entity(self.sprite)
                    .insert(ImageNode::new(texture.clone()));
                self.texture = Some(texture);
            }
            for (axis, label) in self.labels.iter().enumerate() {
                commands
                    .entity(*label)
                    .insert(BorderColor::all(if Some(axis) == highlight {
                        HIGHLIGHT
                    } else {
                        ROTATION_COLORS[axis]
                    }));
            }
            self.appearance = Some(appearance);
        }
        self.center = Some(center);
        for (id, position) in std::iter::once((self.sprite, center - Vec2::splat(SIDE * 0.5)))
            .chain(self.labels.into_iter().enumerate().map(|(axis, id)| {
                (
                    id,
                    center + frame.project(label_radial(frame, axis)) * RADIUS
                        - Vec2::splat(LABEL_RADIUS),
                )
            }))
        {
            if let Ok(mut node) = nodes.get_mut(id) {
                node.display = Display::Flex;
                node.left = px(position.x);
                node.top = px(position.y);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_labels_pick_their_own_visible_arc_in_axial_and_oblique_views() {
        for eye in [
            Vec3::X,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::ONE,
            Vec3::new(-1., 1., -1.),
            Vec3::new(1., 0., 1.),
        ] {
            let model = ModelReference {
                asset: String::new(),
                uid: None,
                direction: eye.to_array(),
                center: [200., 150.],
                pending_frame: None,
                pixels_per_unit: 1.,
                pose: Transform::IDENTITY.into(),
                joints: Default::default(),
            };
            let frame = Frame::new(model, Rect::new(0., 0., 400., 300.), UVec2::new(400, 300));
            for axis in 0..3 {
                let anchor = frame.center + frame.project(label_radial(&frame, axis)) * RADIUS;
                let arc = segments(&frame, axis);
                assert!(
                    arc.iter()
                        .any(|(a, b)| anchor.distance((*a + *b) * 0.5) < 4.0)
                );
                for offset in [Vec2::ZERO, Vec2::X * 7., Vec2::NEG_Y * 7.] {
                    let (picked, tangent, sensitivity) =
                        frame.hit(Tool::Rotate, anchor + offset).unwrap();
                    assert_eq!(picked, axis);
                    assert!(tangent.is_normalized() && sensitivity.is_finite());
                }
            }
            assert_eq!(
                frame
                    .hit(Tool::Rotate, frame.center + Vec2::X * FREE_RADIUS)
                    .unwrap()
                    .0,
                FREE
            );
        }
    }
}
