//! Background UV map baking and the paint workspace's read-only map preview.
use super::*;
use crate::editor::model_document::{Document, MeshData};
use crate::editor::model_bake_settings::Settings;
use crate::editor::model_paint_data::custom;
use bevy::{
    asset::RenderAssetUsages,
    input_focus::InputFocus,
    render::renderer::{RenderDevice, RenderQueue},
    tasks::{AsyncComputeTaskPool, IoTaskPool, Task, block_on, poll_once},
    text::EditableText,
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

#[path = "model_paint_bake_gizmo.rs"]
pub(in crate::editor::ui::model_viewer) mod custom_gizmo;

#[path = "model_paint_cage_brush.rs"]
pub(in crate::editor::ui::model_viewer) mod cage_brush;
#[path = "model_paint_cage_preview.rs"]
pub(in crate::editor::ui::model_viewer) mod cage_preview;
#[path = "model_paint_bake_cache.rs"]
mod disk;
#[path = "model_paint_bake_live.rs"]
pub(in crate::editor::ui::model_viewer) mod live;
#[path = "model_paint_bake_view3d.rs"]
pub(in crate::editor::ui::model_viewer) mod view3d;
#[path = "model_paint_bake_mesh.rs"]
mod mesh;
#[path = "model_paint_bake_levels.rs"]
pub(super) mod levels;
pub(in crate::editor) use levels::import_levels;
pub(super) use levels::Maps;

pub(in crate::editor) use crate::editor::model_paint_data::map_fixes::BakeMap;
use crate::editor::model_paint_data::map_fixes::Fixes;
use crate::editor::model_paint_data::mask::{BakedInputs, Generator, GeneratorInput};
impl BakeMap {
    pub(super) const OPTIONAL: [Self; 7] = [
        Self::Occlusion, Self::Cavity, Self::Thickness, Self::Sky,
        Self::Curvature, Self::EdgeDistance, Self::CreaseDistance,
    ];
    pub const ALL: [Self; 16] = [
        Self::Occlusion,
        Self::Cavity,
        Self::Thickness,
        Self::Sky,
        Self::PartRandom,
        Self::PartGradient,
        Self::BentNormal,
        Self::WorldNormal,
        Self::Position,
        Self::PartId,
        Self::Curvature,
        Self::EdgeDistance,
        Self::CreaseDistance,
        Self::ProjectedNormal,
        Self::ProjectedHeight,
        Self::ProjectionCoverage,
    ];
    pub fn requires_high_poly(self) -> bool {
        matches!(
            self,
            Self::ProjectedNormal | Self::ProjectedHeight | Self::ProjectionCoverage
        )
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Occlusion => "Окклюзия",
            Self::Cavity => "Впадины",
            Self::Thickness => "Толщина",
            Self::Sky => "Открытость небу",
            Self::PartRandom => "Случайное значение части",
            Self::PartGradient => "Градиент высоты части",
            Self::BentNormal => "Нормаль видимости",
            Self::WorldNormal => "Мировые нормали",
            Self::Position => "Положение",
            Self::PartId => "ID частей",
            Self::Curvature => "Кривизна",
            Self::EdgeDistance => "До выпуклых рёбер",
            Self::CreaseDistance => "До вогнутых складок",
            Self::ProjectedNormal => "High-poly · нормали",
            Self::ProjectedHeight => "High-poly · высота",
            Self::ProjectionCoverage => "High-poly · попадания лучей",
            Self::Custom(_) => "Пользовательская карта",
        }
    }
    pub fn export_name(self) -> &'static str {
        match self {
            Self::Occlusion => "AmbientOcclusion",
            Self::Cavity => "Cavity",
            Self::Thickness => "Thickness",
            Self::Sky => "SkyExposure",
            Self::PartRandom => "PartRandom",
            Self::PartGradient => "PartGradient",
            Self::BentNormal => "BentNormal",
            Self::WorldNormal => "WorldNormal",
            Self::Position => "Position",
            Self::PartId => "PartID",
            Self::Curvature => "Curvature",
            Self::EdgeDistance => "EdgeDistance",
            Self::CreaseDistance => "CreaseDistance",
            Self::ProjectedNormal => "BakedNormal",
            Self::ProjectedHeight => "BakedHeight",
            Self::ProjectionCoverage => "ProjectionCoverage",
            Self::Custom(_) => "CustomMap",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MapSelection(u8);

impl MapSelection {
    pub const ALL: Self = Self(0b111_1111);
    pub const MINIMUM: Self = Self(0);
    const GENERATOR_MAPS: [(BakeMap, GeneratorInput); 8] = {
        use GeneratorInput as Input;
        [
            (BakeMap::Occlusion, Input::Occlusion),
            (BakeMap::Cavity, Input::Cavity),
            (BakeMap::Thickness, Input::Thin),
            (BakeMap::Sky, Input::Sky),
            (BakeMap::Curvature, Input::Convexity),
            (BakeMap::Curvature, Input::Concavity),
            (BakeMap::EdgeDistance, Input::Edges),
            (BakeMap::CreaseDistance, Input::Creases),
        ]
    };

    fn bit(map: BakeMap) -> u8 {
        match map {
            BakeMap::Occlusion | BakeMap::BentNormal => 1 << 0,
            BakeMap::Cavity => 1 << 1,
            BakeMap::Thickness => 1 << 2,
            BakeMap::Sky => 1 << 3,
            BakeMap::Curvature => 1 << 4,
            BakeMap::EdgeDistance => 1 << 5,
            BakeMap::CreaseDistance => 1 << 6,
            _ => 0,
        }
    }

    pub fn contains(self, map: BakeMap) -> bool {
        let bit = Self::bit(map);
        bit == 0 || self.0 & bit != 0
    }

    pub fn toggle(self, map: BakeMap) -> Self {
        Self(self.0 ^ Self::bit(map))
    }

    pub fn visibility(self) -> bool {
        self.0 & 0b1111 != 0
    }

    pub fn relief(self) -> bool {
        self.0 & 0b111_0000 != 0
    }

    pub fn bits(self) -> u8 {
        self.0
    }

    fn disabled_inputs(self) -> u32 {
        Self::GENERATOR_MAPS
            .into_iter()
            .filter(|&(map, _)| !self.contains(map))
            .fold(0, |bits, (_, input)| bits | (1 << input as u32))
    }
}

/// Bake the inputs used by reconstructed procedural masks during background SPP import.
pub(in crate::editor) fn import_generator_inputs(
    object: &crate::editor::model_document::Object,
    slot: usize,
    size: u32,
    generators: &[Generator],
) -> Result<Arc<BakedInputs>, String> {
    let maps = MapSelection::GENERATOR_MAPS
        .into_iter()
        .filter(|&(_, input)| {
            generators.iter().any(|generator| generator.weight(input) != 0)
        })
        .fold(MapSelection::MINIMUM, |maps, (map, _)| {
            MapSelection(maps.bits() | MapSelection::bit(map))
        });
    let settings = ProjectionSettings {
        maps,
        quality: Quality::Draft,
        ..default()
    };
    let mut bake_object = object.clone();
    for material in &mut bake_object.materials {
        material.paint = None;
    }
    let maps = mesh::bake_on_device(
        vec![bake_object],
        object.id,
        slot,
        size,
        &settings,
        &Progress::default(),
        None,
    )?;
    if maps.filled == 0 {
        return Err("В наборе нет поверхности с UV-развёрткой".into());
    }
    Ok(maps.generator_inputs().clone())
}

pub(in crate::editor) fn import_bake_map_bits(maps: impl Iterator<Item = BakeMap>) -> u8 {
    maps.fold(0, |bits, map| bits | MapSelection::bit(map))
}

pub(in crate::editor) use crate::editor::model_bake_settings::Quality;

#[derive(Clone, Copy)]
struct Sampling {
    grid: u32,
    occlusion: u32,
    thickness: u32,
    sky: u32,
}

impl Sampling {
    fn denoise_passes(self) -> u32 {
        match self.occlusion {
            0..=32 => 4,
            33..=96 => 3,
            97..=256 => 2,
            _ => 1,
        }
    }
}

impl Quality {
    pub(super) fn hint(self) -> &'static str {
        match self {
            Self::Draft => "1 проба проекции на тексель. Лучи AO, толщины и неба — 16, 8 и 8.",
            Self::Standard => {
                "Сетка сглаживания проекции 2×2. Лучи AO, толщины и неба — 64, 32 и 32."
            }
            Self::High => "Сетка сглаживания проекции 3×3. Лучи AO, толщины и неба — 160, 64 и 64.",
            Self::Ultra => {
                "Сетка сглаживания проекции 8×8. Лучи AO, толщины и неба — 384, 128 и 128. Финальное запекание займёт больше времени."
            }
        }
    }

    fn sampling(self) -> Sampling {
        let (grid, occlusion, thickness, sky) = match self {
            Self::Draft => (1, 16, 8, 8),
            Self::Standard => (2, 64, 32, 32),
            Self::High => (3, 160, 64, 64),
            Self::Ultra => (8, 384, 128, 128),
        };
        Sampling {
            grid,
            occlusion,
            thickness,
            sky,
        }
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct ProjectionSettings {
    pub(super) custom: Arc<Vec<custom::Recipe>>,
    pub(super) path: Option<PathBuf>,
    pub(super) cage: Option<PathBuf>,
    pub(super) auto_fit: bool,
    pub(super) match_names: bool,
    pub(super) fill_misses: bool,
    pub(super) front: u8,
    pub(super) rear: u8,
    pub(super) manual: Option<cage_brush::Scales>,
    pub(super) quality: Quality,
    pub(super) maps: MapSelection,
    source_generation: u64,
}

#[derive(Clone, Copy)]
enum ProjectionFormat {
    Glb,
    Obj,
    Fbx,
}

impl ProjectionFormat {
    fn of(path: &std::path::Path) -> Option<Self> {
        let extension = path.extension()?;
        if extension.eq_ignore_ascii_case("glb") {
            Some(Self::Glb)
        } else if extension.eq_ignore_ascii_case("obj") {
            Some(Self::Obj)
        } else if extension.eq_ignore_ascii_case("fbx") {
            Some(Self::Fbx)
        } else {
            None
        }
    }
}
impl Default for ProjectionSettings {
    fn default() -> Self {
        Self {
            custom: Arc::new(Vec::new()),
            path: None,
            cage: None,
            auto_fit: false,
            match_names: true,
            fill_misses: true,
            front: 5,
            rear: 5,
            manual: None,
            quality: Quality::default(),
            maps: MapSelection::ALL,
            source_generation: 0,
        }
    }
}

impl ProjectionSettings {
    fn saved(&self) -> Settings {
        Settings {
            high_poly: self.path.clone(),
            cage: self.cage.clone(),
            quality: self.quality,
            map_bits: self.maps.bits(),
            front: self.front,
            rear: self.rear,
            match_names: self.match_names,
            fill_misses: self.fill_misses,
            auto_fit: self.auto_fit,
        }
    }

    fn restore(&mut self, settings: &Settings) {
        self.path = settings.high_poly.clone();
        self.cage = settings.cage.clone();
        self.quality = settings.quality;
        self.maps = MapSelection(settings.map_bits);
        self.front = settings.front;
        self.rear = settings.rear;
        self.match_names = settings.match_names;
        self.fill_misses = settings.fill_misses;
        self.auto_fit = settings.auto_fit;
        self.source_generation = self.source_generation.wrapping_add(1);
    }
}

pub(super) fn shelf_maps(
    object: crate::editor::model_document::Object,
    size: u32,
) -> Result<Arc<crate::editor::model_paint_data::mask::BakedInputs>, String> {
    let target = object.id;
    mesh::bake_on_device(
        vec![object], target, 0, size, &ProjectionSettings::default(), &Progress::default(), None,
    ).map(|maps| maps.generator_inputs().clone())
}

pub(super) fn projection_for(
    session: &Session,
    object: &crate::editor::model_document::Object,
) -> ProjectionSettings {
    let mut settings = session.bake_projection.clone();
    if settings.cage.is_none() {
        settings.manual = session
            .cage_edits
            .get(&object.id)
            .filter(|edit| edit.matches(&object.geometry))
            .map(|edit| edit.scales.clone());
    }
    settings
}

#[derive(Clone, Copy)]
enum BakeInput {
    HighPoly,
    Cage,
}

impl BakeInput {
    fn label(self) -> &'static str {
        match self {
            Self::HighPoly => "high-poly",
            Self::Cage => "оболочку",
        }
    }
}

#[derive(Default)]
struct Progress {
    cancelled: AtomicBool,
    rows: AtomicU32,
    #[cfg(test)]
    profile: bool,
    #[cfg(test)]
    serial_gpu: bool,
    #[cfg(test)]
    serial_projection: bool,
    #[cfg(test)]
    projection_peak: AtomicU32,
    #[cfg(test)]
    fresh_high_source: bool,
    #[cfg(test)]
    high_source_hit: AtomicBool,
    #[cfg(test)]
    high_relief_hit: AtomicBool,
    #[cfg(test)]
    legacy_visibility_readback: bool,
    #[cfg(test)]
    visibility_readback_bytes: std::sync::atomic::AtomicU64,
    #[cfg(test)]
    resident_visibility: AtomicBool,
    // Isolate the ownership readback from the regular GPU ray passes in benchmarks.
    gpu_raster: bool,
    #[cfg(test)]
    gpu_raster_ms: std::sync::OnceLock<f64>,
    #[cfg(test)]
    ray_wait_ns: std::sync::atomic::AtomicU64,
    #[cfg(test)]
    projection_wait_ns: std::sync::atomic::AtomicU64,
    #[cfg(test)]
    stage_ms: std::sync::OnceLock<[f64; 4]>,
}
impl Progress {
    fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err("Запекание отменено".into())
        } else {
            Ok(())
        }
    }
}

#[derive(Clone)]
struct Shape {
    id: usize,
    mesh: MeshData,
    transform: Transform,
}

#[derive(Clone)]
struct Key {
    model: usize,
    object: usize,
    slot: usize,
    size: u32,
    offset: Vec2,
    name: String,
    custom: Arc<Vec<custom::Recipe>>,
    settings: Settings,
    cage: Option<cage_brush::Scales>,
    shapes: Vec<Shape>,
}
impl Key {
    fn material<'a>(&self, doc: &'a Document) -> Option<&'a crate::editor::model_document::MaterialSlot> {
        doc.objects.iter().find(|object| object.id == self.object)?.materials.get(self.slot)
    }

    fn new(
        model: usize,
        doc: &Document,
        (object, slot): (usize, usize),
        size: u32,
    ) -> Option<Self> {
        let target = doc.objects.iter().find(|o| o.id == object && !o.hidden)?;
        target.materials.get(slot)?;
        Some(Self {
            model,
            object,
            slot,
            size,
            offset: target.uv_offset(slot),
            name: target.name.clone(),
            custom: doc.custom_bakes.clone(),
            settings: doc.bake_settings.clone(),
            cage: doc.cage_edits.get(&object)
                .filter(|edit| edit.matches(&target.geometry))
                .map(|edit| edit.scales.clone()),
            shapes: doc
                .objects
                .iter()
                .filter(|o| !o.hidden)
                .map(|o| Shape {
                    id: o.id,
                    mesh: o.geometry.clone(),
                    transform: o.transform,
                })
                .collect(),
        })
    }
    fn matches_scene(&self, doc: &Document) -> bool {
        self.matches_setup(doc)
            && self.cage.as_ref() == doc.cage_edits.get(&self.object)
                .filter(|edit| {
                    doc.objects.iter().find(|object| object.id == self.object)
                        .is_some_and(|object| edit.matches(&object.geometry))
                })
                .map(|edit| &edit.scales)
    }

    fn matches_setup(&self, doc: &Document) -> bool {
        self.settings == doc.bake_settings
            && self.matches_geometry(doc)
            && (Arc::ptr_eq(&self.custom, &doc.custom_bakes)
                || (self.custom.len() == doc.custom_bakes.len()
                    && self.custom.iter().all(|recipe| {
                        doc.custom_bakes.iter().any(|other| recipe.same_bake(other))
                    })))
    }

    fn matches_geometry(&self, doc: &Document) -> bool {
        let mut current = doc.objects.iter().filter(|o| !o.hidden);
        self.shapes.iter().all(|shape| {
            current.next().is_some_and(|o| {
                o.id == shape.id
                    && o.transform == shape.transform
                    && o.geometry.shares_storage_with(&shape.mesh)
            })
        }) && current.next().is_none()
            && doc
                .objects
                .iter()
                .find(|o| o.id == self.object)
                .is_some_and(|o| {
                    o.name == self.name
                        && o.materials.get(self.slot).is_some()
                        && o.uv_offset(self.slot) == self.offset
                })
    }
}

struct Job {
    key: Key,
    progress: Arc<Progress>,
    task: Task<Result<(mesh::Maps, bool), String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.progress.cancelled.store(true, Ordering::Relaxed);
    }
}

#[derive(Resource, Default)]
pub(in crate::editor::ui::model_viewer) struct State {
    running: Option<Job>,
    automatic: HashMap<usize, VecDeque<(usize, usize)>>,
    picking: Option<(usize, BakeInput, Task<std::io::Result<Option<PathBuf>>>)>,
    cache: VecDeque<(Key, Arc<Maps>)>,
    disk: disk::Cache,
    clearing_cache: Option<Task<std::io::Result<usize>>>,
    requested: Option<(Arc<Maps>, BakeMap, Task<Vec<u8>>)>,
    preview: Option<(Arc<Maps>, BakeMap, Handle<Image>)>,
    pub(super) fix_preview: Option<FixPreview>,
    pub(super) live: live::State,
    pub(super) masks: engine::live_mask::State,
}

pub(super) struct FixPreview {
    pub model: usize,
    pub binding: (usize, usize),
    pub map: BakeMap,
    pub image: Handle<Image>,
    pub source: Arc<Maps>,
    pub fixes: Arc<Fixes>,
}
impl State {
    fn begin(
        &mut self,
        key: Key,
        doc: &Document,
        mut projection: ProjectionSettings,
        gpu: Option<mesh::RayDevice>,
    ) {
        let objects = doc.objects.clone();
        let (model, object, slot, size) = (key.model, key.object, key.slot, key.size);
        projection.custom = doc.custom_bakes.clone();
        if projection.path.is_some() {
            self.cache.retain(|(entry, _)| {
                (entry.model, entry.object, entry.slot, entry.size) != (model, object, slot, size)
            });
        }
        let disk = self.disk.clone();
        let progress = Arc::new(Progress::default());
        let worker = progress.clone();
        let task = AsyncComputeTaskPool::get().spawn(async move {
            disk.bake(objects, object, slot, size, &projection, &worker, gpu.as_ref())
        });
        self.running = Some(Job { key, progress, task });
    }

    fn automatic_key(
        &mut self,
        model: usize,
        doc: &mut Document,
        selected: (usize, usize),
    ) -> Option<Key> {
        if std::mem::take(&mut doc.auto_bake_pending) {
            let mut sets = VecDeque::new();
            for object in doc.objects.iter().filter(|object| !object.hidden) {
                for slot in 0..object.materials.len() {
                    let key = (object.id, slot);
                    if key == selected {
                        sets.push_front(key);
                    } else {
                        sets.push_back(key);
                    }
                }
            }
            self.automatic.insert(model, sets);
        }
        while let Some(binding) = self.automatic.get_mut(&model).and_then(VecDeque::pop_front) {
            let (id, slot) = binding;
            let Some(object) = doc.objects.iter().find(|object| object.id == id && !object.hidden) else {
                continue;
            };
            let Some(material) = object.materials.get(slot) else { continue };
            let size = doc.paint_size(material);
            if material.paint.as_ref().is_some_and(|surface| surface.baked.is_some())
                || self.maps(model, doc, binding, size).is_some()
                || !engine::projection::has_uv(object, slot)
            {
                continue;
            }
            return Key::new(model, doc, binding, size);
        }
        self.automatic.remove(&model);
        None
    }

    #[cfg(test)]
    pub(super) fn set_disk_cache(&mut self, directory: PathBuf) {
        self.disk = disk::Cache::at(directory);
    }

    fn invalidate_cage(&mut self, model: usize, object: usize) {
        self.cache
            .retain(|(key, _)| key.model != model || key.object != object);
    }

    pub(super) fn maps(
        &self,
        model: usize,
        doc: &Document,
        key: (usize, usize),
        size: u32,
    ) -> Option<&Arc<Maps>> {
        self.cache
            .iter()
            .rev()
            .find(|(entry, _)| {
                entry.model == model
                    && (entry.object, entry.slot) == key
                    && entry.size == size
                    && entry.matches_scene(doc)
            })
            .map(|(_, maps)| maps)
    }
    pub(super) fn preview_for(
        &self, model: usize, doc: &Document, session: &Session, map: BakeMap,
    ) -> Option<&Handle<Image>> {
        let binding = session.binding?;
        if session.bake_live && let BakeMap::Custom(id) = map
            && let Some(image) = self.live.image_for(model, doc, binding, id)
        {
            return Some(image);
        }
        let maps = self.maps(model, doc, binding, session.texture_size)?;
        if let Some(live) = &self.fix_preview
            && live.model == model && live.binding == binding && live.map == map
            && maps.same_background(&live.source)
            && doc.objects.iter().find(|object| object.id == binding.0)
                .and_then(|object| object.materials.get(binding.1))
                .is_some_and(|slot| Arc::ptr_eq(&slot.bake_fixes, &live.fixes))
        {
            return Some(&live.image);
        }
        self.preview
            .as_ref()
            .filter(|(source, kind, _)| *kind == map && Arc::ptr_eq(source, maps))
            .map(|(_, _, image)| image)
    }

    #[cfg(test)]
    pub fn preview(&self, map: BakeMap) -> Option<&Handle<Image>> {
        if let BakeMap::Custom(id) = map && let Some(image) = self.live.image(id) {
            return Some(image);
        }
        self.preview.as_ref().filter(|(_, kind, _)| *kind == map).map(|(_, _, image)| image)
    }
    pub fn progress(&self) -> Option<f32> {
        self.running
            .as_ref()
            .map(|job| job.progress.rows.load(Ordering::Relaxed) as f32 / 1000.)
    }
    fn finish_preview(
        &mut self,
        request: Option<(Arc<Maps>, BakeMap)>,
        images: &mut Assets<Image>,
    ) {
        if request.as_ref().zip(self.fix_preview.as_ref()).is_some_and(|((maps, map), live)| {
            live.map == *map && maps.same_background(&live.source) && maps.has_fixes(&live.fixes)
        }) {
            self.preview = None;
            self.requested = None;
            return;
        }
        let same = |maps: &Arc<Maps>, kind| {
            request
                .as_ref()
                .is_some_and(|(wanted, map)| *map == kind && Arc::ptr_eq(wanted, maps))
        };
        if self
            .preview
            .as_ref()
            .is_some_and(|(maps, kind, _)| !same(maps, *kind))
        {
            self.preview = None;
        }
        if self
            .requested
            .as_ref()
            .is_some_and(|(maps, kind, _)| !same(maps, *kind))
        {
            self.requested = None;
        }
        if let Some((maps, kind, task)) = self.requested.as_mut()
            && let Some(rgba) = block_on(poll_once(task))
        {
            let image = image::RgbaImage::from_raw(maps.size, maps.size, rgba).unwrap();
            let image = images.add(Image::from_dynamic(
                image::DynamicImage::ImageRgba8(image),
                true,
                RenderAssetUsages::default(),
            ));
            self.preview = Some((maps.clone(), *kind, image));
            self.requested = None;
        }
        if self.preview.is_none()
            && self.requested.is_none()
            && let Some((maps, kind)) = request
        {
            let source = maps.clone();
            self.requested = Some((
                maps,
                kind,
                AsyncComputeTaskPool::get().spawn(async move { source.rgba(kind) }),
            ));
        }
    }
}

pub(in crate::editor::ui::model_viewer) fn update(
    mut state: ResMut<State>,
    mut paint: ResMut<PaintState>,
    mut editing: ResMut<editing::State>,
    mut editor: ResMut<Editor>,
    mut images: ResMut<Assets<Image>>,
    mut actions: MessageReader<Action>,
    mut focus: ResMut<InputFocus>,
    keys: Res<ButtonInput<KeyCode>>,
    mut names: Query<(&ui::CustomMapName, &mut EditableText)>,
    device: Option<Res<RenderDevice>>,
    queue: Option<Res<RenderQueue>>,
) {
    let ray_device = || device.as_deref().zip(queue.as_deref()).map(|(device, queue)| {
        mesh::RayDevice { device: device.clone(), queue: queue.clone() }
    });
    state.automatic.retain(|model, _| editing.documents.contains_key(model));
    if let Some(result) = state.clearing_cache.as_mut().and_then(|task| block_on(poll_once(task))) {
        state.clearing_cache = None;
        editor.status = match result {
            Ok(0) => "Дисковый кэш запекания пуст".into(),
            Ok(count) => format!("Дисковый кэш запекания очищен · удалено записей: {count}"),
            Err(error) => format!("Не удалось полностью очистить дисковый кэш запекания · {error}"),
        };
    }
    if state.running.as_ref().is_some_and(|job| {
        editing
            .documents
            .get(&job.key.model)
            .is_none_or(|doc| !job.key.matches_scene(doc))
    }) {
        state.running = None;
        editor.status = "Запекание остановлено после изменения модели".into();
    }
    state.cache.retain_mut(|(key, maps)| {
        editing
            .documents
            .get(&key.model)
            .is_some_and(|doc| {
                if !key.matches_scene(doc) {
                    return false;
                }
                if !Arc::ptr_eq(&key.custom, &doc.custom_bakes) {
                    key.custom = doc.custom_bakes.clone();
                }
                let material = key.material(doc);
                if !maps.matches(material) {
                    *maps = Arc::new(Maps::new(maps.raw.clone(), material));
                }
                true
            })
    });
    if let Some(result) = state
        .running
        .as_mut()
        .and_then(|job| block_on(poll_once(&mut job.task)))
    {
        let job = state.running.take().unwrap();
        match result {
            Ok((maps, from_disk)) => {
                let status = if from_disk {
                    "Карты загружены из дискового кэша"
                } else if maps.gpu_projection {
                    "Карты меша готовы · проекция high-poly и лучи рассчитаны на GPU"
                } else if maps.gpu_rays {
                    "Карты меша готовы · лучи рассчитаны на GPU"
                } else {
                    "Карты меша готовы"
                };
                editor.status = format!(
                    "{status} · UV покрывает {:.0}% карты",
                    maps.filled as f32 * 100. / maps.size.pow(2) as f32
                );
                if maps.gpu_denoise {
                    editor.status.push_str(" · шумоподавление на GPU");
                }
                if maps.gpu_padding {
                    editor.status.push_str(" · UV-отступы на GPU");
                }
                if maps.has_map(BakeMap::ProjectedNormal) {
                    editor.status.push_str(&format!(
                        " · лучи high-poly попали в {:.0}% UV-текселей",
                        maps.hits as f32 * 100. / maps.filled as f32
                    ));
                    if maps.filled_misses > 0 {
                        editor.status.push_str(&format!(
                            " · заполнено {:.0}% UV-текселей",
                            maps.filled_misses as f32 * 100. / maps.filled as f32
                        ));
                    }
                }
                if let Some(fit) = &maps.cage_fit {
                    editor.status.push_str(&format!(
                        " · оболочка расширена у {} вершин, сужена у {}, без подгонки {}",
                        fit.raised, fit.lowered, fit.unresolved,
                    ));
                }
                state.cache.retain(|(key, _)| {
                    (key.model, key.object, key.slot, key.size)
                        != (job.key.model, job.key.object, job.key.slot, job.key.size)
                });
                let material = editing.documents.get(&job.key.model)
                    .and_then(|doc| job.key.material(doc));
                state.cache.push_back((job.key.clone(), Arc::new(Maps::new(Arc::new(maps), material))));
                while state.cache.len() > 1
                    && (state.cache.len() > 8
                        || state
                            .cache
                            .iter()
                            .map(|(_, maps)| maps.bytes())
                            .sum::<usize>()
                            > 192 * 1024 * 1024)
                {
                    state.cache.pop_front();
                }
            }
            Err(error) => editor.status = format!("Не удалось запечь карты · {error}"),
        }
    }
    let active = editor
        .viewing_model
        .filter(|_| editor.pending.is_none() && editor.viewing_image.is_none())
        .and_then(|index| {
            editing
                .documents
                .get(&index)
                .filter(|doc| doc.mode == Mode::Paint)
                .map(|_| index)
        });
    let Some(index) = active else {
        actions.clear();
        state.finish_preview(None, &mut images);
        return;
    };
    let Some(session) = paint.sessions.get_mut(&index) else {
        return;
    };
    cage_brush::sync(&editing.documents[&index], session);
    let settings = &editing.documents[&index].bake_settings;
    if session.bake_projection.saved() != *settings {
        session.bake_projection.restore(settings);
        if state.running.as_ref().is_some_and(|job| job.key.model == index) {
            state.running = None;
        }
        state.cache.retain(|(key, _)| key.model != index);
        session.bake_view = session.bake_view.filter(|&map| session.bake_projection.maps.contains(map));
        session.revision += 1;
    }
    if session.bake_custom_gizmo.is_some_and(|id| {
        !editing.documents[&index].custom_bakes.iter().any(|recipe| recipe.id == id)
    }) {
        session.bake_custom_gizmo = None;
        session.revision += 1;
    }
    if let Some((model, input, outcome)) =
        state.picking.as_mut().and_then(|(model, input, task)| {
            block_on(poll_once(task)).map(|outcome| (*model, *input, outcome))
        })
    {
        state.picking = None;
        if model == index {
            match outcome {
                Ok(Some(path)) if ProjectionFormat::of(&path).is_some() => {
                    match input {
                        BakeInput::HighPoly => session.bake_projection.path = Some(path),
                        BakeInput::Cage => session.bake_projection.cage = Some(path),
                    }
                    session.bake_projection.source_generation =
                        session.bake_projection.source_generation.wrapping_add(1);
                    state.cache.retain(|(key, _)| key.model != index);
                    session.revision += 1;
                }
                Ok(Some(_)) => editor.status = "Выберите GLB, OBJ или FBX".into(),
                Ok(None) => {}
                Err(error) => {
                    editor.status = format!("Не удалось выбрать {} · {error}", input.label())
                }
            }
        }
    }
    let name_key = if session.panel == Panel::Maps
        && let Some(entity) = focus.get()
        && let Ok((field, mut text)) = names.get_mut(entity)
    {
        if keys.just_pressed(KeyCode::Escape) {
            if let Some(recipe) = editing.documents[&index].custom_bakes.iter().find(|recipe| recipe.id == field.0) {
                text.editor_mut().set_text(&recipe.name);
            }
            focus.clear();
        }
        keys.just_pressed(KeyCode::Enter).then_some(PaintAction::BakeCustomRename(field.0))
    } else {
        None
    };
    let paint_actions = actions.read().filter_map(|action| match *action {
        Action::ModelView(ModelViewAction::Paint(action)) => Some(action),
        _ => None,
    });
    for action in paint_actions.chain(name_key) {
        if let PaintAction::BakeFixValue(value) = action {
            if session.active_bake_fix().is_some_and(levels::supports) && !session.busy {
                session.bake_fix_value = value.min(100);
                session.revision += 1;
            }
            continue;
        }
        if let PaintAction::BakeFixTool(map) | PaintAction::BakeClearFix(map) = action {
            let doc = editing.documents.get_mut(&index).unwrap();
            let Some(binding) = session.binding else { continue };
            if session.panel != Panel::Maps || session.bake_view != Some(map) || !map.supports_fix()
                || (doc.paint_busy && matches!(action, PaintAction::BakeFixTool(_)))
                || state.running.is_some()
                || !state.maps(index, doc, binding, session.texture_size).is_some_and(|maps| maps.has_map(map))
            {
                continue;
            }
            if matches!(action, PaintAction::BakeFixTool(_)) {
                session.bake_fix = if session.active_bake_fix() == Some(map) {
                    None
                } else {
                    session.tool = PaintTool::Brush;
                    session.bake_on_model = true;
                    session.bake_live_masks = false;
                    session.bake_paint_cage = false;
                    session.bake_custom_gizmo = None;
                    session.graph_canvas = false;
                    Some(map)
                };
            } else {
                let before = doc.snapshot();
                if let Some(material) = doc.objects.iter_mut().find(|object| object.id == binding.0)
                    .and_then(|object| object.materials.get_mut(binding.1))
                    && material.bake_fixes.iter().any(|(key, _)| *key == map)
                {
                    Arc::make_mut(&mut material.bake_fixes).retain(|(key, _)| *key != map);
                    state.fix_preview = None;
                    doc.checkpoint(before);
                }
            }
            session.revision += 1;
            continue;
        }
        if let PaintAction::BakeAdjustLevels(map, _, _) | PaintAction::BakeResetLevels(map) = action {
            let doc = editing.documents.get_mut(&index).unwrap();
            let Some(binding) = session.binding else { continue };
            if doc.paint_busy || session.bake_view != Some(map)
                || !levels::supports(map)
                || !state.maps(index, doc, binding, session.texture_size).is_some_and(|maps| maps.has_map(map))
            {
                continue;
            }
            let before = doc.snapshot();
            let (object, slot) = binding;
            let Some(material) = doc.objects.iter_mut().find(|item| item.id == object)
                .and_then(|item| item.materials.get_mut(slot)) else { continue };
            let previous = levels::get(material, map);
            let mut current = previous;
            match action {
                PaintAction::BakeAdjustLevels(_, field, amount) => {
                    let multiplier = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) { 5 } else { 1 };
                    current.shift(field, amount.saturating_mul(multiplier));
                }
                _ => current = levels::Levels::default(),
            }
            if current != previous {
                match levels::set(material, map, current) {
                    Ok(()) => {
                        state.fix_preview = None;
                        for (key, maps) in &mut state.cache {
                            if (key.model, key.object, key.slot) == (index, object, slot) {
                                *maps = Arc::new(Maps::new(maps.raw.clone(), Some(material)));
                            }
                        }
                        doc.checkpoint(before);
                        session.revision += 1;
                    }
                    Err(error) => editor.status = error,
                }
            }
            continue;
        }
        if matches!(
            action,
            PaintAction::BakeCustomAdd(_)
                | PaintAction::BakeCustomRemove(_)
                | PaintAction::BakeCustomAdjust(..)
                | PaintAction::BakeCustomRename(_)
                | PaintAction::BakeCustomDuplicate(_)
                | PaintAction::BakeCustomMove(..)
        ) {
            if state.running.is_none() && state.picking.is_none() {
                let doc = editing.documents.get_mut(&index).unwrap();
                let before = doc.snapshot();
                let name = if let PaintAction::BakeCustomRename(id) = action {
                    names.iter().find(|(field, _)| field.0 == id).map(|(_, text)| text.value().to_string())
                } else {
                    None
                };
                match edit_custom_maps(Arc::make_mut(&mut doc.custom_bakes), action, name.as_deref()) {
                    Ok(changed) => {
                        if let PaintAction::BakeCustomRename(_) = action {
                            focus.clear();
                        }
                        if changed {
                            doc.checkpoint(before);
                            state.cache.retain(|(key, _)| key.model != index || key.matches_scene(doc));
                            if let PaintAction::BakeCustomRemove(id) = action
                                && session.bake_view == Some(BakeMap::Custom(id))
                            {
                                session.bake_view = None;
                            }
                            session.revision += 1;
                        }
                    }
                    Err(message) => editor.status = message.into(),
                }
            }
            continue;
        }
        let doc = &editing.documents[&index];
        match action {
            PaintAction::BakeShowOnModel => {
                session.preview_3d = Preview3d::Material;
                session.bake_on_model = !session.bake_on_model;
                if session.bake_on_model {
                    session.bake_live_masks = false;
                }
                if session.view == View::Three && !session.bake_on_model && session.bake_view.is_some() {
                    session.view = View::Split;
                }
                session.revision += 1;
            }
            PaintAction::BakeLiveMasks => {
                session.bake_live_masks = !session.bake_live_masks;
                if session.bake_live_masks {
                    session.bake_on_model = false;
                } else if session.view == View::Three {
                    session.view = View::Split;
                }
                session.revision += 1;
            }
            PaintAction::BakeLivePreview => {
                session.bake_live = !session.bake_live;
                session.revision += 1;
            }
            PaintAction::BakeCustomGizmo(id)
                if state.running.is_none() && state.picking.is_none()
                    && doc.custom_bakes.iter().any(|recipe| recipe.id == id
                        && (recipe.kind.has_direction() || recipe.kind == custom::Kind::Point)) =>
            {
                session.bake_custom_gizmo = (session.bake_custom_gizmo != Some(id)).then_some(id);
                if session.bake_custom_gizmo.is_some() {
                    session.bake_paint_cage = false;
                    if session.bake_live {
                        session.bake_view = Some(BakeMap::Custom(id));
                    }
                    if session.view == View::Two {
                        session.view = View::Split;
                    }
                }
                session.revision += 1;
            }
            PaintAction::BakeQuality(quality)
                if state.running.is_none() && session.bake_projection.quality != quality =>
            {
                session.bake_projection.quality = quality;
                state.cache.retain(|(key, _)| key.model != index);
                session.revision += 1;
            }
            PaintAction::BakeToggleMap(_) | PaintAction::BakeSelectAllMaps(_)
                if state.running.is_none() =>
            {
                let selection = match action {
                    PaintAction::BakeToggleMap(map) => session.bake_projection.maps.toggle(map),
                    PaintAction::BakeSelectAllMaps(true) => MapSelection::ALL,
                    _ => MapSelection::MINIMUM,
                };
                if session.bake_projection.maps != selection {
                    session.bake_projection.maps = selection;
                    if session.bake_view.is_some_and(|map| !selection.contains(map)) {
                        session.bake_view = None;
                    }
                    state.cache.retain(|(key, _)| key.model != index);
                    session.revision += 1;
                }
            }
            PaintAction::BakeMapOptions => {
                session.bake_map_options = !session.bake_map_options;
                session.revision += 1;
            }
            PaintAction::BakePaintCage
                if session.bake_projection.path.is_some()
                    && session.bake_projection.cage.is_none() =>
            {
                session.bake_paint_cage = !session.bake_paint_cage;
                if session.bake_paint_cage {
                    session.bake_custom_gizmo = None;
                    session.bake_inspect_cage = true;
                    if session.view == View::Two {
                        session.view = View::Split;
                    }
                }
                session.revision += 1;
            }
            PaintAction::BakeResetCagePaint if state.running.is_none() => {
                if let Some((object, _)) = session.binding
                    && session.cage_edits.remove(&object).is_some()
                {
                    let doc = editing.documents.get_mut(&index).unwrap();
                    let before = doc.snapshot();
                    Arc::make_mut(&mut doc.cage_edits).remove(&object);
                    doc.checkpoint(before);
                    session.cage_snapshot = doc.cage_edits.clone();
                    state.invalidate_cage(index, object);
                    session.cage_edit_version = session.cage_edit_version.wrapping_add(1);
                    session.revision += 1;
                }
            }
            PaintAction::BakeInspectCage if session.bake_projection.path.is_some() => {
                session.bake_inspect_cage = !session.bake_inspect_cage;
                if !session.bake_inspect_cage {
                    session.bake_paint_cage = false;
                }
                if session.bake_inspect_cage && session.view == View::Two {
                    session.view = View::Split;
                }
                session.revision += 1;
            }
            PaintAction::BakePickHigh | PaintAction::BakePickCage
                if state.picking.is_none()
                    && state.running.is_none()
                    && (action == PaintAction::BakePickHigh
                        || session.bake_projection.path.is_some()) =>
            {
                let root = editor.project.asset_root.clone();
                state.picking = Some((
                    index,
                    if action == PaintAction::BakePickHigh {
                        BakeInput::HighPoly
                    } else {
                        BakeInput::Cage
                    },
                    IoTaskPool::get().spawn(async move {
                        crate::editor::asset_monitor::choose_file_path(&root)
                    }),
                ));
            }
            PaintAction::BakeClearHigh
            | PaintAction::BakeClearCage
            | PaintAction::BakeAutoFitCage
            | PaintAction::BakeMatchNames
            | PaintAction::BakeFillMisses
            | PaintAction::BakeRange { .. }
                if state.running.is_none() && state.picking.is_none() =>
            {
                match action {
                    PaintAction::BakeClearHigh => {
                        session.bake_projection.path = None;
                        session.bake_view = None;
                    }
                    PaintAction::BakeClearCage => session.bake_projection.cage = None,
                    PaintAction::BakeAutoFitCage => {
                        session.bake_projection.auto_fit = !session.bake_projection.auto_fit
                    }
                    PaintAction::BakeMatchNames => {
                        session.bake_projection.match_names = !session.bake_projection.match_names
                    }
                    PaintAction::BakeFillMisses => {
                        session.bake_projection.fill_misses = !session.bake_projection.fill_misses;
                    }
                    PaintAction::BakeRange {
                        front: true,
                        percent,
                    } => session.bake_projection.front = percent.clamp(1, 100),
                    PaintAction::BakeRange {
                        front: false,
                        percent,
                    } => session.bake_projection.rear = percent.clamp(1, 100),
                    _ => unreachable!(),
                }
                state.cache.retain(|(key, _)| key.model != index);
                session.revision += 1;
            }
            PaintAction::BakeCancel if state.running.is_some() => {
                let model = state.running.as_ref().unwrap().key.model;
                state.running = None;
                state.automatic.remove(&model);
                state.automatic.remove(&index);
                editing.documents.get_mut(&index).unwrap().auto_bake_pending = false;
                editor.status = "Запекание отменено".into();
            }
            PaintAction::BakeClearDiskCache
                if state.running.is_none() && state.clearing_cache.is_none() =>
            {
                let disk = state.disk.clone();
                state.clearing_cache = Some(
                    bevy::tasks::IoTaskPool::get().spawn(async move { disk.clear() }),
                );
                editor.status = "Очистка дискового кэша запекания…".into();
            }
            PaintAction::BakeStart
                if state.running.is_none() && state.picking.is_none()
                    && state.clearing_cache.is_none() && !doc.paint_busy =>
            {
                let Some(mut key) = session
                    .binding
                    .and_then(|binding| Key::new(index, doc, binding, session.texture_size))
                else {
                    continue;
                };
                key.settings = session.bake_projection.saved();
                if key.settings == doc.bake_settings
                    && session.bake_projection.path.is_none()
                    && let Some(maps) = state.maps(index, doc, (key.object, key.slot), key.size)
                {
                    if session.bake_view.is_none_or(|map| !maps.has_map(map)) {
                        session.bake_view = Some(if maps.has_map(BakeMap::Occlusion) {
                            BakeMap::Occlusion
                        } else {
                            BakeMap::WorldNormal
                        });
                    }
                    editor.status = "Использованы готовые карты меша".into();
                } else {
                    let projection = projection_for(
                        session,
                        doc.objects.iter().find(|item| item.id == key.object).unwrap(),
                    );
                    let has_projection = projection.path.is_some();
                    state.begin(key, doc, projection, ray_device());
                    if !matches!(session.bake_view, Some(BakeMap::Custom(_))) {
                        session.bake_view = Some(if has_projection {
                            BakeMap::ProjectedNormal
                        } else if session.bake_projection.maps.contains(BakeMap::Occlusion) {
                            BakeMap::Occlusion
                        } else {
                            BakeMap::WorldNormal
                        });
                    }
                    editor.status = "Запекание карт меша…".into();
                }
                if session.view == View::Three && !session.bake_on_model && !session.bake_live_masks {
                    session.view = View::Split;
                }
                session.revision += 1;
            }
            PaintAction::BakePreview(map) => {
                session.preview_3d = Preview3d::Material;
                if session.bake_fix != map {
                    session.bake_fix = None;
                }
                session.bake_view = map.filter(|&map| session.bake_projection.maps.contains(map));
                if map.is_some() && session.view == View::Three && !session.bake_on_model && !session.bake_live_masks {
                    session.view = View::Split;
                }
                session.revision += 1;
            }
            _ => {}
        }
    }
    let doc = editing.documents.get_mut(&index).unwrap();
    let settings = session.bake_projection.saved();
    if doc.bake_settings != settings {
        let before = doc.snapshot();
        doc.bake_settings = settings;
        doc.checkpoint(before);
    }
    if state.running.is_none()
        && state.picking.is_none()
        && state.clearing_cache.is_none()
        && !doc.paint_busy
        && !session.busy
        && doc.bake_settings.high_poly.is_none()
        && let Some(binding) = session.binding
        && let Some(key) = state.automatic_key(index, doc, binding)
    {
        let mut projection = ProjectionSettings::default();
        projection.restore(&doc.bake_settings);
        state.begin(key, doc, projection, ray_device());
        editor.status = "Автозапекание карт меша…".into();
    }
    let maps = session
        .binding
        .and_then(|key| state.maps(index, doc, key, session.texture_size))
        .cloned();
    let busy = state.running.is_some();
    let picking = state.picking.is_some();
    let clearing = state.clearing_cache.is_some();
    if session.bake_ready != maps.is_some()
        || session.bake_busy != busy
        || session.bake_picking != picking
        || session.bake_cache_clearing != clearing
    {
        session.bake_ready = maps.is_some();
        session.bake_busy = busy;
        session.bake_picking = picking;
        session.bake_cache_clearing = clearing;
        session.revision += 1;
    }
    let available: Vec<_> = maps.as_ref().map_or_else(Vec::new, |maps| {
        BakeMap::ALL.into_iter()
            .chain(doc.custom_bakes.iter().map(|recipe| BakeMap::Custom(recipe.id)))
            .filter(|&map| maps.has_map(map)).collect()
    });
    if session.preview_bakes != available {
        session.preview_bakes = available;
        session.revision += 1;
    }
    if let Preview3d::Baked(map) = session.preview_3d
        && !session.preview_bakes.contains(&map)
    {
        session.preview_3d = Preview3d::Material;
        session.revision += 1;
    }
    let request = maps.zip(session.diagnostic_bake()
        .or_else(|| (session.panel == Panel::Maps).then_some(session.bake_view).flatten()))
        .filter(|(maps, map)| maps.has_map(*map));
    state.finish_preview(request, &mut images);
}

fn edit_custom_maps(
    recipes: &mut Vec<custom::Recipe>,
    action: PaintAction,
    name: Option<&str>,
) -> Result<bool, &'static str> {
    Ok(match action {
        PaintAction::BakeCustomAdd(kind) if recipes.len() < custom::MAX_MAPS => {
            recipes.push(custom::Recipe::new(kind, recipes));
            true
        }
        PaintAction::BakeCustomRemove(id) => {
            let count = recipes.len();
            recipes.retain(|recipe| recipe.id != id);
            recipes.len() != count
        }
        PaintAction::BakeCustomAdjust(id, field, step) => recipes
            .iter_mut()
            .find(|recipe| recipe.id == id)
            .is_some_and(|recipe| recipe.adjust(field, step)),
        PaintAction::BakeCustomRename(id) => {
            let Some((index, name)) = recipes.iter().position(|recipe| recipe.id == id).zip(name) else {
                return Ok(false);
            };
            let name = name.trim();
            if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
                return Err("Введите непустое имя карты длиной до 128 байт");
            }
            if recipes.iter().any(|recipe| recipe.id != id && recipe.name == name) {
                return Err("Карта с таким именем уже существует");
            }
            if recipes[index].name == name {
                return Ok(false);
            }
            recipes[index].name = name.into();
            true
        }
        PaintAction::BakeCustomDuplicate(id) if recipes.len() < custom::MAX_MAPS => {
            let Some(index) = recipes.iter().position(|recipe| recipe.id == id) else {
                return Ok(false);
            };
            let copy = recipes[index].duplicate(recipes);
            recipes.insert(index + 1, copy);
            true
        }
        PaintAction::BakeCustomMove(id, step) => {
            let Some(index) = recipes.iter().position(|recipe| recipe.id == id) else {
                return Ok(false);
            };
            let Some(target) = index.checked_add_signed(isize::from(step.signum())).filter(|&i| i < recipes.len()) else {
                return Ok(false);
            };
            recipes.swap(index, target);
            target != index
        }
        _ => false,
    })
}
