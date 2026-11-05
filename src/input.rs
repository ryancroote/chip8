//! Host keyboard input for the graphical CHIP-8 keypad.

use minifb::Key;

use crate::chip8::{KeyInput, State};

/// Apply the currently held host keys to the CHIP-8 keypad.
///
/// Passing `focused = false` releases every keypad key, preventing stale input
/// after the emulator window loses focus.
pub fn update_keypad_input(state: &mut State, focused: bool, host_keys: &[Key]) {
    state.keypad_input = [false; 16];
    if !focused {
        return;
    }

    for host_key in host_keys {
        if let Some(key) = chip8_key(*host_key) {
            state.set_key(key, true);
        }
    }
}

fn chip8_key(host_key: Key) -> Option<KeyInput> {
    Some(match host_key {
        Key::Key1 => KeyInput::One,
        Key::Key2 => KeyInput::Two,
        Key::Key3 => KeyInput::Three,
        Key::Key4 => KeyInput::C,
        Key::Q => KeyInput::Four,
        Key::W => KeyInput::Five,
        Key::E => KeyInput::Six,
        Key::R => KeyInput::D,
        Key::A => KeyInput::Seven,
        Key::S => KeyInput::Eight,
        Key::D => KeyInput::Nine,
        Key::F => KeyInput::E,
        Key::Z => KeyInput::A,
        Key::X => KeyInput::Zero,
        Key::C => KeyInput::B,
        Key::V => KeyInput::F,
        _ => return None,
    })
}
