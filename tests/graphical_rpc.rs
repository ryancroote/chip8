//! Native-window checks: run explicitly on a desktop (see TESTING.md).
use base64::Engine;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Cursor};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use tungstenite::{Message, WebSocket, client};

struct Emulator {
    child: Child,
    url: String,
    _rom: tempfile::TempDir,
}

impl Emulator {
    fn start(headless: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("toggle.ch8");
        // Toggle pixel (0,0) once per key press, then wait for release.
        std::fs::write(
            &rom,
            [
                0x60, 0x00, 0x61, 0x00, 0xA2, 0x10, 0xF2, 0x0A, 0xD0, 0x11, 0xE2, 0x9E, 0x12, 0x06,
                0x12, 0x0A, 0x80,
            ],
        )
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_chip8"));
        command.arg(&rom).args(["--rpc", "--port", "0"]);
        if headless {
            command.arg("--headless");
        }
        let child = command
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut emulator = Self {
            child,
            url: String::new(),
            _rom: dir,
        };
        let stderr = emulator.child.stderr.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            let result = reader.read_line(&mut line).map(|_| line);
            let _ = tx.send(result);
            // Keep draining diagnostics for the child's lifetime. Closing its
            // stderr pipe early makes disconnect diagnostics fail to write.
            for line in reader.lines() {
                match line {
                    Ok(line) => eprintln!("emulator: {line}"),
                    Err(_) => break,
                }
            }
        });
        let line = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("startup timed out")
            .unwrap();
        emulator.url = line
            .trim()
            .strip_prefix("RPC listening at ")
            .expect("RPC address")
            .to_owned();
        // Connect AFTER the native render loop starts: connecting during window
        // construction can hide renderer/RPC contention.
        std::thread::sleep(Duration::from_secs(2));
        emulator
    }

    fn connect(&self) -> WebSocket<TcpStream> {
        let address = self.url.strip_prefix("ws://").unwrap().parse().unwrap();
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(3)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        client(self.url.as_str(), stream)
            .expect("WebSocket handshake")
            .0
    }
}

impl Drop for Emulator {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn receive(socket: &mut WebSocket<TcpStream>) -> Value {
    let Message::Text(text) = socket.read().expect("RPC response within 3 seconds") else {
        panic!("expected JSON text");
    };
    serde_json::from_str(&text).unwrap()
}

fn pixels(frame: &Value) -> Vec<u8> {
    assert_eq!(frame["width"], 64);
    assert_eq!(frame["height"], 32);
    let png = base64::engine::general_purpose::STANDARD
        .decode(frame["data"].as_str().unwrap())
        .unwrap();
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    bytes.truncate(info.buffer_size());
    bytes
}

fn action(socket: &mut WebSocket<TcpStream>, keys: &[u8]) -> Value {
    socket
        .send(Message::Text(
            json!({
                "jsonrpc":"2.0", "id":1, "method":"chip8.action",
                "params":{"keys":keys,"frames":1}
            })
            .to_string()
            .into(),
        ))
        .unwrap();
    let response = receive(socket);
    assert_eq!(response["id"], 1);
    assert!(response.get("error").is_none(), "{response}");
    response["result"].clone()
}

fn repeated_display_changes(headless: bool) {
    let emulator = Emulator::start(headless);
    let mut socket = emulator.connect();
    let initial = receive(&mut socket);
    assert_eq!(initial["method"], "chip8.frame");
    assert_eq!(pixels(&initial["params"]), vec![0; 64 * 32]);
    for lit in [true, false, true] {
        let mut expected = vec![0; 64 * 32];
        expected[0] = if lit { 255 } else { 0 };
        assert_eq!(pixels(&action(&mut socket, &[5])), expected);
        assert_eq!(pixels(&action(&mut socket, &[])), expected);
    }
}

fn controller_handoff(headless: bool) {
    let emulator = Emulator::start(headless);
    let mut first = emulator.connect();
    receive(&mut first);
    let lit = action(&mut first, &[5]);
    action(&mut first, &[]);
    first.close(None).unwrap();
    // Drop without waiting for the close reply: the server must still accept
    // a replacement controller, and the game must not reset.
    drop(first);
    let mut second = emulator.connect();
    assert_eq!(receive(&mut second)["params"], lit);
    // Pause beyond the server's socket poll timeout: thinking is not EOF.
    std::thread::sleep(Duration::from_millis(300));
    let dark = action(&mut second, &[5]);
    assert_eq!(pixels(&dark), vec![0; 64 * 32]);
    action(&mut second, &[]);
    // No WebSocket close handshake: simulate an abruptly lost controller.
    drop(second);
    std::thread::sleep(Duration::from_millis(300));
    let mut third = emulator.connect();
    assert_eq!(receive(&mut third)["params"], dark);
    assert_eq!(pixels(&action(&mut third, &[5])), pixels(&lit));
}

#[test]
#[ignore = "requires a native desktop; run with --ignored --test-threads=1"]
fn graphical_controller_handoff_preserves_the_paused_game() {
    controller_handoff(false);
}

#[test]
fn headless_control_for_controller_handoff() {
    controller_handoff(true);
}

#[test]
#[ignore = "requires a native desktop; run with --ignored --test-threads=1"]
fn graphical_rpc_progresses_after_the_window_loop_starts() {
    repeated_display_changes(false);
}

#[test]
fn headless_control_for_graphical_progress_probe() {
    repeated_display_changes(true);
}
