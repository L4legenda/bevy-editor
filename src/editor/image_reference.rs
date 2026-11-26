//! Render a model into a transparent PhotoCraft reference layer.
use std::{
    collections::VecDeque,
    path::Path,
    time::{Duration, Instant},
};

use bevy::{
    camera::{
        ClearColorConfig, Exposure, Hdr, RenderTarget, ScalingMode, primitives::Aabb,
        visibility::RenderLayers,
    },
    core_pipeline::tonemapping::Tonemapping,
    gltf::Gltf,
    prelude::*,
    render::{
        MainWorld,
        gpu_readback::{ReadbackComplete, ReadbackOnce},
        render_resource::{PipelineCache, TextureFormat, TextureUsages},
    },
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    window::RequestRedraw,
};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect as PixelRect;
use photocraft_raster::Surface;

use super::{
    Action, Editor,
    browser::{Browser, Entry},
    model_source::ModelSource,
    thumbnails::{framing, scene},
    ui::ImageViewer,
};

#[path = "image_reference_cpu.rs"]
mod cpu;
#[path = "image_reference_live.rs"]
pub(super) mod live;
#[path = "image_reference_rig.rs"]
pub(super) mod rig;

const LAYER: usize = 22;
const RESOLUTION: u32 = 1024;
const MAX_CAPTURES: u8 = 3;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Component)]
pub(super) struct ReferenceCamera;

/// Authoring data travels with the raster preview through save, duplication and undo.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(in crate::editor) struct ModelReference {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    pub asset: String,
    pub direction: [f32; 3],
    pub center: [f32; 2],
    pub pose: crate::Pose,
    #[serde(default, skip_serializing_if = "rig::Rotations::is_empty")]
    pub(super) joints: rig::Rotations,
    #[serde(default)]
    pub pixels_per_unit: f32,
    // Present only between an authoring command and its matching raster. The
    // save path checks this before publishing a document or its flat image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) pending_frame: Option<u64>,
}

#[derive(Clone, PartialEq)]
pub(super) struct ReferencePose {
    pub root: crate::Pose,
    pub joints: rig::Rotations,
}

impl ModelReference {
    const START: &str = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:bevy=\"urn:bevy-editor:model-reference:1\"><bevy:reference><![CDATA[";
    const END: &str = "]]></bevy:reference></rdf:Description></rdf:RDF></x:xmpmeta>";

    pub(super) fn to_xmp(&self) -> Result<std::sync::Arc<str>, String> {
        let body = serde_json::to_string(self)
            .map_err(|error| error.to_string())?
            .replace('>', "\\u003e");
        Ok(format!("{}{body}{}", Self::START, Self::END).into())
    }

    pub(super) fn from_xmp(packet: &str) -> Option<Self> {
        let body = packet.strip_prefix(Self::START)?.strip_suffix(Self::END)?;
        let reference: Self = serde_json::from_str(body).ok()?;
        (reference.pose.valid()
            && rig::valid(&reference.joints)
            && reference.pixels_per_unit.is_finite()
            && reference.pixels_per_unit >= 0.0
            && Vec3::from(reference.direction).is_finite()
            && Vec2::from(reference.center).is_finite()
            && crate::data::ModelFormat::from_path(&reference.asset).is_some())
        .then_some(reference)
    }

    pub(super) fn reference_pose(&self) -> ReferencePose {
        ReferencePose {
            root: self.pose,
            joints: self.joints.clone(),
        }
    }

    pub(super) fn resolved(mut self, root: &Path) -> Self {
        if let Ok(index) = crate::asset_registry::index(root) {
            index.read_ref(&mut self.asset, &mut self.uid);
        }
        self
    }

    pub(super) fn bind_layers(
        layers: &mut [photocraft_doc::Layer],
        index: &crate::asset_registry::Index,
    ) -> Result<(), String> {
        for layer in layers {
            if let Some(mut model) = layer.xmp.as_deref().and_then(Self::from_xmp) {
                if model.pending_frame.is_some() {
                    return Err("Для 3D-слоя ещё не готов кадр. Повторите сохранение.".into());
                }
                index.save_ref(&mut model.asset, &mut model.uid);
                layer.xmp = Some(model.to_xmp()?);
            }
            if let Some(children) = layer.children_mut() {
                Self::bind_layers(children, index)?;
            }
        }
        Ok(())
    }
    pub(super) fn view(&self) -> Transform {
        framing::fit(
            crate::BoxBounds {
                center: [0.0; 3],
                half_size: [1.0; 3],
            },
            self.direction.into(),
            1.0,
        )
        .0
    }

    pub(super) fn canvas_center(&self) -> Vec2 {
        let view = self.view();
        let position = Vec3::from(self.pose.position);
        Vec2::from(self.center)
            + Vec2::new(position.dot(*view.right()), -position.dot(*view.up()))
                * self.pixels_per_unit
    }
}

#[derive(Clone, PartialEq)]
pub(in crate::editor) struct ModelView {
    pub document: photocraft_doc::DocId,
    pub layer: Option<photocraft_doc::LayerId>,
    pub asset: String,
    direction: [f32; 3],
    center: [f32; 2],
    pub size: UVec2,
    pub pose: ReferencePose,
}

impl ModelView {
    pub(in crate::editor) fn new(
        document: photocraft_doc::DocId,
        layer: Option<photocraft_doc::LayerId>,
        model: &ModelReference,
        size: UVec2,
        pose: ReferencePose,
    ) -> Self {
        Self {
            document,
            layer,
            asset: model.asset.clone(),
            direction: model.direction,
            center: model.center,
            size,
            pose,
        }
    }

    pub(in crate::editor) fn same_layer_view(&self, other: &Self) -> bool {
        self.document == other.document
            && self.layer == other.layer
            && self.asset == other.asset
            && self.direction == other.direction
            && self.center == other.center
            && self.size == other.size
    }
}

struct Request {
    tab: usize,
    uid: Option<String>,
    view: ModelView,
    replace: Option<ReferenceTarget>,
    started: Option<Instant>,
}

#[derive(Clone)]
pub(super) struct ReferenceTarget {
    pub packet: std::sync::Arc<str>,
    pub previous_pose: ReferencePose,
}

#[derive(Clone)]
struct ReferenceFrame {
    view: Transform,
    projection: Projection,
    center: Vec2,
    pixel_span: f32,
    pixels_per_unit: f32,
}

fn pixel_density(half: Vec3, view: &Transform, size: UVec2) -> f32 {
    let extent = half
        .dot(view.right().abs())
        .max(half.dot(view.up().abs()))
        .max(1e-5);
    (size.min_element() as f32 * 0.6).min(RESOLUTION as f32) / (2.0 * extent)
}

fn projected_frame(
    bounds: crate::BoxBounds,
    direction: Vec3,
    origin: Vec2,
    size: UVec2,
    pose: crate::Pose,
    joints: Vec<Vec3>,
) -> (Transform, ReferenceFrame) {
    let (mut view, _) = framing::fit(bounds, direction, 1.0);
    let pivot = Vec3::from(bounds.center);
    let half = Vec3::from(bounds.half_size);
    let (right, up, back) = (*view.right(), *view.up(), *view.back());
    let mut pose = pose.transform();
    let mut extent = Vec3::ZERO;
    for x in [-1.0, 1.0] {
        for y in [-1.0, 1.0] {
            for z in [-1.0, 1.0] {
                let offset = pose.rotation * (pose.scale * half * Vec3::new(x, y, z));
                extent = extent
                    .max(Vec3::new(offset.dot(right), offset.dot(up), offset.dot(back)).abs());
            }
        }
    }
    // Keep the authored canvas density when a limb moves past the original bounds.
    let joint_padding = (pose.scale.abs() * half).length() * 0.15;
    for joint in joints {
        let offset = pose.rotation * (pose.scale * (joint - pivot));
        extent = extent.max(
            Vec3::new(offset.dot(right), offset.dot(up), offset.dot(back)).abs()
                + Vec3::splat(joint_padding),
        );
    }
    let pixels_per_unit = pixel_density(half, &view, size);
    let displacement = pose.translation;
    let center =
        origin + Vec2::new(displacement.dot(right), -displacement.dot(up)) * pixels_per_unit;
    let span = (2.16 * extent.x.max(extent.y)).max(1e-5);
    let distance = extent.z + span;
    view.translation = pivot + displacement + back * distance;
    let projection = Projection::Orthographic(OrthographicProjection {
        far: distance + extent.z + span,
        scaling_mode: ScalingMode::Fixed {
            width: span,
            height: span,
        },
        area: Rect::from_center_size(Vec2::ZERO, Vec2::splat(span)),
        ..OrthographicProjection::default_3d()
    });
    pose.translation += pivot - pose.rotation * (pose.scale * pivot);
    (
        pose,
        ReferenceFrame {
            view,
            projection,
            center,
            pixel_span: span * pixels_per_unit,
            pixels_per_unit,
        },
    )
}

#[derive(Resource, Default)]
pub(super) struct ReferenceRender {
    pub(super) live: live::Preview,
    queue: VecDeque<Request>,
    failed: Vec<Request>,
}

impl ReferenceRender {
    pub(super) fn pending_reference_pose(
        &self,
        tab: usize,
        layer: photocraft_doc::LayerId,
    ) -> Option<&ReferencePose> {
        self.queue
            .iter()
            .rev()
            .find(|r| r.tab == tab && r.view.layer == Some(layer))
            .map(|r| &r.view.pose)
    }

    pub(in crate::editor) fn previous_pose(
        &self,
        packet: &std::sync::Arc<str>,
    ) -> Option<&ReferencePose> {
        self.queue
            .iter()
            .chain(&self.failed)
            .filter_map(|r| r.replace.as_ref())
            .find(|target| &target.packet == packet)
            .map(|target| &target.previous_pose)
    }

    pub(super) fn retry_failed(&mut self, tab: usize) {
        let retry = self.failed.extract_if(.., |job| job.tab == tab);
        let mut restarted = false;
        for mut job in retry {
            job.started = None;
            self.queue.push_back(job);
            restarted = true;
        }
        if restarted {
            self.live.retry();
        }
    }

    pub(super) fn transform_selected(
        &mut self,
        tab: usize,
        photo: &mut super::photo_document::PhotoDocument,
        pose: crate::Pose,
    ) -> Result<(), String> {
        self.edit_selected(tab, photo, |model| model.pose = pose)
    }

    pub(super) fn pose_selected(
        &mut self,
        tab: usize,
        photo: &mut super::photo_document::PhotoDocument,
        joints: rig::Rotations,
    ) -> Result<(), String> {
        self.edit_selected(tab, photo, |model| model.joints = joints)
    }

    fn edit_selected(
        &mut self,
        tab: usize,
        photo: &mut super::photo_document::PhotoDocument,
        edit: impl FnOnce(&mut ModelReference),
    ) -> Result<(), String> {
        let layer = photo.active_layer().ok_or("Слой больше не существует")?;
        let mut model = layer
            .xmp
            .as_deref()
            .and_then(ModelReference::from_xmp)
            .ok_or("Выберите слой с 3D-моделью")?;
        let before = model.reference_pose();
        edit(&mut model);
        if !model.pose.valid()
            || !rig::valid(&model.joints)
            || layer.locks.position
            || layer.locks.all
        {
            return Err("Трансформация слоя недоступна".into());
        }
        let pose = model.reference_pose();
        if pose == before {
            return Ok(());
        }
        let previous_pose = layer
            .xmp
            .as_ref()
            .and_then(|p| self.previous_pose(p))
            .cloned()
            .unwrap_or(before);
        let id = layer.id;
        let size = photo.size();
        let view = ModelView::new(
            photo.document.id,
            Some(id),
            &model,
            UVec2::new(size.width, size.height),
            pose,
        );
        let packet = photo.author_reference(id, &mut model)?;
        self.queue.push_back(Request {
            tab,
            uid: model.uid,
            view,
            replace: Some(ReferenceTarget {
                packet,
                previous_pose,
            }),
            started: None,
        });
        Ok(())
    }

    pub(super) fn pending_for(&self, document: Option<usize>) -> bool {
        self.queue.iter().any(|r| Some(r.tab) == document)
    }
}

pub(super) fn update(
    mut commands: Commands,
    mut actions: MessageReader<Action>,
    mut editor: ResMut<Editor>,
    mut viewer: ResMut<ImageViewer>,
    browser: Res<Browser>,
    mut render: ResMut<ReferenceRender>,
    assets: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    mut images: ResMut<Assets<Image>>,
    prepared: Query<(), With<scene::Prepared>>,
    subjects: Query<(), With<scene::Subject>>,
    children: Query<&Children>,
    meshes: Query<(&Aabb, &GlobalTransform)>,
    geometry: cpu::Geometry,
    (rig_source, device): (rig::Source, Res<bevy::render::renderer::RenderDevice>),
    mut redraw: MessageWriter<RequestRedraw>,
) {
    for &action in actions.read() {
        let Action::DropModelReference {
            entry,
            scan_revision,
            document,
            point,
        } = action
        else {
            continue;
        };
        if scan_revision != browser.scan_revision
            || editor.viewing_image != Some(document)
            || !viewer.edit.editing(Some(document))
            || editor.pending.is_some()
            || editor.browser_popup
            || editor.hdri_library_open
            || !point.is_finite()
        {
            continue;
        }
        let Some(doc) = viewer.edit.document(Some(document)) else {
            continue;
        };
        let Some(asset) = browser.model_asset(&editor.project.catalog, entry) else {
            continue;
        };
        let direction = match entry {
            Entry::Model(index) => editor
                .project
                .catalog
                .get(index)
                .map_or(Vec3::ZERO, |model| Vec3::from(model.preview_direction)),
            _ => Vec3::ZERO,
        };
        let size = doc.size();
        let uid = crate::asset_registry::index(&editor.root)
            .ok()
            .and_then(|i| i.uid(&asset).map(str::to_owned));
        render.queue.push_back(Request {
            tab: document,
            uid,
            started: None,
            replace: None,
            view: ModelView {
                document: doc.document.id,
                layer: None,
                asset,
                direction: direction.to_array(),
                center: point.to_array(),
                size: UVec2::new(size.width, size.height),
                pose: ReferencePose {
                    root: Transform::IDENTITY.into(),
                    joints: default(),
                },
            },
        });
        viewer.edit.error = None;
        editor.status = "Рендер слоя-референса…".into();
    }

    let render = &mut *render;
    render
        .queue
        .retain(|job| editor.image_tab(job.tab).is_some());
    render
        .failed
        .retain(|job| editor.image_tab(job.tab).is_some());
    if let Some(job) = render.queue.front_mut()
        && job.started.is_none()
    {
        if let Ok(index) = crate::asset_registry::index(&editor.root) {
            index.read_ref(&mut job.view.asset, &mut job.uid);
        }
        job.started = Some(Instant::now());
    }
    let preview = viewer
        .layer_preview
        .as_ref()
        .and_then(|p| p.camera_request());
    let selected = viewer
        .edit
        .editing(editor.viewing_image)
        .then(|| viewer.edit.model_reference(editor.viewing_image))
        .flatten()
        .map(|model| model.resolved(&editor.root).asset);
    let job = render.queue.front();
    let view = job
        .map(|job| &job.view)
        .or_else(|| preview.as_ref().map(|(view, _)| view));
    let asset = view.map(|view| view.asset.as_str()).or(selected.as_deref());
    render.live.load(
        asset,
        &mut commands,
        &assets,
        &gltfs,
        &images,
        &prepared,
        &subjects,
        &children,
        &meshes,
        &geometry,
        &rig_source,
        &mut redraw,
    );
    viewer.edit.model_has_rig =
        asset.is_some() && asset == selected.as_deref() && render.live.rig().is_some();
    let need_pixels = job.is_some() || preview.as_ref().is_some_and(|(_, pixels)| *pixels);
    let result = render.live.render(
        view,
        need_pixels,
        device.limits().max_texture_dimension_2d,
        &mut commands,
        &mut images,
        &mut redraw,
    );
    let image = preview
        .as_ref()
        .and_then(|(view, _)| render.live.presented(view));
    if let Some(preview) = viewer.layer_preview.as_mut() {
        preview.set_camera_image(image);
    }

    let Some(job) = job else { return };
    let tab = job.tab;
    redraw.write(RequestRedraw);
    let result = result.and_then(|()| {
        if let Some(frame) = render
            .live
            .raster()
            .filter(|frame| frame.view() == &job.view)
        {
            let view = &job.view;
            let name = Path::new(&view.asset)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            return viewer.edit.insert_reference(
                tab,
                view.document,
                view.size,
                format!("Референс · {name}"),
                &frame.surface,
                &ModelReference {
                    uid: job.uid.clone(),
                    asset: view.asset.clone(),
                    direction: view.direction,
                    center: view.center,
                    pose: view.pose.root,
                    joints: view.pose.joints.clone(),
                    pixels_per_unit: frame.shot.frame.pixels_per_unit,
                    pending_frame: None,
                },
                job.replace.as_ref(),
            );
        }
        if job
            .started
            .is_some_and(|started| started.elapsed() >= TIMEOUT)
        {
            return Err("Модель не успела отрисоваться".into());
        }
        Ok(false)
    });
    match result {
        Ok(false) => {}
        Ok(true) => {
            editor.status = if job.replace.is_some() {
                "Трансформация 3D-слоя применена"
            } else {
                "Слой-референс добавлен"
            }
            .into();
            if editor.viewing_image == Some(tab)
                && viewer
                    .edit
                    .error
                    .as_deref()
                    .is_some_and(|message| message.starts_with("Не удалось отрисовать 3D-слой"))
            {
                viewer.edit.error = None;
            }
            render.queue.pop_front();
        }
        Err(error) => {
            let message = format!("Не удалось отрисовать 3D-слой · {error}");
            if editor.viewing_image == Some(tab) {
                viewer.edit.error = Some(message.clone());
            }
            editor.status = message;
            let job = render.queue.pop_front().unwrap();
            if job.replace.is_some() {
                render.failed.push(job);
            }
        }
    }
    for image in &mut editor.image_tabs {
        if render.pending_for(Some(image.id)) {
            image.dirty = true;
        } else if image.id == tab {
            image.dirty = viewer
                .edit
                .document(Some(tab))
                .is_some_and(|doc| doc.dirty());
        }
    }
}

fn spawn_camera(
    commands: &mut Commands,
    root: Entity,
    view: Transform,
    projection: Projection,
    image: Handle<Image>,
    layer: usize,
) -> (Entity, Entity) {
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera {
                order: -4,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Image(image.into()),
            RenderLayers::layer(layer),
            view,
            projection,
            Hdr,
            Exposure::BLENDER,
            Tonemapping::AgX,
            AmbientLight {
                brightness: 350.0,
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    let light_direction = *view.back() * 0.55 - *view.right() * 0.55 + *view.up();
    let light = commands
        .spawn((
            DirectionalLight {
                illuminance: 3500.0,
                ..default()
            },
            Transform::from_translation(light_direction).looking_at(Vec3::ZERO, Vec3::Y),
            RenderLayers::layer(layer),
            ChildOf(root),
        ))
        .id();
    (camera, light)
}

fn prepare_surface(
    image: &image::RgbaImage,
    center: Vec2,
    span: f32,
    canvas: UVec2,
) -> Option<Surface> {
    let mut min = UVec2::new(image.width(), image.height());
    let mut max = UVec2::ZERO;
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] != 0 {
            min = min.min(UVec2::new(x, y));
            max = max.max(UVec2::new(x + 1, y + 1));
        }
    }
    if min.cmpge(max).any() {
        return None;
    }
    let frame = UVec2::new(image.width(), image.height()).as_vec2();
    let scale = span / frame.max_element();
    let origin = center - frame * 0.5 * scale;
    let start = (origin + min.as_vec2() * scale)
        .round()
        .clamp(Vec2::ZERO, canvas.as_vec2())
        .as_ivec2();
    let end = (origin + max.as_vec2() * scale)
        .round()
        .clamp(Vec2::ZERO, canvas.as_vec2())
        .as_ivec2();
    if start.cmpge(end).any() {
        return None;
    }
    let clipped = (end - start).as_uvec2();
    let from = ((start.as_vec2() - origin) / scale)
        .floor()
        .clamp(min.as_vec2(), (max - UVec2::ONE).as_vec2())
        .as_uvec2();
    let to = ((end.as_vec2() - origin) / scale)
        .ceil()
        .clamp((from + UVec2::ONE).as_vec2(), max.as_vec2())
        .as_uvec2();
    let crop = image::imageops::crop_imm(image, from.x, from.y, to.x - from.x, to.y - from.y);
    let pixels = image::imageops::resize(
        &crop.to_image(),
        clipped.x,
        clipped.y,
        image::imageops::FilterType::Triangle,
    );
    let bounds = PixelRect::new(start.x, start.y, end.x, end.y);
    let mut surface = Surface::new(PixelFormat::RGBA8);
    surface.write_interleaved(bounds, pixels.as_raw());
    Some(surface)
}

pub(super) fn track_frame(mut world: ResMut<MainWorld>, cache: Res<PipelineCache>) {
    let Some(mut render) = world.get_resource_mut::<ReferenceRender>() else {
        return;
    };
    let ready = cache.waiting_pipelines().next().is_none();
    render.live.track_camera_frame(ready);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_frame_preserves_move_scale_and_rotation_about_model_center() {
        let bounds = crate::BoxBounds {
            center: [5.0, 2.0, 0.0],
            half_size: [2.0, 1.0, 0.5],
        };
        let frame_for = |pose| {
            projected_frame(
                bounds,
                Vec3::Z,
                Vec2::new(320.0, 240.0),
                UVec2::new(640, 480),
                pose,
                Vec::new(),
            )
        };
        let (_, base) = frame_for(Transform::IDENTITY.into());
        let change = Transform::from_xyz(1.0, -2.0, 0.0)
            .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::splat(2.0));
        let (model, frame) = frame_for(change.into());
        let pivot = Vec3::from(bounds.center);
        assert!(
            model
                .transform_point(pivot)
                .abs_diff_eq(pivot + change.translation, 1e-5)
        );
        assert!((frame.pixel_span - 2.0 * base.pixel_span).abs() < 0.001);
        assert!(frame.center.abs_diff_eq(Vec2::new(392.0, 384.0), 0.001));
    }
}
