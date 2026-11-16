//! Lift, luminous drop slots and a short landing confirmation. No input geometry changes.
use super::*;
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub(in crate::editor) struct DragMaterial {
    #[uniform(0)]
    pub state: Vec4,
    #[uniform(1)]
    pub tint: Vec4,
}
impl UiMaterial for DragMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("image_layer_drag_fx.wesl"))
                .with_source("embedded"),
        )
    }
}

pub(in crate::editor) struct LayerDragFxPlugin;
impl Plugin for LayerDragFxPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "image_layer_drag_fx.wesl");
        app.add_plugins(UiMaterialPlugin::<DragMaterial>::default())
            .init_resource::<Motion>()
            .add_systems(Update, animate.after(super::visuals));
    }
}

#[derive(Resource, Default)]
struct Motion {
    active: bool,
    age: f32,
    cursor: Option<Vec2>,
    tilt: f32,
    target: Option<(u64, Place)>,
    target_age: f32,
    landing: u64,
    landing_age: f32,
    landing_rect: Rect,
    pulse: Option<Entity>,
    handles: HashMap<Entity, Handle<DragMaterial>>,
}

/// Analytic critically damped motion. It settles without a row bouncing across its neighbour.
pub(super) fn spring(value: f32, velocity: f32, goal: f32, dt: f32) -> (f32, f32) {
    let dt = dt.clamp(0.0, 0.05);
    let omega = 36.0;
    let error = value - goal;
    let term = (velocity + omega * error) * dt;
    let decay = (-omega * dt).exp();
    let next = goal + (error + term) * decay;
    let speed = (velocity - omega * term) * decay;
    if (next - goal).abs() < 0.12 && speed.abs() < 1.0 {
        (goal, 0.0)
    } else {
        (next, speed)
    }
}

fn animate(
    mut commands: Commands,
    time: Res<Time>,
    drag: Res<LayerDrag>,
    overlay: Res<Overlay>,
    mut motion: ResMut<Motion>,
    mut materials: ResMut<Assets<DragMaterial>>,
    mut nodes: Query<&mut Node>,
    mut transforms: Query<&mut UiTransform>,
    window: Single<&Window, With<PrimaryWindow>>,
    editor: Res<Editor>,
    viewer: Res<ImageViewer>,
) {
    let dt = time.delta_secs().min(0.05);
    let active = drag.active && drag.gesture.is_some();
    if !active
        && !motion.active
        && drag.landing_serial == motion.landing
        && (motion.landing == 0 || motion.landing_age >= 0.32)
    {
        return;
    }
    let fresh = active && !motion.active;
    if fresh {
        motion.age = 0.0;
        motion.landing_age = 1.0;
        motion.cursor = Some(drag.cursor);
        motion.tilt = 0.0;
    }
    if active {
        motion.age += dt;
    }
    let key = drag.target.as_ref().map(|t| (t.id, t.place));
    if key != motion.target {
        motion.target = key;
        motion.target_age = 0.0;
    }
    motion.target_age += dt;
    let velocity = if active {
        motion
            .cursor
            .map_or(Vec2::ZERO, |p| (drag.cursor - p) / dt.max(0.001))
    } else {
        Vec2::ZERO
    };
    motion.cursor = active.then_some(drag.cursor);
    let tilt = (-velocity.x * 0.000018).clamp(-0.022, 0.022);
    motion.tilt += (tilt - motion.tilt) * (1.0 - (-dt * 16.0).exp());
    let reveal = (motion.age / 0.18).clamp(0.0, 1.0);
    let pop = 1.0 - (1.0 - reveal).powi(3);
    if let Ok(mut transform) = transforms.get_mut(overlay.root) {
        transform.scale = Vec2::splat(if active { 0.965 + 0.035 * pop } else { 1.0 });
        transform.rotation = Rot2::radians(if active { motion.tilt } else { 0.0 });
    }
    let target_reveal = 1.0 - (-motion.target_age * 30.0).exp();
    for entity in [overlay.gap, overlay.group] {
        if let Ok(mut transform) = transforms.get_mut(entity) {
            transform.scale = Vec2::new(0.985 + target_reveal * 0.015, 0.86 + target_reveal * 0.14);
        }
    }
    let color = if drag.target.as_ref().is_some_and(|t| t.place == Place::Into) {
        GREEN
    } else {
        FOCUS
    };
    let color = if drag.target.as_ref().is_some_and(|t| t.unchanged) {
        MUTED
    } else {
        color
    };
    let linear = color.to_linear();
    for (entity, kind) in [
        (overlay.root, 1.0),
        (overlay.gap, 0.0),
        (overlay.group, 2.0),
        (overlay.line, 3.0),
    ] {
        let Some(node) = nodes.get(entity).ok() else {
            continue;
        };
        if node.display == Display::None {
            continue;
        }
        let handle = motion
            .handles
            .entry(entity)
            .or_insert_with(|| {
                let h = materials.add(DragMaterial {
                    state: Vec4::ZERO,
                    tint: Vec4::ZERO,
                });
                commands.entity(entity).insert((
                    MaterialNode(h.clone()),
                    BackgroundColor(Color::NONE),
                    BorderColor::all(Color::NONE),
                ));
                h
            })
            .clone();
        if let Some(mut mat) = materials.get_mut(&handle) {
            mat.state = Vec4::new(
                motion.age,
                if kind == 1.0 { pop } else { target_reveal },
                (velocity.length() / 1800.0).min(1.0),
                window.scale_factor(),
            );
            mat.tint = Vec4::new(linear.red, linear.green, linear.blue, kind);
        }
        if kind == 3.0 {
            commands.entity(entity).insert(BackgroundColor(Color::NONE));
        }
    }
    if drag.landing_serial != motion.landing {
        motion.landing = drag.landing_serial;
        motion.landing_age = 0.0;
        motion.landing_rect = drag.landing_rect;
    } else {
        motion.landing_age += dt;
    }
    if motion.landing > 0
        && motion.landing_age < 0.32
        && !active
        && viewer.edit.editing(editor.viewing_image)
        && editor.pending.is_none()
    {
        let pulse = if let Some(e) = motion.pulse {
            e
        } else {
            let e = commands
                .spawn((
                    Node {
                        display: Display::None,
                        position_type: PositionType::Absolute,
                        border_radius: BorderRadius::all(px(8)),
                        ..default()
                    },
                    GlobalZIndex(1102),
                    Pickable::IGNORE,
                    AccessibleLabel("Подтверждение переноса слоя".into()),
                    ChildOf(overlay.parent),
                ))
                .id();
            motion.pulse = Some(e);
            let h = materials.add(DragMaterial {
                state: Vec4::ZERO,
                tint: Vec4::ZERO,
            });
            commands.entity(e).insert(MaterialNode(h.clone()));
            motion.handles.insert(e, h);
            e
        };
        let r = motion.landing_rect;
        if let Ok(mut n) = nodes.get_mut(pulse) {
            n.display = Display::Flex;
            n.left = px(r.min.x);
            n.top = px(r.min.y);
            n.width = px(r.width());
            n.height = px(r.height());
        }
        if let Some(handle) = motion.handles.get(&pulse)
            && let Some(mut mat) = materials.get_mut(handle)
        {
            let tint = FOCUS.to_linear();
            mat.state = Vec4::new(motion.landing_age, 1.0, 0.0, window.scale_factor());
            mat.tint = Vec4::new(tint.red, tint.green, tint.blue, 4.0);
        }
    } else if let Some(pulse) = motion.pulse
        && let Ok(mut n) = nodes.get_mut(pulse)
    {
        n.display = Display::None;
    }
    motion.active = active;
}
