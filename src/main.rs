use std::path::PathBuf;

use chip8::agent;
use chip8::chip8::State;
use chip8::renderer::Renderer;

/// Where a program is loaded and execution begins.
const PROGRAM_ORIGIN: u16 = 0x200;
/// Target ~700 instructions/second at 60 FPS, per the display spec.
const INSTRUCTIONS_PER_FRAME: usize = 11;
const USAGE: &str = "usage: chip8 [ROM] | chip8 agent [ARGS...]";

fn parse_rom_path(args: &[String]) -> Result<Option<PathBuf>, String> {
    let mut rom_path = None;
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Err(USAGE.into()),
            value if value.starts_with('-') => {
                return Err(format!("unknown option: {value}"));
            }
            path => {
                if rom_path.replace(PathBuf::from(path)).is_some() {
                    return Err("only one ROM path may be provided".into());
                }
            }
        }
    }
    Ok(rom_path)
}

fn load_state(rom_path: Option<&PathBuf>) -> Result<State, String> {
    let mut state = State::new();
    if let Some(path) = rom_path {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("failed to read {}: {}", path.display(), error))?;
        state.load_program(&bytes, PROGRAM_ORIGIN);
    }
    Ok(state)
}

fn run_graphical(mut state: State, rom_loaded: bool) -> Result<(), String> {
    let mut renderer = Renderer::new(10)?;
    while renderer.is_open() {
        renderer.update_keypad_input(&mut state);
        // Only execute if a ROM was loaded; otherwise just show the blank
        // initial display.
        if rom_loaded {
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
        renderer.render(&state.display);
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.as_slice(), [help] if help == "--help" || help == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "agent") {
        return agent::run(&args[1..]);
    }

    let rom_path = parse_rom_path(&args)?;
    let state = load_state(rom_path.as_ref())?;
    run_graphical(state, rom_path.is_some())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
