use servo::{
    Code, CompositionEvent, CompositionState, ImeEvent, InputEvent, Key, KeyState,
    KeyboardEvent as ServoKeyboardEvent, Location, Modifiers as ServoModifiers, NamedKey,
};

use crate::types::{ButtonState, Modifiers};

/// Translate Neroa's renderer-independent keyboard contract into Servo 0.5's
/// published keyboard-types contract.
///
/// Neroa keeps physical and logical key values as canonical UI Events strings.
/// keyboard-types 0.8.3 parses those strings directly; unknown values fail
/// closed to Unidentified rather than inventing a key.
pub(crate) fn keyboard_input(
    physical_code: &str,
    logical_key: &str,
    state: ButtonState,
    modifiers: Modifiers,
) -> InputEvent {
    let code = physical_code.parse::<Code>().unwrap_or(Code::Unidentified);
    let key = logical_key
        .parse::<Key>()
        .unwrap_or(Key::Named(NamedKey::Unidentified));
    let state = match state {
        ButtonState::Pressed => KeyState::Down,
        ButtonState::Released => KeyState::Up,
    };

    InputEvent::Keyboard(ServoKeyboardEvent::new_without_event(
        state,
        key,
        code,
        location_from_code(physical_code),
        servo_modifiers(modifiers),
        false,
        false,
    ))
}

/// `BrowserInput::Text` represents committed text, including IME output.
/// It is intentionally not a CPU/rendering fallback and does not alter the
/// renderer-independent input contract.
pub(crate) fn committed_text_input(text: String) -> InputEvent {
    InputEvent::Ime(ImeEvent::Composition(CompositionEvent {
        state: CompositionState::End,
        data: text,
    }))
}

fn location_from_code(physical_code: &str) -> Location {
    if physical_code.starts_with("Numpad") {
        Location::Numpad
    } else if physical_code.ends_with("Left") {
        Location::Left
    } else if physical_code.ends_with("Right") {
        Location::Right
    } else {
        Location::Standard
    }
}

fn servo_modifiers(modifiers: Modifiers) -> ServoModifiers {
    let mut servo = ServoModifiers::empty();
    servo.set(ServoModifiers::SHIFT, modifiers.shift);
    servo.set(ServoModifiers::CONTROL, modifiers.control);
    servo.set(ServoModifiers::ALT, modifiers.alt);
    servo.set(ServoModifiers::META, modifiers.meta);
    servo
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_key_strings_translate_to_servo_keyboard_event() {
        let event = keyboard_input(
            "KeyA",
            "a",
            ButtonState::Pressed,
            Modifiers {
                shift: true,
                control: false,
                alt: false,
                meta: false,
            },
        );

        let InputEvent::Keyboard(event) = event else {
            panic!("expected keyboard event");
        };
        assert_eq!(event.event.state, KeyState::Down);
        assert_eq!(event.event.code, Code::KeyA);
        assert_eq!(event.event.key, Key::Character("a".into()));
        assert_eq!(event.event.location, Location::Standard);
        assert!(event.event.modifiers.contains(ServoModifiers::SHIFT));
    }

    #[test]
    fn unknown_key_strings_fail_closed_to_unidentified() {
        let event = keyboard_input(
            "NeroaUnknownPhysicalKey",
            "NeroaUnknownLogicalKey",
            ButtonState::Released,
            Modifiers::default(),
        );

        let InputEvent::Keyboard(event) = event else {
            panic!("expected keyboard event");
        };
        assert_eq!(event.event.state, KeyState::Up);
        assert_eq!(event.event.code, Code::Unidentified);
        assert_eq!(event.event.key, Key::Named(NamedKey::Unidentified));
    }

    #[test]
    fn location_is_derived_from_canonical_physical_code() {
        assert_eq!(location_from_code("ShiftLeft"), Location::Left);
        assert_eq!(location_from_code("ControlRight"), Location::Right);
        assert_eq!(location_from_code("Numpad7"), Location::Numpad);
        assert_eq!(location_from_code("KeyQ"), Location::Standard);
    }

    #[test]
    fn text_input_is_committed_ime_composition() {
        let event = committed_text_input("hello".into());
        let InputEvent::Ime(ImeEvent::Composition(event)) = event else {
            panic!("expected committed IME composition");
        };
        assert_eq!(event.state, CompositionState::End);
        assert_eq!(event.data, "hello");
    }
}
