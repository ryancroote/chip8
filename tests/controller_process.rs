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
        // Initialize coordinates/sprite, then toggle (0,0) once per 11
        // instructions (= one RPC frame). No input or wall-clock dependency.
        let mut bytes = vec![0x60, 0x00, 0x61, 0x00, 0xa2, 0x1c, 0xd0, 0x11];
        for _ in 0..9 {
            bytes.extend([0x73, 0x00]);
        }
        bytes.extend([0x12, 0x06, 0x80]);
        Self::with_rom(&bytes)
    }
    fn with_rom(bytes: &[u8]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("deterministic.ch8");
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
fn bounded_actions_and_empty_execution_return_confirmed_pixels() {
    let emulator = Emulator::start();
    let mut session = Session::connect(&emulator.endpoint, Timeouts::default()).unwrap();
    assert_eq!(pixels(&session), vec![0; 2048]);
    let confirmed = session.action(&[0xa], 1).unwrap().clone();
    let mut lit = vec![0; 2048];
    lit[0] = 255;
    assert_eq!(pixels(&session), lit);
    let dir = tempfile::tempdir().unwrap();
    assert!(confirmed.save_png(dir.path()).is_err());
    assert_eq!(session.observation(), &confirmed);
    std::thread::sleep(Duration::from_millis(100));
    session.action(&[], 2).unwrap();
    assert_eq!(pixels(&session), lit, "two explicit frames preserve parity");
    session.action(&[], 1).unwrap();
    assert_eq!(pixels(&session), vec![0; 2048]);
    session.disconnect().unwrap();
}

#[test]
fn exact_frame_counts_include_sixty_frames_without_hidden_execution() {
    // Draw the next pixel every 11 instructions, i.e. once per RPC frame.
    let mut rom = vec![0x60, 0, 0x61, 0, 0xa2, 0x1c, 0xd0, 0x11, 0x70, 1];
    for _ in 0..8 {
        rom.extend([0x73, 0]);
    }
    rom.extend([0x12, 6, 0x80]);
    let emulator = Emulator::with_rom(&rom);
    let mut session = Session::connect(&emulator.endpoint, Timeouts::default()).unwrap();
    for (keys, frames) in [(vec![16], 1), (vec![0, 0], 1), (vec![], 0), (vec![], 61)] {
        assert!(matches!(
            session.action(&keys, frames),
            Err(Error::InvalidAction(_))
        ));
    }
    session.action(&[0xf], 1).unwrap();
    let mut expected = vec![0; 2048];
    expected[0] = 255;
    assert_eq!(
        pixels(&session),
        expected,
        "invalid actions must not advance"
    );
    session.action(&[], 2).unwrap();
    expected[1..3].fill(255);
    assert_eq!(pixels(&session), expected);
    let saved = session.observation().clone();
    let dir = tempfile::tempdir().unwrap();
    saved.save_png(dir.path().join("confirmed.png")).unwrap();
    std::thread::sleep(Duration::from_millis(120));
    session.disconnect().unwrap();
    let mut session = Session::connect(&emulator.endpoint, Timeouts::default()).unwrap();
    assert_eq!(
        session.observation(),
        &saved,
        "saving/thinking advanced time"
    );
    let start = std::time::Instant::now();
    session.action(&[], 60).unwrap();
    assert!(
        start.elapsed() >= Duration::from_millis(900),
        "server owns pacing"
    );
    expected[3..63].fill(255);
    assert_eq!(pixels(&session), expected, "exactly 60 more frames");
    session.disconnect().unwrap();
}

#[test]
fn explicit_settling_releases_keys_and_zero_settling_adds_no_frames() {
    // Exactly 11 instructions per frame. Toggle x=0 when A is held,
    // x=1 when released. Each loop initializes all values it uses.
    let rom = [
        0x60, 0x0a, 0x61, 0, 0x62, 0, 0xa2, 0x18, 0xe0, 0x9e, 0x61, 1, 0xe0, 0xa1, 0x73, 0, 0xd1,
        0x21, 0x73, 0, 0x73, 0, 0x12, 0, 0x80,
    ];
    let emulator = Emulator::with_rom(&rom);
    let mut session = Session::connect(&emulator.endpoint, Timeouts::default()).unwrap();
    assert!(matches!(
        session.action_with_settling(&[10], 1, 61),
        Err(Error::InvalidAction(_))
    ));
    session.action_with_settling(&[10], 1, 0).unwrap();
    let mut expected = vec![0; 2048];
    expected[0] = 255;
    assert_eq!(
        pixels(&session),
        expected,
        "zero settling runs only the held frame"
    );
    session.action_with_settling(&[10], 1, 1).unwrap();
    expected[0] = 0;
    expected[1] = 255;
    assert_eq!(
        pixels(&session),
        expected,
        "settling must execute with A released"
    );
    session.action_with_settling(&[10], 60, 60).unwrap();
    assert_eq!(
        pixels(&session),
        expected,
        "both 60-frame stages preserve parity"
    );
    session.disconnect().unwrap();
}

#[test]
fn settling_executes_exactly_the_requested_total_frames() {
    // Draw successive pixels once per 11 instructions (one RPC frame).
    let mut rom = vec![0x60, 0, 0x61, 0, 0xa2, 0x1c, 0xd0, 0x11, 0x70, 1];
    for _ in 0..8 {
        rom.extend([0x73, 0]);
    }
    rom.extend([0x12, 6, 0x80]);
    let emulator = Emulator::with_rom(&rom);
    let mut session = Session::connect(&emulator.endpoint, Timeouts::default()).unwrap();
    session.action_with_settling(&[10], 1, 0).unwrap();
    session.action_with_settling(&[10], 1, 60).unwrap();
    let mut expected = vec![0; 2048];
    expected[..62].fill(255);
    assert_eq!(
        pixels(&session),
        expected,
        "one plus one plus sixty frames, no hidden waits"
    );
    session.disconnect().unwrap();
}

#[test]
fn keypad_chords_are_held_simultaneously_not_split_into_single_keys() {
    // Only draw if A and F are both held at the same time. Once drawn,
    // stay in a stable loop, making the resulting pixel unambiguous.
    let rom = [
        0x60, 0x0a, 0x61, 0x0f, 0x62, 0, 0x63, 0, 0xa2, 0x16, 0xe0, 0x9e, 0x12, 0x0a, 0xe1, 0x9e,
        0x12, 0x0a, 0xd2, 0x31, 0x12, 0x14, 0x80,
    ];
    let emulator = Emulator::with_rom(&rom);
    let mut session = Session::connect(&emulator.endpoint, Timeouts::default()).unwrap();
    session.action(&[0xa], 2).unwrap();
    assert_eq!(pixels(&session), vec![0; 2048]);
    session.action(&[0xf], 2).unwrap();
    assert_eq!(pixels(&session), vec![0; 2048]);
    session.action(&[0xa, 0xf], 1).unwrap();
    let mut expected = vec![0; 2048];
    expected[0] = 255;
    assert_eq!(pixels(&session), expected);
    session.disconnect().unwrap();
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
