//! One exploratory run: mediate between the headless emulator process and an
//! autonomous agent process, then leave a summary, an ordered trace, and one
//! display image per action behind.
//!
//! The run stops at the first unexpected event — an error response, a malformed
//! or mismatched reply, either child exiting, or a reply that never arrives —
//! and leaves the evidence collected so far in place.

use chip8::display_image;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

/// Where a program is loaded, matching the emulator's reset PC.
const PROGRAM_ORIGIN: u16 = 0x200;
/// The agent learns the run seed through this variable.
const SEED_VARIABLE: &str = "CHIP8_RUN_SEED";
const DEFAULT_TIMEOUT_MS: u64 = 5000;
const USAGE: &str = "usage: chip8-explore --rom ROM --seed SEED --actions N --output DIR \
[--agent PROGRAM] [--emulator PROGRAM] [--timeout-ms MS]";

struct Options {
    rom: PathBuf,
    seed: u32,
    actions: usize,
    output: PathBuf,
    agent: Option<PathBuf>,
    emulator: Option<PathBuf>,
    timeout: Duration,
}

/// The kinds of unexpected event that end a run.
#[derive(Clone)]
struct Failure {
    kind: &'static str,
    index: Option<usize>,
    detail: String,
}

impl Failure {
    fn at(kind: &'static str, index: Option<usize>, detail: impl Into<String>) -> Self {
        Self {
            kind,
            index,
            detail: detail.into(),
        }
    }

    fn json(&self) -> Value {
        json!({"kind": self.kind, "index": self.index, "detail": self.detail})
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.index {
            Some(index) => write!(f, "action {index}: {}: {}", self.kind, self.detail),
            None => write!(f, "{}: {}", self.kind, self.detail),
        }
    }
}

/// What a run leaves behind, whether it finished or stopped early.
struct Report {
    output: PathBuf,
    seed: u32,
    rom_sha256: String,
    program_len: usize,
    requested: usize,
    completed: usize,
    last: Value,
    failure: Option<Failure>,
}

impl Report {
    fn json(&self) -> Value {
        let mut summary = json!({
            "version": 1,
            "status": if self.failure.is_some() { "failed" } else { "success" },
            "seed": self.seed,
            "rom_sha256": self.rom_sha256,
            "load": {"at": PROGRAM_ORIGIN, "bytes": self.program_len},
            "requested_actions": self.requested,
            "actions": self.completed,
            "last_snapshot": self.last,
        });
        if let Some(failure) = &self.failure {
            summary["failure"] = failure.json();
        }
        summary
    }

    fn write(&self) -> Result<(), String> {
        let summary = self.json();
        std::fs::write(
            self.output.join("summary.json"),
            serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())? + "\n",
        )
        .map_err(|e| format!("failed to write summary: {e}"))
    }
}

fn parse(args: Vec<String>) -> Result<Options, String> {
    let mut rom = None;
    let mut seed = None;
    let mut actions = None;
    let mut output = None;
    let mut agent = None;
    let mut emulator = None;
    let mut timeout = DEFAULT_TIMEOUT_MS;
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value\n{USAGE}"));
        match flag.as_str() {
            "--rom" => rom = Some(PathBuf::from(value()?)),
            "--seed" => {
                seed = Some(
                    value()?
                        .parse::<u32>()
                        .map_err(|e| format!("invalid seed: {e}"))?,
                )
            }
            "--actions" => {
                actions = Some(
                    value()?
                        .parse::<usize>()
                        .map_err(|e| format!("invalid action budget: {e}"))?,
                )
            }
            "--output" => output = Some(PathBuf::from(value()?)),
            "--agent" => agent = Some(PathBuf::from(value()?)),
            "--emulator" => emulator = Some(PathBuf::from(value()?)),
            "--timeout-ms" => {
                timeout = value()?
                    .parse::<u64>()
                    .map_err(|e| format!("invalid timeout: {e}"))?
            }
            _ => return Err(format!("unknown argument {flag}\n{USAGE}")),
        }
    }
    if timeout == 0 {
        return Err("--timeout-ms must be positive".into());
    }
    Ok(Options {
        rom: rom.ok_or(format!("--rom is required\n{USAGE}"))?,
        seed: seed.ok_or(format!("--seed is required\n{USAGE}"))?,
        actions: actions.ok_or(format!("--actions is required\n{USAGE}"))?,
        output: output.ok_or(format!("--output is required\n{USAGE}"))?,
        agent,
        emulator,
        timeout: Duration::from_millis(timeout),
    })
}

/// A child process speaking one JSON document per line. Reading happens on its
/// own thread so a silent child becomes a timeout instead of a hung run.
struct LineProcess {
    label: &'static str,
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Option<String>>,
}

impl LineProcess {
    fn spawn(label: &'static str, command: &mut Command) -> Result<Self, String> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to start {label}: {e}"))?;
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if sender.send(Some(line.trim_end().to_string())).is_err() {
                            return;
                        }
                    }
                }
            }
            let _ = sender.send(None);
        });
        Ok(Self {
            label,
            stdin: child.stdin.take(),
            child,
            lines,
        })
    }

    fn send(&mut self, value: &Value, index: Option<usize>) -> Result<(), Failure> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or(Failure::at("child_exit", index, "input already closed"))?;
        writeln!(stdin, "{value}")
            .and_then(|_| stdin.flush())
            .map_err(|e| {
                Failure::at(
                    "child_exit",
                    index,
                    format!("{} closed its input: {e}", self.label),
                )
            })
    }

    fn receive(&mut self, timeout: Duration, index: Option<usize>) -> Result<String, Failure> {
        match self.lines.recv_timeout(timeout) {
            Ok(Some(line)) => Ok(line),
            Ok(None) | Err(RecvTimeoutError::Disconnected) => Err(Failure::at(
                "child_exit",
                index,
                format!("{} exited before responding", self.label),
            )),
            Err(RecvTimeoutError::Timeout) => Err(Failure::at(
                "timeout",
                index,
                format!(
                    "{} did not respond within {}ms",
                    self.label,
                    timeout.as_millis()
                ),
            )),
        }
    }

    /// Send a request and return its parsed response.
    fn request(
        &mut self,
        value: &Value,
        timeout: Duration,
        index: Option<usize>,
    ) -> Result<Value, Failure> {
        self.send(value, index)?;
        let line = self.receive(timeout, index)?;
        serde_json::from_str(&line).map_err(|e| {
            Failure::at(
                "malformed_response",
                index,
                format!("{} sent invalid JSON ({e}): {line}", self.label),
            )
        })
    }

    /// Close the input and require a clean exit.
    fn shutdown(mut self) -> Result<(), Failure> {
        self.stdin = None;
        match self.child.wait() {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(Failure::at(
                "child_exit",
                None,
                format!("{} exited with {status}", self.label),
            )),
            Err(e) => Err(Failure::at(
                "child_exit",
                None,
                format!("failed to wait for {}: {e}", self.label),
            )),
        }
    }
}

impl Drop for LineProcess {
    fn drop(&mut self) {
        // A child that already exited ignores this; one that is still running
        // must not outlive the run.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn sibling(name: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = exe
        .parent()
        .ok_or("executable has no parent directory")?
        .join(name);
    if !path.exists() {
        return Err(format!("{} is missing; build it first", path.display()));
    }
    Ok(path)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn run(options: &Options) -> Result<Report, (Failure, Report)> {
    let mut report = Report {
        output: options.output.clone(),
        seed: options.seed,
        rom_sha256: String::new(),
        program_len: 0,
        requested: options.actions,
        completed: 0,
        last: Value::Null,
        failure: None,
    };

    let program = match std::fs::read(&options.rom) {
        Ok(program) => program,
        Err(e) => {
            let failure = Failure::at(
                "emulator_error",
                None,
                format!("failed to read {}: {e}", options.rom.display()),
            );
            return Err((failure, report));
        }
    };
    report.rom_sha256 = hex(&Sha256::digest(&program));
    report.program_len = program.len();
    if usize::from(PROGRAM_ORIGIN) + program.len() > 4096 {
        let failure = Failure::at(
            "emulator_error",
            None,
            format!(
                "{} does not fit in memory above {PROGRAM_ORIGIN:#x}",
                options.rom.display()
            ),
        );
        return Err((failure, report));
    }

    let prepare = || -> Result<(File, PathBuf, LineProcess, LineProcess, Value), String> {
        std::fs::create_dir_all(&options.output)
            .map_err(|e| format!("failed to create {}: {e}", options.output.display()))?;
        let images = options.output.join("display");
        std::fs::create_dir_all(&images)
            .map_err(|e| format!("failed to create {}: {e}", images.display()))?;
        let trace = File::create(options.output.join("trace.jsonl"))
            .map_err(|e| format!("failed to create trace: {e}"))?;

        let emulator_program = match &options.emulator {
            Some(path) => path.clone(),
            None => sibling("chip8-agent")?,
        };
        let mut emulator = LineProcess::spawn("chip8-agent", &mut Command::new(emulator_program))?;
        let loaded = emulator
            .request(
                &json!({"action": {"type": "load", "at": PROGRAM_ORIGIN, "bytes": program}}),
                options.timeout,
                None,
            )
            .map_err(|failure| format!("{failure}"))?;
        if let Some(error) = loaded.get("error") {
            return Err(format!("emulator rejected the ROM: {error}"));
        }

        let agent_program = match &options.agent {
            Some(path) => path.clone(),
            None => sibling("chip8-random-agent")?,
        };
        let agent = LineProcess::spawn(
            "agent",
            Command::new(agent_program).env(SEED_VARIABLE, options.seed.to_string()),
        )?;
        Ok((trace, images, emulator, agent, loaded["snapshot"].clone()))
    };

    let (mut trace, images, mut emulator, mut agent, mut last) = match prepare() {
        Ok(prepared) => prepared,
        Err(detail) => {
            return Err((
                Failure::at("emulator_error", None, format!("setup failed: {detail}")),
                report,
            ));
        }
    };
    report.last = last.clone();

    for index in 0..options.actions {
        let outcome = (|| -> Result<Value, Failure> {
            agent.send(&json!({"index": index, "snapshot": last}), Some(index))?;
            let reply = agent.receive(options.timeout, Some(index))?;
            let reply: Value = serde_json::from_str(&reply).map_err(|e| {
                Failure::at(
                    "malformed_response",
                    Some(index),
                    format!("agent sent invalid JSON ({e}): {reply}"),
                )
            })?;
            let action = reply.get("action").cloned().ok_or(Failure::at(
                "malformed_response",
                Some(index),
                "agent reply has no action",
            ))?;

            let response = emulator.request(&json!({"action": action}), options.timeout, Some(index))?;
            if let Some(error) = response.get("error") {
                return Err(Failure::at(
                    "emulator_error",
                    Some(index),
                    format!("emulator reported {error}"),
                ));
            }
            if response.get("action") != Some(&action) {
                let actual = response.get("action").cloned().unwrap_or(Value::Null);
                return Err(Failure::at(
                    "mismatched_response",
                    Some(index),
                    format!("asked for {action} but emulator answered {actual}"),
                ));
            }
            let display = display_image::display_from_snapshot(&response["snapshot"])
                .map_err(|e| Failure::at("malformed_response", Some(index), e))?;
            display_image::write_png(&images.join(format!("{index:04}.png")), &display)
                .map_err(|e| Failure::at("malformed_response", Some(index), e))?;

            let entry = json!({
                "index": index,
                "action": action,
                "response": response,
            });
            writeln!(trace, "{entry}")
                .and_then(|_| trace.flush())
                .map_err(|e| {
                    Failure::at("malformed_response", Some(index), format!("failed to write trace: {e}"))
                })?;
            Ok(response["snapshot"].clone())
        })();

        match outcome {
            Ok(snapshot) => {
                last = snapshot.clone();
                report.last = snapshot;
                report.completed += 1;
            }
            Err(failure) => {
                report.failure = Some(failure.clone());
                return Err((failure, report));
            }
        }
    }

    if let Err(failure) = agent.shutdown().and_then(|_| emulator.shutdown()) {
        report.failure = Some(failure.clone());
        return Err((failure, report));
    }
    Ok(report)
}

fn main() {
    let options = match parse(std::env::args().skip(1).collect()) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    match run(&options) {
        Ok(report) => {
            if let Err(error) = report.write() {
                eprintln!("{error}");
                std::process::exit(1);
            }
            println!(
                "status=success seed={} actions={} rom_sha256={}",
                report.seed, report.completed, report.rom_sha256
            );
        }
        Err((failure, report)) => {
            if let Err(error) = report.write() {
                eprintln!("{error}");
            }
            eprintln!("{failure}");
            std::process::exit(1);
        }
    }
}
