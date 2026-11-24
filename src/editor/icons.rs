//! Embedded SVG artwork, rasterized once per display-size bucket for Bevy UI.
use std::collections::HashMap;

use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    ui::{InteractionDisabled, UiSystems},
};

use super::{
    Action, Control, DISABLED_TEXT, Editor, FOCUS, ImageViewer, Look, MUTED, ModelEditingState,
    ON_ACCENT, TEXT, control, row, scene_tabs,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Glyph {
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    SubmenuDown,
    ChevronUp,
    ArrowUp,
    LayerClipping,
    LayerBlend,
    Search,
    Flag,
    Hash,
    Close,
    Minimize,
    Maximize,
    Restore,
    Folder,
    FolderOpen,
    File,
    CharacterFile,
    DialogueFile,
    Image,
    Trash,
    Layers,
    Star,
    Plus,
    Cursor,
    Move,
    Rotate,
    Scale,
    Extrude,
    Camera,
    Box,
    Sun,
    Lightbulb,
    Spotlight,
    Rectangle,
    Globe,
    Refresh,
    Magnet,
    Eye,
    EyeOff,
    Brush,
    Pencil,
    Eraser,
    Pipette,
    PaintBucket,
    Undo,
    Redo,
    Selection,
    SelectionEllipse,
    Lasso,
    Polygon,
    Wand,
    SelectVertex,
    SelectEdge,
    SelectFace,
    Xray,
    PrimitivePlane,
    PrimitiveCircle,
    PrimitiveUvSphere,
    PrimitiveIcoSphere,
    PrimitiveCylinder,
    PrimitiveCone,
    PrimitiveTorus,
    Copy,
    Lock,
    Mask,
    Check,
    Play,
    Stop,
}

impl Glyph {
    fn svg(self) -> &'static str {
        match self {
            Self::CharacterFile => include_str!("../../assets/icons/asset-character.svg"),
            Self::DialogueFile => include_str!("../../assets/icons/asset-dialogue.svg"),
            Self::Play => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M6 3.5a1 1 0 0 1 1.5-.86l14 8.5a1 1 0 0 1 0 1.72l-14 8.5A1 1 0 0 1 6 20.5Z"/></svg>"#
            }
            Self::Stop => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><rect x="4" y="4" width="16" height="16" rx="2"/></svg>"#
            }
            Self::Copy => include_str!("../../assets/icons/lucide/copy.svg"),
            Self::Lock => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="10" width="16" height="11" rx="2"/><path d="M8 10V6a4 4 0 0 1 8 0v4M12 14v3"/></svg>"#
            }
            Self::Mask => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="12" cy="12" r="4"/></svg>"#
            }
            Self::Check => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="3"/><path d="m7 12 3 3 7-7"/></svg>"#
            }
            Self::ChevronLeft => include_str!("../../assets/icons/lucide/chevron-left.svg"),
            Self::ChevronRight => include_str!("../../assets/icons/lucide/chevron-right.svg"),
            Self::ChevronDown => include_str!("../../assets/icons/lucide/chevron-down.svg"),
            Self::SubmenuDown => include_str!("../../assets/icons/submenu-down.svg"),
            Self::ChevronUp => include_str!("../../assets/icons/lucide/chevron-up.svg"),
            Self::ArrowUp => include_str!("../../assets/icons/lucide/arrow-up.svg"),
            Self::LayerBlend => include_str!("../../assets/icons/layer-blend.svg"),
            Self::LayerClipping => include_str!("../../assets/icons/layer-clipping.svg"),
            Self::Search => include_str!("../../assets/icons/lucide/search.svg"),
            Self::Flag => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M5 21V4c5-4 9 4 15 0v10c-6 4-10-4-15 0"/></svg>"#
            }
            Self::Hash => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="m10 3-3 18M17 3l-3 18M4 9h16M3 15h16"/></svg>"#
            }
            Self::Close => include_str!("../../assets/icons/lucide/x.svg"),
            Self::Minimize => include_str!("../../assets/icons/lucide/minus.svg"),
            Self::Maximize => include_str!("../../assets/icons/lucide/square.svg"),
            Self::Restore => include_str!("../../assets/icons/lucide/copy.svg"),
            Self::Folder => include_str!("../../assets/icons/lucide/folder.svg"),
            Self::FolderOpen => include_str!("../../assets/icons/lucide/folder-open.svg"),
            Self::File => include_str!("../../assets/icons/lucide/file.svg"),
            Self::Image => include_str!("../../assets/icons/lucide/image.svg"),
            Self::Trash => include_str!("../../assets/icons/lucide/trash-2.svg"),
            Self::Layers => include_str!("../../assets/icons/lucide/layers.svg"),
            Self::Star => include_str!("../../assets/icons/lucide/star.svg"),
            Self::Plus => include_str!("../../assets/icons/lucide/plus.svg"),
            Self::Cursor => include_str!("../../assets/icons/lucide/mouse-pointer.svg"),
            Self::Move => include_str!("../../assets/icons/lucide/move.svg"),
            Self::Rotate => include_str!("../../assets/icons/lucide/rotate-cw.svg"),
            Self::Scale => include_str!("../../assets/icons/lucide/scaling.svg"),
            Self::Extrude => include_str!("../../assets/icons/extrude.svg"),
            Self::Camera => include_str!("../../assets/icons/lucide/camera.svg"),
            Self::Box => include_str!("../../assets/icons/lucide/box.svg"),
            Self::Sun => include_str!("../../assets/icons/lucide/sun.svg"),
            Self::Lightbulb => include_str!("../../assets/icons/lucide/lightbulb.svg"),
            Self::Spotlight => include_str!("../../assets/icons/lucide/flashlight.svg"),
            Self::Rectangle => include_str!("../../assets/icons/lucide/rectangle-horizontal.svg"),
            Self::Globe => include_str!("../../assets/icons/lucide/globe.svg"),
            Self::Refresh => include_str!("../../assets/icons/lucide/refresh-cw.svg"),
            Self::Magnet => include_str!("../../assets/icons/lucide/magnet.svg"),
            Self::Eye => include_str!("../../assets/icons/lucide/eye.svg"),
            Self::EyeOff => include_str!("../../assets/icons/lucide/eye-off.svg"),
            Self::Brush => include_str!("../../assets/icons/lucide/brush.svg"),
            Self::Pencil => include_str!("../../assets/icons/lucide/pencil.svg"),
            Self::Eraser => include_str!("../../assets/icons/lucide/eraser.svg"),
            Self::Pipette => include_str!("../../assets/icons/lucide/pipette.svg"),
            Self::PaintBucket => include_str!("../../assets/icons/lucide/paint-bucket.svg"),
            Self::Undo => include_str!("../../assets/icons/lucide/undo-2.svg"),
            Self::Redo => include_str!("../../assets/icons/lucide/redo-2.svg"),
            Self::Selection => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2"><rect x="4" y="4" width="16" height="16" rx="1" stroke-dasharray="3 2"/></svg>"#
            }
            Self::SelectionEllipse => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2"><ellipse cx="12" cy="12" rx="9" ry="7" stroke-dasharray="3 2"/></svg>"#
            }
            Self::Lasso => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><path d="M7 17C-1 12 4 4 12 4s12 9 5 12-9-1-10 2 4 5 7 2"/><ellipse cx="7" cy="17" rx="2" ry="2"/></svg>"#
            }
            Self::Polygon => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2"><path d="m4 17 2-12 14 3-5 12Z" stroke-dasharray="3 2"/><path d="M3 15h4v4H3zM4 3h4v4H4zM18 6h4v4h-4z"/></svg>"#
            }
            Self::Wand => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><path d="m4 20 12-12 3 3L7 23Zm8-8 3 3M6 3v4M4 5h4M17 1v4M15 3h4M21 16v4M19 18h4"/></svg>"#
            }
            Self::SelectVertex => include_str!("../../assets/icons/select-vertex.svg"),
            Self::SelectEdge => include_str!("../../assets/icons/select-edge.svg"),
            Self::SelectFace => include_str!("../../assets/icons/select-face.svg"),
            Self::Xray => include_str!("../../assets/icons/x-ray.svg"),
            Self::PrimitivePlane => include_str!("../../assets/icons/primitive-plane.svg"),
            Self::PrimitiveCircle => include_str!("../../assets/icons/primitive-circle.svg"),
            Self::PrimitiveUvSphere => include_str!("../../assets/icons/primitive-uv-sphere.svg"),
            Self::PrimitiveIcoSphere => include_str!("../../assets/icons/primitive-ico-sphere.svg"),
            Self::PrimitiveCylinder => include_str!("../../assets/icons/primitive-cylinder.svg"),
            Self::PrimitiveCone => include_str!("../../assets/icons/primitive-cone.svg"),
            Self::PrimitiveTorus => include_str!("../../assets/icons/primitive-torus.svg"),
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

pub(crate) struct SvgIconsPlugin;

impl Plugin for SvgIconsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Library>()
            .add_systems(PostUpdate, sync.after(UiSystems::Layout));
    }
}

pub(crate) fn spawn(
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

pub(crate) fn set_glyph(commands: &mut Commands, entity: Entity, glyph: Glyph) {
    commands
        .entity(entity)
        .entry::<SvgIcon>()
        .and_modify(move |mut icon| {
            icon.glyph = glyph;
        });
}

pub(crate) fn set_tint(commands: &mut Commands, entity: Entity, color: Color) {
    commands
        .entity(entity)
        .entry::<SvgIcon>()
        .and_modify(move |mut icon| {
            icon.tint = Tint::Fixed(color);
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

fn tint(
    icon: &SvgIcon,
    owner: Option<(&Control, bool, bool)>,
    editor: &Editor,
    image_viewer: Option<&ImageViewer>,
    model_edit: Option<&ModelEditingState>,
) -> Color {
    if owner.is_some_and(|(_, _, disabled)| disabled) {
        return DISABLED_TEXT;
    }
    if let Tint::Fixed(color) = icon.tint {
        return color;
    }
    let Some((control, hovered, _)) = owner else {
        return TEXT;
    };
    if let Action::ImageEdit(
        action @ super::ImageEditAction::Layers(
            super::image_viewer::edit::layers::LayerAction::ToggleLock(_),
        ),
    ) = control.action
    {
        return if image_viewer
            .is_some_and(|viewer| viewer.edit.active(action, editor.viewing_image))
        {
            FOCUS
        } else {
            TEXT
        };
    }
    match control.look {
        Look::Tool => {
            let active = match control.action {
                Action::Tool(tool) => {
                    if editor.viewing_model.is_some() && editor.viewing_image.is_none() {
                        model_edit.is_some_and(|state| state.tool == tool)
                    } else {
                        tool == editor.tool
                    }
                }
                Action::ImageEdit(action) => image_viewer
                    .is_some_and(|viewer| viewer.edit.active(action, editor.viewing_image)),
                _ => false,
            };
            if active {
                if matches!(control.action, Action::ImageEdit(_)) {
                    FOCUS
                } else {
                    ON_ACCENT
                }
            } else if hovered {
                TEXT
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
    editor: Option<Res<Editor>>,
    image_viewer: Option<Res<ImageViewer>>,
    model_edit: Option<Res<ModelEditingState>>,
    parents: Query<&ChildOf>,
    controls: Query<(&Control, &super::Hovered, Has<InteractionDisabled>)>,
    disabled: Query<(), With<InteractionDisabled>>,
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
        let color = if parents
            .iter_ancestors(entity)
            .any(|parent| disabled.contains(parent))
        {
            DISABLED_TEXT
        } else if let Some(editor) = editor.as_deref() {
            tint(
                icon,
                owner,
                editor,
                image_viewer.as_deref(),
                model_edit.as_deref(),
            )
        } else if let Tint::Fixed(color) = icon.tint {
            color
        } else {
            TEXT
        };
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
    // File illustrations keep their authored palette. Toolbar glyphs remain
    // white alpha masks so the existing theme and interaction tints apply.
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for pixel in pixmap.pixels() {
        if matches!(glyph, Glyph::CharacterFile | Glyph::DialogueFile) {
            let pixel = pixel.demultiply();
            rgba.extend_from_slice(&[pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]);
        } else {
            rgba.extend_from_slice(&[255, 255, 255, pixel.alpha()]);
        }
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
            Glyph::SubmenuDown,
            Glyph::ChevronUp,
            Glyph::ArrowUp,
            Glyph::LayerClipping,
            Glyph::Search,
            Glyph::Close,
            Glyph::Minimize,
            Glyph::Maximize,
            Glyph::Restore,
            Glyph::Folder,
            Glyph::FolderOpen,
            Glyph::File,
            Glyph::Image,
            Glyph::Trash,
            Glyph::Undo,
            Glyph::Layers,
            Glyph::Star,
            Glyph::Plus,
            Glyph::Cursor,
            Glyph::Move,
            Glyph::Rotate,
            Glyph::Scale,
            Glyph::Extrude,
            Glyph::Camera,
            Glyph::Box,
            Glyph::Sun,
            Glyph::Lightbulb,
            Glyph::Spotlight,
            Glyph::Rectangle,
            Glyph::Globe,
            Glyph::Refresh,
            Glyph::Magnet,
            Glyph::Eye,
            Glyph::EyeOff,
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
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Editor::new(project, crate::LevelEdits::default()))
            .init_resource::<Assets<Image>>()
            .init_resource::<Library>()
            .add_systems(Update, sync);
        app.world_mut().resource_mut::<Editor>().tool = super::super::Tool::Move;
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

        // Model tools are independent of the scene tool, just like their button backgrounds.
        app.init_resource::<ModelEditingState>();
        app.world_mut().resource_mut::<ModelEditingState>().tool = super::super::Tool::Move;
        {
            let mut editor = app.world_mut().resource_mut::<Editor>();
            editor.tool = super::super::Tool::Universal;
            editor.viewing_model = Some(0);
        }
        app.update();
        assert_eq!(app.world().get::<ImageNode>(icon).unwrap().color, ON_ACCENT);
        app.world_mut().get_mut::<Control>(owner).unwrap().action =
            Action::Tool(super::super::Tool::Universal);
        app.update();
        assert_eq!(app.world().get::<ImageNode>(icon).unwrap().color, MUTED);
        app.world_mut().resource_mut::<Editor>().viewing_model = None;
        app.update();
        assert_eq!(app.world().get::<ImageNode>(icon).unwrap().color, ON_ACCENT);

        // Image tools use their own selection state, independently of the 3D tool.
        use super::super::ImageEditAction;
        use crate::editor::photo_document::PhotoTool;
        app.init_resource::<ImageViewer>();
        app.world_mut().get_mut::<Control>(owner).unwrap().action =
            Action::ImageEdit(ImageEditAction::DrawingTool);
        app.update();
        assert_eq!(app.world().get::<ImageNode>(icon).unwrap().color, FOCUS);
        app.world_mut().resource_mut::<ImageViewer>().edit.tool = PhotoTool::Eraser;
        app.update();
        assert_eq!(app.world().get::<ImageNode>(icon).unwrap().color, MUTED);
        app.world_mut().get_mut::<Control>(owner).unwrap().action =
            Action::ImageEdit(ImageEditAction::Tool(PhotoTool::Eraser));
        app.update();
        assert_eq!(app.world().get::<ImageNode>(icon).unwrap().color, FOCUS);
        app.world_mut().get_mut::<Control>(owner).unwrap().action =
            Action::ImageEdit(ImageEditAction::PaintMenu);
        app.world_mut()
            .resource_mut::<ImageViewer>()
            .edit
            .paint_menu = true;
        app.update();
        assert_eq!(app.world().get::<ImageNode>(icon).unwrap().color, FOCUS);
        assert_eq!(app.world().resource::<Assets<Image>>().len(), 2);
    }
}
