//! Cancellable previews for the selected custom bake map.
use super::*;
use std::time::{Duration, Instant};

const QUICK: u32 = 96;
const REFINED: u32 = 256;
const SETTLE: Duration = Duration::from_millis(180);

struct Work<T> {
    progress: Arc<Progress>,
    task: Task<Result<T, String>>,
}

impl<T: Send + 'static> Work<T> {
    fn spawn(compute: impl FnOnce(&Progress) -> Result<T, String> + Send + 'static) -> Self {
        let progress = Arc::new(Progress::default());
        let worker = progress.clone();
        let task = AsyncComputeTaskPool::get().spawn(async move { compute(&worker) });
        Self { progress, task }
    }

    fn poll(&mut self) -> Option<Result<T, String>> {
        block_on(poll_once(&mut self.task))
    }
}

impl<T> Drop for Work<T> {
    fn drop(&mut self) {
        self.progress.cancelled.store(true, Ordering::Relaxed);
    }
}

struct Geometry {
    key: Key,
    settings: ProjectionSettings,
    work: Option<Work<mesh::live::Scene>>,
    ready: Option<Arc<mesh::live::Scene>>,
    error: Option<String>,
}

struct Preview {
    recipe: custom::Recipe,
    size: u32,
    image: Handle<Image>,
    plane: Arc<custom::LivePlane>,
    gpu: bool,
}

#[derive(Default)]
pub(in crate::editor::ui::model_viewer::paint) struct State {
    geometry: Option<Geometry>,
    wanted: Option<(custom::Recipe, u32, Quality, Instant)>,
    running: Option<(custom::Recipe, u32, Work<(Vec<u8>, bool)>)>,
    shown: Option<Preview>,
    failed: Option<(custom::Recipe, String)>,
}

impl State {
    #[cfg(test)]
    pub(in crate::editor::ui::model_viewer::paint) fn uses_gpu(&self, id: u64) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|preview| preview.recipe.id == id && preview.gpu)
    }

    pub fn image(&self, id: u64) -> Option<&Handle<Image>> {
        self.shown
            .as_ref()
            .filter(|preview| preview.recipe.id == id)
            .map(|preview| &preview.image)
    }

    pub(super) fn image_for(
        &self,
        model: usize,
        doc: &Document,
        (object, slot): (usize, usize),
        id: u64,
    ) -> Option<&Handle<Image>> {
        self.scoped_preview(model, doc, (object, slot), id)
            .map(|preview| &preview.image)
    }

    pub(in crate::editor::ui::model_viewer::paint) fn plane_for(
        &self,
        model: usize,
        doc: &Document,
        binding: (usize, usize),
        id: u64,
    ) -> Option<&Arc<custom::LivePlane>> {
        self.scoped_preview(model, doc, binding, id)
            .map(|preview| &preview.plane)
    }

    fn scoped_preview(
        &self,
        model: usize,
        doc: &Document,
        (object, slot): (usize, usize),
        id: u64,
    ) -> Option<&Preview> {
        let scene = self.geometry.as_ref()?;
        if (scene.key.model, scene.key.object, scene.key.slot) != (model, object, slot)
            || !scene.key.matches_geometry(doc)
        {
            return None;
        }
        self.shown
            .as_ref()
            .filter(|preview| preview.recipe.id == id)
    }

    pub fn status(&self, id: u64) -> Option<String> {
        if self
            .wanted
            .as_ref()
            .is_none_or(|(recipe, ..)| recipe.id != id)
        {
            return None;
        }
        if let Some(error) = self
            .geometry
            .as_ref()
            .and_then(|geometry| geometry.error.as_ref())
            .or_else(|| self.failed.as_ref().map(|(_, error)| error))
        {
            return Some(format!("Предпросмотр недоступен · {error}"));
        }
        if let Some(preview) = &self.shown {
            let size = preview.size;
            let device = if preview.gpu { " · GPU" } else { "" };
            return Some(format!(
                "Предпросмотр {size} × {size}{device} · {}",
                preview.recipe.name
            ));
        }
        Some("Расчёт предпросмотра…".into())
    }

    fn hide(&mut self) {
        self.wanted = None;
        self.running = None;
        self.shown = None;
        self.failed = None;
    }

    fn tick(
        &mut self,
        request: Request,
        doc: &Document,
        images: &mut Assets<Image>,
        device: Option<mesh::RayDevice>,
    ) {
        if self.geometry.as_ref().is_none_or(|geometry| {
            geometry.settings != request.projection
                || (geometry.key.model, geometry.key.object, geometry.key.slot)
                    != (request.model, request.object, request.slot)
                || !geometry.key.matches_geometry(doc)
        }) {
            self.hide();
            let Some(mut key) = Key::new(
                request.model,
                doc,
                (request.object, request.slot),
                request.size,
            ) else {
                self.geometry = None;
                return;
            };
            key.custom = Arc::default();
            let objects = doc.objects.clone();
            let settings = request.projection.clone();
            let (object, slot) = (request.object, request.slot);
            self.geometry = Some(Geometry {
                key,
                settings: request.projection,
                work: Some(Work::spawn(move |progress| {
                    mesh::live::Scene::prepare(objects, object, slot, &settings, progress)
                })),
                ready: None,
                error: None,
            });
        }
        let recipe = request.recipe;
        if self.wanted.as_ref().is_some_and(|(_, size, quality, _)| {
            *size != request.size || *quality != request.quality
        }) {
            self.hide();
        }
        if self
            .wanted
            .as_ref()
            .is_none_or(|(prior, ..)| !prior.same_bake(&recipe))
        {
            let quick = QUICK.min(request.size);
            if self
                .running
                .as_ref()
                .is_some_and(|(prior, size, _)| prior.id != recipe.id || *size > quick)
            {
                self.running = None;
            }
            self.shown = self.shown.take().filter(|old| old.recipe.id == recipe.id);
            self.failed = None;
            self.wanted = Some((
                recipe.clone(),
                request.size,
                request.quality,
                Instant::now(),
            ));
        }
        if let Some(preview) = &mut self.shown
            && preview.recipe.id == recipe.id
        {
            preview.recipe.name.clone_from(&recipe.name);
        }
        let geometry = self.geometry.as_mut().unwrap();
        if let Some(result) = geometry.work.as_mut().and_then(Work::poll) {
            geometry.work = None;
            match result {
                Ok(scene) => geometry.ready = Some(Arc::new(scene)),
                Err(error) => geometry.error = Some(error),
            }
        }
        let Some(scene) = &geometry.ready else { return };
        if let Some((baked, size, worker)) = &mut self.running
            && let Some(result) = worker.poll()
        {
            match result {
                Ok((pixels, gpu)) => {
                    let plane = Arc::new(custom::LivePlane {
                        id: baked.id,
                        size: *size,
                        values: pixels.chunks_exact(4).map(|pixel| pixel[0]).collect(),
                    });
                    let image = image::RgbaImage::from_raw(*size, *size, pixels).unwrap();
                    let image = images.add(Image::from_dynamic(
                        image::DynamicImage::ImageRgba8(image),
                        true,
                        RenderAssetUsages::default(),
                    ));
                    let mut baked = baked.clone();
                    baked.name.clone_from(&recipe.name);
                    self.shown = Some(Preview {
                        image,
                        recipe: baked,
                        size: *size,
                        plane,
                        gpu,
                    });
                }
                Err(error) if baked.same_bake(&recipe) => {
                    self.failed = Some((baked.clone(), error))
                }
                Err(_) => {}
            }
            self.running = None;
        }
        if self.running.is_some() || self.failed.is_some() {
            return;
        }
        let quick = QUICK.min(request.size);
        let refined = REFINED.min(request.size);
        let current = self
            .shown
            .as_ref()
            .filter(|preview| preview.recipe.same_bake(&recipe));
        let size = if current.is_none() {
            quick
        } else if current.is_some_and(|preview| preview.size < refined)
            && self
                .wanted
                .as_ref()
                .is_some_and(|(.., changed)| changed.elapsed() >= SETTLE)
        {
            refined
        } else {
            return;
        };
        let scene = scene.clone();
        let quality = if size == quick {
            Quality::Draft
        } else {
            request.quality
        };
        let input = recipe.clone();
        self.running = Some((
            recipe,
            size,
            Work::spawn(move |progress| {
                scene.shade_on_device(&input, size, quality, progress, device.as_ref())
            }),
        ));
    }
}

struct Request {
    model: usize,
    object: usize,
    slot: usize,
    size: u32,
    recipe: custom::Recipe,
    quality: Quality,
    projection: ProjectionSettings,
}

pub(in crate::editor::ui::model_viewer) fn update(
    editor: Res<Editor>,
    editing: Res<editing::State>,
    paint: Res<PaintState>,
    gizmo: Res<custom_gizmo::Gizmo>,
    mut baked: ResMut<super::State>,
    mut images: ResMut<Assets<Image>>,
    device: Option<Res<RenderDevice>>,
    queue: Option<Res<RenderQueue>>,
) {
    let active = (|| {
        if editor.pending.is_some() || editor.viewing_image.is_some() {
            return None;
        }
        let model = editor.viewing_model?;
        let doc = editing.documents.get(&model)?;
        let session = paint.sessions.get(&model)?;
        let Some(BakeMap::Custom(id)) = session.bake_view else {
            return None;
        };
        if doc.mode != Mode::Paint
            || session.panel != Panel::Maps
            || (session.view == View::Three && !session.bake_on_model && !session.bake_live_masks)
            || !session.bake_live
            || session.bake_picking
        {
            return None;
        }
        let recipe = doc.custom_bakes.iter().find(|recipe| recipe.id == id)?;
        let (object, slot) = session.binding?;
        let target = doc
            .objects
            .iter()
            .find(|candidate| candidate.id == object && !candidate.hidden)?;
        let draft = gizmo.draft_for(model, id, doc.revision).unwrap_or(recipe);
        let full = draft.same_bake(recipe)
            && baked
                .maps(model, doc, (object, slot), session.texture_size)
                .is_some_and(|maps| maps.has_map(BakeMap::Custom(id)));
        let mut projection = projection_for(session, target);
        let quality = projection.quality;
        projection.quality = Quality::Draft;
        projection.maps = MapSelection::ALL;
        Some((
            doc,
            full,
            Request {
                model,
                object,
                slot,
                size: session.texture_size,
                recipe: draft.clone(),
                quality,
                projection,
            },
        ))
    })();
    let Some((doc, full, request)) = active else {
        baked.live = State::default();
        return;
    };
    if full || baked.running.is_some() {
        let id = request.recipe.id;
        let final_visible = baked
            .preview
            .as_ref()
            .is_some_and(|(_, map, _)| *map == BakeMap::Custom(id));
        let same_set = baked.live.geometry.as_ref().is_some_and(|scene| {
            (scene.key.model, scene.key.object, scene.key.slot)
                == (request.model, request.object, request.slot)
                && scene.key.matches_geometry(doc)
        });
        baked.live.running = None;
        if final_visible || !same_set || baked.live.image(id).is_none() {
            baked.live.hide();
        }
    } else {
        let gpu = device
            .as_deref()
            .zip(queue.as_deref())
            .map(|(device, queue)| mesh::RayDevice {
                device: device.clone(),
                queue: queue.clone(),
            });
        baked.live.tick(request, doc, &mut images, gpu);
    }
}
