use chip8::chip8::State;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Load { bytes: Vec<u8>, at: u16 },
    Step,
    Inspect,
    Reset,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    action: Action,
}

fn execute(state: &mut State, action: &Action) -> Result<(), String> {
    match action {
        Action::Load { bytes, at } => {
            if usize::from(*at) + bytes.len() > state.memory.len() {
                return Err("program write out of bounds".into());
            }
            state.load_program(bytes, *at);
        }
        // The core currently panics on some malformed-ROM memory accesses.
        // Contain those faults at the process boundary and stop the trace;
        // never continue execution with the partially mutated state.
        Action::Step => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.step()))
            .map_err(|_| "emulator fault during instruction execution".to_string())?
            .map_err(|e| format!("{e:?}"))?,
        Action::Inspect => {}
        Action::Reset => *state = State::new(),
    }
    Ok(())
}

fn emit(value: &Value) -> Result<(), String> {
    let mut out = io::stdout().lock();
    writeln!(out, "{value}")
        .and_then(|_| out.flush())
        .map_err(|e| e.to_string())
}

fn live() -> Result<(), String> {
    let mut state = State::new();
    let mut last = Value::Null;
    for (index, line) in io::stdin().lock().lines().enumerate() {
        let line = line.map_err(|e| e.to_string())?;
        let result = serde_json::from_str::<Request>(&line)
            .map_err(|e| e.to_string())
            .and_then(|request| {
                execute(&mut state, &request.action)?;
                Ok(request)
            });
        match result {
            Ok(request) => {
                last = state.snapshot();
                emit(&json!({"index":index, "action":request.action, "snapshot":last}))?;
            }
            Err(error) => {
                emit(&json!({"index":index,"request":line,"error":error,"last_snapshot":last}))?;
                return Err(error);
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = live() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
