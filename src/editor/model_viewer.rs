//! Isolated model documents. Their meshes, camera and lights never enter a scene.
use super::*;
use bevy::{
    camera::{ClearColorConfig, RenderTarget, primitives::Aabb, visibility::RenderLayers},
    gltf::{Gltf, GltfLoaderSettings},
    light::RectLight,
    render::render_resource::{Extent3d, TextureFormat},
    world_serialization::WorldInstanceReady,
};

const LAYER: usize = 8;
const FOV: f32 = crate::editor::viewport::EDITOR_FOV;

#[path = "model_environment.rs"]
mod environment;
#[path = "model_navigation.rs"]
mod navigation;
#[path = "model_paint.rs"]
mod paint;

#[path = "model_edit.rs"]
pub(in crate::editor) mod editing;

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum ModelViewAction {
    Close,
    Fit,
    Retry,
    Environment(environment::EnvironmentAction),
    Edit(editing::EditAction),
    Uv(editing::uv::UvAction),
    Paint(paint::PaintAction),
}

#[derive(Component)]
pub(in crate::editor) struct Canvas;
#[derive(Component)]
struct ViewCamera;
#[derive(Component)]
struct Prepared;

#[derive(Component)]
struct ModelGrid;
#[derive(Component)]
struct ModelBackground;

#[derive(Clone, Copy)]
struct SavedView {
    center: Vec3,
    distance: f32,
    yaw: f32,
    pitch: f32,
    orthographic: bool,
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
    uv_mode: bool,
    center: Vec3,
    radius: f32,
    distance: f32,
    yaw: f32,
    pitch: f32,
    orthographic: bool,
    gesture: Option<(navigation::Navigation, Vec2)>,
    return_focus: Option<Entity>,
    poses: std::collections::HashMap<usize, SavedView>,
    restore: Option<SavedView>,
}

#[derive(Resource)]
struct ViewerUi {
    overlay: Entity,
    status: Entity,
    image: Handle<Image>,
    toolbar: Entity,
    hint: Entity,
    content: Entity,
}

pub(in crate::editor) struct ModelViewerPlugin;
impl Plugin for ModelViewerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Viewer>()
            .add_plugins(editing::selection::SelectionPlugin)
            .init_resource::<editing::State>()
            .init_resource::<paint::PaintState>()
            .init_resource::<paint::engine::Runtime>()
            .add_plugins(paint::engine::gpu::PluginGpu)
            .add_plugins(editing::uv::UvPlugin)
            .init_resource::<editing::xray::Materials>()
            .init_resource::<editing::wireframe::Materials>()
            .init_resource::<environment::Settings>()
            .add_systems(Startup, setup.after(super::setup))
            .add_systems(Startup, environment::setup.after(super::setup))
            .add_systems(Startup, paint::setup.after(setup).after(environment::setup))
            .add_systems(
                Update,
                paint::actions.after(actions).before(editing::input::input),
            )
            .add_systems(
                Update,
                paint::refresh
                    .after(editing::controls::groups)
                    .after(environment::refresh),
            )
            .add_systems(Update, (paint::engine::update, paint::engine::input, paint::engine::cursor).chain().after(paint::refresh))
            .add_systems(
                Startup,
                editing::controls::setup
                    .after(setup)
                    .before(environment::setup),
            )
            .init_gizmo_group::<editing::input::ModelGizmos>()
            .init_gizmo_group::<editing::input::XrayGizmos>()
            .add_systems(Startup, editing::input::setup_gizmos)
            .add_systems(
                Update,
                editing::controls::fields
                    .after(crate::editor::ui::scrub_numbers)
                    .before(actions)
                    .before(crate::editor::close_editor),
            )
            .add_systems(
                Update,
                (
                    actions,
                    environment::actions,
                    load,
                    navigate,
                    editing::initialize,
                    editing::input::input,
                    editing::render,
                    editing::wireframe::sync,
                    editing::xray::sync,
                    editing::selection::sync,
                    environment::synchronize,
                    navigation::background,
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
        app.add_systems(
            Update,
            (editing::controls::refresh, editing::controls::groups)
                .chain()
                .after(crate::editor::ui::refresh),
        );
        app.add_systems(
            PostUpdate,
            editing::input::draw.after(bevy::transform::TransformSystems::Propagate),
        );
        app.add_systems(
            PostUpdate,
            editing::handles::sync.after(bevy::transform::TransformSystems::Propagate),
        );
    }
}

fn setup(
    mut commands: Commands,
    interface: Res<Interface>,
    mut images: ResMut<Assets<Image>>,
    mut handles: ResMut<Assets<crate::editor::viewport::HandleMaterial>>,
) {
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
    let panel = node(
        &mut commands,
        overlay,
        Node {
            width: percent(100),
            height: percent(100),
            ..column(0.0)
        },
    );
    let toolbar = node(
        &mut commands,
        panel,
        Node {
            position_type: PositionType::Absolute,
            left: px(10),
            right: px(140),
            top: px(10),
            min_height: px(34),
            flex_wrap: FlexWrap::Wrap,
            justify_content: JustifyContent::Start,
            align_items: AlignItems::Start,
            ..row(5.0)
        },
    );
    let content = node(
        &mut commands,
        panel,
        Node {
            width: percent(100),
            height: percent(100),
            min_height: px(0),
            flex_grow: 1.0,
            ..row(0.0)
        },
    );
    let canvas = node(
        &mut commands,
        content,
        Node {
            height: percent(100),
            min_width: px(0),
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
    let handle_overlay =
        crate::editor::viewport::spawn_overlay(&mut commands, &mut handles, canvas);
    commands
        .entity(handle_overlay)
        .insert(crate::editor::viewport::transform_handles::ModelOverlay);
    let status = text(&mut commands, panel, &interface.font, "", 14.0, TEXT);
    commands
        .entity(status)
        .insert(AccessibleLabel("Выделение модели".into()));
    let hint = text(
        &mut commands,
        panel,
        &interface.font,
        "ЛКМ — выбор · ПКМ + WASD / Q E — полёт · Alt+ЛКМ — орбита · СКМ — панорама · G/R/S — трансформации",
        12.0,
        MUTED,
    );
    for (entity, bottom) in [(status, 32.0), (hint, 10.0)] {
        commands.entity(entity).insert((
            ChildOf(canvas),
            Node {
                position_type: PositionType::Absolute,
                left: px(10),
                bottom: px(bottom),
                padding: UiRect::axes(px(6), px(3)),
                border_radius: BorderRadius::all(px(4)),
                ..default()
            },
            BackgroundColor(CHROME.with_alpha(0.85)),
            ZIndex(20),
        ));
    }
    commands.insert_resource(ViewerUi {
        overlay,
        status,
        image,
        toolbar,
        hint,
        content,
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
    mut editing: ResMut<editing::State>,
    mut previews: ResMut<crate::editor::thumbnails::Thumbnails>,
    keys: Res<ButtonInput<KeyCode>>,
    mut grid_assets: (
        ResMut<Assets<Mesh>>,
        ResMut<Assets<crate::editor::viewport_grid::GridMaterial>>,
        ResMut<Assets<crate::editor::viewport_background::BackgroundMaterial>>,
    ),
) {
    let incoming: Vec<_> = messages.read().copied().collect();
    editing::process_actions(
        &incoming,
        &mut editor,
        &mut viewer,
        &mut editing,
        &server,
        &mut previews,
        keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
    );
    let mut retry = false;
    for action in incoming {
        match action {
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
                editing::request_close(&mut editor, index);
                if !editor.model_tabs.contains(&index) {
                    viewer.poses.remove(&index);
                }
            }
            Action::ModelView(action)
                if editor.viewing_model.is_some() && editor.pending.is_none() =>
            {
                match action {
                    ModelViewAction::Close => {
                        let index = editor.viewing_model.unwrap();
                        editing::request_close(&mut editor, index);
                        if !editor.model_tabs.contains(&index) {
                            viewer.poses.remove(&index);
                        }
                    }
                    ModelViewAction::Fit => {
                        viewer.fitted = false;
                        viewer.restore = None;
                    }
                    ModelViewAction::Retry => {
                        let index = editor.viewing_model.unwrap();
                        if editor.dirty_models.contains(&index) {
                            editing.status = "Сначала сохраните или отмените правки модели".into();
                        } else {
                            editing.documents.remove(&index);
                            editing.errors.remove(&index);
                            retry = true;
                        }
                    }
                    ModelViewAction::Environment(_) => {}
                    ModelViewAction::Edit(editing::EditAction::Mode(_)) => {
                        focus.clear();
                    }
                    ModelViewAction::Edit(_) => {}
                    ModelViewAction::Uv(_) => {}
                    ModelViewAction::Paint(_) => {}
                }
            }
            _ => {}
        }
    }
    if editing.last_model != editor.viewing_model {
        editing.clear_gesture(None, false);
        editing.last_model = editor.viewing_model;
        editing.extrude_tool = false;
        editing.menu = false;
        editing.add_menu = false;
        editing.box_start = None;
        editing.box_armed = false;
        editing.knife = false;
        editing.knife_start = None;
        editing.bvh_stamp = None;
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
            orthographic: viewer.orthographic,
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
                settings.load_meshes = bevy::asset::RenderAssetUsages::default();
            })
            .load(model.asset.clone()),
    );
    viewer.started = Some(std::time::Instant::now());
    let stage = commands
        .spawn((Transform::IDENTITY, Visibility::Inherited))
        .id();
    viewer.stage = Some(stage);
    let grid = crate::editor::viewport_grid::spawn(
        &mut commands,
        &mut grid_assets.0,
        &mut grid_assets.1,
        LAYER,
    );
    commands.entity(grid).insert((ModelGrid, ChildOf(stage)));
    let background = crate::editor::viewport_background::spawn(
        &mut commands,
        &mut grid_assets.0,
        &mut grid_assets.2,
        LAYER,
    );
    commands
        .entity(background)
        .insert((ModelBackground, ChildOf(stage)));
    let camera = commands
        .spawn((
            ViewCamera,
            crate::hdri::OrthographicBackdrop(true),
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
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<Canvas>>,
    children: Query<&Children>,
    meshes: Query<(&Aabb, &GlobalTransform), With<Mesh3d>>,
    prepared: Query<(), With<Prepared>>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<(&mut Transform, &mut Projection), (With<ViewCamera>, Without<ModelGrid>)>,
    editing: Res<editing::State>,
    runtime: (
        Res<Time>,
        Res<InputFocus>,
        Query<(), With<bevy::ui_widgets::TextInput>>,
        Res<bevy::input::mouse::AccumulatedMouseMotion>,
        Res<DrawerMotion>,
        Option<ResMut<super::navigation_gizmo::State>>,
    ),
    mut grids: Query<(&mut Transform, &mut Visibility), (With<ModelGrid>, Without<ViewCamera>)>,
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
    let pixels = pixels.round().max(Vec2::ONE).as_uvec2();
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
        viewer.distance = fit_distance(viewer.radius, size.x / size.y).max(viewer.radius * 7.0);
        if let Some(pose) = viewer.restore.take() {
            viewer.center = pose.center;
            viewer.distance = pose.distance;
            viewer.yaw = pose.yaw;
            viewer.pitch = pose.pitch;
            viewer.orthographic = pose.orthographic;
        }
        viewer.fitted = true;
        viewer.error = None;
    }
    let uv_mode = editing.mode(editor.viewing_model) == crate::editor::model_document::Mode::Uv;
    if uv_mode && !viewer.uv_mode {
        // Fit the actual silhouette into the final split pane, including its depth.
        let aspect = (size.x * 0.51 / size.y).max(0.1);
        let rotation = view_pose(&viewer).rotation.inverse();
        let tangent = (FOV * 0.5).tan();
        let mut distance = viewer.radius * 0.1;
        if let Some(doc) = editor.viewing_model.and_then(|i| editing.documents.get(&i)) {
            for object in doc.objects.iter().filter(|o| !o.hidden) {
                for p in &object.geometry.positions {
                    let local = rotation * (object.transform.transform_point(*p) - viewer.center);
                    distance = distance
                        .max(local.z + (local.x.abs() / aspect).max(local.y.abs()) / tangent * 1.2);
                }
            }
        }
        viewer.distance = distance.max(viewer.radius * 0.5);
    }
    viewer.uv_mode = uv_mode;
    let typing = runtime
        .1
        .get()
        .is_some_and(|entity| runtime.2.contains(entity));
    let blocked = typing
        || editing.gesture.is_some()
        || editing.menu
        || editing.add_menu
        || editing.box_armed
        || editing.knife
        || editor.sidebar.resizing()
        || runtime.4.resizing
        || window
            .cursor_position()
            .is_some_and(|point| runtime.4.covers(point, &window));
    let mut runtime = runtime;
    let gizmo_blocked = runtime.5.as_deref_mut().is_some_and(|gizmo| {
        let mut frame = super::navigation_gizmo::Frame {
            workspace: super::navigation_gizmo::Workspace::Model(editor.viewing_model.unwrap()),
            view: view_pose(&viewer),
            pivot: viewer.center,
            rect,
            orthographic: viewer.orthographic,
        };
        let blocked = gizmo.drive(&mut frame, &window, &mouse, &keys, !blocked);
        if blocked {
            viewer.gesture = None;
            viewer.orthographic = frame.orthographic;
            let back = *frame.view.back();
            viewer.yaw = if back.xz().length_squared() < 0.000001 {
                0.0
            } else {
                back.x.atan2(back.z)
            };
            viewer.pitch = back.y.atan2(back.xz().length());
        }
        blocked
    });
    navigation::drive(
        &mut viewer,
        &window,
        &mouse,
        &keys,
        &scroll,
        runtime.0.delta_secs(),
        runtime.3.delta,
        rect,
        blocked || gizmo_blocked,
    );
    for (mut grid, mut visibility) in &mut grids {
        grid.translation = crate::editor::viewport_grid::recenter(&view_pose(&viewer));
        *visibility = if viewer.orthographic {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
    if let Ok((mut transform, mut projection)) = cameras.single_mut() {
        *transform = view_pose(&viewer);
        let mut next = viewport::free_projection(viewer.orthographic, viewer.distance);
        let near = (viewer.radius * 0.001).max(0.000001);
        let far = (viewer.distance + viewer.radius * 100.0).max(10000.0);
        match &mut next {
            Projection::Perspective(p) => {
                p.near = near;
                p.far = far;
            }
            Projection::Orthographic(p) => {
                let depth = orthographic_depth(&viewer);
                p.near = -depth;
                p.far = depth;
            }
            _ => {}
        }
        *projection = next;
    }
}

fn view_pose(viewer: &Viewer) -> Transform {
    let direction = Vec3::new(
        viewer.yaw.sin() * viewer.pitch.cos(),
        viewer.pitch.sin(),
        viewer.yaw.cos() * viewer.pitch.cos(),
    );
    Transform::from_translation(viewer.center + direction * viewer.distance).with_rotation(
        Quat::from_euler(EulerRot::YXZ, viewer.yaw, -viewer.pitch, 0.0),
    )
}

fn view_height(viewer: &Viewer, depth: f32) -> f32 {
    let depth = if viewer.orthographic {
        viewer.distance
    } else {
        depth
    };
    2.0 * depth.max(0.000001) * (FOV * 0.5).tan()
}

// Orthographic magnification is independent of eye distance. Keep the depth
// slab on both sides of the eye, including when zoom puts it inside a model.
fn orthographic_depth(viewer: &Viewer) -> f32 {
    (viewer.radius * 100.0)
        .max(viewer.distance * 2.0)
        .max(0.000001)
}

fn refresh(
    editor: Res<Editor>,
    viewer: Res<Viewer>,
    ui: Res<ViewerUi>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    editing: Res<editing::State>,
) {
    if let Ok(mut overlay) = nodes.get_mut(ui.overlay) {
        overlay.height = Val::Auto;
        overlay.top = px(0);
        overlay.bottom = px(0);
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
                    if !editing.status.is_empty() {
                        return editing.selection_label(editor.viewing_model).map_or_else(
                            || editing.status.clone(),
                            |selected| format!("{selected} · {}", editing.status),
                        );
                    }
                    if viewer.fitted {
                        editing
                            .selection_label(editor.viewing_model)
                            .unwrap_or_else(|| "Модель загружена".into())
                    } else {
                        "Загрузка модели…".into()
                    }
                });
        }
        if let Ok(mut hint) = labels.get_mut(ui.hint) {
            hint.0 = if editing.mode(editor.viewing_model)
                == crate::editor::model_document::Mode::Paint
            {
                "ПКМ + WASD — полёт · Alt+ЛКМ — орбита · СКМ — панорама · Tab — объект".into()
            } else if editing.mode(editor.viewing_model) == crate::editor::model_document::Mode::Uv
            {
                "Alt+ЛКМ — орбита · Ctrl+S — сохранить".into()
            } else if let Some(gesture) = &editing.gesture {
                if gesture.is_extruding() {
                    "Двигайте курсор, чтобы задать расстояние · Ctrl — привязка к шагу".into()
                } else {
                    format!(
                        "{:?} {} {} · X/Y/Z — ось · число — значение · Ctrl — привязка · Enter — применить · Esc — отмена",
                        gesture.tool,
                        gesture.axis.map_or("", |i| ["X", "Y", "Z"][i]),
                        gesture.typed
                    )
                }
            } else if editing.knife {
                "Нож: отметьте две точки в 3D-виде · Esc — отмена".into()
            } else if editing.box_armed || editing.box_start.is_some() {
                "Рамка: протяните ЛКМ · Shift — добавить · Ctrl — исключить · Esc — отмена".into()
            } else if editing.extrude_tool {
                "Потяните «+» на выделении · E — начать вытягивание · Esc — выйти из инструмента".into()
            } else {
                "ЛКМ — выбор / рамка · Shift — добавить · Ctrl — исключить · ПКМ + WASD / Q E — полёт · Alt+ЛКМ — орбита · G/R/S — трансформации".into()
            };
        }
    }
}

#[cfg(test)]
#[path = "model_viewer_tests.rs"]
mod tests;
