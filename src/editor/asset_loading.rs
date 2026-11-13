//! A GPU-only loading animation over unfinished asset-browser previews.
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    prelude::*,
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

use super::{MUTED, WELL, icons};
use crate::editor::{
    Editor,
    browser::{Browser, Entry},
    image_previews::{self, ImagePreviews},
    thumbnails::{self, PreviewState, Thumbnails},
};

const REVEAL_SECONDS: f32 = 0.28;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct LoadingMaterial {
    // Opacity and a stable per-card phase. Animation time comes from the GPU globals.
    #[uniform(0)]
    state: Vec4,
}

impl UiMaterial for LoadingMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("asset_loading.wesl")).with_source("embedded"),
        )
    }
}

pub(crate) struct AssetLoadingPlugin;

impl Plugin for AssetLoadingPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "asset_loading.wesl");
        app.add_plugins(UiMaterialPlugin::<LoadingMaterial>::default())
            .add_systems(
                Update,
                sync.after(thumbnails::update)
                    .after(image_previews::update)
                    .after(super::refresh_browser)
                    .after(super::update_browser_files),
            );
    }
}

struct Visual {
    entity: Entity,
    material: Handle<LoadingMaterial>,
    opacity: f32,
}

#[derive(Component)]
pub(in crate::editor) struct LoadingPreview {
    entry: Entry,
    revision: u64,
    font: Handle<Font>,
    visual: Option<Visual>,
    fallback: Option<Entity>,
}

/// Attach to the preview well, so captions, buttons and drag targets stay intact.
pub(super) fn attach(
    commands: &mut Commands,
    well: Entity,
    entry: Entry,
    revision: u64,
    font: &Handle<Font>,
) {
    commands.entity(well).insert(LoadingPreview {
        entry,
        revision,
        font: font.clone(),
        visual: None,
        fallback: None,
    });
}

fn preview_state(entry: Entry, thumbnails: &Thumbnails, images: &ImagePreviews) -> PreviewState {
    match entry {
        Entry::Image(index) => match images.get(index) {
            None => PreviewState::Loading,
            Some(preview) if preview.image.is_some() => PreviewState::Ready,
            Some(_) => PreviewState::Failed,
        },
        _ => thumbnails.state(entry),
    }
}

fn phase(entry: Entry) -> f32 {
    let (index, offset) = match entry {
        Entry::Model(index) => (index, 0.0),
        Entry::Scene { room, .. } => (room, 1.7),
        Entry::Image(index) => (index, 3.4),
        _ => (0, 0.0),
    };
    (index % 997) as f32 * 2.399_963 + offset
}

fn fallback(commands: &mut Commands, parent: Entity, font: &Handle<Font>) -> Entity {
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: percent(100),
                height: percent(100),
                border_radius: BorderRadius::top(px(9)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(5),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            ZIndex(1),
            BackgroundColor(WELL),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id();
    icons::spawn(commands, root, icons::Glyph::File, 30.0, MUTED);
    commands.spawn((
        Text::new("Нет превью"),
        TextFont::from_font_size(11.0).with_font(font.clone()),
        TextColor(MUTED),
        Pickable::IGNORE,
        ChildOf(root),
    ));
    root
}

pub(in crate::editor) fn sync(
    mut commands: Commands,
    editor: Res<Editor>,
    browser: Res<Browser>,
    thumbnails: Res<Thumbnails>,
    images: Res<ImagePreviews>,
    time: Res<Time>,
    mut slots: Query<(Entity, &mut LoadingPreview)>,
    mut materials: ResMut<Assets<LoadingMaterial>>,
) {
    for (well, mut slot) in &mut slots {
        let current = slot.revision == browser.scan_revision;
        let state = if current {
            preview_state(slot.entry, &thumbnails, &images)
        } else {
            PreviewState::Ready
        };
        if state != PreviewState::Failed {
            if let Some(entity) = slot.fallback.take() {
                commands.entity(entity).try_despawn();
            }
        } else if slot.fallback.is_none() && !matches!(slot.entry, Entry::Image(_)) {
            slot.fallback = Some(fallback(&mut commands, well, &slot.font));
        }
        if state == PreviewState::Loading && editor.drawer {
            if slot.visual.is_none() {
                let material = materials.add(LoadingMaterial {
                    state: Vec4::new(1.0, phase(slot.entry), 0.0, 0.0),
                });
                let entity = commands
                    .spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(0),
                            top: px(0),
                            width: percent(100),
                            height: percent(100),
                            border_radius: BorderRadius::top(px(9)),
                            ..default()
                        },
                        MaterialNode(material.clone()),
                        ZIndex(2),
                        Pickable::IGNORE,
                        ChildOf(well),
                    ))
                    .id();
                slot.visual = Some(Visual {
                    entity,
                    material,
                    opacity: 1.0,
                });
            }
            if let Some(visual) = slot.visual.as_mut()
                && visual.opacity < 1.0
            {
                visual.opacity = 1.0;
                if let Some(mut material) = materials.get_mut(&visual.material) {
                    material.state.x = 1.0;
                }
            }
        } else if let Some(visual) = slot.visual.as_mut() {
            // Hidden/cached cards finish immediately and never flash a loader on return.
            visual.opacity = if editor.drawer && current {
                (visual.opacity - time.delta_secs().min(0.05) / REVEAL_SECONDS).max(0.0)
            } else {
                0.0
            };
            if visual.opacity == 0.0 {
                commands.entity(visual.entity).try_despawn();
                slot.visual = None;
            } else if let Some(mut material) = materials.get_mut(&visual.material) {
                let opacity = visual.opacity;
                material.state.x = opacity * opacity * (3.0 - 2.0 * opacity);
            }
        }
    }
}
