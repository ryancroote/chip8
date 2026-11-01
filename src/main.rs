use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chip8::agent;
use chip8::chip8::State;
use chip8::renderer::Renderer;
use chip8::rpc::{RpcServer, SharedState};

/// Where a program is loaded and execution begins.
const PROGRAM_ORIGIN: u16 = 0x200;
/// Target ~700 instructions/second at 60 FPS, per the display spec.
const INSTRUCTIONS_PER_FRAME: usize = 11;
const DEFAULT_RPC_PORT: u16 = 8787;
const USAGE: &str = "usage: chip8 [ROM] [--rpc [--headless] [--port PORT]] | chip8 agent [ARGS...]";

struct Options {
    rom_path: Option<PathBuf>,
    rpc: bool,
    headless: bool,
    port: u16,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut rom_path = None;
    let mut rpc = false;
    let mut headless = false;
    let mut port = DEFAULT_RPC_PORT;
    let mut port_set = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" => return Err(USAGE.into()),
            "--rpc" => rpc = true,
            "--headless" => headless = true,
            "--port" => {
                index += 1;
                let value = args.get(index).ok_or("--port requires a port number")?;
                port = value
                    .parse()
                    .map_err(|_| "--port must be a number in 0..=65535")?;
                port_set = true;
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option: {value}"));
            }
            path => {
                if rom_path.replace(PathBuf::from(path)).is_some() {
                    return Err("only one ROM path may be provided".into());
                }
            }
        }
        index += 1;
    }

    if headless && !rpc {
        return Err("--headless requires --rpc".into());
    }
    if port_set && !rpc {
        return Err("--port requires --rpc".into());
    }
    if rpc && rom_path.is_none() {
        return Err("RPC mode requires a ROM path".into());
    }

    Ok(Options {
        rom_path,
        rpc,
        headless,
        port,
    })
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

fn run_graphical(state: SharedState, rpc: bool, rom_loaded: bool) -> Result<(), String> {
    let mut renderer = Renderer::new(10)?;
    while renderer.is_open() {
<<<<<<< Updated upstream
        // Only execute if a ROM was loaded; otherwise just show the blank
        // initial display.
        if rom_path.is_some() {
=======
        if !rpc && rom_loaded {
            let mut state = state
                .lock()
                .map_err(|_| "machine state lock poisoned".to_string())?;
            renderer.update_keypad_input(&mut state, true);
>>>>>>> Stashed changes
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
        let state = state
            .lock()
            .map_err(|_| "machine state lock poisoned".to_string())?;
        renderer.render(&state);
    }
    Ok(())
}

fn start_rpc(state: SharedState, port: u16) -> Result<RpcServer, String> {
    let address: SocketAddr = format!("127.0.0.1:{port}")
        .parse()
        .expect("loopback address with a validated port parses");
    let server = RpcServer::start(state, address)?;
    eprintln!("RPC listening at ws://{}", server.local_addr());
    Ok(server)
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

    let options = parse_options(&args)?;
    let state = Arc::new(Mutex::new(load_state(options.rom_path.as_ref())?));
    if options.rpc {
        let _server = start_rpc(Arc::clone(&state), options.port)?;
        if options.headless {
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }
        return run_graphical(state, true, true);
    }
    run_graphical(state, false, options.rom_path.is_some())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
