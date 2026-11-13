//! Studio preview controls reuse the editor's HDRI and camera lighting renderer.
use super::*;
use bevy::{
    camera::Exposure,
    light::{CascadeShadowConfig, CascadeShadowConfigBuilder},
    ui_widgets::{
        Slider, SliderRange, SliderStep, SliderThumb, SliderValue, TrackClick, ValueChange,
    },
};
use serde::{Deserialize, Serialize};

const SETTINGS_FILE: &str = ".bevy-editor/model-preview.json";
// Match a unit-exposure material preview to Bevy's physical lighting units.
const LIGHT_SCALE: f32 = 1000.0;

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum Setting {
    Intensity,
    Rotation,
    Exposure,
    Blur,
    BackgroundBrightness,
    KeyIntensity,
    KeyAzimuth,
    KeyElevation,
}

impl Setting {
    fn range(self) -> (f32, f32) {
        match self {
            Self::Intensity => (0.0, 4.0),
            Self::Rotation | Self::KeyAzimuth => (-180.0, 180.0),
            Self::Exposure => (-4.0, 3.0),
            Self::Blur => (0.0, 1.0),
            Self::BackgroundBrightness => (0.0, 2.0),
            Self::KeyIntensity => (0.0, 8.0),
            Self::KeyElevation => (5.0, 90.0),
        }
    }

    fn value(self, settings: &Settings) -> f32 {
        match self {
            Self::Intensity => settings.lighting.hdri_brightness / LIGHT_SCALE,
            Self::Rotation => settings.lighting.hdri_rotation,
            Self::Exposure => Exposure::BLENDER.ev100 - settings.lighting.exposure_ev100,
            Self::Blur => settings.blur,
            Self::BackgroundBrightness => settings.background_brightness,
            Self::KeyIntensity => settings.key_intensity,
            Self::KeyAzimuth => settings.key_azimuth,
            Self::KeyElevation => settings.key_elevation,
        }
    }

    fn set(self, settings: &mut Settings, value: f32) {
        if !value.is_finite() {
            return;
        }
        let (min, max) = self.range();
        let value = value.clamp(min, max);
        match self {
            Self::Intensity => settings.lighting.hdri_brightness = value * LIGHT_SCALE,
            Self::Rotation => settings.lighting.hdri_rotation = value,
            Self::Exposure => settings.lighting.exposure_ev100 = Exposure::BLENDER.ev100 - value,
            Self::Blur => settings.blur = value,
            Self::BackgroundBrightness => settings.background_brightness = value,
            Self::KeyIntensity => settings.key_intensity = value,
            Self::KeyAzimuth => settings.key_azimuth = value,
            Self::KeyElevation => settings.key_elevation = value,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum EnvironmentAction {
    Set(Setting, f32, bool),
    Preset(crate::EnvironmentPreset),
    ToggleBackground,
    ToggleKey,
    ToneMap,
    Reset,
}

#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Settings {
    lighting: crate::SceneLighting,
    blur: f32,
    background_brightness: f32,
    key_enabled: bool,
    key_intensity: f32,
    key_azimuth: f32,
    key_elevation: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lighting: crate::SceneLighting {
                hdri: Some(crate::EnvironmentPreset::SmallStudio),
                hdri_brightness: LIGHT_SCALE,
                hdri_background: true,
                ambient_brightness: 0.0,
                exposure_ev100: Exposure::BLENDER.ev100,
                tonemapping: crate::ToneMap::AcesFitted,
                ..default()
            },
            blur: 0.6,
            background_brightness: 0.55,
            key_enabled: true,
            key_intensity: 2.2,
            key_azimuth: 40.0,
            key_elevation: 45.0,
        }
    }
}

impl Settings {
    fn valid(&self) -> bool {
        self.lighting.validate().is_ok()
            && self.lighting.hdri.is_some()
            && self.lighting.hdri_asset.is_none()
            && [
                Setting::Intensity,
                Setting::Rotation,
                Setting::Exposure,
                Setting::Blur,
                Setting::BackgroundBrightness,
                Setting::KeyIntensity,
                Setting::KeyAzimuth,
                Setting::KeyElevation,
            ]
            .into_iter()
            .all(|field| {
                let value = field.value(self);
                let (min, max) = field.range();
                value.is_finite() && (min..=max).contains(&value)
            })
    }
}

#[derive(Component)]
pub(super) struct KeyLight;
#[derive(Component)]
pub(super) struct Applied(Settings);
#[derive(Component)]
pub(super) struct Panel;
#[derive(Component)]
pub(super) struct ControlValue {
    setting: Setting,
    thumb: Entity,
    readout: Entity,
}
#[derive(Component)]
pub(super) enum Caption {
    Background,
    Key,
    ToneMap,
}

#[derive(Component)]
pub(super) struct PresetChoice(crate::EnvironmentPreset);

fn action(action: EnvironmentAction) -> Action {
    Action::ModelView(ModelViewAction::Environment(action))
}

pub(super) fn setup(mut commands: Commands, interface: Res<Interface>, editor: Res<Editor>) {
    let settings = std::fs::read(editor.project.asset_root.join(SETTINGS_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
        .filter(Settings::valid)
        .unwrap_or_default();
    let panel = node(
        &mut commands,
        interface.inspector,
        Node {
            display: Display::None,
            width: percent(100),
            flex_shrink: 0.0,
            ..column(6.0)
        },
    );
    commands
        .entity(panel)
        .insert((Panel, AccessibleLabel("Окружение просмотра модели".into())));
    let font = &interface.font;
    text(
        &mut commands,
        panel,
        font,
        "ОКРУЖЕНИЕ · ПРОСМОТР",
        11.0,
        MUTED,
    );
    let presets = node(
        &mut commands,
        panel,
        Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(3),
            ..row(3.0)
        },
    );
    for preset in crate::EnvironmentPreset::ALL {
        let button = button(
            &mut commands,
            presets,
            font,
            preset.label(),
            action(EnvironmentAction::Preset(preset)),
            Look::Regular,
        );
        commands.entity(button).insert((
            PresetChoice(preset),
            AccessibleLabel(format!("Окружение модели: {}", preset.label())),
        ));
    }
    for (label, field) in [
        ("Яркость окружения", Setting::Intensity),
        ("Поворот окружения, °", Setting::Rotation),
        ("Экспозиция, EV", Setting::Exposure),
    ] {
        scalar(&mut commands, panel, font, label, field, &settings);
    }
    let tone = button(
        &mut commands,
        panel,
        font,
        "",
        action(EnvironmentAction::ToneMap),
        Look::Regular,
    );
    caption(&mut commands, tone, Caption::ToneMap);
    text(&mut commands, panel, font, "ФОН", 11.0, MUTED);
    let background = button(
        &mut commands,
        panel,
        font,
        "",
        action(EnvironmentAction::ToggleBackground),
        Look::Regular,
    );
    caption(&mut commands, background, Caption::Background);
    scalar(
        &mut commands,
        panel,
        font,
        "Размытие фона",
        Setting::Blur,
        &settings,
    );
    scalar(
        &mut commands,
        panel,
        font,
        "Яркость фона",
        Setting::BackgroundBrightness,
        &settings,
    );
    text(&mut commands, panel, font, "ОСНОВНОЙ СВЕТ", 11.0, MUTED);
    let key = button(
        &mut commands,
        panel,
        font,
        "",
        action(EnvironmentAction::ToggleKey),
        Look::Regular,
    );
    caption(&mut commands, key, Caption::Key);
    for (label, field) in [
        ("Сила света", Setting::KeyIntensity),
        ("Направление света, °", Setting::KeyAzimuth),
        ("Высота света, °", Setting::KeyElevation),
    ] {
        scalar(&mut commands, panel, font, label, field, &settings);
    }
    button(
        &mut commands,
        panel,
        font,
        "Сбросить просмотр",
        action(EnvironmentAction::Reset),
        Look::Regular,
    );
    commands.insert_resource(settings);
}

fn caption(commands: &mut Commands, parent: Entity, caption: Caption) {
    // The standard button creates exactly one text child.
    commands.entity(parent).insert(caption);
}

fn scalar(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    setting: Setting,
    settings: &Settings,
) {
    let header = node(
        commands,
        parent,
        Node {
            width: percent(100),
            ..row(6.0)
        },
    );
    let title = text(commands, header, font, label, 12.0, MUTED);
    commands.entity(title).insert(Node {
        flex_grow: 1.0,
        ..default()
    });
    let readout = text(commands, header, font, "", 12.0, TEXT);
    let (min, max) = setting.range();
    let slider = commands
        .spawn((
            Slider {
                track_click: TrackClick::Snap,
                ..default()
            },
            SliderRange::new(min, max),
            SliderValue(setting.value(settings)),
            SliderStep(if max > 10.0 { 1.0 } else { 0.01 }),
            TabIndex(0),
            Hovered::default(),
            AccessibleLabel(format!("Просмотр: {label}")),
            Node {
                width: percent(100),
                height: px(22),
                flex_shrink: 0.0,
                ..default()
            },
            ChildOf(parent),
        ))
        .observe(change_slider)
        .id();
    let track = surface(
        commands,
        slider,
        Node {
            position_type: PositionType::Absolute,
            left: px(7),
            right: px(7),
            top: px(9),
            height: px(4),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        WELL,
    );
    commands.entity(track).insert(Pickable::IGNORE);
    let thumb = commands
        .spawn((
            SliderThumb,
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                width: px(14),
                height: px(14),
                top: px(4),
                border: UiRect::all(px(2)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(PANEL),
            BorderColor::all(FOCUS),
            ChildOf(slider),
        ))
        .id();
    commands.entity(slider).insert(ControlValue {
        setting,
        thumb,
        readout,
    });
}

fn change_slider(
    event: On<ValueChange<f32>>,
    controls: Query<&ControlValue>,
    mut actions: MessageWriter<Action>,
) {
    if let Ok(control) = controls.get(event.source) {
        actions.write(action(EnvironmentAction::Set(
            control.setting,
            event.value,
            event.is_final,
        )));
    }
}

pub(super) fn actions(
    mut events: MessageReader<Action>,
    editor: Res<Editor>,
    mut settings: ResMut<Settings>,
) {
    if editor.viewing_model.is_none()
        || editor.pending.is_some()
        || editor.viewing_image.is_some()
        || editor.hdri_library_open
    {
        events.clear();
        return;
    }
    let mut save = false;
    for event in events.read() {
        let Action::ModelView(ModelViewAction::Environment(action)) = *event else {
            continue;
        };
        save = true;
        match action {
            EnvironmentAction::Set(field, value, final_value) => {
                field.set(&mut settings, value);
                save = final_value;
            }
            EnvironmentAction::Preset(preset) => settings.lighting.hdri = Some(preset),
            EnvironmentAction::ToggleBackground => {
                settings.lighting.hdri_background = !settings.lighting.hdri_background
            }
            EnvironmentAction::ToggleKey => settings.key_enabled = !settings.key_enabled,
            EnvironmentAction::ToneMap => {
                settings.lighting.tonemapping = match settings.lighting.tonemapping {
                    crate::ToneMap::AcesFitted => crate::ToneMap::AgX,
                    crate::ToneMap::AgX => crate::ToneMap::KhronosPbrNeutral,
                    _ => crate::ToneMap::AcesFitted,
                }
            }
            EnvironmentAction::Reset => *settings = Settings::default(),
        }
    }
    if save {
        let path = editor.project.asset_root.join(SETTINGS_FILE);
        let result = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(path.parent().unwrap())?;
            let bytes = serde_json::to_vec_pretty(&*settings)?;
            let temporary = path.with_extension("json.tmp");
            std::fs::write(&temporary, bytes)?;
            std::fs::rename(temporary, path)
        })();
        if let Err(error) = result {
            warn!("Не удалось сохранить настройки просмотра: {error}");
        }
    }
}

pub(super) fn synchronize(
    settings: Res<Settings>,
    viewer: Res<Viewer>,
    mut commands: Commands,
    cameras: Query<(Entity, Option<&Applied>), With<ViewCamera>>,
    mut lights: Query<
        (
            Entity,
            &mut DirectionalLight,
            &mut Transform,
            Option<&CascadeShadowConfig>,
        ),
        With<KeyLight>,
    >,
) {
    let mut changed = false;
    for (entity, applied) in &cameras {
        if applied.is_some_and(|applied| applied.0 == *settings) {
            continue;
        }
        let mut camera = commands.entity(entity);
        crate::lighting::apply_camera_lighting(&mut camera, &settings.lighting);
        camera.insert((
            crate::hdri::EnvironmentBackdrop {
                brightness: settings.background_brightness,
                blur: settings.blur,
            },
            Applied(settings.clone()),
        ));
        changed = true;
    }
    for (entity, mut light, mut pose, cascades) in &mut lights {
        if changed {
            light.illuminance = if settings.key_enabled {
                settings.key_intensity * LIGHT_SCALE
            } else {
                0.0
            };
            light.color = Color::linear_rgb(1.0, 0.96, 0.9);
            light.shadow_maps_enabled = settings.key_enabled;
            let azimuth = (settings.key_azimuth + settings.lighting.hdri_rotation).to_radians();
            let elevation = settings.key_elevation.to_radians();
            let direction = Vec3::new(
                elevation.cos() * azimuth.sin(),
                elevation.sin(),
                elevation.cos() * azimuth.cos(),
            );
            let up = if direction.dot(Vec3::Y).abs() > 0.99 {
                Vec3::Z
            } else {
                Vec3::Y
            };
            *pose = Transform::from_translation(direction).looking_at(Vec3::ZERO, up);
        }
        // Cascades follow the inspected model's scale, including zoomed-out views.
        let distance = viewer.distance + viewer.radius * 4.0;
        if viewer.fitted
            && cascades.is_none_or(|cascades| cascades.bounds.last() != Some(&distance))
        {
            commands.entity(entity).insert(
                CascadeShadowConfigBuilder {
                    num_cascades: 1,
                    minimum_distance: (viewer.radius * 0.001).max(0.000001),
                    maximum_distance: distance,
                    ..default()
                }
                .build(),
            );
        }
    }
}

pub(super) fn refresh(
    editor: Res<Editor>,
    settings: Res<Settings>,
    mut commands: Commands,
    mut panels: Query<&mut Node, With<Panel>>,
    controls: Query<(Entity, &ControlValue, &ComputedNode)>,
    mut nodes: Query<&mut Node, Without<Panel>>,
    mut labels: Query<&mut Text>,
    captions: Query<(&Caption, &Children)>,
    mut presets: Query<(&PresetChoice, &Hovered, &mut BackgroundColor)>,
) {
    let shown = editor.viewing_model.is_some();
    let enabled = shown
        && editor.pending.is_none()
        && editor.viewing_image.is_none()
        && !editor.hdri_library_open;
    for mut panel in &mut panels {
        panel.display = if shown { Display::Flex } else { Display::None };
    }
    for (entity, control, computed) in &controls {
        let value = control.setting.value(&settings);
        let (min, max) = control.setting.range();
        let position = (value - min) / (max - min);
        if let Ok(mut thumb) = nodes.get_mut(control.thumb) {
            thumb.left = px(
                (computed.size().x * computed.inverse_scale_factor() - 14.0).max(0.0) * position,
            );
        }
        if let Ok(mut label) = labels.get_mut(control.readout) {
            label.0 = format!("{value:.2}");
        }
        commands
            .entity(entity)
            .insert((SliderValue(value), TabIndex(if enabled { 0 } else { -1 })));
        if enabled {
            commands.entity(entity).remove::<InteractionDisabled>();
        } else {
            commands.entity(entity).insert(InteractionDisabled);
        }
    }
    for (caption, children) in &captions {
        let value = match caption {
            Caption::Background => {
                if settings.lighting.hdri_background {
                    "Фон: окружение".into()
                } else {
                    "Фон: тёмный".into()
                }
            }
            Caption::Key => {
                if settings.key_enabled {
                    "Направленный свет: вкл.".into()
                } else {
                    "Направленный свет: выкл.".into()
                }
            }
            Caption::ToneMap => format!(
                "Тональное отображение: {}",
                settings.lighting.tonemapping.label()
            ),
        };
        for child in children.iter() {
            if let Ok(mut label) = labels.get_mut(child) {
                label.0.clone_from(&value);
            }
        }
    }
    for (preset, hovered, mut color) in &mut presets {
        color.0 = if settings.lighting.hdri == Some(preset.0) {
            ACTIVE
        } else if hovered.0 {
            HOVER
        } else {
            Color::NONE
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_controls_save_preferences_without_editing_the_scene() {
        let mut project = crate::EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        let scratch = std::env::temp_dir().join(format!(
            "bevy-model-environment-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        project.asset_root = scratch.clone();
        let mut editor = Editor::new(project, crate::LevelEdits::default());
        editor.viewing_model = Some(0);
        let before = editor.edits.clone();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(editor)
            .init_resource::<Settings>()
            .add_message::<Action>()
            .add_systems(Update, actions);
        app.world_mut()
            .write_message(action(EnvironmentAction::Set(Setting::Exposure, 1.5, true)));
        app.world_mut()
            .write_message(action(EnvironmentAction::Preset(
                crate::EnvironmentPreset::PhotoStudio,
            )));
        app.update();
        let saved: Settings =
            serde_json::from_slice(&std::fs::read(scratch.join(SETTINGS_FILE)).unwrap()).unwrap();
        assert!(saved.valid());
        assert_eq!(saved, *app.world().resource::<Settings>());
        assert_eq!(Setting::Exposure.value(&saved), 1.5);
        assert_eq!(
            saved.lighting.hdri,
            Some(crate::EnvironmentPreset::PhotoStudio)
        );
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.edits, before);
        assert!(editor.undo.is_empty());
        assert!(!editor.dirty());
        std::fs::remove_file(scratch.join(SETTINGS_FILE)).unwrap();
        std::fs::remove_dir(scratch.join(".bevy-editor")).unwrap();
        std::fs::remove_dir(scratch).unwrap();
    }

    #[test]
    fn studio_settings_round_trip_and_reject_invalid_preferences() {
        let settings = Settings::default();
        assert!(settings.valid());
        assert_eq!(settings.lighting.tonemapping, crate::ToneMap::AcesFitted);
        assert_eq!(
            settings.lighting.hdri,
            Some(crate::EnvironmentPreset::SmallStudio)
        );
        let json = serde_json::to_vec(&settings).unwrap();
        assert_eq!(serde_json::from_slice::<Settings>(&json).unwrap(), settings);
        for invalid in [f32::NAN, f32::INFINITY, -1.0, 1.1] {
            let mut corrupt = settings.clone();
            corrupt.blur = invalid;
            assert!(!corrupt.valid());
        }
    }
}
