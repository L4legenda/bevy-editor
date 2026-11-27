use super::*;
use bevy::{
    asset::RenderAssetUsages,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    window::{CursorIcon, CustomCursor, CustomCursorImage, SystemCursorIcon},
};

#[derive(Default)]
pub(in crate::editor) struct StateCache {
    icons: [Option<CursorIcon>; 3],
    previous: Option<CursorIcon>,
    applied: Option<CursorIcon>,
}

fn icon(path: &str, images: &mut Assets<Image>) -> Option<CursorIcon> {
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">
        <g fill="none" stroke-linecap="round" stroke-linejoin="round">
          <path d="{path}" stroke="#15151b" stroke-width="4.5"/>
          <path d="{path}" stroke="#ffffff" stroke-width="2"/>
        </g></svg>"##
    );
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(32, 32)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let pixel = pixel.demultiply();
            [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
        })
        .collect();
    let image = images.add(Image::new(
        Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD,
    ));
    Some(CursorIcon::Custom(CustomCursor::Image(CustomCursorImage {
        handle: image,
        hotspot: (16, 16),
        ..default()
    })))
}

pub(in crate::editor) fn update(
    editor: Res<Editor>,
    state: Res<State>,
    window: Single<(Entity, &Window, Option<&CursorIcon>), With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut cache: Local<StateCache>,
    mut commands: Commands,
) {
    let (entity, window, current) = *window;
    let tool = state
        .gesture
        .as_ref()
        .filter(|gesture| {
            window.focused
                && editor.pending.is_none()
                && !editor.browser_popup
                && !editor.hdri_library_open
                && editor.viewing_image.is_none()
                && editor.viewing_model == Some(gesture.model)
                && matches!(state.mode(editor.viewing_model), Mode::Object | Mode::Edit)
        })
        .and_then(|gesture| gesture.transform_tool());
    let Some(tool) = tool else {
        if let Some(applied) = cache.applied.take()
            && current == Some(&applied)
        {
            if let Some(previous) = cache.previous.take() {
                commands.entity(entity).insert(previous);
            } else {
                commands.entity(entity).remove::<CursorIcon>();
            }
        }
        cache.previous = None;
        return;
    };
    let (index, path, fallback) = match tool {
        Tool::Universal | Tool::Move => (
            0,
            "M16 6v20 M6 16h20 M12 10l4-4 4 4 M12 22l4 4 4-4 M10 12l-4 4 4 4 M22 12l4 4-4 4",
            SystemCursorIcon::Move,
        ),
        Tool::Rotate => (
            1,
            "M25 15a9 9 0 1 1-3-6l3 3 M25 6v6h-6",
            SystemCursorIcon::Crosshair,
        ),
        Tool::Scale => (
            2,
            "M7 25 25 7 M7 18v7h7 M18 7h7v7",
            SystemCursorIcon::NeswResize,
        ),
    };
    let desired = cache.icons[index]
        .get_or_insert_with(|| icon(path, &mut images).unwrap_or_else(|| fallback.into()))
        .clone();
    if cache.applied.is_none() || current != cache.applied.as_ref() {
        cache.previous = current.cloned();
    }
    cache.applied = Some(desired.clone());
    if current != Some(&desired) {
        commands.entity(entity).insert(desired);
    }
}
