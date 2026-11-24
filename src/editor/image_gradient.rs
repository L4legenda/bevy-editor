//! Gradient gesture, tool options, and a cancellable composited preview.
use super::*;
use crate::editor::photo_document::{PhotoDocument, PhotoTool};
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

#[derive(Default)]
pub(super) struct Settings {
    pub radial: bool,
    pub transparent: bool,
    pub reverse: bool,
}

impl Settings {
    pub fn colors(&self, foreground: [f32; 4], mut background: [f32; 4]) -> [[f32; 4]; 2] {
        if self.transparent {
            background = [foreground[0], foreground[1], foreground[2], 0.0];
        }
        if self.reverse {
            [background, foreground]
        } else {
            [foreground, background]
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Draft {
    pub from: Vec2,
    pub to: Vec2,
    pub colors: [[f32; 4]; 2],
    pub radial: bool,
    pub min_length: f32,
    pub revision: u64,
    pub view_revision: u64,
    pub layer: Option<photocraft_doc::LayerId>,
    pub target: &'static str,
}

impl Draft {
    pub fn valid_for(self, doc: &PhotoDocument) -> bool {
        self.from.distance(self.to) >= self.min_length
            && self.revision == doc.revision
            && self.view_revision == doc.view_revision
            && self.layer == doc.active_id()
            && self.target == doc.paint_target()
            && doc.paintable()
    }

    pub fn params(self) -> serde_json::Value {
        // The raster engine evaluates index - 0.5. Gesture coordinates measure
        // from the document edge, so shift the endpoints by one texel.
        serde_json::json!({
            "from": (self.from - Vec2::ONE).to_array(),
            "to": (self.to - Vec2::ONE).to_array(),
            "colors": self.colors,
            "style": if self.radial { "radial" } else { "linear" },
            "target": self.target,
            "dither": true,
        })
    }
}

pub(super) fn snap(from: Vec2, to: Vec2, shift: bool) -> Vec2 {
    let delta = to - from;
    if shift {
        let step = std::f32::consts::FRAC_PI_4;
        let angle = (delta.to_angle() / step).round() * step;
        from + Vec2::from_angle(angle) * delta.length()
    } else {
        to
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
struct Guide {
    #[uniform(0)]
    points: Vec4,
    #[uniform(1)]
    style: Vec4,
    #[uniform(2)]
    first: Vec4,
    #[uniform(3)]
    last: Vec4,
}

impl UiMaterial for Guide {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("image_gradient.wesl")).with_source("embedded"),
        )
    }
}

pub(super) struct GradientPlugin;

impl Plugin for GradientPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "image_gradient.wesl");
        app.init_resource::<Preview>()
            .add_plugins(UiMaterialPlugin::<Guide>::default())
            .add_systems(Update, refresh.after(super::update).before(super::refresh));
    }
}

#[derive(Resource)]
struct Interface {
    options: Entity,
    ramp: Entity,
    guide: Entity,
}

pub(super) fn setup(commands: &mut Commands, toolbar: Entity, canvas: Entity, font: &Handle<Font>) {
    use edit::EditAction;
    let options = node(
        commands,
        toolbar,
        Node {
            display: Display::None,
            flex_shrink: 0.0,
            ..row(4.0)
        },
    );
    let ramp = node(
        commands,
        options,
        Node {
            width: px(38),
            height: px(16),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
            ..default()
        },
    );
    commands
        .entity(ramp)
        .insert((BorderColor::all(EDGE), Pickable::IGNORE));
    for (title, action) in [
        ("Линейный", EditAction::GradientRadial(false)),
        ("Радиальный", EditAction::GradientRadial(true)),
        ("В прозрачность", EditAction::GradientTransparent),
    ] {
        button(
            commands,
            options,
            font,
            title,
            Action::ImageEdit(action),
            Look::Tool,
        );
    }
    icons::button(
        commands,
        options,
        icons::Glyph::Refresh,
        "Развернуть градиент",
        Action::ImageEdit(EditAction::GradientReverse),
        Look::Tool,
    );
    let guide = commands
        .spawn((
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                left: px(0),
                top: px(0),
                ..default()
            },
            ZIndex(5),
            Pickable::IGNORE,
            AccessibleLabel("Направление градиента".into()),
            ChildOf(canvas),
        ))
        .id();
    commands.insert_resource(Interface {
        options,
        ramp,
        guide,
    });
}

#[derive(Default, Resource)]
pub(in crate::editor) struct Preview {
    task: Option<(usize, Draft, Task<Result<DecodedImage, String>>)>,
    texture: Option<Handle<Image>>,
    rendered: Option<(usize, Draft)>,
    pub image: Option<(Handle<Image>, bool)>,
    guide: Option<Handle<Guide>>,
    last: Option<(usize, Draft)>,
    visibility: f32,
}

fn srgb(color: [f32; 4]) -> Color {
    Color::srgba(color[0], color[1], color[2], color[3])
}

fn linear(color: [f32; 4]) -> Vec4 {
    let c = srgb(color).to_linear();
    Vec4::new(c.red, c.green, c.blue, c.alpha)
}

fn refresh(
    editor: Res<Editor>,
    viewer: Res<ImageViewer>,
    interface: Option<Res<Interface>>,
    mut preview: ResMut<Preview>,
    mut nodes: Query<&mut Node>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut guides: ResMut<Assets<Guide>>,
    time: Res<Time>,
    mut commands: Commands,
    mut ramps: Query<&mut BackgroundGradient>,
) {
    let Some(ui) = interface else { return };
    let tab = editor.viewing_image;
    let enabled = viewer.edit.editing(tab) && viewer.edit.tool == PhotoTool::Gradient;
    if let Ok(mut options) = nodes.get_mut(ui.options) {
        let display = if enabled {
            Display::Flex
        } else {
            Display::None
        };
        if options.display != display {
            options.display = display;
        }
    }
    if enabled {
        let [first, last] = viewer
            .edit
            .gradient
            .colors(viewer.edit.color(), viewer.edit.background());
        let ramp: BackgroundGradient = LinearGradient::new(
            LinearGradient::TO_RIGHT,
            vec![ColorStop::auto(srgb(first)), ColorStop::auto(srgb(last))],
        )
        .into();
        if let Ok(mut existing) = ramps.get_mut(ui.ramp) {
            existing.set_if_neq(ramp);
        } else {
            commands.entity(ui.ramp).insert(ramp);
        }
    }
    let draft = tab
        .filter(|_| enabled)
        .and_then(|tab| Some((tab, viewer.edit.gradient_draft(tab)?)));
    let valid = draft.filter(|(tab, drag)| {
        viewer
            .edit
            .document(Some(*tab))
            .is_some_and(|doc| drag.valid_for(doc))
    });
    preview.update_image(valid, &viewer, &mut images);
    if let Some(draft) = draft {
        preview.last = Some(draft);
    }
    let increment = time.delta_secs().min(0.05) / 0.15;
    preview.visibility = (preview.visibility
        + if draft.is_some() {
            increment
        } else {
            -increment
        })
    .clamp(0.0, 1.0);
    let visible = preview
        .last
        .filter(|(owner, _)| Some(*owner) == tab && preview.visibility > 0.0);
    let Ok(mut node) = nodes.get_mut(ui.guide) else {
        return;
    };
    let Some(((_, drag), dimensions)) = visible.zip(viewer.dimensions) else {
        if node.display != Display::None {
            node.display = Display::None;
        }
        return;
    };
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }
    let rect = canvas_rect(&canvas, &window);
    let image = viewer.image_rect(dimensions, rect);
    let scale = image.size() / dimensions.as_vec2();
    let from = image.min - rect.min + drag.from * scale;
    let to = image.min - rect.min + drag.to * scale;
    let p = preview.visibility;
    let data = Guide {
        points: Vec4::new(from.x, from.y, to.x, to.y),
        style: Vec4::new(
            window.scale_factor(),
            p * p * (3.0 - 2.0 * p),
            time.elapsed_secs() % 120.0,
            if drag.radial { 1.0 } else { 0.0 },
        ),
        first: linear(drag.colors[0]),
        last: linear(drag.colors[1]),
    };
    if let Some(mut material) = preview
        .guide
        .as_ref()
        .and_then(|handle| guides.get_mut(handle))
    {
        *material = data;
    } else {
        let handle = guides.add(data);
        commands
            .entity(ui.guide)
            .insert(MaterialNode(handle.clone()));
        preview.guide = Some(handle);
    }
}

impl Preview {
    fn update_image(
        &mut self,
        valid: Option<(usize, Draft)>,
        viewer: &ImageViewer,
        images: &mut Assets<Image>,
    ) {
        let Some((tab, draft)) = valid else {
            self.image = None;
            self.rendered = None;
            self.task = None;
            return;
        };
        // Only a result from this drag and the same document revision can appear.
        let same_drag = |owner: usize, other: Draft| {
            owner == tab
                && Draft {
                    to: draft.to,
                    ..other
                } == draft
        };
        if self
            .rendered
            .is_some_and(|(owner, other)| !same_drag(owner, other))
        {
            self.rendered = None;
            self.image = None;
        }
        if let Some((owner, other, task)) = &mut self.task
            && let Some(result) = block_on(poll_once(task))
        {
            if same_drag(*owner, *other) {
                self.image = if let Ok(decoded) = result {
                    let full_resolution = decoded.size == decoded.original;
                    let image = image_previews::texture_image(decoded);
                    let handle = if let Some(handle) = self.texture.as_ref()
                        && let Some(mut texture) = images.get_mut(handle)
                    {
                        *texture = image;
                        handle.clone()
                    } else {
                        images.add(image)
                    };
                    self.texture = Some(handle.clone());
                    Some((handle, full_resolution))
                } else {
                    None
                };
                // A failed draft is not retried until the endpoint moves.
                self.rendered = Some((*owner, *other));
            }
            self.task = None;
        }
        if self.task.is_none()
            && self.rendered != valid
            && let Some(doc) = viewer.edit.document(Some(tab))
        {
            let render = doc.gradient_preview(draft.params());
            self.task = Some((
                tab,
                draft,
                AsyncComputeTaskPool::get().spawn(async move {
                    let image = render()?;
                    let original = UVec2::new(image.width, image.height);
                    if original.max_element() <= MAX_TEXTURE {
                        return Ok(DecodedImage {
                            size: original,
                            original,
                            rgba: image.pixels,
                        });
                    }
                    let rgba = image::RgbaImage::from_raw(image.width, image.height, image.pixels)
                        .ok_or("Некорректные пиксели градиента")?;
                    let reduced = image::DynamicImage::ImageRgba8(rgba)
                        .thumbnail(MAX_TEXTURE, MAX_TEXTURE)
                        .into_rgba8();
                    Ok(DecodedImage {
                        size: UVec2::new(reduced.width(), reduced.height()),
                        original,
                        rgba: reduced.into_raw(),
                    })
                }),
            ));
        }
    }
}
