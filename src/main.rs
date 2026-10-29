use std::path::PathBuf;

use chip8::chip8::State;
use chip8::renderer::Renderer;

/// Where a program is loaded and execution begins.
const PROGRAM_ORIGIN: u16 = 0x200;
/// Target ~700 instructions/second at 60 FPS, per the display spec.
const INSTRUCTIONS_PER_FRAME: usize = 11;

fn main() -> Result<(), String> {
    let rom_path = std::env::args().nth(1).map(PathBuf::from);

    let mut state = State::new();
    if let Some(path) = &rom_path {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("failed to read {}: {}", path.display(), error))?;
        state.load_program(&bytes, PROGRAM_ORIGIN);
    }

    let mut renderer = Renderer::new(10)?;

    while renderer.is_open() {
        // Only execute if a ROM was loaded; otherwise just show the blank
        // initial display.
        if rom_path.is_some() {
            let mut error_free = true;
            for _ in 0..INSTRUCTIONS_PER_FRAME {
                if state.step().is_err() {
                    error_free = false;
                    break;
                }
            }
            if error_free {
                state.tick_timers();
            }
        }

        renderer.render(&state);
    }

    Ok(())
}