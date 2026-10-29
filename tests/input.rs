//! Keypad input handling.

mod common;

use chip8::chip8::{KeyInput, State};
use common::new_case;

#[test]
fn key_input_variants_map_to_their_hex_index() {
    assert_eq!(KeyInput::Zero.index(), 0x0);
    assert_eq!(KeyInput::Nine.index(), 0x9);
    assert_eq!(KeyInput::A.index(), 0xA);
    assert_eq!(KeyInput::F.index(), 0xF);
}

#[test]
fn harness_can_inject_key_state() {
    let mut emulator = State::new();
    emulator.set_key(KeyInput::A, true);
    emulator.set_key(KeyInput::B, false);

    let state = emulator.clone();
    assert!(state.keypad_input[KeyInput::A.index()]);
    assert!(!state.keypad_input[KeyInput::B.index()]);
}

#[test]
fn ex9e_skips_when_key_is_pressed() {
    // Key A is held, so 0xE09E skips the jump and runs 0x6001.
    let mut case = new_case("EX9E skips the next instruction when the key is held");
    case.initial.v[0] = 0xA;
    case.initial.keypad_input[KeyInput::A.index()] = true;
    case.program = vec![0xE09E, 0x1234, 0x6001];
    case.steps = 2;
    case.expected.registers = vec![(0, 0x01)];
    common::run(case);
}

#[test]
fn exa1_skips_when_key_is_not_pressed() {
    // Key A is not held, so 0xE0A1 skips the jump and runs 0x6001.
    let mut case = new_case("EXA1 skips the next instruction when the key is not held");
    case.initial.v[0] = 0xA;
    case.program = vec![0xE0A1, 0x1234, 0x6001];
    case.steps = 2;
    case.expected.registers = vec![(0, 0x01)];
    common::run(case);
}
