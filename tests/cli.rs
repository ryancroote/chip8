use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;
use tungstenite::{Message, connect};

#[test]
fn headless_rpc_cli_starts_a_loopback_controller_and_sends_initial_frame() {
    let dir = tempfile::tempdir().unwrap();
    let rom_path = dir.path().join("loop.ch8");
    std::fs::write(&rom_path, [0x12, 0x00]).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_chip8"))
        .args([
            "--rpc",
            "--headless",
            "--port",
            "0",
            rom_path.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut stderr = BufReader::new(stderr);
    let mut line = String::new();
    stderr.read_line(&mut line).unwrap();
    assert!(
        line.starts_with("RPC listening at ws://127.0.0.1:"),
        "{line}"
    );
    let address = line.trim().strip_prefix("RPC listening at ws://").unwrap();
    let (mut client, _) = connect(format!("ws://{address}")).unwrap();
    let tungstenite::stream::MaybeTlsStream::Plain(stream) = client.get_mut() else {
        unreachable!("test client uses plain WebSocket")
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let Message::Text(frame) = client.read().unwrap() else {
        panic!("expected initial frame notification")
    };
    let frame: Value = serde_json::from_str(&frame).unwrap();
    assert_eq!(frame["method"], "chip8.frame");
    assert_eq!(frame["params"]["width"], 64);
    assert_eq!(frame["params"]["height"], 32);
    assert_eq!(frame["params"]["format"], "png");

    child.kill().unwrap();
    let _ = child.wait();
}

#[test]
fn help_describes_the_unified_executable() {
    let help = Command::new(env!("CARGO_BIN_EXE_chip8"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("chip8 agent"));
}

#[test]
fn headless_mode_requires_rpc_and_rpc_requires_a_rom() {
    let binary = env!("CARGO_BIN_EXE_chip8");
    let headless = Command::new(binary).arg("--headless").output().unwrap();
    assert!(!headless.status.success());
    assert!(String::from_utf8_lossy(&headless.stderr).contains("--headless requires --rpc"));

    let rpc = Command::new(binary).arg("--rpc").output().unwrap();
    assert!(!rpc.status.success());
    assert!(String::from_utf8_lossy(&rpc.stderr).contains("RPC mode requires a ROM path"));
}
