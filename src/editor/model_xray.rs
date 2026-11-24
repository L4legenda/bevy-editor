//! View-only materials: original GLTF materials remain shared with the scene.
use super::super::shading::{Mode, Shading};
use super::*;
use bevy::asset::AssetId;

#[derive(Component)]
pub(in crate::editor) struct SourceMaterial(pub Handle<StandardMaterial>);

#[derive(Resource, Default)]
pub(in crate::editor) struct Materials {
    variants: HashMap<(AssetId<StandardMaterial>, bool, bool), Handle<StandardMaterial>>,
    solid: Option<Handle<StandardMaterial>>,
}

pub(in crate::editor) fn sync(
    state: Res<State>,
    shading: Res<Shading>,
    mut cache: ResMut<Materials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    meshes: Res<Assets<Mesh>>,
    mut changes: MessageReader<AssetEvent<StandardMaterial>>,
    mut surfaces: Query<(
        &SourceMaterial,
        &Mesh3d,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut RenderLayers,
    )>,
) {
    for event in changes.read() {
        if let AssetEvent::Modified { id } = event {
            cache.variants.retain(|(source, _, _), _| source != id);
        }
    }
    let mut used = std::collections::HashSet::new();
    for (source, mesh, mut displayed, mut layers) in &mut surfaces {
        let wire_only = shading.mode == Mode::Wire
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
        let source = if shading.mode == Mode::Solid {
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
            source.0.clone()
        };
        let unlit = shading.mode == Mode::Material;
        let handle = if unlit || state.xray {
            let key = (source.id(), unlit, state.xray);
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
    cache.variants.retain(|key, _| used.contains(key));
}
