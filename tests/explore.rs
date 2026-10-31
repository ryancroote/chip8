//! Exploratory runs driven through the public command line: a real
//! `chip8-agent` process on one side, a real agent process on the other.

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Clear the display, then jump back to the first instruction.
const LOOP_ROM: [u8; 4] = [0x00, 0xE0, 0x12, 0x00];
/// SHA-256 of `LOOP_ROM`, computed independently with `shasum -a 256`.
const LOOP_ROM_SHA256: &str = "e357f2b9006e099c8c00ea4527943fd793dd4bdc06b1af59aac678ce2d56db43";

fn rom(dir: &Path) -> PathBuf {
    let path = dir.join("loop.ch8");
    std::fs::write(&path, LOOP_ROM).unwrap();
    path
}

fn explore(dir: &Path, name: &str, rom: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_chip8-explore"))
        .args([
            "--rom",
            rom.to_str().unwrap(),
            "--seed",
            "42",
            "--actions",
            "8",
            "--output",
            dir.join(name).to_str().unwrap(),
        ])
        .args(extra)
        .output()
        .unwrap()
}

fn trace(dir: &Path, name: &str) -> Vec<Value> {
    std::fs::read_to_string(dir.join(name).join("trace.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn summary(dir: &Path, name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(dir.join(name).join("summary.json")).unwrap()).unwrap()
}

#[test]
fn seeded_run_reaches_its_action_budget_and_repeats_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let rom = rom(dir.path());

    let first = explore(dir.path(), "first", &rom, &[]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = explore(dir.path(), "second", &rom, &[]);
    assert!(second.status.success());

    let first_summary = summary(dir.path(), "first");
    assert_eq!(first_summary["status"], "success");
    assert_eq!(first_summary["seed"], 42);
    assert_eq!(first_summary["actions"], 8);
    assert_eq!(first_summary["rom_sha256"], LOOP_ROM_SHA256);

    assert_eq!(first_summary, summary(dir.path(), "second"));
    assert_eq!(trace(dir.path(), "first"), trace(dir.path(), "second"));

    let trace = trace(dir.path(), "first");
    assert_eq!(trace.len(), 8);
    for (index, entry) in trace.iter().enumerate() {
        assert_eq!(entry["index"], index);
        let action = entry["action"]["type"].as_str().unwrap();
        assert!(
            matches!(action, "step" | "inspect" | "key" | "tick"),
            "unexpected action {action}"
        );
        assert!(entry["response"]["snapshot"]["display"].is_array());
    }
}

#[test]
fn the_agent_is_a_separate_process_that_sees_every_full_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let rom = rom(dir.path());

    let log = dir.path().join("turns.log");
    let agent = dir.path().join("recording-agent.sh");
    let mut file = std::fs::File::create(&agent).unwrap();
    writeln!(
        file,
        "#!/bin/sh\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> {}\n  printf '%s\\n' '{{\"action\":{{\"type\":\"step\"}}}}'\ndone",
        log.display()
    )
    .unwrap();
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let run = explore(
        dir.path(),
        "custom",
        &rom,
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );

    let turns: Vec<Value> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(turns.len(), 8);
    for (index, turn) in turns.iter().enumerate() {
        assert_eq!(turn["index"], index);
        assert_eq!(turn["snapshot"]["memory"].as_array().unwrap().len(), 4096);
        assert_eq!(turn["snapshot"]["display"].as_array().unwrap().len(), 32);
        assert!(!turn["snapshot"]["pc"].is_null());
    }
    assert_eq!(trace(dir.path(), "custom").len(), 8);
}
