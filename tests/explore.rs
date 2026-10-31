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

/// Scale factor between one CHIP-8 pixel and one PNG pixel.
const SCALE: usize = 8;

/// Decode a display image into 64x32 booleans, requiring every CHIP-8 pixel to
/// be a uniform block of one grayscale value.
fn decode_image(path: &Path) -> Vec<Vec<bool>> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!(info.width as usize, 64 * SCALE);
    assert_eq!(info.height as usize, 32 * SCALE);
    assert_eq!(info.color_type, png::ColorType::Grayscale);
    let mut display = vec![vec![false; 64]; 32];
    for (row, columns) in display.iter_mut().enumerate() {
        for (column, pixel) in columns.iter_mut().enumerate() {
            let mut value = None;
            for y in 0..SCALE {
                for x in 0..SCALE {
                    let at = (row * SCALE + y) * (64 * SCALE) + column * SCALE + x;
                    let sample = buffer[at];
                    assert!(sample == 0 || sample == 255, "non-monochrome pixel");
                    match value {
                        None => value = Some(sample),
                        Some(known) => assert_eq!(known, sample, "ragged pixel block"),
                    }
                }
            }
            *pixel = value == Some(255);
        }
    }
    display
}

fn snapshot_display(entry: &Value) -> Vec<Vec<bool>> {
    entry["response"]["snapshot"]["display"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_array()
                .unwrap()
                .iter()
                .map(|pixel| pixel.as_bool().unwrap())
                .collect()
        })
        .collect()
}

fn images(dir: &Path, name: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir.join(name).join("display"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

#[test]
fn every_action_leaves_a_display_image_matching_its_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let rom = rom(dir.path());
    assert!(explore(dir.path(), "first", &rom, &[]).status.success());
    assert!(explore(dir.path(), "second", &rom, &[]).status.success());

    let trace = trace(dir.path(), "first");
    let first_images = images(dir.path(), "first");
    assert_eq!(first_images.len(), trace.len(), "one image per action");

    for (entry, path) in trace.iter().zip(&first_images) {
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            format!("{:04}.png", entry["index"].as_u64().unwrap())
        );
        assert_eq!(decode_image(path), snapshot_display(entry));
    }

    for (first, second) in first_images.iter().zip(images(dir.path(), "second")) {
        assert_eq!(std::fs::read(first).unwrap(), std::fs::read(second).unwrap());
    }
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// Run an agent that misbehaves in one specific way and assert the run stops
/// immediately with usable evidence.
fn assert_failure(agent_body: &str, kind: &str, completed: u64, timeout_ms: &str) {
    let dir = tempfile::tempdir().unwrap();
    let rom = rom(dir.path());
    let agent = script(dir.path(), "agent.sh", agent_body);
    let run = explore(
        dir.path(),
        "run",
        &rom,
        &[
            "--agent",
            agent.to_str().unwrap(),
            "--timeout-ms",
            timeout_ms,
        ],
    );
    assert!(!run.status.success(), "run should fail: {kind}");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains(kind),
        "stderr should name the failure kind: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let summary = summary(dir.path(), "run");
    assert_eq!(summary["status"], "failed");
    assert_eq!(summary["failure"]["kind"], kind);
    assert_eq!(summary["seed"], 42);
    assert_eq!(summary["rom_sha256"], LOOP_ROM_SHA256);
    assert_eq!(summary["actions"], completed);
    assert_eq!(summary["failure"]["index"], completed);
    assert!(summary["last_snapshot"]["display"].is_array());
    assert_eq!(trace(dir.path(), "run").len() as u64, completed);
    assert_eq!(images(dir.path(), "run").len() as u64, completed);
}

#[test]
fn unexpected_agent_responses_stop_the_run_and_keep_evidence() {
    // Replies that are not JSON.
    assert_failure(
        "while IFS= read -r line; do echo 'not json'; done",
        "malformed_response",
        0,
        "5000",
    );
    // JSON that carries no action.
    assert_failure(
        "while IFS= read -r line; do echo '{\"snapshot\":1}'; done",
        "malformed_response",
        0,
        "5000",
    );
    // Answers once, then exits without answering the second turn.
    assert_failure(
        "head -n 1 >/dev/null; echo '{\"action\":{\"type\":\"step\"}}'; exit 3",
        "child_exit",
        1,
        "5000",
    );
    // Stops responding entirely.
    assert_failure(
        "while IFS= read -r line; do sleep 5; done",
        "timeout",
        0,
        "200",
    );
}

#[test]
fn an_emulator_error_response_stops_the_run() {
    assert_failure(
        "while IFS= read -r line; do echo '{\"action\":{\"type\":\"key\",\"key\":99,\"pressed\":true}}'; done",
        "emulator_error",
        0,
        "5000",
    );
}

#[test]
fn a_response_that_does_not_match_the_request_stops_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let rom = rom(dir.path());
    // Answers the ROM load, then always claims a `tick` regardless of the request.
    let emulator = script(
        dir.path(),
        "emulator.sh",
        "n=0\nwhile IFS= read -r line; do\n  if [ \"$n\" -eq 0 ]; then printf '%s\\n' '{\"index\":0,\"action\":{\"type\":\"load\"},\"snapshot\":{\"pc\":512}}'; else printf '%s\\n' '{\"index\":1,\"action\":{\"type\":\"tick\"},\"snapshot\":{\"pc\":512}}'; fi\n  n=$((n+1))\ndone",
    );
    let agent = script(
        dir.path(),
        "agent.sh",
        "while IFS= read -r line; do echo '{\"action\":{\"type\":\"step\"}}'; done",
    );
    let run = explore(
        dir.path(),
        "run",
        &rom,
        &[
            "--agent",
            agent.to_str().unwrap(),
            "--emulator",
            emulator.to_str().unwrap(),
            "--timeout-ms",
            "5000",
        ],
    );
    assert!(!run.status.success());
    let summary = summary(dir.path(), "run");
    assert_eq!(summary["failure"]["kind"], "mismatched_response");
    assert_eq!(summary["failure"]["index"], 0);
}
