//! Recover every OS pointer sample in press..release order instead of sampling once per frame.
use bevy::input::touch::{ForceTouch, TouchPhase};
use bevy::{input::ButtonState, prelude::*, window::WindowEvent};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Sample {
    Pressure(f32),
    Down(Vec2),
    Move(Vec2),
    Up(Vec2),
}

#[derive(Default)]
pub(in crate::editor) struct Samples {
    position: Option<Vec2>,
    down: bool,
    pen: Option<u64>,
    pressure: f32,
    native_down: bool,
    pen_cursor: Option<Vec2>,
}

fn pressure(force: ForceTouch) -> f32 {
    let value = match force {
        ForceTouch::Normalized(value) => value,
        ForceTouch::Calibrated {
            force,
            max_possible_force,
            ..
        } => force / max_possible_force,
    };
    if value.is_finite() {
        value.clamp(0.0, 1.0) as f32
    } else {
        1.0
    }
}

impl Samples {
    pub(super) fn pen_cursor(&self) -> Option<Vec2> {
        self.pen_cursor
    }
    pub(super) fn pen_down(&self) -> bool {
        self.native_down || (self.pen.is_some() && self.down)
    }
    pub(super) fn collect<'a>(
        &mut self,
        events: impl Iterator<Item = &'a WindowEvent>,
        window: Entity,
        cursor: Option<Vec2>,
        mouse: &ButtonInput<MouseButton>,
    ) -> Vec<Sample> {
        let events: Vec<_> = events.collect();
        // Windows Ink reports pen hover as TouchInput with no force. It does
        // not have to update Window::cursor_position (the mouse pointer).
        let pen_frame = events
            .iter()
            .any(|event| matches!(event, WindowEvent::TouchInput(touch) if touch.window == window));
        for event in &events {
            match event {
                WindowEvent::TouchInput(touch) if touch.window == window => {
                    self.pen_cursor =
                        (touch.phase != TouchPhase::Canceled).then_some(touch.position);
                }
                WindowEvent::CursorMoved(event) if event.window == window && !pen_frame => {
                    self.pen_cursor = None;
                }
                WindowEvent::CursorLeft(event) if event.window == window => {
                    self.pen_cursor = None;
                }
                WindowEvent::WindowFocused(event) if event.window == window && !event.focused => {
                    self.pen_cursor = None;
                }
                _ => {}
            }
        }
        #[cfg(target_os = "windows")]
        {
            let (native, down) = super::wintab::poll(window);
            self.native_down = down;
            if let Some(samples) = native {
                for sample in &samples {
                    if let Sample::Down(point) | Sample::Move(point) | Sample::Up(point) = sample {
                        self.pen_cursor = Some(*point);
                    }
                }
                self.pen = None;
                self.down = down;
                return samples;
            }
        }
        let pen_frame = self.pen.is_some()
            || events.iter().any(|event| {
                matches!(event,
            WindowEvent::TouchInput(touch) if touch.window == window && touch.force.is_some())
            });
        let mut samples = Vec::new();
        let mut button_event = false;
        for event in events {
            match event {
                WindowEvent::CursorMoved(event) if event.window == window => {
                    if pen_frame {
                        continue;
                    }
                    self.position = Some(event.position);
                    if self.down {
                        samples.push(Sample::Move(event.position));
                    }
                }
                WindowEvent::MouseButtonInput(event)
                    if event.window == window && event.button == MouseButton::Left =>
                {
                    if pen_frame {
                        continue;
                    }
                    button_event = true;
                    self.down = event.state == ButtonState::Pressed;
                    if let Some(point) = self.position.or(cursor) {
                        samples.push(if self.down {
                            Sample::Down(point)
                        } else {
                            Sample::Up(point)
                        });
                    }
                }
                WindowEvent::WindowFocused(event) if event.window == window && !event.focused => {
                    self.down = false;
                    self.position = None;
                    self.pen = None;
                }
                WindowEvent::TouchInput(touch)
                    if touch.window == window
                        && (self.pen == Some(touch.id)
                            || (self.pen.is_none()
                                && touch.force.is_some()
                                && matches!(
                                    touch.phase,
                                    TouchPhase::Started | TouchPhase::Moved
                                ))) =>
                {
                    // Some Windows drivers report zero pressure (represented by
                    // winit as None) on DOWN; the first real pressure arrives on MOVE.
                    if touch.force.is_some() && self.pen.is_none() {
                        self.pen = Some(touch.id);
                        self.down = true;
                        self.pressure = touch.force.map(pressure).unwrap_or(1.0);
                        samples.push(Sample::Pressure(self.pressure));
                        samples.push(Sample::Down(touch.position));
                    } else if self.pen == Some(touch.id) {
                        if touch.phase == TouchPhase::Moved {
                            self.pressure = touch.force.map(pressure).unwrap_or(self.pressure);
                            samples.push(Sample::Pressure(self.pressure));
                            samples.push(Sample::Move(touch.position));
                        } else if matches!(touch.phase, TouchPhase::Ended | TouchPhase::Canceled) {
                            samples.push(Sample::Pressure(self.pressure));
                            samples.push(Sample::Up(touch.position));
                            self.pen = None;
                            self.down = false;
                        }
                    }
                    self.position = Some(touch.position);
                }
                _ => {}
            }
        }
        // Programmatic/headless callers can drive Window and typed input without WindowEvent.
        if !button_event && !pen_frame {
            if let Some(point) = cursor {
                if mouse.just_pressed(MouseButton::Left) && !self.down {
                    samples.push(Sample::Down(point));
                    self.down = true;
                }
                if self.down {
                    samples.push(Sample::Move(point));
                }
            }
            // The pointer can leave the OS window, so release must also end the
            // capture when Window no longer has a cursor position.
            if self.down
                && (mouse.just_released(MouseButton::Left) || !mouse.pressed(MouseButton::Left))
            {
                if let Some(point) = cursor.or(self.position) {
                    samples.push(Sample::Up(point));
                }
                self.down = false;
            }
        }
        if !pen_frame {
            self.position = cursor;
        }
        samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{input::mouse::MouseButtonInput, window::CursorMoved};

    #[test]
    fn pen_contact_can_start_when_pressure_first_arrives_on_move() {
        use bevy::input::touch::TouchInput;
        let window = Entity::PLACEHOLDER;
        let mut samples = Samples::default();
        let mouse = ButtonInput::default();
        let event = |phase, force| {
            WindowEvent::TouchInput(TouchInput {
                window,
                id: 3,
                phase,
                force,
                position: Vec2::ONE,
            })
        };
        samples.collect(
            [event(TouchPhase::Started, None)].iter(),
            window,
            None,
            &mouse,
        );
        assert_eq!(
            samples.collect(
                [event(TouchPhase::Moved, Some(ForceTouch::Normalized(0.3)))].iter(),
                window,
                None,
                &mouse
            ),
            vec![Sample::Pressure(0.3), Sample::Down(Vec2::ONE)]
        );
        assert!(samples.pen_down());
    }

    #[test]
    fn pen_pressure_survives_frames_and_suppresses_promoted_mouse_events() {
        use bevy::input::touch::TouchInput;
        let window = Entity::PLACEHOLDER;
        let touch = |phase, force| {
            WindowEvent::TouchInput(TouchInput {
                window,
                id: 7,
                phase,
                position: Vec2::ONE,
                force,
            })
        };
        let mut input = Samples::default();
        let mouse = ButtonInput::default();
        let events = [
            touch(TouchPhase::Started, Some(ForceTouch::Normalized(0.25))),
            WindowEvent::MouseButtonInput(MouseButtonInput {
                window,
                button: MouseButton::Left,
                state: ButtonState::Pressed,
            }),
        ];
        assert_eq!(
            input.collect(events.iter(), window, None, &mouse),
            vec![Sample::Pressure(0.25), Sample::Down(Vec2::ONE)]
        );
        assert!(
            input
                .collect(std::iter::empty(), window, None, &mouse)
                .is_empty()
        );
        assert!(input.pen_down());
        assert_eq!(
            input.collect(
                [touch(TouchPhase::Moved, Some(ForceTouch::Normalized(0.8)))].iter(),
                window,
                None,
                &mouse
            ),
            vec![Sample::Pressure(0.8), Sample::Move(Vec2::ONE)]
        );
        assert_eq!(
            input.collect(
                [touch(TouchPhase::Ended, None)].iter(),
                window,
                None,
                &mouse
            ),
            vec![Sample::Pressure(0.8), Sample::Up(Vec2::ONE)]
        );
        assert!(!input.pen_down());
        assert_eq!(
            pressure(ForceTouch::Calibrated {
                force: 2.0,
                max_possible_force: 4.0,
                altitude_angle: None
            }),
            0.5
        );
        assert_eq!(pressure(ForceTouch::Normalized(f64::NAN)), 1.0);
    }

    #[test]
    fn release_without_cursor_position_ends_capture_before_the_next_press() {
        let window = Entity::PLACEHOLDER;
        let mut input = Samples::default();
        let mut mouse = ButtonInput::default();
        mouse.press(MouseButton::Left);
        input.collect(std::iter::empty(), window, Some(Vec2::ONE), &mouse);
        mouse.clear();
        // CursorLeft clears Window's position. Keep holding for a whole frame.
        assert!(
            input
                .collect(std::iter::empty(), window, None, &mouse)
                .is_empty()
        );
        assert!(input.down);
        mouse.release(MouseButton::Left);
        input.collect(std::iter::empty(), window, None, &mouse);
        assert!(!input.down);
        mouse.clear();
        mouse.press(MouseButton::Left);
        assert!(matches!(
            input
                .collect(std::iter::empty(), window, Some(Vec2::ONE), &mouse)
                .first(),
            Some(Sample::Down(_))
        ));
    }

    #[test]
    fn batched_pointer_samples_preserve_corners_and_press_release_boundaries() {
        let window = Entity::PLACEHOLDER;
        let movement = |x, y| {
            WindowEvent::CursorMoved(CursorMoved {
                window,
                position: Vec2::new(x, y),
                delta: None,
            })
        };
        let button = |state| {
            WindowEvent::MouseButtonInput(MouseButtonInput {
                window,
                button: MouseButton::Left,
                state,
            })
        };
        let events = [
            movement(1.0, 2.0),
            button(ButtonState::Pressed),
            movement(10.0, 2.0),
            movement(10.0, 20.0),
            button(ButtonState::Released),
            movement(50.0, 50.0),
        ];
        let samples = Samples::default().collect(
            events.iter(),
            window,
            Some(Vec2::splat(50.0)),
            &ButtonInput::default(),
        );
        assert_eq!(
            samples,
            [
                Sample::Down(Vec2::new(1.0, 2.0)),
                Sample::Move(Vec2::new(10.0, 2.0)),
                Sample::Move(Vec2::new(10.0, 20.0)),
                Sample::Up(Vec2::new(10.0, 20.0))
            ]
        );
    }
}
