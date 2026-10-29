//! Timer behavior.
//!
//! Timers are advanced through the explicit `tick_timers()` hook, so these
//! tests are independent of wall-clock time and of how many instructions ran.

mod common;

use common::new_case;

#[test]
fn delay_and_sound_timers_tick_down() {
    let mut case = new_case("delay and sound timers decrement per tick");
    case.initial.delay = 3;
    case.initial.sound = 2;
    case.timer_ticks = 2;
    case.expected.delay = Some(1);
    case.expected.sound = Some(0);
    common::run(case);
}

#[test]
fn timers_saturate_at_zero() {
    let mut case = new_case("timers do not underflow below zero");
    case.initial.delay = 1;
    case.initial.sound = 0;
    case.timer_ticks = 5;
    case.expected.delay = Some(0);
    case.expected.sound = Some(0);
    common::run(case);
}

#[test]
fn fx07_reads_delay_into_vx() {
    let mut case = new_case("FX07 copies the delay timer into Vx");
    case.initial.delay = 0x2A;
    case.program = vec![0xF007];
    case.steps = 1;
    case.expected.registers = vec![(0, 0x2A)];
    common::run(case);
}

#[test]
fn fx15_sets_delay_from_vx() {
    let mut case = new_case("FX15 sets the delay timer from Vx");
    case.initial.v[0] = 0x2A;
    case.program = vec![0xF015];
    case.steps = 1;
    case.expected.delay = Some(0x2A);
    common::run(case);
}

#[test]
fn fx18_sets_sound_from_vx() {
    let mut case = new_case("FX18 sets the sound timer from Vx");
    case.initial.v[0] = 0x05;
    case.program = vec![0xF018];
    case.steps = 1;
    case.expected.sound = Some(0x05);
    common::run(case);
}
