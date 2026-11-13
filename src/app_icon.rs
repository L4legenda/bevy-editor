//! The same embedded artwork is used by native windows and in-app headers.
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

pub(crate) struct AppIconPlugin;

impl Plugin for AppIconPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Assets<Image>>()
            .init_resource::<AppIcon>()
            .add_observer(attach_brand);
    }
}

#[derive(Resource)]
pub(crate) struct AppIcon {
    image: Handle<Image>,
    pixels: image::RgbaImage,
}

impl FromWorld for AppIcon {
    fn from_world(world: &mut World) -> Self {
        let pixels = image::load_from_memory_with_format(
            include_bytes!("../assets/icons/bevy-editor.ico"),
            image::ImageFormat::Ico,
        )
        .expect("decode embedded application icon")
        .into_rgba8();
        let mut image = Image::new(
            Extent3d {
                width: pixels.width(),
                height: pixels.height(),
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels.to_vec(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        image.sampler = bevy::image::ImageSampler::linear();
        Self {
            image: world.resource_mut::<Assets<Image>>().add(image),
            pixels,
        }
    }
}

impl AppIcon {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn window_icon(&self) -> Result<winit::window::Icon, winit::window::BadIcon> {
        winit::window::Icon::from_rgba(
            self.pixels.to_vec(),
            self.pixels.width(),
            self.pixels.height(),
        )
    }
}

#[derive(Component)]
#[require(Node, ImageNode)]
struct Brand;

pub(crate) fn brand(commands: &mut Commands, parent: Entity) -> Entity {
    commands
        .spawn((
            Brand,
            Node {
                width: px(31),
                height: px(31),
                flex_shrink: 0.0,
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}

fn attach_brand(added: On<Add<Brand>>, icon: Res<AppIcon>, mut commands: Commands) {
    commands
        .entity(added.entity)
        .insert(ImageNode::new(icon.image.clone()));
}
