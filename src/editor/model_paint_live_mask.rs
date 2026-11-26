//! Composes temporary viewport textures without updating the editable paint surface.
use super::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use crate::editor::model_paint_data::{custom::LivePlane, mask::GeneratorSurface};

#[derive(Clone)]
struct Source {
    key: Key,
    surface: Arc<Surface>,
    painted: bool,
    geometry: crate::editor::model_document::MeshData,
    transform: Transform,
    offset: Vec2,
    slot_material: Handle<StandardMaterial>,
    template: Handle<StandardMaterial>,
    preview: crate::editor::model_document::PaintPreview,
    baked_preview: Option<Arc<composite::PreviewBake>>,
}

impl Source {
    fn same(&self, other: &Self) -> bool {
        self.key == other.key
            && self.painted == other.painted
            && (if self.painted { Arc::ptr_eq(&self.surface, &other.surface) } else { self.surface.size == other.surface.size })
            && self.geometry.shares_storage_with(&other.geometry)
            && self.transform == other.transform
            && self.offset == other.offset
            && self.slot_material == other.slot_material
            && self.template == other.template
            && self.preview == other.preview
            && composite::same_preview_bake(self.baked_preview.as_ref(), other.baked_preview.as_ref())
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
                        match &slot.paint {
                            Some(surface) => self.painted && Arc::ptr_eq(surface, &self.surface),
                            None => !self.painted && slot.material == self.slot_material,
                        }
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
    map: bake::BakeMap,
    inputs: Inputs,
    task: Task<Composition>,
}

struct Composition {
    reduced: Arc<Surface>,
    updates: Updates,
    lighting: Option<Arc<composite::PreviewBake>>,
}

struct Display {
    source: Source,
    map: bake::BakeMap,
    inputs: Inputs,
    gpu: Option<Gpu>,
    #[cfg(test)]
    mask_digest: blake3::Hash,
}

#[derive(Clone)]
enum Inputs {
    Custom(Arc<LivePlane>),
    Builtin(Arc<bake::Maps>),
}

struct GraphRenderer {
    surface: material_graph::surface::Source,
    baked: Option<Arc<bake::Maps>>,
    device: RenderDevice,
    queue: RenderQueue,
}

impl GraphRenderer {
    fn refresh(&self, surface: &mut Surface, source: &Inputs) -> bool {
        let (maps, live) = match source {
            Inputs::Builtin(maps) => (Some(maps.as_ref()), None),
            Inputs::Custom(plane) => (self.baked.as_deref(), Some(plane.as_ref())),
        };
        let inputs = material_graph::Inputs { maps, surface: Some(&self.surface), live };
        let graphs = material_graph::affected_layers(&surface.stack, true, false, None);
        let mut changed = false;
        for (index, is_mask, mut graph) in graphs {
            if let Some(plane) = live && !material_graph::mesh_maps::used(&graph).into_iter().any(|value| {
                material_graph::mesh_maps::kind(value) == Some(bake::BakeMap::Custom(plane.id))
            }) {
                continue;
            }
            // This render belongs to the temporary preview stack, at its own resolution.
            graph.match_set = true;
            if let Err(error) = material_graph::refresh_layer(
                &mut surface.stack.layers[index], is_mask, graph, surface.size, inputs,
                &self.device, &self.queue,
            ) {
                warn!("Live paint graph preview failed: {error}");
            } else {
                changed = true;
            }
        }
        changed
    }
}

impl Inputs {
    fn size(&self) -> u32 {
        match self {
            Self::Custom(plane) => plane.size,
            Self::Builtin(inputs) => inputs.size,
        }
    }

    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Custom(a), Self::Custom(b)) => Arc::ptr_eq(a, b),
            (Self::Builtin(a), Self::Builtin(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

#[derive(Default)]
pub(in crate::editor::ui::model_viewer::paint) struct State {
    prepared: Option<Preparation>,
    running: Option<Job>,
    displayed: Option<Display>,
}

fn composition_map(map: bake::BakeMap) -> bake::BakeMap {
    match map {
        bake::BakeMap::Custom(_) => map,
        _ => bake::BakeMap::WorldNormal,
    }
}

impl State {
    #[cfg(test)]
    pub(in crate::editor::ui::model_viewer::paint) fn mask_digest(
        &self,
    ) -> Option<(u32, blake3::Hash)> {
        self.displayed
            .as_ref()
            .map(|display| (display.inputs.size(), display.mask_digest))
    }

    fn display(&self, model: usize, doc: &Document, session: &Session) -> Option<&Display> {
        let binding = session.binding?;
        let map = composition_map(session.live_mask_map()?);
        self.displayed.as_ref().filter(|display| {
            display.map == map
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
    inputs: Inputs,
    prepared: Option<Arc<Surface>>,
    renderer: Option<GraphRenderer>,
) -> Composition {
    let (_, _, slot) = source.key;
    let reduced = prepared.unwrap_or_else(|| {
        let mut surface = (*source.surface).clone();
        surface.resize(inputs.size());
        surface.bind_texture_fills(&source.geometry, slot, source.offset, source.transform);
        surface.bind_generators(&source.geometry, slot, source.offset, source.transform);
        Arc::new(surface)
    });
    let mut surface = (*reduced).clone();
    if let Some(renderer) = renderer && renderer.refresh(&mut surface, &inputs) {
        surface.bind_texture_fills(&source.geometry, slot, source.offset, source.transform);
    }
    let lighting = match &inputs {
        Inputs::Builtin(maps) if composite::PreviewBake::available(maps, source.preview) => {
            Some(Arc::new(composite::PreviewBake::new(maps.clone(), source.preview)))
        }
        _ => source.baked_preview.clone(),
    };
    match inputs {
        Inputs::Builtin(maps) => {
            surface.baked = Some(maps.generator_inputs().clone());
            surface.bind_generators(&source.geometry, slot, source.offset, source.transform);
        }
        Inputs::Custom(plane) => {
            let id = plane.id;
            let context = Arc::new(GeneratorSurface::new(
                source.geometry.clone(), slot, surface.size, source.offset, source.transform,
                surface.baked.clone(),
            ).with_live(plane));
            for mask in surface.stack.layers.iter_mut().filter_map(|layer| layer.mask.as_mut()) {
                if mask.uses_custom_map(id) {
                    Arc::make_mut(mask).bind(context.clone());
                }
            }
        }
    }
    let updates = compose_maps(&surface, source.preview, lighting.as_deref());
    Composition { reduced, updates, lighting }
}

pub(in crate::editor::ui::model_viewer) fn update(
    editor: Res<Editor>,
    editing: Res<editing::State>,
    paint: Res<PaintState>,
    runtime: Res<Runtime>,
    mut baked: ResMut<bake::State>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut uploads: ResMut<crate::editor::image_texture::TextureUploads>,
    device: Option<Res<RenderDevice>>,
    queue: Option<Res<RenderQueue>>,
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
        let map = session.live_mask_map()?;
        let binding = session.binding?;
        if !session
            .set_appearance
            .visibility
            .shown(binding.0, binding.1)
        {
            return None;
        }
        let object = doc
            .objects
            .iter()
            .find(|object| object.id == binding.0 && !object.hidden)?;
        let slot = object.materials.get(binding.1)?;
        let key = (model, binding.0, binding.1);
        let live = runtime.lives.get(&key);
        let painted = slot.paint.is_some();
        let surface = slot.paint.clone().or_else(|| {
            let live = live?;
            if live.published.is_some() { return None }
            Some(Arc::new(live.work.as_ref()?.surface.clone()))
        })?;
        let inputs = match map {
            bake::BakeMap::Custom(id) => {
                if !material_graph::has_graph_layers(&surface.stack)
                    && !surface.stack.layers.iter().filter_map(|layer| layer.mask.as_ref())
                    .any(|mask| mask.uses_custom_map(id))
                {
                    return None;
                }
                Inputs::Custom(baked.live.plane_for(model, doc, binding, id)?.clone())
            }
            _ => {
                let maps = baked.live_inputs_for(model, doc, binding, map)?;
                if !surface.needs_bake() && !material_graph::has_graph_layers(&surface.stack)
                    && !composite::PreviewBake::available(maps, doc.paint_preview)
                {
                    return None;
                }
                Inputs::Builtin(maps.clone())
            }
        };
        Some((
            Source {
                key,
                surface,
                painted,
                geometry: object.geometry.clone(),
                transform: object.transform,
                offset: object.uv_offset(binding.1),
                slot_material: slot.material.clone(),
                template: live.map_or_else(|| slot.material.clone(), |live| live.origin.material.clone()),
                preview: doc.paint_preview,
                baked_preview: live.and_then(|live| live.gpu.as_ref()?.baked_preview.clone()),
            },
            composition_map(map),
            inputs,
        ))
    })();
    let graph_bake = active.as_ref().and_then(|(source, _, inputs)| {
        if let Inputs::Custom(_) = inputs {
            let (model, object, slot) = source.key;
            let session = paint.sessions.get(&model)?;
            baked.maps(model, editing.documents.get(&model)?, (object, slot), session.texture_size).cloned()
        } else {
            None
        }
    });
    let state = &mut baked.masks;
    let Some((source, map, inputs)) = active else {
        *state = State::default();
        return;
    };
    if state
        .displayed
        .as_ref()
        .is_some_and(|display| !display.source.same(&source) || display.map != map)
    {
        state.displayed = None;
    }
    if state
        .running
        .as_ref()
        .is_some_and(|job| !job.source.same(&source) || job.map != map)
    {
        state.running = None;
    }
    if let Some(job) = &mut state.running
        && let Some(Composition { reduced, updates, lighting }) = block_on(poll_once(&mut job.task))
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
            .filter(|display| display.inputs.size() == job.inputs.size())
            .and_then(|display| display.gpu);
        upload_textures(
            &mut gpu,
            updates,
            &source.template,
            source.preview.emission_strength,
            lighting.as_ref(),
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
            map,
            inputs: job.inputs,
            gpu,
            #[cfg(test)]
            mask_digest,
        });
    }
    if state.running.is_none()
        && state
            .displayed
            .as_ref()
            .is_none_or(|display| !display.inputs.same(&inputs))
    {
        let reduced = state
            .prepared
            .as_ref()
            .filter(|prepared| prepared.source.same(&source) && prepared.reduced.size == inputs.size())
            .map(|prepared| prepared.reduced.clone());
        let input = source.clone();
        let sample = inputs.clone();
        let renderer = if material_graph::has_graph_layers(&source.surface.stack) {
            let (model, object, slot) = source.key;
            device.as_deref().zip(queue.as_deref()).and_then(|(device, queue)| {
                let surface = material_graph::surface::Source::capture(&editing.documents[&model], (object, slot))?;
                Some(GraphRenderer { surface, baked: graph_bake, device: device.clone(), queue: queue.clone() })
            })
        } else {
            None
        };
        let task = AsyncComputeTaskPool::get().spawn(async move { compose_preview(&input, sample, reduced, renderer) });
        state.running = Some(Job {
            source,
            map,
            inputs,
            task,
        });
    }
}
