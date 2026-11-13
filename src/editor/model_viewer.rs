//! Isolated, read-only model inspection. Its camera and lights never enter a scene.
use super::*;
use bevy::{
    camera::{ClearColorConfig, RenderTarget, primitives::Aabb, visibility::RenderLayers},
    gltf::{Gltf, GltfLoaderSettings},
    light::RectLight,
    render::render_resource::{Extent3d, TextureFormat},
    world_serialization::WorldInstanceReady,
};

const LAYER: usize = 8;
const FOV: f32 = std::f32::consts::FRAC_PI_4;

#[path = "model_environment.rs"]
mod environment;

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum ModelViewAction {
    Close,
    Fit,
    Retry,
    Environment(environment::EnvironmentAction),
}

#[derive(Component)]
struct Canvas;
#[derive(Component)]
struct ViewCamera;
#[derive(Component)]
struct Prepared;

#[derive(Clone, Copy)]
struct SavedView {
    center: Vec3,
    distance: f32,
    yaw: f32,
    pitch: f32,
}

#[derive(Resource, Default)]
struct Viewer {
    model: Option<usize>,
    stage: Option<Entity>,
    root: Option<Entity>,
    source: Option<Handle<Gltf>>,
    started: Option<std::time::Instant>,
    error: Option<String>,
    fitted: bool,
    center: Vec3,
    radius: f32,
    distance: f32,
    yaw: f32,
    pitch: f32,
    gesture: Option<(bool, Vec2)>,
    return_focus: Option<Entity>,
    poses: std::collections::HashMap<usize, SavedView>,
    restore: Option<SavedView>,
}

#[derive(Resource)]
struct ViewerUi {
    overlay: Entity,
    status: Entity,
    image: Handle<Image>,
}

pub(in crate::editor) struct ModelViewerPlugin;
impl Plugin for ModelViewerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Viewer>()
            .init_resource::<environment::Settings>()
            .add_systems(Startup, setup.after(super::setup))
            .add_systems(Startup, environment::setup.after(super::setup))
            .add_systems(
                Update,
                (
                    actions,
                    environment::actions,
                    load,
                    navigate,
                    environment::synchronize,
                    refresh,
                )
                    .chain()
                    .after(super::view_images)
                    .before(crate::editor::asset_placement::update),
            );
        app.add_systems(
            Update,
            environment::refresh.after(crate::editor::ui::refresh),
        );
    }
}

fn setup(mut commands: Commands, interface: Res<Interface>, mut images: ResMut<Assets<Image>>) {
    let image = images.add(Image::new_target_texture(
        640,
        480,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let overlay = commands
        .spawn((
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            ZIndex(6),
            ChildOf(interface.viewport),
            AccessibleLabel("Просмотр 3D-модели".into()),
        ))
        .id();
    let panel = surface(
        &mut commands,
        overlay,
        Node {
            width: percent(100),
            height: percent(100),
            padding: UiRect::all(px(14)),
            border_radius: BorderRadius::all(px(10)),
            ..column(10.0)
        },
        PANEL,
    );
    let canvas = node(
        &mut commands,
        panel,
        Node {
            width: percent(100),
            flex_grow: 1.0,
            min_height: px(0),
            overflow: Overflow::clip(),
            ..default()
        },
    );
    commands
        .entity(canvas)
        .insert((Canvas, AccessibleLabel("3D-вид модели".into())));
    commands.spawn((
        ImageNode::new(image.clone()),
        Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        ChildOf(canvas),
    ));
    let status = text(&mut commands, panel, &interface.font, "", 12.0, MUTED);
    text(
        &mut commands,
        panel,
        &interface.font,
        "ЛКМ / ПКМ — вращение · СКМ — панорама · Колесо — приближение · F — весь объект · Esc — закрыть",
        12.0,
        MUTED,
    );
    commands.insert_resource(ViewerUi {
        overlay,
        status,
        image,
    });
}

fn actions(
    mut messages: MessageReader<Action>,
    mut editor: ResMut<Editor>,
    mut viewer: ResMut<Viewer>,
    ui: Res<ViewerUi>,
    server: Res<AssetServer>,
    mut commands: Commands,
    mut focus: ResMut<InputFocus>,
    controls: Query<(Entity, &Control)>,
) {
    let mut retry = false;
    for action in messages.read() {
        match *action {
            Action::ViewModel(index)
                if editor.pending.is_none()
                    && !editor.browser_popup
                    && index < editor.project.catalog.len() =>
            {
                viewer.return_focus = controls.iter().find_map(|(entity, control)| {
                    matches!(control.action, Action::ViewModel(model) if model == index)
                        .then_some(entity)
                });
                editor.open_model_tab(index);
                focus.clear();
            }
            Action::CloseModelTab(index) if editor.pending.is_none() => {
                editor.close_model_tab(index);
                viewer.poses.remove(&index);
            }
            Action::ModelView(action)
                if editor.viewing_model.is_some() && editor.pending.is_none() =>
            {
                match action {
                    ModelViewAction::Close => {
                        let index = editor.viewing_model.unwrap();
                        editor.close_model_tab(index);
                        viewer.poses.remove(&index);
                    }
                    ModelViewAction::Fit => {
                        viewer.fitted = false;
                        viewer.restore = None;
                    }
                    ModelViewAction::Retry => retry = true,
                    ModelViewAction::Environment(_) => {}
                }
            }
            _ => {}
        }
    }
    if editor.viewing_model == viewer.model && !retry {
        return;
    }
    if let Some(stage) = viewer.stage.take() {
        commands.entity(stage).try_despawn();
    }
    if let Some(index) = viewer
        .model
        .filter(|index| editor.model_tabs.contains(index))
        && viewer.fitted
        && !retry
    {
        let pose = SavedView {
            center: viewer.center,
            distance: viewer.distance,
            yaw: viewer.yaw,
            pitch: viewer.pitch,
        };
        viewer.poses.insert(index, pose);
    }
    let poses = std::mem::take(&mut viewer.poses);
    let restore = editor
        .viewing_model
        .and_then(|index| poses.get(&index).copied())
        .filter(|_| !retry);
    let return_focus = viewer.return_focus;
    *viewer = Viewer {
        model: editor.viewing_model,
        return_focus,
        poses,
        restore,
        yaw: 0.65,
        pitch: 0.35,
        ..default()
    };
    let Some(index) = editor.viewing_model else {
        if let Some(entity) = return_focus.filter(|entity| controls.contains(*entity)) {
            focus.set(entity, bevy::input_focus::FocusCause::Pressed);
        } else {
            focus.clear();
        }
        return;
    };
    let Some(model) = editor.project.catalog.get(index) else {
        editor.viewing_model = None;
        return;
    };
    if retry {
        server.reload(model.asset.clone());
    }
    viewer.source = Some(
        server
            .load_builder()
            .with_settings(|settings: &mut GltfLoaderSettings| {
                settings.load_cameras = false;
                settings.load_lights = false;
                settings.load_animations = false;
            })
            .load(model.asset.clone()),
    );
    viewer.started = Some(std::time::Instant::now());
    let stage = commands
        .spawn((Transform::IDENTITY, Visibility::Inherited))
        .id();
    viewer.stage = Some(stage);
    let camera = commands
        .spawn((
            ViewCamera,
            Camera3d::default(),
            bevy::camera::Hdr,
            Camera {
                order: -2,
                clear_color: ClearColorConfig::Custom(Color::linear_rgb(0.05, 0.05, 0.055)),
                ..default()
            },
            RenderTarget::Image(ui.image.clone().into()),
            RenderLayers::layer(LAYER),
            Transform::from_xyz(3.0, 2.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
            Projection::Perspective(PerspectiveProjection {
                fov: FOV,
                ..default()
            }),
            AmbientLight {
                brightness: 0.0,
                ..default()
            },
            bevy::camera::Exposure::BLENDER,
            bevy::render::view::Tonemapping::AcesFitted,
            Msaa::Off,
            ChildOf(stage),
        ))
        .id();
    commands
        .entity(camera)
        .insert(bevy::anti_alias::fxaa::Fxaa::default());
    commands.spawn((
        environment::KeyLight,
        DirectionalLight::default(),
        Transform::IDENTITY,
        RenderLayers::layer(LAYER),
        ChildOf(stage),
    ));
}

fn prepare(
    ready: On<WorldInstanceReady>,
    children: Query<&Children>,
    mut cameras: Query<&mut Camera>,
    mut commands: Commands,
) {
    for entity in std::iter::once(ready.entity).chain(children.iter_descendants(ready.entity)) {
        commands
            .entity(entity)
            .insert(RenderLayers::layer(LAYER))
            .remove::<(PointLight, SpotLight, DirectionalLight, RectLight)>();
        if let Ok(mut camera) = cameras.get_mut(entity) {
            camera.is_active = false;
        }
    }
    commands.entity(ready.entity).insert(Prepared);
}

fn load(
    mut viewer: ResMut<Viewer>,
    server: Res<AssetServer>,
    assets: Res<Assets<Gltf>>,
    mut commands: Commands,
) {
    let Some(source) = &viewer.source else {
        return;
    };
    if viewer.error.is_some() || viewer.root.is_some() {
        return;
    }
    if let Some(error) = crate::editor::load_error(&server, source) {
        viewer.error = Some(error);
        return;
    }
    if !server.is_loaded_with_dependencies(source.id()) {
        if viewer
            .started
            .is_some_and(|start| start.elapsed().as_secs() > 30)
        {
            viewer.error =
                Some("Загрузка занимает слишком много времени. Нажмите F5 для обновления.".into());
        }
        return;
    }
    let Some(asset) = assets.get(source) else {
        return;
    };
    let Some(world) = asset
        .default_scene
        .as_ref()
        .or_else(|| asset.scenes.first())
    else {
        viewer.error = Some("В GLB нет доступной геометрии.".into());
        return;
    };
    let stage = viewer.stage.unwrap();
    viewer.root = Some(
        commands
            .spawn((WorldAssetRoot(world.clone()), ChildOf(stage)))
            .observe(prepare)
            .id(),
    );
}

fn fit_distance(radius: f32, aspect: f32) -> f32 {
    let half_angle = (FOV * 0.5).min(((FOV * 0.5).tan() * aspect.max(0.01)).atan());
    radius / half_angle.sin() * 1.15
}

fn navigate(
    editor: Res<Editor>,
    mut viewer: ResMut<Viewer>,
    ui: Res<ViewerUi>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<Canvas>>,
    children: Query<&Children>,
    meshes: Query<(&Aabb, &GlobalTransform), With<Mesh3d>>,
    prepared: Query<(), With<Prepared>>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<ViewCamera>>,
) {
    if editor.viewing_model.is_none()
        || editor.pending.is_some()
        || editor.viewing_image.is_some()
        || editor.browser_popup
        || editor.hdri_library_open
    {
        viewer.gesture = None;
        return;
    }
    let Ok((computed, transform)) = canvas.single() else {
        return;
    };
    let size = computed.size() * computed.inverse_scale_factor();
    if size.min_element() < 1.0 {
        return;
    }
    let center = transform.affine().translation * computed.inverse_scale_factor();
    let rect = Rect::from_center_size(center, size);
    let pixels = size * window.scale_factor();
    let pixels = (pixels * (2048.0 / pixels.max_element()).min(1.0))
        .round()
        .max(Vec2::ONE)
        .as_uvec2();
    if images.get(&ui.image).is_some_and(|image| {
        image.texture_descriptor.size.width != pixels.x
            || image.texture_descriptor.size.height != pixels.y
    }) {
        if let Some(mut image) = images.get_mut(&ui.image) {
            image.resize(Extent3d {
                width: pixels.x,
                height: pixels.y,
                depth_or_array_layers: 1,
            });
            viewer.fitted = false;
        }
    }
    let Some(root) = viewer.root.filter(|root| prepared.contains(*root)) else {
        return;
    };
    if !viewer.fitted {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for entity in std::iter::once(root).chain(children.iter_descendants(root)) {
            if let Ok((aabb, world)) = meshes.get(entity) {
                let affine = world.affine();
                let center = affine.transform_point3(aabb.center.into());
                let half = Vec3::from(affine.matrix3.abs() * aabb.half_extents);
                min = min.min(center - half);
                max = max.max(center + half);
            }
        }
        if !min.is_finite() || !max.is_finite() || !(max - min).length().is_finite() {
            viewer.error = Some("В модели нет видимой геометрии.".into());
            return;
        }
        viewer.center = (min + max) * 0.5;
        viewer.radius = ((max - min).length() * 0.5).max(0.0001);
        viewer.distance = fit_distance(viewer.radius, size.x / size.y);
        if let Some(pose) = viewer.restore.take() {
            viewer.center = pose.center;
            viewer.distance = pose.distance;
            viewer.yaw = pose.yaw;
            viewer.pitch = pose.pitch;
        }
        viewer.fitted = true;
        viewer.error = None;
    }
    if !window.focused {
        viewer.gesture = None;
    }
    if let Some(cursor) = window.cursor_position().filter(|_| window.focused) {
        if rect.contains(cursor) {
            if mouse.just_pressed(MouseButton::Middle) {
                viewer.gesture = Some((true, cursor));
            } else if mouse.any_just_pressed([MouseButton::Left, MouseButton::Right]) {
                viewer.gesture = Some((false, cursor));
            }
            if scroll.delta != Vec2::ZERO {
                let delta = if scroll.unit == MouseScrollUnit::Pixel {
                    scroll.delta.y / 32.0
                } else {
                    scroll.delta.y
                };
                viewer.distance = (viewer.distance * (-delta * 0.15).exp())
                    .clamp(viewer.radius * 0.06, viewer.radius * 1000.0);
            }
        }
        if let Some((pan, previous)) = viewer.gesture {
            let pressed = if pan {
                mouse.pressed(MouseButton::Middle)
            } else {
                mouse.any_pressed([MouseButton::Left, MouseButton::Right])
            };
            if pressed {
                let delta = cursor - previous;
                if pan {
                    let pose = view_pose(&viewer);
                    let scale = viewer.distance * 2.0 * (FOV * 0.5).tan() / size.y;
                    viewer.center += (*pose.right() * -delta.x + *pose.up() * delta.y) * scale;
                } else {
                    viewer.yaw -= delta.x * 0.008;
                    viewer.pitch = (viewer.pitch + delta.y * 0.008).clamp(-1.5, 1.5);
                }
                viewer.gesture = Some((pan, cursor));
            } else {
                viewer.gesture = None;
            }
        }
    }
    if let Ok((mut transform, mut projection)) = cameras.single_mut() {
        *transform = view_pose(&viewer);
        if let Projection::Perspective(perspective) = &mut *projection {
            perspective.near = (viewer.radius * 0.001).max(0.000001);
            perspective.far = viewer.distance + viewer.radius * 100.0;
        }
    }
}

fn view_pose(viewer: &Viewer) -> Transform {
    let direction = Vec3::new(
        viewer.yaw.sin() * viewer.pitch.cos(),
        viewer.pitch.sin(),
        viewer.yaw.cos() * viewer.pitch.cos(),
    );
    Transform::from_translation(viewer.center + direction * viewer.distance)
        .looking_at(viewer.center, Vec3::Y)
}

fn refresh(
    editor: Res<Editor>,
    drawer: Res<DrawerMotion>,
    viewer: Res<Viewer>,
    ui: Res<ViewerUi>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
) {
    if let Ok(mut overlay) = nodes.get_mut(ui.overlay) {
        overlay.height = Val::Auto;
        overlay.top = px(0);
        overlay.bottom = px(drawer.visible_height);
        overlay.display = if editor.viewing_model.is_some() && editor.pending.is_none() {
            Display::Flex
        } else {
            Display::None
        };
    }
    if editor
        .viewing_model
        .and_then(|index| editor.project.catalog.get(index))
        .is_some()
    {
        if let Ok(mut status) = labels.get_mut(ui.status) {
            status.0 = viewer
                .error
                .as_ref()
                .map(|error| format!("Не удалось показать модель: {error}"))
                .unwrap_or_else(|| {
                    if viewer.fitted {
                        "Модель загружена".into()
                    } else {
                        "Загрузка модели…".into()
                    }
                });
        }
    }
}

#[cfg(test)]
#[path = "model_viewer_tests.rs"]
mod tests;
