//! Keep one prepared mesh snapshot and one in-flight canvas preview.
use super::*;
use std::sync::Arc;
#[path = "image_reference_camera.rs"]
mod camera;
const LIVE_LAYER: usize = 23;

enum Stage {
    Loading(ModelSource),
    Framing(Entity),
    Ready(crate::BoxBounds, Arc<Vec<cpu::Part>>),
    Failed,
}

struct Mesh {
    asset: String,
    stage: Stage,
    started: Instant,
}

#[derive(Default)]
pub(in crate::editor) struct Preview {
    mesh: Option<Mesh>,
    world_root: Option<Entity>,
    camera: Option<camera::LiveCamera>,
    epoch: u64,
    task: Option<Task<Frame>>,
    raster: Option<(crate::Pose, Arc<image::RgbaImage>)>,
    wanted: Option<crate::Pose>,
    scheduled: Option<crate::Pose>,
}

struct Frame {
    epoch: u64,
    pose: crate::Pose,
    raster: Arc<image::RgbaImage>,
    image: photocraft_raster::Rgba8Image,
}

impl Preview {
    pub(in crate::editor) fn mesh_ready(&self, asset: &str) -> bool {
        self.mesh
            .as_ref()
            .is_some_and(|m| m.asset == asset && matches!(&m.stage, Stage::Ready(..)))
    }

    /// A saved reference raster is clipped at the document edges. Recover the
    /// off-canvas part once so moving it back onto the canvas stays a GPU translation.
    pub(in crate::editor) fn extend_layer(
        &self,
        reference: &ModelReference,
        size: UVec2,
        existing: PixelRect,
    ) -> Option<(
        PixelRect,
        crate::editor::ui::image_viewer::edit::selection::live::PrepareLayer,
    )> {
        let mesh = self.mesh.as_ref().filter(|m| m.asset == reference.asset)?;
        let Stage::Ready(bounds, parts) = &mesh.stage else {
            return None;
        };
        let (model, frame) = projected_frame(
            *bounds,
            reference.direction.into(),
            reference.center.into(),
            size,
            reference.pose,
        );
        if !frame.center.is_finite() || !frame.pixel_span.is_finite() || frame.pixel_span > 2048.0 {
            return None;
        }
        let lo = (frame.center - Vec2::splat(frame.pixel_span * 0.5))
            .floor()
            .as_ivec2();
        let hi = (frame.center + Vec2::splat(frame.pixel_span * 0.5))
            .ceil()
            .as_ivec2();
        let full = existing.union(&PixelRect::new(lo.x, lo.y, hi.x, hi.y));
        // Sparse documents may hold very large models. Bound the temporary sprite.
        if full.width() as i64 * full.height() as i64 > 4_194_304 {
            return None;
        }
        let parts = parts.clone();
        Some((
            full,
            Box::new(move |authored| {
                let image = cpu::render_transformed(
                    &parts,
                    model.compute_affine(),
                    frame.view,
                    frame.projection,
                );
                let Some(extra) = prepare_region(&image, frame.center, frame.pixel_span, full)
                else {
                    return;
                };
                let visible = PixelRect::new(0, 0, size.x as i32, size.y as i32);
                // Preserve the exact authored pixels on the canvas, including transparency.
                for outside in [
                    PixelRect::new(full.x0, full.y0, full.x1, 0),
                    PixelRect::new(full.x0, visible.y1, full.x1, full.y1),
                    PixelRect::new(full.x0, 0, 0, visible.y1),
                    PixelRect::new(visible.x1, 0, full.x1, visible.y1),
                ] {
                    let r = outside.intersect(&full);
                    if r.is_empty() {
                        continue;
                    }
                    let mut pixels = vec![[0; 4]; r.width() as usize * r.height() as usize];
                    extra.read_rgba8_into(r, &mut pixels);
                    authored.write_interleaved(r, pixels.as_flattened());
                }
            }),
        ))
    }

    pub(in crate::editor) fn calibrate(
        &self,
        mut model: ModelReference,
        size: UVec2,
    ) -> Option<ModelReference> {
        if model.pixels_per_unit == 0.0 {
            let mesh = self.mesh.as_ref()?;
            let Stage::Ready(bounds, _) = &mesh.stage else {
                return None;
            };
            if mesh.asset != model.asset {
                return None;
            }
            model.pixels_per_unit = pixel_density(bounds.half_size.into(), &model.view(), size);
        }
        Some(model)
    }

    pub(super) fn load(
        &mut self,
        asset: Option<String>,
        commands: &mut Commands,
        assets: &AssetServer,
        gltfs: &Assets<Gltf>,
        images: &Assets<Image>,
        prepared: &Query<(), With<scene::Prepared>>,
        subjects: &Query<(), With<scene::Subject>>,
        children: &Query<&Children>,
        meshes: &Query<(&Aabb, &GlobalTransform)>,
        geometry: &cpu::Geometry,
        redraw: &mut MessageWriter<RequestRedraw>,
    ) {
        if self.mesh.as_ref().map(|mesh| &mesh.asset) != asset.as_ref() {
            self.reset();
            self.clear_camera(commands);
            if let Some(root) = self.world_root.take() {
                commands.entity(root).try_despawn();
            }

            self.mesh = asset.map(|asset| Mesh {
                stage: Stage::Loading(ModelSource::load(assets, &asset, false)),
                started: Instant::now(),
                asset,
            });
        }
        let Some(mesh) = &mut self.mesh else { return };
        match &mesh.stage {
            Stage::Loading(source) => match source.world(assets, gltfs) {
                Ok(Some(world)) => {
                    let root = commands
                        .spawn((
                            WorldAssetRoot(world),
                            scene::CatalogModel,
                            Transform::IDENTITY,
                            Visibility::Inherited,
                            RenderLayers::layer(LIVE_LAYER),
                        ))
                        .observe(scene::prepare)
                        .id();
                    self.world_root = Some(root);
                    mesh.stage = Stage::Framing(root);
                    redraw.write(RequestRedraw);
                }
                Ok(None) if mesh.started.elapsed() < TIMEOUT => {
                    redraw.write(RequestRedraw);
                }
                _ => mesh.stage = Stage::Failed,
            },
            Stage::Framing(root) if prepared.contains(*root) => {
                let root = *root;
                let bounds = framing::bounds(root, subjects, children, meshes);
                let parts = geometry.snapshot(children.iter_descendants(root), images);
                mesh.stage = bounds
                    .filter(|_| !parts.is_empty())
                    .map_or(Stage::Failed, |bounds| {
                        Stage::Ready(bounds, Arc::new(parts))
                    });
                if matches!(mesh.stage, Stage::Failed) {
                    self.world_root = None;
                    commands.entity(root).try_despawn();
                }
            }
            Stage::Framing(root) if mesh.started.elapsed() >= TIMEOUT => {
                self.world_root = None;
                commands.entity(*root).try_despawn();
                mesh.stage = Stage::Failed;
            }
            Stage::Framing(_) => {
                redraw.write(RequestRedraw);
            }
            Stage::Ready(..) | Stage::Failed => {}
        }
        if self.wanted.is_none()
            && self
                .task
                .as_mut()
                .is_some_and(|task| block_on(poll_once(task)).is_some())
        {
            self.task = None;
        }
        if self.task.is_some() && self.wanted.is_some() {
            redraw.write(RequestRedraw);
        }
    }

    pub(in crate::editor) fn reset(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.raster = None;
        self.wanted = None;
        self.scheduled = None;
        // A running worker may still finish. Its epoch keeps it out of the next gesture.
    }

    pub(in crate::editor) fn poll(
        &mut self,
        photo: &super::super::photo_document::PhotoDocument,
        reference: &ModelReference,
        pose: crate::Pose,
        released: bool,
    ) -> Option<photocraft_raster::Rgba8Image> {
        self.wanted = Some(pose);
        let completed = self
            .task
            .as_mut()
            .and_then(|task| block_on(poll_once(task)));
        if let Some(frame) = &completed {
            self.task = None;
            if frame.epoch == self.epoch {
                self.raster = Some((frame.pose, frame.raster.clone()));
            }
        }
        if self.task.is_none()
            && self.scheduled != self.wanted
            && let Some(Mesh {
                asset,
                stage: Stage::Ready(bounds, parts),
                ..
            }) = &self.mesh
            && *asset == reference.asset
            && let Some(layer) = photo.active_id()
        {
            let doc = photo.document.clone();
            let canvas_size = UVec2::new(doc.size.width, doc.size.height);
            let (model, frame) = projected_frame(
                *bounds,
                reference.direction.into(),
                reference.center.into(),
                canvas_size,
                pose,
            );
            let parts = parts.clone();
            let epoch = self.epoch;
            // Orthographic translation changes the placement on the canvas, not the
            // camera-relative image of the mesh. Keep it for complex layer stacks too.
            let cached = self
                .raster
                .as_ref()
                .filter(|(source, _)| {
                    source.scale == pose.scale && source.rotation == pose.rotation
                })
                .map(|(_, image)| image.clone());
            self.scheduled = Some(pose);
            self.task = Some(AsyncComputeTaskPool::get().spawn(async move {
                let raster = cached.unwrap_or_else(|| {
                    Arc::new(cpu::render_transformed(
                        &parts,
                        model.compute_affine(),
                        frame.view,
                        frame.projection,
                    ))
                });
                let surface = prepare_surface(&raster, frame.center, frame.pixel_span, canvas_size)
                    .unwrap_or_else(|| Surface::new(PixelFormat::RGBA8));
                let mut doc = doc;
                doc.layer_mut(layer).unwrap().content =
                    photocraft_doc::LayerContent::Raster(surface);
                let image = crate::editor::photo_document::composite_region(
                    &doc,
                    PixelRect::from_size(doc.size),
                );
                Frame {
                    epoch,
                    pose,
                    raster,
                    image,
                }
            }));
        }
        completed
            .filter(|frame| frame.epoch == self.epoch && (!released || frame.pose == pose))
            .map(|frame| frame.image)
    }
}
