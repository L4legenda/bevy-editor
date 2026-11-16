//! Background render workers share the editor's renderer and MCP without desktop UI.
use std::time::Duration;

use bevy::{
    prelude::*,
    window::{WindowResizeConstraints, WindowResolution},
    winit::{UpdateMode, WinitSettings},
};

#[derive(Resource)]
pub(super) struct RenderWorker;

pub(super) fn enabled() -> bool {
    std::env::var_os("BEVY_EDITOR_RENDER_WORKER").is_some_and(|value| value == "1")
}

pub(super) fn window(title: &str, worker: bool) -> Window {
    Window {
        title: title.to_owned(),
        decorations: false,
        visible: !worker,
        focused: !worker,
        resolution: if worker {
            WindowResolution::new(1280, 800).with_scale_factor_override(1.0)
        } else {
            (1280, 760).into()
        },
        resize_constraints: WindowResizeConstraints {
            min_width: 1000.0,
            min_height: 620.0,
            ..default()
        },
        ..default()
    }
}

pub(super) fn update_settings() -> WinitSettings {
    // MCP requests wake the event loop immediately. A short periodic update also
    // drains asynchronous GPU screenshot results when no further requests arrive.
    // This keeps five idle workers from each rendering continuously at 60 Hz.
    let mode = UpdateMode::reactive_low_power(Duration::from_millis(100));
    WinitSettings {
        focused_mode: mode,
        unfocused_mode: mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_window_is_hidden_and_has_a_fixed_render_scale() {
        let window = window("Worker", true);
        assert!(!window.visible);
        assert!(!window.focused);
        assert_eq!(window.resolution.width(), 1280.0);
        assert_eq!(window.resolution.height(), 800.0);
        assert_eq!(window.resolution.scale_factor_override(), Some(1.0));
    }

    #[test]
    fn normal_editor_window_preserves_interactive_defaults() {
        let window = window("Editor", false);
        assert!(window.visible);
        assert!(window.focused);
        assert_eq!(window.resolution.width(), 1280.0);
        assert_eq!(window.resolution.height(), 760.0);
        assert_eq!(window.resolution.scale_factor_override(), None);
    }

    #[test]
    fn worker_loop_honors_mcp_wakes_and_periodically_finishes_captures() {
        let settings = update_settings();
        for mode in [settings.focused_mode, settings.unfocused_mode] {
            assert!(matches!(mode, UpdateMode::Reactive {
                wait,
                react_to_user_events: true,
                react_to_device_events: false,
                ..
            } if wait == Duration::from_millis(100)));
        }
    }
}
