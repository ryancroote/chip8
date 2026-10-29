//! Program counter, skips, jumps, and subroutine control flow.

mod common;

use common::new_case;

#[test]
fn pc_advances_by_two_per_instruction() {
    let mut case = new_case("PC advances by 2 for each sequential instruction");
    case.program = vec![0x6001, 0x6002];
    case.steps = 2;
    case.expected.pc = Some(0x204);
    common::run(case);
}

#[test]
fn jump_sets_pc() {
    let mut case = new_case("1NNN jumps to NNN");
    case.program = vec![0x1234];
    case.steps = 1;
    case.expected.pc = Some(0x234);
    common::run(case);
}

#[test]
fn skip_if_equal_skips_next_instruction() {
    // V0 = 5, so 0x3005 skips the 0x1234 jump and lands on 0x6001.
    let mut case = new_case("3XNN skips the next instruction when equal");
    case.initial.v[0] = 0x05;
    case.program = vec![0x3005, 0x1234, 0x6001];
    case.steps = 2;
    case.expected.registers = vec![(0, 0x01)];
    case.expected.pc = Some(0x206);
    common::run(case);
}

#[test]
fn call_and_return_roundtrip() {
    // At 0x200: CALL 0x345. The return instruction 0x00EE lives at 0x345.
    let mut case = new_case("2NNN calls and 00EE returns");
    case.program = vec![0x2345];
    case.initial.memory[0x345] = 0x00;
    case.initial.memory[0x346] = 0xEE;
    case.steps = 2;
    case.expected.pc = Some(0x202);
    case.expected.stack = Some(vec![]);
    common::run(case);
}
