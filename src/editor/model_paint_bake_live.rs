//! Cancellable previews for the selected bake map.
use super::*;
use std::time::{Duration, Instant};

const QUICK: u32 = 96;
const REFINED: u32 = 256;
const BUILTIN_SETTLE: Duration = Duration::from_millis(60);
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

pub(super) struct BuiltinPreview {
    key: Key,
    map: BakeMap,
    settings: ProjectionSettings,
    include_inputs: bool,
    changed: Instant,
    running: Option<(u32, Work<BuiltinFrame>)>,
    shown: Option<(u32, Handle<Image>, Option<Arc<Maps>>)>,
    shown_current: bool,
    error: Option<String>,
}

struct BuiltinFrame {
    map: BakeMap,
    pixels: Vec<u8>,
    inputs: Option<Arc<Maps>>,
}

impl BuiltinPreview {
    fn image(size: u32, pixels: Vec<u8>, images: &mut Assets<Image>) -> Handle<Image> {
        let image = image::RgbaImage::from_raw(size, size, pixels).unwrap();
        images.add(Image::from_dynamic(
            image::DynamicImage::ImageRgba8(image), true, RenderAssetUsages::default(),
        ))
    }

    fn select(&mut self, map: BakeMap, images: &mut Assets<Image>) {
        if self.map != map {
            self.map = map;
            if let Some((size, image, Some(inputs))) = &mut self.shown {
                *image = Self::image(*size, inputs.rgba(map), images);
            }
        }
    }

    fn keep_image(
        mut self, model: usize, doc: &Document, binding: (usize, usize), map: BakeMap,
        include_inputs: bool, images: &mut Assets<Image>,
    ) -> Option<(u32, Handle<Image>, Option<Arc<Maps>>)> {
        if !self.matches_target(model, doc, binding) || self.include_inputs != include_inputs
            || (self.map != map && !self.shown.as_ref().and_then(|(_, _, inputs)| inputs.as_ref())
                .is_some_and(|maps| maps.has_map(map)))
        {
            return None;
        }
        self.select(map, images);
        self.shown.take()
    }

    pub(super) fn image_for(
        &self, model: usize, doc: &Document, binding: (usize, usize), map: BakeMap,
    ) -> Option<&Handle<Image>> {
        if self.map != map || !self.matches_target(model, doc, binding) {
            return None;
        }
        self.shown.as_ref().map(|(_, image, _)| image)
    }

    pub(super) fn inputs_for(
        &self, model: usize, doc: &Document, binding: (usize, usize), map: BakeMap,
    ) -> Option<&Arc<Maps>> {
        if self.map != map || !self.matches_target(model, doc, binding) {
            return None;
        }
        self.shown.as_ref()?.2.as_ref()
    }

    pub(super) fn status(&self, map: BakeMap) -> Option<String> {
        (self.map == map).then(|| {
            if let Some(error) = &self.error {
                if self.shown.is_some() {
                    format!("Не удалось обновить предпросмотр · {error}")
                } else {
                    format!("Предпросмотр недоступен · {error}")
                }
            } else if !self.shown_current && self.shown.is_some() {
                format!("Обновление предпросмотра… · {}", map.label())
            } else if let Some((size, ..)) = &self.shown {
                format!("Предпросмотр {size} × {size} · {}", map.label())
            } else {
                "Расчёт предпросмотра…".into()
            }
        })
    }

    fn matches_target(&self, model: usize, doc: &Document, binding: (usize, usize)) -> bool {
        (self.key.model, self.key.object, self.key.slot) == (model, binding.0, binding.1)
            && self.key.matches_geometry(doc)
    }

    fn tick(&mut self, doc: &Document, images: &mut Assets<Image>, device: Option<mesh::RayDevice>) {
        if let Some((size, work)) = &mut self.running
            && let Some(result) = work.poll()
        {
            match result {
                Ok(BuiltinFrame { map, pixels, inputs }) => {
                    let pixels = if map != self.map && let Some(inputs) = &inputs {
                        inputs.rgba(self.map)
                    } else {
                        pixels
                    };
                    self.shown = Some((*size, Self::image(*size, pixels, images), inputs));
                    self.shown_current = true;
                }
                Err(error) => self.error = Some(error),
            }
            self.running = None;
        }
        if self.running.is_some() || self.error.is_some() {
            return;
        }
        let quick = QUICK.min(self.key.size);
        let refined = REFINED.min(self.key.size);
        let size = if !self.shown_current && self.changed.elapsed() >= BUILTIN_SETTLE {
            quick
        } else if self.shown_current && self.shown.as_ref().is_some_and(|(size, ..)| *size < refined)
            && self.changed.elapsed() >= SETTLE
        {
            refined
        } else {
            return;
        };
        let mut settings = self.settings.clone();
        if size == quick {
            settings.quality = Quality::Draft;
        }
        let objects = doc.objects.clone();
        let (object, slot, map) = (self.key.object, self.key.slot, self.map);
        let include_inputs = self.include_inputs;
        self.running = Some((size, Work::spawn(move |progress| {
            let maps = mesh::bake_on_device(objects, object, slot, size, &settings, progress, device.as_ref())?;
            if !maps.has_map(map) {
                return Err(format!("Карта «{}» недоступна для этой модели", map.label()));
            }
            Ok(BuiltinFrame {
                map,
                pixels: maps.rgba(map),
                inputs: include_inputs.then(|| Arc::new(Maps::new(Arc::new(maps), None))),
            })
        })));
    }
}

fn builtin_projection(
    mut projection: ProjectionSettings, doc: &Document, map: BakeMap, include_inputs: bool,
) -> ProjectionSettings {
    if include_inputs {
        projection.custom = doc.custom_bakes.clone();
    } else {
        projection.maps = MapSelection::MINIMUM.toggle(map);
        projection.custom = Arc::default();
    }
    projection
}

#[cfg(feature = "mcp")]
pub(in crate::editor::ui::model_viewer::paint) fn progress<'a>(
    baked: &'a super::State, model: usize, doc: &Document, session: &Session, map: BakeMap,
) -> (&'static str, Option<u32>, Option<&'a str>) {
    let unavailable = ("unavailable", None, None);
    let pending = ("pending", None, None);
    let Some(binding) = session.binding.filter(|&(object, slot)| {
        !session.bake_picking && session.set_appearance.visibility.shown(object, slot)
    }) else {
        return unavailable;
    };
    let Some(target) = doc.objects.iter().find(|object| object.id == binding.0 && !object.hidden) else {
        return unavailable;
    };
    let full = || baked.maps(model, doc, binding, session.texture_size)
        .is_some_and(|maps| maps.has_map(map));
    let baked_status = ("baked", Some(session.texture_size), None);
    let mut projection = projection_for(session, target);
    let (displayed, current, error) = if let BakeMap::Custom(id) = map {
        let Some(recipe) = doc.custom_bakes.iter().find(|recipe| recipe.id == id) else {
            return unavailable;
        };
        let live = &baked.live;
        let quality = projection.quality;
        projection.quality = Quality::Draft;
        projection.maps = MapSelection::ALL;
        let active = live.geometry.as_ref().filter(|scene| {
            (scene.key.model, scene.key.object, scene.key.slot) == (model, binding.0, binding.1)
                && scene.key.matches_geometry(doc) && scene.settings == projection
        }).zip(live.wanted.as_ref()).filter(|(_, (wanted, size, wanted_quality, _))| {
            wanted.same_bake(recipe) && *size == session.texture_size && *wanted_quality == quality
        });
        let Some((scene, _)) = active else {
            return if full() { baked_status } else { pending };
        };
        let shown = live.scoped_preview(model, doc, binding, id);
        if full() && shown.is_none() {
            return baked_status;
        }
        (shown.map(|item| item.size), shown.is_some_and(|item| item.recipe.same_bake(recipe)),
            scene.error.as_deref().or_else(|| live.failed.as_ref()
                .filter(|(failed, _)| failed.same_bake(recipe)).map(|(_, error)| error.as_str())))
    } else {
        if full() {
            return baked_status;
        }
        if !projection.maps.contains(map) || (map.requires_high_poly() && projection.path.is_none()) {
            return unavailable;
        }
        let include_inputs = session.live_mask_map().is_some();
        let projection = builtin_projection(projection, doc, map, include_inputs);
        let Some(preview) = baked.builtin_live.as_ref().filter(|preview| {
            preview.map == map && preview.key.size == session.texture_size
                && preview.matches_target(model, doc, binding)
                && preview.include_inputs == include_inputs && preview.settings == projection
        }) else {
            return pending;
        };
        (preview.shown.as_ref().map(|(size, ..)| *size), preview.shown_current, preview.error.as_deref())
    };
    let status = if error.is_some() {
        "failed"
    } else if current {
        "ready"
    } else {
        "pending"
    };
    (status, displayed, error)
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
        let map = session.bake_view.or_else(|| session.live_mask_map())?;
        if doc.mode != Mode::Paint
            || session.panel != Panel::Maps
            || (session.view == View::Three && !session.bake_on_model && !session.bake_live_masks)
            || !session.bake_live
            || session.bake_picking
        {
            return None;
        }
        let (object, slot) = session.binding?;
        let target = doc
            .objects
            .iter()
            .find(|candidate| candidate.id == object && !candidate.hidden)?;
        Some((model, doc, session, map, (object, slot), projection_for(session, target)))
    })();
    let Some((model, doc, session, map, binding, mut projection)) = active else {
        baked.live = State::default();
        baked.builtin_live = None;
        return;
    };
    let full = baked.maps(model, doc, binding, session.texture_size).is_some_and(|maps| maps.has_map(map));
    let BakeMap::Custom(id) = map else {
        baked.live = State::default();
        if full || !session.bake_projection.maps.contains(map)
            || (map.requires_high_poly() && projection.path.is_none())
        {
            baked.builtin_live = None;
            return;
        }
        let include_inputs = session.live_mask_map().is_some();
        let projection = builtin_projection(projection, doc, map, include_inputs);
        if baked.builtin_live.as_ref().is_none_or(|preview| {
            (!include_inputs && preview.map != map) || preview.key.size != session.texture_size || preview.settings != projection
                || preview.include_inputs != include_inputs
                || !preview.matches_target(model, doc, binding)
        }) {
            let shown = baked.builtin_live.take().and_then(|preview| {
                preview.keep_image(model, doc, binding, map, include_inputs, &mut images)
            });
            baked.builtin_live = Key::new(model, doc, binding, session.texture_size).map(|key| BuiltinPreview {
                key, map, settings: projection, include_inputs, changed: Instant::now(),
                running: None, shown, shown_current: false, error: None,
            });
        }
        if baked.running.is_some() {
            if let Some(preview) = &mut baked.builtin_live {
                preview.running = None;
            }
        } else {
            let gpu = baked.ray_device(device.as_deref(), queue.as_deref());
            if let Some(preview) = &mut baked.builtin_live {
                preview.select(map, &mut images);
                preview.tick(doc, &mut images, gpu);
            }
        }
        return;
    };
    baked.builtin_live = None;
    let Some(recipe) = doc.custom_bakes.iter().find(|recipe| recipe.id == id) else {
        baked.live = State::default();
        return;
    };
    let draft = gizmo.draft_for(model, id, doc.revision).unwrap_or(recipe);
    let full = full && draft.same_bake(recipe);
    let quality = projection.quality;
    projection.quality = Quality::Draft;
    projection.maps = MapSelection::ALL;
    let request = Request {
        model, object: binding.0, slot: binding.1, size: session.texture_size,
        recipe: draft.clone(), quality, projection,
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
        let gpu = baked.ray_device(device.as_deref(), queue.as_deref());
        baked.live.tick(request, doc, &mut images, gpu);
    }
}
