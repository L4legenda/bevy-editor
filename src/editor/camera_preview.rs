use bevy::{
    anti_alias::taa::TemporalAntiAliasing,
    camera::{RenderTarget, visibility::RenderLayers},
    pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceTransmission},
    prelude::*,
    render::render_resource::TextureFormat,
    window::PrimaryWindow,
};

use super::{
    Action, DrawerMotion, Editor, Look, MUTED, Selection, TEXT, WELL, button, column, node,
    scene_input_rect, surface, text, viewport,
};
use crate::data::CameraPlacement;

const WIDTH: f32 = 306.0;
const HEIGHT: f32 = 242.0;
const MARGIN: f32 = 14.0;
const IMAGE_WIDTH: f32 = 288.0;
const IMAGE_HEIGHT: f32 = IMAGE_WIDTH / viewport::CAMERA_ASPECT;

#[derive(Component)]
pub(in crate::editor) struct PreviewCamera;

#[derive(Resource)]
pub(in crate::editor) struct Preview {
    image: Handle<Image>,
    card: Entity,
    title: Entity,
    exit: Entity,
    help: Entity,
    camera: Option<Entity>,
    room: Option<usize>,
    last_view: Option<(usize, CameraPlacement)>,
}

pub(in crate::editor) fn rect(
    window: &Window,
    drawer: &DrawerMotion,
    sidebar: &super::SidebarLayout,
) -> Rect {
    let scene = scene_input_rect(window, drawer, sidebar);
    if scene.width() < WIDTH + MARGIN * 2.0 || scene.height() < HEIGHT + MARGIN * 2.0 {
        return Rect::default();
    }
    let max = scene.max - Vec2::splat(MARGIN);
    Rect::from_corners(max - Vec2::new(WIDTH, HEIGHT), max)
}

pub(super) fn setup(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    viewport: Entity,
    font: &Handle<Font>,
) {
    let image = images.add(Image::new_target_texture(
        640,
        (640.0 / viewport::CAMERA_ASPECT) as u32,
        TextureFormat::Rgba8Unorm,
        Some(TextureFormat::Rgba8UnormSrgb),
    ));
    let card = surface(
        commands,
        viewport,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            right: px(MARGIN),
            bottom: px(MARGIN),
            width: px(WIDTH),
            height: px(HEIGHT),
            padding: UiRect::all(px(8)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(9)),
            overflow: Overflow::clip(),
            ..column(6.0)
        },
        WELL,
    );
    commands.entity(card).insert(ZIndex(5));
    let title = text(commands, card, font, "Камера · 16:9", 12.0, MUTED);
    commands.spawn((
        ImageNode::new(image.clone()),
        Node {
            width: px(IMAGE_WIDTH),
            height: px(IMAGE_HEIGHT),
            flex_shrink: 0.0,
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(card),
    ));
    button(
        commands,
        card,
        font,
        "Управление камерой",
        Action::Pilot,
        Look::Primary,
    );

    let exit = node(
        commands,
        viewport,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: px(MARGIN),
            right: px(MARGIN),
            ..default()
        },
    );
    commands.entity(exit).insert(ZIndex(5));
    button(
        commands,
        exit,
        font,
        "Выйти из камеры · Esc",
        Action::Pilot,
        Look::Regular,
    );

    let help = surface(
        commands,
        viewport,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(MARGIN),
            bottom: px(MARGIN),
            padding: UiRect::axes(px(12), px(8)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(9)),
            ..column(3.0)
        },
        Color::srgba(0.082, 0.090, 0.112, 0.91),
    );
    commands.entity(help).insert((Pickable::IGNORE, ZIndex(5)));
    text(
        commands,
        help,
        font,
        "ПКМ + WASD · движение   Q/E · вверх/вниз",
        12.0,
        TEXT,
    );
    text(
        commands,
        help,
        font,
        "Мышь · поворот   Shift · быстрее   Ctrl+Z · отмена",
        11.0,
        MUTED,
    );
    commands.insert_resource(Preview {
        image,
        card,
        title,
        exit,
        help,
        camera: None,
        room: None,
        last_view: None,
    });
}

pub(in crate::editor) fn update(
    mut commands: Commands,
    editor: Res<Editor>,
    mut preview: ResMut<Preview>,
    drawer: Res<DrawerMotion>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut cameras: Query<(&mut Camera, &mut Transform, &mut Projection), With<PreviewCamera>>,
    mut panes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
) {
    // Reapply host camera effects when opening another scene, just like the editor camera.
    if preview.room != Some(editor.room) {
        if let Some(camera) = preview.camera.take() {
            commands.entity(camera).despawn();
        }
        let mut camera = commands.spawn((
            Camera3d::default(),
            ScreenSpaceTransmission::default(),
            AmbientLight {
                brightness: 150.0,
                ..default()
            },
        ));
        if let Some(setup) = editor.project.camera_setup {
            let asset = crate::asset_paths::original_asset_path(
                &editor.project.asset_root,
                &editor.definition().asset,
            )
            .unwrap_or_else(|_| editor.definition().asset.clone());
            setup(&mut camera, &asset);
        }
        camera
            .insert((
                PreviewCamera,
                RenderTarget::Image(preview.image.clone().into()),
            ))
            .remove::<(TemporalAntiAliasing, ScreenSpaceAmbientOcclusion)>();
        camera
            .entry::<RenderLayers>()
            .or_default()
            .and_modify(|mut layers| {
                *layers = layers
                    .clone()
                    .without(viewport::EDITOR_GIZMO_LAYER)
                    .without(super::super::selection_outline::LAYER);
            });
        camera.entry::<Camera>().and_modify(|mut camera| {
            camera.order = -2;
            camera.is_active = false;
            camera.viewport = None;
        });
        preview.camera = Some(camera.id());
        preview.room = Some(editor.room);
        preview.last_view = None;
    }

    let selected = match editor.selection {
        Some(Selection::Camera(shot)) if editor.ready => Some(shot),
        _ => None,
    };
    let shown = selected.is_some()
        && !editor.pilot
        && rect(&window, &drawer, &editor.sidebar).width() > 0.0;
    for (entity, visible) in [
        (preview.card, shown),
        (preview.exit, selected.is_some() && editor.pilot),
        (preview.help, selected.is_some() && editor.pilot),
    ] {
        if let Ok(mut pane) = panes.get_mut(entity) {
            let display = if visible {
                Display::Flex
            } else {
                Display::None
            };
            if pane.display != display {
                pane.display = display;
            }
            if entity != preview.exit {
                let bottom = px(MARGIN + drawer.visible_height);
                if pane.bottom != bottom {
                    pane.bottom = bottom;
                }
            }
        }
    }
    if let Some(shot) = selected {
        if let Ok(mut title) = labels.get_mut(preview.title) {
            let label = format!("{} · 16:9", editor.definition().cameras[shot].label);
            if title.0 != label {
                title.0 = label;
            }
        }
    }
    if let Some(entity) = preview.camera
        && let Ok((mut camera, mut pose, mut lens)) = cameras.get_mut(entity)
    {
        if camera.is_active != shown {
            camera.is_active = shown;
        }
        if let Some(shot) = selected.filter(|_| shown) {
            let placement = editor.camera(shot);
            if preview.last_view != Some((shot, placement)) {
                pose.set_if_neq(placement.pose.transform());
                *lens = viewport::projection(
                    editor.definition().cameras[shot].projection,
                    placement.lens,
                );
                preview.last_view = Some((shot, placement));
            }
        }
    }
}
