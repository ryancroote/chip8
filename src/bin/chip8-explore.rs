//! One exploratory run: mediate between the headless emulator process and an
//! autonomous agent process, then leave a summary and an ordered trace behind.

use chip8::display_image;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// Where a program is loaded, matching the emulator's reset PC.
const PROGRAM_ORIGIN: u16 = 0x200;
/// The agent learns the run seed through this variable.
const SEED_VARIABLE: &str = "CHIP8_RUN_SEED";

struct Options {
    rom: PathBuf,
    seed: u32,
    actions: usize,
    output: PathBuf,
    agent: Option<PathBuf>,
}

fn parse(args: Vec<String>) -> Result<Options, String> {
    const USAGE: &str =
        "usage: chip8-explore --rom ROM --seed SEED --actions N --output DIR [--agent PROGRAM]";
    let mut rom = None;
    let mut seed = None;
    let mut actions = None;
    let mut output = None;
    let mut agent = None;
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
            _ => return Err(format!("unknown argument {flag}\n{USAGE}")),
        }
    }
    Ok(Options {
        rom: rom.ok_or(format!("--rom is required\n{USAGE}"))?,
        seed: seed.ok_or(format!("--seed is required\n{USAGE}"))?,
        actions: actions.ok_or(format!("--actions is required\n{USAGE}"))?,
        output: output.ok_or(format!("--output is required\n{USAGE}"))?,
        agent,
    })
}

/// A child process speaking one JSON document per line.
struct LineProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    label: &'static str,
}

impl LineProcess {
    fn spawn(label: &'static str, command: &mut Command) -> Result<Self, String> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to start {label}: {e}"))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok(Self {
            child,
            stdin,
            stdout,
            label,
        })
    }

    fn send(&mut self, value: &Value) -> Result<(), String> {
        writeln!(self.stdin, "{value}")
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("{} closed its input: {e}", self.label))
    }

    fn receive(&mut self) -> Result<String, String> {
        let mut line = String::new();
        match self.stdout.read_line(&mut line) {
            Ok(0) => Err(format!("{} exited before responding", self.label)),
            Ok(_) => Ok(line.trim_end().to_string()),
            Err(e) => Err(format!("failed to read from {}: {e}", self.label)),
        }
    }

    /// Send a request and return the JSON response.
    fn request(&mut self, value: &Value) -> Result<Value, String> {
        self.send(value)?;
        let line = self.receive()?;
        serde_json::from_str(&line)
            .map_err(|e| format!("{} sent invalid JSON: {e}: {line}", self.label))
    }

    fn finish(mut self) -> Result<(), String> {
        drop(self.stdin);
        let status = self
            .child
            .wait()
            .map_err(|e| format!("failed to wait for {}: {e}", self.label))?;
        if !status.success() {
            return Err(format!("{} exited with {status}", self.label));
        }
        Ok(())
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

fn run(options: &Options) -> Result<Value, String> {
    let program = std::fs::read(&options.rom)
        .map_err(|e| format!("failed to read {}: {e}", options.rom.display()))?;
    if usize::from(PROGRAM_ORIGIN) + program.len() > 4096 {
        return Err(format!(
            "{} does not fit in memory above {PROGRAM_ORIGIN:#x}",
            options.rom.display()
        ));
    }
    std::fs::create_dir_all(&options.output)
        .map_err(|e| format!("failed to create {}: {e}", options.output.display()))?;
    let images = options.output.join("display");
    std::fs::create_dir_all(&images).map_err(|e| format!("failed to create {}: {e}", images.display()))?;
    let mut trace = File::create(options.output.join("trace.jsonl"))
        .map_err(|e| format!("failed to create trace: {e}"))?;

    let mut emulator = LineProcess::spawn(
        "chip8-agent",
        &mut Command::new(sibling("chip8-agent")?),
    )?;
    let loaded = emulator.request(&json!({
        "action": {"type": "load", "at": PROGRAM_ORIGIN, "bytes": program}
    }))?;
    if let Some(error) = loaded.get("error") {
        return Err(format!("emulator rejected the ROM: {error}"));
    }
    let snapshot = loaded["snapshot"].clone();

    let agent_path = match &options.agent {
        Some(path) => path.clone(),
        None => sibling("chip8-random-agent")?,
    };
    let mut agent = LineProcess::spawn(
        "agent",
        Command::new(&agent_path).env(SEED_VARIABLE, options.seed.to_string()),
    )?;

    let mut completed = 0;
    let mut last = snapshot.clone();
    for index in 0..options.actions {
        agent.send(&json!({"index": index, "snapshot": last}))?;
        let reply = agent.receive()?;
        let reply: Value = serde_json::from_str(&reply)
            .map_err(|e| format!("agent sent invalid JSON at action {index}: {e}: {reply}"))?;
        let action = reply
            .get("action")
            .ok_or(format!("agent reply at action {index} has no action"))?
            .clone();

        let response = emulator.request(&json!({"action": action}))?;
        if let Some(error) = response.get("error") {
            return Err(format!("emulator error at action {index}: {error}"));
        }
        let entry = json!({
            "index": index,
            "action": response["action"],
            "response": response,
        });
        writeln!(trace, "{entry}")
            .and_then(|_| trace.flush())
            .map_err(|e| format!("failed to write trace: {e}"))?;
        let display = display_image::display_from_snapshot(&response["snapshot"])?;
        display_image::write_png(&images.join(format!("{index:04}.png")), &display)?;
        last = response["snapshot"].clone();
        completed += 1;
    }

    agent.finish()?;
    emulator.finish()?;

    let summary = json!({
        "version": 1,
        "status": "success",
        "seed": options.seed,
        "rom_sha256": hex(&Sha256::digest(&program)),
        "load": {"at": PROGRAM_ORIGIN, "bytes": program.len()},
        "requested_actions": options.actions,
        "actions": completed,
    });
    write_summary(&options.output, &summary)?;
    Ok(summary)
}

fn write_summary(output: &Path, summary: &Value) -> Result<(), String> {
    std::fs::write(
        output.join("summary.json"),
        serde_json::to_string_pretty(summary).map_err(|e| e.to_string())? + "\n",
    )
    .map_err(|e| format!("failed to write summary: {e}"))
}

fn main() {
    let result = parse(std::env::args().skip(1).collect()).and_then(|o| run(&o));
    match result {
        Ok(summary) => println!("{summary}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
