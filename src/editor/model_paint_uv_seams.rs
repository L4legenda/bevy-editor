//! Depth-tested paint viewport overlays built from the UV editor's seam topology.
use super::{PaintState, editing, sets::SetPart};
use crate::editor::{
    Editor,
    model_document::{MeshData, Mode},
    model_uv_mesh::{has_face_uv, seam_faces},
    wireframe::WireMaterial,
};
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::RenderLayers,
    light::{NotShadowCaster, NotShadowReceiver},
    mesh::PrimitiveTopology,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    window::RequestRedraw,
};
use std::collections::{HashMap, HashSet};

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct Overlay;

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct SeamChild(Entity);

type LinesBySlot = HashMap<usize, Vec<[f32; 3]>>;

struct ObjectCache {
    geometry: MeshData,
    job: Option<Task<LinesBySlot>>,
    meshes: HashMap<usize, Handle<Mesh>>,
}

impl ObjectCache {
    fn new(geometry: MeshData) -> Self {
        let source = geometry.clone();
        Self {
            geometry,
            job: Some(AsyncComputeTaskPool::get().spawn(async move { lines_by_slot(&source) })),
            meshes: HashMap::new(),
        }
    }
}

#[derive(Default)]
pub(in crate::editor::ui::model_viewer) struct Cache {
    model: Option<usize>,
    objects: HashMap<usize, ObjectCache>,
    material: Option<Handle<WireMaterial>>,
}

fn lines_by_slot(mesh: &MeshData) -> LinesBySlot {
    let valid: Vec<_> = mesh.faces.iter().map(has_face_uv).collect();
    let mut lines = LinesBySlot::new();
    let mut slots = Vec::new();
    for (edge, faces) in seam_faces(mesh) {
        slots.clear();
        slots.extend(
            faces
                .into_iter()
                .filter(|&face| valid[face])
                .map(|face| mesh.faces[face].material),
        );
        slots.sort_unstable();
        slots.dedup();
        for &slot in &slots {
            lines.entry(slot).or_default().extend([
                mesh.positions[edge.0].to_array(),
                mesh.positions[edge.1].to_array(),
            ]);
        }
    }
    lines
}

pub(in crate::editor::ui::model_viewer) fn sync(
    mut commands: Commands,
    editor: Res<Editor>,
    editing: Res<editing::State>,
    paint: Res<PaintState>,
    maker: Res<super::ui::ShaderMaker>,
    mut cache: Local<Cache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WireMaterial>>,
    parts: Query<(Entity, &SetPart, &RenderLayers, Option<&SeamChild>)>,
    overlays: Query<(Entity, &Mesh3d), With<Overlay>>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let model = editor.viewing_model;
    if cache.model != model {
        cache.model = model;
        cache.objects.clear();
    }
    let doc = model
        .and_then(|index| editing.documents.get(&index))
        .filter(|doc| {
            doc.mode == Mode::Paint
                && doc.paint_uv.show_seams_3d
                && editor.pending.is_none()
                && editor.viewing_image.is_none()
                && paint.has_3d_view(model, maker.shows_model())
                && !paint.blocks_view_navigation(model, maker.shows_model())
        });
    let Some(doc) = doc else {
        for (entity, _) in &overlays {
            commands.entity(entity).try_despawn();
            redraw.write(RequestRedraw);
        }
        return;
    };
    let ids: HashSet<_> = doc
        .objects
        .iter()
        .filter(|object| !object.hidden)
        .map(|object| object.id)
        .collect();
    cache.objects.retain(|id, _| ids.contains(id));
    for object in doc.objects.iter().filter(|object| !object.hidden) {
        let entry = cache
            .objects
            .entry(object.id)
            .or_insert_with(|| ObjectCache::new(object.geometry.clone()));
        if !entry.geometry.shares_storage_with(&object.geometry) {
            *entry = ObjectCache::new(object.geometry.clone());
        }
        if let Some(job) = entry.job.as_mut() {
            redraw.write(RequestRedraw);
            if let Some(lines) = block_on(poll_once(job)) {
                entry.job = None;
                entry.meshes = lines
                    .into_iter()
                    .map(|(slot, positions)| {
                        let mesh =
                            Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default())
                                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions);
                        (slot, meshes.add(mesh))
                    })
                    .collect();
            }
        }
    }
    for (parent, part, layers, child) in &parts {
        let current = child.and_then(|child| overlays.get(child.0).ok());
        let next = cache
            .objects
            .get(&part.0)
            .and_then(|object| object.meshes.get(&part.1));
        if current.is_some_and(|(_, mesh)| Some(&mesh.0) == next) {
            continue;
        }
        if let Some((entity, _)) = current {
            commands.entity(entity).try_despawn();
            redraw.write(RequestRedraw);
        }
        if let Some(mesh) = next.cloned() {
            let material = cache.material.get_or_insert_with(|| {
                materials.add(
                    WireMaterial::colored(Vec4::new(3.0, 0.24, 0.12, 1.0)).after_other_lines(),
                )
            });
            let child = commands
                .spawn((
                    Overlay,
                    Mesh3d(mesh),
                    MeshMaterial3d(material.clone()),
                    Transform::IDENTITY,
                    Visibility::Inherited,
                    layers.clone(),
                    Pickable::IGNORE,
                    NotShadowCaster,
                    NotShadowReceiver,
                    ChildOf(parent),
                ))
                .id();
            commands.entity(parent).try_insert(SeamChild(child));
            redraw.write(RequestRedraw);
        }
    }
}
