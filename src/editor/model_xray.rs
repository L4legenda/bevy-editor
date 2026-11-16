//! View-only materials: original GLTF materials remain shared with the scene.
use super::*;
use bevy::asset::AssetId;

#[derive(Component)]
pub(in crate::editor) struct SourceMaterial(pub Handle<StandardMaterial>);

#[derive(Resource, Default)]
pub(in crate::editor) struct Materials {
    transparent: HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
}

pub(in crate::editor) fn sync(
    state: Res<State>,
    mut cache: ResMut<Materials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: Query<(&SourceMaterial, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let mut used = std::collections::HashSet::new();
    for (source, mut displayed) in &mut meshes {
        let handle = if state.xray {
            let id = source.0.id();
            used.insert(id);
            if !cache.transparent.contains_key(&id) {
                let Some(mut material) = materials.get(&source.0).cloned() else {
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
            source.0.clone()
        };
        if displayed.0 != handle {
            displayed.0 = handle;
        }
    }
    cache.transparent.retain(|id, _| used.contains(id));
}
