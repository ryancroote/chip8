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
