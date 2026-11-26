//! Publish only extracted camera frames. The UI samples the front texture while
//! the same camera and skeleton prepare the next pose in the other texture.
use super::*;

struct CameraFrame {
    shot: Arc<Shot>,
    slot: usize,
    frames: u8,
    started: Instant,
    pipelines_ready: bool,
}

pub(super) struct LiveCamera {
    root: Entity,
    camera: Entity,
    light: Entity,
    images: [Handle<Image>; 2],
    side: u32,
    work: Option<CameraFrame>,
    front: Option<(Handle<Image>, Arc<Shot>)>,
    active: bool,
    verified: bool,
    readback: Option<Entity>,
    attempts: u8,
}

impl LiveCamera {
    fn targets(images: &mut Assets<Image>, side: u32) -> [Handle<Image>; 2] {
        std::array::from_fn(|_| {
            let mut target =
                Image::new_target_texture(side, side, TextureFormat::Rgba8UnormSrgb, None);
            target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            images.add(target)
        })
    }
    fn new(
        commands: &mut Commands,
        images: &mut Assets<Image>,
        side: u32,
        frame: &ReferenceFrame,
    ) -> Self {
        let targets = Self::targets(images, side);
        let root = commands
            .spawn((Transform::IDENTITY, Visibility::Inherited))
            .id();
        let (camera, light) = spawn_camera(
            commands,
            root,
            frame.view,
            frame.projection.clone(),
            targets[0].clone(),
            LAYER,
        );
        commands.entity(camera).insert(ReferenceCamera);
        Self {
            root,
            camera,
            light,
            images: targets,
            side,
            work: None,
            front: None,
            active: true,
            verified: false,
            readback: None,
            attempts: 0,
        }
    }

    fn active(&mut self, active: bool, commands: &mut Commands) {
        if self.active != active {
            self.active = active;
            commands
                .entity(self.camera)
                .entry::<Camera>()
                .and_modify(move |mut c| c.is_active = active);
        }
    }
}

fn resolution(span: f32, device_limit: u32) -> u32 {
    // Each 64 RGBA pixels fills a readback row alignment unit. Size classes avoid
    // reallocating on every pointer sample while scaling a large character.
    let limit = device_limit.min(4096) / 64 * 64;
    (span.ceil() as u32)
        .clamp(RESOLUTION, 4096)
        .next_power_of_two()
        .min(limit)
}

impl Preview {
    #[cfg(test)]
    pub(in crate::editor) fn presented_frame(&self) -> Option<(u64, &Handle<Image>)> {
        self.camera
            .as_ref()?
            .front
            .as_ref()
            .map(|(image, shot)| (shot.id, image))
    }

    pub(super) fn rebind_camera(&mut self, commands: &mut Commands) {
        if let Some(camera) = &mut self.camera {
            if let Some(readback) = camera.readback.take() {
                commands.entity(readback).try_despawn();
            }
            camera.work = None;
            camera.attempts = 0;
            camera.verified = false;
            camera.active(false, commands);
        }
    }
    pub(super) fn clear_camera(&mut self, commands: &mut Commands) {
        if let Some(camera) = self.camera.take() {
            commands.entity(camera.root).try_despawn();
        }
    }

    pub(in crate::editor::image_reference) fn track_camera_frame(&mut self, pipelines_ready: bool) {
        if let Some(camera) = &mut self.camera
            && camera.active
            && let Some(work) = &mut camera.work
        {
            work.frames = work.frames.saturating_add(1);
            work.pipelines_ready = pipelines_ready;
        }
    }

    pub(in crate::editor::image_reference) fn presented(
        &self,
        view: &ModelView,
    ) -> Option<CameraImage> {
        let camera = self.camera.as_ref()?;
        let (image, shot) = camera
            .front
            .as_ref()
            .filter(|(_, shot)| shot.view.same_layer_view(view))?;
        let half = Vec2::splat(shot.frame.pixel_span * 0.5);
        let lo = shot.frame.center - half;
        let hi = shot.frame.center + half;
        Some(CameraImage {
            image: image.clone(),
            rect: [lo.x, lo.y, hi.x, hi.y],
            shot: shot.clone(),
        })
    }

    pub(in crate::editor::image_reference) fn render(
        &mut self,
        view: Option<&ModelView>,
        pixels: bool,
        max_side: u32,
        commands: &mut Commands,
        images: &mut Assets<Image>,
        redraw: &mut MessageWriter<RequestRedraw>,
    ) -> Result<(), String> {
        if let Some(result) = self
            .processing
            .as_mut()
            .and_then(|task| block_on(poll_once(task)))
        {
            self.processing = None;
            self.raster = Some(result?);
        }
        let Some(view) = view else {
            if let Some(camera) = &mut self.camera {
                camera.active(false, commands);
            }
            return Ok(());
        };
        let Some(mesh) = &self.mesh else {
            return Ok(());
        };
        if let Stage::Failed(error) = &mesh.stage {
            return Err(error.clone());
        }
        let Stage::Ready(bounds, _) = mesh.stage else {
            return Ok(());
        };
        let same = self
            .camera
            .as_ref()
            .and_then(|c| c.work.as_ref())
            .is_some_and(|work| &work.shot.view == view);
        let have_pixels = self.raster.as_ref().is_some_and(|r| &r.shot.view == view);
        if let Some(camera) = &self.camera
            && same
            && !camera.active
            && camera.readback.is_none()
            && ((camera.verified && (!pixels || have_pixels))
                || (camera.attempts >= MAX_CAPTURES && have_pixels))
        {
            return Ok(());
        }
        if !self.rig.contains_pose(&view.pose.joints) {
            return Err("Скелет модели изменился".into());
        }
        let (transform, frame) = projected_frame(
            bounds,
            view.direction.into(),
            view.center.into(),
            view.size,
            view.pose.root,
            self.rig.positions(&view.pose.joints),
        );
        let side = resolution(frame.pixel_span, max_side);
        let camera = self
            .camera
            .get_or_insert_with(|| LiveCamera::new(commands, images, side, &frame));
        if camera.side != side && camera.readback.is_none() {
            // The previous front stays visible while the camera draws at the new resolution.
            camera.images = LiveCamera::targets(images, side);
            camera.side = side;
            camera.work = None;
        }
        if camera.readback.is_some() {
            redraw.write(RequestRedraw);
            return Ok(());
        }
        if camera.attempts >= MAX_CAPTURES && !camera.verified {
            camera.active(false, commands);
            if !have_pixels && self.processing.is_none() {
                self.serial += 1;
                self.software(Arc::new(Shot {
                    view: view.clone(),
                    frame,
                    id: self.serial,
                    rig: self.rig.clone(),
                    model: transform,
                }));
            }
            if !have_pixels {
                redraw.write(RequestRedraw);
            }
            return Ok(());
        }
        if let Some(work) = &camera.work {
            if camera.verified && work.frames > 0 {
                camera.front = Some((camera.images[work.slot].clone(), work.shot.clone()));
            }
            // Other editor views may be compiling their own pipelines. The
            // camera's readback can prove this model is ready independently.
            let probe =
                !camera.verified && work.frames >= if work.pipelines_ready { 3 } else { 12 };
            let capture = pixels
                && !have_pixels
                && self.processing.is_none()
                && &work.shot.view == view
                && camera.verified
                && work.frames > 0;
            if probe || capture {
                camera.readback = Some(
                    commands
                        .spawn((
                            ReadbackOnce::texture(camera.images[work.slot].clone()),
                            ChildOf(camera.root),
                        ))
                        .observe(captured)
                        .id(),
                );
                redraw.write(RequestRedraw);
                return Ok(());
            }
        }
        let change = camera
            .work
            .as_ref()
            .is_none_or(|work| &work.shot.view != view && camera.verified && work.frames > 0);
        if change {
            self.serial += 1;
            let slot = usize::from(
                camera
                    .front
                    .as_ref()
                    .is_some_and(|(image, _)| image == &camera.images[0]),
            );
            let root = self.world_root.expect("a prepared scene has a root");
            commands.entity(root).insert(transform);
            if camera
                .work
                .as_ref()
                .is_none_or(|work| work.shot.view.pose.joints != view.pose.joints)
            {
                self.rig.apply(&view.pose.joints, commands);
            }
            commands.entity(camera.camera).insert((
                frame.view,
                frame.projection.clone(),
                RenderTarget::Image(camera.images[slot].clone().into()),
            ));
            let direction =
                *frame.view.back() * 0.55 - *frame.view.right() * 0.55 + *frame.view.up();
            commands
                .entity(camera.light)
                .insert(Transform::from_translation(direction).looking_at(Vec3::ZERO, Vec3::Y));
            camera.work = Some(CameraFrame {
                slot,
                frames: 0,
                started: Instant::now(),
                pipelines_ready: false,
                shot: Arc::new(Shot {
                    view: view.clone(),
                    frame,
                    id: self.serial,
                    rig: self.rig.clone(),
                    model: transform,
                }),
            });
        }
        let waiting = !camera.verified || camera.work.as_ref().is_some_and(|work| work.frames == 0);
        camera.active(waiting, commands);
        if waiting || (pixels && !have_pixels) {
            redraw.write(RequestRedraw);
        }
        Ok(())
    }
}

fn captured(
    mut event: On<ReadbackComplete>,
    mut render: ResMut<ReferenceRender>,
    mut commands: Commands,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let preview = &mut render.live;
    let Some(camera) = preview
        .camera
        .as_mut()
        .filter(|c| c.readback == Some(event.entity))
    else {
        return;
    };
    commands.entity(event.entity).try_despawn();
    camera.readback = None;
    let Some(work) = &mut camera.work else { return };
    let bytes = std::mem::take(&mut event.event_mut().data);
    if bytes.chunks_exact(4).any(|p| p[3] != 0) {
        camera.verified = true;
        camera.attempts = 0;
        let shot = work.shot.clone();
        let side = camera.side;
        preview.processing = Some(AsyncComputeTaskPool::get().spawn(async move {
            let image = image::RgbaImage::from_raw(side, side, bytes)
                .ok_or("Не удалось прочитать кадр модели")?;
            Ok(shot.surface(&image))
        }));
    } else {
        work.frames = 0;
        camera.verified = false;
        if work.pipelines_ready || work.started.elapsed() >= TIMEOUT / 3 {
            camera.attempts += 1;
        }
        if camera.attempts >= MAX_CAPTURES {
            camera.front = None;
        }
        camera.active(true, &mut commands);
    }
    redraw.write(RequestRedraw);
}

#[cfg(test)]
mod tests {
    use super::resolution;

    #[test]
    fn camera_resolution_matches_layer_detail_and_aligned_device_limits() {
        assert_eq!(resolution(256., 8192), 1024);
        assert_eq!(resolution(1600., 8192), 2048);
        assert_eq!(resolution(7500., 8192), 4096);
        assert_eq!(resolution(3500., 2048), 2048);
    }
}
