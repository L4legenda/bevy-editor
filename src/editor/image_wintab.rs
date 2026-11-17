//! Optional legacy tablet input. All WinTab calls run on the window thread.
use super::paint_input::Sample;
use bevy::prelude::*;
use std::ffi::c_void;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

type Handle = *mut c_void;
type Info = unsafe extern "system" fn(u32, u32, Handle) -> u32;
type Open = unsafe extern "system" fn(Handle, *mut Context, i32) -> Handle;
type Close = unsafe extern "system" fn(Handle) -> i32;
type Enable = unsafe extern "system" fn(Handle, i32) -> i32;
type Packets = unsafe extern "system" fn(Handle, i32, Handle) -> i32;

// Wacom SDK LOGCONTEXTA / AXIS. WinTab LONG, UINT and FIX32 are 32-bit even on x64.
#[repr(C)]
struct Context {
    name: [u8; 40],
    options: u32,
    status: u32,
    locks: u32,
    msg_base: u32,
    device: u32,
    rate: u32,
    data: u32,
    mode: u32,
    movement: u32,
    buttons_down: u32,
    buttons_up: u32,
    input_origin: [i32; 3],
    input_extent: [i32; 3],
    output_origin: [i32; 3],
    output_extent: [i32; 3],
    sensitivity: [u32; 3],
    system_mode: i32,
    system_origin: [i32; 2],
    system_extent: [i32; 2],
    system_sensitivity: [u32; 2],
}

impl Default for Context {
    fn default() -> Self {
        // All fields are integers, so the C structure accepts an all-zero initial value.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Default)]
struct Axis {
    min: i32,
    max: i32,
    units: u32,
    resolution: u32,
}

// Packet members follow the ascending PK_* flags: X, Y, NORMAL_PRESSURE.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Packet {
    x: i32,
    y: i32,
    pressure: u32,
}

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExW(path: *const u16, file: Handle, flags: u32) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> Handle;
    fn FreeLibrary(module: Handle) -> i32;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn ScreenToClient(window: Handle, point: *mut Point) -> i32;
}

struct Driver {
    module: Handle,
    context: Handle,
    hwnd: Handle,
    close: Close,
    enable: Enable,
    packets: Packets,
    range: (i32, i32),
    focused: bool,
}

impl Drop for Driver {
    fn drop(&mut self) {
        // SAFETY: both handles belong to this driver, and functions stay loaded until close.
        unsafe {
            (self.close)(self.context);
            FreeLibrary(self.module);
        }
    }
}

impl Driver {
    fn open(hwnd: Handle) -> Option<Self> {
        // SAFETY: fixed system DLL name and system-directory-only search avoid project DLLs.
        unsafe {
            let path: Vec<u16> = "Wintab32.dll\0".encode_utf16().collect();
            let module = LoadLibraryExW(path.as_ptr(), std::ptr::null_mut(), 0x800);
            if module.is_null() {
                return None;
            }
            macro_rules! function {
                ($name:literal, $ty:ty) => {{
                    let address = GetProcAddress(module, concat!($name, "\0").as_ptr());
                    if address.is_null() {
                        FreeLibrary(module);
                        return None;
                    }
                    std::mem::transmute::<Handle, $ty>(address)
                }};
            }
            let info = function!("WTInfoA", Info);
            let open = function!("WTOpenA", Open);
            let close = function!("WTClose", Close);
            let enable = function!("WTEnable", Enable);
            let packets = function!("WTPacketsGet", Packets);
            let mut context = Context::default();
            let mut axis = Axis::default();
            if info(4, 0, (&mut context as *mut Context).cast()) != size_of::<Context>() as u32
                || info(100 + context.device, 15, (&mut axis as *mut Axis).cast())
                    != size_of::<Axis>() as u32
                || axis.max <= axis.min
            {
                FreeLibrary(module);
                return None;
            }
            // System context preserves the driver's monitor mapping and screen coordinates.
            // Poll its queue; do not inject WT_PACKET messages into winit's event loop.
            context.options = (context.options | 1) & !0xc;
            context.data = 0x80 | 0x100 | 0x400;
            context.mode = 0;
            context.movement = context.data;
            let handle = open(hwnd, &mut context, 0);
            if handle.is_null() {
                FreeLibrary(module);
                return None;
            }
            info!("WinTab tablet ready, pressure {}..{}", axis.min, axis.max);
            Some(Self {
                module,
                context: handle,
                hwnd,
                close,
                enable,
                packets,
                range: (axis.min, axis.max),
                focused: false,
            })
        }
    }
}

#[derive(Default)]
pub(super) struct Tablet {
    driver: Option<Driver>,
    retry: Option<std::time::Instant>,
    contact: bool,
    position: Vec2,
    pressure: f32,
}

thread_local! { static TABLET: std::cell::RefCell<Tablet> = std::cell::RefCell::default(); }

pub(super) fn poll(window: Entity) -> (Option<Vec<Sample>>, bool) {
    TABLET.with_borrow_mut(|tablet| {
        let samples = tablet.poll(window);
        (samples, tablet.down())
    })
}

impl Tablet {
    pub(super) fn down(&self) -> bool {
        self.contact
    }

    pub(super) fn poll(&mut self, window: Entity) -> Option<Vec<Sample>> {
        bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
            let window = windows.get_window(window)?;
            let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
                return None;
            };
            let hwnd = handle.hwnd.get() as Handle;
            if self.driver.as_ref().is_some_and(|driver| driver.hwnd != hwnd) {
                self.driver = None;
                self.retry = None;
                self.contact = false;
            }
            if self.driver.is_none() {
                if self.retry.is_some_and(|time| time.elapsed().as_secs() < 5) {
                    return None;
                }
                self.retry = Some(std::time::Instant::now());
                self.driver = Driver::open(hwnd);
            }
            let driver = self.driver.as_mut()?;
            let focused = window.has_focus();
            if focused != driver.focused {
                // SAFETY: a valid context, accessed only by this main-thread system.
                unsafe {
                    (driver.enable)(driver.context, i32::from(focused));
                }
                driver.focused = focused;
            }
            let was_down = self.contact;
            let mut result = Vec::new();
            if !focused {
                if self.contact {
                    result.push(Sample::Up(self.position));
                }
                self.contact = false;
                return was_down.then_some(result);
            }
            let mut buffer = [Packet::default(); 128];
            // Drain in bounded batches to preserve corners and pressure within a frame.
            for _ in 0..8 {
                // SAFETY: buffer has capacity for precisely 128 packets of the requested layout.
                let count =
                    unsafe { (driver.packets)(driver.context, 128, buffer.as_mut_ptr().cast()) };
                if count <= 0 {
                    break;
                }
                for packet in &buffer[..(count as usize).min(buffer.len())] {
                    let mut point = Point {
                        x: packet.x,
                        y: packet.y,
                    };
                    // SAFETY: live window and writable POINT.
                    if unsafe { ScreenToClient(driver.hwnd, &mut point) } == 0 {
                        continue;
                    }
                    let position =
                        Vec2::new(point.x as f32, point.y as f32) / window.scale_factor() as f32;
                    let pressure = ((packet.pressure as f64 - driver.range.0 as f64)
                        / (driver.range.1 as f64 - driver.range.0 as f64))
                        .clamp(0.0, 1.0) as f32;
                    contact_samples(
                        &mut self.contact,
                        &mut self.pressure,
                        pressure,
                        position,
                        &mut result,
                    );
                    self.position = position;
                }
                if count < 128 {
                    break;
                }
            }
            (was_down || self.contact || !result.is_empty()).then_some(result)
        })
    }
}

fn contact_samples(
    down: &mut bool,
    last: &mut f32,
    pressure: f32,
    position: Vec2,
    out: &mut Vec<Sample>,
) {
    if pressure > 0.0 {
        out.push(Sample::Pressure(pressure));
        out.push(if *down {
            Sample::Move(position)
        } else {
            Sample::Down(position)
        });
        *down = true;
        *last = pressure;
    } else if *down {
        out.push(Sample::Pressure(*last));
        out.push(Sample::Up(position));
        *down = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wintab_hover_contact_pressure_and_release() {
        assert_eq!(size_of::<Context>(), 172);
        assert_eq!(size_of::<Packet>(), 12);
        let (mut down, mut pressure, mut samples) = (false, 0.0, Vec::new());
        for value in [0.0, 0.2, 0.8, 0.0, 0.0] {
            contact_samples(&mut down, &mut pressure, value, Vec2::ONE, &mut samples);
        }
        assert_eq!(
            samples,
            vec![
                Sample::Pressure(0.2),
                Sample::Down(Vec2::ONE),
                Sample::Pressure(0.8),
                Sample::Move(Vec2::ONE),
                Sample::Pressure(0.8),
                Sample::Up(Vec2::ONE)
            ]
        );
        assert!(!down);
    }
}
