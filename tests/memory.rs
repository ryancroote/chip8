//! Memory load/store instructions.

mod common;

use common::new_case;

#[test]
fn store_registers_to_memory() {
    // FX55 writes V0..VX starting at I. Note: the post-store value of I is a
    // documented CHIP-8 quirk, so this test deliberately does not assert it.
    let mut case = new_case("FX55 stores V0..VX at I");
    case.initial.i = 0x300;
    case.initial.v[0] = 0xAA;
    case.initial.v[1] = 0xBB;
    case.program = vec![0xF155];
    case.steps = 1;
    case.expected.memory = vec![(0x300, 0xAA), (0x301, 0xBB)];
    common::run(case);
}

#[test]
fn load_registers_from_memory() {
    let mut case = new_case("FX65 loads V0..VX from I");
    case.initial.i = 0x300;
    case.initial.memory[0x300] = 0xAA;
    case.initial.memory[0x301] = 0xBB;
    case.program = vec![0xF165];
    case.steps = 1;
    case.expected.registers = vec![(0, 0xAA), (1, 0xBB)];
    common::run(case);
}

#[test]
fn jump_with_offset_uses_v0() {
    // BNNN: PC = NNN + V0. Some variants use Vx instead of V0; this encodes the
    // common (V0) behavior and should be revisited if the project pins another
    // quirk.
    let mut case = new_case("BNNN jumps to NNN + V0");
    case.initial.v[0] = 0x10;
    case.program = vec![0xB300];
    case.steps = 1;
    case.expected.pc = Some(0x310);
    common::run(case);
}
