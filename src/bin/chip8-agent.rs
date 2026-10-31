use chip8::chip8::{KeyInput, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Load { bytes: Vec<u8>, at: u16 },
    Step {},
    Inspect {},
    Reset {},
    Seed { seed: u32 },
    Tick {},
    Key { key: u8, pressed: bool },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    action: Action,
    #[serde(default)]
    checkpoint: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot: Option<Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    version: u32,
    actions: Vec<Request>,
}

fn read_fixture(path: &str) -> Result<Fixture, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let fixture: Fixture = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if fixture.version != 1 {
        return Err("unsupported fixture version".into());
    }
    Ok(fixture)
}

fn differences(expected: &Value, actual: &Value, path: &str, out: &mut Vec<String>) {
    if expected == actual {
        return;
    }
    match (expected, actual) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for key in keys {
                match (a.get(key), b.get(key)) {
                    (Some(a), Some(b)) => differences(a, b, &format!("{path}/{key}"), out),
                    (Some(a), None) => {
                        out.push(format!("{path}/{key}: expected {a}, actual <missing>"))
                    }
                    (None, Some(b)) => {
                        out.push(format!("{path}/{key}: expected <missing>, actual {b}"))
                    }
                    (None, None) => unreachable!(),
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                differences(a, b, &format!("{path}/{i}"), out);
            }
        }
        _ => out.push(format!("{path}: expected {expected}, actual {actual}")),
    }
}

struct Session {
    state: State,
    last: Value,
}

impl Session {
    fn new() -> Self {
        Self {
            state: State::new(),
            last: Value::Null,
        }
    }

    fn apply(&mut self, index: usize, request: &mut Request, record: bool) -> Result<(), String> {
        // Recorded observations are untrusted input, never the current result.
        request.snapshot = None;
        let result = (|| {
            if !record && request.checkpoint && request.expected.is_none() {
                return Err("checkpoint requires expected snapshot".into());
            }
            if !request.checkpoint && request.expected.is_some() {
                return Err("expected snapshot requires explicit checkpoint".into());
            }
            execute(&mut self.state, &request.action)?;
            let snapshot = self.state.snapshot();
            request.snapshot = Some(snapshot.clone());
            if request.checkpoint {
                if record {
                    request.expected = Some(snapshot.clone());
                } else {
                    let mut diff = Vec::new();
                    differences(
                        request.expected.as_ref().unwrap(),
                        &snapshot,
                        "snapshot",
                        &mut diff,
                    );
                    if !diff.is_empty() {
                        return Err(format!("checkpoint mismatch: {}", diff.join("; ")));
                    }
                }
            }
            Ok(snapshot)
        })();
        match result {
            Ok(snapshot) => {
                emit(&json!({"index":index,"action":request.action,"snapshot":snapshot}))?;
                self.last = snapshot;
                Ok(())
            }
            Err(error) => {
                emit(&json!({"index":index,"action":request.action,"error":error,
                    "snapshot":request.snapshot,"last_snapshot":self.last}))?;
                Err(error)
            }
        }
    }
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
        Action::Step {} => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.step()))
            .map_err(|_| "emulator fault during instruction execution".to_string())?
            .map_err(|e| format!("{e:?}"))?,
        Action::Inspect {} => {}
        Action::Reset {} => *state = State::new(),
        Action::Seed { seed } => state.seed_random(*seed),
        Action::Tick {} => state.tick_timers(),
        Action::Key { key, pressed } => {
            let keys = [
                KeyInput::Zero,
                KeyInput::One,
                KeyInput::Two,
                KeyInput::Three,
                KeyInput::Four,
                KeyInput::Five,
                KeyInput::Six,
                KeyInput::Seven,
                KeyInput::Eight,
                KeyInput::Nine,
                KeyInput::A,
                KeyInput::B,
                KeyInput::C,
                KeyInput::D,
                KeyInput::E,
                KeyInput::F,
            ];
            let key = keys.get(usize::from(*key)).ok_or("key must be in 0..=15")?;
            state.set_key(*key, *pressed);
        }
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
    let mut session = Session::new();
    for (index, line) in io::stdin().lock().lines().enumerate() {
        let line = line.map_err(|e| e.to_string())?;
        match serde_json::from_str::<Request>(&line) {
            Ok(mut request) => session.apply(index, &mut request, false)?,
            Err(error) => {
                emit(
                    &json!({"index":index,"request":line,"error":error.to_string(),"last_snapshot":session.last}),
                )?;
                return Err(error.to_string());
            }
        }
    }
    Ok(())
}

fn record(source: &str, candidate: &str) -> Result<(), String> {
    let mut fixture = read_fixture(source)?;
    // create_new rejects existing files, symlinks and hard-link aliases alike.
    // Reserve the output before execution, but remove it if execution fails.
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(candidate)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut session = Session::new();
        for (index, request) in fixture.actions.iter_mut().enumerate() {
            session.apply(index, request, true)?;
        }
        let text = serde_json::to_string_pretty(&fixture).map_err(|e| e.to_string())? + "\n";
        output.write_all(text.as_bytes()).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(candidate);
    }
    result
}

fn review(candidate: &str, approved: &str) -> Result<(), String> {
    let candidate = serde_json::to_value(read_fixture(candidate)?).map_err(|e| e.to_string())?;
    match std::fs::read(approved) {
        Ok(bytes) => {
            let previous: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let mut diff = Vec::new();
            differences(&previous, &candidate, "fixture", &mut diff);
            emit(&json!({"differences":diff}))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            emit(&json!({"initial":true,"candidate":candidate}))
        }
        Err(e) => Err(e.to_string()),
    }
}

fn promote(candidate: &str, approved: &str) -> Result<(), String> {
    let fixture = read_fixture(candidate)?;
    if !fixture.actions.iter().any(|r| r.checkpoint) {
        return Err("promotion requires at least one checkpoint".into());
    }
    for request in &fixture.actions {
        if request.snapshot.is_none()
            || (request.checkpoint && request.expected != request.snapshot)
            || (!request.checkpoint && request.expected.is_some())
        {
            return Err("promotion requires a complete recorded candidate".into());
        }
    }
    let destination = std::path::Path::new(approved);
    if destination.exists()
        && std::fs::canonicalize(candidate).ok() == std::fs::canonicalize(destination).ok()
    {
        return Err("candidate and approved fixture must differ".into());
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(&fixture).map_err(|e| e.to_string())? + "\n";
    temp.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(destination).map_err(|e| e.to_string())?;
    emit(&json!({"promoted":approved}))
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => live(),
        [mode, path] if mode == "replay" => {
            let mut fixture = read_fixture(path)?;
            let mut session = Session::new();
            for (index, request) in fixture.actions.iter_mut().enumerate() {
                session.apply(index, request, false)?;
            }
            Ok(())
        }
        [mode, source, candidate] if mode == "record" => record(source, candidate),
        [mode, candidate, approved] if mode == "review" => review(candidate, approved),
        [mode, candidate, approved, flag] if mode == "promote" && flag == "--replace" => promote(candidate, approved),
        _ => Err("usage: chip8-agent [replay FIXTURE | record SOURCE NEW_CANDIDATE | review CANDIDATE APPROVED | promote CANDIDATE APPROVED --replace]".into()),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
