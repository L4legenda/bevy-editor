//! View-only materials: original GLTF materials remain shared with the scene.
use super::super::paint::{PaintState, bake, engine::Runtime, sets::SetPart};
use super::*;
use bevy::asset::AssetId;

#[derive(Component)]
pub(in crate::editor) struct SourceMaterial(pub Handle<StandardMaterial>);

#[derive(Resource, Default)]
pub(in crate::editor) struct Materials {
    transparent: HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
    channels: HashMap<AssetId<Image>, Handle<StandardMaterial>>,
    bake_map: bake::view3d::Overlay,
}

pub(in crate::editor::ui::model_viewer) fn sync(
    state: Res<State>,
    editor: Res<Editor>,
    paint: Option<Res<Runtime>>,
    paint_state: Option<Res<PaintState>>,
    baked: Option<Res<bake::State>>,
    mut cache: ResMut<Materials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: Query<(
        &SourceMaterial,
        Option<&SetPart>,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let mut used = std::collections::HashSet::new();
    let mut shown_channels = std::collections::HashSet::new();
    let paint_model = editor
        .viewing_model
        .filter(|&index| state.mode(Some(index)) == Mode::Paint);
    let map = (|| {
        if editor.viewing_image.is_some() {
            return None;
        }
        let index = paint_model?;
        cache.bake_map.update(
            index,
            state.documents.get(&index)?,
            paint_state.as_deref()?,
            baked.as_deref()?,
            &mut materials,
        )
    })();
    if map.is_none() {
        cache.bake_map.clear();
    }
    let channel = paint_model.filter(|_| editor.viewing_image.is_none())
        .and_then(|index| Some((index, paint_state.as_deref()?.view_channel(index)?)));
    for (source, part, mut displayed) in &mut meshes {
        if let Some((key, handle)) = &map
            && part.is_some_and(|part| (part.0, part.1) == *key)
        {
            if displayed.0 != *handle {
                displayed.0 = handle.clone();
            }
            continue;
        }
        if let Some((index, channel)) = channel
            && let Some(part) = part
            && let Some(image) = paint.as_ref().and_then(|paint| paint.preview(index, part.0, part.1, channel))
        {
            shown_channels.insert(image.id());
            let handle = cache.channels.entry(image.id()).or_insert_with(|| {
                let mut material = StandardMaterial {
                    unlit: true,
                    base_color_texture: Some(image),
                    ..default()
                };
                if let Some(source) = materials.get(&source.0) {
                    material.double_sided = source.double_sided;
                    material.cull_mode = source.cull_mode;
                }
                materials.add(material)
            });
            if displayed.0 != *handle {
                displayed.0 = handle.clone();
            }
            continue;
        }
        let source = paint_model
            .zip(part)
            .and_then(|(index, part)| paint.as_ref()?.working_material(index, part.0, part.1))
            .unwrap_or(&source.0);
        let handle = if state.xray {
            let id = source.id();
            used.insert(id);
            if !cache.transparent.contains_key(&id) {
                let Some(mut material) = materials.get(source).cloned() else {
                    continue;
                };
                material.base_color = material
                    .base_color
                    .with_alpha(material.base_color.alpha() * 0.22);
                material.alpha_mode = AlphaMode::Blend;
                material.double_sided = true;
                material.cull_mode = None;
                cache.transparent.insert(id, materials.add(material));
            }
            cache.transparent[&id].clone()
        } else {
            source.clone()
        };
        if displayed.0 != handle {
            displayed.0 = handle;
        }
    }
    cache.transparent.retain(|id, _| used.contains(id));
    cache.channels.retain(|id, _| shown_channels.contains(id));
}
