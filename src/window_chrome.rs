//! Shared, in-app window controls for the manager and editor headers.
use bevy::{
    ecs::system::NonSendMarker,
    input_focus::{InputFocus, tab_navigation::TabIndex},
    math::CompassOctant,
    picking::{events::PointerPress, hover::Hovered, pointer::PointerButton},
    prelude::*,
    ui::{Pressed, UiSystems},
    ui_widgets::{Activate, Button, TextInput},
    window::{CursorIcon, PrimaryWindow, SystemCursorIcon, WindowCloseRequested},
    winit::WINIT_WINDOWS,
};

use crate::{
    editor::icons::{self, Glyph},
    ui_theme::*,
};

pub(crate) struct WindowChromePlugin;

impl Plugin for WindowChromePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::app_icon::AppIconPlugin)
            .init_resource::<ChromeState>()
            .add_systems(PostUpdate, refresh.after(UiSystems::Layout));
        #[cfg(not(target_arch = "wasm32"))]
        app.add_systems(Update, configure_native_windows);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Component)]
struct NativeAppearanceApplied;

#[cfg(not(target_arch = "wasm32"))]
fn configure_native_windows(
    _main_thread: NonSendMarker,
    windows: Query<Entity, (With<Window>, Without<NativeAppearanceApplied>)>,
    icon: Res<crate::app_icon::AppIcon>,
    mut commands: Commands,
) {
    WINIT_WINDOWS.with_borrow(|native| {
        for entity in &windows {
            let Some(window) = native.get_window(entity) else {
                continue;
            };
            match icon.window_icon() {
                Ok(icon) => {
                    window.set_window_icon(Some(icon.clone()));
                    #[cfg(target_os = "windows")]
                    {
                        use winit::platform::windows::WindowExtWindows;
                        window.set_taskbar_icon(Some(icon));
                    }
                }
                Err(error) => warn!("Could not set application icon: {error}"),
            }
            #[cfg(target_os = "windows")]
            round_native_window(window);
            commands.entity(entity).insert(NativeAppearanceApplied);
        }
    });
}

#[cfg(target_os = "windows")]
fn round_native_window(window: &winit::window::Window) {
    use std::ffi::c_void;
    use winit::{
        platform::windows::WindowExtWindows,
        raw_window_handle::{HasWindowHandle, RawWindowHandle},
    };

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut c_void,
            attribute: u32,
            value: *const c_void,
            size: u32,
        ) -> i32;
    }

    // Keep a native frame for the shadow and Windows 11's rounded corners.
    // DWM automatically restores square corners while the window is maximized.
    window.set_undecorated_shadow(true);
    if let Ok(handle) = window.window_handle()
        && let RawWindowHandle::Win32(handle) = handle.as_raw()
    {
        const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
        const DWMWCP_ROUND: u32 = 2;
        // SAFETY: winit owns this live HWND; DWM reads a four-byte preference.
        let result = unsafe {
            DwmSetWindowAttribute(
                handle.hwnd.get() as *mut c_void,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&DWMWCP_ROUND as *const u32).cast(),
                size_of::<u32>() as u32,
            )
        };
        debug!(result, "Requested rounded native window corners");
    }
}

#[derive(Resource, Default)]
struct ChromeState {
    maximized: bool,
    owns_cursor: bool,
    previous_cursor: Option<CursorIcon>,
}

#[derive(Component)]
struct DragRegion;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum WindowControl {
    Minimize,
    Maximize,
    Close,
}

#[derive(Component)]
struct ControlIcon(Entity);

#[derive(Component)]
struct ResizeEdge(CompassOctant);

/// Empty header space and the brand act as the native titlebar. Descendant
/// buttons, tabs and text inputs keep their normal pointer interactions.
pub(crate) fn drag_region(commands: &mut Commands, header: Entity) {
    commands
        .entity(header)
        .insert((DragRegion, Pickable::default()))
        .observe(drag_pressed);
}

pub(crate) fn controls(commands: &mut Commands, header: Entity) {
    let group = commands
        .spawn((
            Node {
                height: px(34),
                flex_shrink: 0.0,
                column_gap: px(2),
                padding: UiRect::left(px(8)),
                border: UiRect::left(px(1)),
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(EDGE.with_alpha(0.55)),
            GlobalZIndex(200),
            Pickable::IGNORE,
            ChildOf(header),
        ))
        .id();
    for (action, glyph, label) in [
        (WindowControl::Minimize, Glyph::Minimize, "Свернуть окно"),
        (WindowControl::Maximize, Glyph::Maximize, "Развернуть окно"),
        (WindowControl::Close, Glyph::Close, "Закрыть окно"),
    ] {
        let button = commands
            .spawn((
                Button,
                action,
                Hovered::default(),
                TabIndex(0),
                AccessibleLabel(label.into()),
                Node {
                    width: px(34),
                    height: px(30),
                    flex_shrink: 0.0,
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(6)),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::NONE),
                BorderColor::all(Color::NONE),
                ChildOf(group),
            ))
            .observe(control_activated)
            .observe(|mut event: On<PointerPress>| event.propagate(false))
            .id();
        let icon = icons::spawn(commands, button, glyph, 14.0, TEXT);
        commands.entity(button).insert(ControlIcon(icon));
    }
}

/// Native resize requests preserve platform constraints and DPI behavior.
/// Corners are larger than the narrow edge strips, without covering controls.
pub(crate) fn resize_edges(commands: &mut Commands, root: Entity) {
    use CompassOctant::*;
    for direction in [
        North, South, East, West, NorthEast, NorthWest, SouthEast, SouthWest,
    ] {
        let horizontal = matches!(direction, North | South);
        let vertical = matches!(direction, East | West);
        let mut node = Node {
            position_type: PositionType::Absolute,
            width: px(if vertical { 4 } else { 10 }),
            height: px(if horizontal { 4 } else { 10 }),
            ..default()
        };
        if horizontal {
            node.left = px(10);
            node.right = px(10);
            node.width = Val::Auto;
        }
        if vertical {
            node.top = px(10);
            node.bottom = px(10);
            node.height = Val::Auto;
        }
        if matches!(direction, North | NorthEast | NorthWest) {
            node.top = px(0);
        }
        if matches!(direction, South | SouthEast | SouthWest) {
            node.bottom = px(0);
        }
        if matches!(direction, East | NorthEast | SouthEast) {
            node.right = px(0);
        }
        if matches!(direction, West | NorthWest | SouthWest) {
            node.left = px(0);
        }
        commands
            .spawn((
                node,
                ResizeEdge(direction),
                Hovered::default(),
                Pickable::default(),
                GlobalZIndex(300),
                ChildOf(root),
            ))
            .observe(resize_pressed);
    }
}

fn native_maximized(entity: Entity, fallback: bool) -> bool {
    WINIT_WINDOWS.with_borrow(|windows| {
        windows
            .get_window(entity)
            .map_or(fallback, |window| window.is_maximized())
    })
}

fn toggle_maximized(window: &mut Window, state: &mut ChromeState, entity: Entity) {
    state.maximized = !native_maximized(entity, state.maximized);
    window.set_maximized(state.maximized);
}

fn control_activated(
    event: On<Activate>,
    controls: Query<&WindowControl>,
    mut window: Single<(Entity, &mut Window), With<PrimaryWindow>>,
    mut state: ResMut<ChromeState>,
    _main_thread: NonSendMarker,
    mouse: Res<ButtonInput<MouseButton>>,
    mut close: MessageWriter<WindowCloseRequested>,
) {
    // Button also emits Activate for secondary clicks; native caption controls
    // respond only to the primary button or keyboard activation.
    if mouse.just_released(MouseButton::Right) || mouse.just_released(MouseButton::Middle) {
        return;
    }
    let Ok(action) = controls.get(event.entity) else {
        return;
    };
    let (entity, window) = &mut *window;
    match action {
        WindowControl::Minimize => window.set_minimized(true),
        WindowControl::Maximize => toggle_maximized(window, &mut state, *entity),
        WindowControl::Close => {
            // Use the same path as Alt+F4: the editor can ask to save changes.
            close.write(WindowCloseRequested { window: *entity });
        }
    }
}

fn drag_pressed(
    mut event: On<PointerPress>,
    regions: Query<(), With<DragRegion>>,
    interactive: Query<(), Or<(With<Button>, With<TextInput>)>>,
    parents: Query<&ChildOf>,
    mut window: Single<(Entity, &mut Window), With<PrimaryWindow>>,
    mut state: ResMut<ChromeState>,
    _main_thread: NonSendMarker,
) {
    if event.button != PointerButton::Primary {
        return;
    }
    let mut target = event.original_event_target();
    loop {
        if interactive.contains(target) {
            return;
        }
        if regions.contains(target) {
            break;
        }
        let Ok(parent) = parents.get(target) else {
            return;
        };
        target = parent.parent();
    }
    event.propagate(false);
    let (entity, window) = &mut *window;
    if event.count % 2 == 0 {
        toggle_maximized(window, &mut state, *entity);
    } else {
        window.start_drag_move();
    }
}

fn resize_pressed(
    mut event: On<PointerPress>,
    edges: Query<&ResizeEdge>,
    mut window: Single<(Entity, &mut Window), With<PrimaryWindow>>,
    state: Res<ChromeState>,
    _main_thread: NonSendMarker,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
) {
    if event.button != PointerButton::Primary {
        return;
    }
    let Ok(edge) = edges.get(event.entity) else {
        return;
    };
    let (entity, window) = &mut *window;
    if !window.resizable || native_maximized(*entity, state.maximized) {
        return;
    }
    event.propagate(false);
    window.start_drag_resize(edge.0);
    // The viewport must not also select or begin a transform behind the grip.
    mouse.clear_just_pressed(MouseButton::Left);
}

fn resize_cursor(direction: CompassOctant) -> SystemCursorIcon {
    use CompassOctant::*;
    match direction {
        North | South => SystemCursorIcon::NsResize,
        East | West => SystemCursorIcon::EwResize,
        NorthEast | SouthWest => SystemCursorIcon::NeswResize,
        NorthWest | SouthEast => SystemCursorIcon::NwseResize,
    }
}

fn refresh(
    mut commands: Commands,
    window: Single<(Entity, &Window, Option<&CursorIcon>), With<PrimaryWindow>>,
    _main_thread: NonSendMarker,
    mut state: ResMut<ChromeState>,
    mut controls: Query<(
        Entity,
        &WindowControl,
        &ControlIcon,
        &Hovered,
        Has<Pressed>,
        &mut BackgroundColor,
        &mut BorderColor,
        &AccessibleLabel,
    )>,
    mut edges: Query<(&ResizeEdge, &Hovered, &mut Node, &mut Pickable)>,
    focus: Option<Res<InputFocus>>,
) {
    let (window_entity, window, cursor) = *window;
    state.maximized = native_maximized(window_entity, state.maximized);
    for (entity, action, icon, hover, pressed, mut background, mut border, label) in &mut controls {
        let highlighted = hover.get() || pressed;
        background.set_if_neq(BackgroundColor(if highlighted {
            if *action == WindowControl::Close {
                Color::srgba(0.84, 0.27, 0.25, if pressed { 0.60 } else { 0.34 })
            } else if pressed {
                ACTIVE
            } else {
                HOVER
            }
        } else {
            Color::NONE
        }));
        border.set_if_neq(BorderColor::all(
            if focus
                .as_ref()
                .is_some_and(|focus| focus.get() == Some(entity))
            {
                FOCUS
            } else {
                Color::NONE
            },
        ));
        if *action == WindowControl::Maximize {
            icons::set_glyph(
                &mut commands,
                icon.0,
                if state.maximized {
                    Glyph::Restore
                } else {
                    Glyph::Maximize
                },
            );
            let caption = if state.maximized {
                "Восстановить размер окна"
            } else {
                "Развернуть окно"
            };
            if label.0 != caption {
                commands
                    .entity(entity)
                    .insert(AccessibleLabel(caption.into()));
            }
        }
    }
    let resizable = window.resizable && !state.maximized && !window.decorations;
    let mut hovered = None;
    for (edge, hover, mut node, mut pickable) in &mut edges {
        node.display = if resizable {
            Display::Flex
        } else {
            Display::None
        };
        *pickable = if resizable {
            Pickable::default()
        } else {
            Pickable::IGNORE
        };
        if resizable && hover.get() {
            hovered = Some(resize_cursor(edge.0));
        }
    }
    if let Some(resize) = hovered {
        if !state.owns_cursor {
            state.previous_cursor = cursor.cloned();
            state.owns_cursor = true;
        }
        commands
            .entity(window_entity)
            .insert(CursorIcon::System(resize));
    } else if std::mem::take(&mut state.owns_cursor) {
        if let Some(previous) = state.previous_cursor.take() {
            commands.entity(window_entity).insert(previous);
        } else {
            commands.entity(window_entity).remove::<CursorIcon>();
        }
    }
}

#[cfg(test)]
mod tests;
