//! The same embedded artwork is used by native windows and in-app headers.
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

const ICON_DATA: &[u8] = include_bytes!("../assets/icons/bevy-editor.ico");

pub(crate) struct AppIconPlugin;

impl Plugin for AppIconPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Assets<Image>>()
            .init_resource::<AppIcon>()
            .add_observer(attach_brand);
        #[cfg(target_os = "macos")]
        if app.is_plugin_added::<bevy::winit::WinitPlugin>() {
            use bevy::{
                ecs::schedule::common_conditions::{on_message, run_once},
                window::WindowCreated,
            };

            app.add_systems(
                Update,
                set_dock_icon.run_if(on_message::<WindowCreated>.and_then(run_once)),
            );
        }
    }
}

#[derive(Resource)]
pub(crate) struct AppIcon {
    image: Handle<Image>,
    #[cfg(not(target_arch = "wasm32"))]
    pixels: image::RgbaImage,
}

impl FromWorld for AppIcon {
    fn from_world(world: &mut World) -> Self {
        let pixels = image::load_from_memory_with_format(ICON_DATA, image::ImageFormat::Ico)
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
            #[cfg(not(target_arch = "wasm32"))]
            pixels,
        }
    }
}

#[cfg(not(any(target_arch = "wasm32", target_os = "macos")))]
impl AppIcon {
    pub(crate) fn window_icon(&self) -> Result<winit::window::Icon, winit::window::BadIcon> {
        winit::window::Icon::from_rgba(
            self.pixels.to_vec(),
            self.pixels.width(),
            self.pixels.height(),
        )
    }
}

#[cfg(target_os = "macos")]
fn set_dock_icon(_main_thread: bevy::ecs::system::NonSendMarker, artwork: Res<AppIcon>) {
    use image::{ImageEncoder, RgbaImage, codecs::png::PngEncoder, imageops};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{MainThreadMarker, NSData};

    // Leave a transparent margin so the icon's tile aligns with the surrounding Dock icons.
    let pixels = &artwork.pixels;
    let padding = (pixels.width() as f32 * 0.09).round() as u32;
    let mut padded = RgbaImage::new(pixels.width() + 2 * padding, pixels.height() + 2 * padding);
    imageops::replace(&mut padded, pixels, i64::from(padding), i64::from(padding));
    let mut png = Vec::new();
    if let Err(error) = PngEncoder::new(&mut png).write_image(
        padded.as_raw(),
        padded.width(),
        padded.height(),
        image::ExtendedColorType::Rgba8,
    ) {
        warn!("Could not encode the Dock icon: {error}");
        return;
    }
    let main_thread = MainThreadMarker::new().expect("Dock icon requires the main thread");
    let data = NSData::with_bytes(&png);
    let Some(icon) = NSImage::initWithData(main_thread.alloc(), &data) else {
        warn!("Could not decode the Dock icon");
        return;
    };
    // SAFETY: This runs on the AppKit main thread after winit creates the window.
    // AppKit retains the non-null image.
    unsafe { NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&icon)) };
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
