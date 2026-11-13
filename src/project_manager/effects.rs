//! Manager motion and lightweight, cursor-reactive UI materials.
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    picking::hover::Hovered,
    prelude::*,
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
    ui::{UiGlobalTransform, UiTransform, Val2},
    window::PrimaryWindow,
};

use super::{Manager, ui};
use crate::ui_theme::{ACCENT, ACTIVE, FOCUS, PANEL, gradient};

const RIPPLE_END: f32 = 0.85;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(super) struct ActionMaterial {
    #[uniform(0)]
    state: Vec4,
    #[uniform(1)]
    cursor: Vec4,
    #[uniform(2)]
    details: Vec4,
}

impl UiMaterial for ActionMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("action.wesl")).with_source("embedded"),
        )
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(super) struct AmbientMaterial {
    #[uniform(0)]
    cursor: Vec4,
}

impl UiMaterial for AmbientMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("ambience.wesl")).with_source("embedded"),
        )
    }
}

#[derive(Resource)]
pub(super) struct Ambience(Handle<AmbientMaterial>);

impl FromWorld for Ambience {
    fn from_world(world: &mut World) -> Self {
        Self(
            world
                .resource_mut::<Assets<AmbientMaterial>>()
                .add(AmbientMaterial {
                    cursor: Vec4::new(0.5, 0.4, 0.0, 0.0),
                }),
        )
    }
}

#[derive(Component)]
pub(super) struct AmbientCanvas;

#[derive(Component)]
#[require(UiTransform)]
pub(super) struct Motion {
    primary: bool,
    card: bool,
    pub selected: bool,
    pub focused: bool,
    pub disabled: bool,
    pub color: Color,
    pub border: Color,
    hover: f32,
    press: f32,
    selection: f32,
    age: f32,
    entered: f32,
    delay: f32,
    pointer: Vec2,
    origin: Vec2,
    was_pressed: bool,
    activated: bool,
}

impl Motion {
    pub fn button(primary: bool) -> Self {
        Self {
            primary,
            card: false,
            selected: false,
            focused: false,
            disabled: false,
            color: Color::NONE,
            border: Color::NONE,
            hover: 0.0,
            press: 0.0,
            selection: 0.0,
            age: RIPPLE_END,
            entered: 1.0,
            delay: 0.0,
            pointer: Vec2::splat(0.5),
            origin: Vec2::splat(0.5),
            was_pressed: false,
            activated: false,
        }
    }

    pub fn card(index: usize) -> Self {
        Self {
            card: true,
            entered: 0.0,
            delay: (index as f32 * 0.028).min(0.16),
            ..Self::button(false)
        }
    }

    pub fn activate(&mut self) {
        self.activated = true;
    }
}

#[derive(Component)]
#[require(UiTransform)]
pub(super) struct Reveal {
    elapsed: f32,
    distance: f32,
    scale: f32,
}

impl Reveal {
    pub fn panel() -> Self {
        Self {
            elapsed: 0.0,
            distance: 10.0,
            scale: 1.0,
        }
    }

    pub fn dialog() -> Self {
        Self {
            elapsed: 0.0,
            distance: 16.0,
            scale: 0.965,
        }
    }
}

#[derive(Component, Default)]
pub(super) struct Backdrop(f32);

#[derive(Component)]
#[require(UiTransform)]
pub(super) struct Orbit;

/// Move the artwork while keeping the button's hit target stationary.
#[derive(Component)]
#[require(UiTransform)]
pub(super) struct Content(pub Entity);

pub(super) struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "action.wesl");
        embedded_asset!(app, "ambience.wesl");
        app.add_plugins((
            UiMaterialPlugin::<ActionMaterial>::default(),
            UiMaterialPlugin::<AmbientMaterial>::default(),
        ))
        .init_resource::<Ambience>()
        .add_systems(
            Update,
            (attach, animate, reveal, contents, ambience)
                .chain()
                .after(ui::styles),
        );
    }
}

pub(super) fn background(commands: &mut Commands, parent: Entity, ambience: &Ambience) {
    commands.spawn((
        AmbientCanvas,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        MaterialNode(ambience.0.clone()),
        ZIndex(-2),
        Pickable::IGNORE,
        ChildOf(parent),
    ));
}

fn follow(value: f32, target: f32, delta: f32, rate: f32) -> f32 {
    let next = value.lerp(target, 1.0 - (-delta * rate).exp());
    if (next - target).abs() < 0.0005 {
        target
    } else {
        next
    }
}

fn ease_out(progress: f32) -> f32 {
    1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(3)
}

fn attach(
    mut commands: Commands,
    mut materials: ResMut<Assets<ActionMaterial>>,
    controls: Query<Entity, (With<Motion>, Without<MaterialNode<ActionMaterial>>)>,
) {
    for entity in &controls {
        commands
            .entity(entity)
            .try_insert(MaterialNode(materials.add(ActionMaterial {
                state: Vec4::ZERO,
                cursor: Vec4::splat(0.5),
                details: Vec4::new(RIPPLE_END, 0.0, 0.0, 0.0),
            })));
    }
}

fn animate(
    time: Res<Time>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut materials: ResMut<Assets<ActionMaterial>>,
    mut controls: Query<(
        &mut Motion,
        &Hovered,
        Has<bevy::ui::Pressed>,
        &ComputedNode,
        &UiGlobalTransform,
        &MaterialNode<ActionMaterial>,
        &mut UiTransform,
        &mut BackgroundColor,
        &mut BackgroundGradient,
        &mut BorderColor,
    )>,
) {
    let delta = time.delta_secs().min(0.05);
    for (
        mut motion,
        hovered,
        pressed,
        computed,
        global,
        material,
        mut transform,
        mut color,
        mut fill,
        mut border,
    ) in &mut controls
    {
        let interactive = !motion.disabled && window.focused && computed.size().min_element() > 0.0;
        let hovering = interactive && hovered.0;
        let pressing = interactive && pressed;
        if hovering && let Some(cursor) = window.cursor_position() {
            let local = global
                .affine()
                .inverse()
                .transform_point2(cursor * window.scale_factor());
            let uv = local / computed.size() + Vec2::splat(0.5);
            if uv.is_finite() {
                motion.pointer = uv.clamp(Vec2::ZERO, Vec2::ONE);
            }
        }
        let activated = std::mem::take(&mut motion.activated);
        if interactive && ((pressing && !motion.was_pressed) || (activated && !motion.was_pressed))
        {
            motion.age = 0.0;
            motion.origin = if hovering {
                motion.pointer
            } else {
                Vec2::splat(0.5)
            };
        } else {
            motion.age = (motion.age + delta).min(RIPPLE_END);
        }
        if !interactive {
            motion.age = RIPPLE_END;
        }
        motion.was_pressed = pressing;
        motion.hover = follow(motion.hover, if hovering { 1.0 } else { 0.0 }, delta, 14.0);
        let press_target = if pressing {
            1.0
        } else if motion.age < 0.14 {
            0.45
        } else {
            0.0
        };
        motion.press = follow(
            motion.press,
            if interactive { press_target } else { 0.0 },
            delta,
            25.0,
        );
        motion.selection = follow(
            motion.selection,
            if motion.selected && !motion.disabled {
                1.0
            } else {
                0.0
            },
            delta,
            12.0,
        );
        motion.entered = (motion.entered + delta).min(1.0);
        let entrance = ease_out((motion.entered - motion.delay) / 0.24);
        transform.set_if_neq(UiTransform {
            translation: Val2::px(0.0, (1.0 - entrance) * 12.0),
            scale: Vec2::splat(1.0 - (1.0 - entrance) * 0.012),
            ..default()
        });
        let blend = 1.0 - (-delta * 16.0).exp();
        let next_color = color.0.mix(&motion.color, blend);
        let next_border = border.top.mix(&motion.border, blend);
        color.set_if_neq(BackgroundColor(next_color));
        border.set_if_neq(BorderColor::all(next_border));
        fill.set_if_neq(if motion.primary && !motion.disabled {
            gradient(
                LinearGradient::TO_BOTTOM_RIGHT,
                [
                    FOCUS.mix(&Color::srgb(0.985, 0.873, 0.660), motion.hover),
                    ACCENT,
                    Color::srgb(0.700, 0.535, 0.330),
                ],
            )
        } else if motion.selection > 0.0 {
            let alpha = motion.selection * (1.0 - motion.hover * 0.4);
            gradient(
                LinearGradient::TO_BOTTOM_RIGHT,
                [
                    ACTIVE.with_alpha(alpha),
                    Color::srgb(0.125, 0.116, 0.137).with_alpha(alpha),
                    PANEL.with_alpha(alpha),
                ],
            )
        } else {
            BackgroundGradient::default()
        });
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.state = Vec4::new(
                motion.hover,
                motion.press,
                motion.selection,
                if motion.disabled { 0.0 } else { 1.0 },
            );
            material.cursor = Vec4::new(
                motion.pointer.x,
                motion.pointer.y,
                motion.origin.x,
                motion.origin.y,
            );
            material.details = Vec4::new(
                motion.age,
                if motion.primary { 1.0 } else { 0.0 },
                if interactive && motion.focused {
                    1.0
                } else {
                    0.0
                },
                if motion.card { 1.0 } else { 0.0 },
            );
        }
    }
}

fn reveal(
    time: Res<Time>,
    mut reveals: Query<(&mut Reveal, &mut UiTransform), Without<Motion>>,
    mut backdrops: Query<(&mut Backdrop, &mut BackgroundColor)>,
    mut orbits: Query<&mut UiTransform, (With<Orbit>, Without<Reveal>, Without<Motion>)>,
) {
    let delta = time.delta_secs().min(0.05);
    for (mut reveal, mut transform) in &mut reveals {
        reveal.elapsed = (reveal.elapsed + delta).min(0.28);
        let amount = ease_out(reveal.elapsed / 0.28);
        transform.set_if_neq(UiTransform {
            translation: Val2::px(0.0, reveal.distance * (1.0 - amount)),
            scale: Vec2::splat(reveal.scale.lerp(1.0, amount)),
            ..default()
        });
    }
    for (mut backdrop, mut color) in &mut backdrops {
        backdrop.0 = (backdrop.0 + delta).min(0.18);
        color.set_if_neq(BackgroundColor(Color::srgba(
            0.0,
            0.0,
            0.0,
            0.65 * ease_out(backdrop.0 / 0.18),
        )));
    }
    for mut transform in &mut orbits {
        let phase = time.elapsed_secs() * 0.55;
        transform.set_if_neq(UiTransform {
            translation: Val2::px(0.0, phase.sin() * 3.0),
            rotation: Rot2::degrees((phase * 0.6).sin() * 1.4),
            ..default()
        });
    }
}

fn contents(
    owners: Query<&Motion>,
    mut content: Query<(&Content, &mut UiTransform), Without<Motion>>,
) {
    for (owner, mut transform) in &mut content {
        let Ok(motion) = owners.get(owner.0) else {
            continue;
        };
        transform.set_if_neq(UiTransform {
            translation: Val2::px(0.0, -motion.hover * 0.8 + motion.press * 1.4),
            scale: Vec2::splat(1.0 + motion.hover * 0.012 - motion.press * 0.035),
            ..default()
        });
    }
}

fn ambience(
    time: Res<Time>,
    window: Single<&Window, With<PrimaryWindow>>,
    manager: Res<Manager>,
    ambience: Res<Ambience>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<AmbientCanvas>>,
    mut materials: ResMut<Assets<AmbientMaterial>>,
) {
    let Some(mut material) = materials.get_mut(&ambience.0) else {
        return;
    };
    let delta = time.delta_secs().min(0.05);
    let mut target = Vec2::new(0.5, 0.4);
    let mut near = 0.0;
    if window.focused
        && let Some(cursor) = window.cursor_position()
    {
        for (computed, transform) in &canvas {
            if computed.size().min_element() <= 0.0 {
                continue;
            }
            let local = transform
                .affine()
                .inverse()
                .transform_point2(cursor * window.scale_factor());
            let uv = local / computed.size() + Vec2::splat(0.5);
            if uv.is_finite() && uv.cmpge(Vec2::ZERO).all() && uv.cmple(Vec2::ONE).all() {
                target = uv;
                near = 1.0;
            }
        }
    }
    let target = Vec4::new(
        target.x,
        target.y,
        near,
        if manager.form.is_some() { 1.0 } else { 0.0 },
    );
    material.cursor = material.cursor.lerp(target, 1.0 - (-delta * 5.0).exp());
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{time::TimeUpdateStrategy, ui::Pressed};
    use std::time::Duration;

    #[test]
    fn hover_and_keyboard_activation_animate_without_moving_the_hit_target_or_leaking_materials() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                20,
            )))
            .init_resource::<Assets<ActionMaterial>>()
            .add_systems(Update, (attach, animate, contents).chain());
        app.world_mut().spawn((
            PrimaryWindow,
            Window {
                focused: true,
                ..default()
            },
        ));
        let control = app
            .world_mut()
            .spawn((
                Node::default(),
                Motion::button(false),
                Hovered::default(),
                BackgroundColor::default(),
                BackgroundGradient::default(),
                BorderColor::default(),
            ))
            .id();
        app.world_mut()
            .get_mut::<ComputedNode>(control)
            .unwrap()
            .size = Vec2::new(140.0, 32.0);
        let label = app
            .world_mut()
            .spawn((Node::default(), Content(control), ChildOf(control)))
            .id();
        app.update();
        app.world_mut().entity_mut(control).insert(Hovered(true));
        app.update();
        let motion = app.world().get::<Motion>(control).unwrap();
        assert!(motion.hover > 0.0 && motion.hover < 1.0);
        for _ in 0..30 {
            app.update();
        }
        assert!(app.world().get::<Motion>(control).unwrap().hover > 0.99);
        assert_eq!(
            *app.world().get::<UiTransform>(control).unwrap(),
            UiTransform::IDENTITY
        );
        assert_ne!(
            *app.world().get::<UiTransform>(label).unwrap(),
            UiTransform::IDENTITY
        );
        app.world_mut().entity_mut(control).insert(Pressed);
        app.update();
        assert!(app.world().get::<Motion>(control).unwrap().press > 0.0);
        app.world_mut().entity_mut(control).remove::<Pressed>();
        app.world_mut().entity_mut(control).insert(Hovered(false));
        app.update();
        app.world_mut()
            .get_mut::<Motion>(control)
            .unwrap()
            .activate();
        app.update();
        let handle = &app
            .world()
            .get::<MaterialNode<ActionMaterial>>(control)
            .unwrap()
            .0;
        let material = app
            .world()
            .resource::<Assets<ActionMaterial>>()
            .get(handle)
            .unwrap();
        assert_eq!(
            material.details.x, 0.0,
            "Keyboard activation must start a centered ripple"
        );
        assert_eq!(
            Vec2::new(material.cursor.z, material.cursor.w),
            Vec2::splat(0.5)
        );
        app.world_mut().get_mut::<Motion>(control).unwrap().disabled = true;
        app.update();
        let handle = &app
            .world()
            .get::<MaterialNode<ActionMaterial>>(control)
            .unwrap()
            .0;
        let materials = app.world().resource::<Assets<ActionMaterial>>();
        assert_eq!(materials.get(handle).unwrap().state.w, 0.0);
        assert_eq!(materials.len(), 1);
    }

    #[test]
    fn dialog_motion_settles_at_its_layout_at_different_frame_rates() {
        for milliseconds in [10, 20, 50] {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                    milliseconds,
                )))
                .add_systems(Update, reveal);
            let card = app
                .world_mut()
                .spawn((Node::default(), Reveal::dialog()))
                .id();
            let backdrop = app
                .world_mut()
                .spawn((
                    Node::default(),
                    Backdrop::default(),
                    BackgroundColor::default(),
                ))
                .id();
            app.update();
            app.update();
            let transform = app.world().get::<UiTransform>(card).unwrap();
            assert!(transform.scale.x < 1.0 && transform.scale.x > 0.96);
            for _ in 0..(400 / milliseconds) {
                app.update();
            }
            assert_eq!(
                *app.world().get::<UiTransform>(card).unwrap(),
                UiTransform::IDENTITY
            );
            assert!(
                (app.world()
                    .get::<BackgroundColor>(backdrop)
                    .unwrap()
                    .0
                    .alpha()
                    - 0.65)
                    .abs()
                    < 0.001
            );
        }
    }
}
