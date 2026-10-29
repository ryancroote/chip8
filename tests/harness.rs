//! Sanity checks for the harness itself: these should pass regardless of how
//! much of the emulator is implemented, and they demonstrate that state can be
//! constructed, loaded, mutated, and inspected deterministically.

mod common;

use chip8::chip8::{KeyInput, State};

#[test]
fn fresh_state_is_zeroed_except_for_font() {
    let state = State::new();

    assert_eq!(state.pc, 512);
    assert_eq!(state.i, 0);
    assert_eq!(state.v, [0u8; 16]);
    assert_eq!(state.delay, 0);
    assert_eq!(state.sound, 0);
    assert!(state.stack.is_empty());
    assert!(state.keypad_input.iter().all(|pressed| !pressed));

    let display = state.display;
    assert!(display.iter().all(|row| row.iter().all(|pixel| !pixel)));

    // Memory before the font is zero, and the digit '0' is present at 0x50.
    let memory = state.memory;
    assert!(memory[..80].iter().all(|byte| *byte == 0));
    assert_eq!(&memory[80..85], &[0xF0, 0x90, 0x90, 0x90, 0xF0]);
}

#[test]
fn harness_can_load_program_and_snapshot() {
    let mut emulator = State::new();
    emulator.load_program(&[0x12, 0x34], 0x200);

    let memory = emulator.clone().memory;
    assert_eq!(memory[0x200], 0x12);
    assert_eq!(memory[0x201], 0x34);
}

#[test]
fn harness_can_inject_key_state() {
    let mut emulator = State::new();
    emulator.set_key(KeyInput::A, true);

    let state = emulator.clone();
    assert!(state.keypad_input[KeyInput::A.index()]);
    assert!(!state.keypad_input[KeyInput::B.index()]);
}

#[test]
fn run_accepts_a_matching_state() {
    let mut case = common::new_case("empty program leaves initial state intact");
    case.expected.pc = Some(common::PROGRAM_START);
    common::run(case);
}

#[test]
fn program_words_are_written_big_endian() {
    let mut case = common::new_case("0x60FF is stored as 0x60 0xFF");
    case.program = vec![0x60FF];
    case.steps = 0;
    case.expected.memory = vec![(0x200, 0x60), (0x201, 0xFF)];
    common::run(case);
}
