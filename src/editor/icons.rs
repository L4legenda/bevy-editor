//! Embedded SVG artwork, rasterized once per display-size bucket for Bevy UI.
use std::collections::HashMap;

use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    ui::{InteractionDisabled, UiSystems},
};

use super::{
    Action, Control, DISABLED_TEXT, Editor, Look, MUTED, ON_ACCENT, TEXT, control, row, scene_tabs,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Glyph {
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    ChevronUp,
    ArrowUp,
    Search,
    Close,
    Folder,
    FolderOpen,
    File,
    Layers,
    Plus,
    Move,
    Rotate,
    Scale,
    Camera,
    Box,
    Sun,
    Lightbulb,
    Spotlight,
    Rectangle,
    Globe,
    Refresh,
    Magnet,
}

impl Glyph {
    fn svg(self) -> &'static str {
        match self {
            Self::ChevronLeft => include_str!("../../assets/icons/lucide/chevron-left.svg"),
            Self::ChevronRight => include_str!("../../assets/icons/lucide/chevron-right.svg"),
            Self::ChevronDown => include_str!("../../assets/icons/lucide/chevron-down.svg"),
            Self::ChevronUp => include_str!("../../assets/icons/lucide/chevron-up.svg"),
            Self::ArrowUp => include_str!("../../assets/icons/lucide/arrow-up.svg"),
            Self::Search => include_str!("../../assets/icons/lucide/search.svg"),
            Self::Close => include_str!("../../assets/icons/lucide/x.svg"),
            Self::Folder => include_str!("../../assets/icons/lucide/folder.svg"),
            Self::FolderOpen => include_str!("../../assets/icons/lucide/folder-open.svg"),
            Self::File => include_str!("../../assets/icons/lucide/file.svg"),
            Self::Layers => include_str!("../../assets/icons/lucide/layers.svg"),
            Self::Plus => include_str!("../../assets/icons/lucide/plus.svg"),
            Self::Move => include_str!("../../assets/icons/lucide/move.svg"),
            Self::Rotate => include_str!("../../assets/icons/lucide/rotate-cw.svg"),
            Self::Scale => include_str!("../../assets/icons/lucide/scaling.svg"),
            Self::Camera => include_str!("../../assets/icons/lucide/camera.svg"),
            Self::Box => include_str!("../../assets/icons/lucide/box.svg"),
            Self::Sun => include_str!("../../assets/icons/lucide/sun.svg"),
            Self::Lightbulb => include_str!("../../assets/icons/lucide/lightbulb.svg"),
            Self::Spotlight => include_str!("../../assets/icons/lucide/flashlight.svg"),
            Self::Rectangle => include_str!("../../assets/icons/lucide/rectangle-horizontal.svg"),
            Self::Globe => include_str!("../../assets/icons/lucide/globe.svg"),
            Self::Refresh => include_str!("../../assets/icons/lucide/refresh-cw.svg"),
            Self::Magnet => include_str!("../../assets/icons/lucide/magnet.svg"),
        }
    }
}

#[derive(Clone, Copy)]
enum Tint {
    Fixed(Color),
    Control,
}

#[derive(Component)]
struct SvgIcon {
    glyph: Glyph,
    tint: Tint,
}

#[derive(Resource, Default)]
struct Library {
    images: HashMap<(Glyph, u32), Option<Handle<Image>>>,
}

impl Library {
    fn image(
        &mut self,
        glyph: Glyph,
        size: u32,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        self.images
            .entry((glyph, size))
            .or_insert_with(|| match rasterize(glyph, size) {
                Ok(image) => Some(images.add(image)),
                Err(error) => {
                    warn!("SVG icon {glyph:?}: {error}");
                    None
                }
            })
            .clone()
    }
}

pub(in crate::editor) struct SvgIconsPlugin;

impl Plugin for SvgIconsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Library>()
            .add_systems(PostUpdate, sync.after(UiSystems::Layout));
    }
}

pub(super) fn spawn(
    commands: &mut Commands,
    parent: Entity,
    glyph: Glyph,
    size: f32,
    color: Color,
) -> Entity {
    spawn_icon(commands, parent, glyph, size, Tint::Fixed(color))
}

pub(super) fn themed(commands: &mut Commands, parent: Entity, glyph: Glyph, size: f32) -> Entity {
    spawn_icon(commands, parent, glyph, size, Tint::Control)
}

pub(super) fn set_glyph(commands: &mut Commands, entity: Entity, glyph: Glyph) {
    commands
        .entity(entity)
        .entry::<SvgIcon>()
        .and_modify(move |mut icon| {
            icon.glyph = glyph;
        });
}

fn spawn_icon(
    commands: &mut Commands,
    parent: Entity,
    glyph: Glyph,
    size: f32,
    tint: Tint,
) -> Entity {
    commands
        .spawn((
            SvgIcon { glyph, tint },
            ImageNode {
                color: Color::NONE,
                ..default()
            },
            Node {
                width: px(size),
                height: px(size),
                flex_shrink: 0.0,
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}

pub(super) fn button(
    commands: &mut Commands,
    parent: Entity,
    glyph: Glyph,
    label: impl Into<String>,
    action: Action,
    look: Look,
) -> Entity {
    let button = control(
        commands,
        parent,
        action,
        look,
        Node {
            width: px(30),
            height: px(29),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..row(0.0)
        },
    );
    commands
        .entity(button)
        .insert(AccessibleLabel(label.into()));
    themed(commands, button, glyph, 16.0);
    button
}

fn resolution(size: Vec2) -> Option<u32> {
    (size.is_finite() && size.min_element() > 0.0)
        .then(|| (size.max_element().ceil().clamp(8.0, 256.0) as u32).next_power_of_two())
}

fn tint(icon: &SvgIcon, owner: Option<(&Control, bool, bool)>, editor: &Editor) -> Color {
    if owner.is_some_and(|(_, _, disabled)| disabled) {
        return DISABLED_TEXT;
    }
    if let Tint::Fixed(color) = icon.tint {
        return color;
    }
    let Some((control, hovered, _)) = owner else {
        return TEXT;
    };
    match control.look {
        Look::Tool => {
            if matches!(control.action, Action::Tool(tool) if tool == editor.tool) {
                ON_ACCENT
            } else {
                MUTED
            }
        }
        Look::Primary => ON_ACCENT,
        Look::Tab => {
            let active = matches!(control.action, Action::Open(room) if editor.has_scene() && editor.room == room);
            if active || hovered {
                scene_tabs::LABEL
            } else {
                scene_tabs::INACTIVE_LABEL
            }
        }
        _ => TEXT,
    }
}

fn sync(
    mut library: ResMut<Library>,
    mut images: ResMut<Assets<Image>>,
    editor: Res<Editor>,
    parents: Query<&ChildOf>,
    controls: Query<(&Control, &super::Hovered, Has<InteractionDisabled>)>,
    mut icons: Query<(Entity, &SvgIcon, &ComputedNode, &mut ImageNode)>,
) {
    for (entity, icon, computed, mut image) in &mut icons {
        let Some(size) = resolution(computed.size()) else {
            continue;
        };
        let Some(handle) = library.image(icon.glyph, size, &mut images) else {
            continue;
        };
        let owner = parents.iter_ancestors(entity).find_map(|parent| {
            controls
                .get(parent)
                .ok()
                .map(|(control, hovered, disabled)| (control, hovered.0, disabled))
        });
        let color = tint(icon, owner, &editor);
        if image.image != handle {
            image.image = handle;
        }
        if image.color != color {
            image.color = color;
        }
    }
}

fn rasterize(glyph: Glyph, size: u32) -> Result<Image, String> {
    let svg = glyph.svg().replace("currentColor", "#ffffff");
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
        .map_err(|error| error.to_string())?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).ok_or("Invalid SVG raster size")?;
    let scale = resvg::tiny_skia::Transform::from_scale(
        size as f32 / tree.size().width(),
        size as f32 / tree.size().height(),
    );
    resvg::render(&tree, scale, &mut pixmap.as_mut());
    // Monochrome SVGs become alpha masks for the theme. Transparent white
    // preserves clean edges when the GPU filters between covered and empty pixels.
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for pixel in pixmap.pixels() {
        rgba.extend_from_slice(&[255, 255, 255, pixel.alpha()]);
    }
    Ok(Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_svgs_render_transparent_white_artwork_at_both_display_scales() {
        for glyph in [
            Glyph::ChevronLeft,
            Glyph::ChevronRight,
            Glyph::ChevronDown,
            Glyph::ChevronUp,
            Glyph::ArrowUp,
            Glyph::Search,
            Glyph::Close,
            Glyph::Folder,
            Glyph::FolderOpen,
            Glyph::File,
            Glyph::Layers,
            Glyph::Plus,
            Glyph::Move,
            Glyph::Rotate,
            Glyph::Scale,
            Glyph::Camera,
            Glyph::Box,
            Glyph::Sun,
            Glyph::Lightbulb,
            Glyph::Spotlight,
            Glyph::Rectangle,
            Glyph::Globe,
            Glyph::Refresh,
            Glyph::Magnet,
        ] {
            for size in [16, 32] {
                let image = rasterize(glyph, size).unwrap();
                let pixels = image.data.as_ref().unwrap();
                assert_eq!(pixels.len(), (size * size * 4) as usize);
                assert!(
                    pixels.chunks_exact(4).any(|pixel| pixel[3] > 0),
                    "{glyph:?}"
                );
                assert!(
                    pixels.chunks_exact(4).any(|pixel| pixel[3] == 0),
                    "{glyph:?}"
                );
                for pixel in pixels.chunks_exact(4).filter(|pixel| pixel[3] > 0) {
                    assert!(
                        pixel[..3].iter().all(|&channel| channel >= 254),
                        "{glyph:?}: {pixel:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn raster_buckets_follow_physical_dpi_and_ignore_hidden_or_invalid_sizes() {
        assert_eq!(resolution(Vec2::splat(16.0)), Some(16));
        assert_eq!(resolution(Vec2::splat(32.0)), Some(32));
        assert_eq!(resolution(Vec2::splat(36.0)), Some(64));
        assert_eq!(resolution(Vec2::splat(1_000.0)), Some(256));
        assert_eq!(resolution(Vec2::ZERO), None);
        assert_eq!(resolution(Vec2::splat(f32::NAN)), None);
    }

    #[test]
    fn nested_button_icons_follow_disabled_and_tool_states_and_reuse_dpi_textures() {
        let project = crate::EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"),
        )
        .unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Editor::new(project, crate::LevelEdits::default()))
            .init_resource::<Assets<Image>>()
            .init_resource::<Library>()
            .add_systems(Update, sync);
        let owner = app
            .world_mut()
            .spawn((
                Control {
                    action: Action::Tool(super::super::Tool::Move),
                    look: Look::Tool,
                },
                super::super::Hovered::default(),
                Node::default(),
            ))
            .id();
        let middle = app
            .world_mut()
            .spawn((Node::default(), ChildOf(owner)))
            .id();
        let icon = themed(&mut app.world_mut().commands(), middle, Glyph::Move, 16.0);
        app.world_mut().flush();
        app.world_mut().get_mut::<ComputedNode>(icon).unwrap().size = Vec2::splat(32.0);
        app.update();
        let image = app.world().get::<ImageNode>(icon).unwrap();
        assert_eq!(image.color, ON_ACCENT);
        let original = image.image.clone();
        assert_eq!(app.world().resource::<Assets<Image>>().len(), 1);

        app.world_mut()
            .entity_mut(owner)
            .insert(InteractionDisabled);
        app.update();
        assert_eq!(
            app.world().get::<ImageNode>(icon).unwrap().color,
            DISABLED_TEXT
        );
        app.world_mut()
            .entity_mut(owner)
            .remove::<InteractionDisabled>();
        app.world_mut().resource_mut::<Editor>().tool = super::super::Tool::Rotate;
        app.update();
        assert_eq!(app.world().get::<ImageNode>(icon).unwrap().color, MUTED);

        app.world_mut().get_mut::<ComputedNode>(icon).unwrap().size = Vec2::splat(64.0);
        app.update();
        assert_ne!(app.world().get::<ImageNode>(icon).unwrap().image, original);
        app.world_mut().get_mut::<ComputedNode>(icon).unwrap().size = Vec2::splat(32.0);
        app.update();
        assert_eq!(app.world().get::<ImageNode>(icon).unwrap().image, original);
        assert_eq!(app.world().resource::<Assets<Image>>().len(), 2);
    }
}
