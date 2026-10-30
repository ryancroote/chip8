//! Register and arithmetic instructions.
//!
//! Expected values come from standard CHIP-8 semantics.

mod common;

use common::new_case;

#[test]
fn registers_and_index_register_start_zeroed() {
    let mut case = new_case("V0..VF and I start at zero");
    case.steps = 0;
    case.expected.registers = (0..16).map(|index| (index, 0u8)).collect();
    case.expected.i = Some(0);
    common::run(case);
}

#[test]
fn ld_vx_byte_loads_immediate_then_add() {
    // 0x6005 -> V0 = 0x05 ; 0x7003 -> V0 = V0 + 0x03 = 0x08
    let mut case = new_case("6XNN loads immediate and 7XNN adds to it");
    case.program = vec![0x6005, 0x7003];
    case.steps = 2;
    case.expected.registers = vec![(0, 0x08)];
    case.expected.pc = Some(0x204);
    common::run(case);
}

#[test]
fn ld_vx_byte_only_touches_target_register() {
    let mut case = new_case("6XNN writes only Vx");
    case.program = vec![0x6107];
    case.steps = 1;
    case.expected.registers = vec![(0, 0x00), (1, 0x07), (2, 0x00)];
    common::run(case);
}

#[test]
fn add_with_carry_sets_vf() {
    // 0xF0 + 0x20 = 0x110 -> V0 = 0x10, VF = 1
    let mut case = new_case("8XY4 sets VF=1 when the sum carries");
    case.initial.v[0] = 0xF0;
    case.initial.v[1] = 0x20;
    case.program = vec![0x8014];
    case.steps = 1;
    case.expected.registers = vec![(0, 0x10), (0xF, 0x01)];
    common::run(case);
}

#[test]
fn add_without_carry_clears_vf() {
    let mut case = new_case("8XY4 sets VF=0 when the sum does not carry");
    case.initial.v[0] = 0x10;
    case.initial.v[1] = 0x20;
    case.initial.v[0xF] = 0xFF;
    case.program = vec![0x8014];
    case.steps = 1;
    case.expected.registers = vec![(0, 0x30), (0xF, 0x00)];
    common::run(case);
}

#[test]
fn ld_i_loads_immediate() {
    let mut case = new_case("ANNN sets I to NNN");
    case.program = vec![0xA123];
    case.steps = 1;
    case.expected.i = Some(0x123);
    common::run(case);
}
