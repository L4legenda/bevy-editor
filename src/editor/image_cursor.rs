//! High-contrast painting cursors, carried by the OS so strokes cannot delay pointer feedback.
use super::*;
use crate::editor::photo_document::PhotoTool;
use bevy::window::{CustomCursor, CustomCursorImage, RawHandleWrapper, SystemCursorIcon};
use std::collections::VecDeque;

#[path = "image_pencil_preview.rs"]
pub(super) mod pencil;
#[path = "image_pipette_cursor.rs"]
mod pipette;
#[path = "image_pipette_preview.rs"]
mod preview;
pub(super) use preview::Sample as PipetteSample;

// Stay within common hardware-cursor limits. Larger tips retain their full UI outline
// with a transparent native cursor so no second marker appears inside the outline.
const MAX_SIDE: u32 = 255;
const CACHE_SIZE: usize = 8;
const CURSOR_NAMESPACE: u128 = 0x9cbd_805d_4fe3_4e62_927a_f261_0000_0000;
const DARK: Color = Color::srgba(0.0, 0.0, 0.0, 160.0 / 255.0);
const LIGHT: Color = Color::srgba(1.0, 1.0, 1.0, 235.0 / 255.0);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Tip {
    // Physical pixels, matching the displayed document at its current zoom.
    diameter: f32,
    square: bool,
    density: f32,
    hidden: bool,
}

impl Tip {
    fn new(diameter: f32, square: bool, density: f32) -> Self {
        let density = if density.is_finite() {
            density.clamp(0.5, 8.0)
        } else {
            1.0
        };
        Self {
            diameter: if diameter.is_finite() {
                diameter.max(0.0)
            } else {
                0.0
            },
            square,
            density,
            hidden: false,
        }
    }

    fn outline(self) -> bool {
        self.diameter >= 4.0 * self.density
    }

    fn side(self) -> f32 {
        if self.hidden {
            return 1.0;
        }
        let reach = if self.outline() {
            self.diameter * 0.5
        } else {
            0.0
        };
        (reach.max(4.0 * self.density) + 1.5 * self.density + 1.0).ceil() * 2.0 + 1.0
    }

    fn distance(self, p: Vec2) -> f32 {
        if self.hidden {
            return f32::INFINITY;
        }
        let p = p.abs();
        let arm = 4.0 * self.density;
        let cross = Vec2::new((p.x - arm).max(0.0), p.y)
            .length()
            .min(Vec2::new(p.x, (p.y - arm).max(0.0)).length());
        if !self.outline() {
            return cross;
        }
        let radius = self.diameter * 0.5;
        if self.square {
            let q = p - Vec2::splat(radius);
            (q.max(Vec2::ZERO).length() + q.max_element().min(0.0)).abs()
        } else {
            (p.length() - radius).abs()
        }
    }

    fn rasterize(self) -> (Image, u16) {
        let side = self.side() as u32;
        let hot = side / 2;
        let mut rgba = Vec::with_capacity((side * side * 4) as usize);
        for y in 0..side {
            for x in 0..side {
                let distance =
                    self.distance(Vec2::new(x as f32 - hot as f32, y as f32 - hot as f32));
                let light = (0.5 * self.density + 0.5 - distance).clamp(0.0, 1.0) * (235.0 / 255.0);
                let dark = (1.5 * self.density + 0.5 - distance).clamp(0.0, 1.0) * (160.0 / 255.0);
                let alpha = light + dark * (1.0 - light);
                let value = if alpha > 0.0 {
                    (light / alpha * 255.0).round() as u8
                } else {
                    0
                };
                rgba.extend_from_slice(&[value, value, value, (alpha * 255.0).round() as u8]);
            }
        }
        (
            Image::new(
                Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::MAIN_WORLD,
            ),
            hot as u16,
        )
    }
}

#[derive(Default)]
pub(in crate::editor) struct Cache {
    tips: VecDeque<(Tip, CustomCursorImage)>,
    pipettes: VecDeque<(u16, CustomCursorImage)>,
}

impl Cache {
    #[cfg(test)]
    fn cursor(&mut self, tip: Tip, images: &mut Assets<Image>) -> (CursorIcon, bool) {
        self.cursor_with_overlay(tip, images, false)
    }
    fn cursor_with_overlay(
        &mut self,
        tip: Tip,
        images: &mut Assets<Image>,
        overlay: bool,
    ) -> (CursorIcon, bool) {
        // Pixel-sized changes matter; subpixel slider motion must not allocate new cursors.
        let mut tip = Tip::new(
            tip.diameter.round(),
            tip.square,
            (tip.density * 64.0).round() / 64.0,
        );
        let full = !overlay && tip.side() <= MAX_SIDE as f32;
        if !full {
            tip = Tip {
                diameter: 0.0,
                square: false,
                density: 1.0,
                hidden: true,
            };
        } else if !tip.outline() {
            tip = Tip::new(0.0, false, tip.density);
        }
        if let Some((_, image)) = self.tips.iter().find(|(key, _)| *key == tip) {
            return (CursorIcon::Custom(CustomCursor::Image(image.clone())), full);
        }
        if self.tips.len() >= CACHE_SIZE
            && let Some((_, old)) = self.tips.pop_front()
        {
            images.remove(old.handle.id());
        }
        let (image, hotspot) = tip.rasterize();
        // Winit caches native objects by asset ID. Deterministic IDs reuse those objects
        // even after the bounded CPU image cache evicts a size and later recreates it.
        let key = ((tip.hidden as u128) << 19)
            | ((tip.square as u128) << 18)
            | ((tip.diameter as u128) << 10)
            | (tip.density * 64.0) as u128;
        let handle = Handle::Uuid(
            bevy::asset::uuid::Uuid::from_u128(CURSOR_NAMESPACE | key),
            default(),
        );
        // Inserting a UUID asset cannot fail with an invalid generation.
        let _ = images.insert(handle.id(), image);
        let image = CustomCursorImage {
            handle,
            hotspot: (hotspot, hotspot),
            ..default()
        };
        self.tips.push_back((tip, image.clone()));
        (CursorIcon::Custom(CustomCursor::Image(image)), full)
    }

    fn pipette(&mut self, density: f32, images: &mut Assets<Image>) -> CursorIcon {
        let key = (Tip::new(0.0, false, density).density * 64.0).round() as u16;
        if let Some((_, cursor)) = self.pipettes.iter().find(|(k, _)| *k == key) {
            return CursorIcon::Custom(CustomCursor::Image(cursor.clone()));
        }
        let (image, hotspot) = match pipette::image(key as f32 / 64.0) {
            Ok(result) => result,
            Err(error) => {
                warn!("Eyedropper cursor: {error}");
                return SystemCursorIcon::Crosshair.into();
            }
        };
        if self.pipettes.len() >= CACHE_SIZE
            && let Some((_, old)) = self.pipettes.pop_front()
        {
            images.remove(old.handle.id());
        }
        let handle = Handle::Uuid(
            bevy::asset::uuid::Uuid::from_u128(CURSOR_NAMESPACE | (1 << 24) | u128::from(key)),
            default(),
        );
        let _ = images.insert(handle.id(), image);
        let cursor = CustomCursorImage {
            handle,
            hotspot,
            ..default()
        };
        self.pipettes.push_back((key, cursor.clone()));
        CursorIcon::Custom(CustomCursor::Image(cursor))
    }
}

// Other cursor owners can retain an old bitmap after the bounded cache evicts it.
// Recognise the reserved asset namespace rather than just the current cache entries.
fn is_tool_cursor(cursor: &CursorIcon) -> bool {
    matches!(cursor, CursorIcon::Custom(CustomCursor::Image(image))
        if matches!(image.handle.id(), bevy::asset::AssetId::Uuid { uuid }
            if uuid.as_u128() >> 32 == CURSOR_NAMESPACE >> 32))
}

fn is_last_tool_cursor(viewer: &ImageViewer, cursor: &CursorIcon) -> bool {
    is_tool_cursor(cursor) || viewer.last_cursor.as_ref() == Some(cursor)
}

fn is_resize_cursor(cursor: &CursorIcon) -> bool {
    matches!(
        cursor,
        CursorIcon::System(
            SystemCursorIcon::EwResize
                | SystemCursorIcon::NsResize
                | SystemCursorIcon::NeswResize
                | SystemCursorIcon::NwseResize
        )
    )
}

/// Chrome restores its cursor in PostUpdate. Clear stale tool icons after every
/// owner has finished, before Winit applies the final native cursor in Last.
pub(in crate::editor) fn cleanup(
    mut commands: Commands,
    viewer: Res<ImageViewer>,
    windows: Query<(Entity, &CursorIcon), With<PrimaryWindow>>,
) {
    if !viewer.cursor_owned {
        for (entity, cursor) in &windows {
            if is_last_tool_cursor(&viewer, cursor) {
                commands
                    .entity(entity)
                    .insert(viewer.previous_cursor.clone().unwrap_or_default());
            }
        }
    }
}

pub(in crate::editor) fn cursor(
    mut commands: Commands,
    editor: Res<Editor>,
    mut viewer: ResMut<ImageViewer>,
    window: Single<
        (Entity, &Window, Option<&CursorIcon>, Has<RawHandleWrapper>),
        With<PrimaryWindow>,
    >,
    (canvas, controls): (
        Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
        Query<(&ComputedNode, &UiGlobalTransform), With<CanvasControl>>,
    ),
    drawer: Option<Res<DrawerMotion>>,
    layer_drag: Option<Res<crate::editor::ui::LayerDrag>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut images: ResMut<Assets<Image>>,
    mut cache: Local<Cache>,
    mut sampling: Local<preview::Sampling>,
) {
    let (entity, window, current, native_window) = *window;
    let rect = canvas_rect(&canvas, window);
    let input_rect = drawer
        .as_ref()
        .map_or(rect, |drawer| drawer.input_rect(rect, window.height()));
    let editing = viewer.edit.editing(editor.viewing_image);
    let resize_center = viewer.edit.size_adjust.center();
    let active = editor.viewing_image.is_some()
        && !layer_drag.as_ref().is_some_and(|drag| drag.active)
        && editor.pending.is_none()
        && !editor.browser_popup
        && !editor.hdri_library_open
        && window.focused
        && viewer.error.is_none()
        && !viewer.edit.menu
        && !viewer.edit.paint_menu
        && !viewer.edit.size_menu
        && viewer.edit.selection.menu == 0
        && viewer.edit.color_picker.is_none()
        && !viewer.edit.layers.menu
        && viewer.edit.layers.form.is_none()
        && !viewer.edit.busy()
        && !viewer.panel.blocks_canvas()
        && viewer.dimensions.is_some()
        && resize_center
            .or_else(|| window.cursor_position())
            .is_some_and(|point| input_rect.contains(point) && !over_controls(&controls, point));
    viewer.brush_cursor = None;
    viewer.brush_cursor_position = None;
    viewer.native_brush_cursor = false;
    viewer.pipette_preview = None;
    let desired = active.then(|| {
        let hand = !editing
            || viewer.edit.tool == PhotoTool::Hand
            || keys.pressed(KeyCode::Space)
            || mouse.pressed(MouseButton::Middle);
        if hand {
            return CursorIcon::from(if viewer.gesture.is_some() {
                SystemCursorIcon::Grabbing
            } else {
                SystemCursorIcon::Grab
            });
        }
        if viewer.edit.eyedropper(&keys, editor.viewing_image) {
            viewer.pipette_preview = window.cursor_position().and_then(|cursor| {
                let dimensions = viewer.dimensions?;
                let image = viewer.image_rect(dimensions, rect);
                let doc = viewer.edit.document(editor.viewing_image)?;
                sampling.sample(doc, cursor, image, input_rect)
            });
            return cache.pipette(window.scale_factor(), &mut images);
        }
        if matches!(viewer.edit.tool, PhotoTool::Select(_)) {
            return CursorIcon::from(SystemCursorIcon::Crosshair);
        }
        if matches!(viewer.edit.tool, PhotoTool::Move | PhotoTool::Transform) {
            let icon = viewer
                .edit
                .selection
                .transform
                .as_ref()
                .and_then(|t| {
                    let dims = viewer.dimensions?;
                    let image = viewer.image_rect(dims, rect);
                    let scale = image.width() / dims.x as f32;
                    let point = (window.cursor_position()? - image.min) / scale;
                    super::edit::selection::hit(t.quad, point, scale)
                })
                .map_or(SystemCursorIcon::Move, |hit| match hit {
                    0 | 4 => SystemCursorIcon::NwseResize,
                    2 | 6 => SystemCursorIcon::NeswResize,
                    1 | 5 => SystemCursorIcon::NsResize,
                    3 | 7 => SystemCursorIcon::EwResize,
                    8 => SystemCursorIcon::Crosshair,
                    _ => SystemCursorIcon::Move,
                });
            return CursorIcon::from(icon);
        }
        let diameter = if matches!(
            viewer.edit.tool,
            PhotoTool::Brush | PhotoTool::Pencil | PhotoTool::Eraser
        ) {
            viewer.dimensions.map_or(0.0, |dimensions| {
                viewer.edit.size
                    * viewer
                        .zoom
                        .scale(viewer.source_size(dimensions), rect.size())
            })
        } else {
            0.0
        };
        let tip = Tip::new(diameter, viewer.edit.pixel_square(), window.scale_factor());
        let pixel_tool = viewer.edit.pixel_tool();
        let position = resize_center
            .or_else(|| window.cursor_position())
            .map(|cursor| {
                if resize_center.is_none()
                    && pixel_tool
                    && let Some(dimensions) = viewer.dimensions
                {
                    let image = viewer.image_rect(dimensions, rect);
                    let per_pixel = image.size() / dimensions.as_vec2();
                    let point = (cursor - image.min) / per_pixel;
                    image.min + viewer.edit.snap_point(point) * per_pixel
                } else {
                    cursor
                }
            });
        if viewer.edit.tool == PhotoTool::Pencil
            && let (Some(position), Some(dimensions)) = (position, viewer.dimensions)
        {
            let image = viewer.image_rect(dimensions, rect);
            let half = image.size() / dimensions.as_vec2() * viewer.edit.size * 0.5;
            if (position + half).cmple(image.min).any() || (position - half).cmpge(image.max).any()
            {
                // There are no paintable cells to preview here. Keep a visible
                // pointer over the surrounding workspace instead of hiding it.
                return CursorIcon::from(SystemCursorIcon::Default);
            }
        }
        // Pixel tools show the cell they will touch, rather than an outline sliding inside it.
        let (icon, full) = cache.cursor_with_overlay(
            tip,
            &mut images,
            resize_center.is_some()
                || viewer.edit.tool == PhotoTool::Pencil
                || (pixel_tool && tip.outline()),
        );
        viewer.brush_cursor = Some(tip);
        viewer.brush_cursor_position = position;
        viewer.native_brush_cursor = full && native_window;
        icon
    });
    if let Some(desired) = desired {
        if !viewer.cursor_owned {
            if !current.is_some_and(|cursor| {
                is_last_tool_cursor(&viewer, cursor) || is_resize_cursor(cursor)
            }) {
                viewer.previous_cursor = current.cloned();
            }
            viewer.cursor_owned = true;
        }
        viewer.last_cursor = Some(desired.clone());
        if current != Some(&desired) {
            commands.entity(entity).insert(desired);
        }
    } else if viewer.cursor_owned {
        viewer.cursor_owned = false;
        // A splitter may already have taken over this frame. Keep its cursor;
        // cleanup will reject a stale brush bitmap if it is restored later.
        if current.is_some_and(|cursor| is_last_tool_cursor(&viewer, cursor)) {
            commands
                .entity(entity)
                .insert(viewer.previous_cursor.clone().unwrap_or_default());
        }
    }
}

/// The same contrasting tip for headless rendering and oversized hardware cursors.
pub(super) struct Overlay {
    root: Entity,
    rings: [Entity; 2],
    cross: Entity,
    readout: Entity,
    readout_text: Entity,
    pipette: preview::Preview,
}

impl Overlay {
    pub fn setup(commands: &mut Commands, canvas: Entity, font: &Handle<Font>) -> Self {
        let root = node(
            commands,
            canvas,
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                width: px(0),
                height: px(0),
                ..default()
            },
        );
        commands.entity(root).insert((ZIndex(8), Pickable::IGNORE));
        let rings = [(3.0, DARK), (1.0, LIGHT)].map(|(width, color)| {
            let ring = node(
                commands,
                root,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(0),
                    border: UiRect::all(px(width)),
                    ..default()
                },
            );
            commands.entity(ring).insert((
                BorderColor::all(color),
                Pickable::IGNORE,
                UiTransform::from_translation(Val2::percent(-50, -50)),
            ));
            ring
        });
        let cross = node(
            commands,
            root,
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: px(0),
                height: px(0),
                ..default()
            },
        );
        commands.entity(cross).insert(Pickable::IGNORE);
        for (width, color) in [(3.0, DARK), (1.0, LIGHT)] {
            for size in [Vec2::new(8.0 + width, width), Vec2::new(width, 8.0 + width)] {
                let arm = node(
                    commands,
                    cross,
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(0),
                        top: px(0),
                        width: px(size.x),
                        height: px(size.y),
                        ..default()
                    },
                );
                commands.entity(arm).insert((
                    BackgroundColor(color),
                    Pickable::IGNORE,
                    UiTransform::from_translation(Val2::percent(-50, -50)),
                ));
            }
        }
        let readout = surface(
            commands,
            root,
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                left: px(14),
                padding: UiRect::axes(px(6), px(3)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
                ..row(0.0)
            },
            PANEL,
        );
        commands.entity(readout).insert(Pickable::IGNORE);
        let readout_text = text(commands, readout, font, "", 11.0, TEXT);
        Self {
            root,
            rings,
            cross,
            readout,
            readout_text,
            pipette: preview::Preview::setup(commands, canvas),
        }
    }

    pub fn refresh_pipette(
        &self,
        nodes: &mut Query<&mut Node>,
        colors: &mut Query<&mut BackgroundColor>,
        sample: Option<PipetteSample>,
    ) {
        self.pipette.refresh(nodes, colors, sample);
    }

    pub fn refresh(
        &self,
        nodes: &mut Query<&mut Node>,
        labels: &mut Query<&mut Text>,
        tip: Option<Tip>,
        position: Option<Vec2>,
        resize: Option<f32>,
        pixels: bool,
    ) {
        let shown = tip.zip(position);
        if let Ok(mut readout) = nodes.get_mut(self.readout) {
            readout.display = if shown.is_some() && resize.is_some() {
                Display::Flex
            } else {
                Display::None
            };
            if let Some(tip) = tip {
                readout.top = px(tip.diameter / tip.density * 0.5 + 12.0);
            }
        }
        if let Some(size) = resize
            && let Ok(mut text) = labels.get_mut(self.readout_text)
        {
            text.set_if_neq(Text(if (size - size.round()).abs() < 0.05 {
                format!("{size:.0} px")
            } else {
                format!("{size:.1} px")
            }));
        }
        if let Ok(mut root) = nodes.get_mut(self.root) {
            root.display = if shown.is_some() {
                Display::Flex
            } else {
                Display::None
            };
            if let Some((_, position)) = shown {
                root.left = px(position.x);
                root.top = px(position.y);
            }
        }
        if let Some((tip, _)) = shown {
            if let Ok(mut cross) = nodes.get_mut(self.cross) {
                cross.display = if tip.outline() || pixels {
                    Display::None
                } else {
                    Display::Flex
                };
            }
            for (entity, stroke) in self.rings.into_iter().zip([3.0, 1.0]) {
                if let Ok(mut ring) = nodes.get_mut(entity) {
                    ring.display = if tip.outline() && !pixels {
                        Display::Flex
                    } else {
                        Display::None
                    };
                    ring.width = px(tip.diameter / tip.density + stroke);
                    ring.height = ring.width;
                    ring.border_radius = if tip.square {
                        BorderRadius::ZERO
                    } else {
                        BorderRadius::MAX
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eyedropper_cursor_has_a_tip_hotspot_contrast_and_reuses_dpi_images() {
        let mut cache = Cache::default();
        let mut images = Assets::<Image>::default();
        for density in [1.0, 1.5, 2.0] {
            let icon = cache.pipette(density, &mut images);
            assert!(is_tool_cursor(&icon));
            let cursor = custom(icon);
            assert_eq!(
                cursor.hotspot,
                ((4.0 * density) as u16, (24.0 * density) as u16)
            );
            let image = images.get(&cursor.handle).unwrap();
            assert_eq!(image.width(), (28.0 * density) as u32);
            let pixels = image.data.as_ref().unwrap();
            assert!(
                pixels.chunks_exact(4).any(|p| p[0] < 30 && p[3] > 200),
                "A dark outline is visible on white"
            );
            assert!(
                pixels.chunks_exact(4).any(|p| p[0] > 230 && p[3] > 200),
                "A light stroke is visible on black"
            );
            assert_eq!(pixels[3], 0);
            let hot = (usize::from(cursor.hotspot.1) * image.width() as usize
                + usize::from(cursor.hotspot.0))
                * 4;
            assert!(
                pixels[hot + 3] > 200,
                "The sampling point must be on the pipette tip"
            );
            assert_eq!(cursor, custom(cache.pipette(density, &mut images)));
        }
        assert_eq!(images.len(), 3);
    }

    fn custom(icon: CursorIcon) -> CustomCursorImage {
        let CursorIcon::Custom(CustomCursor::Image(image)) = icon else {
            panic!("Painting must use a contrasting bitmap instead of the OS inversion crosshair");
        };
        image
    }

    #[test]
    fn painting_cursor_contrasts_with_white_black_and_middle_grey() {
        for density in [1.0, 2.0] {
            for (diameter, square) in [(24.0, false), (24.0, true), (1.0, false)] {
                let (image, hot) = Tip::new(diameter, square, density).rasterize();
                let side = image.width();
                assert_eq!(side, u32::from(hot) * 2 + 1);
                let pixels = image.data.as_ref().unwrap();
                assert_eq!(
                    pixels[3], 0,
                    "The bitmap must not hide document pixels outside the tip"
                );
                let center_alpha =
                    pixels[((u32::from(hot) * side + u32::from(hot)) * 4 + 3) as usize];
                if diameter >= 4.0 * density {
                    assert_eq!(
                        center_alpha, 0,
                        "An outlined tip must have no crosshair inside"
                    );
                } else {
                    assert!(center_alpha > 200, "Tiny tips still need a visible marker");
                }
                for background in [0.0f32, 128.0, 255.0] {
                    let contrasting = pixels
                        .chunks_exact(4)
                        .filter(|pixel| {
                            let alpha = f32::from(pixel[3]) / 255.0;
                            let composed = f32::from(pixel[0]) * alpha + background * (1.0 - alpha);
                            (composed - background).abs() > 60.0
                        })
                        .count();
                    assert!(
                        contrasting >= 12,
                        "Invisible cursor on {background}: {diameter}, {square}, {density}"
                    );
                }
            }
        }
    }

    #[test]
    fn cursor_cache_reuses_images_and_bounds_size_and_count() {
        let mut cache = Cache::default();
        let mut images = Assets::<Image>::default();
        let tip = Tip::new(24.0, false, 1.0);
        let first = custom(cache.cursor(tip, &mut images).0);
        assert_eq!(first, custom(cache.cursor(tip, &mut images).0));
        assert_eq!(images.len(), 1);
        for diameter in 32..80 {
            let (icon, full) = cache.cursor(Tip::new(diameter as f32, false, 1.0), &mut images);
            let image = custom(icon);
            assert!(full);
            assert!(images.get(&image.handle).unwrap().width() <= MAX_SIDE);
            assert!(images.len() <= CACHE_SIZE);
        }
        for diameter in [8192.0, f32::MAX] {
            let (icon, full) = cache.cursor(Tip::new(diameter, false, 2.0), &mut images);
            assert!(!full, "Huge tips must use only the UI outline");
            let image = custom(icon);
            assert!(images.get(&image.handle).unwrap().width() < 32);
            assert!(
                images
                    .get(&image.handle)
                    .unwrap()
                    .data
                    .as_ref()
                    .unwrap()
                    .iter()
                    .all(|value| *value == 0)
            );
        }
        assert!(images.len() <= CACHE_SIZE);
        assert_eq!(
            first,
            custom(cache.cursor(tip, &mut images).0),
            "Revisiting a size must reuse Winit's native object"
        );
    }
}
