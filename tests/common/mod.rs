//! Shared deterministic test harness for the CHIP-8 emulator.
//!
//! This module knows nothing about instruction semantics. It only knows how to
//! build a known `State`, load program words, call `State::step()` a fixed
//! number of times, advance timers explicitly, and compare cloned state against
//! the fields a test chose to assert. `State` has public fields, so tests read
//! and write it directly.
//!
//! Each test file includes this module with `mod common;`.

#![allow(dead_code)]

use chip8::chip8::{KeyInput, State};

/// Default test program origin. This is a *test harness* choice, not an
/// assumption about where the emulator loads ROMs. Tests keep `initial.pc`
/// equal to `load_at` unless they are deliberately testing control flow.
pub const PROGRAM_START: u16 = 0x200;

/// State fields a test wants to verify. Only the fields a test populates are
/// checked; everything else is ignored.
#[derive(Default)]
pub struct Expected {
    /// (register index, value)
    pub registers: Vec<(usize, u8)>,
    pub i: Option<u16>,
    pub pc: Option<u16>,
    pub stack: Option<Vec<u16>>,
    /// (address, value)
    pub memory: Vec<(u16, u8)>,
    /// (start address, bytes)
    pub memory_ranges: Vec<(u16, Vec<u8>)>,
    /// (row, column) expected ON
    pub pixels_on: Vec<(usize, usize)>,
    /// (row, column) expected OFF
    pub pixels_off: Vec<(usize, usize)>,
    pub delay: Option<u8>,
    pub sound: Option<u8>,
    /// (key, expected pressed)
    pub keys: Vec<(KeyInput, bool)>,
}

pub struct TestCase {
    pub name: &'static str,
    pub initial: State,
    /// Big-endian opcodes written contiguously starting at `load_at`.
    pub program: Vec<u16>,
    pub load_at: u16,
    /// Number of `State::step()` calls to perform.
    pub steps: u32,
    /// Number of `State::tick_timers()` calls to perform after stepping.
    pub timer_ticks: u32,
    pub expected: Expected,
}

/// Start a test case from a zeroed machine with the PC at `PROGRAM_START`.
pub fn new_case(name: &'static str) -> TestCase {
    let mut initial = State::new();
    initial.pc = PROGRAM_START;
    TestCase {
        name,
        initial,
        program: Vec::new(),
        load_at: PROGRAM_START,
        steps: 0,
        timer_ticks: 0,
        expected: Expected::default(),
    }
}

/// Execute a test case and panic with a structured report on any mismatch.
pub fn run(case: TestCase) {
    let mut emulator = case.initial.clone();

    for (index, opcode) in case.program.iter().enumerate() {
        let address = case.load_at + (index as u16) * 2;
        emulator.load_program(&opcode.to_be_bytes(), address);
    }

    for step in 0..case.steps {
        match emulator.step() {
            Ok(()) => {}
            Err(error) => panic!(
                "FAIL: {}\nEmulator returned an error on step {} (instruction 0x{:04X}): {:?}",
                case.name,
                step + 1,
                case.program.get(step as usize).copied().unwrap_or(0),
                error,
            ),
        }
    }

    for _ in 0..case.timer_ticks {
        emulator.tick_timers();
    }

    let state = emulator.clone();
    let failures = check(&case, &state);
    if !failures.is_empty() {
        panic!("{}", report(&case, &state, &failures));
    }
}

fn check(case: &TestCase, state: &State) -> Vec<String> {
    let mut failures = Vec::new();
    let expected = &case.expected;

    for &(index, value) in &expected.registers {
        let actual = state.v[index];
        if actual != value {
            failures.push(format!(
                "  V{}: expected 0x{:02X}, actual 0x{:02X}",
                index, value, actual
            ));
        }
    }

    if let Some(value) = expected.i {
        if state.i != value {
            failures.push(format!(
                "  I: expected 0x{:03X}, actual 0x{:03X}",
                value, state.i
            ));
        }
    }

    if let Some(value) = expected.pc {
        if state.pc != value {
            failures.push(format!(
                "  PC: expected 0x{:03X}, actual 0x{:03X}",
                value, state.pc
            ));
        }
    }

    if let Some(expected_stack) = &expected.stack {
        if &state.stack != expected_stack {
            failures.push(format!(
                "  Stack: expected {:?}, actual {:?}",
                expected_stack, state.stack
            ));
        }
    }

    for &(address, value) in &expected.memory {
        let actual = state.memory[address as usize];
        if actual != value {
            failures.push(format!(
                "  mem[0x{:03X}]: expected 0x{:02X}, actual 0x{:02X}",
                address, value, actual
            ));
        }
    }

    for (address, bytes) in &expected.memory_ranges {
        let start = *address as usize;
        let actual = &state.memory[start..start + bytes.len()];
        if actual != bytes.as_slice() {
            failures.push(format!(
                "  mem[0x{:03X}..0x{:03X}]: expected {:02X?}, actual {:02X?}",
                address,
                address + bytes.len() as u16,
                bytes,
                actual
            ));
        }
    }

    for &(row, column) in &expected.pixels_on {
        if !state.display[row][column] {
            failures.push(format!(
                "  pixel(row={}, col={}): expected ON, actual OFF",
                row, column
            ));
        }
    }

    for &(row, column) in &expected.pixels_off {
        if state.display[row][column] {
            failures.push(format!(
                "  pixel(row={}, col={}): expected OFF, actual ON",
                row, column
            ));
        }
    }

    if let Some(value) = expected.delay {
        if state.delay != value {
            failures.push(format!(
                "  Delay timer: expected {}, actual {}",
                value, state.delay
            ));
        }
    }

    if let Some(value) = expected.sound {
        if state.sound != value {
            failures.push(format!(
                "  Sound timer: expected {}, actual {}",
                value, state.sound
            ));
        }
    }

    for &(key, value) in &expected.keys {
        let actual = state.keypad_input[key.index()];
        if actual != value {
            failures.push(format!(
                "  key {:?}: expected {}, actual {}",
                key, value, actual
            ));
        }
    }

    failures
}

fn report(case: &TestCase, state: &State, failures: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&format!("\nFAIL: {}\n", case.name));

    out.push_str(&format!("Program (loaded at 0x{:03X}):\n", case.load_at));
    if case.program.is_empty() {
        out.push_str("  <none>\n");
    } else {
        for (index, opcode) in case.program.iter().enumerate() {
            out.push_str(&format!(
                "  [{}] 0x{:03X}: 0x{:04X}\n",
                index,
                case.load_at + (index as u16) * 2,
                opcode
            ));
        }
    }

    out.push_str(&format!("Instructions executed: {}\n", case.steps));
    if case.steps > 0 {
        if let Some(opcode) = case.program.get(case.steps as usize - 1) {
            out.push_str(&format!(
                "Instruction associated with failure: 0x{:04X} at 0x{:03X} (step {})\n",
                opcode,
                case.load_at + (case.steps as u16 - 1) * 2,
                case.steps
            ));
        }
    }

    out.push_str("\nExpected vs actual:\n");
    for failure in failures {
        out.push_str(failure);
        out.push('\n');
    }

    out.push_str("\nMachine state:\n");
    out.push_str(&format!(
        "  PC=0x{:03X}  I=0x{:03X}  SP={}  delay={}  sound={}\n",
        state.pc,
        state.i,
        state.stack.len(),
        state.delay,
        state.sound
    ));
    out.push_str("  V= ");
    for value in state.v.iter() {
        out.push_str(&format!("{:02X} ", value));
    }
    out.push('\n');
    out.push_str(&format!("  Stack={:?}\n", state.stack));
    out.push_str(&format!("  Keys={:?}\n", state.keypad_input));

    out
}