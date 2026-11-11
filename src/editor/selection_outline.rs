//! An isolated selection mask, composited over the scene even behind occluders.
use std::collections::{HashMap, HashSet};

use bevy::{
    camera::{
        ClearColorConfig, ImageRenderTarget, RenderTarget,
        visibility::{InheritedVisibility, NoFrustumCulling, RenderLayers, ViewVisibility},
    },
    light::{NotShadowCaster, NotShadowReceiver},
    mesh::{morph::MeshMorphWeights, skinning::SkinnedMesh},
    prelude::*,
    render::{
        batching::NoAutomaticBatching,
        render_resource::{Extent3d, TextureFormat},
        view::{DebandDither, Tonemapping},
    },
};

use super::{Editor, Scene, Selection, viewport::EditorCamera};

#[path = "selection_outline_material.rs"]
mod material;
pub(super) use material::{OutlineMaskMaterial, OutlineMaterial, OutlineMaterialPlugin};

#[cfg(test)]
#[path = "selection_outline_render_tests.rs"]
pub(in crate::editor) mod render_tests;

pub(super) const LAYER: usize = 4;
const WIDTH: f32 = 2.5;

#[derive(Component)]
struct MaskCamera;

#[derive(Component)]
struct Proxy;

#[derive(Resource)]
pub(super) struct Outline {
    pub material: Handle<OutlineMaterial>,
    pub mask: Handle<Image>,
    pub camera: Entity,
    pub visible: bool,
    white: Handle<OutlineMaskMaterial>,
    scale: f32,
    proxies: HashMap<Entity, Entity>,
}

pub(super) fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<OutlineMaterial>>,
    mut masks: ResMut<Assets<OutlineMaskMaterial>>,
) {
    let mask = images.add(Image::new_target_texture(
        1,
        1,
        TextureFormat::Rgba8Unorm,
        Some(TextureFormat::Rgba8UnormSrgb),
    ));
    let mut material = OutlineMaterial::new(mask.clone());
    material.set_width(0.0);
    let camera = commands
        .spawn((
            MaskCamera,
            Camera3d::default(),
            Camera {
                order: -2,
                is_active: false,
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            RenderTarget::Image(mask.clone().into()),
            RenderLayers::layer(LAYER),
            Msaa::Off,
            Tonemapping::None,
            DebandDither::Disabled,
        ))
        .id();
    commands.insert_resource(Outline {
        material: materials.add(material),
        mask,
        camera,
        visible: false,
        white: masks.add(OutlineMaskMaterial::new(LinearRgba::WHITE)),
        scale: 1.0,
        proxies: HashMap::new(),
    });
}

fn selected(world: &World) -> Option<Entity> {
    let editor = world.get_resource::<Editor>()?;
    if !editor.has_scene() || !editor.ready || editor.pilot {
        return None;
    }
    match editor.selection? {
        Selection::Imported(index) => editor.imported.get(index).map(|node| node.entity),
        Selection::Model(id) => world
            .get_resource::<Scene>()?
            .models
            .get(&id)
            .map(|(entity, _)| *entity),
        Selection::Camera(_) | Selection::Light(_) | Selection::Environment => None,
    }
    .filter(|&entity| world.entities().contains(entity))
}

fn width(world: &mut World, outline: &Outline, pixels: f32) {
    if let Some(mut materials) = world.get_resource_mut::<Assets<OutlineMaterial>>()
        && let Some(mut material) = materials.get_mut(&outline.material)
    {
        material.set_width(pixels);
    }
}

fn hide(world: &mut World, outline: &mut Outline) {
    outline.visible = false;
    width(world, outline, 0.0);
    if let Some(mut camera) = world.get_mut::<Camera>(outline.camera) {
        camera.is_active = false;
    }
}

/// Update before CameraUpdateSystems/UpdateFrusta, after viewport navigation.
/// Camera transforms are propagated normally; mesh globals are copied later.
pub(super) fn sync_camera(world: &mut World) {
    let Some(mut outline) = world.remove_resource::<Outline>() else {
        return;
    };
    sync_camera_inner(world, &mut outline);
    world.insert_resource(outline);
}

fn sync_camera_inner(world: &mut World, outline: &mut Outline) {
    if selected(world).is_none() || !world.entities().contains(outline.camera) {
        hide(world, outline);
        return;
    }
    let source = world.query_filtered::<(&Camera, &RenderTarget, &Transform, &Projection), (With<EditorCamera>, Without<MaskCamera>)>()
        .iter(world).next().map(|(camera, target, transform, projection)| (camera.clone(), target.clone(), *transform, projection.clone()));
    let Some((source, RenderTarget::Image(target), transform, projection)) = source else {
        hide(world, outline);
        return;
    };
    if !source.is_active || !target.scale_factor.is_finite() || target.scale_factor <= 0.0 {
        hide(world, outline);
        return;
    }
    let size = world
        .get_resource::<Assets<Image>>()
        .and_then(|images| images.get(&target.handle))
        .map(|image| image.texture_descriptor.size);
    let Some(size) = size.filter(|size| size.width > 0 && size.height > 0) else {
        hide(world, outline);
        return;
    };
    if let Some(mut images) = world.get_resource_mut::<Assets<Image>>()
        && let Some(mut mask) = images.get_mut(&outline.mask)
        && mask.texture_descriptor.size != size
    {
        mask.resize(Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        });
    }
    if let Some(mut camera) = world.get_mut::<Camera>(outline.camera) {
        camera.viewport = source.viewport;
        camera.sub_camera_view = source.sub_camera_view;
        camera.is_active = true;
        camera.invert_culling = source.invert_culling;
    }
    world.entity_mut(outline.camera).insert((
        transform,
        projection,
        RenderTarget::Image(ImageRenderTarget {
            handle: outline.mask.clone(),
            scale_factor: target.scale_factor,
        }),
    ));
    outline.scale = target.scale_factor;
    outline.visible = true;
    width(world, outline, WIDTH * outline.scale);
}

fn in_hierarchy(
    world: &World,
    mut entity: Entity,
    mut accepts: impl FnMut(Entity) -> bool,
) -> bool {
    loop {
        if accepts(entity) {
            return true;
        }
        let Some(parent) = world.get::<ChildOf>(entity) else {
            return false;
        };
        entity = parent.parent();
    }
}

struct Snapshot {
    source: Entity,
    mesh: Mesh3d,
    global: GlobalTransform,
    inherited: InheritedVisibility,
    skin: Option<SkinnedMesh>,
    morph: Option<MeshMorphWeights>,
}

fn clear(world: &mut World, outline: &mut Outline) {
    for (_, proxy) in outline.proxies.drain() {
        if let Ok(entity) = world.get_entity_mut(proxy) {
            entity.despawn();
        }
    }
    hide(world, outline);
}

/// Run after TransformSystems::Propagate and VisibilityPropagate, before
/// CheckVisibility. Exact globals preserve non-uniform parent scale and shear.
pub(super) fn sync(world: &mut World) {
    let Some(mut outline) = world.remove_resource::<Outline>() else {
        return;
    };
    sync_inner(world, &mut outline);
    world.insert_resource(outline);
}

fn sync_inner(world: &mut World, outline: &mut Outline) {
    let Some(selected) = selected(world).filter(|_| outline.visible) else {
        clear(world, outline);
        return;
    };
    let camera = world.query_filtered::<(&Camera, &GlobalTransform, Option<&RenderLayers>), (With<EditorCamera>, Without<MaskCamera>)>()
        .iter(world).next().map(|(camera, global, layers)| (camera.is_active, *global, layers.cloned().unwrap_or_default()));
    let Some((true, camera_global, layers)) = camera else {
        clear(world, outline);
        return;
    };
    if let Some(mut global) = world.get_mut::<GlobalTransform>(outline.camera) {
        *global = camera_global;
    }
    let Some(scene) = world.get_resource::<Scene>() else {
        clear(world, outline);
        return;
    };
    let roots: HashSet<_> = scene
        .root
        .into_iter()
        .chain(scene.models.values().map(|(root, _)| *root))
        .collect();
    let mut query = world.query_filtered::<(
        Entity,
        &Mesh3d,
        &GlobalTransform,
        Option<&InheritedVisibility>,
        Option<&Visibility>,
        Option<&RenderLayers>,
        Option<&SkinnedMesh>,
        Option<&MeshMorphWeights>,
    ), Without<Proxy>>();
    let sources: Vec<_> = query
        .iter(world)
        .filter_map(
            |(source, mesh, global, inherited, visibility, source_layers, skin, morph)| {
                // Only the selected hierarchy contributes to this independent
                // mask. Other scene meshes must never erase its hidden edges.
                if !in_hierarchy(world, source, |entity| entity == selected)
                    || !in_hierarchy(world, source, |entity| roots.contains(&entity))
                    || !layers.intersects(source_layers.unwrap_or(&RenderLayers::default()))
                {
                    return None;
                }
                Some(Snapshot {
                    source,
                    mesh: mesh.clone(),
                    global: *global,
                    inherited: inherited.copied().unwrap_or(
                        if matches!(visibility, Some(Visibility::Hidden)) {
                            InheritedVisibility::HIDDEN
                        } else {
                            InheritedVisibility::VISIBLE
                        },
                    ),
                    skin: skin.cloned(),
                    morph: morph.cloned(),
                })
            },
        )
        .collect();
    if !sources.iter().any(|source| source.inherited.get()) {
        clear(world, outline);
        return;
    }
    let wanted: HashSet<_> = sources.iter().map(|source| source.source).collect();
    outline.proxies.retain(|source, proxy| {
        if wanted.contains(source) && world.entities().contains(*proxy) {
            return true;
        }
        if let Ok(entity) = world.get_entity_mut(*proxy) {
            entity.despawn();
        }
        false
    });
    for source in sources {
        let color = outline.white.clone();
        let proxy = *outline.proxies.entry(source.source).or_insert_with(|| {
            world
                .spawn((
                    Proxy,
                    Mesh3d(source.mesh.0.clone()),
                    MeshMaterial3d(color.clone()),
                    Transform::IDENTITY,
                    source.global,
                    Visibility::Visible,
                    source.inherited,
                    ViewVisibility::default(),
                    RenderLayers::layer(LAYER),
                    NoFrustumCulling,
                    NotShadowCaster,
                    NotShadowReceiver,
                    Pickable::IGNORE,
                ))
                .id()
        });
        if source.skin.is_some() || source.morph.is_some() {
            world.entity_mut(proxy).insert(NoAutomaticBatching);
        }
        world.entity_mut(proxy).insert((
            source.mesh,
            MeshMaterial3d(color),
            source.global,
            source.inherited,
            if source.inherited.get() {
                Visibility::Visible
            } else {
                Visibility::Hidden
            },
        ));
        if let Some(skin) = source.skin {
            world.entity_mut(proxy).insert(skin);
        } else {
            world.entity_mut(proxy).remove::<SkinnedMesh>();
        }
        if let Some(morph) = source.morph {
            world.entity_mut(proxy).insert(morph);
        } else {
            world.entity_mut(proxy).remove::<MeshMorphWeights>();
        }
    }
    outline.visible = true;
    width(world, outline, WIDTH * outline.scale);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data::{LevelEdits, Pose},
        project::EditorProject,
    };
    use bevy::{
        camera::Viewport,
        transform::{TransformPlugin, TransformSystems},
    };

    fn app() -> (App, Entity) {
        let project = EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"),
        )
        .unwrap();
        let mut editor = Editor::new(project, LevelEdits::default());
        editor.ready = true;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(editor)
            .init_resource::<Scene>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<OutlineMaterial>>()
            .init_resource::<Assets<OutlineMaskMaterial>>()
            .add_systems(Startup, setup)
            .add_systems(Update, sync_camera)
            .add_systems(PostUpdate, sync.after(TransformSystems::Propagate));
        let image = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::new_target_texture(
                320,
                240,
                TextureFormat::Rgba8Unorm,
                None,
            ));
        let camera = app
            .world_mut()
            .spawn((
                EditorCamera,
                Camera3d::default(),
                Camera {
                    viewport: Some(Viewport {
                        physical_position: UVec2::new(8, 12),
                        physical_size: UVec2::new(200, 128),
                        depth: 0.2..0.8,
                    }),
                    ..default()
                },
                RenderTarget::Image(ImageRenderTarget {
                    handle: image,
                    scale_factor: 1.5,
                }),
                Transform::from_xyz(2.0, 3.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
                RenderLayers::layer(0),
            ))
            .id();
        (app, camera)
    }

    fn mesh(world: &mut World, parent: Option<Entity>, transform: Transform) -> Entity {
        let entity = world
            .spawn((
                Mesh3d::default(),
                transform,
                Visibility::Visible,
                InheritedVisibility::VISIBLE,
            ))
            .id();
        if let Some(parent) = parent {
            world.entity_mut(entity).insert(ChildOf(parent));
        }
        entity
    }

    fn placed(world: &mut World, id: u32, entity: Entity) {
        world
            .resource_mut::<Scene>()
            .models
            .insert(id, (entity, format!("model{id}.glb")));
    }

    fn select(world: &mut World, selection: Selection) {
        world.resource_mut::<Editor>().selection = Some(selection);
    }

    #[test]
    fn imported_hierarchy_copies_exact_sheared_globals_and_camera_without_touching_sources() {
        let (mut app, camera) = app();
        let scene = app
            .world_mut()
            .spawn(Transform::from_scale(Vec3::new(2.0, 1.0, 3.0)))
            .id();
        let selected = mesh(
            app.world_mut(),
            Some(scene),
            Transform::from_rotation(Quat::from_rotation_y(0.6)),
        );
        let child = mesh(
            app.world_mut(),
            Some(selected),
            Transform::from_xyz(1.0, 0.0, 0.0),
        );
        let occluder = mesh(
            app.world_mut(),
            Some(scene),
            Transform::from_xyz(0.0, 0.0, 1.0),
        );
        let thumbnail = mesh(app.world_mut(), None, Transform::IDENTITY);
        app.world_mut()
            .entity_mut(thumbnail)
            .insert(RenderLayers::layer(2));
        let preview = mesh(app.world_mut(), None, Transform::IDENTITY);
        let original = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::from(Color::srgb(0.0, 0.0, 1.0)));
        app.world_mut()
            .entity_mut(selected)
            .insert(MeshMaterial3d(original.clone()));
        app.world_mut().resource_mut::<Scene>().root = Some(scene);
        app.world_mut()
            .resource_mut::<Editor>()
            .imported
            .push(crate::editor::Imported {
                entity: selected,
                name: "Selected".into(),
                original: Pose::from(Transform::IDENTITY),
                parent: GlobalTransform::IDENTITY,
                local: Transform::IDENTITY,
            });
        select(app.world_mut(), Selection::Imported(0));
        app.update();
        let world = app.world();
        let outline = world.resource::<Outline>();
        assert!(outline.visible);
        assert_eq!(outline.proxies.len(), 2);
        assert!(!outline.proxies.contains_key(&occluder));
        assert!(!outline.proxies.contains_key(&thumbnail));
        assert!(!outline.proxies.contains_key(&preview));
        let affine = world.get::<GlobalTransform>(selected).unwrap().affine();
        assert!(
            affine.matrix3.x_axis.dot(affine.matrix3.z_axis).abs() > 0.1,
            "fixture must include parent-induced shear"
        );
        for source in [selected, child] {
            let proxy = outline.proxies[&source];
            assert_eq!(
                world.get::<GlobalTransform>(proxy).unwrap().affine(),
                world.get::<GlobalTransform>(source).unwrap().affine()
            );
            assert!(world.get::<ChildOf>(proxy).is_none());
            assert!(world.get::<NotShadowCaster>(proxy).is_some());
            assert!(world.get::<NotShadowReceiver>(proxy).is_some());
            assert_eq!(
                *world.get::<RenderLayers>(proxy).unwrap(),
                RenderLayers::layer(LAYER)
            );
            assert_eq!(
                &world
                    .get::<MeshMaterial3d<OutlineMaskMaterial>>(proxy)
                    .unwrap()
                    .0,
                &outline.white
            );
        }
        assert_eq!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(selected)
                .unwrap()
                .0,
            original
        );
        assert_eq!(world.get::<ChildOf>(child).unwrap().parent(), selected);
        assert_eq!(
            world
                .get::<GlobalTransform>(outline.camera)
                .unwrap()
                .affine(),
            world.get::<GlobalTransform>(camera).unwrap().affine()
        );
        let mask = world
            .resource::<Assets<Image>>()
            .get(&outline.mask)
            .unwrap();
        assert_eq!(
            (
                mask.texture_descriptor.size.width,
                mask.texture_descriptor.size.height
            ),
            (320, 240)
        );
        let source_viewport = world
            .get::<Camera>(camera)
            .unwrap()
            .viewport
            .as_ref()
            .unwrap();
        let mask_viewport = world
            .get::<Camera>(outline.camera)
            .unwrap()
            .viewport
            .as_ref()
            .unwrap();
        assert_eq!(
            source_viewport.physical_position,
            mask_viewport.physical_position
        );
        assert_eq!(source_viewport.physical_size, mask_viewport.physical_size);
        assert_eq!(source_viewport.depth, mask_viewport.depth);
        assert_eq!(outline.scale, 1.5);
    }

    #[test]
    fn selection_switching_async_children_and_deletion_leave_no_stale_proxies() {
        let (mut app, _) = app();
        let first = mesh(app.world_mut(), None, Transform::IDENTITY);
        let second = mesh(app.world_mut(), None, Transform::from_xyz(2.0, 0.0, 0.0));
        placed(app.world_mut(), 1, first);
        placed(app.world_mut(), 2, second);
        select(app.world_mut(), Selection::Model(1));
        app.update();
        let child = mesh(
            app.world_mut(),
            Some(first),
            Transform::from_xyz(0.0, 1.0, 0.0),
        );
        app.update();
        assert!(
            app.world()
                .resource::<Outline>()
                .proxies
                .contains_key(&child)
        );
        select(app.world_mut(), Selection::Model(2));
        app.update();
        let outline = app.world().resource::<Outline>();
        assert_eq!(outline.proxies.len(), 1);
        assert!(!outline.proxies.contains_key(&first));
        assert!(!outline.proxies.contains_key(&child));
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<OutlineMaskMaterial>>(outline.proxies[&second])
                .unwrap()
                .0,
            outline.white
        );
        app.world_mut().entity_mut(first).despawn();
        app.update();
        assert_eq!(app.world().resource::<Outline>().proxies.len(), 1);
        app.world_mut().entity_mut(second).despawn();
        app.update();
        assert!(app.world().resource::<Outline>().proxies.is_empty());
        assert!(!app.world().resource::<Outline>().visible);
    }

    #[test]
    fn hidden_meshes_layers_skinning_and_closed_scenes_are_respected() {
        let (mut app, _) = app();
        let selected = mesh(app.world_mut(), None, Transform::IDENTITY);
        let transparent = mesh(app.world_mut(), None, Transform::IDENTITY);
        let hidden_layer = mesh(app.world_mut(), None, Transform::IDENTITY);
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..default()
            });
        app.world_mut()
            .entity_mut(transparent)
            .insert(MeshMaterial3d(material));
        app.world_mut()
            .entity_mut(hidden_layer)
            .insert((RenderLayers::layer(9), ChildOf(selected)));
        let joint = app.world_mut().spawn(Transform::IDENTITY).id();
        app.world_mut().entity_mut(selected).insert((
            SkinnedMesh {
                joints: vec![joint],
                ..default()
            },
            MeshMorphWeights::Value {
                weights: vec![0.25, 0.75],
            },
        ));
        for (id, entity) in [(1, selected), (2, transparent)] {
            placed(app.world_mut(), id, entity);
        }
        select(app.world_mut(), Selection::Model(1));
        app.update();
        let outline = app.world().resource::<Outline>();
        assert_eq!(outline.proxies.len(), 1);
        let proxy = outline.proxies[&selected];
        assert_eq!(
            app.world().get::<SkinnedMesh>(proxy).unwrap().joints,
            vec![joint]
        );
        assert!(
            matches!(app.world().get::<MeshMorphWeights>(proxy),Some(MeshMorphWeights::Value {weights}) if weights == &[0.25,0.75])
        );
        app.world_mut()
            .entity_mut(selected)
            .insert(InheritedVisibility::HIDDEN);
        app.update();
        assert!(!app.world().resource::<Outline>().visible);
        app.world_mut()
            .entity_mut(selected)
            .insert(InheritedVisibility::VISIBLE);
        app.update();
        assert!(app.world().resource::<Outline>().visible);
        app.world_mut().resource_mut::<Editor>().pilot = true;
        app.update();
        assert!(!app.world().resource::<Outline>().visible);
        app.world_mut().resource_mut::<Editor>().pilot = false;
        app.update();
        assert!(app.world().resource::<Outline>().visible);
        app.world_mut().resource_mut::<Editor>().close_document(0);
        app.update();
        assert!(app.world().resource::<Outline>().proxies.is_empty());
        assert!(!app.world().resource::<Outline>().visible);
    }
}
