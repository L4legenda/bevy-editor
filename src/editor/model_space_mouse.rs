//! SpaceMouse HID reports and object-mode navigation for the model viewport.
use super::*;
use bevy::winit::{EventLoopProxyWrapper, WinitUserEvent};
use bevy_scene_format::{Revision, atomic_write, read_limited};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

const KNOWN_DEVICES: &[(u16, u16)] = &[
    (0x046d, 0xc603),
    (0x046d, 0xc605),
    (0x046d, 0xc606),
    (0x046d, 0xc621),
    (0x046d, 0xc623),
    (0x046d, 0xc625),
    (0x046d, 0xc626),
    (0x046d, 0xc627),
    (0x046d, 0xc628),
    (0x046d, 0xc629),
    (0x046d, 0xc62b),
    (0x256f, 0xc62e),
    (0x256f, 0xc62f),
    (0x256f, 0xc631),
    (0x256f, 0xc632),
    (0x256f, 0xc633),
    (0x256f, 0xc635),
    (0x256f, 0xc636),
    (0x256f, 0xc652),
];

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum Setting {
    Controls,
    Enable,
    Scan,
    Speed(i8),
    InvertRotate,
    InvertPan,
    InvertZoom,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    enabled: bool,
    speed: u8,
    invert_rotate: bool,
    invert_pan: bool,
    invert_zoom: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: true,
            speed: 10,
            invert_rotate: false,
            invert_pan: false,
            invert_zoom: false,
        }
    }
}

impl Preferences {
    fn load(path: &Path) -> std::io::Result<Self> {
        let mut prefs: Self = serde_json::from_slice(&read_limited(path, 8192)?)?;
        prefs.speed = prefs.speed.clamp(1, 50);
        Ok(prefs)
    }

    fn save(self, path: &Path) -> std::io::Result<()> {
        atomic_write(path, &serde_json::to_vec(&self)?, Revision::read(path)?)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Motion {
    translation: [f32; 3],
    rotation: [f32; 3],
}

#[derive(Clone, Copy, Default)]
struct Reports {
    motion: Motion,
    buttons: u32,
    pressed: u32,
}

impl Reports {
    fn read(&mut self, report: &[u8]) -> bool {
        let axes = |bytes: &[u8]| {
            std::array::from_fn(|i| {
                (i16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]) as f32 / 350.).clamp(-1., 1.)
            })
        };
        let previous = (self.motion, self.buttons);
        match report {
            [1, payload @ ..] if payload.len() >= 6 => {
                self.motion.translation = axes(payload);
                if payload.len() >= 12 {
                    self.motion.rotation = axes(&payload[6..]);
                }
            }
            [2, payload @ ..] if payload.len() >= 6 => self.motion.rotation = axes(payload),
            [3, first, rest @ ..] => {
                let mut button_bytes = [*first, 0, 0, 0];
                let count = rest.len().min(3);
                button_bytes[1..1 + count].copy_from_slice(&rest[..count]);
                let buttons = u32::from_le_bytes(button_bytes);
                self.pressed |= buttons & !self.buttons;
                self.buttons = buttons;
            }
            _ => {}
        }
        previous != (self.motion, self.buttons)
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
enum DeviceStatus {
    #[default]
    Idle,
    Searching,
    Connected(String),
    Missing,
    Error(String),
}

struct Worker {
    stop: Arc<AtomicBool>,
    input: Arc<Mutex<(DeviceStatus, Reports)>>,
}

impl Worker {
    fn start(wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let worker = Self {
            stop: Arc::new(AtomicBool::new(false)),
            input: Arc::new(Mutex::new((DeviceStatus::Searching, Reports::default()))),
        };
        let stop = worker.stop.clone();
        let input = worker.input.clone();
        std::thread::Builder::new()
            .name("SpaceMouse".into())
            .spawn(move || {
                let set_status = |status| {
                    let mut state = input.lock().unwrap_or_else(|e| e.into_inner());
                    *state = (status, Reports::default());
                    drop(state);
                    wake();
                };
                let (name, device) = match open_device() {
                    Ok(device) => device,
                    Err(status) => {
                        set_status(status);
                        return;
                    }
                };
                set_status(DeviceStatus::Connected(name));
                let mut bytes = [0; 64];
                while !stop.load(Ordering::Relaxed) {
                    match device.read_timeout(&mut bytes, 50) {
                        Ok(0) => {}
                        Ok(count) => {
                            let changed = input
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .1
                                .read(&bytes[..count]);
                            if changed {
                                wake();
                            }
                        }
                        Err(_) => {
                            set_status(DeviceStatus::Error("Устройство отключено".into()));
                            return;
                        }
                    }
                }
            })?;
        Ok(worker)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn open_device() -> Result<(String, hidapi::HidDevice), DeviceStatus> {
    let api = hidapi::HidApi::new()
        .map_err(|_| DeviceStatus::Error("Не удалось открыть системный HID".into()))?;
    let multi_axis = |device: &hidapi::DeviceInfo| device.usage_page() == 1 && device.usage() == 8;
    let info = api
        .device_list()
        .find(|device| multi_axis(device))
        .or_else(|| {
            api.device_list().find(|device| {
                device.usage_page() == 0
                    && KNOWN_DEVICES.contains(&(device.vendor_id(), device.product_id()))
            })
        })
        .ok_or(DeviceStatus::Missing)?;
    let name = info
        .product_string()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("SpaceMouse")
        .to_owned();
    let device = info
        .open_device(&api)
        .map_err(|_| DeviceStatus::Error(format!("Не удалось открыть {name}")))?;
    Ok((name, device))
}

#[derive(Resource)]
pub(super) struct State {
    prefs: Preferences,
    path: Option<PathBuf>,
    worker: Option<Worker>,
    status: DeviceStatus,
    frame: Reports,
    focused: bool,
    show_controls: bool,
    revision: usize,
}

impl Default for State {
    fn default() -> Self {
        let path = paint::presets::directory()
            .and_then(|path| path.parent().map(|root| root.join("space-mouse.json")));
        Self {
            prefs: path
                .as_deref()
                .and_then(|path| Preferences::load(path).ok())
                .unwrap_or_default(),
            path,
            worker: None,
            status: DeviceStatus::Idle,
            frame: Reports::default(),
            focused: false,
            show_controls: false,
            revision: 0,
        }
    }
}

impl State {
    pub(super) fn revision(&self) -> usize {
        self.revision
    }

    fn set_status(&mut self, status: DeviceStatus) {
        if self.status != status {
            self.status = status;
            self.revision += 1;
        }
    }

    pub(super) fn drive(&self, viewer: &mut Viewer, dt: f32) -> bool {
        if !self.focused || !self.prefs.enabled {
            return false;
        }
        if self.frame.pressed & 0b11 != 0 {
            viewer.fitted = false;
            viewer.restore = None;
            return true;
        }
        apply_motion(viewer, self.frame.motion, dt, self.prefs)
    }
}

pub(super) fn update(
    mut state: ResMut<State>,
    mut actions: MessageReader<Action>,
    editor: Res<Editor>,
    window: Single<&Window, With<PrimaryWindow>>,
    event_loop: Option<Res<EventLoopProxyWrapper>>,
) {
    let mut scan = false;
    let old = state.prefs;
    for action in actions.read() {
        if let Action::ModelView(ModelViewAction::SpaceMouse(setting)) = *action {
            match setting {
                Setting::Controls => {
                    state.show_controls = !state.show_controls;
                    state.revision += 1;
                }
                Setting::Enable => {
                    state.prefs.enabled = !state.prefs.enabled;
                    scan = state.prefs.enabled;
                }
                Setting::Scan => scan = true,
                Setting::Speed(step) => {
                    state.prefs.speed =
                        (state.prefs.speed as i16 + i16::from(step)).clamp(1, 50) as u8
                }
                Setting::InvertRotate => state.prefs.invert_rotate = !state.prefs.invert_rotate,
                Setting::InvertPan => state.prefs.invert_pan = !state.prefs.invert_pan,
                Setting::InvertZoom => state.prefs.invert_zoom = !state.prefs.invert_zoom,
            }
        }
    }
    if state.prefs != old {
        state.revision += 1;
        if let Some(path) = &state.path
            && let Err(error) = state.prefs.save(path)
        {
            warn!("Could not save SpaceMouse settings: {error}");
        }
    }
    state.frame = Reports::default();
    let focused =
        window.focused && editor.viewing_model.is_some() && editor.viewing_image.is_none();
    let was_focused = state.focused;
    scan |= focused && !was_focused;
    state.focused = focused;
    if !state.prefs.enabled {
        state.worker = None;
        state.set_status(DeviceStatus::Idle);
        return;
    }
    if let Some(worker) = &state.worker {
        let (status, frame) = {
            let mut input = worker.input.lock().unwrap_or_else(|e| e.into_inner());
            let frame = input.1;
            input.1.pressed = 0;
            (input.0.clone(), frame)
        };
        if matches!(status, DeviceStatus::Missing | DeviceStatus::Error(_)) {
            state.worker = None;
        }
        if focused && was_focused && status == state.status {
            state.frame = frame;
        }
        state.set_status(status);
    }
    // Headless editor fixtures have no native event loop or physical input.
    if focused
        && scan
        && state.worker.is_none()
        && let Some(event_loop) = event_loop
    {
        let proxy = std::ops::Deref::deref(&*event_loop).clone();
        match Worker::start(move || {
            let _ = proxy.send_event(WinitUserEvent::WakeUp);
        }) {
            Ok(worker) => {
                state.worker = Some(worker);
                state.set_status(DeviceStatus::Searching);
            }
            Err(_) => state.set_status(DeviceStatus::Error(
                "Не удалось запустить чтение HID".into(),
            )),
        }
    }
}

fn apply_motion(viewer: &mut Viewer, motion: Motion, dt: f32, prefs: Preferences) -> bool {
    let deadzone = |v: f32| v.signum() * ((v.abs() - 0.04).max(0.) / 0.96);
    let [tx, ty, tz] = motion.translation.map(deadzone);
    let [rx, _, rz] = motion.rotation.map(deadzone);
    if [tx, ty, tz, rx, rz].iter().all(|&value| value == 0.) {
        return false;
    }
    let sign = |invert| if invert { -1. } else { 1. };
    let step = dt.min(0.1) * f32::from(prefs.speed) / 10.;
    if rx != 0. || rz != 0. {
        let angle = std::f32::consts::PI * step * sign(prefs.invert_rotate);
        viewer.yaw += rz * angle;
        viewer.pitch = (viewer.pitch + rx * angle).clamp(-1.53, 1.53);
        viewer.orthographic = false;
    }
    let camera = view_pose(viewer);
    viewer.center += (-*camera.right() * tx + *camera.up() * tz)
        * viewer.distance
        * step
        * sign(prefs.invert_pan);
    if ty != 0. {
        navigation::zoom(viewer, -ty * 1.5 * step * sign(prefs.invert_zoom));
    }
    true
}

pub(super) fn controls(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    state: &State,
) {
    let button = |commands: &mut Commands, parent, title, setting, active| {
        super::button(
            commands,
            parent,
            font,
            title,
            Action::ModelView(ModelViewAction::SpaceMouse(setting)),
            if active { Look::Primary } else { Look::Regular },
        );
    };
    button(
        commands,
        parent,
        "Настройки SpaceMouse",
        Setting::Controls,
        state.show_controls,
    );
    if !state.show_controls {
        return;
    }
    button(
        commands,
        parent,
        "Навигация SpaceMouse",
        Setting::Enable,
        state.prefs.enabled,
    );
    if !state.prefs.enabled {
        return;
    }
    let status = match &state.status {
        DeviceStatus::Idle => "Подключите 3D-мышь",
        DeviceStatus::Searching => "Поиск 3D-мыши…",
        DeviceStatus::Connected(name) | DeviceStatus::Error(name) => name,
        DeviceStatus::Missing => "3D-мышь не найдена",
    };
    text(commands, parent, font, status, 11., MUTED);
    if !matches!(
        state.status,
        DeviceStatus::Searching | DeviceStatus::Connected(_)
    ) {
        button(commands, parent, "Найти 3D-мышь", Setting::Scan, false);
    }
    let line = node(commands, parent, row(4.));
    text(
        commands,
        line,
        font,
        &format!("Скорость · {:.1}×", f32::from(state.prefs.speed) / 10.),
        12.,
        TEXT,
    );
    button(commands, line, "−", Setting::Speed(-1), false);
    button(commands, line, "+", Setting::Speed(1), false);
    for (label, setting, active) in [
        (
            "Инверсия вращения",
            Setting::InvertRotate,
            state.prefs.invert_rotate,
        ),
        (
            "Инверсия сдвига",
            Setting::InvertPan,
            state.prefs.invert_pan,
        ),
        (
            "Инверсия масштаба",
            Setting::InvertZoom,
            state.prefs.invert_zoom,
        ),
    ] {
        button(commands, parent, label, setting, active);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(kind: u8, axes: &[i16]) -> Vec<u8> {
        std::iter::once(kind)
            .chain(axes.iter().flat_map(|axis| axis.to_le_bytes()))
            .collect()
    }

    #[test]
    fn spacemouse_reads_split_and_combined_axes_and_accumulates_button_edges() {
        let mut input = Reports::default();
        input.read(&report(1, &[350, -175, 80]));
        input.read(&report(2, &[-350, 0, 700]));
        assert_eq!(input.motion.translation[..2], [1., -0.5]);
        assert_eq!(input.motion.rotation, [-1., 0., 1.]);
        input.read(&report(1, &[0, 0, 0, 0, 0, 35]));
        assert_eq!(
            input.motion,
            Motion {
                translation: [0.; 3],
                rotation: [0., 0., 0.1]
            }
        );
        assert!(!input.read(&[1, 0, 0]));
        input.read(&[3, 1]);
        input.read(&[3, 0]);
        input.read(&[3, 2]);
        assert_eq!(input.pressed, 3);
        input.pressed = 0;
        assert!(!input.read(&[3, 2]));
        assert_eq!(input.pressed, 0);
    }

    #[test]
    fn spacemouse_applies_object_motion_and_fit_only_with_focus() {
        let mut viewer = Viewer {
            fitted: true,
            distance: 4.,
            radius: 1.,
            orthographic: true,
            ..default()
        };
        let mut state = State::default();
        state.frame.motion.translation = [0.02, 0., 0.];
        state.focused = true;
        assert!(!state.drive(&mut viewer, 0.1));
        assert!(viewer.orthographic);
        state.frame.motion = Motion {
            translation: [1., 1., 0.],
            rotation: [1., 0., 1.],
        };
        state.focused = false;
        assert!(!state.drive(&mut viewer, 0.1));
        state.focused = true;
        assert!(state.drive(&mut viewer, 0.1));
        assert!(viewer.yaw > 0. && viewer.pitch > 0.);
        assert!(!viewer.orthographic);
        assert!(viewer.center.x < 0. && viewer.distance < 4.);
        let mut inverse = Viewer {
            fitted: true,
            distance: 4.,
            radius: 1.,
            ..default()
        };
        state.prefs.invert_rotate = true;
        state.prefs.invert_pan = true;
        state.prefs.invert_zoom = true;
        assert!(state.drive(&mut inverse, 0.1));
        assert!(inverse.yaw < 0. && inverse.pitch < 0.);
        assert!(inverse.center.x > 0. && inverse.distance > 4.);
        state.frame.pressed = 2;
        let center = inverse.center;
        assert!(state.drive(&mut inverse, 0.1));
        assert!(!inverse.fitted);
        assert_eq!(inverse.center, center);
        state.prefs.enabled = false;
        assert!(!state.drive(&mut inverse, 0.1));
    }

    #[test]
    fn spacemouse_restores_user_settings() {
        let root = std::env::temp_dir().join(format!(
            "bevy-space-mouse-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        let path = root.join("space-mouse.json");
        let prefs = Preferences {
            enabled: false,
            speed: 28,
            invert_rotate: true,
            invert_pan: true,
            invert_zoom: true,
        };
        prefs.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap(), prefs);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "probes the local HID bus for a physical 3D mouse"]
    fn spacemouse_probe_device() {
        match open_device() {
            Ok((name, _)) => println!("SpaceMouse: {name}"),
            Err(DeviceStatus::Missing) => println!("SpaceMouse: no device attached"),
            Err(DeviceStatus::Error(error)) => panic!("{error}"),
            _ => unreachable!(),
        }
    }
}
