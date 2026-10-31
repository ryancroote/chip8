use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};

fn agent(input: &[Value]) -> (bool, Vec<Value>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_chip8-agent"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for value in input {
        writeln!(stdin, "{value}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    (
        output.status.success(),
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect(),
    )
}

fn command(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_chip8-agent"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn documented_scenario_records_promotes_and_replays() {
    let dir = tempfile::tempdir().unwrap();
    let candidate = dir.path().join("candidate.json");
    let approved = dir.path().join("approved.json");
    let candidate = candidate.to_str().unwrap();
    let approved = approved.to_str().unwrap();
    assert!(
        command(&["record", "examples/agent-scenario.json", candidate])
            .status
            .success()
    );
    let fixture: Value = serde_json::from_slice(&std::fs::read(candidate).unwrap()).unwrap();
    let snapshot = &fixture["actions"][9]["expected"];
    assert_eq!(snapshot["v"][0], 5);
    assert_eq!(snapshot["v"][1], 33);
    assert_eq!(snapshot["delay"], 4);
    assert_eq!(snapshot["sound"], 4);
    assert_eq!(snapshot["pc"], 520);
    assert!(command(&["review", candidate, approved]).status.success());
    // Tests explicitly exercise promotion; this does not claim human approval.
    assert!(
        command(&["promote", candidate, approved, "--replace"])
            .status
            .success()
    );
    assert!(command(&["replay", approved]).status.success());
}

#[test]
fn recording_requires_separate_candidates_and_explicit_promotion() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.json");
    let candidate = dir.path().join("candidate.json");
    let approved = dir.path().join("approved.json");
    let source = source.to_str().unwrap();
    let candidate = candidate.to_str().unwrap();
    let approved = approved.to_str().unwrap();
    let input = json!({"version":1,"actions":[
        {"action":{"type":"reset"}},
        {"action":{"type":"seed","seed":1}},
        {"action":{"type":"load","bytes":[192,255],"at":512}},
        {"action":{"type":"step"},"checkpoint":true}
    ]})
    .to_string();
    std::fs::write(source, &input).unwrap();
    assert!(command(&["record", source, candidate]).status.success());
    assert_eq!(std::fs::read_to_string(source).unwrap(), input);
    let recorded: Value = serde_json::from_slice(&std::fs::read(candidate).unwrap()).unwrap();
    assert_eq!(recorded["actions"][3]["expected"]["v"][0], 33);
    assert!(!recorded["actions"][0]["snapshot"].is_null());
    assert!(!command(&["record", source, source]).status.success());
    assert!(!command(&["record", source, candidate]).status.success());
    assert!(!command(&["promote", candidate, approved]).status.success());
    assert!(!std::path::Path::new(approved).exists());
    let review = command(&["review", candidate, approved]);
    assert!(review.status.success());
    assert!(
        String::from_utf8(review.stdout)
            .unwrap()
            .contains("initial")
    );
    assert!(
        command(&["promote", candidate, approved, "--replace"])
            .status
            .success()
    );
    assert!(command(&["replay", approved]).status.success());
    assert!(!command(&["record", source, approved]).status.success());
    assert_eq!(
        std::fs::read(approved).unwrap(),
        std::fs::read(candidate).unwrap()
    );
}

#[test]
fn fixtures_compare_only_explicit_checkpoints_and_stop_at_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.json");
    let snapshot = agent(&[json!({"action":{"type":"inspect"}})]).1[0]["snapshot"].clone();
    let mut fixture = json!({"version":1,"actions":[
        {"action":{"type":"inspect"}},
        {"action":{"type":"reset"},"checkpoint":true,"expected":snapshot},
        {"action":{"type":"inspect"}}
    ]});
    std::fs::write(&path, fixture.to_string()).unwrap();
    let result = command(&["replay", path.to_str().unwrap()]);
    assert!(result.status.success(), "{:?}", result);
    assert_eq!(String::from_utf8(result.stdout).unwrap().lines().count(), 3);
    fixture["actions"][1]["expected"]["random_state"] = json!(99);
    std::fs::write(&path, fixture.to_string()).unwrap();
    let result = command(&["replay", path.to_str().unwrap()]);
    assert!(!result.status.success());
    let lines: Vec<Value> = String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["index"], 1);
    assert!(lines[1]["error"].as_str().unwrap().contains("random_state"));
    assert_eq!(lines[1]["last_snapshot"], lines[0]["snapshot"]);
}

#[test]
fn keypad_timers_and_seeded_randomness_are_explicit_and_reproducible() {
    let actions = vec![
        json!({"action":{"type":"seed","seed":1}}),
        json!({"action":{"type":"load","bytes":[240,10,240,21,193,255],"at":512}}),
        json!({"action":{"type":"key","key":5,"pressed":true}}),
        json!({"action":{"type":"step"}}),
        json!({"action":{"type":"step"}}),
        json!({"action":{"type":"key","key":5,"pressed":false}}),
        json!({"action":{"type":"step"}}),
        json!({"action":{"type":"tick"}}),
    ];
    let (ok, trace) = agent(&actions);
    assert!(ok);
    assert_eq!(trace[3]["snapshot"]["v"][0], 5);
    assert_eq!(trace[6]["snapshot"]["delay"], 5);
    assert_eq!(trace[7]["snapshot"]["delay"], 4);
    assert_eq!(trace[6]["snapshot"]["v"][1], 33);
    assert_eq!(trace[6]["snapshot"]["keypad_input"][5], false);
    assert_eq!(trace, agent(&actions).1);
    assert!(!agent(&[json!({"action":{"type":"key","key":16,"pressed":true}})]).0);
}

#[test]
fn failures_stop_the_trace_and_preserve_last_successful_snapshot() {
    for bytes in [vec![0, 0], vec![31, 255]] {
        let (ok, trace) = agent(&[
            json!({"action":{"type":"load","bytes":bytes,"at":512}}),
            json!({"action":{"type":"step"}}),
            json!({"action":{"type":"step"}}),
            json!({"action":{"type":"reset"}}),
        ]);
        assert!(!ok);
        let failure = trace.last().unwrap();
        assert!(failure["error"].is_string());
        assert_eq!(failure["last_snapshot"], trace[trace.len() - 2]["snapshot"]);
        assert!(trace.len() < 4);
    }
}

#[test]
fn agent_loads_steps_inspects_and_resets_with_complete_snapshots() {
    let (ok, trace) = agent(&[
        json!({"action":{"type":"load","bytes":[96,5],"at":512}}),
        json!({"action":{"type":"step"}}),
        json!({"action":{"type":"inspect"}}),
        json!({"action":{"type":"reset"}}),
    ]);
    assert!(ok);
    assert_eq!(trace.len(), 4);
    let state = &trace[1]["snapshot"];
    assert_eq!(state["pc"], 514);
    assert_eq!(state["v"][0], 5);
    assert_eq!(state["memory"].as_array().unwrap().len(), 4096);
    assert_eq!(state["display"].as_array().unwrap().len(), 32);
    for field in [
        "i",
        "stack",
        "delay",
        "sound",
        "keypad_input",
        "random_state",
    ] {
        assert!(!state[field].is_null(), "missing {field}");
    }
    assert_eq!(trace[2]["snapshot"], *state);
    assert_eq!(trace[3]["snapshot"]["pc"], 512);
    assert_eq!(trace[3]["snapshot"]["v"][0], 0);
}
