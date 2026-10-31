//! One exploratory run: mediate between the headless emulator process and an
//! autonomous agent process, then leave a summary, an ordered trace, and one
//! display image per action behind.
//!
//! The run stops at the first unexpected event — an error response, a malformed
//! or mismatched reply, either child exiting, or a reply that never arrives —
//! and leaves the evidence collected so far in place.

use chip8::display_image;
use chip8::run_viewer::RunViewer;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{
    Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel,
};
use std::time::Duration;

/// Where a program is loaded, matching the emulator's reset PC.
const PROGRAM_ORIGIN: u16 = 0x200;
/// The agent learns the run seed through this variable.
const SEED_VARIABLE: &str = "CHIP8_RUN_SEED";
const DEFAULT_TIMEOUT_MS: u64 = 5000;
const USAGE: &str = "usage: chip8-explore --rom ROM --seed SEED --actions N --output DIR \
[--agent PROGRAM] [--emulator PROGRAM] [--timeout-ms MS] [--view] [--delay-ms MS]";

struct Options {
    rom: PathBuf,
    seed: u32,
    actions: usize,
    output: PathBuf,
    agent: Option<PathBuf>,
    emulator: Option<PathBuf>,
    timeout: Duration,
    view: bool,
    delay: Duration,
}

/// The closed set of unexpected events that end a run. The names appear in
/// summary files and documentation, so they only change deliberately.
enum Kind {
    EmulatorError,
    MalformedResponse,
    MismatchedResponse,
    ChildExit,
    Timeout,
}

impl Kind {
    fn name(&self) -> &'static str {
        match self {
            Self::EmulatorError => "emulator_error",
            Self::MalformedResponse => "malformed_response",
            Self::MismatchedResponse => "mismatched_response",
            Self::ChildExit => "child_exit",
            Self::Timeout => "timeout",
        }
    }
}

/// The unexpected event that ended a run.
struct Failure {
    kind: Kind,
    index: Option<usize>,
    detail: String,
}

impl Failure {
    fn at(kind: Kind, index: Option<usize>, detail: impl Into<String>) -> Self {
        Self {
            kind,
            index,
            detail: detail.into(),
        }
    }

    fn json(&self) -> Value {
        json!({"kind": self.kind.name(), "index": self.index, "detail": self.detail})
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.index {
            Some(index) => write!(f, "action {index}: {}: {}", self.kind.name(), self.detail),
            None => write!(f, "{}: {}", self.kind.name(), self.detail),
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
    view: bool,
    delay_ms: u64,
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
            "viewer": self.view,
            "delay_ms": self.delay_ms,
        });
        // The summary stays concise; a failure adds the evidence needed to
        // reproduce it, including the last trustworthy snapshot.
        if let Some(failure) = &self.failure {
            summary["failure"] = failure.json();
            summary["last_snapshot"] = self.last.clone();
        }
        summary
    }

    /// Record the event that ended the run.
    fn fail(&mut self, kind: Kind, detail: impl Into<String>) {
        self.failure = Some(Failure::at(kind, None, detail));
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
    let mut view = false;
    let mut delay = 0;
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
            "--delay-ms" => {
                delay = value()?
                    .parse::<u64>()
                    .map_err(|e| format!("invalid pacing delay: {e}"))?
            }
            "--view" => view = true,
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
        view,
        delay: Duration::from_millis(delay),
    })
}

/// A child process speaking one JSON document per line.
///
/// Both directions run on their own threads behind bounded channels, so a
/// child that stops reading or stops answering becomes a timeout instead of a
/// hung run.
struct LineProcess {
    label: &'static str,
    child: Child,
    writes: Option<SyncSender<String>>,
    lines: Receiver<Option<String>>,
}

impl LineProcess {
    fn spawn(label: &'static str, command: &mut Command) -> Result<Self, String> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to start {label}: {e}"))?;

        let mut stdin = child.stdin.take().unwrap();
        let (writes, pending) = sync_channel::<String>(1);
        std::thread::spawn(move || {
            while let Ok(line) = pending.recv() {
                if writeln!(stdin, "{line}")
                    .and_then(|_| stdin.flush())
                    .is_err()
                {
                    return;
                }
            }
        });

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
            child,
            writes: Some(writes),
            lines,
        })
    }

    fn send(
        &mut self,
        value: &Value,
        timeout: Duration,
        index: Option<usize>,
    ) -> Result<(), Failure> {
        let writes = self.writes.as_ref().ok_or(Failure::at(
            Kind::ChildExit,
            index,
            "input already closed",
        ))?;
        // The writer thread keeps a bounded queue, so a child that stops
        // draining its input shows up as a full queue rather than a blocked
        // write.
        let deadline = std::time::Instant::now() + timeout;
        let mut pending = value.to_string();
        loop {
            match writes.try_send(pending) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(line)) if std::time::Instant::now() < deadline => {
                    pending = line;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TrySendError::Full(_)) => {
                    return Err(Failure::at(
                        Kind::Timeout,
                        index,
                        format!(
                            "{} did not accept input within {}ms",
                            self.label,
                            timeout.as_millis()
                        ),
                    ));
                }
                Err(TrySendError::Disconnected(_)) => {
                    return Err(Failure::at(
                        Kind::ChildExit,
                        index,
                        format!("{} closed its input", self.label),
                    ));
                }
            }
        }
    }

    fn receive(&mut self, timeout: Duration, index: Option<usize>) -> Result<String, Failure> {
        match self.lines.recv_timeout(timeout) {
            Ok(Some(line)) => Ok(line),
            Ok(None) | Err(RecvTimeoutError::Disconnected) => Err(Failure::at(
                Kind::ChildExit,
                index,
                format!("{} exited before responding", self.label),
            )),
            Err(RecvTimeoutError::Timeout) => Err(Failure::at(
                Kind::Timeout,
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
        self.send(value, timeout, index)?;
        let line = self.receive(timeout, index)?;
        serde_json::from_str(&line).map_err(|e| {
            Failure::at(
                Kind::MalformedResponse,
                index,
                format!("{} sent invalid JSON ({e}): {line}", self.label),
            )
        })
    }

    /// Close the input and require a clean exit within `timeout`.
    fn shutdown(&mut self, timeout: Duration) -> Result<(), Failure> {
        self.writes = None;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => {
                    return Err(Failure::at(
                        Kind::ChildExit,
                        None,
                        format!("{} exited with {status}", self.label),
                    ));
                }
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(None) => {
                    return Err(Failure::at(
                        Kind::Timeout,
                        None,
                        format!(
                            "{} did not exit within {}ms",
                            self.label,
                            timeout.as_millis()
                        ),
                    ));
                }
                Err(e) => {
                    return Err(Failure::at(
                        Kind::ChildExit,
                        None,
                        format!("failed to wait for {}: {e}", self.label),
                    ));
                }
            }
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

/// A short, stable name for the action, for the viewer's status area.
fn action_name(action: &Value) -> String {
    let kind = action["type"].as_str().unwrap_or("?");
    match action.get("key").and_then(Value::as_u64) {
        Some(key) => format!("{kind} {key:x}"),
        None => kind.to_string(),
    }
}

/// Everything one action needs: the two children, the artifacts being written,
/// and the last trustworthy snapshot.
struct Session {
    trace: File,
    images: PathBuf,
    emulator: LineProcess,
    agent: LineProcess,
    last: Value,
}

impl Session {
    /// Ask the agent for one action, apply it, and record what happened.
    /// Returns the action's short name and its resulting display.
    fn turn(
        &mut self,
        index: usize,
        options: &Options,
    ) -> Result<(String, Vec<Vec<bool>>), Failure> {
        self.agent
            .send(&json!({"index": index, "snapshot": self.last}), options.timeout, Some(index))?;
        let reply = self.agent.receive(options.timeout, Some(index))?;
        let reply: Value = serde_json::from_str(&reply).map_err(|e| {
            Failure::at(
                Kind::MalformedResponse,
                Some(index),
                format!("agent sent invalid JSON ({e}): {reply}"),
            )
        })?;
        let action = reply.get("action").cloned().ok_or(Failure::at(
            Kind::MalformedResponse,
            Some(index),
            "agent reply has no action",
        ))?;

        let response =
            self.emulator
                .request(&json!({"action": action}), options.timeout, Some(index))?;
        if let Some(error) = response.get("error") {
            return Err(Failure::at(
                Kind::EmulatorError,
                Some(index),
                format!("emulator reported {error}"),
            ));
        }
        if response.get("action") != Some(&action) {
            let actual = response.get("action").cloned().unwrap_or(Value::Null);
            return Err(Failure::at(
                Kind::MismatchedResponse,
                Some(index),
                format!("asked for {action} but emulator answered {actual}"),
            ));
        }

        let display = display_image::display_from_snapshot(&response["snapshot"])
            .map_err(|e| Failure::at(Kind::MalformedResponse, Some(index), e))?;
        display_image::write_png(&self.images.join(format!("{index:04}.png")), &display)
            .map_err(|e| Failure::at(Kind::MalformedResponse, Some(index), e))?;

        let entry = json!({
            "index": index,
            "action": action,
            "response": response,
        });
        writeln!(self.trace, "{entry}")
            .and_then(|_| self.trace.flush())
            .map_err(|e| {
                Failure::at(
                    Kind::MalformedResponse,
                    Some(index),
                    format!("failed to write trace: {e}"),
                )
            })?;

        self.last = response["snapshot"].clone();
        Ok((action_name(&action), display))
    }
}

/// Create the output layout and start both children.
fn prepare(options: &Options, program: &[u8]) -> Result<Session, String> {
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
        .map_err(|failure| failure.to_string())?;
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

    Ok(Session {
        trace,
        images,
        emulator,
        agent,
        last: loaded["snapshot"].clone(),
    })
}

fn run(options: &Options) -> Result<Report, Report> {
    let mut report = Report {
        output: options.output.clone(),
        seed: options.seed,
        rom_sha256: String::new(),
        program_len: 0,
        requested: options.actions,
        completed: 0,
        last: Value::Null,
        failure: None,
        view: options.view,
        delay_ms: options.delay.as_millis() as u64,
    };

    let program = match std::fs::read(&options.rom) {
        Ok(program) => program,
        Err(e) => {
            report.fail(
                Kind::EmulatorError,
                format!("failed to read {}: {e}", options.rom.display()),
            );
            return Err(report);
        }
    };
    report.rom_sha256 = hex(&Sha256::digest(&program));
    report.program_len = program.len();
    if usize::from(PROGRAM_ORIGIN) + program.len() > 4096 {
        report.fail(
            Kind::EmulatorError,
            format!(
                "{} does not fit in memory above {PROGRAM_ORIGIN:#x}",
                options.rom.display()
            ),
        );
        return Err(report);
    }

    let mut session = match prepare(options, &program) {
        Ok(session) => session,
        Err(detail) => {
            report.fail(Kind::EmulatorError, format!("setup failed: {detail}"));
            return Err(report);
        }
    };
    report.last = session.last.clone();

    let mut viewer = if options.view {
        let title = format!("CHIP-8 run - {}", options.rom.display());
        match RunViewer::new(10, &title) {
            Ok(viewer) => Some(viewer),
            Err(e) => {
                report.fail(
                    Kind::EmulatorError,
                    format!("failed to open the viewer: {e}"),
                );
                return Err(report);
            }
        }
    } else {
        None
    };

    for index in 0..options.actions {
        match session.turn(index, options) {
            Ok((latest, display)) => {
                report.last = session.last.clone();
                report.completed += 1;
                // Closing the window only stops watching; the run continues.
                if let Some(open) = viewer.as_mut().filter(|viewer| viewer.is_open()) {
                    if open
                        .show(&display, report.completed, options.actions, &latest)
                        .is_err()
                    {
                        viewer = None;
                    }
                }
                if !options.delay.is_zero() {
                    std::thread::sleep(options.delay);
                }
            }
            Err(failure) => {
                report.failure = Some(failure);
                return Err(report);
            }
        }
    }

    if let Err(failure) = session
        .agent
        .shutdown(options.timeout)
        .and_then(|_| session.emulator.shutdown(options.timeout))
    {
        report.failure = Some(failure);
        return Err(report);
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
        Err(report) => {
            if let Err(error) = report.write() {
                eprintln!("{error}");
            }
            if let Some(failure) = &report.failure {
                eprintln!("{failure}");
            }
            std::process::exit(1);
        }
    }
}
