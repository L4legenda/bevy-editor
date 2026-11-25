//! Windows Ink drops the side-button flag when winit emits touch input.
use super::paint_input::PenPan;
use bevy::prelude::*;
use std::cell::Cell;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::ScreenToClient,
    UI::{
        Input::Pointer::{GetPointerPenInfo, POINTER_FLAG_CANCELED, POINTER_PEN_INFO},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            PEN_FLAG_BARREL, WM_KILLFOCUS, WM_NCDESTROY,
            WM_POINTERCAPTURECHANGED, WM_POINTERDOWN, WM_POINTERLEAVE, WM_POINTERUP,
            WM_POINTERUPDATE,
        },
    },
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[derive(Clone, Copy, Default)]
struct State {
    pan: Option<(u32, PenPan)>,
    handled: bool,
}

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) };
    static STATE: Cell<State> = Cell::default();
}

pub(super) fn poll(entity: Entity, scale: f32) -> (Option<PenPan>, bool) {
    let installed = bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        let raw = windows.get_window(entity)?.window_handle().ok()?.as_raw();
        let RawWindowHandle::Win32(handle) = raw else { return None };
        let hwnd = handle.hwnd.get() as HWND;
        if WINDOW.get() != hwnd {
            // SAFETY: winit owns this HWND and collect_frame runs on its window thread.
            if unsafe { SetWindowSubclass(hwnd, Some(watch_pen), 0, 0) } == 0 {
                return None;
            }
            STATE.set(State::default());
            WINDOW.set(hwnd);
        }
        Some(())
    });
    if installed.is_none() {
        return (None, false);
    }
    let mut state = STATE.get();
    let pan = state.pan.map(|(_, pan)| PenPan {
        start: pan.start.map(|point| point / scale),
        point: pan.point / scale,
    });
    if let Some((_, pan)) = &mut state.pan {
        pan.start = None;
    }
    let handled = state.handled || pan.is_some();
    state.handled = false;
    STATE.set(state);
    (pan, handled)
}

unsafe extern "system" fn watch_pen(
    hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, id: usize, _: usize,
) -> LRESULT {
    if hwnd == WINDOW.get() {
        let mut state = STATE.get();
        let pointer = (wparam & 0xffff) as u32;
        match message {
            WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
                let mut info = POINTER_PEN_INFO::default();
                // SAFETY: the pointer ID comes from the current pointer message.
                if unsafe { GetPointerPenInfo(pointer, &mut info) } != 0 {
                    let held = info.penFlags & PEN_FLAG_BARREL != 0
                        && info.pointerInfo.pointerFlags & POINTER_FLAG_CANCELED == 0;
                    let mut point = info.pointerInfo.ptPixelLocation;
                    // SAFETY: the HWND is the live window receiving the pointer message.
                    if held && unsafe { ScreenToClient(hwnd, &mut point) } != 0 {
                        let point = Vec2::new(point.x as f32, point.y as f32);
                        let start = state.pan
                            .filter(|(owner, _)| *owner == pointer)
                            .map_or(Some(point), |(_, pan)| pan.start);
                        state.pan = Some((pointer, PenPan { start, point }));
                        state.handled = true;
                    } else if state.pan.is_some_and(|(owner, _)| owner == pointer) {
                        state.pan = None;
                        state.handled = true;
                    }
                }
            }
            WM_POINTERLEAVE | WM_POINTERCAPTURECHANGED
                if state.pan.is_some_and(|(owner, _)| owner == pointer) =>
            {
                state.pan = None;
                state.handled = true;
            }
            WM_KILLFOCUS | WM_NCDESTROY => state = State::default(),
            _ => {}
        }
        STATE.set(state);
    }
    if message == WM_NCDESTROY {
        if hwnd == WINDOW.get() {
            WINDOW.set(std::ptr::null_mut());
        }
        // SAFETY: remove only our callback from the window being destroyed.
        unsafe { RemoveWindowSubclass(hwnd, Some(watch_pen), id) };
    }
    // SAFETY: forward the original message through the installed subclass chain.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}
