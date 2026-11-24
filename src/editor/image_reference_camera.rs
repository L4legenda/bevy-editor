//! Reuse the prepared model and an offscreen PBR camera throughout a canvas gesture.
use super::*;

#[derive(PartialEq)]
struct View {
    direction: [f32; 3],
    center: [f32; 2],
    size: UVec2,
    pose: crate::Pose,
}
pub(super) struct LiveCamera {
    root: Entity,
    camera: Entity,
    light: Entity,
    image: Handle<Image>,
    view: Option<View>,
    rect: [f32; 4],
    active: bool,
    warm_frames: u8,
    readback: Option<Entity>,
    attempts: u8,
    verified: bool,
}

impl Preview {
    pub(super) fn clear_camera(&mut self, commands: &mut Commands) {
        if let Some(rig) = self.camera.take() {
            commands.entity(rig.root).try_despawn();
        }
    }

    pub(in crate::editor) fn track_camera_frame(&mut self, ready: bool) {
        if let Some(rig) = self.camera.as_mut().filter(|r| r.active && !r.verified) {
            rig.warm_frames = if ready {
                rig.warm_frames.saturating_add(1)
            } else {
                0
            };
        }
    }

    pub(in crate::editor) fn update_camera(
        &mut self,
        request: Option<(&ModelReference, UVec2, Option<crate::Pose>)>,
        commands: &mut Commands,
        images: &mut Assets<Image>,
        redraw: &mut MessageWriter<RequestRedraw>,
    ) -> Option<(Handle<Image>, [f32; 4])> {
        let Some((reference, size, requested)) = request else {
            self.clear_camera(commands);
            return None;
        };
        let mesh = self.mesh.as_ref().filter(|m| m.asset == reference.asset)?;
        let Stage::Ready(bounds, _) = &mesh.stage else {
            return None;
        };
        let model = self.world_root?;
        let view = View {
            direction: reference.direction,
            center: reference.center,
            size,
            pose: requested.unwrap_or(reference.pose),
        };
        let rig = self.camera.get_or_insert_with(|| {
            let mut target = Image::new_target_texture(
                RESOLUTION,
                RESOLUTION,
                TextureFormat::Rgba8UnormSrgb,
                None,
            );
            target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            let image = images.add(target);
            let root = commands
                .spawn((Transform::IDENTITY, Visibility::Inherited))
                .id();
            let (_, frame) = projected_frame(
                *bounds,
                view.direction.into(),
                view.center.into(),
                size,
                view.pose,
            );
            let (camera, light) = spawn_camera(
                commands,
                root,
                frame.view,
                frame.projection,
                image.clone(),
                LIVE_LAYER,
            );
            LiveCamera {
                root,
                camera,
                light,
                image,
                view: None,
                rect: [0.; 4],
                active: true,
                warm_frames: 0,
                readback: None,
                attempts: 0,
                verified: false,
            }
        });
        let verifying = !rig.verified && (rig.attempts < MAX_CAPTURES || rig.readback.is_some());
        let active = verifying || (rig.verified && requested.is_some());
        if rig.active != active {
            commands
                .entity(rig.camera)
                .entry::<Camera>()
                .and_modify(move |mut c| c.is_active = active);
            rig.active = active;
        }
        if !active {
            return None;
        }
        if rig.view.as_ref() != Some(&view) {
            let (transform, frame) = projected_frame(
                *bounds,
                view.direction.into(),
                view.center.into(),
                size,
                view.pose,
            );
            commands.entity(model).insert(transform);
            commands
                .entity(rig.camera)
                .insert((frame.view, frame.projection));
            let direction =
                *frame.view.back() * 0.55 - *frame.view.right() * 0.55 + *frame.view.up();
            commands
                .entity(rig.light)
                .insert(Transform::from_translation(direction).looking_at(Vec3::ZERO, Vec3::Y));
            let lo = frame.center - Vec2::splat(frame.pixel_span * 0.5);
            let hi = frame.center + Vec2::splat(frame.pixel_span * 0.5);
            rig.rect = [lo.x, lo.y, hi.x, hi.y];
            rig.view = Some(view);
        }
        if verifying {
            // Probe once on a working adapter. Empty captures retain the software path.
            if rig.warm_frames >= 3 && rig.readback.is_none() {
                rig.attempts += 1;
                rig.readback = Some(
                    commands
                        .spawn((ReadbackOnce::texture(rig.image.clone()), ChildOf(rig.root)))
                        .observe(probed)
                        .id(),
                );
            }
            redraw.write(RequestRedraw);
        }
        (requested.is_some() && rig.verified).then(|| (rig.image.clone(), rig.rect))
    }
}

fn probed(
    event: On<ReadbackComplete>,
    mut render: ResMut<ReferenceRender>,
    mut commands: Commands,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    if let Some(rig) = render.live.camera.as_mut()
        && rig.readback == Some(event.entity)
    {
        rig.verified = event.data.chunks_exact(4).any(|p| p[3] != 0);
        rig.readback = None;
        rig.warm_frames = 0;
        commands.entity(event.entity).try_despawn();
        redraw.write(RequestRedraw);
    }
}
