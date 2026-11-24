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
mod live;

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
    #[serde(default)]
    pub pixels_per_unit: f32,
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
            && reference.pixels_per_unit.is_finite()
            && reference.pixels_per_unit >= 0.0
            && Vec3::from(reference.direction).is_finite()
            && Vec2::from(reference.center).is_finite()
            && crate::data::ModelFormat::from_path(&reference.asset).is_some())
        .then_some(reference)
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

struct Request {
    uid: Option<String>,
    document: usize,
    photo: photocraft_doc::DocId,
    asset: String,
    direction: Vec3,
    center: Vec2,
    canvas_size: UVec2,
    pose: crate::Pose,
    replace: Option<ReferenceTarget>,
}

#[derive(Clone, Copy)]
pub(super) struct ReferenceTarget {
    pub layer: photocraft_doc::LayerId,
    pub revision: u64,
}

struct Job {
    request: Request,
    started: Instant,
    captures: u8,
    stage: Stage,
    frame: Option<ReferenceFrame>,
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

fn reference_frame(bounds: crate::BoxBounds, request: &Request) -> (Transform, ReferenceFrame) {
    projected_frame(
        bounds,
        request.direction,
        request.center,
        request.canvas_size,
        request.pose,
    )
}

fn projected_frame(
    bounds: crate::BoxBounds,
    direction: Vec3,
    origin: Vec2,
    size: UVec2,
    pose: crate::Pose,
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

enum Stage {
    Loading(ModelSource),
    Framing {
        root: Entity,
        model: Entity,
    },
    Rendering {
        root: Entity,
        image: Handle<Image>,
        frames: u8,
    },
    Capturing {
        root: Entity,
        image: Handle<Image>,
        readback: Entity,
    },
    Processing {
        root: Entity,
        image: Handle<Image>,
        task: Task<Result<Option<Surface>, String>>,
    },
    Software(Task<Result<Surface, String>>),
    Inserting(Surface),
}

impl Stage {
    fn cleanup(&self, commands: &mut Commands) {
        match self {
            Self::Framing { root, .. }
            | Self::Rendering { root, .. }
            | Self::Processing { root, .. } => commands.entity(*root).try_despawn(),
            Self::Capturing { root, readback, .. } => {
                commands.entity(*root).try_despawn();
                commands.entity(*readback).try_despawn();
            }
            _ => {}
        }
    }
}

#[derive(Resource, Default)]
pub(super) struct ReferenceRender {
    pub(super) live: live::Preview,
    queue: VecDeque<Request>,
    job: Option<Job>,
}

impl ReferenceRender {
    fn pending_requests(&self) -> impl DoubleEndedIterator<Item = &Request> {
        self.job
            .iter()
            .map(|job| &job.request)
            .chain(self.queue.iter())
    }

    pub(in crate::editor) fn pending_pose(
        &self,
        tab: usize,
        layer: photocraft_doc::LayerId,
    ) -> Option<crate::Pose> {
        self.pending_requests()
            .rev()
            .find(|r| r.document == tab && r.replace.is_some_and(|target| target.layer == layer))
            .map(|r| r.pose)
    }

    pub(super) fn transform_selected(
        &mut self,
        document: usize,
        photo: &super::photo_document::PhotoDocument,
        pose: crate::Pose,
    ) -> Result<(), String> {
        let layer = photo.active_layer().ok_or("Слой больше не существует")?;
        if self.pending_requests().any(|r| {
            r.document == document
                && (r.photo != photo.document.id || r.replace.is_none_or(|t| t.layer != layer.id))
        }) {
            return Err("Дождитесь обновления 3D-слоя".into());
        }
        let model = layer
            .xmp
            .as_deref()
            .and_then(ModelReference::from_xmp)
            .ok_or("Выберите слой с 3D-моделью")?;
        if !pose.valid() || layer.locks.position || layer.locks.all {
            return Err("Трансформация слоя недоступна".into());
        }
        if pose != self.pending_pose(document, layer.id).unwrap_or(model.pose) {
            let size = photo.size();
            self.queue.push_back(Request {
                document,
                photo: photo.document.id,
                uid: model.uid,
                asset: model.asset,
                direction: model.direction.into(),
                center: model.center.into(),
                canvas_size: UVec2::new(size.width, size.height),
                pose,
                replace: Some(ReferenceTarget {
                    layer: layer.id,
                    revision: photo.revision,
                }),
            });
        }
        Ok(())
    }

    pub(super) fn pending_for(&self, document: Option<usize>) -> bool {
        self.job
            .as_ref()
            .is_some_and(|job| Some(job.request.document) == document)
            || self
                .queue
                .iter()
                .any(|request| Some(request.document) == document)
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
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let live_asset = viewer
        .edit
        .model_tool
        .and_then(|_| {
            let id = editor.viewing_image;
            viewer
                .edit
                .editing(id)
                .then(|| viewer.edit.model_reference(id))
                .flatten()
        })
        .map(|model| model.resolved(&editor.root).asset);
    render.live.load(
        live_asset,
        &mut commands,
        &assets,
        &gltfs,
        &images,
        &prepared,
        &subjects,
        &children,
        &meshes,
        &geometry,
        &mut redraw,
    );
    let frame = render.live.update_camera(
        viewer
            .layer_preview
            .as_ref()
            .and_then(|p| p.camera_request()),
        &mut commands,
        &mut images,
        &mut redraw,
    );
    if let Some(preview) = viewer.layer_preview.as_mut() {
        preview.set_camera_image(frame);
    }
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
        render.queue.push_back(Request {
            document,
            photo: doc.document.id,
            uid: crate::asset_registry::index(&editor.root)
                .ok()
                .and_then(|i| i.uid(&asset).map(str::to_owned)),
            asset,
            direction,
            center: point,
            canvas_size: UVec2::new(size.width, size.height),
            pose: Transform::IDENTITY.into(),
            replace: None,
        });
        viewer.edit.error = None;
        editor.status = "Рендер слоя-референса…".into();
    }

    render
        .queue
        .retain(|request| editor.image_tab(request.document).is_some());
    if let Some(job) = &render.job
        && editor.image_tab(job.request.document).is_none()
    {
        job.stage.cleanup(&mut commands);
        render.job = None;
    }
    if render.job.is_none()
        && let Some(mut request) = render.queue.pop_front()
    {
        if let Ok(index) = crate::asset_registry::index(&editor.root) {
            index.read_ref(&mut request.asset, &mut request.uid);
        }
        render.job = Some(Job {
            stage: Stage::Loading(ModelSource::load(&assets, &request.asset, false)),
            request,
            started: Instant::now(),
            captures: 0,
            frame: None,
        });
    }
    let Some(job) = &mut render.job else { return };
    // Drive the offscreen render even when the pointer and window are idle.
    if !matches!(job.stage, Stage::Inserting(_)) {
        redraw.write(RequestRedraw);
    }
    let document = job.request.document;
    let result = if job.started.elapsed() >= TIMEOUT
        && !matches!(job.stage, Stage::Software(_) | Stage::Inserting(_))
    {
        Err("модель не успела отрисоваться".into())
    } else {
        match &mut job.stage {
            Stage::Loading(source) => source.world(&assets, &gltfs).map(|world| {
                if let Some(world) = world {
                    let root = commands
                        .spawn((Transform::IDENTITY, Visibility::Inherited))
                        .id();
                    let model = commands
                        .spawn((
                            WorldAssetRoot(world),
                            scene::CatalogModel,
                            Transform::IDENTITY,
                            Visibility::Hidden,
                            RenderLayers::layer(LAYER),
                            ChildOf(root),
                        ))
                        .observe(scene::prepare)
                        .id();
                    job.stage = Stage::Framing { root, model };
                }
                false
            }),
            Stage::Framing { root, model } if prepared.contains(*model) => {
                if let Some(bounds) = framing::bounds(*model, &subjects, &children, &meshes) {
                    let (pose, frame) = reference_frame(bounds, &job.request);
                    let mut target = Image::new_target_texture(
                        RESOLUTION,
                        RESOLUTION,
                        TextureFormat::Rgba8UnormSrgb,
                        None,
                    );
                    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
                    let image = images.add(target);
                    commands
                        .entity(*model)
                        .insert((Visibility::Inherited, pose));
                    let (camera, _) = spawn_camera(
                        &mut commands,
                        *root,
                        frame.view,
                        frame.projection.clone(),
                        image.clone(),
                        LAYER,
                    );
                    commands.entity(camera).insert(ReferenceCamera);
                    job.frame = Some(frame);
                    job.stage = Stage::Rendering {
                        root: *root,
                        image,
                        frames: 0,
                    };
                    Ok(false)
                } else {
                    Err("в модели нет геометрии для референса".into())
                }
            }
            Stage::Rendering {
                root,
                image,
                frames,
            } if *frames >= 3 => {
                let readback = commands
                    .spawn(ReadbackOnce::texture(image.clone()))
                    .observe(captured)
                    .id();
                job.captures += 1;
                job.stage = Stage::Capturing {
                    root: *root,
                    image: image.clone(),
                    readback,
                };
                Ok(false)
            }
            Stage::Processing { root, image, task } => match block_on(poll_once(task)) {
                Some(Ok(Some(surface))) => {
                    commands.entity(*root).try_despawn();
                    job.stage = Stage::Inserting(surface);
                    Ok(false)
                }
                // A transparent frame can arrive while the model's render pipelines warm up.
                Some(Ok(None)) if job.captures < MAX_CAPTURES => {
                    job.stage = Stage::Rendering {
                        root: *root,
                        image: image.clone(),
                        frames: 0,
                    };
                    Ok(false)
                }
                Some(Ok(None)) => {
                    let root = *root;
                    job.frame
                        .clone()
                        .ok_or_else(|| "камера модели недоступна".to_string())
                        .map(|frame| {
                            let parts = geometry.snapshot(children.iter_descendants(root), &images);
                            let canvas = job.request.canvas_size;
                            let replacing = job.request.replace.is_some();
                            commands.entity(root).try_despawn();
                            job.stage =
                                Stage::Software(AsyncComputeTaskPool::get().spawn(async move {
                                    prepare_surface(
                                        &cpu::render(parts, frame.view, frame.projection),
                                        frame.center,
                                        frame.pixel_span,
                                        canvas,
                                    )
                                    .or_else(|| replacing.then(|| Surface::new(PixelFormat::RGBA8)))
                                    .ok_or_else(|| "не удалось отрисовать геометрию модели".into())
                                }));
                            false
                        })
                }
                Some(Err(error)) => Err(error),
                None => Ok(false),
            },
            Stage::Software(task) => match block_on(poll_once(task)) {
                Some(Ok(surface)) => {
                    job.stage = Stage::Inserting(surface);
                    Ok(false)
                }
                Some(Err(error)) => Err(error),
                None => Ok(false),
            },
            Stage::Inserting(surface) if editor.pending.is_none() => {
                let name = Path::new(&job.request.asset)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                viewer.edit.insert_reference(
                    document,
                    job.request.photo,
                    job.request.canvas_size,
                    format!("Референс · {name}"),
                    surface,
                    &ModelReference {
                        uid: job.request.uid.clone(),
                        asset: job.request.asset.clone(),
                        direction: job.request.direction.to_array(),
                        center: job.request.center.to_array(),
                        pose: job.request.pose,
                        pixels_per_unit: job
                            .frame
                            .as_ref()
                            .map_or(0.0, |frame| frame.pixels_per_unit),
                    },
                    job.request.replace,
                )
            }
            _ => Ok(false),
        }
    };
    match result {
        Ok(false) => {}
        Ok(true) => {
            if let Some(tab) = editor.image_tabs.iter_mut().find(|tab| tab.id == document) {
                tab.dirty = true;
            }
            editor.status = if job.request.replace.is_some() {
                "Трансформация 3D-слоя применена"
            } else {
                "Слой-референс добавлен"
            }
            .into();
            if editor.viewing_image == Some(document)
                && viewer
                    .edit
                    .error
                    .as_deref()
                    .is_some_and(|error| error.starts_with("Не удалось добавить слой-референс"))
            {
                viewer.edit.error = None;
            }
            // Rebase consecutive field edits only on successful replacement of this layer.
            if let Some(completed) = job.request.replace {
                let photo = job.request.photo;
                let revision = viewer.edit.document(Some(document)).unwrap().revision;
                for pending in &mut render.queue {
                    if pending.document == document
                        && pending.photo == photo
                        && let Some(target) = &mut pending.replace
                        && target.layer == completed.layer
                        && target.revision == completed.revision
                    {
                        target.revision = revision;
                    }
                }
            }
            render.job = None;
            redraw.write(RequestRedraw);
        }
        Err(error) => {
            job.stage.cleanup(&mut commands);
            let message = format!("Не удалось добавить слой-референс · {error}");
            if editor.viewing_image == Some(document) {
                viewer.edit.error = Some(message.clone());
            }
            editor.status = message;
            if let Some(failed) = job.request.replace {
                render.queue.retain(|request| {
                    !(request.document == document
                        && request
                            .replace
                            .is_some_and(|target| target.layer == failed.layer))
                });
            }
            render.job = None;
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

fn captured(
    mut capture: On<ReadbackComplete>,
    mut render: ResMut<ReferenceRender>,
    mut commands: Commands,
) {
    let Some(job) = &mut render.job else { return };
    let Stage::Capturing {
        root,
        image,
        readback,
    } = &job.stage
    else {
        return;
    };
    if capture.entity != *readback {
        return;
    }
    commands.entity(*readback).try_despawn();
    let data = std::mem::take(&mut capture.event_mut().data);
    let Some(frame) = job.frame.as_ref() else {
        return;
    };
    let center = frame.center;
    let span = frame.pixel_span;
    let canvas = job.request.canvas_size;
    job.stage = Stage::Processing {
        root: *root,
        image: image.clone(),
        task: AsyncComputeTaskPool::get().spawn(async move {
            // The 1024-pixel RGBA rows are already aligned for GPU readback.
            let image = image::RgbaImage::from_raw(RESOLUTION, RESOLUTION, data)
                .ok_or("не удалось прочитать пиксели модели")?;
            Ok(prepare_surface(&image, center, span, canvas))
        }),
    };
}

fn prepare_surface(
    image: &image::RgbaImage,
    center: Vec2,
    span: f32,
    canvas: UVec2,
) -> Option<Surface> {
    prepare_region(
        image,
        center,
        span,
        PixelRect::new(0, 0, canvas.x as i32, canvas.y as i32),
    )
}

fn prepare_region(
    image: &image::RgbaImage,
    center: Vec2,
    span: f32,
    clip: PixelRect,
) -> Option<Surface> {
    let low = Vec2::new(clip.x0 as f32, clip.y0 as f32);
    let high = Vec2::new(clip.x1 as f32, clip.y1 as f32);
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
        .clamp(low, high)
        .as_ivec2();
    let end = (origin + max.as_vec2() * scale)
        .round()
        .clamp(low, high)
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
    if let Some(Job {
        stage: Stage::Rendering { frames, .. },
        ..
    }) = &mut render.job
    {
        *frames = if ready { frames.saturating_add(1) } else { 0 };
    }
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
        let mut request = Request {
            uid: None,
            document: 0,
            photo: photocraft_doc::DocId::fresh(),
            asset: "models/block.glb".into(),
            direction: Vec3::Z,
            center: Vec2::new(320.0, 240.0),
            canvas_size: UVec2::new(640, 480),
            pose: Transform::IDENTITY.into(),
            replace: None,
        };
        let (_, base) = reference_frame(bounds, &request);
        let change = Transform::from_xyz(1.0, -2.0, 0.0)
            .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::splat(2.0));
        request.pose = change.into();
        let (model, frame) = reference_frame(bounds, &request);
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
