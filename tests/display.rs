//! Display instructions: CLS and DRW.
//!
//! The project's display spec pins XOR drawing with wrapping on both axes and
//! VF as the collision flag, so these expectations use that behavior.

mod common;

use common::new_case;

#[test]
fn cls_clears_the_display() {
    let mut case = new_case("00E0 clears every pixel");
    case.initial.display[0][0] = true;
    case.initial.display[31][63] = true;
    case.program = vec![0x00E0];
    case.steps = 1;
    case.expected.pixels_off = vec![(0, 0), (5, 5), (31, 63)];
    common::run(case);
}

#[test]
fn draw_sets_pixels_and_clears_vf() {
    // Sprite 0xF0 at I=0x300, drawn at (V0,V1)=(0,0): pixels (0,0)..(0,3).
    let mut case = new_case("DXYN draws a sprite and clears VF on no collision");
    case.initial.v[0] = 0x00;
    case.initial.v[1] = 0x00;
    case.initial.v[0xF] = 0xFF;
    case.initial.i = 0x300;
    case.initial.memory[0x300] = 0xF0;
    case.program = vec![0xD011];
    case.steps = 1;
    case.expected.pixels_on = vec![(0, 0), (0, 1), (0, 2), (0, 3)];
    case.expected.registers = vec![(0xF, 0x00)];
    common::run(case);
}

#[test]
fn draw_collision_erases_pixels_and_sets_vf() {
    // Drawing the same sprite twice XORs it back off and reports a collision.
    let mut case = new_case("DXYN collision toggles pixels off and sets VF=1");
    case.initial.v[0] = 0x00;
    case.initial.v[1] = 0x00;
    case.initial.i = 0x300;
    case.initial.memory[0x300] = 0xF0;
    case.program = vec![0xD011, 0xD011];
    case.steps = 2;
    case.expected.pixels_off = vec![(0, 0), (0, 1), (0, 2), (0, 3)];
    case.expected.registers = vec![(0xF, 0x01)];
    common::run(case);
}

#[test]
fn draw_wraps_past_the_right_edge() {
    // V0 = 61, sprite 0xF0 lights columns 61, 62, 63 and wraps to column 0.
    let mut case = new_case("DXYN wraps at the right edge (mod 64)");
    case.initial.v[0] = 61;
    case.initial.v[1] = 0;
    case.initial.i = 0x300;
    case.initial.memory[0x300] = 0xF0;
    case.program = vec![0xD011];
    case.steps = 1;
    case.expected.pixels_on = vec![(0, 0), (0, 61), (0, 62), (0, 63)];
    common::run(case);
}
