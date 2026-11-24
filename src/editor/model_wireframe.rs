//! View-only polygon edges, including a GLTF fallback for protected documents.
use super::*;
use crate::editor::wireframe::WireMaterial;
use bevy::{
    asset::{AssetId, RenderAssetUsages},
    light::{NotShadowCaster, NotShadowReceiver},
    mesh::{VertexAttributeValues, morph::MeshMorphWeights, skinning::SkinnedMesh},
};

#[derive(Resource)]
pub(in crate::editor) struct Materials(Handle<WireMaterial>);

impl FromWorld for Materials {
    fn from_world(world: &mut World) -> Self {
        let mut materials = world.resource_mut::<Assets<WireMaterial>>();
        Self(materials.add(WireMaterial::colored(Vec4::new(2.1, 1.35, 0.48, 1.0))))
    }
}

#[derive(Component)]
pub(in crate::editor) struct Overlay {
    source: Option<Entity>,
}

#[derive(Component)]
pub(in crate::editor) struct Owner {
    mesh: Option<AssetId<Mesh>>,
    overlay: Entity,
}

fn spawn(
    commands: &mut Commands,
    parent: Entity,
    mesh: Handle<Mesh>,
    material: Handle<WireMaterial>,
    source: Option<Entity>,
) -> Entity {
    commands
        .spawn((
            Overlay { source },
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Visibility::Inherited,
            RenderLayers::layer(LAYER),
            Pickable::IGNORE,
            NotShadowCaster,
            NotShadowReceiver,
            ChildOf(parent),
        ))
        .id()
}

fn polygon_mesh(object: &RenderedObject) -> Mesh {
    let indices: Vec<u32> = object
        .edges
        .keys()
        .flat_map(|edge| [edge.0 as u32, edge.1 as u32])
        .collect();
    Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default())
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            object
                .geometry
                .positions
                .iter()
                .map(|p| p.to_array())
                .collect::<Vec<_>>(),
        )
        .with_inserted_indices(Indices::U32(indices))
}

fn source_mesh(source: &Mesh) -> Option<Mesh> {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        source.try_attribute_option(Mesh::ATTRIBUTE_POSITION).ok()?
    else {
        return None;
    };
    let indices: Vec<u32> = source.try_indices_option().ok()?.map_or_else(
        || (0..positions.len() as u32).collect(),
        |indices| indices.iter().map(|index| index as u32).collect(),
    );
    let mut edges = Vec::<[u32; 2]>::new();
    let mut edge = |a: u32, b: u32| {
        if a != b && (a as usize) < positions.len() && (b as usize) < positions.len() {
            edges.push([a.min(b), a.max(b)]);
        }
    };
    match source.primitive_topology() {
        PrimitiveTopology::TriangleList => {
            for face in indices.chunks_exact(3) {
                edge(face[0], face[1]);
                edge(face[1], face[2]);
                edge(face[2], face[0]);
            }
        }
        PrimitiveTopology::TriangleStrip => {
            for face in indices.windows(3) {
                if face[0] != face[1] && face[1] != face[2] && face[2] != face[0] {
                    edge(face[0], face[1]);
                    edge(face[1], face[2]);
                    edge(face[2], face[0]);
                }
            }
        }
        PrimitiveTopology::LineList => {
            for pair in indices.chunks_exact(2) {
                edge(pair[0], pair[1]);
            }
        }
        PrimitiveTopology::LineStrip => {
            for pair in indices.windows(2) {
                edge(pair[0], pair[1]);
            }
        }
        _ => return None,
    }
    edges.sort_unstable();
    edges.dedup();
    let mut mesh = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default());
    // Keep the original vertex order, joints and morph target indices.
    for (attribute, values) in source.try_attributes().ok()? {
        mesh.insert_attribute(attribute.clone(), values.clone());
    }
    mesh.insert_indices(Indices::U32(edges.into_iter().flatten().collect()));
    if let Some(targets) = source.morph_targets() {
        mesh.set_morph_targets(targets.clone());
    }
    Some(mesh)
}

pub(in crate::editor) fn sync(
    mut commands: Commands,
    viewer: Res<Viewer>,
    editor: Res<Editor>,
    state: Res<State>,
    shading: Res<super::super::shading::Shading>,
    materials: Res<Materials>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut changes: MessageReader<AssetEvent<Mesh>>,
    children: Query<&Children>,
    owners: Query<&Owner>,
    sources: Query<
        (&Mesh3d, Option<&SkinnedMesh>, Option<Ref<MeshMorphWeights>>),
        Without<Overlay>,
    >,
    mut overlays: Query<
        (
            Entity,
            &Overlay,
            &mut Visibility,
            Option<&mut MeshMorphWeights>,
        ),
        With<Overlay>,
    >,
) {
    let changed: std::collections::HashSet<_> = changes
        .read()
        .filter_map(|event| match event {
            AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect();
    let editing_mode = state.mode(editor.viewing_model);
    let (mode, _) = shading.viewport(editing_mode);
    let show_edges = mode == super::super::shading::Mode::Wire
        || (editing_mode == crate::editor::model_document::Mode::Paint && state.wireframe);
    for (entity, overlay, mut visibility, weights) in &mut overlays {
        visibility.set_if_neq(if show_edges {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if let Some(source) = overlay.source
            && let Ok((_, _, Some(source_weights))) = sources.get(source)
            && source_weights.is_changed()
        {
            if let Some(mut weights) = weights {
                *weights = (*source_weights).clone();
            } else {
                commands.entity(entity).insert((*source_weights).clone());
            }
        }
    }
    let (Some(index), Some(root)) = (editor.viewing_model, viewer.root) else {
        return;
    };
    if !show_edges {
        return;
    }
    if state.documents.contains_key(&index) {
        for object in state.render_objects.values() {
            if owners
                .get(object.entity)
                .is_ok_and(|owner| overlays.contains(owner.overlay))
            {
                continue;
            }
            let overlay = spawn(
                &mut commands,
                object.entity,
                meshes.add(polygon_mesh(object)),
                materials.0.clone(),
                None,
            );
            commands.entity(object.entity).insert(Owner {
                mesh: None,
                overlay,
            });
        }
    } else {
        for entity in std::iter::once(root).chain(children.iter_descendants(root)) {
            let Ok((handle, skin, weights)) = sources.get(entity) else {
                continue;
            };
            let owner = owners.get(entity).ok();
            if owner.is_some_and(|owner| {
                owner.mesh == Some(handle.id()) && overlays.contains(owner.overlay)
            }) && !changed.contains(&handle.id())
            {
                continue;
            }
            let Some(mesh) = meshes.get(&handle.0).and_then(source_mesh) else {
                continue;
            };
            if let Some(owner) = owner {
                commands.entity(owner.overlay).try_despawn();
            }
            let overlay = spawn(
                &mut commands,
                entity,
                meshes.add(mesh),
                materials.0.clone(),
                Some(entity),
            );
            if let Some(skin) = skin {
                commands.entity(overlay).insert(skin.clone());
            }
            if let Some(weights) = weights {
                commands.entity(overlay).insert((*weights).clone());
            }
            commands.entity(entity).insert(Owner {
                mesh: Some(handle.id()),
                overlay,
            });
        }
    }
}
