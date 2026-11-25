//! A request camera renders the live GPU material into its own image.
use super::*;
use crate::editor::ui::model_viewer::{
    LAYER, REQUEST_MODEL_LAYER, ViewCamera, model_pose, model_projection,
};
use crate::hdri::{CameraEnvironment, EnvironmentBackdrop, OrthographicBackdrop};
use bevy::{
    asset::RenderAssetUsages,
    camera::{ClearColorConfig, Exposure, Hdr, RenderTarget, visibility::RenderLayers},
    light::EnvironmentMapLight,
    render::view::Tonemapping,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    window::RequestRedraw,
};
use std::sync::Arc;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct RenderArgs {
    model_id: usize,
    /// Output width in pixels (16–4096, default 1024).
    width: Option<u32>,
    /// Output height in pixels (16–4096, default 768).
    height: Option<u32>,
    /// Optional camera changes applied only to the requested image.
    camera: Option<CameraPatch>,
    /// Override the model's current raster preview only for this image.
    preview: Option<MapPreview>,
    #[serde(default)]
    transparent: bool,
}

type Lighting<'a> = (
    &'a Camera,
    &'a Exposure,
    &'a AmbientLight,
    Option<&'a DistanceFog>,
    Option<&'a EnvironmentMapLight>,
    Option<&'a CameraEnvironment>,
    Option<&'a EnvironmentBackdrop>,
);

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct RenderRequest {
    args: RenderArgs,
    reply: Option<CaptureReply>,
    deadline: Instant,
    bake: Option<BakeImage>,
}

struct BakeImage {
    maps: Arc<bake::Maps>,
    pixels: Task<Vec<u8>>,
}

impl RenderRequest {
    pub(in crate::editor::ui::model_viewer::paint) fn needs_channels(&self, model: usize) -> bool {
        model == self.args.model_id && matches!(self.args.preview, Some(MapPreview::Channel { .. }))
    }
}

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct RenderImage {
    model: usize,
    camera: Entity,
    image: Handle<Image>,
    size: UVec2,
    deadline: Instant,
    frames: u8,
    reply: Option<CaptureReply>,
}

pub(in crate::editor) fn render(
    commands: &mut Commands,
    editor: &Editor,
    args: RenderArgs,
    reply: CaptureReply,
) {
    match active_model(editor, args.model_id) {
        Ok(_) => {
            commands.spawn(RenderRequest {
                args,
                reply: Some(reply),
                deadline: Instant::now() + Duration::from_secs(25),
                bake: None,
            });
        }
        Err(error) => reply(Err(error)),
    }
}

type Surface<'a> = (
    &'a sets::SetPart,
    &'a Mesh3d,
    &'a GlobalTransform,
    &'a InheritedVisibility,
    &'a editing::xray::SourceMaterial,
);

fn baked_image(
    pending: &mut Option<BakeImage>,
    model: usize,
    doc: &crate::editor::model_document::Document,
    session: &Session,
    baked: &bake::State,
    map: bake::BakeMap,
    images: &mut Assets<Image>,
) -> Result<Option<Handle<Image>>, String> {
    let key = session
        .binding
        .ok_or("Select a texture set to render a baked map")?;
    if !session.set_appearance.visibility.shown(key.0, key.1) {
        return Err("Show the selected texture set to render its baked map".into());
    }
    if let Some(image) = baked.preview_for(model, doc, session, map) {
        return Ok(Some(image.clone()));
    }
    let maps = baked
        .maps(model, doc, key, session.texture_size)
        .filter(|maps| maps.has_map(map))
        .ok_or("The requested map is not baked for the active texture set")?;
    if let Some(job) = pending
        && Arc::ptr_eq(&job.maps, maps)
    {
        return Ok(block_on(poll_once(&mut job.pixels)).map(|pixels| {
            images.add(Image::new(
                bevy::render::render_resource::Extent3d {
                    width: maps.size,
                    height: maps.size,
                    depth_or_array_layers: 1,
                },
                bevy::render::render_resource::TextureDimension::D2,
                pixels,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            ))
        }));
    }
    let source = maps.clone();
    *pending = Some(BakeImage {
        maps: maps.clone(),
        pixels: AsyncComputeTaskPool::get().spawn(async move { source.rgba(map) }),
    });
    Ok(None)
}

pub(in crate::editor::ui::model_viewer) fn render_requests(
    mut commands: Commands,
    editor: Res<Editor>,
    viewer: Res<Viewer>,
    editing: Res<editing::State>,
    paint: Res<PaintState>,
    settings: Res<environment::Settings>,
    device: Option<Res<RenderDevice>>,
    lighting: Query<Lighting, With<ViewCamera>>,
    mut scene: (
        Res<engine::Runtime>,
        Res<bake::State>,
        ResMut<Assets<StandardMaterial>>,
        Query<Surface>,
    ),
    mut images: ResMut<Assets<Image>>,
    mut requests: Query<(Entity, &mut RenderRequest)>,
    mut renders: Query<(Entity, &mut RenderImage)>,
    cameras: Query<&Camera>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let mut rendering = !renders.is_empty();
    for (entity, mut request) in &mut requests {
        let Some(reply) = request.reply.take() else {
            continue;
        };
        let args = request.args.clone();
        let start = (|| {
            let model = active_model(&editor, args.model_id)?;
            if rendering {
                return Err("A paint render is already in progress".into());
            }
            rendering = true;
            if Instant::now() >= request.deadline {
                return Err("Timed out preparing the paint render".into());
            }
            let doc = editing
                .documents
                .get(&model)
                .filter(|doc| doc.mode == Mode::Paint)
                .ok_or("Switch the model to Painting mode first")?;
            let session = paint
                .sessions
                .get(&model)
                .ok_or("Wait for Painting mode to load")?;
            if viewer.model != Some(model)
                || !viewer.fitted
                || viewer.pending_document_camera
                || device.is_none()
            {
                return Err("Wait for the model renderer to load".into());
            }
            let preview = args.preview.as_ref().map(preview_mode).transpose()?;
            if doc.paint_busy && !matches!(preview, Some(Preview3d::Channel(_))) {
                return Err("Finish the current paint operation first".into());
            }
            let size = UVec2::new(args.width.unwrap_or(1024), args.height.unwrap_or(768));
            if size.min_element() < 16 || size.max_element() > 4096 {
                return Err("Render width and height must be between 16 and 4096 pixels".into());
            }
            let current = viewer.camera();
            let pose = args
                .camera
                .as_ref()
                .map_or(Ok(current), |patch| patch.apply(current))?;
            let (camera, exposure, ambient, fog, light, environment, backdrop) =
                lighting.single().map_err(|_| "No model lighting camera")?;
            if let Some(preview) = preview {
                let bake_texture = if let Preview3d::Baked(map) = preview {
                    let Some(image) = baked_image(
                        &mut request.bake,
                        model,
                        doc,
                        session,
                        &scene.1,
                        map,
                        &mut images,
                    )?
                    else {
                        return Ok(None);
                    };
                    Some(image)
                } else {
                    None
                };
                if let Preview3d::Channel(channel) = preview {
                    let mut available = false;
                    for object in doc.objects.iter().filter(|object| !object.hidden) {
                        for slot in 0..object.materials.len() {
                            if session.set_appearance.visibility.shown(object.id, slot)
                                && engine::projection::has_uv(object, slot)
                            {
                                available = true;
                                if scene.0.preview(model, object.id, slot, channel).is_none()
                                    || doc.paint_busy
                                {
                                    return Ok(None);
                                }
                            }
                        }
                    }
                    if !available {
                        return Err(
                            "No visible texture set has UVs for the requested channel".into()
                        );
                    }
                }
                for (part, mesh, pose, visibility, source) in &scene.3 {
                    if !visibility.get() {
                        continue;
                    }
                    let key = (part.0, part.1);
                    let diagnostic = match preview {
                        Preview3d::Channel(channel) => {
                            scene.0.preview(model, part.0, part.1, channel)
                        }
                        Preview3d::Baked(_) if session.binding == Some(key) => bake_texture.clone(),
                        _ => None,
                    };
                    let material = if let Some(image) = diagnostic {
                        let bake = bake_texture.is_some();
                        let source = scene.2.get(&source.0);
                        let material = StandardMaterial {
                            unlit: true,
                            base_color_texture: Some(image),
                            double_sided: bake || source.is_some_and(|source| source.double_sided),
                            cull_mode: if bake {
                                None
                            } else {
                                source.and_then(|source| source.cull_mode)
                            },
                            ..default()
                        };
                        scene.2.add(material)
                    } else {
                        scene
                            .0
                            .working_material(model, part.0, part.1)
                            .unwrap_or(&source.0)
                            .clone()
                    };
                    commands.spawn((
                        mesh.clone(),
                        MeshMaterial3d(material),
                        pose.compute_transform(),
                        RenderLayers::layer(REQUEST_MODEL_LAYER),
                        ChildOf(entity),
                    ));
                }
            }
            let clear_color = if args.transparent {
                ClearColorConfig::Custom(Color::NONE)
            } else {
                camera.clear_color.clone()
            };
            let mut target =
                Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
            target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            let image = images.add(target);
            let mut camera = commands.spawn((
                Camera3d::default(),
                Hdr,
                Msaa::Off,
                Camera {
                    order: -1,
                    clear_color,
                    ..default()
                },
                RenderTarget::Image(image.clone().into()),
                RenderLayers::layer(if preview.is_some() {
                    REQUEST_MODEL_LAYER
                } else {
                    LAYER
                }),
                model_pose(pose),
                model_projection(pose, viewer.radius),
                OrthographicBackdrop(true),
                *exposure,
                ambient.clone(),
                bevy::anti_alias::fxaa::Fxaa::default(),
                ChildOf(entity),
            ));
            if let Some(fog) = fog {
                camera.insert(fog.clone());
            }
            if let Some(light) = light {
                camera.insert(light.clone());
            }
            if let Some(backdrop) = backdrop {
                camera.insert(*backdrop);
            }
            if let Some(environment) = environment {
                camera.insert(if args.transparent {
                    environment.without_background()
                } else {
                    environment.clone()
                });
            }
            let post = settings.paint_post(Some(doc));
            let (_, baseline, curve) = settings.trace_light(None);
            let diagnostic = preview.map_or_else(
                || session.diagnostic_3d(),
                |preview| preview != Preview3d::Material,
            );
            if !diagnostic && (post.effects.enabled() || post.exposure != baseline) {
                camera.insert((
                    Tonemapping::None,
                    post::RasterGrade(
                        post::Grade {
                            exposure: post.exposure - baseline,
                            curve,
                            effects: post.effects,
                        },
                        None,
                    ),
                ));
            } else {
                camera.insert(settings.tone_mapper());
            }
            Ok(Some((model, camera.id(), image, size)))
        })();
        match start {
            Ok(Some((model, camera, image, size))) => {
                commands.entity(entity).remove::<RenderRequest>().insert((
                    Transform::IDENTITY,
                    Visibility::Inherited,
                    RenderImage {
                        model,
                        camera,
                        image,
                        size,
                        frames: 0,
                        deadline: request.deadline,
                        reply: Some(reply),
                    },
                ));
                redraw.write(RequestRedraw);
            }
            Ok(None) => {
                request.reply = Some(reply);
                redraw.write(RequestRedraw);
            }
            Err(error) => {
                commands.entity(entity).try_despawn();
                reply(Err(error));
            }
        }
    }
    for (entity, mut render) in &mut renders {
        if Instant::now() >= render.deadline || active_model(&editor, render.model).is_err() {
            if let Some(reply) = render.reply.take() {
                reply(Err(
                    "The paint render timed out or its model was closed".into()
                ));
            }
            commands.entity(entity).try_despawn();
            continue;
        }
        redraw.write(RequestRedraw);
        if render.reply.is_none()
            || cameras
                .get(render.camera)
                .ok()
                .and_then(Camera::physical_target_size)
                != Some(render.size)
        {
            continue;
        }
        // Leave time for camera extraction, environment preparation and new view pipelines.
        render.frames += 1;
        if render.frames < 8 {
            continue;
        }
        let reply = std::sync::Mutex::new(render.reply.take());
        let size = render.size;
        commands
            .entity(entity)
            .insert(ReadbackOnce::texture(render.image.clone()))
            .observe(move |event: On<ReadbackComplete>, mut commands: Commands| {
                if let Some(reply) = reply.lock().ok().and_then(|mut reply| reply.take()) {
                    reply(decode_pixels(size.x, size.y, &event.data));
                }
                commands.entity(event.entity).try_despawn();
            });
    }
}
