//! View-only materials: original GLTF materials remain shared with the scene.
use super::super::paint::{PaintState, bake, engine::Runtime, sets::SetPart};
use super::super::shading::{Mode, Shading};
use super::*;
use bevy::asset::AssetId;

#[derive(Component)]
pub(in crate::editor) struct SourceMaterial(pub Handle<StandardMaterial>);

#[derive(Resource, Default)]
pub(in crate::editor) struct Materials {
    channels: HashMap<AssetId<Image>, Handle<StandardMaterial>>,
    bake_map: bake::view3d::Overlay,
    variants: HashMap<(AssetId<StandardMaterial>, bool, bool, bool), Handle<StandardMaterial>>,
    solid: Option<Handle<StandardMaterial>>,
}

pub(in crate::editor::ui::model_viewer) fn sync(
    state: Res<State>,
    editor: Res<Editor>,
    paint: Option<Res<Runtime>>,
    paint_state: Option<Res<PaintState>>,
    baked: Option<Res<bake::State>>,
    shading: Res<Shading>,
    mut cache: ResMut<Materials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    meshes: Res<Assets<Mesh>>,
    mut changes: MessageReader<AssetEvent<StandardMaterial>>,
    mut surfaces: Query<(
        &SourceMaterial,
        &Mesh3d,
        Option<&SetPart>,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut RenderLayers,
    )>,
) {
    for event in changes.read() {
        if let AssetEvent::Modified { id } = event {
            cache.variants.retain(|(source, ..), _| source != id);
        }
    }
    let mut used = std::collections::HashSet::new();
    let mut shown_channels = std::collections::HashSet::new();
    let editing_mode = state.mode(editor.viewing_model);
    // Newly extruded strips are open surfaces and must be visible from both sides.
    let mesh_editing = editing_mode == crate::editor::model_document::Mode::Edit;
    let (mode, _) = shading.viewport(editing_mode);
    let paint_model = editor
        .viewing_model
        .filter(|_| editing_mode == crate::editor::model_document::Mode::Paint);
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
    for (source, mesh, part, mut displayed, mut layers) in &mut surfaces {
        let wire_only = mode == Mode::Wire
            && meshes.get(&mesh.0).is_some_and(|mesh| {
                matches!(
                    mesh.primitive_topology(),
                    PrimitiveTopology::TriangleList | PrimitiveTopology::TriangleStrip
                )
            });
        layers.set_if_neq(if wire_only {
            RenderLayers::none()
        } else {
            RenderLayers::layer(LAYER)
        });
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
        let source = if mode == Mode::Solid {
            cache
                .solid
                .get_or_insert_with(|| {
                    materials.add(StandardMaterial {
                        base_color: Color::srgb(0.62, 0.63, 0.66),
                        perceptual_roughness: 0.82,
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    })
                })
                .clone()
        } else {
            paint_model
                .zip(part)
                .and_then(|(index, part)| paint.as_ref()?.working_material(index, part.0, part.1))
                .unwrap_or(&source.0)
                .clone()
        };
        let unlit = mode == Mode::Material;
        let two_sided = !state.xray && mesh_editing && materials.get(&source)
            .is_some_and(|material| material.cull_mode.is_some() || !material.double_sided);
        let handle = if unlit || state.xray || two_sided {
            let key = (source.id(), unlit, state.xray, two_sided);
            used.insert(key);
            if !cache.variants.contains_key(&key) {
                let Some(mut material) = materials.get(&source).cloned() else {
                    continue;
                };
                material.unlit |= unlit;
                if state.xray {
                    material.base_color = material
                        .base_color
                        .with_alpha(material.base_color.alpha() * 0.22);
                    material.alpha_mode = AlphaMode::Blend;
                }
                if two_sided || state.xray {
                    material.double_sided = true;
                    material.cull_mode = None;
                }
                cache.variants.insert(key, materials.add(material));
            }
            cache.variants[&key].clone()
        } else {
            source
        };
        if displayed.0 != handle {
            displayed.0 = handle;
        }
    }
    cache.channels.retain(|id, _| shown_channels.contains(id));
    cache.variants.retain(|key, _| used.contains(key));
}
