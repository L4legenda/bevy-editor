//! Model workspace lighting starts with the same defaults as a new scene.
use super::paint::post::{DocumentPost, Effects, Field as PostField};
use super::*;
use crate::editor::ui::environment_library::{self, Library, List};
use bevy::{
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
    PaintExposure,
    Blur,
    BackgroundBrightness,
    KeyIntensity,
    KeyAzimuth,
    KeyElevation,
    Post(PostField),
}

impl Setting {
    fn hint(self) -> &'static str {
        match self {
            Self::Intensity => {
                "Сила освещения от окружения. Меняет только просмотр модели, настройки сцены остаются прежними."
            }
            Self::Rotation => {
                "Поворот окружения вокруг модели. Позволяет проверить отражения и освещение с разных направлений."
            }
            Self::Exposure | Self::PaintExposure => {
                "Общая яркость предпросмотра. Положительные значения делают изображение светлее, отрицательные — темнее."
            }
            Self::Blur => "Размытие видимого фона. На освещение модели не влияет.",
            Self::BackgroundBrightness => {
                "Яркость фонового изображения. Освещение модели настраивается отдельно."
            }
            Self::KeyIntensity => {
                "Яркость дополнительного направленного света. Сначала включите свет в этом разделе."
            }
            Self::KeyAzimuth => {
                "Горизонтальное направление дополнительного света относительно окружения, в градусах."
            }
            Self::KeyElevation => "Высота дополнительного света над горизонтом, в градусах.",
            Self::Post(field) => field.hint(),
        }
    }
    fn is_paint_post(self) -> bool {
        matches!(self, Self::PaintExposure | Self::Post(_))
    }

    fn post_value(self, post: DocumentPost) -> f32 {
        match self {
            Self::PaintExposure => post.exposure,
            Self::Post(field) => field.value(post.effects),
            _ => unreachable!("only post-processing sliders read the model grade"),
        }
    }

    fn range(self) -> (f32, f32) {
        match self {
            Self::Intensity => (0.0, 4.0),
            Self::Rotation | Self::KeyAzimuth => (-180.0, 180.0),
            Self::Exposure => (-4.0, 3.0),
            Self::PaintExposure => DocumentPost::EXPOSURE_RANGE,
            Self::Blur => (0.0, 1.0),
            Self::BackgroundBrightness => (0.0, 2.0),
            Self::KeyIntensity => (0.0, 8.0),
            Self::KeyElevation => (5.0, 90.0),
            Self::Post(field) => field.range(),
        }
    }

    fn value(self, settings: &Settings) -> f32 {
        match self {
            Self::Intensity => settings.lighting.hdri_brightness / LIGHT_SCALE,
            Self::Rotation => settings.lighting.hdri_rotation,
            Self::Exposure | Self::PaintExposure => {
                crate::SceneLighting::editor_default().exposure_ev100
                    - settings.lighting.exposure_ev100
            }
            Self::Blur => settings.blur,
            Self::BackgroundBrightness => settings.background_brightness,
            Self::KeyIntensity => settings.key_intensity,
            Self::KeyAzimuth => settings.key_azimuth,
            Self::KeyElevation => settings.key_elevation,
            Self::Post(field) => field.value(settings.post),
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
            Self::Exposure | Self::PaintExposure => {
                settings.lighting.exposure_ev100 =
                    crate::SceneLighting::editor_default().exposure_ev100 - value
            }
            Self::Blur => settings.blur = value,
            Self::BackgroundBrightness => settings.background_brightness = value,
            Self::KeyIntensity => settings.key_intensity = value,
            Self::KeyAzimuth => settings.key_azimuth = value,
            Self::KeyElevation => settings.key_elevation = value,
            Self::Post(field) => field.set(&mut settings.post, value),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum EnvironmentAction {
    TogglePanel,
    Set(Setting, f32, bool),
    Preset(crate::EnvironmentPreset),
    ToggleBackground,
    ToggleKey,
    ToneMap,
    PostTone,
    ResetPost,
    Reset,
}

#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Settings {
    #[serde(default)]
    version: u32,
    lighting: crate::SceneLighting,
    blur: f32,
    background_brightness: f32,
    key_enabled: bool,
    key_intensity: f32,
    key_azimuth: f32,
    key_elevation: f32,
    pub(super) post: Effects,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 2,
            lighting: crate::SceneLighting::editor_default(),
            blur: 0.0,
            background_brightness: 1.0,
            key_enabled: false,
            key_intensity: 2.2,
            key_azimuth: 40.0,
            key_elevation: 45.0,
            post: Effects::default(),
        }
    }
}

impl Settings {
    pub(super) fn paint_post(&self, doc: Option<&crate::editor::model_document::Document>) -> DocumentPost {
        doc.and_then(|doc| doc.paint_post).unwrap_or(DocumentPost {
            exposure: Setting::Exposure.value(self),
            effects: self.post,
        })
    }

    pub(super) fn tone_mapper(&self) -> bevy::render::view::Tonemapping {
        self.lighting.tonemapping.into()
    }

    pub(super) fn trace_light(&self, key: Option<(&DirectionalLight, &Transform)>) -> (super::paint::pathtrace::Lighting, f32, crate::ToneMap) {
        let (key_direction, key_color, key_intensity) = key
            .map(|(light, transform)| {
                let color = light.color.to_linear();
                (*transform.back(), Vec3::new(color.red, color.green, color.blue), light.illuminance / LIGHT_SCALE)
            })
            .unwrap_or((Vec3::Y, Vec3::ONE, 0.0));
        (
            super::paint::pathtrace::Lighting {
                environment_rotation: self.lighting.hdri_rotation.to_radians(),
                environment_intensity: self.lighting.hdri_brightness / LIGHT_SCALE,
                background: (!self.lighting.hdri_background).then_some(Vec3::new(0.05, 0.05, 0.055)),
                background_blur: self.blur,
                background_intensity: self.background_brightness,
                key_direction,
                key_color,
                key_intensity,
            },
            Setting::Exposure.value(self),
            self.lighting.tonemapping,
        )
    }

    fn valid(&self) -> bool {
        self.version == 2
            && self.post.valid()
            && self.lighting.validate().is_ok()
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
pub(super) struct Applied {
    mode: shading::Mode,
    lighting: crate::SceneLighting,
    backdrop: crate::hdri::EnvironmentBackdrop,
}
#[derive(Component)]
pub(super) struct Panel;
#[derive(Resource)]
pub(super) struct PanelUi {
    section: Entity,
    overrides: Entity,
    picker: Entity,
    uv_open: bool,
    model_open: bool,
}
impl PanelUi {
    fn open(&self, uv: bool) -> bool {
        if uv { self.uv_open } else { self.model_open }
    }
}
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
    PostTone,
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
            ..column(0.0)
        },
    );
    commands.entity(panel).insert(Panel);
    let font = &interface.font;
    let section = inspector::section(
        &mut commands,
        panel,
        font,
        "Окружение",
        "environment lighting hdr hdri",
        true,
    );
    commands.entity(section.button).insert((
        Control {
            action: action(EnvironmentAction::TogglePanel),
            look: Look::Regular,
        },
        AccessibleLabel("Окружение просмотра модели".into()),
    ));
    let picker = node(&mut commands, section.body, column(0.0));
    environment_library::picker(&mut commands, picker, interface.root, font, List::Model);
    let overrides = node(
        &mut commands,
        section.body,
        Node {
            width: percent(100),
            ..column(6.0)
        },
    );
    commands.insert_resource(PanelUi {
        section: section.root,
        overrides,
        picker,
        uv_open: false,
        model_open: true,
    });
    let panel = overrides;
    let presets = node(
        &mut commands,
        panel,
        Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(3),
            ..row(3.0)
        },
    );
    inspector::searchable(&mut commands, presets, "схемы освещения пресеты presets");
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
        control_hints::attach(
            &mut commands,
            button,
            "Применить схему освещения для просмотра модели. Настройки сцены и материалы не меняются.",
        );
    }
    for (label, field) in [
        ("Яркость окружения", Setting::Intensity),
        ("Поворот окружения, °", Setting::Rotation),
        ("Экспозиция, EV", Setting::Exposure),
    ] {
        scalar(&mut commands, panel, font, label, field, field.value(&settings));
    }
    let tone_row = node(&mut commands, panel, column(0.0));
    inspector::searchable(
        &mut commands,
        tone_row,
        "тонмаппинг цвет tone mapping aces agx",
    );
    let tone = button(
        &mut commands,
        tone_row,
        font,
        "",
        action(EnvironmentAction::ToneMap),
        Look::Regular,
    );
    caption(&mut commands, tone, Caption::ToneMap);
    control_hints::attach(
        &mut commands,
        tone,
        "Переключить обработку яркости и цвета: ACES, AgX или нейтральное отображение PBR. Меняет только предпросмотр.",
    );
    let heading = text(&mut commands, panel, font, "Фон", 12.0, MUTED);
    control_hints::attach(
        &mut commands,
        heading,
        "Отображение окружения за моделью. Фон можно затемнить или размыть независимо от освещения.",
    );
    let background_row = node(&mut commands, panel, column(0.0));
    inspector::searchable(
        &mut commands,
        background_row,
        "фон показать скрыть background",
    );
    let background = button(
        &mut commands,
        background_row,
        font,
        "",
        action(EnvironmentAction::ToggleBackground),
        Look::Regular,
    );
    caption(&mut commands, background, Caption::Background);
    control_hints::attach(
        &mut commands,
        background,
        "Переключить фон между изображением окружения и тёмной заливкой. Освещение модели сохраняется.",
    );
    scalar(
        &mut commands,
        panel,
        font,
        "Размытие фона",
        Setting::Blur,
        Setting::Blur.value(&settings),
    );
    scalar(
        &mut commands,
        panel,
        font,
        "Яркость фона",
        Setting::BackgroundBrightness,
        Setting::BackgroundBrightness.value(&settings),
    );
    let heading = text(
        &mut commands,
        panel,
        font,
        "Дополнительный свет",
        12.0,
        MUTED,
    );
    control_hints::attach(
        &mut commands,
        heading,
        "Направленный свет для проверки формы и теней. Работает вместе с окружением и не добавляется в сцену.",
    );
    let key_row = node(&mut commands, panel, column(0.0));
    inspector::searchable(
        &mut commands,
        key_row,
        "направленный дополнительный свет key light",
    );
    let key = button(
        &mut commands,
        key_row,
        font,
        "",
        action(EnvironmentAction::ToggleKey),
        Look::Regular,
    );
    caption(&mut commands, key, Caption::Key);
    control_hints::attach(
        &mut commands,
        key,
        "Включить или выключить дополнительный направленный свет в предпросмотре.",
    );
    for (label, field) in [
        ("Сила света", Setting::KeyIntensity),
        ("Направление света, °", Setting::KeyAzimuth),
        ("Высота света, °", Setting::KeyElevation),
    ] {
        scalar(&mut commands, panel, font, label, field, field.value(&settings));
    }
    let reset_row = node(&mut commands, panel, column(0.0));
    inspector::searchable(&mut commands, reset_row, "сбросить просмотр reset defaults");
    let reset = button(
        &mut commands,
        reset_row,
        font,
        "Сбросить просмотр",
        action(EnvironmentAction::Reset),
        Look::Regular,
    );
    control_hints::attach(
        &mut commands,
        reset,
        "Вернуть исходные настройки окружения, фона, экспозиции и дополнительного света. UV и геометрия не меняются.",
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
    value: f32,
) {
    let parent = node(commands, parent, column(0.0));
    inspector::searchable(commands, parent, label);
    let header = node(
        commands,
        parent,
        Node {
            width: percent(100),
            ..row(6.0)
        },
    );
    let title = text(commands, header, font, label, 12.0, MUTED);
    control_hints::attach(commands, title, setting.hint());
    commands.entity(title).insert(Node {
        flex_grow: 1.0,
        ..default()
    });
    let readout = text(commands, header, font, "", 12.0, TEXT);
    control_hints::attach(commands, readout, setting.hint());
    let (min, max) = setting.range();
    let slider = commands
        .spawn((
            Slider {
                track_click: TrackClick::Snap,
                ..default()
            },
            SliderRange::new(min, max),
            SliderValue(value),
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
    control_hints::attach(commands, slider, setting.hint());
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

pub(super) fn paint_post_controls(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    post: DocumentPost,
) {
    scalar(commands, parent, font, "Экспозиция", Setting::PaintExposure, post.exposure);
    let tone = button(commands, parent, font, "", action(EnvironmentAction::PostTone), Look::Regular);
    caption(commands, tone, Caption::PostTone);
    for (field, label) in PostField::ALL {
        scalar(commands, parent, font, label, Setting::Post(field), field.value(post.effects));
    }
    button(
        commands, parent, font, "Сбросить обработку изображения",
        action(EnvironmentAction::ResetPost), Look::Regular,
    );
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
    mut panel: Option<ResMut<PanelUi>>,
    mut editing: Option<ResMut<editing::State>>,
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
        if matches!(action, EnvironmentAction::TogglePanel) {
            if let Some(panel) = panel.as_deref_mut() {
                let uv = editing.as_ref().is_some_and(|s| {
                    s.mode(editor.viewing_model) == crate::editor::model_document::Mode::Uv
                });
                if uv {
                    panel.uv_open = !panel.uv_open;
                } else {
                    panel.model_open = !panel.model_open;
                }
            }
            continue;
        }
        let paint_post = match action {
            EnvironmentAction::Set(field, ..) => field.is_paint_post(),
            EnvironmentAction::PostTone | EnvironmentAction::ResetPost => true,
            _ => false,
        };
        if paint_post
            && let Some(doc) = editor.viewing_model.and_then(|index| editing.as_deref_mut()?.documents.get_mut(&index))
                .filter(|doc| doc.mode == crate::editor::model_document::Mode::Paint)
        {
            if doc.can_store_paint_preview() {
                let mut post = settings.paint_post(Some(doc));
                match action {
                    EnvironmentAction::Set(Setting::PaintExposure, value, _) if value.is_finite() => {
                        let (min, max) = DocumentPost::EXPOSURE_RANGE;
                        post.exposure = value.clamp(min, max);
                    }
                    EnvironmentAction::Set(Setting::Post(field), value, _) if value.is_finite() => {
                        field.set(&mut post.effects, value);
                    }
                    EnvironmentAction::PostTone => post.effects.cycle_curve(),
                    EnvironmentAction::ResetPost => post = DocumentPost::default(),
                    _ => continue,
                }
                if doc.paint_post != Some(post) {
                    doc.paint_post = Some(post);
                    doc.ui_revision += 1;
                }
            }
            continue;
        }
        if matches!(action, EnvironmentAction::Set(Setting::PaintExposure, ..)) {
            continue;
        }
        save = true;
        match action {
            EnvironmentAction::TogglePanel => unreachable!(),
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
            EnvironmentAction::PostTone => settings.post.cycle_curve(),
            EnvironmentAction::ResetPost => {
                settings.post = Effects::default();
                Setting::Exposure.set(&mut settings, 0.0);
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
    library: Res<Library>,
    editor: Res<Editor>,
    shading: Res<shading::Shading>,
    editing: Res<editing::State>,
    viewer: Res<Viewer>,
    mut commands: Commands,
    cameras: Query<(Entity, Option<&Applied>), With<ViewCamera>>,
    scene_light_entities: Query<Entity, With<PreviewSceneLight>>,
    mut scene_lights: Local<Option<(Entity, bool, Vec<crate::lighting::SceneLight>)>>,
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
    use shading::Mode;
    let (mode, preview_lighting) = shading.viewport(editing.mode(editor.viewing_model));
    let saved = if preview_lighting
        && editing.mode(editor.viewing_model) != crate::editor::model_document::Mode::Paint
    {
        library.model_lighting()
    } else {
        None
    };
    let lighting = match mode {
        Mode::Rendered if preview_lighting => saved.unwrap_or(&settings.lighting).clone(),
        Mode::Rendered => editor.environment().clone(),
        Mode::Solid => crate::SceneLighting {
            ambient_brightness: 420.0,
            exposure_ev100: 8.0,
            shadows: false,
            ..default()
        },
        Mode::Material | Mode::Wire => crate::SceneLighting {
            ambient_brightness: 0.0,
            tonemapping: crate::ToneMap::Linear,
            shadows: false,
            ..default()
        },
    };
    let backdrop = if preview_lighting && saved.is_none() {
        crate::hdri::EnvironmentBackdrop {
            brightness: settings.background_brightness,
            blur: settings.blur,
        }
    } else {
        crate::hdri::EnvironmentBackdrop {
            brightness: 1.0,
            blur: 0.0,
        }
    };
    for (entity, applied) in &cameras {
        if applied.is_some_and(|applied| {
            applied.mode == mode
                && applied.lighting == lighting
                && applied.backdrop.brightness == backdrop.brightness
                && applied.backdrop.blur == backdrop.blur
        }) {
            continue;
        }
        let mut camera = commands.entity(entity);
        crate::lighting::apply_camera_lighting(&mut camera, &lighting);
        camera.insert((
            backdrop,
            Applied {
                mode,
                lighting: lighting.clone(),
                backdrop,
            },
        ));
    }
    let desired_lights = viewer
        .stage
        .filter(|_| mode == Mode::Rendered && !preview_lighting)
        .map(|stage| {
            let lights = editor
                .edits
                .lights
                .iter()
                .filter(|light| light.enabled && !editor.object_hidden(Selection::Light(light.id)))
                .collect::<Vec<_>>();
            (stage, lights)
        });
    let lights_unchanged = match (&*scene_lights, &desired_lights) {
        (Some((stage, shadows, current)), Some((desired_stage, desired))) => {
            stage == desired_stage
                && *shadows == lighting.shadows
                && current.iter().eq(desired.iter().copied())
        }
        (None, None) => true,
        _ => false,
    };
    if !lights_unchanged {
        for entity in &scene_light_entities {
            commands.entity(entity).try_despawn();
        }
        *scene_lights = desired_lights.map(|(stage, lights)| {
            let shadows = lighting.shadows;
            for &light in &lights {
                let mut effective = light.clone();
                effective.shadows &= shadows;
                effective.contact_shadows &= shadows;
                let entity = crate::lighting::spawn_light(&mut commands, &effective);
                commands.entity(entity).insert((
                    PreviewSceneLight,
                    RenderLayers::layer(LAYER),
                    ChildOf(stage),
                ));
            }
            (stage, shadows, lights.into_iter().cloned().collect())
        });
    }
    for (entity, mut light, mut pose, cascades) in &mut lights {
        let (illuminance, color, shadows, azimuth, elevation) = match mode {
            Mode::Rendered if preview_lighting && saved.is_none() => (
                if settings.key_enabled {
                    settings.key_intensity * LIGHT_SCALE
                } else {
                    0.0
                },
                Color::linear_rgb(1.0, 0.96, 0.9),
                settings.key_enabled,
                settings.key_azimuth + settings.lighting.hdri_rotation,
                settings.key_elevation,
            ),
            Mode::Solid => (2400.0, Color::WHITE, false, 35.0, 55.0),
            Mode::Rendered | Mode::Wire | Mode::Material => (0.0, Color::WHITE, false, 0.0, 45.0),
        };
        if light.illuminance != illuminance
            || light.color != color
            || light.shadow_maps_enabled != shadows
        {
            light.illuminance = illuminance;
            light.color = color;
            light.shadow_maps_enabled = shadows;
        }
        let azimuth = azimuth.to_radians();
        let elevation = elevation.to_radians();
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
        pose.set_if_neq(Transform::from_translation(direction).looking_at(Vec3::ZERO, up));
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

#[derive(Component)]
pub(super) struct PreviewSceneLight;

pub(super) fn refresh(
    editor: Res<Editor>,
    settings: Res<Settings>,
    library: Res<Library>,
    ui: Res<PanelUi>,
    mut sections: Query<&mut inspector::Section>,
    shading: Res<shading::Shading>,
    editing: Res<editing::State>,
    mut commands: Commands,
    mut panels: Query<&mut Node, With<Panel>>,
    controls: Query<(Entity, &ControlValue, &ComputedNode)>,
    mut nodes: Query<&mut Node, Without<Panel>>,
    mut labels: Query<&mut Text>,
    captions: Query<(&Caption, &Children)>,
    mut presets: Query<(&PresetChoice, &Hovered, &mut BackgroundColor)>,
) {
    use crate::editor::model_document::Mode;
    let editing_mode = editing.mode(editor.viewing_model);
    let (mode, preview_lighting) = shading.viewport(editing_mode);
    let shown = editor.viewing_model.is_some() && editor.viewing_image.is_none();
    let open = ui.open(editing_mode == Mode::Uv);
    if let Ok(mut section) = sections.get_mut(ui.section)
        && section.open() != open
    {
        section.set_open(open);
    }
    if let Ok(mut picker) = nodes.get_mut(ui.picker) {
        picker.display = if editing_mode == Mode::Paint {
            Display::None
        } else {
            Display::Flex
        };
    }
    let manual = mode == shading::Mode::Rendered
        && preview_lighting
        && (editing_mode == Mode::Paint || library.model_lighting().is_none());
    if let Ok(mut overrides) = nodes.get_mut(ui.overrides) {
        overrides.display = if manual { Display::Flex } else { Display::None };
    }
    let preview_enabled = shown && manual && editor.pending.is_none() && !editor.hdri_library_open;
    let paint_doc = editor
        .viewing_model
        .and_then(|index| editing.documents.get(&index))
        .filter(|doc| doc.mode == Mode::Paint);
    let post = settings.paint_post(paint_doc);
    for mut panel in &mut panels {
        panel.display = if shown { Display::Flex } else { Display::None };
    }
    for (entity, control, computed) in &controls {
        let paint_control = control.setting.is_paint_post();
        let value = if paint_control { control.setting.post_value(post) } else { control.setting.value(&settings) };
        let (min, max) = control.setting.range();
        let position = (value - min) / (max - min);
        if let Ok(mut thumb) = nodes.get_mut(control.thumb) {
            thumb.left = px(
                (computed.size().x * computed.inverse_scale_factor() - 14.0).max(0.0) * position,
            );
        }
        let enabled = preview_enabled && !computed.is_empty()
            && if paint_control {
                paint_doc.is_some_and(|doc| doc.can_store_paint_preview())
            } else {
                open
            };
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
            Caption::ToneMap => format!("Цвет: {}", settings.lighting.tonemapping.label()),
            Caption::PostTone => format!("Тональная компрессия · {}", post.effects.curve.map_or("Окружение", |curve| curve.label())),
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
        app.world_mut().write_message(action(EnvironmentAction::Set(
            Setting::Post(PostField::BloomThreshold), 6.0, true,
        )));
        app.world_mut().write_message(action(EnvironmentAction::PostTone));
        app.update();
        let saved: Settings =
            serde_json::from_slice(&std::fs::read(scratch.join(SETTINGS_FILE)).unwrap()).unwrap();
        assert!(saved.valid());
        assert_eq!(saved, *app.world().resource::<Settings>());
        assert_eq!(Setting::Exposure.value(&saved), 1.5);
        assert_eq!(PostField::BloomThreshold.value(saved.post), 4.0);
        assert_eq!(saved.post.curve, Some(crate::ToneMap::AcesFitted));
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
    fn scene_settings_round_trip_and_reject_legacy_studio_preferences() {
        let settings = Settings::default();
        assert!(settings.valid());
        assert_eq!(settings.lighting, crate::SceneLighting::editor_default());
        assert!(!settings.key_enabled);
        assert_eq!(
            settings.lighting.hdri,
            Some(crate::EnvironmentPreset::Cloudy)
        );
        let json = serde_json::to_vec(&settings).unwrap();
        assert_eq!(serde_json::from_slice::<Settings>(&json).unwrap(), settings);
        let mut before_post = serde_json::to_value(&settings).unwrap();
        before_post.as_object_mut().unwrap().remove("post");
        assert_eq!(serde_json::from_value::<Settings>(before_post).unwrap(), settings);
        let mut legacy = serde_json::to_value(&settings).unwrap();
        legacy.as_object_mut().unwrap().remove("version");
        assert!(!serde_json::from_value::<Settings>(legacy).unwrap().valid());
        for invalid in [f32::NAN, f32::INFINITY, -1.0, 1.1] {
            let mut corrupt = settings.clone();
            corrupt.blur = invalid;
            assert!(!corrupt.valid());
        }
    }
}
