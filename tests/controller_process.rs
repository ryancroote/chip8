//! Public client against a real, caller-owned headless emulator.
use chip8::controller::{Error, Session, Stage, Timeouts};
use serde_json::json;
use std::io::{BufRead, BufReader, Cursor};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use tungstenite::Message;

struct Emulator {
    child: Child,
    endpoint: String,
    _dir: tempfile::TempDir,
}
impl Emulator {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("toggle.ch8");
        // Initialize coordinates/sprite, then toggle (0,0) once per 11
        // instructions (= one RPC frame). No input or wall-clock dependency.
        let mut bytes = vec![0x60, 0x00, 0x61, 0x00, 0xa2, 0x1c, 0xd0, 0x11];
        for _ in 0..9 {
            bytes.extend([0x73, 0x00]);
        }
        bytes.extend([0x12, 0x06, 0x80]);
        std::fs::write(&rom, bytes).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_chip8"))
            .arg(rom)
            .args(["--headless", "--rpc", "--port", "0"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut emulator = Self {
            child,
            endpoint: String::new(),
            _dir: dir,
        };
        let stderr = emulator.child.stderr.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut lines = BufReader::new(stderr).lines();
            let _ = tx.send(lines.next().unwrap().unwrap());
            for line in lines {
                if line.is_err() {
                    break;
                }
            }
        });
        let line = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("bounded startup");
        emulator.endpoint = line
            .trim()
            .strip_prefix("RPC listening at ")
            .unwrap()
            .to_owned();
        emulator
    }
    fn advance_one_frame(&self) {
        let address = self
            .endpoint
            .strip_prefix("ws://")
            .unwrap()
            .parse()
            .unwrap();
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let (mut socket, _) = tungstenite::client(self.endpoint.as_str(), stream).unwrap();
        socket.read().unwrap();
        socket
            .send(Message::Text(
                json!({"jsonrpc":"2.0","id":1,"method":"chip8.action",
            "params":{"keys":[],"frames":1}})
                .to_string()
                .into(),
            ))
            .unwrap();
        let Message::Text(response) = socket.read().unwrap() else {
            panic!("action response")
        };
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(response.get("result").is_some(), "{response}");
        socket.close(None).unwrap();
        let _ = socket.read();
    }
}
impl Drop for Emulator {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn pixels(session: &Session) -> Vec<u8> {
    let mut reader = png::Decoder::new(Cursor::new(session.observation().png_bytes()))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; 2048];
    reader.next_frame(&mut pixels).unwrap();
    pixels
}

#[test]
fn read_only_use_preserves_paused_game_across_controller_handoff() {
    let mut emulator = Emulator::start();
    let session = Session::connect(&emulator.endpoint, Timeouts::default()).unwrap();
    assert_eq!(pixels(&session), vec![0; 2048]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("initial.png");
    session.observation().save_png(&path).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        session.observation().png_bytes()
    );
    std::thread::sleep(Duration::from_millis(120));
    // A second controller waits for ownership; it must not steal the game or
    // fall back to another instance when its handshake budget expires.
    assert!(matches!(
        Session::connect(
            &emulator.endpoint,
            Timeouts {
                handshake: Duration::from_millis(100),
                ..Timeouts::default()
            }
        ),
        Err(Error::Deadline {
            stage: Stage::Handshake
        })
    ));
    session.disconnect().unwrap();
    assert!(emulator.child.try_wait().unwrap().is_none());
    let session = Session::connect(&emulator.endpoint, Timeouts::default()).unwrap();
    assert_eq!(
        pixels(&session),
        vec![0; 2048],
        "inspection advanced the game"
    );
    session.disconnect().unwrap();
    // An independent controller advances exactly one frame. Reattaching must
    // observe that progress, not restart or load a fresh game.
    emulator.advance_one_frame();
    let session = Session::connect(&emulator.endpoint, Timeouts::default()).unwrap();
    let mut expected = vec![0; 2048];
    expected[0] = 255;
    assert_eq!(pixels(&session), expected);
    session.disconnect().unwrap();
    let session = Session::connect(&emulator.endpoint, Timeouts::default()).unwrap();
    assert_eq!(pixels(&session), expected);
    drop(session); // Abrupt release also permits handoff, with no shutdown.
    let session = Session::connect(&emulator.endpoint, Timeouts::default()).unwrap();
    assert_eq!(pixels(&session), expected);
    session.disconnect().unwrap();
    assert!(emulator.child.try_wait().unwrap().is_none());
}
