//! Editable model state and transactions. Scene documents never share these meshes.
use super::model_mesh::{BevelOptions, Corner, Edge, Element, Face, Geometry, Selection};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology, VertexAttributeValues},
    prelude::*,
};
use std::{
    collections::{BTreeSet, HashMap},
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::Arc,
};

#[path = "model_glb.rs"]
pub(super) mod glb;
#[path = "model_udim.rs"]
pub(super) mod udim;

#[derive(Clone, Debug)]
pub(super) struct MaterialSlot {
    pub material: Handle<StandardMaterial>,
    pub source: serde_json::Value,
    pub paint: Option<Arc<super::model_paint_data::Surface>>,
    pub authored: Option<Arc<serde_json::Value>>,
    pub bake_fixes: Arc<super::model_paint_data::map_fixes::Fixes>,
}
impl MaterialSlot {
    pub fn name<'a>(&'a self, source: &'a serde_json::Value) -> Option<&'a str> {
        self.source["name"].as_str()
            .or_else(|| self.authored.as_deref().and_then(|material| material["name"].as_str()))
            .or_else(|| source["materials"][self.source["material"].as_u64()? as usize]["name"].as_str())
    }
}
impl PartialEq for MaterialSlot {
    fn eq(&self, other: &Self) -> bool {
        self.material == other.material
            && self.source == other.source
            && self.authored == other.authored
            && self.bake_fixes == other.bake_fixes
            && match (&self.paint, &other.paint) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a.same(b),
                _ => false,
            }
    }
}

#[derive(Clone)]
pub(super) struct MaterialDraft {
    pub name: String,
    pub color: bevy::color::Srgba,
    pub roughness: f32,
    pub metallic: f32,
}

impl Default for MaterialDraft {
    fn default() -> Self {
        Self {
            name: "Материал".into(),
            color: bevy::color::Srgba::WHITE,
            roughness: 0.5,
            metallic: 0.0,
        }
    }
}

impl MaterialDraft {
    pub fn slot(&self, materials: &mut Assets<StandardMaterial>) -> MaterialSlot {
        let color = Color::from(self.color);
        let transparent = self.color.alpha < 1.0;
        let material = materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: self.roughness,
            metallic: self.metallic,
            alpha_mode: if transparent { AlphaMode::Blend } else { AlphaMode::Opaque },
            ..default()
        });
        MaterialSlot {
            bake_fixes: Default::default(),
            material,
            source: serde_json::json!({}),
            paint: None,
            authored: Some(Arc::new(serde_json::json!({
                "name": self.name,
                "alphaMode": if transparent { "BLEND" } else { "OPAQUE" },
                "pbrMetallicRoughness": {
                    "baseColorFactor": color.to_linear().to_f32_array(),
                    "metallicFactor": self.metallic,
                    "roughnessFactor": self.roughness
                }
            }))),
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct MeshData(Arc<Geometry>);
impl MeshData {
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl From<Geometry> for MeshData {
    fn from(mesh: Geometry) -> Self {
        Self(Arc::new(mesh))
    }
}
impl Deref for MeshData {
    type Target = Geometry;
    fn deref(&self) -> &Geometry {
        &self.0
    }
}
impl DerefMut for MeshData {
    fn deref_mut(&mut self) -> &mut Geometry {
        Arc::make_mut(&mut self.0)
    }
}
impl PartialEq for MeshData {
    fn eq(&self, other: &Self) -> bool {
        self.shares_storage_with(other) || self.0 == other.0
    }
}

#[derive(Clone, Debug)]
pub(super) struct Object {
    pub id: usize,
    pub source_node: Option<usize>,
    pub name: String,
    pub geometry: MeshData,
    pub transform: Transform,
    pub materials: Vec<MaterialSlot>,
    pub hidden: bool,
}
impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.source_node == other.source_node
            && self.name == other.name
            && self.geometry == other.geometry
            && self.transform == other.transform
            && self.materials == other.materials
    }
}
#[derive(Clone, Debug)]
pub(super) struct Snapshot {
    pub bake_settings: super::model_bake_settings::Settings,
    pub cage_edits: Arc<super::model_cage_data::Edits>,
    pub objects: Vec<Object>,
    pub custom_bakes: Arc<Vec<super::model_paint_data::custom::Recipe>>,
    pub active: Option<usize>,
    pub selected: BTreeSet<usize>,
    pub selection: Selection,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum Mode {
    #[default]
    Object,
    Edit,
    Uv,
    Paint,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum Pivot {
    #[default]
    Median,
    Bounds,
    ObjectOrigin,
    WorldOrigin,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum Operation {
    Extrude,
    Inset,
    Subdivide,
    Triangulate,
    Quads,
    Dissolve,
    Delete,
    DeleteFaces,
    Duplicate,
    MergeCenter,
    MergeDistance,
    Fill,
    FlipNormals,
    RecalculateNormals,
    Smooth,
    ShadeSmooth,
    ShadeFlat,
    Mirror(usize),
    LoopCut,
    SlideEdges { reverse: bool },
    BevelVertices,
    BevelEdges,
    MarkUvSeam,
    ClearUvSeam,
    ProjectUv(usize),
    SphericalUv,
    Separate,
    Join,
    ApplyTransform,
    OriginToGeometry,
}

pub(super) const PREVIEW_SHADER_EXTRA: &str = "bevy_editor_preview_shader";
pub(super) const PAINT_SET_EXTRA: &str = "bevy_editor_active_paint_set";
pub(super) const MODEL_CAMERA_EXTRA: &str = "bevy_editor_model_camera";
pub(super) const PAINT_PREVIEW_EXTRA: &str = "bevy_editor_paint_preview";

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub(super) struct PaintPreview {
    pub height_strength: Option<f32>,
    pub emission_strength: f32,
}

impl Default for PaintPreview {
    fn default() -> Self {
        Self { height_strength: None, emission_strength: 1. }
    }
}

impl PaintPreview {
    pub fn relief_factor(self, size: u32) -> f32 {
        self.height_strength.map_or(4., |strength| strength * size as f32 / 512.)
    }

    fn read(root: &serde_json::Value) -> Self {
        let mut settings = root["extras"].get(PAINT_PREVIEW_EXTRA)
            .and_then(|value| serde_json::from_value::<Self>(value.clone()).ok())
            .unwrap_or_default();
        settings.height_strength = settings.height_strength.filter(|value| value.is_finite())
            .map(|value| value.clamp(0., 8.));
        settings.emission_strength = settings.emission_strength.clamp(0., 20.);
        settings
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub(super) struct ModelCamera {
    pub center: [f32; 3],
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub fov: f32,
    #[serde(default)]
    pub orthographic: bool,
}

impl ModelCamera {
    pub fn valid(self) -> bool {
        let center = Vec3::from_array(self.center);
        center.is_finite()
            && self.distance > 0.
            && (center.abs().max_element() + self.distance).is_finite()
            && self.yaw.is_finite()
            && self.pitch.abs() <= std::f32::consts::FRAC_PI_2
            && (1.0_f32.to_radians()..=175.0_f32.to_radians()).contains(&self.fov)
    }

    fn read(root: &serde_json::Value) -> Option<Self> {
        serde_json::from_value::<Self>(root["extras"].get(MODEL_CAMERA_EXTRA)?.clone())
            .ok().filter(|camera| camera.valid())
    }
}

pub(super) struct Document {
    pub source: glb::Glb,
    pub objects: Vec<Object>,
    pub original: Vec<Object>,
    pub saved: Vec<Object>,
    pub custom_bakes: Arc<Vec<super::model_paint_data::custom::Recipe>>,
    pub saved_custom_bakes: Arc<Vec<super::model_paint_data::custom::Recipe>>,
    pub bake_settings: super::model_bake_settings::Settings,
    pub saved_bake_settings: super::model_bake_settings::Settings,
    pub preview_shader: Option<Arc<serde_json::Value>>,
    pub paint_set: Option<usize>,
    pub model_camera: Option<ModelCamera>,
    pub paint_preview: PaintPreview,
    pub cage_edits: Arc<super::model_cage_data::Edits>,
    pub saved_cage_edits: Arc<super::model_cage_data::Edits>,
    pub active: Option<usize>,
    pub selected: BTreeSet<usize>,
    pub selection: Selection,
    pub mode: Mode,
    pub pivot_mode: Pivot,
    pub undo: Vec<Snapshot>,
    pub redo: Vec<Snapshot>,
    pub revision: u64,
    pub ui_revision: u64,
    pub recovery_edits: u64,
    pub saved_recovery_edits: u64,
    pub amount: f32,
    pub slide_percent: f32,
    pub material_draft: MaterialDraft,
    pub paint_busy: bool,
    pub paint_pending: bool,
    pub paint_default_size: u32,
    pub auto_bake_pending: bool,
}
impl Document {
    pub fn new(source: glb::Glb, objects: Vec<Object>) -> Self {
        let active = objects.first().map(|o| o.id);
        let selected = active.into_iter().collect();
        let custom_bakes = Arc::new(super::model_paint_data::custom::read_document(&source.json));
        let bake_settings = super::model_bake_settings::read(&source.json, &source.path);
        let preview = &source.json["extras"][PREVIEW_SHADER_EXTRA];
        let preview_shader = preview.is_object().then(|| Arc::new(preview.clone()));
        let paint_set = source.json["extras"][PAINT_SET_EXTRA].as_u64()
            .and_then(|index| usize::try_from(index).ok());
        let model_camera = ModelCamera::read(&source.json);
        let paint_preview = PaintPreview::read(&source.json);
        let cage_edits = Arc::new(objects.iter().filter_map(|object| {
            let node = source.json["nodes"].get(object.source_node?)?;
            super::model_cage_data::read(node, object).map(|edit| (object.id, edit))
        }).collect::<super::model_cage_data::Edits>());
        Self {
            source,
            original: objects.clone(),
            saved: objects.clone(),
            saved_custom_bakes: custom_bakes.clone(),
            saved_bake_settings: bake_settings.clone(),
            bake_settings,
            preview_shader,
            paint_set,
            model_camera,
            paint_preview,
            saved_cage_edits: cage_edits.clone(),
            cage_edits,
            custom_bakes,
            objects,
            active,
            selected,
            selection: Selection::default(),
            mode: Mode::Object,
            pivot_mode: Pivot::Median,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            ui_revision: 0,
            recovery_edits: 0,
            saved_recovery_edits: 0,
            amount: 0.1,
            slide_percent: 10.0,
            material_draft: MaterialDraft::default(),
            paint_busy: false,
            paint_pending: false,
            paint_default_size: super::model_paint_data::DEFAULT_SIZE,
            auto_bake_pending: false,
        }
    }
    pub fn paint_size(&self, material: &MaterialSlot) -> u32 {
        material.paint.as_ref().map_or(self.paint_default_size, |surface| surface.size)
    }

    pub(super) fn initialize_bake_defaults(&mut self, defaults: super::model_bake_settings::Defaults) {
        if super::model_bake_settings::accepts_defaults(&self.source.json) {
            self.bake_settings.quality = defaults.quality;
            self.bake_settings.match_names = defaults.match_names;
            self.saved_bake_settings = self.bake_settings.clone();
        }
    }
    pub fn dirty(&self) -> bool {
        self.paint_pending || self.objects != self.saved || self.custom_bakes != self.saved_custom_bakes
            || self.bake_settings != self.saved_bake_settings
            || self.preview_shader_dirty()
            || (!Arc::ptr_eq(&self.cage_edits, &self.saved_cage_edits)
                && self.cage_edits != self.saved_cage_edits)
    }

    pub(super) fn preview_shader_dirty(&self) -> bool {
        self.preview_shader.as_deref().is_some_and(|graph| {
            graph != &self.source.json["extras"][PREVIEW_SHADER_EXTRA]
        })
    }

    pub(super) fn source_for_save(&self) -> glb::Glb {
        let mut source = self.source.clone();
        if let Some(graph) = &self.preview_shader {
            source.json["extras"][PREVIEW_SHADER_EXTRA] = graph.as_ref().clone();
        }
        if let Some(set) = self.writable_paint_set() {
            source.json["extras"][PAINT_SET_EXTRA] = serde_json::json!(set);
        }
        if let Some(camera) = self.model_camera.filter(|camera| camera.valid()) {
            source.json["extras"][MODEL_CAMERA_EXTRA] = serde_json::json!(camera);
        }
        source
    }

    pub(super) fn paint_set_index(&self) -> usize {
        let count: usize = self.objects.iter().map(|object| object.materials.len()).sum();
        self.paint_set.unwrap_or(0).min(count.saturating_sub(1))
    }

    fn writable_paint_set(&self) -> Option<usize> {
        let extras = &self.source.json["extras"];
        self.paint_set.filter(|_| extras.is_null() || extras.is_object())
            .map(|_| self.paint_set_index())
    }

    pub(super) fn paint_set_changed(&self) -> bool {
        self.writable_paint_set().is_some_and(|index| {
            self.source.json["extras"][PAINT_SET_EXTRA].as_u64().unwrap_or(0) != index as u64
        })
    }

    pub(super) fn model_camera_changed(&self) -> bool {
        self.model_camera.is_some_and(|camera| ModelCamera::read(&self.source.json) != Some(camera))
    }

    pub(super) fn paint_target(&self, set: usize) -> Option<(&Object, usize)> {
        self.objects.iter()
            .flat_map(|object| (0..object.materials.len()).map(move |slot| (object, slot)))
            .nth(set)
    }
    pub fn active(&self) -> Option<&Object> {
        let id = self.active?;
        self.objects.iter().find(|o| o.id == id)
    }
    pub fn active_mut(&mut self) -> Option<&mut Object> {
        let id = self.active?;
        self.objects.iter_mut().find(|o| o.id == id)
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            bake_settings: self.bake_settings.clone(),
            cage_edits: self.cage_edits.clone(),
            objects: self.objects.clone(),
            custom_bakes: self.custom_bakes.clone(),
            active: self.active,
            selected: self.selected.clone(),
            selection: self.selection.clone(),
        }
    }
    pub fn restore(&mut self, state: Snapshot) {
        self.objects = state.objects;
        self.custom_bakes = state.custom_bakes;
        self.bake_settings = state.bake_settings;
        self.cage_edits = state.cage_edits;
        self.active = state.active;
        self.selected = state.selected;
        self.selection = state.selection;
        self.revision += 1;
        self.ui_revision += 1;
    }
    pub fn checkpoint(&mut self, before: Snapshot) {
        if before.objects == self.objects && before.custom_bakes == self.custom_bakes
            && before.bake_settings == self.bake_settings
            && (Arc::ptr_eq(&before.cage_edits, &self.cage_edits)
                || before.cage_edits == self.cage_edits)
        {
            return;
        }
        self.undo.push(before);
        if self.undo.len() > 32 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.recovery_edits += 1;
        self.revision += 1;
        self.ui_revision += 1;
    }
    pub fn can_assign_material(&self) -> bool {
        self.mode == Mode::Edit && self.selection.kind == Element::Face
            && !self.selection.faces.is_empty()
            && self.active().is_some_and(|o| !o.hidden)
    }

    pub fn assign_material(
        &mut self,
        material: MaterialSlot,
        fallback: &Handle<StandardMaterial>,
    ) -> Result<(), String> {
        if !self.can_assign_material() {
            return Err("Выделите грани для назначения материала".into());
        }
        let faces = self.selection.selected_faces(&self.active().unwrap().geometry);
        if faces.is_empty() {
            return Err("Выделите грани для назначения материала".into());
        }
        let before = self.snapshot();
        let object = self.active_mut().unwrap();
        if object.materials.is_empty() {
            object.materials.push(MaterialSlot {
                bake_fixes: Default::default(),
                material: fallback.clone(),
                source: serde_json::json!({}),
                paint: None,
                authored: None,
            });
        }
        let slot = object.materials.iter().position(|slot| *slot == material)
            .unwrap_or_else(|| {
                let slot = object.materials.len();
                object.materials.push(material);
                slot
            });
        for face in faces {
            object.geometry.faces[face].material = slot;
        }
        self.checkpoint(before);
        Ok(())
    }
    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(previous);
            self.recovery_edits += 1;
        }
    }
    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(next);
            self.recovery_edits += 1;
        }
    }
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        if mode == Mode::Paint && self.paint_set.is_some()
            && let Some((object, _)) = self.paint_target(self.paint_set_index())
            && self.active != Some(object.id)
        {
            self.select_object(object.id, false);
        }
        if mode == Mode::Edit
            && self
                .selection
                .selected_vertices(
                    self.active()
                        .map(|o| &*o.geometry)
                        .unwrap_or(&Geometry::default()),
                )
                .is_empty()
        {
            if let Some(object) = self.active() {
                let mesh = object.geometry.clone();
                self.selection.all(&mesh);
            }
        }
        self.ui_revision += 1;
    }
    pub fn select_object(&mut self, id: usize, toggle: bool) {
        if !self.objects.iter().any(|o| o.id == id) {
            return;
        }
        if !toggle {
            self.selected.clear();
        }
        if toggle && !self.selected.insert(id) {
            self.selected.remove(&id);
        } else {
            self.selected.insert(id);
            self.active = Some(id);
        }
        self.selection.clear();
        self.ui_revision += 1;
    }
    pub fn selection_center(&self) -> Option<Vec3> {
        self.pivot_at(Pivot::Median)
    }

    pub fn pivot(&self) -> Option<Vec3> {
        self.pivot_at(self.pivot_mode)
    }

    fn pivot_at(&self, mode: Pivot) -> Option<Vec3> {
        let origin = self
            .active()
            .filter(|o| !o.hidden)
            .map(|o| o.transform.translation);
        if self.mode == Mode::Edit {
            let o = self.active().filter(|o| !o.hidden)?;
            let selected = self.selection.selected_vertices(&o.geometry);
            Self::pivot_from_points(
                mode,
                origin,
                selected
                    .iter()
                    .map(|&v| o.transform.transform_point(o.geometry.positions[v])),
            )
        } else {
            Self::pivot_from_points(
                mode,
                origin.filter(|_| self.active.is_some_and(|id| self.selected.contains(&id))),
                self.objects
                    .iter()
                    .filter(|o| self.selected.contains(&o.id) && !o.hidden)
                    .map(|o| o.transform.translation),
            )
        }
    }

    fn pivot_from_points(
        mode: Pivot,
        origin: Option<Vec3>,
        mut points: impl Iterator<Item = Vec3>,
    ) -> Option<Vec3> {
        let first = points.next()?;
        Some(match mode {
            Pivot::WorldOrigin => Vec3::ZERO,
            Pivot::ObjectOrigin => origin.unwrap_or(first),
            Pivot::Bounds => {
                let (min, max) =
                    points.fold((first, first), |(min, max), p| (min.min(p), max.max(p)));
                (min + max) * 0.5
            }
            Pivot::Median => {
                let (sum, count) = points.fold((first, 1), |(sum, count), p| (sum + p, count + 1));
                sum / count as f32
            }
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut count = 0;
        for o in &self.objects {
            count += o.geometry.positions.len();
            if !o.transform.translation.is_finite()
                || !o.transform.rotation.is_finite()
                || !o.transform.scale.is_finite()
                || o.transform.scale.abs().min_element() < 1.0e-7
            {
                return Err("Недопустимое преобразование объекта".into());
            }
            o.geometry.validate()?;
        }
        if count > 250_000 {
            return Err(
                "Для интерактивного редактирования поддерживается до 250 000 вершин".into(),
            );
        }
        Ok(())
    }
    pub fn transform(&mut self, base: &Snapshot, matrix: Mat4, proportional: Option<(f32, bool)>) {
        self.objects = base.objects.clone();
        if self.mode == Mode::Object {
            for o in &mut self.objects {
                if base.selected.contains(&o.id) {
                    let target = matrix * o.transform.to_matrix();
                    let mut transform = Transform::from_matrix(target);
                    transform.rotation = transform.rotation.normalize();
                    // A world-axis scale of a rotated object may contain shear.
                    // Bake only the residual into its owned mesh to preserve the
                    // exact shape while keeping the inspector's TRS editable.
                    let residual = transform.to_matrix().inverse() * target;
                    if !residual.abs_diff_eq(Mat4::IDENTITY, 0.00001) {
                        let normal_matrix = residual.inverse().transpose();
                        for p in &mut o.geometry.positions {
                            *p = residual.transform_point3(*p);
                        }
                        for face in &mut o.geometry.faces {
                            for c in &mut face.corners {
                                c.source_normal = c.source_normal.map(|n| {
                                    normal_matrix.transform_vector3(n).normalize_or_zero()
                                });
                            }
                        }
                    }
                    o.transform = transform;
                }
            }
        } else if let Some(o) = self.active_mut() {
            let matrix = o.transform.to_matrix().inverse() * matrix * o.transform.to_matrix();
            o.geometry.transform(&base.selection, matrix, proportional);
            if !matrix.abs_diff_eq(Mat4::IDENTITY, 1.0e-7) {
                o.geometry.discard_normals();
            }
        }
        self.revision += 1;
    }
    #[cfg(test)]
    pub fn save(&mut self) -> Result<(), String> {
        self.validate()?;
        let mut source = self.source_for_save();
        super::model_bake_settings::write(&mut source.json, &source.path, &self.bake_settings)?;
        source.save_with_cage(&self.objects, &self.original, Some(&self.cage_edits))?;
        self.source = source;
        self.saved = self.objects.clone();
        self.saved_bake_settings = self.bake_settings.clone();
        self.saved_recovery_edits = self.recovery_edits;
        self.saved_cage_edits = self.cage_edits.clone();
        self.ui_revision += 1;
        Ok(())
    }
    pub fn operate(&mut self, operation: Operation) -> Result<(), String> {
        let before = self.snapshot();
        let result = self.apply(operation).and_then(|_| {
            if !matches!(
                operation,
                Operation::ProjectUv(_) | Operation::SphericalUv
                    | Operation::MarkUvSeam | Operation::ClearUvSeam
            ) {
                for object in &mut self.objects {
                    if before
                        .objects
                        .iter()
                        .find(|old| old.id == object.id)
                        .is_some_and(|old| old.geometry != object.geometry)
                    {
                        object.geometry.prune_uv_seams();
                        object.geometry.discard_normals();
                    }
                }
            }
            self.validate()
        });
        match result {
            Ok(()) => {
                self.checkpoint(before);
                Ok(())
            }
            Err(error) => {
                self.restore(before);
                Err(error)
            }
        }
    }

    /// Build each preview from the original mesh, leaving the undo stack alone.
    pub fn preview_bevel(
        &mut self,
        base: &Snapshot,
        vertices: bool,
        options: BevelOptions,
    ) -> Result<(), String> {
        if options.amount == 0.0 {
            self.restore(base.clone());
            return Ok(());
        }
        let mut preview = base.clone();
        let object = preview
            .objects
            .iter_mut()
            .find(|o| Some(o.id) == base.active && !o.hidden)
            .ok_or("Выберите объект для фаски")?;
        if options.material.is_some_and(|index| index >= object.materials.len()) {
            return Err("Выберите существующий материал для фаски".into());
        }
        let faces = if vertices {
            let selected = preview.selection.selected_vertices(&object.geometry);
            object.geometry.bevel_vertices_with(&selected, options)?
        } else {
            let geometry = &object.geometry;
            let edges = match preview.selection.kind {
                Element::Edge => preview.selection.edges.clone(),
                Element::Face => preview.selection.selected_faces(geometry)
                    .into_iter()
                    .flat_map(|i| geometry.faces[i].edges().map(|(a, b)| Edge::new(a, b)))
                    .collect(),
                Element::Vertex => geometry.edges().into_keys()
                    .filter(|edge| preview.selection.vertices.contains(&edge.0)
                        && preview.selection.vertices.contains(&edge.1))
                    .collect(),
            };
            object.geometry.bevel_edges_with(&edges, options)?
        };
        if !faces.is_empty() {
            preview.selection.select_face_elements(&object.geometry, faces);
        }
        object.geometry.prune_uv_seams();
        if !(options.harden_normals || options.preserve_sharp) {
            object.geometry.discard_normals();
        }
        object.geometry.validate()?;
        if preview.objects.iter().map(|o| o.geometry.positions.len()).sum::<usize>() > 250_000 {
            return Err("Фаска превысит лимит модели в 250 000 вершин".into());
        }
        self.restore(preview);
        Ok(())
    }
    fn apply(&mut self, operation: Operation) -> Result<(), String> {
        if self.mode == Mode::Object {
            return self.object_operation(operation);
        }
        let id = self.active.ok_or("Выберите объект")?;
        let index = self
            .objects
            .iter()
            .position(|o| o.id == id)
            .ok_or("Объект не найден")?;
        let radius = self.objects[index].geometry.radius();
        let amount = self.amount.max(radius * 0.00001);
        if matches!(operation, Operation::Separate) {
            let chosen = self.selection.selected_faces(&self.objects[index].geometry);
            if chosen.is_empty() {
                return Err("Выберите грани для отделения".into());
            }
            let mut object = self.objects[index].clone();
            object.id = self.objects.iter().map(|o| o.id).max().unwrap_or(0) + 1;
            object.source_node = None;
            object.name = format!("{} · отделено", object.name);
            object.geometry.faces = object
                .geometry
                .faces
                .iter()
                .enumerate()
                .filter(|(i, _)| chosen.contains(i))
                .map(|(_, f)| f.clone())
                .collect();
            object.geometry.loose_edges.clear();
            object.geometry.compact();
            self.objects[index]
                .geometry
                .delete(&mut self.selection, true);
            self.objects.push(object);
            return Ok(());
        }
        let mesh = &mut self.objects[index].geometry;
        match operation {
            Operation::Extrude => mesh.extrude(&mut self.selection, amount)?,
            Operation::Inset => mesh.inset(&mut self.selection, amount)?,
            Operation::Subdivide => mesh.subdivide(&mut self.selection)?,
            Operation::Triangulate => mesh.triangulate(&mut self.selection),
            Operation::Quads => mesh.dissolve(&mut self.selection, true)?,
            Operation::Dissolve => mesh.dissolve(&mut self.selection, false)?,
            Operation::Delete => mesh.delete(&mut self.selection, false),
            Operation::DeleteFaces => mesh.delete(&mut self.selection, true),
            Operation::Duplicate => mesh.duplicate(&mut self.selection)?,
            Operation::MergeCenter => mesh.merge(&mut self.selection, None)?,
            Operation::MergeDistance => mesh.merge(&mut self.selection, Some(amount))?,
            Operation::Fill => mesh.fill(&mut self.selection)?,
            Operation::FlipNormals => mesh.recalculate_normals(&self.selection, true),
            Operation::RecalculateNormals => mesh.recalculate_normals(&self.selection, false),
            Operation::Smooth => mesh.smooth_vertices(&self.selection, 0.5),
            Operation::ShadeSmooth | Operation::ShadeFlat => {
                let faces = self.selection.selected_faces(mesh);
                for i in faces {
                    mesh.faces[i].smooth = matches!(operation, Operation::ShadeSmooth);
                    for c in &mut mesh.faces[i].corners {
                        c.source_normal = None;
                    }
                }
            }
            Operation::Mirror(axis) => {
                let vertices = self.selection.selected_vertices(mesh);
                if vertices.is_empty() {
                    return Err("Нет выделенных вершин".into());
                }
                let pivot = vertices.iter().map(|&v| mesh.positions[v]).sum::<Vec3>()
                    / vertices.len() as f32;
                for v in vertices {
                    mesh.positions[v][axis] = 2.0 * pivot[axis] - mesh.positions[v][axis];
                }
                let selected = self.selection.selected_faces(mesh);
                for i in selected {
                    mesh.faces[i].corners.reverse();
                }
            }
            Operation::LoopCut => {
                let edge = self
                    .selection
                    .edges
                    .first()
                    .copied()
                    .or_else(|| {
                        self.selection
                            .selected_faces(mesh)
                            .first()
                            .and_then(|&i| mesh.faces[i].edges().next())
                            .map(|(a, b)| super::model_mesh::Edge::new(a, b))
                    })
                    .ok_or("Выберите ребро для разреза петли")?;
                let edges = mesh.loop_cut(edge)?;
                self.selection.clear();
                self.selection.kind = Element::Edge;
                self.selection.edges = edges;
            }
            Operation::SlideEdges { reverse } => {
                if self.selection.kind != Element::Edge {
                    return Err("Переключитесь на выбор рёбер".into());
                }
                let fraction = self.slide_percent / 100.0;
                mesh.slide_edges(
                    &self.selection.edges,
                    if reverse { -fraction } else { fraction },
                )?;
            }
            Operation::BevelVertices => mesh.bevel_vertices(&mut self.selection, amount)?,
            Operation::BevelEdges => {
                if self.selection.kind != Element::Edge {
                    return Err("Переключитесь на выбор рёбер".into());
                }
                let faces = mesh.bevel_edges(&self.selection.edges, amount)?;
                self.selection.select_face_elements(mesh, faces);
            }
            Operation::MarkUvSeam | Operation::ClearUvSeam => {
                if self.selection.kind != Element::Edge || self.selection.edges.is_empty() {
                    return Err("Выделите рёбра для разметки UV-швов".into());
                }
                if operation == Operation::MarkUvSeam {
                    mesh.uv_seams.extend(self.selection.edges.iter().copied());
                    mesh.prune_uv_seams();
                } else {
                    mesh.uv_seams.retain(|edge| !self.selection.edges.contains(edge));
                }
            }
            Operation::ProjectUv(axis) => mesh.project_uv(&self.selection, axis, false),
            Operation::SphericalUv => mesh.project_uv(&self.selection, 0, true),
            _ => return Err("Операция доступна в режиме «Объект»".into()),
        }
        Ok(())
    }
    fn object_operation(&mut self, operation: Operation) -> Result<(), String> {
        if self.selected.is_empty() {
            return Err("Выберите объект".into());
        }
        match operation {
            Operation::Delete => {
                self.objects.retain(|o| !self.selected.contains(&o.id));
                self.active = self.objects.first().map(|o| o.id);
                self.selected = self.active.into_iter().collect();
            }
            Operation::Duplicate => {
                let mut next = self.objects.iter().map(|o| o.id).max().unwrap_or(0) + 1;
                let mut copies = Vec::new();
                for o in &self.objects {
                    if self.selected.contains(&o.id) {
                        let mut copy = o.clone();
                        copy.id = next;
                        next += 1;
                        copy.source_node = None;
                        copy.name = format!("{}.001", copy.name);
                        copies.push(copy);
                    }
                }
                self.selected = copies.iter().map(|o| o.id).collect();
                self.active = copies.first().map(|o| o.id);
                self.objects.extend(copies);
            }
            Operation::Join => {
                let active = self.active.ok_or("Нет активного объекта")?;
                let index = self
                    .objects
                    .iter()
                    .position(|o| o.id == active)
                    .ok_or("Нет активного объекта")?;
                let inverse = self.objects[index].transform.to_matrix().inverse();
                let others: Vec<_> = self
                    .objects
                    .iter()
                    .filter(|o| o.id != active && self.selected.contains(&o.id))
                    .cloned()
                    .collect();
                if others.is_empty() {
                    return Err("Для объединения выберите несколько объектов с Shift".into());
                }
                for o in others {
                    let offset = self.objects[index].geometry.positions.len();
                    let material_offset = self.objects[index].materials.len();
                    let m = inverse * o.transform.to_matrix();
                    self.objects[index]
                        .geometry
                        .positions
                        .extend(o.geometry.positions.iter().map(|p| m.transform_point3(*p)));
                    self.objects[index]
                        .geometry
                        .faces
                        .extend(o.geometry.faces.iter().map(|f| {
                            let mut f = f.clone();
                            f.material += material_offset;
                            for c in &mut f.corners {
                                c.vertex += offset;
                            }
                            if m.determinant() < 0.0 {
                                f.corners.reverse();
                            }
                            f
                        }));
                    self.objects[index].materials.extend(o.materials);
                    self.objects[index].geometry.loose_edges.extend(
                        o.geometry.loose_edges.iter().map(|e| Edge::new(e.0 + offset, e.1 + offset))
                    );
                    self.objects[index].geometry.uv_seams.extend(
                        o.geometry.uv_seams.iter().map(|e| Edge::new(e.0 + offset, e.1 + offset))
                    );
                }
                self.objects
                    .retain(|o| o.id == active || !self.selected.contains(&o.id));
                self.selected = [active].into();
            }
            Operation::ApplyTransform => {
                for o in &mut self.objects {
                    if self.selected.contains(&o.id) {
                        let m = Mat4::from_scale_rotation_translation(
                            o.transform.scale,
                            o.transform.rotation,
                            Vec3::ZERO,
                        );
                        for p in &mut o.geometry.positions {
                            *p = m.transform_point3(*p);
                        }
                        if o.transform.scale.element_product() < 0.0 {
                            for f in &mut o.geometry.faces {
                                f.corners.reverse();
                            }
                        }
                        o.transform.rotation = Quat::IDENTITY;
                        o.transform.scale = Vec3::ONE;
                    }
                }
            }
            Operation::OriginToGeometry => {
                for o in &mut self.objects {
                    if self.selected.contains(&o.id) && !o.geometry.positions.is_empty() {
                        let center = o.geometry.positions.iter().sum::<Vec3>()
                            / o.geometry.positions.len() as f32;
                        for p in &mut o.geometry.positions {
                            *p -= center;
                        }
                        o.transform.translation = o.transform.transform_point(center);
                    }
                }
            }
            Operation::ShadeFlat | Operation::ShadeSmooth => {
                for o in &mut self.objects {
                    if self.selected.contains(&o.id) {
                        for f in &mut o.geometry.faces {
                            f.smooth = matches!(operation, Operation::ShadeSmooth);
                            for c in &mut f.corners {
                                c.source_normal = None;
                            }
                        }
                    }
                }
            }
            Operation::Mirror(axis) => {
                for o in &mut self.objects {
                    if self.selected.contains(&o.id) {
                        for p in &mut o.geometry.positions {
                            p[axis] = -p[axis];
                        }
                        for f in &mut o.geometry.faces {
                            f.corners.reverse();
                        }
                    }
                }
            }
            _ => return Err("Для правки топологии включите «Редактирование»".into()),
        }
        Ok(())
    }
}

pub(super) fn from_mesh(
    mesh: &Mesh,
    geometry: &mut Geometry,
    material: usize,
) -> Result<(), String> {
    from_mesh_with_topology(mesh, geometry, material, None, None)
}

pub(super) fn from_mesh_with_topology(
    mesh: &Mesh,
    geometry: &mut Geometry,
    material: usize,
    polygons: Option<&serde_json::Value>,
    id_colors: Option<&[Option<Vec3>]>,
) -> Result<(), String> {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return Err("В меше нет координат вершин".into());
    };
    let normals = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(v)) => Some(v),
        _ => None,
    };
    let uv = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(v)) => Some(v),
        _ => None,
    };
    let uv1 = match mesh.attribute(Mesh::ATTRIBUTE_UV_1) {
        Some(VertexAttributeValues::Float32x2(v)) => Some(v),
        _ => None,
    };
    let colors = match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
        Some(VertexAttributeValues::Float32x4(v)) => Some(v),
        _ => None,
    };
    let use_visible_colors = colors.is_some_and(|values| {
        polygons.is_none() || values.iter().any(|rgba| *rgba != [1.; 4])
    });
    if [
        normals.map(Vec::len),
        uv.map(Vec::len),
        uv1.map(Vec::len),
        colors.map(Vec::len),
        id_colors.map(<[_]>::len),
    ]
    .into_iter()
    .flatten()
    .any(|count| count != positions.len())
    {
        return Err("Число нормалей, UV или цветов не совпадает с числом вершин".into());
    }
    let mut shared: HashMap<[u32; 3], usize> = geometry
        .positions
        .iter()
        .enumerate()
        .map(|(i, p)| {
            (
                p.to_array().map(|x| if x == 0.0 { 0 } else { x.to_bits() }),
                i,
            )
        })
        .collect();
    let mapped: Vec<_> = positions
        .iter()
        .map(|p| {
            *shared
                .entry(p.map(|x| if x == 0.0 { 0 } else { x.to_bits() }))
                .or_insert_with(|| {
                    let i = geometry.positions.len();
                    geometry.positions.push(Vec3::from_array(*p));
                    i
                })
        })
        .collect();
    let indices: Vec<_> = mesh
        .indices()
        .map(|v| v.iter().collect())
        .unwrap_or_else(|| (0..positions.len()).collect());
    if mesh.primitive_topology() == PrimitiveTopology::LineList {
        if indices.len() % 2 != 0 || indices.iter().any(|&i| i >= mapped.len()) {
            return Err("Некорректные индексы свободных рёбер".into());
        }
        geometry.loose_edges.extend(indices.chunks_exact(2)
            .map(|e| Edge::new(mapped[e[0]], mapped[e[1]]))
            .filter(|e| e.0 != e.1));
        return Ok(());
    }
    let triangles: Vec<[usize; 3]> = match mesh.primitive_topology() {
        PrimitiveTopology::TriangleList => indices
            .chunks_exact(3)
            .map(|v| [v[0], v[1], v[2]])
            .collect(),
        PrimitiveTopology::TriangleStrip => indices
            .windows(3)
            .enumerate()
            .map(|(i, v)| {
                if i % 2 == 0 {
                    [v[0], v[1], v[2]]
                } else {
                    [v[1], v[0], v[2]]
                }
            })
            .collect(),
        _ => return Err("Редактирование поддерживает треугольные поверхности".into()),
    };
    if triangles.iter().flatten().any(|&v| v >= positions.len()) {
        return Err("Некорректные индексы геометрии".into());
    }
    let make_face = |vertices: &[usize]| {
        let smooth = normals.is_some_and(|n| {
            let a = Vec3::from_array(n[vertices[0]]);
            vertices
                .iter()
                .any(|&i| a.distance_squared(Vec3::from_array(n[i])) > 0.0001)
        });
        Face {
            corners: vertices
                .iter()
                .map(|&i| Corner {
                    vertex: mapped[i],
                    uv: uv.map_or(Vec2::ZERO, |v| Vec2::from_array(v[i])),
                    uv1: uv1.map_or(Vec2::ZERO, |v| Vec2::from_array(v[i])),
                    color: colors.map_or(Vec4::ONE, |v| Vec4::from_array(v[i])),
                    id_color: id_colors.map_or_else(
                        || colors.filter(|_| use_visible_colors).map(|values| Vec4::from_array(values[i]).truncate()),
                        |values| values[i],
                    ),
                    source_normal: normals
                        .map(|v| Vec3::from_array(v[i]).normalize_or_zero())
                        .filter(|n| n.length_squared() > 0.0),
                })
                .collect(),
            material,
            smooth,
        }
    };
    if let Some(polygons) = polygons {
        let error = "Повреждена полигональная топология модели";
        let polygons: Vec<Vec<usize>> =
            serde_json::from_value(polygons.clone()).map_err(|_| error.to_owned())?;
        if polygons.len() > triangles.len()
            || polygons
                .iter()
                .any(|p| !(3..=4096).contains(&p.len()) || p.iter().any(|&i| i >= positions.len()))
        {
            return Err(error.into());
        }
        let faces: Vec<_> = polygons.iter().map(|p| make_face(p)).collect();
        fn counts(triangles: impl Iterator<Item = [usize; 3]>) -> HashMap<[usize; 3], usize> {
            let mut counts = HashMap::new();
            for mut triangle in triangles {
                let first = (0..3).min_by_key(|&i| triangle[i]).unwrap();
                triangle.rotate_left(first);
                *counts.entry(triangle).or_default() += 1;
            }
            counts
        }
        let restored = counts(faces.iter().zip(&polygons).flat_map(|(face, polygon)| {
            geometry
                .triangles(face)
                .into_iter()
                .map(move |t| t.map(|i| polygon[i]))
        }));
        if restored != counts(triangles.into_iter()) {
            return Err(error.into());
        }
        geometry.faces.extend(faces);
        return Ok(());
    }
    let first_face = geometry.faces.len();
    for triangle in triangles {
        if mapped[triangle[0]] == mapped[triangle[1]]
            || mapped[triangle[1]] == mapped[triangle[2]]
            || mapped[triangle[0]] == mapped[triangle[2]]
        {
            continue;
        }
        geometry.faces.push(make_face(&triangle));
    }
    // glTF and Bevy render triangles. Restore polygon pairs for editing,
    // including primitives created in the editor, without crossing material slots.
    let mut selection = Selection {
        kind: super::model_mesh::Element::Face,
        faces: (first_face..geometry.faces.len()).collect(),
        ..default()
    };
    // No compatible pair is a valid result for a triangular surface.
    let _ = geometry.dissolve(&mut selection, true);
    Ok(())
}

pub(super) fn render_mesh(buffers: super::model_mesh::Buffers) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, buffers.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, buffers.normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, buffers.uv);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, buffers.uv1);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, buffers.colors);
    mesh.insert_indices(Indices::U32(buffers.indices));
    let _ = mesh.generate_tangents();
    mesh
}

pub(super) fn source_path(root: &std::path::Path, asset: &str) -> Result<PathBuf, String> {
    crate::asset_paths::resolve_asset_path(root, asset)
        .map(|path| root.join(path))
        .map_err(|e| e.to_string())
}
