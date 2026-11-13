use super::*;
use bevy::{
    camera::RenderTarget,
    picking::{
        backend::HitData,
        events::Pointer,
        pointer::{Location, PointerId},
    },
    window::WindowRef,
};

/// Exercise the real backend registry: a headless fixture cannot detect using
/// the old, absent NonSend<WinitWindows> resource instead of WINIT_WINDOWS.
#[cfg(target_os = "windows")]
#[test]
#[ignore = "requires a Windows 11 desktop; creates a hidden native window"]
#[allow(deprecated)]
fn native_windows_receive_icon_and_rounded_corners() {
    use std::ffi::c_void;
    use winit::{
        event_loop::EventLoop,
        platform::windows::EventLoopBuilderExtWindows,
        raw_window_handle::{HasWindowHandle, RawWindowHandle},
    };

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmGetWindowAttribute(
            hwnd: *mut c_void,
            attribute: u32,
            value: *mut c_void,
            size: u32,
        ) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SendMessageW(hwnd: *mut c_void, message: u32, wparam: usize, lparam: isize) -> isize;
        fn GetWindowLongW(hwnd: *mut c_void, index: i32) -> i32;
    }

    let event_loop = EventLoop::builder().with_any_thread(true).build().unwrap();
    let native_window = event_loop
        .create_window(
            winit::window::Window::default_attributes()
                .with_title("Bevy Editor native appearance test")
                .with_visible(false)
                .with_decorations(false),
        )
        .unwrap();
    let RawWindowHandle::Win32(handle) = native_window.window_handle().unwrap().as_raw() else {
        panic!("Expected a Windows window handle");
    };
    let hwnd = handle.hwnd.get() as *mut c_void;
    let id = native_window.id();
    let (mut app, entity) = app();
    assert!(app.world().get::<NativeAppearanceApplied>(entity).is_none());

    WINIT_WINDOWS.with_borrow_mut(|windows| {
        windows.entity_to_winit.insert(entity, id);
        windows.winit_to_entity.insert(id, entity);
        windows
            .windows
            .insert(id, bevy::window::WindowWrapper::new(native_window));
    });
    struct Registration(Entity);
    impl Drop for Registration {
        fn drop(&mut self) {
            WINIT_WINDOWS.with_borrow_mut(|windows| {
                windows.remove_window(self.0);
            });
        }
    }
    let _registration = Registration(entity);
    app.update();
    assert!(
        app.world().get::<NativeAppearanceApplied>(entity).is_some(),
        "Native appearance must reach Bevy's actual window registry"
    );

    let mut preference = 0u32;
    // SAFETY: the registration owns this live HWND and preference is writable.
    let result =
        unsafe { DwmGetWindowAttribute(hwnd, 33, (&mut preference as *mut u32).cast(), 4) };
    assert_eq!(result, 0, "Read corner preference from Windows DWM");
    assert_eq!(preference, 2, "Apply the rounded-corner preference");
    // SAFETY: WM_GETICON only reads the live window's small and large icons.
    unsafe {
        let style = GetWindowLongW(hwnd, -16) as u32;
        assert_ne!(
            style & 0x00c40000,
            0,
            "Native frame required for DWM rounding"
        );
        assert_ne!(SendMessageW(hwnd, 0x7f, 0, 0), 0, "Small window icon");
        assert_ne!(SendMessageW(hwnd, 0x7f, 1, 0), 0, "Taskbar icon");
    }
}

fn setup(mut commands: Commands) {
    let header = commands.spawn(Node::default()).id();
    drag_region(&mut commands, header);
    controls(&mut commands, header);
    resize_edges(&mut commands, header);
}

fn app() -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(WindowChromePlugin)
        .init_resource::<ButtonInput<MouseButton>>()
        .add_message::<WindowCloseRequested>()
        .add_systems(Startup, setup);
    let window = app
        .world_mut()
        .spawn((
            PrimaryWindow,
            Window {
                decorations: false,
                ..default()
            },
        ))
        .id();
    app.update();
    (app, window)
}

fn control(app: &mut App, action: WindowControl) -> Entity {
    let world = app.world_mut();
    world
        .query::<(Entity, &WindowControl)>()
        .iter(world)
        .find(|(_, control)| **control == action)
        .unwrap()
        .0
}

fn press(app: &mut App, window: Entity, entity: Entity, button: PointerButton, count: u8) {
    app.world_mut().trigger(PointerPress {
        entity,
        pointer: Pointer::new(
            PointerId::Mouse,
            Location {
                target: RenderTarget::Window(WindowRef::Entity(window))
                    .normalize(None)
                    .unwrap(),
                position: Vec2::ZERO,
            },
        ),
        button,
        hit: HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
        count,
    });
}

#[test]
fn caption_buttons_toggle_native_window_state_and_request_normal_close() {
    let (mut app, window) = app();
    let maximize = control(&mut app, WindowControl::Maximize);
    for expected in [true, false] {
        app.world_mut().trigger(Activate { entity: maximize });
        assert_eq!(
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .internal
                .take_maximize_request(),
            Some(expected)
        );
        app.update();
        assert_eq!(
            app.world().get::<AccessibleLabel>(maximize).unwrap().0,
            if expected {
                "Восстановить размер окна"
            } else {
                "Развернуть окно"
            }
        );
    }
    let minimize = control(&mut app, WindowControl::Minimize);
    app.world_mut().trigger(Activate { entity: minimize });
    assert_eq!(
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .internal
            .take_minimize_request(),
        Some(true)
    );
    let close = control(&mut app, WindowControl::Close);
    app.world_mut().trigger(Activate { entity: close });
    let requests: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<WindowCloseRequested>>()
        .drain()
        .collect();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].window, window);
    assert!(
        app.world().get_entity(window).is_ok(),
        "Close must let the editor handle unsaved changes"
    );
    {
        let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        mouse.press(MouseButton::Right);
        mouse.release(MouseButton::Right);
    }
    app.world_mut().trigger(Activate { entity: close });
    assert!(
        app.world()
            .resource::<Messages<WindowCloseRequested>>()
            .is_empty(),
        "Secondary clicks must not close the window"
    );
}

#[test]
fn header_drag_and_double_press_do_not_steal_button_interactions() {
    let (mut app, window) = app();
    let header = app
        .world_mut()
        .query_filtered::<Entity, With<DragRegion>>()
        .single(app.world())
        .unwrap();
    let brand = app
        .world_mut()
        .spawn((Node::default(), ChildOf(header)))
        .id();
    press(&mut app, window, brand, PointerButton::Primary, 1);
    assert!(
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .internal
            .take_move_request()
    );
    press(&mut app, window, brand, PointerButton::Primary, 2);
    assert_eq!(
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .internal
            .take_maximize_request(),
        Some(true)
    );
    let tab = app.world_mut().spawn((Button, ChildOf(header))).id();
    let tab_label = app.world_mut().spawn((Node::default(), ChildOf(tab))).id();
    press(&mut app, window, tab_label, PointerButton::Primary, 1);
    press(&mut app, window, brand, PointerButton::Secondary, 1);
    assert!(
        !app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .internal
            .take_move_request()
    );
}

#[test]
fn all_resize_edges_request_native_resize_without_selecting_the_scene() {
    let (mut app, window) = app();
    let edges: Vec<_> = app
        .world_mut()
        .query::<(Entity, &ResizeEdge)>()
        .iter(app.world())
        .map(|(entity, edge)| (entity, edge.0))
        .collect();
    assert_eq!(edges.len(), 8);
    for (edge, direction) in &edges {
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        press(&mut app, window, *edge, PointerButton::Primary, 1);
        assert_eq!(
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .internal
                .take_resize_request(),
            Some(*direction)
        );
        assert!(
            !app.world()
                .resource::<ButtonInput<MouseButton>>()
                .just_pressed(MouseButton::Left)
        );
    }
    app.world_mut().resource_mut::<ChromeState>().maximized = true;
    press(&mut app, window, edges[0].0, PointerButton::Primary, 1);
    assert_eq!(
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .internal
            .take_resize_request(),
        None
    );
    app.update();
    assert!(
        edges
            .iter()
            .all(|(edge, _)| app.world().get::<Node>(*edge).unwrap().display == Display::None)
    );
}
