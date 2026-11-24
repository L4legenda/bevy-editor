//! Preview polygon-fill borders and the active rectangle or lasso.
use super::*;
use crate::editor::{area_selection, ui::model_viewer::editing::input::ModelGizmos};
use bevy::{ecs::system::SystemParam, math::Rot2, ui::UiTransform};

const LINE_LIMIT: usize = 768;

#[derive(Clone, Copy, PartialEq)]
struct Request {
    key: Key,
    version: GeometryVersion,
    target: surface::FillTarget,
    point: Vec2,
}

#[derive(Resource, Default)]
pub(in crate::editor::ui::model_viewer) struct Preview {
    job: Option<(Request, Task<Vec<surface::OutlineEdge>>)>,
    ready: Option<(Request, Vec<surface::OutlineEdge>)>,
    lines: Vec<Entity>,
    circles: Vec<Entity>,
    points: Vec<Entity>,
}

#[cfg(test)]
impl Preview {
    pub(in crate::editor::ui::model_viewer) fn uv_lines(&self) -> &[Entity] {
        &self.lines
    }

    pub(in crate::editor::ui::model_viewer) fn path_dabs(&self) -> &[Entity] {
        &self.circles
    }

    pub(in crate::editor::ui::model_viewer) fn path_points(&self) -> &[Entity] {
        &self.points
    }
}

#[derive(SystemParam)]
pub(in crate::editor::ui::model_viewer) struct Pointer<'w, 's> {
    window: Single<'w, 's, &'static Window, With<PrimaryWindow>>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    drawer: Res<'w, DrawerMotion>,
    samples: Res<'w, PointerFrame>,
}

pub(in crate::editor::ui::model_viewer) fn draw(
    mut commands: Commands,
    mut preview: ResMut<Preview>,
    mut runtime: ResMut<Runtime>,
    editor: Res<Editor>,
    editing: Res<editing::State>,
    paint: Res<PaintState>,
    viewer: Res<Viewer>,
    ui: Res<ViewerUi>,
    pointer: Pointer,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<Canvas>>,
    texture: UvCanvas,
    controls: Query<(&ComputedNode, &UiGlobalTransform), With<Control>>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    mut nodes: Query<(&mut Node, &mut UiTransform, &mut BackgroundColor)>,
    mut gizmos: Gizmos<ModelGizmos>,
) {
    let path_lines = (|| {
        let model = editor.viewing_model?;
        let doc = editing.documents.get(&model)?;
        let session = paint.sessions.get(&model)?;
        let (object, slot) = session.binding?;
        if doc.mode != Mode::Paint
            || session.tool != PaintTool::Path
            || session.paint_blocked_by_bake()
            || !session.set_appearance.visibility.shown(object, slot)
            || editor.pending.is_some()
            || editor.viewing_image.is_some()
            || editor.browser_popup
            || editor.hdri_library_open
        {
            return None;
        }
        let live = runtime.lives.get(&(model, object, slot))?;
        let views = path_editor::Views {
            model: (session.view != View::Two)
                .then(|| canvas.single().ok().and_then(|(n, t)| node_rect(n, t)))
                .flatten()
                .map(|rect| camera_map(&viewer, rect)),
            uv: (session.view != View::Three)
                .then(|| texture.camera(&viewer))
                .flatten(),
        };
        let cursor = if pointer.window.focused {
            pointer.samples.pen_cursor().or_else(|| pointer.window.cursor_position())
        } else {
            None
        };
        Some(live.path.overlay(views, session, cursor))
    })();
    let hover = (|| {
        let model = editor.viewing_model?;
        let doc = editing.documents.get(&model)?;
        let session = paint.sessions.get(&model)?;
        let (object, slot) = session.binding?;
        let point = pointer
            .samples
            .pen_cursor()
            .or_else(|| pointer.window.cursor_position())?;
        if doc.mode != Mode::Paint
            || session.tool != PaintTool::Fill
            || session.paint_blocked_by_bake()
            || !session.set_appearance.visibility.shown(object, slot)
            || !session.can_paint_layer()
            || !pointer.window.focused
            || editor.pending.is_some()
            || editor.viewing_image.is_some()
            || editor.browser_popup
            || editor.hdri_library_open
            || pointer.drawer.resizing
            || pointer.drawer.covers(point, &pointer.window)
            || pointer
                .mouse
                .any_pressed([MouseButton::Right, MouseButton::Middle])
            || pointer
                .keys
                .any_pressed([KeyCode::AltLeft, KeyCode::AltRight])
            || controls
                .iter()
                .any(|(n, t)| node_rect(n, t).is_some_and(|r| r.contains(point)))
        {
            return None;
        }
        let uv = (session.view != View::Three)
            .then(|| texture.camera(&viewer))
            .flatten()
            .filter(|camera| camera.clip.contains(point));
        let model_view = (session.view != View::Two)
            .then(|| canvas.single().ok().and_then(|(n, t)| node_rect(n, t)))
            .flatten()
            .filter(|rect| rect.contains(point));
        let camera = uv.or_else(|| model_view.map(|rect| camera_map(&viewer, rect)))?;
        Some((
            Request {
                key: (model, object, slot),
                version: (doc.revision, session.set_appearance.visibility.revision),
                target: surface::FillTarget {
                    view: gpu::View {
                        camera,
                        uv: uv.is_some(),
                    },
                    object,
                    slot,
                    mode: session.fill_mode,
                    angle: session.fill_angle,
                },
                point,
            },
            &doc.objects,
            &session.set_appearance.visibility,
            session.fill_erase
                != pointer
                    .keys
                    .any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]),
        ))
    })();
    let drag = runtime
        .gesture
        .as_ref()
        .and_then(|gesture| gesture.fill_drag.as_ref());
    let mut lines = Vec::new();
    let mut circles = Vec::new();
    let mut points = Vec::new();
    let erase = drag.map_or_else(
        || hover.as_ref().is_some_and(|(_, _, _, erase)| *erase),
        |drag| drag.erase,
    );
    let color = if erase {
        Color::srgb(1.0, 0.43, 0.37)
    } else {
        Color::srgb(0.38, 0.88, 1.0)
    };
    if let Some(path) = path_lines {
        preview.job = None;
        preview.ready = None;
        lines = path.lines;
        circles = path.circles;
        points = path.points;
    } else if let Some(region) = drag.and_then(fill_drag::Drag::region) {
        lines.extend(region.edges());
    } else if let Some((request, objects, visibility, _)) = hover {
        if let Some((wanted, job)) = &mut preview.job
            && let Some(result) = block_on(poll_once(job))
        {
            preview.ready = Some((*wanted, result));
            preview.job = None;
        }
        if preview.job.is_none()
            && preview
                .ready
                .as_ref()
                .is_none_or(|(ready, _)| *ready != request)
            && let Some(live) = runtime.lives.get_mut(&request.key)
            && let Some(trace) = live
                .controller
                .trace(request.version.0, visibility, objects, true)
        {
            let job = AsyncComputeTaskPool::get()
                .spawn(async move { trace.fill_outline(request.target, request.point) });
            preview.job = Some((request, job));
        }
        if let Some((ready, edges)) = &preview.ready
            && ready.key == request.key
            && ready.version == request.version
            && ready.target == request.target
            && ready.point.distance(request.point) <= 12.
        {
            if request.target.view.uv {
                let rect = request.target.view.camera.rect;
                lines.extend(
                    edges
                        .iter()
                        .filter_map(|edge| {
                            let [a, b] = edge.uv.map(|uv| rect.min + uv * rect.size());
                            area_selection::clip_segment(request.target.view.camera.clip, a, b)
                        })
                        .take(LINE_LIMIT),
                );
            } else {
                for edge in edges.iter().take(LINE_LIMIT) {
                    gizmos.line(edge.world[0], edge.world[1], color);
                }
            }
        }
    } else {
        preview.job = None;
        preview.ready = None;
    }
    let origin = geometry
        .get(ui.overlay)
        .ok()
        .and_then(|(n, t)| node_rect(n, t))
        .map(|r| r.min);
    if origin.is_none() {
        lines.clear();
        circles.clear();
        points.clear();
    }
    let origin = origin.unwrap_or_default();
    while preview.lines.len() < lines.len() {
        preview.lines.push(
            commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        height: px(2),
                        ..default()
                    },
                    BackgroundColor(color),
                    Pickable::IGNORE,
                    ZIndex(46),
                    ChildOf(ui.overlay),
                ))
                .id(),
        );
    }
    for (index, &entity) in preview.lines.iter().enumerate() {
        let Ok((mut node, mut transform, mut background)) = nodes.get_mut(entity) else {
            continue;
        };
        background.set_if_neq(BackgroundColor(color));
        let Some(&[a, b]) = lines.get(index) else {
            if node.display != Display::None {
                node.display = Display::None;
            }
            continue;
        };
        let delta = b - a;
        let length = delta.length();
        let center = (a + b) * 0.5 - origin;
        let left = px(center.x - length * 0.5);
        let top = px(center.y - 1.);
        if (node.display, node.left, node.top, node.width) != (Display::Flex, left, top, px(length))
        {
            node.display = Display::Flex;
            node.left = left;
            node.top = top;
            node.width = px(length);
        }
        transform.set_if_neq(UiTransform::from_rotation(Rot2::radians(delta.to_angle())));
    }
    while preview.circles.len() < circles.len() {
        preview.circles.push(
            commands
                .spawn((
                    Node::default(),
                    BackgroundColor(Color::NONE),
                    BorderColor::all(Color::srgba(1., 1., 1., 0.62)),
                    Pickable::IGNORE,
                    ZIndex(45),
                    ChildOf(ui.overlay),
                ))
                .id(),
        );
    }
    while preview.points.len() < points.len() {
        preview.points.push(
            commands
                .spawn((
                    Node::default(),
                    BackgroundColor(Color::NONE),
                    BorderColor::all(Color::srgb(0.14, 0.10, 0.10)),
                    Pickable::IGNORE,
                    ZIndex(47),
                    ChildOf(ui.overlay),
                ))
                .id(),
        );
    }
    let dabs = preview.circles.iter().enumerate().map(|(i, &entity)| {
        let style = circles.get(i).map(|&(center, radius)| (center, radius, 1., Color::NONE));
        (entity, style)
    });
    let markers = preview.points.iter().enumerate().map(|(i, &entity)| {
        let style = points.get(i).map(|point| {
            let color = if point.first { Color::WHITE } else { Color::srgb(1., 0.73, 0.38) };
            (point.center, point.radius, 1.5, color)
        });
        (entity, style)
    });
    for (entity, style) in dabs.chain(markers) {
        let Ok((mut node, _, mut background)) = nodes.get_mut(entity) else {
            continue;
        };
        if let Some((center, radius, border, color)) = style {
            let corner = center - origin - Vec2::splat(radius);
            background.set_if_neq(BackgroundColor(color));
            node.set_if_neq(Node {
                position_type: PositionType::Absolute,
                left: px(corner.x),
                top: px(corner.y),
                width: px(radius * 2.),
                height: px(radius * 2.),
                border: UiRect::all(px(border)),
                border_radius: BorderRadius::MAX,
                ..default()
            });
        } else if node.display != Display::None {
            node.display = Display::None;
        }
    }
}
