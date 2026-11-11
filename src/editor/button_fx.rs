//! Tactile silk highlights without changing button geometry or input behavior.
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    input_focus::InputFocusVisible,
    prelude::*,
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

use super::*;

const RIPPLE_END: f32 = 1.2;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct ButtonMaterial {
    #[uniform(0)]
    state: Vec4,
    #[uniform(1)]
    pointer: Vec4,
}

impl UiMaterial for ButtonMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("button_fx.wesl")).with_source("embedded"),
        )
    }
}

#[derive(Component)]
pub(super) struct Effect {
    material: Handle<ButtonMaterial>,
    hover: f32,
    press: f32,
    age: f32,
    pointer: Vec2,
    origin: Vec2,
    strength: f32,
    was_pressed: bool,
    activated: bool,
    keyboard_pulse: bool,
    attached: bool,
}

impl Effect {
    /// Called only after the ordinary control activation has been accepted.
    pub(super) fn activate(&mut self) {
        self.activated = true;
    }
}

pub(in crate::editor) struct ButtonFxPlugin;

impl Plugin for ButtonFxPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "button_fx.wesl");
        app.add_plugins(UiMaterialPlugin::<ButtonMaterial>::default())
            .add_systems(
                Update,
                (attach, animate)
                    .chain()
                    .after(super::refresh)
                    .after(super::refresh_lighting)
                    .after(super::refresh_browser),
            );
    }
}

fn attach(
    mut commands: Commands,
    mut materials: ResMut<Assets<ButtonMaterial>>,
    controls: Query<(Entity, &Control), Without<Effect>>,
) {
    for (entity, control) in &controls {
        let strength = match control.look {
            Look::Primary => 0.6,
            Look::Asset | Look::Outline => 0.75,
            Look::Tool | Look::Tab => 0.85,
            _ => 1.0,
        };
        // The handle lives with the control, so Bevy frees its material when
        // a browser row or scene tab is rebuilt or removed.
        commands.entity(entity).try_insert(Effect {
            material: materials.add(ButtonMaterial {
                state: Vec4::new(0.0, 0.0, RIPPLE_END, strength),
                pointer: Vec4::splat(0.5),
            }),
            hover: 0.0,
            press: 0.0,
            age: RIPPLE_END,
            pointer: Vec2::splat(0.5),
            origin: Vec2::splat(0.5),
            strength,
            was_pressed: false,
            activated: false,
            keyboard_pulse: false,
            attached: false,
        });
    }
}

fn animate(
    mut commands: Commands,
    time: Res<Time>,
    window: Single<&Window, With<PrimaryWindow>>,
    focus: Res<InputFocus>,
    focus_visible: Res<InputFocusVisible>,
    drag: Option<Res<AssetDrag>>,
    mut materials: ResMut<Assets<ButtonMaterial>>,
    mut controls: Query<(
        Entity,
        &Hovered,
        Has<Pressed>,
        Has<InteractionDisabled>,
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
        &mut Effect,
    )>,
) {
    let delta = time.delta_secs().min(0.05);
    let dragging = drag.as_ref().is_some_and(|drag| drag.active());
    let smooth = |value: f32, target: f32, rate: f32| {
        let next = value.lerp(target, 1.0 - (-delta * rate).exp());
        if (next - target).abs() < 0.001 {
            target
        } else {
            next
        }
    };
    for (entity, hovered, pressed, disabled, computed, transform, visibility, mut effect) in
        &mut controls
    {
        let enabled = !disabled
            && !dragging
            && window.focused
            && visibility.get()
            && computed.size().is_finite()
            && computed.size().min_element() > 0.0;
        let activated = std::mem::take(&mut effect.activated);
        if !enabled {
            effect.hover = 0.0;
            effect.press = 0.0;
            effect.age = RIPPLE_END;
            effect.was_pressed = false;
            effect.keyboard_pulse = false;
        } else {
            let pointer_hovered = hovered.0;
            if pointer_hovered && let Some(cursor) = window.cursor_position() {
                let local = transform
                    .affine()
                    .inverse()
                    .transform_point2(cursor * window.scale_factor());
                let uv = local / computed.size() + Vec2::splat(0.5);
                if uv.is_finite() {
                    effect.pointer = uv.clamp(Vec2::ZERO, Vec2::ONE);
                }
            }
            let started = pressed && !effect.was_pressed;
            if started || (activated && !effect.was_pressed) {
                effect.age = 0.0;
                effect.origin = if pointer_hovered {
                    effect.pointer
                } else {
                    Vec2::splat(0.5)
                };
                effect.keyboard_pulse = !pressed;
            } else {
                effect.age = (effect.age + delta).min(RIPPLE_END);
            }
            let focused = focus_visible.0 && focus.get() == Some(entity);
            effect.hover = smooth(
                effect.hover,
                if pointer_hovered {
                    1.0
                } else if focused {
                    0.5
                } else {
                    0.0
                },
                14.0,
            );
            let press_target = if pressed {
                1.0
            } else if effect.keyboard_pulse && effect.age < 0.18 {
                0.8 * (1.0 - effect.age / 0.18)
            } else {
                0.0
            };
            effect.press = smooth(effect.press, press_target, 26.0);
            effect.was_pressed = pressed;
        }
        let active = effect.hover > 0.0 || effect.press > 0.0 || effect.age < RIPPLE_END;
        if active != effect.attached {
            if active {
                commands
                    .entity(entity)
                    .try_insert(MaterialNode(effect.material.clone()));
            } else {
                commands
                    .entity(entity)
                    .try_remove::<MaterialNode<ButtonMaterial>>();
            }
            effect.attached = active;
        }
        if !active {
            continue;
        }
        let state = Vec4::new(effect.hover, effect.press, effect.age, effect.strength);
        let pointer = Vec4::new(
            effect.pointer.x,
            effect.pointer.y,
            effect.origin.x,
            effect.origin.y,
        );
        if materials
            .get(&effect.material)
            .is_some_and(|material| material.state != state || material.pointer != pointer)
            && let Some(mut material) = materials.get_mut(&effect.material)
        {
            material.state = state;
            material.pointer = pointer;
        }
    }
}
