//! Recover every OS pointer sample in press..release order instead of sampling once per frame.
use bevy::{input::ButtonState, prelude::*, window::WindowEvent};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Sample {
    Down(Vec2),
    Move(Vec2),
    Up(Vec2),
}

#[derive(Default)]
pub(in crate::editor) struct Samples {
    position: Option<Vec2>,
    down: bool,
}

impl Samples {
    pub(super) fn collect<'a>(
        &mut self,
        events: impl Iterator<Item = &'a WindowEvent>,
        window: Entity,
        cursor: Option<Vec2>,
        mouse: &ButtonInput<MouseButton>,
    ) -> Vec<Sample> {
        let mut samples = Vec::new();
        let mut button_event = false;
        for event in events {
            match event {
                WindowEvent::CursorMoved(event) if event.window == window => {
                    self.position = Some(event.position);
                    if self.down {
                        samples.push(Sample::Move(event.position));
                    }
                }
                WindowEvent::MouseButtonInput(event)
                    if event.window == window && event.button == MouseButton::Left =>
                {
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
                }
                _ => {}
            }
        }
        // Programmatic/headless callers can drive Window and typed input without WindowEvent.
        if !button_event {
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
        self.position = cursor;
        samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{input::mouse::MouseButtonInput, window::CursorMoved};

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
