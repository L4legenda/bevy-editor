//! Direct lighting controls: HSV color planes and undoable, live sliders.
use super::*;
use bevy::{
    asset::RenderAssetUsages,
    color::{Hsva, Srgba},
    input::{ButtonState, keyboard::KeyboardInput},
    input_focus::FocusedInput,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    ui::{CalculatedClip, widget::NodeImageMode},
    ui_widgets::{
        Slider, SliderOrientation, SliderPrecision, SliderRange, SliderStep, SliderThumb,
        SliderValue, TrackClick, ValueChange,
    },
};

const THUMB: f32 = 14.0;
const PLANE_WIDTH: u32 = 256;
const PLANE_HEIGHT: u32 = 128;

#[cfg(test)]
#[path = "lighting_controls_tests.rs"]
mod tests;

#[derive(Clone, Copy)]
enum Binding {
    Number(Field),
    Hue(Entity),
}

#[derive(Component)]
pub(in crate::editor) struct LightingSlider {
    binding: Binding,
    thumb: Entity,
    fill: Option<Entity>,
    readout: Option<Entity>,
}

#[derive(Component)]
pub(in crate::editor) struct ColorPicker {
    swatch: Swatch,
    hsv: Hsva,
    last_rgb: [f32; 3],
    context: Option<(Room, Selection)>,
    marker: Entity,
    chip: Entity,
    code: Entity,
    image: Handle<Image>,
    drawn_hue: f32,
}

impl ColorPicker {
    fn sync(&mut self, editor: &Editor) {
        let context = editor.selection.map(|selection| (editor.room, selection));
        let rgb = color_value(editor, self.swatch);
        if self.context != context || self.last_rgb != rgb {
            let mut hsv = Hsva::from(Srgba::rgb(rgb[0], rgb[1], rgb[2]));
            // A white or black color has no hue, but the user's chosen hue
            // must survive until they increase saturation or brightness.
            if self.context == context && (hsv.saturation < 1e-6 || hsv.value < 1e-6) {
                hsv.hue = self.hsv.hue;
            }
            self.hsv = hsv;
            self.last_rgb = rgb;
            self.context = context;
        }
    }
}

#[derive(Clone, Copy)]
enum Change {
    Slider(f32),
    Plane(Vec2),
}

struct Queued {
    source: Entity,
    context: Option<(Room, Selection)>,
    change: Change,
    final_value: bool,
}

struct Drag {
    room: Room,
    selection: Selection,
    source: Entity,
    plane: bool,
    before: crate::editor::RoomEdits,
    picker: Option<(Entity, Hsva)>,
}

#[derive(Default)]
pub(super) struct State {
    queued: Vec<Queued>,
    drag: Option<Drag>,
    suppressed: bool,
}

impl State {
    pub fn active(&self) -> bool {
        self.drag.is_some()
    }

    fn finish(&mut self, editor: &mut Editor) {
        if let Some(drag) = self.drag.take()
            && drag.room == editor.room
        {
            editor.finish_edit(drag.before);
        }
    }

    fn begin(
        &mut self,
        editor: &mut Editor,
        source: Entity,
        plane: bool,
        picker: Option<(Entity, Hsva)>,
    ) {
        if self.drag.as_ref().is_some_and(|drag| drag.source == source) {
            return;
        }
        self.finish(editor);
        let Some(selection) = editor.selection else {
            return;
        };
        editor.finish_camera_navigation();
        self.drag = Some(Drag {
            room: editor.room,
            selection,
            source,
            plane,
            before: editor.edits.clone(),
            picker,
        });
    }
}

#[derive(Clone, Copy)]
struct Limits {
    min: f32,
    max: f32,
    logarithmic: bool,
}

impl Limits {
    fn position(self, value: f32) -> f32 {
        let span = self.max - self.min;
        if span <= 0.0 {
            return 0.0;
        }
        let value = (value - self.min).clamp(0.0, span);
        if self.logarithmic {
            value.ln_1p() / span.ln_1p()
        } else {
            value / span
        }
    }

    fn value(self, position: f32) -> f32 {
        let span = (self.max - self.min).max(0.0);
        let t = position.clamp(0.0, 1.0);
        let value = if self.logarithmic {
            (span.ln_1p() * t).exp_m1()
        } else {
            span * t
        };
        (self.min + value).clamp(self.min, self.max)
    }
}

fn limits(editor: &Editor, field: Field) -> Option<Limits> {
    use EnvironmentField as E;
    use LightField as L;
    let (min, max, logarithmic) = match field {
        Field::Environment(field) => {
            let fog = editor.environment().fog.as_ref();
            match field {
                E::AmbientBrightness => (0.0, 100_000.0, true),
                E::Exposure => (-10.0, 24.0, false),
                E::HdriBrightness => (0.0, 100_000.0, true),
                E::HdriRotation => (-180.0, 180.0, false),
                E::FogStart => (
                    0.0,
                    fog.map_or(100_000.0, |fog| (fog.end - 0.01).min(100_000.0)),
                    true,
                ),
                E::FogEnd => (fog.map_or(0.01, |fog| fog.start + 0.01), 1_000_000.0, true),
            }
        }
        Field::Light(field) => {
            let Some(Selection::Light(id)) = editor.selection else {
                return None;
            };
            let light = editor.light(id)?;
            match field {
                L::Intensity => (0.0, 1.0e9, true),
                L::Range => (
                    if matches!(light.kind, LightKind::Point | LightKind::Spot) {
                        0.01_f32.max(light.radius).max(light.shadow_near + 0.001)
                    } else {
                        0.01
                    },
                    10_000.0,
                    true,
                ),
                L::Radius => (0.0, light.range.min(1000.0), true),
                L::Width | L::Height => (0.01, 100.0, true),
                L::InnerAngle => (0.0, light.outer_angle, false),
                L::OuterAngle => (light.inner_angle.max(0.1), 89.9, false),
                L::Temperature => (1000.0, 40_000.0, false),
                L::ShadowDepthBias => (0.0, 10.0, true),
                L::ShadowNormalBias => (0.0, 100.0, true),
                L::ShadowNear => (0.001, (light.range - 0.001).min(1000.0), true),
                L::ShadowDistance => ((light.shadow_near + 0.001).max(0.01), 10_000.0, true),
            }
        }
        _ => return None,
    };
    Some(Limits {
        min,
        max: max.max(min),
        logarithmic,
    })
}

fn color_value(editor: &Editor, swatch: Swatch) -> [f32; 3] {
    match swatch {
        Swatch::Light => match editor.selection {
            Some(Selection::Light(id)) => editor
                .light(id)
                .map_or([1.0; 3], |light| light.resolved_rgb()),
            _ => [1.0; 3],
        },
        Swatch::Ambient => editor.environment().ambient_color,
        Swatch::Fog => editor
            .environment()
            .fog
            .as_ref()
            .map_or([0.5; 3], |fog| fog.color),
    }
}

fn color_allowed(editor: &Editor, swatch: Swatch) -> bool {
    match (swatch, editor.selection) {
        (Swatch::Light, Some(Selection::Light(id))) => editor.light(id).is_some(),
        (Swatch::Ambient, Some(Selection::Environment)) => true,
        (Swatch::Fog, Some(Selection::Environment)) => editor.environment().fog.is_some(),
        _ => false,
    }
}

fn set_color(editor: &mut Editor, swatch: Swatch, hsv: Hsva) {
    let color = Srgba::from(hsv);
    let rgb = [color.red, color.green, color.blue].map(|value| value.clamp(0.0, 1.0));
    if color_value(editor, swatch)
        .into_iter()
        .zip(rgb)
        .all(|(a, b)| (a - b).abs() < 1e-6)
        && !matches!(editor.selection, Some(Selection::Light(id)) if editor.light(id).is_some_and(|light| light.temperature.is_some()))
    {
        return;
    }
    match (swatch, editor.selection) {
        (Swatch::Light, Some(Selection::Light(id))) => {
            if let Some(light) = editor.edits.lights.iter_mut().find(|light| light.id == id) {
                light.color = rgb;
                light.temperature = None;
            }
        }
        (Swatch::Ambient | Swatch::Fog, Some(Selection::Environment)) => {
            let mut settings = editor.environment().clone();
            if matches!(swatch, Swatch::Ambient) {
                settings.ambient_color = rgb;
            } else if let Some(fog) = &mut settings.fog {
                fog.color = rgb;
            }
            editor.edits.lighting = Some(settings);
        }
        _ => {}
    }
}

fn color_gradient(angle: f32, colors: impl IntoIterator<Item = Color>) -> BackgroundGradient {
    LinearGradient {
        angle,
        stops: colors.into_iter().map(ColorStop::auto).collect(),
        color_space: bevy::ui::InterpolationColorSpace::Srgba,
    }
    .into()
}

// Actual sRGB texels keep the color under the marker consistent with the light.
// Translucent black/white overlays would instead blend the plane in linear RGB.
fn paint_plane(data: &mut [u8], hue: f32) {
    for (index, pixel) in data.chunks_exact_mut(4).enumerate() {
        let x = index as u32 % PLANE_WIDTH;
        let y = index as u32 / PLANE_WIDTH;
        let color = Srgba::from(Hsva::hsv(
            hue,
            x as f32 / (PLANE_WIDTH - 1) as f32,
            1.0 - y as f32 / (PLANE_HEIGHT - 1) as f32,
        ));
        pixel.copy_from_slice(&[
            (color.red * 255.0).round() as u8,
            (color.green * 255.0).round() as u8,
            (color.blue * 255.0).round() as u8,
            255,
        ]);
    }
}

fn plane_image(hue: f32) -> Image {
    let mut data = vec![255; (PLANE_WIDTH * PLANE_HEIGHT * 4) as usize];
    paint_plane(&mut data, hue);
    Image::new(
        Extent3d {
            width: PLANE_WIDTH,
            height: PLANE_HEIGHT,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

fn slider(
    commands: &mut Commands,
    parent: Entity,
    label: &str,
    binding: Binding,
    readout: Option<Entity>,
    hue: bool,
) -> Entity {
    let slider = commands
        .spawn((
            Slider {
                track_click: TrackClick::Snap,
                orientation: SliderOrientation::Horizontal,
            },
            SliderRange::new(0.0, 1.0),
            SliderValue(0.0),
            SliderStep(0.01),
            SliderPrecision(4),
            TabIndex(0),
            Hovered::default(),
            AccessibleLabel(label.into()),
            Help("Перетащите ползунок или используйте стрелки ←/→. Esc отменяет текущее перетаскивание."),
            BorderColor::all(Color::NONE),
            Node {
                width: percent(100),
                height: px(24),
                flex_shrink: 0.0,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(5)),
                ..default()
            },
            ChildOf(parent),
        ))
        .observe(queue_slider)
        .id();
    let track = surface(
        commands,
        slider,
        Node {
            position_type: PositionType::Absolute,
            left: px(THUMB * 0.5),
            right: px(THUMB * 0.5),
            top: px(if hue { 5 } else { 9 }),
            height: px(if hue { 12 } else { 4 }),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        WELL,
    );
    commands.entity(track).insert(Pickable::IGNORE);
    let fill = if hue {
        commands.entity(track).insert(color_gradient(
            LinearGradient::TO_RIGHT,
            (0..=6).map(|index| Color::from(Hsva::hsv(index as f32 * 60.0, 1.0, 1.0))),
        ));
        None
    } else {
        let fill = surface(
            commands,
            slider,
            Node {
                position_type: PositionType::Absolute,
                left: px(THUMB * 0.5),
                top: px(9),
                width: px(0),
                height: px(4),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            ACCENT,
        );
        commands.entity(fill).insert(Pickable::IGNORE);
        Some(fill)
    };
    let thumb = commands
        .spawn((
            SliderThumb,
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                width: px(THUMB),
                height: px(THUMB),
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
    commands.entity(slider).insert(LightingSlider {
        binding,
        thumb,
        fill,
        readout,
    });
    slider
}

pub(super) fn scalar(
    commands: &mut Commands,
    parent: Entity,
    font_handle: &Handle<Font>,
    label: &str,
    field: Field,
    unit: &str,
) -> Entity {
    let line = node(
        commands,
        parent,
        Node {
            width: percent(100),
            flex_shrink: 0.0,
            ..column(0.0)
        },
    );
    let header = node(
        commands,
        line,
        Node {
            width: percent(100),
            height: px(20),
            ..row(5.0)
        },
    );
    let caption = text(commands, header, font_handle, label, 12.0, MUTED);
    commands.entity(caption).insert(Node {
        flex_grow: 1.0,
        min_width: px(0),
        ..default()
    });
    let readout = text(commands, header, font_handle, "", 12.0, TEXT);
    let suffix = text(commands, header, font_handle, unit, 10.0, MUTED);
    let control = slider(
        commands,
        line,
        label,
        Binding::Number(field),
        Some(readout),
        false,
    );
    commands.entity(control).insert(field);
    suffix
}

pub(super) fn color_editor(
    commands: &mut Commands,
    parent: Entity,
    font_handle: &Handle<Font>,
    swatch: Swatch,
    images: &mut Assets<Image>,
) {
    let group = node(
        commands,
        parent,
        Node {
            width: percent(100),
            flex_shrink: 0.0,
            ..column(4.0)
        },
    );
    let label = match swatch {
        Swatch::Light => "Цвет источника",
        Swatch::Ambient => "Цвет фонового света",
        Swatch::Fog => "Цвет тумана",
    };
    let image = images.add(plane_image(0.0));
    let area = commands
        .spawn((
            Button,
            TabIndex(0),
            Hovered::default(),
            AccessibleLabel(format!("{label}: насыщенность и яркость")),
            Help("Слева цвет спокойнее, справа — насыщеннее; сверху светлее, снизу темнее. Полоса под палитрой меняет оттенок."),
            Node {
                width: percent(100),
                height: px(112),
                flex_shrink: 0.0,
                border_radius: BorderRadius::all(px(5)),
                border: UiRect::all(px(1)),
                ..default()
            },
            ImageNode { image_mode: NodeImageMode::Stretch, ..ImageNode::new(image.clone()) },
            BorderColor::all(Color::NONE),
            ChildOf(group),
        ))
        .observe(plane_keys)
        .id();
    let marker = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: px(14),
                height: px(14),
                margin: UiRect {
                    left: px(-7),
                    top: px(-7),
                    ..default()
                },
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::MAX,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(Color::BLACK),
            Pickable::IGNORE,
            ChildOf(area),
        ))
        .id();
    commands.spawn((
        Node {
            width: px(10),
            height: px(10),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BorderColor::all(Color::WHITE),
        Pickable::IGNORE,
        ChildOf(marker),
    ));
    slider(
        commands,
        group,
        &format!("{label}: оттенок"),
        Binding::Hue(area),
        None,
        true,
    );
    let sample = node(
        commands,
        group,
        Node {
            height: px(22),
            width: percent(100),
            ..row(7.0)
        },
    );
    let chip = surface(
        commands,
        sample,
        Node {
            width: px(22),
            height: px(18),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
            ..default()
        },
        Color::WHITE,
    );
    commands.entity(chip).insert(Pickable::IGNORE);
    let code = text(commands, sample, font_handle, "#FFFFFF", 11.0, MUTED);
    commands.entity(area).insert(ColorPicker {
        swatch,
        hsv: Hsva::default(),
        last_rgb: [1.0; 3],
        context: None,
        marker,
        chip,
        code,
        image,
        drawn_hue: 0.0,
    });
}

fn queue_slider(event: On<ValueChange<f32>>, editor: Res<Editor>, mut ui: ResMut<LightingUi>) {
    ui.controls.queued.push(Queued {
        source: event.source,
        context: editor.selection.map(|selection| (editor.room, selection)),
        change: Change::Slider(event.value),
        final_value: event.is_final,
    });
}

fn plane_keys(
    mut event: On<FocusedInput<KeyboardInput>>,
    editor: Res<Editor>,
    mut ui: ResMut<LightingUi>,
) {
    if event.input.state != ButtonState::Pressed {
        return;
    }
    let delta = match event.input.key_code {
        KeyCode::ArrowLeft => Vec2::new(-0.01, 0.0),
        KeyCode::ArrowRight => Vec2::new(0.01, 0.0),
        KeyCode::ArrowUp => Vec2::new(0.0, 0.01),
        KeyCode::ArrowDown => Vec2::new(0.0, -0.01),
        _ => return,
    };
    event.propagate(false);
    ui.controls.queued.push(Queued {
        source: event.focused_entity,
        context: editor.selection.map(|selection| (editor.room, selection)),
        change: Change::Plane(delta),
        final_value: true,
    });
}

pub(crate) fn input(
    mut editor: ResMut<Editor>,
    mut ui: ResMut<LightingUi>,
    mut focus: ResMut<InputFocus>,
    window: Single<&Window, With<PrimaryWindow>>,
    motion: Res<DrawerMotion>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    numeric: Res<NumericInput>,
    mut actions: MessageReader<Action>,
    mut closing: MessageReader<WindowCloseRequested>,
    sliders: Query<&LightingSlider>,
    mut pickers: Query<(Entity, &mut ColorPicker)>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform, Option<&CalculatedClip>)>,
) {
    let force = actions.read().last().is_some() || closing.read().last().is_some();
    let context = editor.selection.map(|selection| (editor.room, selection));
    let menu_open = ui.menu_open;
    let state = &mut ui.controls;
    if state
        .drag
        .as_ref()
        .is_some_and(|drag| Some((drag.room, drag.selection)) != context)
    {
        state.finish(&mut editor);
        state.suppressed = mouse.pressed(MouseButton::Left);
    }
    if keys.just_pressed(KeyCode::Escape) && state.active() {
        if let Some(drag) = state.drag.take() {
            if Some((drag.room, drag.selection)) == context {
                editor.edits = drag.before;
            }
            if let Some((entity, hsv)) = drag.picker
                && let Ok((_, mut picker)) = pickers.get_mut(entity)
            {
                picker.hsv = hsv;
                picker.last_rgb = color_value(&editor, picker.swatch);
            }
        }
        state.queued.clear();
        state.suppressed = true;
        focus.clear();
        ui.pointer_consumed = true;
        return;
    }
    if !mouse.pressed(MouseButton::Left) && !mouse.just_released(MouseButton::Left) {
        state.suppressed = false;
    }
    if !editor.has_scene()
        || !editor.ready
        || editor.pending.is_some()
        || editor.asset_viewer_open()
        || editor.browser_popup
        || menu_open
        || numeric.error_message().is_some()
        || state.suppressed
    {
        state.finish(&mut editor);
        state.queued.clear();
        return;
    }
    let mut finish = false;
    for event in std::mem::take(&mut state.queued) {
        if event.context != context {
            continue;
        }
        match event.change {
            Change::Slider(value) if value.is_finite() => {
                let Ok(slider) = sliders.get(event.source) else {
                    continue;
                };
                match slider.binding {
                    Binding::Number(field) => {
                        let Some(selection) = editor.selection else {
                            continue;
                        };
                        if !editor.field_allowed(selection, field) {
                            continue;
                        }
                        let Some(range) = limits(&editor, field) else {
                            continue;
                        };
                        let value = range.value(value);
                        let original = editor.field_value(selection, field).unwrap_or(value);
                        if (original - value).abs() > 1e-6 * original.abs().max(1.0) {
                            state.begin(&mut editor, event.source, false, None);
                            editor.set_field(field, value);
                        }
                    }
                    Binding::Hue(entity) => {
                        if let Ok((_, mut picker)) = pickers.get_mut(entity) {
                            if !color_allowed(&editor, picker.swatch) {
                                continue;
                            }
                            picker.sync(&editor);
                            state.begin(
                                &mut editor,
                                event.source,
                                false,
                                Some((entity, picker.hsv)),
                            );
                            picker.hsv.hue = value.clamp(0.0, 1.0) * 360.0;
                            set_color(&mut editor, picker.swatch, picker.hsv);
                            picker.last_rgb = color_value(&editor, picker.swatch);
                        }
                    }
                }
            }
            Change::Plane(delta) if delta.is_finite() => {
                if let Ok((_, mut picker)) = pickers.get_mut(event.source) {
                    if !color_allowed(&editor, picker.swatch) {
                        continue;
                    }
                    picker.sync(&editor);
                    state.begin(
                        &mut editor,
                        event.source,
                        true,
                        Some((event.source, picker.hsv)),
                    );
                    picker.hsv.saturation = (picker.hsv.saturation + delta.x).clamp(0.0, 1.0);
                    picker.hsv.value = (picker.hsv.value + delta.y).clamp(0.0, 1.0);
                    set_color(&mut editor, picker.swatch, picker.hsv);
                    picker.last_rgb = color_value(&editor, picker.swatch);
                }
            }
            _ => {}
        }
        finish |= event.final_value;
    }
    if !force
        && window.focused
        && let Some(cursor) = window.cursor_position()
    {
        let physical = cursor * window.scale_factor();
        if mouse.just_pressed(MouseButton::Left)
            && cursor.y < window.height() - FOOT - motion.visible_height
        {
            for (entity, mut picker) in &mut pickers {
                let Ok((node, transform, clip)) = geometry.get(entity) else {
                    continue;
                };
                if color_allowed(&editor, picker.swatch)
                    && !node.is_empty()
                    && node.contains_point(*transform, physical)
                    && clip.is_none_or(|clip| clip.contains_point(physical))
                {
                    picker.sync(&editor);
                    state.begin(&mut editor, entity, true, Some((entity, picker.hsv)));
                    focus.set(entity, FocusCause::Pressed);
                    break;
                }
            }
        }
        if let Some(drag) = state.drag.as_ref().filter(|drag| drag.plane)
            && (mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left))
            && let Ok((node, transform, _)) = geometry.get(drag.source)
            && let Some(point) = node.normalize_point(*transform, physical)
            && let Ok((_, mut picker)) = pickers.get_mut(drag.source)
        {
            picker.hsv.saturation = (point.x + 0.5).clamp(0.0, 1.0);
            picker.hsv.value = (0.5 - point.y).clamp(0.0, 1.0);
            set_color(&mut editor, picker.swatch, picker.hsv);
            picker.last_rgb = color_value(&editor, picker.swatch);
        }
    }
    if state
        .drag
        .as_ref()
        .is_some_and(|drag| drag.before != editor.edits)
    {
        editor.status = "Освещение изменено".into();
    }
    if force || finish || !mouse.pressed(MouseButton::Left) || !window.focused {
        state.finish(&mut editor);
        if force {
            state.suppressed = mouse.pressed(MouseButton::Left);
        }
    }
}

pub(crate) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    focus: Res<InputFocus>,
    ui: Res<LightingUi>,
    mut pickers: Query<(Entity, &mut ColorPicker)>,
    sliders: Query<(Entity, &LightingSlider, &SliderValue, &Hovered)>,
    computed: Query<&ComputedNode>,
    mut nodes: Query<&mut Node>,
    mut colors: Query<&mut BackgroundColor>,
    mut borders: Query<&mut BorderColor>,
    mut labels: Query<&mut Text>,
    mut images: ResMut<Assets<Image>>,
) {
    let enabled = editor.has_scene() && editor.ready && editor.pending.is_none() && !ui.menu_open;
    for (entity, mut picker) in &mut pickers {
        picker.sync(&editor);
        if picker.drawn_hue != picker.hsv.hue
            && let Some(mut image) = images.get_mut(&picker.image)
            && let Some(data) = image.data.as_mut()
        {
            paint_plane(data, picker.hsv.hue);
            picker.drawn_hue = picker.hsv.hue;
        }
        if let Ok(mut marker) = nodes.get_mut(picker.marker) {
            marker.left = percent(picker.hsv.saturation * 100.0);
            marker.top = percent((1.0 - picker.hsv.value) * 100.0);
        }
        let rgb = color_value(&editor, picker.swatch);
        if let Ok(mut color) = colors.get_mut(picker.chip) {
            color.set_if_neq(BackgroundColor(Color::srgb(rgb[0], rgb[1], rgb[2])));
        }
        if let Ok(mut code) = labels.get_mut(picker.code) {
            let [r, g, b] = rgb.map(|value| (value * 255.0).round() as u8);
            let value = format!("#{r:02X}{g:02X}{b:02X}");
            if code.0 != value {
                code.0 = value;
            }
        }
        let allowed = enabled && color_allowed(&editor, picker.swatch);
        if let Ok(mut border) = borders.get_mut(entity) {
            border.set_if_neq(BorderColor::all(
                if allowed && focus.get() == Some(entity) {
                    FOCUS
                } else {
                    Color::NONE
                },
            ));
        }
        if allowed {
            commands.entity(entity).try_remove::<InteractionDisabled>();
        } else {
            commands.entity(entity).try_insert(InteractionDisabled);
        }
        commands
            .entity(entity)
            .try_insert(TabIndex(if allowed { 0 } else { -1 }));
    }
    for (entity, slider, stored, hovered) in &sliders {
        let (position, allowed, value_text) = match slider.binding {
            Binding::Number(field) => {
                let allowed = enabled
                    && editor
                        .selection
                        .is_some_and(|target| editor.field_allowed(target, field));
                let value = editor
                    .selection
                    .and_then(|target| editor.field_value(target, field))
                    .unwrap_or(0.0);
                let position = limits(&editor, field).map_or(0.0, |range| range.position(value));
                let text = if value.abs() >= 1000.0 {
                    format!("{value:.0}")
                } else if matches!(field, Field::Light(LightField::Temperature)) {
                    format!("{value:.0}")
                } else if value != 0.0 && value.abs() < 0.1 {
                    format!("{value:.3}")
                } else {
                    format!("{value:.2}")
                };
                (position, allowed, Some(text))
            }
            Binding::Hue(entity) => {
                let Ok((_, picker)) = pickers.get(entity) else {
                    continue;
                };
                (
                    picker.hsv.hue / 360.0,
                    enabled && color_allowed(&editor, picker.swatch),
                    None,
                )
            }
        };
        if (stored.0 - position).abs() > 1e-6 {
            commands.entity(entity).try_insert(SliderValue(position));
        }
        if allowed {
            commands.entity(entity).try_remove::<InteractionDisabled>();
        } else {
            commands.entity(entity).try_insert(InteractionDisabled);
        }
        commands
            .entity(entity)
            .try_insert(TabIndex(if allowed { 0 } else { -1 }));
        if let Ok(mut border) = borders.get_mut(entity) {
            border.set_if_neq(BorderColor::all(
                if allowed && (hovered.0 || focus.get() == Some(entity)) {
                    FOCUS
                } else {
                    Color::NONE
                },
            ));
        }
        let travel = computed.get(entity).map_or(0.0, |node| {
            (node.size().x * node.inverse_scale_factor() - THUMB - 2.0).max(0.0)
        });
        if let Ok(mut thumb) = nodes.get_mut(slider.thumb) {
            thumb.left = px(travel * position);
        }
        if let Some(fill) = slider.fill
            && let Ok(mut node) = nodes.get_mut(fill)
        {
            node.width = px(travel * position);
        }
        if let Some(readout) = slider.readout
            && let Some(value) = value_text
            && let Ok(mut text) = labels.get_mut(readout)
        {
            if text.0 != value {
                text.0 = value;
            }
        }
    }
}
