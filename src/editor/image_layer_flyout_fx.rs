//! A brief entrance for the layer actions flyout; interaction stays on the UI nodes.
use super::*;
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct FlyoutMaterial {
    // Rim sweep progress, opacity, and physical pixels per logical pixel.
    #[uniform(0)]
    motion: Vec4,
    #[uniform(1)]
    panel: Vec4,
    #[uniform(2)]
    accent: Vec4,
}

impl UiMaterial for FlyoutMaterial {
    fn fragment_shader() -> ShaderRef {
        AssetPath::from_path_buf(embedded_path!("image_layer_flyout_fx.wesl"))
            .with_source("embedded")
            .into()
    }
}

pub(in crate::editor) struct LayerFlyoutFxPlugin;

impl Plugin for LayerFlyoutFxPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "image_layer_flyout_fx.wesl");
        app.add_plugins(UiMaterialPlugin::<FlyoutMaterial>::default())
            .add_systems(
                Update,
                (animate_panel, animate_rows)
                    .after(super::refresh)
                    .after(crate::editor::ui::refresh),
            );
    }
}

#[derive(Component)]
struct Transition {
    opened: f64,
    closed: Option<f64>,
}

impl Transition {
    fn age(&self, now: f64) -> f32 {
        (self.closed.unwrap_or(now) - self.opened).max(0.0) as f32
    }

    fn exit(&self, now: f64) -> f32 {
        self.closed.map_or(0.0, |closed| {
            (((now - closed) as f32) / 0.18).clamp(0.0, 1.0)
        })
    }
}

#[derive(Component)]
struct RowTransition(Transition);

pub(super) fn close(
    commands: &mut Commands,
    panel: Entity,
    rows: Option<Entity>,
    opened: f64,
    closed: f64,
) {
    commands
        .entity(panel)
        .remove::<CanvasControl>()
        .insert(Transition {
            opened,
            closed: Some(closed),
        });
    if let Some(rows) = rows {
        commands.entity(rows).insert(RowTransition(Transition {
            opened,
            closed: Some(closed),
        }));
    }
    commands.queue(move |world: &mut World| {
        let mut pending = vec![panel];
        while let Some(entity) = pending.pop() {
            if let Some(children) = world.get::<Children>(entity) {
                pending.extend(children.iter());
            }
            if let Ok(mut entity) = world.get_entity_mut(entity) {
                entity
                    .insert(Pickable::IGNORE)
                    .remove::<(TabIndex, Control, Button, bevy::ui_widgets::TextInput)>();
            }
        }
    });
}

pub(super) fn attach(commands: &mut Commands, panel: Entity, rows: Option<Entity>, started: f64) {
    commands
        .entity(panel)
        .remove::<BackgroundGradient>()
        .insert((
            Transition {
                opened: started,
                closed: None,
            },
            UiTransform::default(),
            BackgroundColor(Color::NONE),
            BorderColor::all(Color::NONE),
            BoxShadow(vec![ShadowStyle {
                color: Color::NONE,
                x_offset: px(-3.0),
                y_offset: px(7.0),
                blur_radius: px(18.0),
                spread_radius: px(1.0),
            }]),
        ));
    if let Some(rows) = rows {
        commands.entity(rows).insert(RowTransition(Transition {
            opened: started,
            closed: None,
        }));
    }
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

fn animate_panel(
    mut commands: Commands,
    time: Res<Time>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut materials: ResMut<Assets<FlyoutMaterial>>,
    mut panels: Query<(
        Entity,
        &Transition,
        &mut UiTransform,
        &mut BoxShadow,
        Option<&MaterialNode<FlyoutMaterial>>,
    )>,
) {
    for (entity, entrance, mut transform, mut shadow, handle) in &mut panels {
        let now = time.elapsed_secs_f64();
        let age = entrance.age(now);
        let exit = entrance.exit(now);
        if exit >= 1.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let travel = ease_out(age / 0.24) * (1.0 - exit * exit);
        let opacity = ease_out(age / 0.17) * (1.0 - exit);
        transform.set_if_neq(UiTransform {
            translation: Val2::px(19.0 * (1.0 - travel), 0.0),
            scale: Vec2::new(0.97, 0.985).lerp(Vec2::ONE, travel),
            ..default()
        });
        shadow.0[0].color = Color::srgba(0.0, 0.0, 0.0, 0.42 * opacity);
        let motion = Vec4::new(
            (age / 0.40).clamp(0.0, 1.0),
            opacity,
            window.scale_factor(),
            0.0,
        );
        if let Some(handle) = handle {
            if let Some(mut material) = materials.get_mut(&handle.0) {
                material.motion = motion;
            }
        } else {
            let material = materials.add(FlyoutMaterial {
                motion,
                panel: Vec4::from_array(PANEL.to_linear().to_f32_array()),
                accent: Vec4::from_array(FOCUS.to_linear().to_f32_array()),
            });
            commands.entity(entity).insert(MaterialNode(material));
        }
        if motion.x >= 1.0 && entrance.closed.is_none() {
            commands.entity(entity).remove::<Transition>();
        }
    }
}

fn animate_rows(
    mut commands: Commands,
    time: Res<Time>,
    lists: Query<(Entity, &RowTransition, &Children)>,
    children: Query<&Children>,
    mut transforms: Query<&mut UiTransform>,
    mut colors: Query<&mut TextColor>,
) {
    for (entity, entrance, rows) in &lists {
        let now = time.elapsed_secs_f64();
        let age = entrance.0.age(now);
        let exit = entrance.0.exit(now);
        for (index, row) in rows.iter().enumerate() {
            let delay = index.min(8) as f32 * 0.012;
            let visibility = ease_out((age - delay) / 0.16) * (1.0 - exit);
            if let Ok(mut transform) = transforms.get_mut(row) {
                transform.set_if_neq(UiTransform::from_translation(Val2::px(
                    7.0 * (1.0 - visibility),
                    0.0,
                )));
            }
            if let Ok(labels) = children.get(row) {
                for label in labels.iter() {
                    if let Ok(mut color) = colors.get_mut(label) {
                        let tint = color.0.with_alpha(visibility);
                        color.set_if_neq(TextColor(tint));
                    }
                }
            }
        }
        if age >= 0.26 && entrance.0.closed.is_none() {
            commands.entity(entity).remove::<RowTransition>();
        }
    }
}
