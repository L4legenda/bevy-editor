//! A single prepared model serves the interactive canvas and the layer's saved pixels.
use super::*;
use std::sync::Arc;

#[path = "image_reference_camera.rs"]
mod camera;

enum Stage {
    Loading(ModelSource),
    Framing(Entity),
    Ready(crate::BoxBounds, Arc<Vec<cpu::Part>>),
    Failed(String),
}

struct Mesh {
    asset: String,
    stage: Stage,
    started: Instant,
}

pub(in crate::editor) struct Shot {
    pub view: ModelView,
    pub(super) frame: ReferenceFrame,
    pub id: u64,
    rig: Arc<rig::Rig>,
    model: Transform,
}

pub(in crate::editor) struct RasterFrame {
    pub(in crate::editor) shot: Arc<Shot>,
    pub(in crate::editor) surface: Surface,
}

impl RasterFrame {
    pub(in crate::editor) fn view(&self) -> &ModelView {
        &self.shot.view
    }
    pub(in crate::editor) fn id(&self) -> u64 {
        self.shot.id
    }
}

#[derive(Clone)]
pub(in crate::editor) struct CameraImage {
    pub image: Handle<Image>,
    pub rect: [f32; 4],
    pub shot: Arc<Shot>,
}

#[derive(Default)]
pub(in crate::editor) struct Preview {
    mesh: Option<Mesh>,
    rig: Arc<rig::Rig>,
    world_root: Option<Entity>,
    camera: Option<camera::LiveCamera>,
    serial: u64,
    pub(in crate::editor) generation: u64,
    raster: Option<RasterFrame>,
    processing: Option<Task<Result<RasterFrame, String>>>,
}

impl Preview {
    #[cfg(test)]
    pub(in crate::editor) fn model_root(&self) -> Entity {
        self.world_root.unwrap()
    }
    pub(in crate::editor) fn rig(&self) -> Option<&rig::Rig> {
        (!self.rig.is_empty()).then_some(&self.rig)
    }

    fn bounds(&self, asset: &str) -> Option<crate::BoxBounds> {
        let mesh = self.mesh.as_ref().filter(|mesh| mesh.asset == asset)?;
        match &mesh.stage {
            Stage::Ready(bounds, _) => Some(*bounds),
            _ => None,
        }
    }

    pub(in crate::editor) fn joint_view(
        &self,
        reference: &ModelReference,
        size: UVec2,
    ) -> Option<(Transform, Vec2, Transform, f32)> {
        let (model, frame) = projected_frame(
            self.bounds(&reference.asset)?,
            reference.direction.into(),
            reference.center.into(),
            size,
            reference.pose,
            self.rig.positions(&reference.joints),
        );
        Some((model, frame.center, frame.view, frame.pixels_per_unit))
    }

    pub(in crate::editor) fn calibrate(
        &self,
        mut model: ModelReference,
        size: UVec2,
    ) -> Option<ModelReference> {
        if model.pixels_per_unit == 0.0 {
            let bounds = self.bounds(&model.asset)?;
            model.pixels_per_unit = pixel_density(bounds.half_size.into(), &model.view(), size);
        }
        Some(model)
    }

    pub(super) fn load(
        &mut self,
        asset: Option<&str>,
        commands: &mut Commands,
        assets: &AssetServer,
        gltfs: &Assets<Gltf>,
        images: &Assets<Image>,
        prepared: &Query<(), With<scene::Prepared>>,
        subjects: &Query<(), With<scene::Subject>>,
        children: &Query<&Children>,
        meshes: &Query<(&Aabb, &GlobalTransform)>,
        geometry: &cpu::Geometry,
        rig_source: &rig::Source,
        redraw: &mut MessageWriter<RequestRedraw>,
    ) {
        if self.mesh.as_ref().map(|mesh| mesh.asset.as_str()) != asset {
            self.invalidate(commands);
            self.rig = Arc::default();
            if let Some(root) = self.world_root.take() {
                commands.entity(root).try_despawn();
            }
            self.mesh = asset.map(|path| Mesh {
                stage: Stage::Loading(ModelSource::load(assets, path, false)),
                started: Instant::now(),
                asset: path.into(),
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
                            RenderLayers::layer(LAYER),
                        ))
                        .observe(scene::prepare)
                        .observe(instance_ready)
                        .id();
                    self.world_root = Some(root);
                    mesh.stage = Stage::Framing(root);
                    redraw.write(RequestRedraw);
                }
                Ok(None) if mesh.started.elapsed() < TIMEOUT => {
                    redraw.write(RequestRedraw);
                }
                Ok(None) => mesh.stage = Stage::Failed("Модель не успела загрузиться".into()),
                Err(error) => mesh.stage = Stage::Failed(error),
            },
            Stage::Framing(root) if prepared.contains(*root) => {
                let Some(bounds) = framing::bounds(*root, subjects, children, meshes) else {
                    mesh.stage = Stage::Failed("В модели нет геометрии для референса".into());
                    return;
                };
                // Retain bind-space geometry only once for adapters that cannot produce a frame.
                let parts = geometry.snapshot(children.iter_descendants(*root), images);
                self.rig = Arc::new(rig_source.read(*root, children));
                mesh.stage = Stage::Ready(bounds, Arc::new(parts));
            }
            Stage::Framing(_) if mesh.started.elapsed() >= TIMEOUT => {
                mesh.stage = Stage::Failed("Модель не успела подготовиться".into());
            }
            Stage::Framing(_) => {
                redraw.write(RequestRedraw);
            }
            _ => {}
        }
    }

    pub(super) fn retry(&mut self) {
        if self
            .mesh
            .as_ref()
            .is_some_and(|m| matches!(m.stage, Stage::Failed(_)))
        {
            self.mesh = None;
        }
    }

    pub(super) fn invalidate(&mut self, commands: &mut Commands) {
        self.generation += 1;
        self.clear_camera(commands);
        self.raster = None;
        self.processing = None;
    }

    pub(in crate::editor) fn raster(&self) -> Option<&RasterFrame> {
        self.raster.as_ref()
    }

    fn software(&mut self, shot: Arc<Shot>) {
        let Some(Stage::Ready(_, parts)) = self.mesh.as_ref().map(|m| &m.stage) else {
            return;
        };
        let parts = parts.clone();
        let joints = shot.rig.matrices(&shot.view.pose.joints);
        self.processing = Some(AsyncComputeTaskPool::get().spawn(async move {
            let raster = cpu::render_transformed(
                &parts,
                shot.model.compute_affine(),
                shot.frame.view,
                shot.frame.projection.clone(),
                &joints,
            );
            if !raster.pixels().any(|p| p[3] != 0) {
                return Err("Не удалось отрисовать геометрию модели".into());
            }
            Ok(shot.surface(&raster))
        }));
    }
}

impl Shot {
    pub(in crate::editor) fn rig(&self) -> &rig::Rig {
        &self.rig
    }

    pub(in crate::editor) fn joint_view(&self) -> (Transform, Vec2, Transform, f32) {
        (
            self.model,
            self.frame.center,
            self.frame.view,
            self.frame.pixels_per_unit,
        )
    }

    fn surface(self: Arc<Self>, pixels: &image::RgbaImage) -> RasterFrame {
        // A nonempty camera image can legitimately fall entirely outside the canvas.
        let surface = prepare_surface(
            pixels,
            self.frame.center,
            self.frame.pixel_span,
            self.view.size,
        )
        .unwrap_or_else(|| Surface::new(PixelFormat::RGBA8));
        RasterFrame {
            shot: self,
            surface,
        }
    }
}

fn instance_ready(
    ready: On<bevy::world_serialization::WorldInstanceReady>,
    mut render: ResMut<ReferenceRender>,
    mut commands: Commands,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let preview = &mut render.live;
    if preview.world_root != Some(ready.entity) {
        return;
    }
    let Some(mesh) = preview
        .mesh
        .as_mut()
        .filter(|m| matches!(m.stage, Stage::Ready(..)))
    else {
        return;
    };
    mesh.stage = Stage::Framing(ready.entity);
    mesh.started = Instant::now();
    commands.entity(ready.entity).insert(Transform::IDENTITY);
    preview.rig = Arc::default();
    preview.generation += 1;
    preview.raster = None;
    preview.processing = None;
    preview.rebind_camera(&mut commands);
    redraw.write(RequestRedraw);
}
