//! CHIP-8 emulator library target.
//!
//! Exposing the emulator as a library lets the deterministic test harness in
//! `tests/` live fully outside the emulator module.

pub mod chip8;
pub mod display_image;
pub mod renderer;
pub mod run_viewer;
