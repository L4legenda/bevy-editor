//! Composes temporary viewport textures without updating the editable paint surface.
use super::*;
use crate::editor::model_paint_data::{custom::LivePlane, mask::GeneratorSurface};

#[derive(Clone)]
struct Source {
    key: Key,
    surface: Arc<Surface>,
    geometry: crate::editor::model_document::MeshData,
    transform: Transform,
    offset: Vec2,
    template: Handle<StandardMaterial>,
}

impl Source {
    fn same(&self, other: &Self) -> bool {
        self.key == other.key
            && Arc::ptr_eq(&self.surface, &other.surface)
            && self.geometry.shares_storage_with(&other.geometry)
            && self.transform == other.transform
            && self.offset == other.offset
            && self.template == other.template
    }

    fn current(&self, model: usize, doc: &Document, binding: (usize, usize)) -> bool {
        if self.key != (model, binding.0, binding.1) {
            return false;
        }
        doc.objects
            .iter()
            .find(|object| object.id == binding.0 && !object.hidden)
            .is_some_and(|object| {
                object.geometry.shares_storage_with(&self.geometry)
                    && object.transform == self.transform
                    && object.uv_offset(binding.1) == self.offset
                    && object.materials.get(binding.1).is_some_and(|slot| {
                        slot.paint
                            .as_ref()
                            .is_some_and(|surface| Arc::ptr_eq(surface, &self.surface))
                    })
            })
    }
}

struct Preparation {
    source: Source,
    reduced: Arc<Surface>,
}

struct Job {
    source: Source,
    plane: Arc<LivePlane>,
    task: Task<(Arc<Surface>, Updates)>,
}

struct Display {
    source: Source,
    plane: Arc<LivePlane>,
    gpu: Option<Gpu>,
    #[cfg(test)]
    mask_digest: blake3::Hash,
}

#[derive(Default)]
pub(in crate::editor::ui::model_viewer::paint) struct State {
    prepared: Option<Preparation>,
    running: Option<Job>,
    displayed: Option<Display>,
}

fn selected_map(session: &Session) -> Option<u64> {
    if session.bake_live_masks
        && session.bake_live
        && session.showing_bake()
        && let Some(bake::BakeMap::Custom(id)) = session.bake_view
    {
        Some(id)
    } else {
        None
    }
}

impl State {
    #[cfg(test)]
    pub(in crate::editor::ui::model_viewer::paint) fn mask_digest(
        &self,
    ) -> Option<(u32, blake3::Hash)> {
        self.displayed
            .as_ref()
            .map(|display| (display.plane.size, display.mask_digest))
    }

    fn display(&self, model: usize, doc: &Document, session: &Session) -> Option<&Display> {
        let binding = session.binding?;
        let id = selected_map(session)?;
        self.displayed.as_ref().filter(|display| {
            display.plane.id == id
                && session
                    .set_appearance
                    .visibility
                    .shown(binding.0, binding.1)
                && display.source.current(model, doc, binding)
        })
    }

    pub(in crate::editor::ui::model_viewer::paint) fn material(
        &self,
        model: usize,
        doc: &Document,
        session: &Session,
    ) -> Option<Handle<StandardMaterial>> {
        let display = self.display(model, doc, session)?;
        Some(display.gpu.as_ref()?.material.clone())
    }

    pub(in crate::editor::ui::model_viewer::paint) fn mask(
        &self,
        model: usize,
        doc: &Document,
        session: &Session,
    ) -> Option<Handle<Image>> {
        let display = self.display(model, doc, session)?;
        Some(display.gpu.as_ref()?.images[composite::MASK_PREVIEW].clone())
    }
}

fn compose_preview(
    source: &Source,
    plane: Arc<LivePlane>,
    prepared: Option<Arc<Surface>>,
) -> (Arc<Surface>, Updates) {
    let (_, _, slot) = source.key;
    let reduced = prepared.unwrap_or_else(|| {
        let mut surface = (*source.surface).clone();
        surface.resize(plane.size);
        surface.bind_texture_fills(&source.geometry, slot, source.offset, source.transform);
        surface.bind_generators(&source.geometry, slot, source.offset, source.transform);
        Arc::new(surface)
    });
    let mut surface = (*reduced).clone();
    let id = plane.id;
    let context = Arc::new(
        GeneratorSurface::new(
            source.geometry.clone(),
            slot,
            surface.size,
            source.offset,
            source.transform,
            surface.baked.clone(),
        )
        .with_live(plane),
    );
    for mask in surface
        .stack
        .layers
        .iter_mut()
        .filter_map(|layer| layer.mask.as_mut())
    {
        if mask.uses_custom_map(id) {
            Arc::make_mut(mask).bind(context.clone());
        }
    }
    (reduced, compose_maps(&surface))
}

pub(in crate::editor::ui::model_viewer) fn update(
    editor: Res<Editor>,
    editing: Res<editing::State>,
    paint: Res<PaintState>,
    mut baked: ResMut<bake::State>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut uploads: ResMut<crate::editor::image_texture::TextureUploads>,
) {
    let active = (|| {
        if editor.viewing_image.is_some() || editor.pending.is_some() {
            return None;
        }
        let model = editor.viewing_model?;
        let doc = editing
            .documents
            .get(&model)
            .filter(|doc| doc.mode == Mode::Paint)?;
        let session = paint.sessions.get(&model)?;
        let id = selected_map(session)?;
        let binding = session.binding?;
        if !session
            .set_appearance
            .visibility
            .shown(binding.0, binding.1)
        {
            return None;
        }
        let plane = baked.live.plane_for(model, doc, binding, id)?.clone();
        let object = doc
            .objects
            .iter()
            .find(|object| object.id == binding.0 && !object.hidden)?;
        let slot = object.materials.get(binding.1)?;
        let surface = slot.paint.as_ref()?;
        if !surface
            .stack
            .layers
            .iter()
            .filter_map(|layer| layer.mask.as_ref())
            .any(|mask| mask.uses_custom_map(id))
        {
            return None;
        }
        Some((
            Source {
                key: (model, binding.0, binding.1),
                surface: surface.clone(),
                geometry: object.geometry.clone(),
                transform: object.transform,
                offset: object.uv_offset(binding.1),
                template: slot.material.clone(),
            },
            plane,
        ))
    })();
    let state = &mut baked.masks;
    let Some((source, plane)) = active else {
        *state = State::default();
        return;
    };
    if state
        .displayed
        .as_ref()
        .is_some_and(|display| !display.source.same(&source) || display.plane.id != plane.id)
    {
        state.displayed = None;
    }
    if state
        .running
        .as_ref()
        .is_some_and(|job| !job.source.same(&source) || job.plane.id != plane.id)
    {
        state.running = None;
    }
    if let Some(job) = &mut state.running
        && let Some((reduced, updates)) = block_on(poll_once(&mut job.task))
    {
        let job = state.running.take().unwrap();
        #[cfg(test)]
        let mask_digest = blake3::hash(
            &updates.textures[composite::MASK_PREVIEW]
                .as_ref()
                .unwrap()
                .rgba,
        );
        let mut gpu = state
            .displayed
            .take()
            .filter(|display| display.plane.size == job.plane.size)
            .and_then(|display| display.gpu);
        upload_textures(
            &mut gpu,
            updates,
            &source.template,
            &mut images,
            &mut materials,
            &mut uploads,
        );
        state.prepared = Some(Preparation {
            source: source.clone(),
            reduced,
        });
        state.displayed = Some(Display {
            source: source.clone(),
            plane: job.plane,
            gpu,
            #[cfg(test)]
            mask_digest,
        });
    }
    if state.running.is_none()
        && state
            .displayed
            .as_ref()
            .is_none_or(|display| !Arc::ptr_eq(&display.plane, &plane))
    {
        let reduced = state
            .prepared
            .as_ref()
            .filter(|prepared| prepared.source.same(&source) && prepared.reduced.size == plane.size)
            .map(|prepared| prepared.reduced.clone());
        let input = source.clone();
        let map = plane.clone();
        let task =
            AsyncComputeTaskPool::get().spawn(async move { compose_preview(&input, map, reduced) });
        state.running = Some(Job {
            source,
            plane,
            task,
        });
    }
}
