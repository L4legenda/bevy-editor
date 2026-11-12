//! A transient model follows the pointer; only release creates an undoable edit.
use bevy::{
    camera::{primitives::Aabb, visibility::RenderLayers},
    ecs::system::SystemParam,
    light::{NotShadowCaster, RectLight},
    picking::mesh_picking::ray_cast::MeshRayCast,
    prelude::*,
    window::PrimaryWindow,
    world_serialization::WorldInstanceReady,
};

use super::{drop_point, placement_position};
use crate::editor::{
    Action, Editor, Scene, Selection, apply_asset_drop, bounds_for,
    browser::{Browser, Entry},
    load_error, ui, viewport,
};
use crate::{BoxBounds, PlacedModel, runtime};

const COLOR: Color = Color::srgb(0.29, 0.77, 1.0);
const LOAD_TIMEOUT: f64 = 30.0;
const PREVIEW_OPACITY: f32 = 0.8;

#[derive(Component)]
pub(in crate::editor) struct PlacementGhost;
#[derive(Component)]
pub(in crate::editor) struct Prepared;

struct Job {
    root: Entity,
    handle: Handle<WorldAsset>,
    asset: String,
    entry: Entry,
    room: usize,
    scan_revision: u64,
    generation: u64,
    started: f64,
    bounds: Option<BoxBounds>,
    contact: Option<Vec3>,
    pending: bool,
    error: Option<&'static str>,
}

#[derive(Resource, Default)]
pub(in crate::editor) struct PlacementPreview {
    job: Option<Job>,
}

#[derive(SystemParam)]
pub(in crate::editor) struct PlacementRay<'w, 's> {
    camera: Query<
        'w,
        's,
        (&'static Camera, &'static Transform),
        (With<viewport::EditorCamera>, Without<PlacementGhost>),
    >,
    parents: Query<'w, 's, &'static ChildOf>,
    meshes: MeshRayCast<'w, 's>,
}
impl PlacementPreview {
    fn clear(&mut self, commands: &mut Commands) {
        if let Some(job) = self.job.take() {
            commands.entity(job.root).try_despawn();
        }
    }
    pub fn loading(&self) -> bool {
        self.job.as_ref().is_some_and(|job| job.bounds.is_none())
    }
    #[cfg(test)]
    pub fn position(&self) -> Option<Vec3> {
        let job = self.job.as_ref()?;
        placement_position(job.contact?, job.bounds?)
    }
    #[cfg(test)]
    pub fn contact(&self) -> Option<Vec3> {
        self.job.as_ref()?.contact
    }
}

fn prepare(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    meshes: Query<&MeshMaterial3d<StandardMaterial>, With<Mesh3d>>,
    mut cameras: Query<&mut Camera>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Each source material gets a temporary copy; textures and PBR properties
    // stay shared, while the real model's opacity remains untouched.
    let mut copies = std::collections::HashMap::<
        bevy::asset::AssetId<StandardMaterial>,
        Handle<StandardMaterial>,
    >::new();
    for entity in std::iter::once(ready.entity).chain(children.iter_descendants(ready.entity)) {
        let mut instance = commands.entity(entity);
        // Match the final model's layer so scene lights shade the preview too.
        instance.insert(RenderLayers::default());
        // Imported cameras and lights must not affect any editor view.
        instance.remove::<(PointLight, SpotLight, DirectionalLight, RectLight)>();
        if let Ok(original) = meshes.get(entity) {
            let id = original.0.id();
            let copy = if let Some(handle) = copies.get(&id) {
                Some(handle.clone())
            } else if let Some(mut material) = materials.get(&original.0).cloned() {
                material.base_color = material
                    .base_color
                    .with_alpha(material.base_color.alpha() * PREVIEW_OPACITY);
                material.alpha_mode = AlphaMode::Blend;
                let handle = materials.add(material);
                copies.insert(id, handle.clone());
                Some(handle)
            } else {
                None
            };
            if let Some(copy) = copy {
                instance.insert((MeshMaterial3d(copy), NotShadowCaster));
            }
        }
        if let Ok(mut camera) = cameras.get_mut(entity) {
            camera.is_active = false;
        }
    }
    commands.entity(ready.entity).insert(Prepared);
}

// Bounds are measured after propagation, using the same helper as saved models.
// This also works with translated/scaled children and off-center GLB origins.
pub(in crate::editor) fn measure(
    mut preview: ResMut<PlacementPreview>,
    prepared: Query<(), With<Prepared>>,
    children: Query<&Children>,
    meshes: Query<(&Aabb, &GlobalTransform)>,
    transforms: Query<&GlobalTransform>,
    geometry: Query<(), With<Mesh3d>>,
) {
    let Some(job) = preview.job.as_mut().filter(|job| job.bounds.is_none()) else {
        return;
    };
    if !prepared.contains(job.root) {
        return;
    }
    if let Some(bounds) = bounds_for(job.root, &children, &meshes, &transforms) {
        if bounds.valid() {
            job.bounds = Some(bounds);
        } else {
            job.error = Some("Модель слишком велика для размещения");
        }
    } else if !children
        .iter_descendants(job.root)
        .any(|entity| geometry.contains(entity))
    {
        job.error = Some("В модели нет геометрии");
    }
}

pub(in crate::editor) fn update(
    mut commands: Commands,
    mut editor: ResMut<Editor>,
    browser: Res<Browser>,
    scene: Res<Scene>,
    window: Single<&Window, With<PrimaryWindow>>,
    motion: Res<ui::DrawerMotion>,
    drag: Res<ui::AssetDrag>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    server: Res<AssetServer>,
    mut preview: ResMut<PlacementPreview>,
    mut ray: PlacementRay,
    mut roots: Query<(&mut Transform, &mut Visibility), With<PlacementGhost>>,
    numeric: Option<Res<ui::NumericInput>>,
    mut actions: MessageReader<Action>,
) {
    let mut released = None;
    let mut changing_scene = false;
    for action in actions.read() {
        match *action {
            Action::DropAsset(entry, cursor) => released = Some((entry, cursor)),
            Action::Open(room) if room != editor.room => changing_scene = true,
            Action::CloseTab(room) if room == editor.room => changing_scene = true,
            _ => {}
        }
    }
    if !window.focused
        || !editor.has_scene()
        || !editor.ready
        || editor.pending.is_some()
        || drag.cancelled_this_frame
        || keys.just_pressed(KeyCode::Escape)
        || changing_scene
        || preview.job.as_ref().is_some_and(|job| {
            job.room != editor.room || job.scan_revision != browser.scan_revision
        })
    {
        if preview.job.as_ref().is_some_and(|job| job.pending) {
            editor.status = "Размещение отменено".into();
        }
        preview.clear(&mut commands);
        return;
    }
    let source = drag.payload.filter(|_| drag.active());
    let entry = released
        .map(|(entry, _)| entry)
        .or_else(|| source.map(|source| source.entry));
    if let Some(entry) = entry {
        let Some(asset) = browser.model_asset(&editor.project.catalog, entry) else {
            preview.clear(&mut commands);
            return;
        };
        let generation = drag.generation;
        let replace = preview
            .job
            .as_ref()
            .is_none_or(|job| job.asset != asset || job.generation != generation);
        if replace {
            preview.clear(&mut commands);
            let model = PlacedModel {
                id: 0,
                asset: asset.clone(),
                pose: Transform::IDENTITY.into(),
                bounds: None,
            };
            let handle = runtime::load_model(&server, &model);
            let root = commands
                .spawn((
                    PlacementGhost,
                    WorldAssetRoot(handle.clone()),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                    Name::new("Placement preview"),
                ))
                .observe(prepare)
                .id();
            preview.job = Some(Job {
                root,
                handle,
                asset,
                entry,
                room: editor.room,
                scan_revision: browser.scan_revision,
                generation,
                started: time.elapsed_secs_f64(),
                bounds: None,
                contact: None,
                pending: false,
                error: None,
            });
        }
    }
    let Some(job) = preview.job.as_mut() else {
        return;
    };
    let error = job
        .error
        .map(str::to_owned)
        .or_else(|| load_error(&server, &job.handle))
        .or_else(|| {
            (time.elapsed_secs_f64() - job.started > LOAD_TIMEOUT)
                .then(|| "Не удалось загрузить модель".into())
        });
    if let Some(error) = error {
        editor.status = format!("Не удалось разместить · {error}");
        preview.clear(&mut commands);
        return;
    }
    if released.is_some() || source.is_some() {
        let cursor = released
            .map(|(_, cursor)| cursor)
            .or_else(|| drag.scene_cursor());
        job.contact = cursor.and_then(|cursor| {
            let over_preview = !editor.pilot
                && matches!(editor.selection, Some(Selection::Camera(_)))
                && ui::camera_preview_rect(&window, &motion, &editor.sidebar).contains(cursor);
            if !ui::scene_input_rect(&window, &motion, &editor.sidebar).contains(cursor)
                || over_preview
            {
                return None;
            }
            let (camera, transform) = ray.camera.single().ok()?;
            drop_point(
                camera,
                transform,
                cursor - ui::viewport_rect(&window, &editor.sidebar).min,
                &mut ray.meshes,
                &scene,
                &ray.parents,
                editor.snap,
            )
        });
    }
    if released.is_some() {
        if let Some(error) = numeric.as_ref().and_then(|input| input.error_message()) {
            editor.status = error.into();
            preview.clear(&mut commands);
            return;
        }
        if job.contact.is_none() {
            preview.clear(&mut commands);
            return;
        }
        job.pending = true;
        if job.bounds.is_none() {
            editor.status = "Загрузка модели…".into();
        }
    }
    if source.is_none() && !job.pending {
        preview.clear(&mut commands);
        return;
    }
    let position = job
        .contact
        .zip(job.bounds)
        .and_then(|(point, bounds)| placement_position(point, bounds));
    if let Ok((mut transform, mut visibility)) = roots.get_mut(job.root) {
        *visibility = if position.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if let Some(position) = position {
            transform.translation = position;
        }
    }
    if job.pending
        && let Some(position) = position
    {
        apply_asset_drop(&mut editor, &browser, job.entry, position, job.bounds);
        preview.clear(&mut commands);
    }
}

pub(in crate::editor) fn draw(
    preview: Res<PlacementPreview>,
    mut gizmos: Gizmos<viewport::TransformGizmos>,
) {
    let Some(job) = &preview.job else {
        return;
    };
    let Some(point) = job.contact else {
        return;
    };
    let point = point + Vec3::Y * 0.012;
    let radius = job.bounds.map_or(0.25, |bounds| {
        (bounds.half_size[0].min(bounds.half_size[2]) * 0.4).clamp(0.12, 0.45)
    });
    gizmos.circle(
        Isometry3d::new(point, Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        radius,
        COLOR,
    );
    gizmos.line(
        point - Vec3::X * radius * 1.4,
        point + Vec3::X * radius * 1.4,
        COLOR,
    );
    gizmos.line(
        point - Vec3::Z * radius * 1.4,
        point + Vec3::Z * radius * 1.4,
        COLOR,
    );
    if let Some(bounds) = job.bounds {
        let x = bounds.half_size[0];
        let z = bounds.half_size[2];
        gizmos.linestrip(
            [
                point + Vec3::new(-x, 0.0, -z),
                point + Vec3::new(x, 0.0, -z),
                point + Vec3::new(x, 0.0, z),
                point + Vec3::new(-x, 0.0, z),
                point + Vec3::new(-x, 0.0, -z),
            ],
            COLOR,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::world_serialization::WorldInstanceSpawner;

    #[test]
    fn preview_material_is_local_and_imported_lights_and_cameras_cannot_affect_the_scene() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<StandardMaterial>>();
        app.update();
        let texture = Handle::<Image>::default();
        let original = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::srgb(0.72, 0.36, 0.13),
                base_color_texture: Some(texture.clone()),
                normal_map_texture: Some(texture.clone()),
                metallic: 0.6,
                perceptual_roughness: 0.25,
                ..default()
            });
        let root = app
            .world_mut()
            .spawn((PlacementGhost, Transform::IDENTITY))
            .observe(prepare)
            .id();
        let mesh = app
            .world_mut()
            .spawn((
                Mesh3d(Handle::default()),
                MeshMaterial3d(original.clone()),
                ChildOf(root),
            ))
            .id();
        let shared_mesh = app
            .world_mut()
            .spawn((
                Mesh3d(Handle::default()),
                MeshMaterial3d(original.clone()),
                ChildOf(root),
            ))
            .id();
        let glass_original = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::srgba(0.2, 0.6, 0.8, 0.5),
                alpha_mode: AlphaMode::Blend,
                ..default()
            });
        let glass_mesh = app
            .world_mut()
            .spawn((
                Mesh3d(Handle::default()),
                MeshMaterial3d(glass_original.clone()),
                ChildOf(root),
            ))
            .id();
        let light = app
            .world_mut()
            .spawn((PointLight::default(), ChildOf(root)))
            .id();
        let camera = app
            .world_mut()
            .spawn((Camera::default(), ChildOf(root)))
            .id();
        let main_light = app.world_mut().spawn(PointLight::default()).id();
        let main_camera = app.world_mut().spawn(Camera::default()).id();
        let mut worlds = Assets::<WorldAsset>::default();
        let world = worlds.add(WorldAsset::new(World::new()));
        app.insert_resource(worlds);
        let instance_id = WorldInstanceSpawner::default()
            .spawn_sync(app.world_mut(), world.id())
            .unwrap();
        app.world_mut().trigger(WorldInstanceReady {
            entity: root,
            instance_id,
        });
        app.update();
        assert!(app.world().get::<Prepared>(root).is_some());
        assert!(app.world().get::<PointLight>(light).is_none());
        assert!(!app.world().get::<Camera>(camera).unwrap().is_active);
        assert!(app.world().get::<PointLight>(main_light).is_some());
        assert!(app.world().get::<Camera>(main_camera).unwrap().is_active);
        assert_eq!(
            app.world().get::<RenderLayers>(mesh),
            Some(&RenderLayers::default())
        );
        assert!(app.world().get::<NotShadowCaster>(mesh).is_some());
        let ghost = app
            .world()
            .get::<MeshMaterial3d<StandardMaterial>>(mesh)
            .unwrap();
        assert_ne!(ghost.0, original);
        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let copied = materials.get(&ghost.0).unwrap();
        assert!(!copied.unlit);
        assert_eq!(copied.base_color, Color::srgba(0.72, 0.36, 0.13, 0.8));
        assert_eq!(copied.base_color_texture, Some(texture.clone()));
        assert_eq!(copied.normal_map_texture, Some(texture));
        assert_eq!(copied.metallic, 0.6);
        assert_eq!(copied.perceptual_roughness, 0.25);
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(shared_mesh)
                .unwrap()
                .0,
            ghost.0
        );
        let glass_copy = &app
            .world()
            .get::<MeshMaterial3d<StandardMaterial>>(glass_mesh)
            .unwrap()
            .0;
        assert!((materials.get(glass_copy).unwrap().base_color.alpha() - 0.4).abs() < 1e-6);
        assert_eq!(
            materials.get(&glass_original).unwrap().base_color.alpha(),
            0.5
        );
        assert_eq!(materials.get(&original).unwrap().base_color.alpha(), 1.0);
        assert_eq!(
            materials.get(&ghost.0).unwrap().alpha_mode,
            AlphaMode::Blend
        );
        assert_eq!(
            materials.get(&original).unwrap().alpha_mode,
            AlphaMode::Opaque
        );
    }
}

#[cfg(test)]
mod pending_tests {
    use super::*;

    fn pending() -> (App, Entity) {
        let mut project = crate::EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"),
        )
        .unwrap();
        let browser = Browser::new(&mut project);
        let revision = browser.scan_revision;
        let asset = project.catalog[0].asset.clone();
        let mut editor = Editor::new(project, Default::default());
        editor.ready = true;
        editor.reload = false;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<WorldAsset>()
            .init_resource::<Assets<Mesh>>()
            .insert_resource(editor)
            .insert_resource(browser)
            .init_resource::<Scene>()
            .init_resource::<ui::DrawerMotion>()
            .init_resource::<ui::AssetDrag>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<Action>()
            .add_systems(Update, update);
        app.world_mut().spawn((
            PrimaryWindow,
            Window {
                focused: true,
                ..default()
            },
        ));
        let root = app
            .world_mut()
            .spawn((PlacementGhost, Transform::IDENTITY, Visibility::Hidden))
            .id();
        let handle = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .add(WorldAsset::new(World::new()));
        app.insert_resource(PlacementPreview {
            job: Some(Job {
                root,
                handle,
                asset,
                entry: Entry::Model(0),
                room: 0,
                scan_revision: revision,
                generation: 0,
                started: 0.0,
                bounds: None,
                contact: Some(Vec3::new(3.0, 0.0, -2.0)),
                pending: true,
                error: None,
            }),
        });
        (app, root)
    }

    #[test]
    fn release_waits_for_geometry_then_places_once_at_the_frozen_contact() {
        let (mut app, root) = pending();
        app.update();
        assert!(app.world().resource::<Editor>().edits.models.is_empty());
        assert!(app.world().resource::<Editor>().undo.is_empty());
        assert!(app.world().get_entity(root).is_ok());
        app.world_mut()
            .resource_mut::<PlacementPreview>()
            .job
            .as_mut()
            .unwrap()
            .bounds = Some(BoxBounds {
            center: [0.0; 3],
            half_size: [0.5; 3],
        });
        app.update();
        app.update();
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.edits.models.len(), 1);
        assert_eq!(editor.edits.models[0].pose.position, [3.0, 0.5, -2.0]);
        assert_eq!(editor.undo.len(), 1);
        assert!(app.world().get_entity(root).is_err());
        assert!(app.world().resource::<PlacementPreview>().job.is_none());
    }

    #[test]
    fn escape_lost_focus_scene_switch_and_rescan_cancel_an_unfinished_drop() {
        for reason in 0..4 {
            let (mut app, root) = pending();
            match reason {
                0 => app
                    .world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::Escape),
                1 => {
                    let world = app.world_mut();
                    let mut windows = world.query::<&mut Window>();
                    windows.single_mut(world).unwrap().focused = false;
                }
                2 => app.world_mut().resource_mut::<Editor>().room = 1,
                _ => app.world_mut().resource_mut::<Browser>().scan_revision += 1,
            }
            app.update();
            assert!(app.world().resource::<PlacementPreview>().job.is_none());
            assert!(app.world().resource::<Editor>().edits.models.is_empty());
            assert!(app.world().resource::<Editor>().undo.is_empty());
            assert!(app.world().get_entity(root).is_err());
        }
    }
}
