mod common;

use chip8::chip8::KeyInput;
use common::new_case;

#[test]
fn register_logic_and_subtract_instructions() {
    let mut case = new_case("8XY instructions implement logic, subtraction, and shifts");
    case.initial.v[1] = 0xF0;
    case.initial.v[2] = 0x0F;
    case.program = vec![
        0x8010, 0x8011, 0x8012, 0x8013, 0x8134, 0x8135, 0x8127, 0x8126, 0x812E,
    ];
    case.steps = case.program.len() as u32;
    case.expected.registers = vec![(0, 0x00), (1, 0x1E), (2, 0x0F), (0xF, 0)];
    common::run(case);
}

#[test]
fn fx0a_waits_then_stores_first_pressed_key() {
    let mut state = chip8::chip8::State::new();
    state.load_program(&[0xF0, 0x0A], 0x200);
    state.step().unwrap();
    assert_eq!(state.pc, 0x200);
    state.set_key(KeyInput::C, true);
    state.step().unwrap();
    assert_eq!(state.v[0], 0xC);
    assert_eq!(state.pc, 0x202);
}

#[test]
fn index_and_bcd_instructions() {
    let mut case = new_case("Fx1E, Fx29, and Fx33 behavior");
    case.initial.i = 0xFFF;
    case.initial.v[0] = 255;
    case.initial.v[1] = 0xA;
    case.program = vec![0xF01E, 0xF129, 0xA300, 0xF033];
    case.steps = 4;
    case.expected.i = Some(0x300); // BCD stores at the selected address.
    case.expected.memory = vec![(0x300, 2), (0x301, 5), (0x302, 5)];
    common::run(case);
}

#[test]
fn register_load_store_leave_i_unchanged() {
    let mut case = new_case("Fx55 and Fx65 preserve I");
    case.initial.i = 0x300;
    case.initial.v[0] = 0x12;
    case.program = vec![0xF055, 0x6000, 0xF065];
    case.steps = 3;
    case.expected.i = Some(0x300);
    case.expected.registers = vec![(0, 0x12)];
    case.expected.memory = vec![(0x300, 0x12)];
    common::run(case);
}
