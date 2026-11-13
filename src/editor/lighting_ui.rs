//! Lighting controls keep view preferences separate from undoable scene data.
use super::*;
use crate::{
    editor::lighting::{EnvironmentField, LightField, LightingAction},
    lighting::{EnvironmentPreset, LightKind, LightPreset, LightingPreset},
};
use bevy::{input_focus::FocusCause, window::WindowCloseRequested};

#[path = "lighting_controls.rs"]
mod controls;
use controls::{color_editor, scalar};
pub(crate) use controls::{input as edit_controls, refresh as refresh_controls};

const MENU_WIDTH: f32 = 450.0;

struct NameDraft {
    room: Room,
    light: u32,
    kind: LightKind,
    original: String,
    value: String,
}

impl NameDraft {
    fn clean_name(&self) -> String {
        let value: String = self
            .value
            .chars()
            .filter(|value| !value.is_control())
            .take(128)
            .collect();
        let value = value.trim();
        if value.is_empty() {
            self.kind.label().into()
        } else {
            value.into()
        }
    }
}

#[derive(Resource)]
pub(crate) struct LightingUi {
    add: Entity,
    menu: Entity,
    menu_first: Entity,
    inspector: Entity,
    name: Entity,
    tooltip: Entity,
    tooltip_label: Entity,
    menu_open: bool,
    advanced: bool,
    pointer_consumed: bool,
    controls: controls::State,
    name_draft: Option<NameDraft>,
    hint_target: Option<Entity>,
    hint_elapsed: f32,
}

impl LightingUi {
    pub fn is_menu_open(&self) -> bool {
        self.menu_open
    }

    pub fn blocks_scene_input(&self) -> bool {
        self.menu_open || self.pointer_consumed || self.controls.active()
    }

    pub fn is_control_active(&self) -> bool {
        self.controls.active()
    }

    pub fn name_has_changes(&self, editor: &Editor) -> bool {
        self.name_draft
            .as_ref()
            .is_some_and(|draft| draft.room == editor.room && draft.clean_name() != draft.original)
    }
}

#[derive(Component)]
pub(crate) struct NameInput;

#[derive(Component)]
pub(in crate::editor) struct AdvancedIcon(bool);

#[derive(Component)]
pub(crate) struct Help(&'static str);

#[derive(Clone, Copy, Component)]
pub(crate) enum VisibleWhen {
    Light,
    Environment,
    Hdri,
    Local,
    Radius,
    Spot,
    Rect,
    NonRect,
    Temperature,
    Fog,
    AdvancedEnvironment,
    AdvancedNonRect,
    AdvancedLocal,
    AdvancedDirectional,
}

#[derive(Clone, Copy, Component)]
pub(crate) enum Choice {
    Kind(LightKind),
    Environment(Option<EnvironmentPreset>),
    HdriBackground,
    Enabled,
    Temperature,
    Shadows,
    ContactShadows,
    Imported,
    GlobalShadows,
    Fog,
    Resolution(u32),
    Advanced,
    Menu,
    Gizmos,
}

#[derive(Clone, Copy, Component)]
pub(crate) enum DynamicLabel {
    HdriBackground,
    Enabled,
    Temperature,
    Shadows,
    ContactShadows,
    Imported,
    GlobalShadows,
    Fog,
    Advanced,
    ToneMap,
    Intensity,
    Gizmos,
}

#[derive(Clone, Copy, Component)]
pub(crate) enum Swatch {
    Light,
    Ambient,
    Fog,
}

fn help(commands: &mut Commands, button: Entity, label: &'static str) {
    commands.entity(button).insert(Help(label));
}

fn light_button(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    action: LightingAction,
    hint: &'static str,
) -> Entity {
    let button = button(
        commands,
        parent,
        font,
        label,
        Action::Lighting(action),
        Look::Regular,
    );
    help(commands, button, hint);
    button
}

fn dynamic_button(
    commands: &mut Commands,
    parent: Entity,
    font_handle: &Handle<Font>,
    label: &str,
    action: LightingAction,
    choice: Choice,
    dynamic: DynamicLabel,
    hint: &'static str,
) {
    let button = control(
        commands,
        parent,
        Action::Lighting(action),
        Look::Regular,
        Node {
            height: px(29),
            padding: UiRect::horizontal(px(8)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            overflow: Overflow::clip(),
            ..row(5.0)
        },
    );
    commands
        .entity(button)
        .insert((choice, Help(hint), AccessibleLabel(hint.into())));
    let label = text(commands, button, font_handle, label, 12.0, TEXT);
    commands.entity(label).insert(dynamic);
}

fn section(commands: &mut Commands, parent: Entity, font: &Handle<Font>, title: &str) {
    let line = node(
        commands,
        parent,
        Node {
            width: percent(100),
            margin: UiRect::top(px(7)),
            height: px(21),
            flex_shrink: 0.0,
            ..row(0.0)
        },
    );
    text(commands, line, font, title, 10.0, MUTED);
}

fn pane(commands: &mut Commands, parent: Entity, condition: VisibleWhen) -> Entity {
    let pane = node(
        commands,
        parent,
        Node {
            width: percent(100),
            ..column(5.0)
        },
    );
    commands.entity(pane).insert(condition);
    pane
}

fn wrap(commands: &mut Commands, parent: Entity, gap: f32) -> Entity {
    node(
        commands,
        parent,
        Node {
            width: percent(100),
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(gap),
            ..row(gap)
        },
    )
}

fn palette(commands: &mut Commands, parent: Entity) {
    let palette = node(
        commands,
        parent,
        Node {
            width: percent(100),
            ..row(6.0)
        },
    );
    for (name, rgb) in [
        ("Белый", [255, 255, 255]),
        ("Тёплый", [255, 194, 122]),
        ("Холодный", [155, 196, 255]),
        ("Красный", [255, 72, 55]),
        ("Зелёный", [100, 255, 156]),
        ("Синий", [72, 135, 255]),
        ("Фиолетовый", [190, 105, 255]),
    ] {
        let button = control(
            commands,
            palette,
            Action::Lighting(LightingAction::SetColor(rgb)),
            Look::Regular,
            Node {
                width: px(29),
                height: px(27),
                justify_content: JustifyContent::Center,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(5)),
                ..row(0.0)
            },
        );
        commands.entity(button).insert((
            AccessibleLabel(format!("Цвет света: {name}")),
            Help("Выбрать готовый цвет. Оттенок, насыщенность и яркость можно изменить в палитре выше."),
        ));
        let chip = surface(
            commands,
            button,
            Node {
                width: px(19),
                height: px(17),
                border_radius: BorderRadius::all(px(3)),
                ..default()
            },
            Color::srgb_u8(rgb[0], rgb[1], rgb[2]),
        );
        commands.entity(chip).insert(Pickable::IGNORE);
    }
}

fn light_icon(commands: &mut Commands, parent: Entity, kind: LightKind) {
    let icon = node(
        commands,
        parent,
        Node {
            width: px(32),
            height: px(31),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
    );
    commands.entity(icon).insert(Pickable::IGNORE);
    let glyph = match kind {
        LightKind::Point => icons::Glyph::Lightbulb,
        LightKind::Directional => icons::Glyph::Sun,
        LightKind::Spot => icons::Glyph::Spotlight,
        LightKind::Rect => icons::Glyph::Rectangle,
    };
    icons::spawn(commands, icon, glyph, 28.0, AMBER);
}

pub(crate) fn setup(
    commands: &mut Commands,
    tree_header: Entity,
    inspector: Entity,
    root: Entity,
    font_handle: &Handle<Font>,
    images: &mut Assets<Image>,
) {
    let add = button(
        commands,
        tree_header,
        font_handle,
        "Свет",
        Action::LightMenu,
        Look::Regular,
    );
    let plus = icons::themed(commands, add, icons::Glyph::Plus, 14.0);
    commands.entity(add).insert_children(0, &[plus]);
    commands.entity(add).insert((
        AccessibleLabel("+ Свет".into()),
        Choice::Menu,
        Help("Добавить источник света или применить готовую схему освещения сцены."),
    ));
    let menu = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: px(TOP),
            left: px(12),
            width: px(MENU_WIDTH),
            padding: UiRect::all(px(12)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(9)),
            overflow: Overflow::scroll_y(),
            ..column(9.0)
        },
        PANEL,
    );
    commands.entity(menu).insert((
        ZIndex(70),
        TabGroup::modal(),
        AccessibleLabel("Добавление света и схемы освещения".into()),
    ));
    text(commands, menu, font_handle, "Добавить свет", 17.0, TEXT);
    let kinds = node(
        commands,
        menu,
        Node {
            width: percent(100),
            ..row(8.0)
        },
    );
    let mut menu_first = add;
    for (label, kind, preset, hint) in [
        (
            "Точечный",
            LightKind::Point,
            LightPreset::Bulb,
            "Лампа светит во все стороны. Интенсивность измеряется в люменах.",
        ),
        (
            "Прожектор",
            LightKind::Spot,
            LightPreset::Spot,
            "Направленный конус света. Внутренний и внешний углы регулируются в инспекторе.",
        ),
        (
            "Солнце",
            LightKind::Directional,
            LightPreset::Sun,
            "Параллельные лучи без ограничения дальности. Освещённость измеряется в люксах.",
        ),
        (
            "Панель",
            LightKind::Rect,
            LightPreset::Softbox,
            "Прямоугольный источник с шириной и высотой. Панель не отбрасывает тени.",
        ),
    ] {
        let card = control(
            commands,
            kinds,
            Action::Lighting(LightingAction::Add(preset)),
            Look::Asset,
            Node {
                width: px(99),
                height: px(80),
                flex_shrink: 0.0,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(7)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..column(7.0)
            },
        );
        commands
            .entity(card)
            .insert((AccessibleLabel(format!("Добавить: {label}")), Help(hint)));
        if kind == LightKind::Point {
            menu_first = card;
        }
        light_icon(commands, card, kind);
        text(commands, card, font_handle, label, 12.0, TEXT);
    }
    section(commands, menu, font_handle, "ГОТОВЫЕ ИСТОЧНИКИ");
    let presets = wrap(commands, menu, 5.0);
    for preset in LightPreset::ALL {
        light_button(
            commands,
            presets,
            font_handle,
            preset.label(),
            LightingAction::Add(preset),
            "Добавить один готовый источник. Его цвет, яркость и положение можно изменить в инспекторе.",
        );
    }
    section(commands, menu, font_handle, "СХЕМЫ СЦЕНЫ");
    let schemes = wrap(commands, menu, 5.0);
    for preset in LightingPreset::ALL {
        light_button(
            commands,
            schemes,
            font_handle,
            preset.label(),
            LightingAction::ApplyPreset(preset),
            "Применить готовый набор света и настройки окружения. Отмена возвращает предыдущее освещение.",
        );
    }
    let environment = button(
        commands,
        menu,
        font_handle,
        "Окружение, HDRI и экспозиция…",
        Action::Select(Selection::Environment),
        Look::Regular,
    );
    help(
        commands,
        environment,
        "Открыть настройки HDRI, отражений, фонового света, экспозиции, теней и тумана.",
    );
    dynamic_button(
        commands,
        menu,
        font_handle,
        "Значки света: вкл.",
        LightingAction::ToggleGizmos,
        Choice::Gizmos,
        DynamicLabel::Gizmos,
        "Показывать значки и контуры источников в редакторе. Это настройка интерфейса, она не меняет освещение сцены.",
    );
    text(
        commands,
        menu,
        font_handle,
        "Esc или клик снаружи — закрыть",
        11.0,
        MUTED,
    );

    let panel = node(
        commands,
        inspector,
        Node {
            display: Display::None,
            width: percent(100),
            ..column(6.0)
        },
    );
    let light = pane(commands, panel, VisibleWhen::Light);
    let name = commands
        .spawn((
            NameInput,
            TextInput,
            EditableText::new(""),
            TextCursorStyle::default(),
            AccessibleLabel("Имя источника света".into()),
            font(font_handle, 13.0),
            TextColor(TEXT),
            TextLayout::no_wrap(),
            TabIndex(0),
            BackgroundColor(WELL),
            BorderColor::all(EDGE),
            ChildOf(light),
            Node {
                width: percent(100),
                height: px(29),
                flex_shrink: 0.0,
                padding: UiRect::axes(px(7), px(5)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(5)),
                overflow: Overflow::clip(),
                ..default()
            },
        ))
        .id();
    let kinds = wrap(commands, light, 3.0);
    for (label, kind) in [
        ("Точка", LightKind::Point),
        ("Конус", LightKind::Spot),
        ("Солнце", LightKind::Directional),
        ("Панель", LightKind::Rect),
    ] {
        let choice = light_button(
            commands,
            kinds,
            font_handle,
            label,
            LightingAction::SetKind(kind),
            "Изменить тип выбранного источника, сохранив его положение и имя.",
        );
        commands.entity(choice).insert(Choice::Kind(kind));
    }
    dynamic_button(
        commands,
        light,
        font_handle,
        "Свет включён",
        LightingAction::ToggleEnabled,
        Choice::Enabled,
        DynamicLabel::Enabled,
        "Включить или выключить источник, сохранив все его настройки.",
    );
    let intensity = scalar(
        commands,
        light,
        font_handle,
        "Интенсивность",
        Field::Light(LightField::Intensity),
        "лм",
    );
    commands.entity(intensity).insert(DynamicLabel::Intensity);
    let local = pane(commands, light, VisibleWhen::Local);
    scalar(
        commands,
        local,
        font_handle,
        "Дальность",
        Field::Light(LightField::Range),
        "м",
    );
    section(commands, light, font_handle, "ЦВЕТ");
    color_editor(commands, light, font_handle, Swatch::Light, images);
    palette(commands, light);
    dynamic_button(
        commands,
        light,
        font_handle,
        "Температура: выкл.",
        LightingAction::ToggleTemperature,
        Choice::Temperature,
        DynamicLabel::Temperature,
        "Рассчитать цвет по температуре в Кельвинах. Тёплая лампа — около 2700 K, дневной свет — 6500 K.",
    );
    let temperature = pane(commands, light, VisibleWhen::Temperature);
    scalar(
        commands,
        temperature,
        font_handle,
        "Температура",
        Field::Light(LightField::Temperature),
        "K",
    );
    let kelvin = wrap(commands, temperature, 3.0);
    for (label, value) in [("2700 K", 2700.0), ("4000 K", 4000.0), ("6500 K", 6500.0)] {
        light_button(
            commands,
            kelvin,
            font_handle,
            label,
            LightingAction::SetNumber(Field::Light(LightField::Temperature), value),
            "Быстро задать цветовую температуру выбранного источника.",
        );
    }
    let radius = pane(commands, light, VisibleWhen::Radius);
    scalar(
        commands,
        radius,
        font_handle,
        "Радиус источника",
        Field::Light(LightField::Radius),
        "м",
    );
    let spot = pane(commands, light, VisibleWhen::Spot);
    scalar(
        commands,
        spot,
        font_handle,
        "Внутренний угол",
        Field::Light(LightField::InnerAngle),
        "°",
    );
    scalar(
        commands,
        spot,
        font_handle,
        "Внешний угол",
        Field::Light(LightField::OuterAngle),
        "°",
    );
    let rect = pane(commands, light, VisibleWhen::Rect);
    scalar(
        commands,
        rect,
        font_handle,
        "Ширина панели",
        Field::Light(LightField::Width),
        "м",
    );
    scalar(
        commands,
        rect,
        font_handle,
        "Высота панели",
        Field::Light(LightField::Height),
        "м",
    );
    let notice = text(
        commands,
        rect,
        font_handle,
        "Панель не отбрасывает тени",
        11.0,
        MUTED,
    );
    commands.entity(notice).insert(TextLayout::default());
    let shadows = pane(commands, light, VisibleWhen::NonRect);
    dynamic_button(
        commands,
        shadows,
        font_handle,
        "Тени включены",
        LightingAction::ToggleShadows,
        Choice::Shadows,
        DynamicLabel::Shadows,
        "Включить тени для выбранного источника. Тени требуют дополнительного времени GPU.",
    );
    let placement = wrap(commands, light, 4.0);
    light_button(
        commands,
        placement,
        font_handle,
        "Из вида",
        LightingAction::PlaceAtView,
        "Перенести источник в положение свободной камеры и скопировать её направление.",
    );
    light_button(
        commands,
        placement,
        font_handle,
        "Направить из вида",
        LightingAction::AimAtView,
        "Скопировать направление свободной камеры, сохранив позицию источника.",
    );

    let environment = pane(commands, panel, VisibleWhen::Environment);
    section(commands, environment, font_handle, "HDRI · ОКРУЖЕНИЕ");
    button(
        commands,
        environment,
        font_handle,
        "Скачать HDRI…",
        Action::HdriLibrary(super::HdriAction::Open),
        Look::Regular,
    );
    let environments = wrap(commands, environment, 4.0);
    for preset in std::iter::once(None).chain(EnvironmentPreset::ALL.into_iter().map(Some)) {
        let label = preset.map_or("Выкл.", EnvironmentPreset::label);
        let choice = light_button(
            commands,
            environments,
            font_handle,
            label,
            LightingAction::SetEnvironment(preset),
            "Выбрать панораму для фона, освещения и отражений на материалах.",
        );
        commands.entity(choice).insert((
            Choice::Environment(preset),
            AccessibleLabel(format!("HDRI: {label}")),
        ));
    }
    let downloaded = node(
        commands,
        environment,
        Node {
            width: percent(100),
            ..column(4.0)
        },
    );
    commands
        .entity(downloaded)
        .insert(super::hdri_library::DownloadedList);
    let hdri = pane(commands, environment, VisibleWhen::Hdri);
    dynamic_button(
        commands,
        hdri,
        font_handle,
        "Панорама на фоне: вкл.",
        LightingAction::ToggleHdriBackground,
        Choice::HdriBackground,
        DynamicLabel::HdriBackground,
        "Показывать выбранную панораму вдали. При скрытом фоне HDRI продолжает освещать сцену и отражаться в материалах.",
    );
    scalar(
        commands,
        hdri,
        font_handle,
        "Яркость HDRI",
        Field::Environment(EnvironmentField::HdriBrightness),
        "кд/м²",
    );
    scalar(
        commands,
        hdri,
        font_handle,
        "Поворот HDRI",
        Field::Environment(EnvironmentField::HdriRotation),
        "°",
    );
    section(commands, environment, font_handle, "ФОНОВЫЙ СВЕТ");
    color_editor(commands, environment, font_handle, Swatch::Ambient, images);
    palette(commands, environment);
    scalar(
        commands,
        environment,
        font_handle,
        "Яркость фонового света",
        Field::Environment(EnvironmentField::AmbientBrightness),
        "",
    );
    section(commands, environment, font_handle, "КАМЕРА");
    scalar(
        commands,
        environment,
        font_handle,
        "Экспозиция",
        Field::Environment(EnvironmentField::Exposure),
        "EV",
    );
    let exposures = wrap(commands, environment, 4.0);
    for (label, value) in [("Интерьер", 8.0), ("День", 12.0), ("Солнце", 15.0)] {
        light_button(
            commands,
            exposures,
            font_handle,
            label,
            LightingAction::SetNumber(Field::Environment(EnvironmentField::Exposure), value),
            "Задать экспозицию камеры. Большее EV делает изображение темнее.",
        );
    }
    let tonemap = control(
        commands,
        environment,
        Action::Lighting(LightingAction::CycleToneMapping),
        Look::Regular,
        Node {
            width: percent(100),
            height: px(29),
            padding: UiRect::horizontal(px(8)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            overflow: Overflow::clip(),
            ..row(0.0)
        },
    );
    help(
        commands,
        tonemap,
        "Переключить преобразование HDR-яркости в цвет изображения.",
    );
    let tonemap_caption = text(
        commands,
        tonemap,
        font_handle,
        "Тонмаппинг: AgX",
        12.0,
        TEXT,
    );
    commands
        .entity(tonemap_caption)
        .insert(DynamicLabel::ToneMap);
    section(commands, environment, font_handle, "СЦЕНА");
    dynamic_button(
        commands,
        environment,
        font_handle,
        "Свет из GLB: вкл.",
        LightingAction::ToggleImportedLights,
        Choice::Imported,
        DynamicLabel::Imported,
        "Показывать или скрывать источники, импортированные из GLB. Их исходные настройки сохраняются.",
    );
    dynamic_button(
        commands,
        environment,
        font_handle,
        "Тени сцены: вкл.",
        LightingAction::ToggleGlobalShadows,
        Choice::GlobalShadows,
        DynamicLabel::GlobalShadows,
        "Глобально включить или выключить все тени сцены.",
    );
    dynamic_button(
        commands,
        environment,
        font_handle,
        "Туман: выкл.",
        LightingAction::ToggleFog,
        Choice::Fog,
        DynamicLabel::Fog,
        "Добавить линейный туман между начальной и конечной дистанциями.",
    );
    let fog = pane(commands, environment, VisibleWhen::Fog);
    color_editor(commands, fog, font_handle, Swatch::Fog, images);
    scalar(
        commands,
        fog,
        font_handle,
        "Начало тумана",
        Field::Environment(EnvironmentField::FogStart),
        "м",
    );
    scalar(
        commands,
        fog,
        font_handle,
        "Конец тумана",
        Field::Environment(EnvironmentField::FogEnd),
        "м",
    );

    let advanced_button = control(
        commands,
        panel,
        Action::LightAdvanced,
        Look::Regular,
        Node {
            width: percent(100),
            height: px(29),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            ..row(8.0)
        },
    );
    commands.entity(advanced_button).insert((
        AccessibleLabel("Дополнительно".into()),
        Choice::Advanced,
        Help("Раскрыть дополнительные настройки теней и качества освещения."),
    ));
    let advanced_icon = icons::spawn(
        commands,
        advanced_button,
        icons::Glyph::ChevronDown,
        14.0,
        MUTED,
    );
    commands.entity(advanced_icon).insert(AdvancedIcon(false));
    let advanced_label = text(
        commands,
        advanced_button,
        font_handle,
        "Дополнительно",
        12.0,
        MUTED,
    );
    commands
        .entity(advanced_label)
        .insert(DynamicLabel::Advanced);
    let advanced_light = pane(commands, panel, VisibleWhen::AdvancedNonRect);
    dynamic_button(
        commands,
        advanced_light,
        font_handle,
        "Контактные тени: вкл.",
        LightingAction::ToggleContactShadows,
        Choice::ContactShadows,
        DynamicLabel::ContactShadows,
        "Дополнительные короткие тени в местах соприкосновения объектов.",
    );
    scalar(
        commands,
        advanced_light,
        font_handle,
        "Смещение глубины",
        Field::Light(LightField::ShadowDepthBias),
        "",
    );
    scalar(
        commands,
        advanced_light,
        font_handle,
        "Смещение нормали",
        Field::Light(LightField::ShadowNormalBias),
        "",
    );
    let advanced_local = pane(commands, advanced_light, VisibleWhen::AdvancedLocal);
    scalar(
        commands,
        advanced_local,
        font_handle,
        "Ближняя тень",
        Field::Light(LightField::ShadowNear),
        "м",
    );
    let advanced_directional = pane(commands, advanced_light, VisibleWhen::AdvancedDirectional);
    scalar(
        commands,
        advanced_directional,
        font_handle,
        "Дальность теней",
        Field::Light(LightField::ShadowDistance),
        "м",
    );
    let advanced_env = pane(commands, panel, VisibleWhen::AdvancedEnvironment);
    section(commands, advanced_env, font_handle, "РАЗРЕШЕНИЕ ТЕНЕЙ");
    let resolutions = wrap(commands, advanced_env, 4.0);
    for resolution in [512, 1024, 2048, 4096] {
        let choice = light_button(
            commands,
            resolutions,
            font_handle,
            &resolution.to_string(),
            LightingAction::SetShadowResolution(resolution),
            "Разрешение карт теней. Более высокое значение улучшает детали и увеличивает расход памяти GPU.",
        );
        commands
            .entity(choice)
            .insert(Choice::Resolution(resolution));
    }
    light_button(
        commands,
        advanced_env,
        font_handle,
        "Сбросить окружение",
        LightingAction::ResetEnvironment,
        "Вернуть настройки окружения по умолчанию. Источники света остаются на месте.",
    );

    let tooltip = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: px(300),
            padding: UiRect::all(px(9)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        WELL,
    );
    commands
        .entity(tooltip)
        .insert((ZIndex(110), Pickable::IGNORE));
    let tooltip_label = text(commands, tooltip, font_handle, "", 12.0, TEXT);
    commands.entity(tooltip_label).insert(TextLayout::default());
    commands.insert_resource(LightingUi {
        add,
        menu,
        menu_first,
        inspector: panel,
        name,
        tooltip,
        tooltip_label,
        menu_open: false,
        advanced: false,
        pointer_consumed: false,
        controls: controls::State::default(),
        name_draft: None,
        hint_target: None,
        hint_elapsed: 0.0,
    });
}

fn contains(
    entity: Entity,
    cursor: Vec2,
    geometry: &Query<(&ComputedNode, &UiGlobalTransform)>,
) -> bool {
    geometry.get(entity).is_ok_and(|(computed, transform)| {
        computed.size().min_element() > 0.0 && computed.contains_point(*transform, cursor)
    })
}

pub(crate) fn edit_name(
    mut editor: ResMut<Editor>,
    mut ui: ResMut<LightingUi>,
    mut focus: ResMut<InputFocus>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut actions: MessageReader<Action>,
    mut close: MessageReader<WindowCloseRequested>,
    mut inputs: Query<&mut EditableText, With<NameInput>>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    mut offsets: Query<&mut ScrollPosition>,
    parents: Query<&ChildOf>,
) {
    ui.pointer_consumed = false;
    let was_open = ui.menu_open;
    let mut force_commit = close.read().last().is_some();
    for action in actions.read() {
        force_commit = true;
        match *action {
            Action::LightMenu => {
                ui.menu_open = !ui.menu_open;
            }
            Action::LightAdvanced => {
                ui.advanced = !ui.advanced;
            }
            Action::Lighting(LightingAction::Add(_) | LightingAction::ApplyPreset(_))
            | Action::Select(Selection::Environment) => {
                ui.menu_open = false;
            }
            _ => {}
        }
    }
    if !was_open && ui.menu_open {
        focus.set(ui.menu_first, FocusCause::Navigated);
    }
    if ui.menu_open
        && (keys.just_pressed(KeyCode::Escape)
            || editor.pending.is_some()
            || editor.asset_viewer_open()
            || editor.browser_popup
            || !editor.has_scene())
    {
        ui.menu_open = false;
        ui.pointer_consumed = true;
    }
    if ui.menu_open
        && let Some(cursor) = window.cursor_position()
    {
        let physical = cursor * window.scale_factor();
        if mouse.any_just_pressed([MouseButton::Left, MouseButton::Right, MouseButton::Middle])
            && !contains(ui.menu, physical, &geometry)
            && !contains(ui.add, physical, &geometry)
        {
            ui.menu_open = false;
            ui.pointer_consumed = true;
        } else if contains(ui.menu, physical, &geometry)
            && scroll.delta.y != 0.0
            && let Ok((computed, _)) = geometry.get(ui.menu)
            && let Ok(mut position) = offsets.get_mut(ui.menu)
        {
            let amount = if scroll.unit == MouseScrollUnit::Line {
                33.0
            } else {
                1.0
            };
            let max = ((computed.content_size().y - computed.size().y)
                * computed.inverse_scale_factor())
            .max(0.0);
            position.y = (position.y - scroll.delta.y * amount).clamp(0.0, max);
        }
    }
    if was_open && !ui.menu_open {
        let menu_focused = focus.get().is_some_and(|entity| {
            entity == ui.menu
                || parents
                    .iter_ancestors(entity)
                    .any(|parent| parent == ui.menu)
        });
        if keys.just_pressed(KeyCode::Escape) && editor.pending.is_none() {
            focus.set(ui.add, FocusCause::Navigated);
        } else if menu_focused {
            focus.clear();
        }
    }
    let Ok(mut input) = inputs.get_mut(ui.name) else {
        return;
    };
    let focused = focus.get() == Some(ui.name);
    if focused
        && ui.name_draft.is_none()
        && let Some(Selection::Light(id)) = editor.selection
        && let Some(light) = editor.light(id)
    {
        ui.name_draft = Some(NameDraft {
            room: editor.room,
            light: id,
            kind: light.kind,
            original: light.name.clone(),
            value: input.value().to_string(),
        });
    }
    if let Some(draft) = &mut ui.name_draft {
        draft.value = input.value().to_string();
    }
    if keys.just_pressed(KeyCode::Escape) && ui.name_draft.is_some() {
        if let Some(draft) = ui.name_draft.take() {
            input.editor_mut().set_text(&draft.original);
        }
        focus.clear();
        return;
    }
    let enter = focused && keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]);
    if (!focused || force_commit || enter)
        && let Some(draft) = ui.name_draft.take()
    {
        if draft.room == editor.room {
            let name = draft.clean_name();
            if name != draft.original && editor.light(draft.light).is_some() {
                editor.finish_camera_navigation();
                let before = editor.edits.clone();
                if let Some(light) = editor
                    .edits
                    .lights
                    .iter_mut()
                    .find(|light| light.id == draft.light)
                {
                    light.name = name.clone();
                }
                editor.finish_edit(before);
                editor.status = format!("Источник переименован · {name}");
            }
            input.editor_mut().set_text(&name);
        }
        if enter {
            focus.clear();
        }
    }
    if !focused && ui.name_draft.is_none() {
        let name = match editor.selection {
            Some(Selection::Light(id)) => editor.light(id).map(|light| light.name.as_str()),
            _ => None,
        };
        if let Some(name) = name
            && input.value().to_string() != name
        {
            input.editor_mut().set_text(name);
        }
    }
}

pub(crate) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    mut ui: ResMut<LightingUi>,
    time: Res<Time>,
    focus: Res<InputFocus>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut nodes: Query<&mut Node>,
    conditions: Query<(Entity, &VisibleWhen)>,
    mut labels: Query<(Entity, &mut Text, Option<&DynamicLabel>)>,
    mut colors: Query<
        (Option<&Choice>, Option<&Swatch>, &mut BackgroundColor),
        Or<(With<Choice>, With<Swatch>)>,
    >,
    hints: Query<(Entity, &Help, &Hovered)>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    mut advanced_icons: Query<(Entity, &mut AdvancedIcon)>,
) {
    for (entity, mut icon) in &mut advanced_icons {
        if icon.0 != ui.advanced {
            icon.0 = ui.advanced;
            icons::set_glyph(
                &mut commands,
                entity,
                if ui.advanced {
                    icons::Glyph::ChevronUp
                } else {
                    icons::Glyph::ChevronDown
                },
            );
        }
    }
    let light = match editor.selection {
        Some(Selection::Light(id)) => editor.light(id),
        _ => None,
    };
    let environment_selected =
        editor.has_scene() && editor.selection == Some(Selection::Environment);
    let environment = editor.environment();
    let show = editor.has_scene()
        && editor.viewing_model.is_none()
        && (light.is_some() || environment_selected);
    if let Ok(mut panel) = nodes.get_mut(ui.inspector) {
        panel.display = if show { Display::Flex } else { Display::None };
    }
    if let Ok(mut menu) = nodes.get_mut(ui.menu) {
        menu.display = if ui.menu_open {
            Display::Flex
        } else {
            Display::None
        };
        if let Ok((computed, transform)) = geometry.get(ui.add) {
            let size = computed.size() / window.scale_factor();
            let anchor = transform.affine().translation / window.scale_factor() - size * 0.5;
            let top = anchor.y + size.y + 6.0;
            menu.left = px((anchor.x + size.x - MENU_WIDTH)
                .clamp(8.0, (window.width() - MENU_WIDTH - 8.0).max(8.0)));
            menu.top = px(top);
            menu.max_height = px((window.height() - FOOT - 12.0 - top).max(0.0));
        }
    }
    for (entity, condition) in &conditions {
        let visible = match *condition {
            VisibleWhen::Light => light.is_some(),
            VisibleWhen::Environment => environment_selected,
            VisibleWhen::Hdri => environment_selected && environment.has_hdri(),
            VisibleWhen::Local => light.is_some_and(|light| light.kind != LightKind::Directional),
            VisibleWhen::Radius => {
                light.is_some_and(|light| matches!(light.kind, LightKind::Point | LightKind::Spot))
            }
            VisibleWhen::Spot => light.is_some_and(|light| light.kind == LightKind::Spot),
            VisibleWhen::Rect => light.is_some_and(|light| light.kind == LightKind::Rect),
            VisibleWhen::NonRect => light.is_some_and(|light| light.kind != LightKind::Rect),
            VisibleWhen::Temperature => light.is_some_and(|light| light.temperature.is_some()),
            VisibleWhen::Fog => environment_selected && environment.fog.is_some(),
            VisibleWhen::AdvancedEnvironment => ui.advanced && environment_selected,
            VisibleWhen::AdvancedNonRect => {
                ui.advanced && light.is_some_and(|light| light.kind != LightKind::Rect)
            }
            VisibleWhen::AdvancedLocal => {
                ui.advanced
                    && light.is_some_and(|light| {
                        matches!(light.kind, LightKind::Point | LightKind::Spot)
                    })
            }
            VisibleWhen::AdvancedDirectional => {
                ui.advanced && light.is_some_and(|light| light.kind == LightKind::Directional)
            }
        };
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.display = if visible {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    for (_, mut text, label) in &mut labels {
        let Some(label) = label else {
            continue;
        };
        let value = match *label {
            DynamicLabel::HdriBackground => if environment.hdri_background {
                "Панорама на фоне: вкл."
            } else {
                "Панорама на фоне: выкл."
            }
            .into(),
            DynamicLabel::Enabled => if light.is_some_and(|light| light.enabled) {
                "Свет включён"
            } else {
                "Свет выключен"
            }
            .into(),
            DynamicLabel::Temperature => {
                if light.is_some_and(|light| light.temperature.is_some()) {
                    "Температура: вкл."
                } else {
                    "Температура: выкл."
                }
                .into()
            }
            DynamicLabel::Shadows => if light.is_some_and(|light| light.shadows) {
                "Тени: вкл."
            } else {
                "Тени: выкл."
            }
            .into(),
            DynamicLabel::ContactShadows => if light.is_some_and(|light| light.contact_shadows) {
                "Контактные тени: вкл."
            } else {
                "Контактные тени: выкл."
            }
            .into(),
            DynamicLabel::Imported => if environment.scene_lights {
                "Свет из GLB: вкл."
            } else {
                "Свет из GLB: выкл."
            }
            .into(),
            DynamicLabel::GlobalShadows => if environment.shadows {
                "Тени сцены: вкл."
            } else {
                "Тени сцены: выкл."
            }
            .into(),
            DynamicLabel::Fog => if environment.fog.is_some() {
                "Туман: вкл."
            } else {
                "Туман: выкл."
            }
            .into(),
            DynamicLabel::Advanced => "Дополнительно".into(),
            DynamicLabel::ToneMap => format!("Тонмаппинг: {}", environment.tonemapping.label()),
            DynamicLabel::Gizmos => if editor.show_lights {
                "Значки света: вкл."
            } else {
                "Значки света: выкл."
            }
            .into(),
            DynamicLabel::Intensity => {
                if light.is_some_and(|light| light.kind == LightKind::Directional) {
                    "лк"
                } else {
                    "лм"
                }
                .into()
            }
        };
        if text.0 != value {
            text.0 = value;
        }
    }
    for (choice, swatch, mut color) in &mut colors {
        if let Some(choice) = choice {
            let active = match *choice {
                Choice::Kind(kind) => light.is_some_and(|light| light.kind == kind),
                Choice::Environment(preset) => {
                    environment.hdri == preset && environment.hdri_asset.is_none()
                }
                Choice::HdriBackground => environment.hdri_background,
                Choice::Enabled => light.is_some_and(|light| light.enabled),
                Choice::Temperature => light.is_some_and(|light| light.temperature.is_some()),
                Choice::Shadows => light.is_some_and(|light| light.shadows),
                Choice::ContactShadows => light.is_some_and(|light| light.contact_shadows),
                Choice::Imported => environment.scene_lights,
                Choice::GlobalShadows => environment.shadows,
                Choice::Fog => environment.fog.is_some(),
                Choice::Resolution(value) => environment.shadow_resolution == value,
                Choice::Advanced => ui.advanced,
                Choice::Menu => ui.menu_open,
                Choice::Gizmos => editor.show_lights,
            };
            if active {
                color.set_if_neq(BackgroundColor(ACTIVE));
            }
        } else if let Some(swatch) = swatch {
            let rgb = match *swatch {
                Swatch::Light => light.map_or([1.0; 3], |light| light.resolved_rgb()),
                Swatch::Ambient => environment.ambient_color,
                Swatch::Fog => environment.fog.as_ref().map_or([0.5; 3], |fog| fog.color),
            };
            color.set_if_neq(BackgroundColor(Color::srgb(rgb[0], rgb[1], rgb[2])));
        }
    }
    let hovered = hints
        .iter()
        .find(|(entity, _, hovered)| hovered.0 || focus.get() == Some(*entity))
        .map(|(entity, help, _)| (entity, help.0));
    let target = hovered.map(|(entity, _)| entity);
    if target == ui.hint_target {
        ui.hint_elapsed += time.delta_secs();
    } else {
        ui.hint_target = target;
        ui.hint_elapsed = 0.0;
    }
    let hint = hovered
        .filter(|_| ui.hint_elapsed >= 0.45 && editor.pending.is_none() && !ui.is_control_active());
    if let Ok(mut tooltip) = nodes.get_mut(ui.tooltip) {
        tooltip.display = if hint.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if let Some(cursor) = window.cursor_position() {
            tooltip.left = px((cursor.x + 14.0).clamp(8.0, (window.width() - 308.0).max(8.0)));
            tooltip.top =
                px((cursor.y + 20.0).clamp(TOP, (window.height() - FOOT - 100.0).max(TOP)));
        }
    }
    if let Some((_, hint)) = hint
        && let Ok((_, mut text, _)) = labels.get_mut(ui.tooltip_label)
        && text.0 != hint
    {
        text.0 = hint.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_app() -> (App, Entity, Entity) {
        let project = crate::project::EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        let mut editor = Editor::new(project, crate::data::LevelEdits::default());
        editor.edits.lights.push(LightPreset::Bulb.create(1));
        editor.selection = Some(Selection::Light(1));
        editor.ready = true;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(editor)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseScroll>()
            .add_message::<Action>()
            .add_message::<WindowCloseRequested>()
            .add_systems(Update, edit_name);
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: (1280, 760).into(),
                    focused: true,
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        let add = app.world_mut().spawn_empty().id();
        let menu = app.world_mut().spawn_empty().id();
        let first = app.world_mut().spawn(ChildOf(menu)).id();
        let panel = app.world_mut().spawn_empty().id();
        let name = app
            .world_mut()
            .spawn((
                NameInput,
                TextInput,
                EditableText::new(LightPreset::Bulb.label()),
            ))
            .id();
        app.insert_resource(InputFocus::from_entity(name))
            .insert_resource(LightingUi {
                add,
                menu,
                menu_first: first,
                inspector: panel,
                name,
                tooltip: panel,
                tooltip_label: panel,
                menu_open: false,
                advanced: false,
                pointer_consumed: false,
                controls: controls::State::default(),
                name_draft: None,
                hint_target: None,
                hint_elapsed: 0.0,
            });
        app.update();
        (app, name, window)
    }

    #[test]
    fn name_drafts_commit_once_on_blur_enter_save_or_window_close() {
        for trigger in 0..4 {
            let (mut app, name, window) = name_app();
            app.world_mut()
                .get_mut::<EditableText>(name)
                .unwrap()
                .editor_mut()
                .set_text("  Новая лампа  ");
            app.update();
            assert!(
                app.world()
                    .resource::<LightingUi>()
                    .name_has_changes(app.world().resource::<Editor>())
            );
            assert_eq!(
                app.world().resource::<Editor>().light(1).unwrap().name,
                LightPreset::Bulb.label()
            );
            match trigger {
                0 => app.world_mut().resource_mut::<InputFocus>().clear(),
                1 => app
                    .world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::Enter),
                2 => {
                    app.world_mut().write_message(Action::Save);
                }
                _ => {
                    app.world_mut()
                        .write_message(WindowCloseRequested { window });
                }
            }
            app.update();
            assert_eq!(
                app.world().resource::<Editor>().light(1).unwrap().name,
                "Новая лампа"
            );
            assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
            app.update();
            assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
        }
    }

    #[test]
    fn escape_cancels_a_name_draft_and_closes_the_menu_without_clickthrough() {
        let (mut app, name, _) = name_app();
        app.world_mut()
            .get_mut::<EditableText>(name)
            .unwrap()
            .editor_mut()
            .set_text("Не сохранять");
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        assert_eq!(
            app.world().resource::<Editor>().light(1).unwrap().name,
            LightPreset::Bulb.label()
        );
        assert!(app.world().resource::<Editor>().undo.is_empty());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut().write_message(Action::LightMenu);
        app.update();
        assert!(app.world().resource::<LightingUi>().is_menu_open());
        let first = app.world().resource::<LightingUi>().menu_first;
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(first));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        let ui = app.world().resource::<LightingUi>();
        assert!(!ui.is_menu_open());
        assert!(ui.blocks_scene_input());
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(ui.add));
    }

    #[test]
    fn empty_or_long_names_become_valid_scene_names() {
        let mut draft = NameDraft {
            room: 0,
            light: 1,
            kind: LightKind::Rect,
            original: "Панель".into(),
            value: "\n\t  ".into(),
        };
        assert_eq!(draft.clean_name(), LightKind::Rect.label());
        draft.value = "Ж".repeat(150);
        assert_eq!(draft.clean_name().chars().count(), 128);
    }
}
