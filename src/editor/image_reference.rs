//! Render a model into a transparent PhotoCraft reference layer.
use std::{
    collections::VecDeque,
    path::Path,
    time::{Duration, Instant},
};

use bevy::{
    camera::{
        ClearColorConfig, Exposure, Hdr, RenderTarget, primitives::Aabb, visibility::RenderLayers,
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

const LAYER: usize = 22;
const RESOLUTION: u32 = 1024;
const MAX_CAPTURES: u8 = 3;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Component)]
pub(super) struct ReferenceCamera;

struct Request {
    document: usize,
    photo: photocraft_doc::DocId,
    asset: String,
    direction: Vec3,
    center: Vec2,
    canvas_size: UVec2,
}

struct Job {
    request: Request,
    started: Instant,
    captures: u8,
    stage: Stage,
}

enum Stage {
    Loading(ModelSource),
    Framing(Entity),
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
            Self::Framing(root) | Self::Rendering { root, .. } | Self::Processing { root, .. } => {
                commands.entity(*root).try_despawn()
            }
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
    queue: VecDeque<Request>,
    job: Option<Job>,
}

impl ReferenceRender {
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
            asset,
            direction,
            center: point,
            canvas_size: UVec2::new(size.width, size.height),
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
        && let Some(request) = render.queue.pop_front()
    {
        render.job = Some(Job {
            stage: Stage::Loading(ModelSource::load(&assets, &request.asset, false)),
            request,
            started: Instant::now(),
            captures: 0,
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
                        .spawn((
                            WorldAssetRoot(world),
                            scene::CatalogModel,
                            Transform::IDENTITY,
                            Visibility::Hidden,
                            RenderLayers::layer(LAYER),
                        ))
                        .observe(scene::prepare)
                        .id();
                    job.stage = Stage::Framing(root);
                }
                false
            }),
            Stage::Framing(root) if prepared.contains(*root) => {
                if let Some(bounds) = framing::bounds(*root, &subjects, &children, &meshes) {
                    let (view, projection) = framing::fit(bounds, job.request.direction, 1.0);
                    let mut target = Image::new_target_texture(
                        RESOLUTION,
                        RESOLUTION,
                        TextureFormat::Rgba8UnormSrgb,
                        None,
                    );
                    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
                    let image = images.add(target);
                    commands.entity(*root).insert(Visibility::Inherited);
                    spawn_camera(&mut commands, *root, view, projection, image.clone());
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
                    framing::bounds(root, &subjects, &children, &meshes)
                        .ok_or_else(|| "геометрия модели недоступна".to_string())
                        .map(|bounds| {
                            let (view, projection) =
                                framing::fit(bounds, job.request.direction, 1.0);
                            let parts = geometry.snapshot(children.iter_descendants(root), &images);
                            let center = job.request.center;
                            let canvas = job.request.canvas_size;
                            commands.entity(root).try_despawn();
                            job.stage =
                                Stage::Software(AsyncComputeTaskPool::get().spawn(async move {
                                    prepare_surface(
                                        cpu::render(parts, view, projection),
                                        center,
                                        canvas,
                                    )
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
            editor.status = "Слой-референс добавлен".into();
            if editor.viewing_image == Some(document)
                && viewer
                    .edit
                    .error
                    .as_deref()
                    .is_some_and(|error| error.starts_with("Не удалось добавить слой-референс"))
            {
                viewer.edit.error = None;
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
) {
    commands.spawn((
        ReferenceCamera,
        Camera3d::default(),
        Camera {
            order: -4,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(image.into()),
        RenderLayers::layer(LAYER),
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
    ));
    let light_direction = *view.back() * 0.55 - *view.right() * 0.55 + *view.up();
    commands.spawn((
        DirectionalLight {
            illuminance: 3500.0,
            ..default()
        },
        Transform::from_translation(light_direction).looking_at(Vec3::ZERO, Vec3::Y),
        RenderLayers::layer(LAYER),
        ChildOf(root),
    ));
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
    let center = job.request.center;
    let canvas = job.request.canvas_size;
    job.stage = Stage::Processing {
        root: *root,
        image: image.clone(),
        task: AsyncComputeTaskPool::get().spawn(async move {
            // The 1024-pixel RGBA rows are already aligned for GPU readback.
            let image = image::RgbaImage::from_raw(RESOLUTION, RESOLUTION, data)
                .ok_or("не удалось прочитать пиксели модели")?;
            Ok(prepare_surface(image, center, canvas))
        }),
    };
}

fn prepare_surface(image: image::RgbaImage, center: Vec2, canvas: UVec2) -> Option<Surface> {
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
    let original = max - min;
    let cropped =
        image::imageops::crop_imm(&image, min.x, min.y, original.x, original.y).to_image();
    let scale = ((canvas.as_vec2() * 0.6) / original.as_vec2())
        .min_element()
        .min(1.0);
    let size = (original.as_vec2() * scale)
        .round()
        .max(Vec2::ONE)
        .as_uvec2();
    let pixels = if size == original {
        cropped
    } else {
        image::imageops::resize(
            &cropped,
            size.x,
            size.y,
            image::imageops::FilterType::Triangle,
        )
    };
    let offset = (center - size.as_vec2() * 0.5)
        .round()
        .clamp(Vec2::ZERO, (canvas - size).as_vec2())
        .as_ivec2();
    let bounds = PixelRect::new(
        offset.x,
        offset.y,
        offset.x + size.x as i32,
        offset.y + size.y as i32,
    );
    let mut surface = Surface::new(PixelFormat::RGBA8);
    surface.write_interleaved(bounds, pixels.as_raw());
    Some(surface)
}

pub(super) fn track_frame(mut world: ResMut<MainWorld>, cache: Res<PipelineCache>) {
    if let Some(mut render) = world.get_resource_mut::<ReferenceRender>()
        && let Some(Job {
            stage: Stage::Rendering { frames, .. },
            ..
        }) = &mut render.job
    {
        *frames = if cache.waiting_pipelines().next().is_none() {
            frames.saturating_add(1)
        } else {
            0
        };
    }
}
