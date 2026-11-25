//! Diagnostic cage overlay in the model viewer, independent of the UV bake.
use super::*;
use crate::editor::ui::model_viewer::editing::input::ModelGizmos;
use bevy::{
    camera::visibility::RenderLayers,
    light::{NotShadowCaster, NotShadowReceiver},
    mesh::{Indices, PrimitiveTopology},
};
use std::time::{Duration, Instant};

#[derive(Component)]
pub(in crate::editor::ui::model_viewer::paint) struct Shell;

struct Job {
    progress: Arc<Progress>,
    version: u64,
    task: Task<Result<(Arc<mesh::probe::Prepared>, mesh::probe::Diagnostic), String>>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.progress.cancelled.store(true, Ordering::Relaxed);
    }
}

#[derive(Resource, Default)]
pub(in crate::editor::ui::model_viewer) struct State {
    request: Option<(Key, ProjectionSettings)>,
    pending: Option<Job>,
    prepared: Option<Arc<mesh::probe::Prepared>>,
    applied_version: Option<u64>,
    last_update: Option<Instant>,
    ready: Option<mesh::probe::Diagnostic>,
    entity: Option<Entity>,
    material: Option<Handle<StandardMaterial>>,
    status: String,
    #[cfg(feature = "mcp")]
    error: Option<String>,
}

impl State {
    fn clear(&mut self, commands: &mut Commands) {
        self.request = None;
        self.pending = None;
        self.prepared = None;
        self.applied_version = None;
        self.last_update = None;
        self.ready = None;
        self.status.clear();
        #[cfg(feature = "mcp")]
        {
            self.error = None;
        }
        if let Some(entity) = self.entity.take() {
            commands.entity(entity).try_despawn();
        }
    }

    #[cfg(feature = "mcp")]
    pub(in crate::editor::ui::model_viewer::paint) fn report(
        &self, model: usize, doc: &Document, binding: (usize, usize), version: u64,
    ) -> serde_json::Value {
        use serde_json::json;
        if !self.request.as_ref().is_some_and(|(key, _)| (key.model, key.object, key.slot) == (model, binding.0, binding.1) && key.matches_setup(doc))
            || self.pending.is_some() || self.applied_version != Some(version)
        {
            return json!({"status": "pending"});
        }
        if let Some(error) = &self.error {
            return json!({"status": "failed", "error": error});
        }
        self.ready.as_ref().map_or_else(|| json!({"status": "pending"}), |diagnostic| {
            let [covered, tight, missed, overlapping] = diagnostic.counts();
            json!({"status": "ready", "covered": covered, "tight": tight, "missed": missed,
                "overlapping": overlapping, "sampled": diagnostic.sampled})
        })
    }
}

pub(in crate::editor::ui::model_viewer) fn draw(
    mut commands: Commands,
    editor: Res<Editor>,
    editing: Res<editing::State>,
    mut paint: ResMut<PaintState>,
    mut state: ResMut<State>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut lines: Gizmos<ModelGizmos>,
) {
    let active = (|| {
        let model = editor.viewing_model?;
        let doc = editing.documents.get(&model)?;
        let session = paint.sessions.get(&model)?;
        let (object, slot) = session.binding?;
        if doc.mode != Mode::Paint
            || session.panel != Panel::Maps
            || session.view == View::Two
            || !session.bake_inspect_cage
            || session.bake_projection.path.is_none()
            || editor.pending.is_some()
            || editor.viewing_image.is_some()
            || editor.browser_popup
            || editor.hdri_library_open
            || !session.set_appearance.visibility.shown(object, slot)
        {
            return None;
        }
        Some((model, doc, object, slot))
    })();
    let Some((model, doc, object, slot)) = active else {
        state.clear(&mut commands);
        return;
    };
    let session = paint.sessions.get_mut(&model).unwrap();
    let Some(item) = doc
        .objects
        .iter()
        .find(|item| item.id == object && !item.hidden)
    else {
        state.clear(&mut commands);
        return;
    };
    let mut settings = projection_for(session, item);
    settings.fill_misses = false;
    settings.quality = Quality::default();
    let mut key_settings = settings.clone();
    key_settings.manual = None;
    if !state.request.as_ref().is_some_and(|(key, source)| {
        key.model == model
            && key.object == object
            && key.slot == slot
            && *source == key_settings
            && key.matches_setup(doc)
    }) {
        state.clear(&mut commands);
        let Some(key) = Key::new(model, doc, (object, slot), 0) else {
            return;
        };
        state.request = Some((key, key_settings));
        state.status = "Проверка покрытия оболочки…".into();
    }
    let version = session.cage_edit_version;
    if state.pending.is_none()
        && state.applied_version != Some(version)
        && state
            .last_update
            .is_none_or(|time| time.elapsed() >= Duration::from_millis(85))
    {
        let prepared = state.prepared.clone();
        let objects = prepared.is_none().then(|| doc.objects.clone());
        let progress = Arc::new(Progress::default());
        let worker = progress.clone();
        state.last_update = Some(Instant::now());
        state.pending = Some(Job {
            progress,
            version,
            task: AsyncComputeTaskPool::get().spawn(async move {
                let prepared = match prepared {
                    Some(prepared) => prepared,
                    None => Arc::new(mesh::probe::prepare(
                        objects.unwrap(),
                        object,
                        &settings,
                        &worker,
                    )?),
                };
                let diagnostic = prepared.sample(slot, &settings, &worker)?;
                Ok((prepared, diagnostic))
            }),
        });
    }
    if let Some(result) = state
        .pending
        .as_mut()
        .and_then(|job| block_on(poll_once(&mut job.task)))
    {
        state.applied_version = state.pending.take().map(|job| job.version);
        match result {
            Ok((prepared, diagnostic)) => {
                #[cfg(feature = "mcp")]
                {
                    state.error = None;
                }
                state.prepared = Some(prepared);
                if let Some(entity) = state.entity.take() {
                    commands.entity(entity).try_despawn();
                }
                let mut mesh = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                );
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_POSITION,
                    diagnostic
                        .points
                        .iter()
                        .map(|point| point.to_array())
                        .collect::<Vec<_>>(),
                );
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, diagnostic.colors.clone());
                mesh.insert_indices(Indices::U32(diagnostic.indices.clone()));
                let material = state
                    .material
                    .get_or_insert_with(|| {
                        materials.add(StandardMaterial {
                            base_color: Color::linear_rgba(1., 1., 1., 0.32),
                            alpha_mode: AlphaMode::Blend,
                            unlit: true,
                            cull_mode: None,
                            ..default()
                        })
                    })
                    .clone();
                state.entity = Some(
                    commands
                        .spawn((
                            Shell,
                            Mesh3d(meshes.add(mesh)),
                            MeshMaterial3d(material),
                            Transform::IDENTITY,
                            RenderLayers::layer(LAYER),
                            NotShadowCaster,
                            NotShadowReceiver,
                        ))
                        .id(),
                );
                state.status = diagnostic.summary();
                state.ready = Some(diagnostic);
            }
            Err(error) => {
                state.status = format!("Не удалось проверить оболочку · {error}");
                #[cfg(feature = "mcp")]
                {
                    state.error = Some(error);
                }
            }
        }
    }
    if session.bake_cage_status != state.status {
        session.bake_cage_status.clone_from(&state.status);
        session.revision += 1;
    }
    if let Some(diagnostic) = &state.ready {
        let color = |i: usize| {
            let [r, g, b, _] = diagnostic.colors[i];
            Color::linear_rgba(r, g, b, 0.82)
        };
        for &(a, b) in &diagnostic.edges {
            lines.line_gradient(
                diagnostic.points[a],
                diagnostic.points[b],
                color(a),
                color(b),
            );
        }
    }
}
