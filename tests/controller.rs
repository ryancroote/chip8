use base64::Engine;
use chip8::controller::{Error, Session, Stage, Timeouts};
use serde_json::{Value, json};
use std::io::Cursor;
use std::net::{TcpListener, TcpStream};
use std::thread::JoinHandle;
use std::time::Duration;
use tungstenite::{Message, WebSocket};

struct Peer {
    endpoint: String,
    thread: Option<JoinHandle<()>>,
}

impl Peer {
    fn start(run: impl FnOnce(WebSocket<TcpStream>) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline, "peer accept timeout");
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            run(tungstenite::accept(stream).unwrap());
        });
        Self {
            endpoint,
            thread: Some(thread),
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        let result = self.thread.take().unwrap().join();
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}

fn initial() -> (Value, Vec<u8>) {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 64, 32);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let mut pixels = [0; 2048];
        pixels[0] = 255;
        writer.write_image_data(&pixels).unwrap();
    }
    (
        json!({"jsonrpc":"2.0", "method":"chip8.frame", "params": {
            "width":64,"height":32,"format":"png",
            "data":base64::engine::general_purpose::STANDARD.encode(&bytes)
        }}),
        bytes,
    )
}

fn send(socket: &mut WebSocket<TcpStream>, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .unwrap();
}

#[test]
fn attach_validates_and_saves_original_image_without_sending_an_action() {
    let (mut notification, bytes) = initial();
    notification["extra"] = json!(true);
    notification["params"]["extra"] = json!(true);
    let peer = Peer::start(move |mut socket| {
        socket.send(Message::Ping(vec![1].into())).unwrap();
        send(&mut socket, notification);
        loop {
            match socket.read().unwrap() {
                Message::Pong(_) => {}
                Message::Close(_) => {
                    let _ = socket.flush();
                    break;
                }
                message => panic!("read-only session sent {message:?}"),
            }
        }
    });
    let session = Session::connect(&peer.endpoint, Timeouts::default()).unwrap();
    let frame = session.observation();
    assert_eq!(
        (frame.width(), frame.height(), frame.format()),
        (64, 32, "png")
    );
    assert_eq!(frame.png_bytes(), bytes);
    let mut reader = png::Decoder::new(Cursor::new(frame.png_bytes()))
        .read_info()
        .unwrap();
    let mut pixels = [0; 2048];
    reader.next_frame(&mut pixels).unwrap();
    assert_eq!(pixels[0], 255);
    assert!(pixels[1..].iter().all(|p| *p == 0));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("display.png");
    frame.save_png(&path).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert!(frame.save_png(dir.path()).is_err());
    assert_eq!(session.observation().png_bytes(), bytes);
    session.disconnect().unwrap();
}

#[test]
fn rejects_invalid_initial_envelopes_and_frames() {
    let (valid, png) = initial();
    let mut cases = vec![json!(null), json!({}), json!([valid.clone()])];
    for (path, value) in [
        (vec!["jsonrpc"], json!("1.0")),
        (vec!["method"], json!("other")),
        (vec!["id"], json!(null)),
        (vec!["result"], json!({})),
        (vec!["error"], json!({"code":-1,"message":"no"})),
        (vec!["params", "width"], json!(128)),
        (vec!["params", "height"], json!(64)),
        (vec!["params", "format"], json!("jpeg")),
        (vec!["params", "data"], json!("%%%")),
        (
            vec!["params", "data"],
            json!(base64::engine::general_purpose::STANDARD.encode(b"not PNG")),
        ),
        (
            vec!["params", "data"],
            json!(base64::engine::general_purpose::STANDARD.encode(&png[..png.len() - 12])),
        ),
        (vec!["params", "data"], json!("A".repeat(24 * 1024))),
    ] {
        let mut bad = valid.clone();
        let mut field = &mut bad;
        for key in path {
            field = &mut field[key];
        }
        *field = value;
        cases.push(bad);
    }
    cases.push(json!({"jsonrpc":"2.0","method":"chip8.frame","params":null}));
    for (width, height, color, depth) in [
        (32, 32, png::ColorType::Grayscale, png::BitDepth::Eight),
        (64, 16, png::ColorType::Grayscale, png::BitDepth::Eight),
        (64, 32, png::ColorType::Rgb, png::BitDepth::Eight),
        (64, 32, png::ColorType::Grayscale, png::BitDepth::One),
        (64, 32, png::ColorType::Grayscale, png::BitDepth::Sixteen),
    ] {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(color);
            encoder.set_depth(depth);
            let mut writer = encoder.write_header().unwrap();
            let size =
                (width as usize * color.samples() * depth as usize).div_ceil(8) * height as usize;
            writer.write_image_data(&vec![0; size]).unwrap();
        }
        let mut bad = valid.clone();
        bad["params"]["data"] = json!(base64::engine::general_purpose::STANDARD.encode(bytes));
        cases.push(bad);
    }
    for notification in cases {
        let label = notification.to_string();
        let peer = Peer::start(move |mut socket| {
            send(&mut socket, notification);
        });
        assert!(
            matches!(
                Session::connect(&peer.endpoint, Timeouts::default()),
                Err(Error::InvalidProtocol {
                    stage: Stage::InitialObservation,
                    ..
                })
            ),
            "accepted {label}"
        );
    }
}

fn short_timeouts() -> Timeouts {
    Timeouts {
        connect: Duration::from_millis(100),
        handshake: Duration::from_millis(100),
        observation: Duration::from_millis(100),
        disconnect: Duration::from_millis(100),
    }
}

#[test]
fn missing_initial_and_control_floods_expire_a_total_deadline() {
    for flood in [false, true] {
        let peer = Peer::start(move |mut socket| {
            let until = std::time::Instant::now() + Duration::from_millis(350);
            while std::time::Instant::now() < until {
                if flood && socket.send(Message::Pong(vec![1].into())).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        let start = std::time::Instant::now();
        assert!(matches!(
            Session::connect(&peer.endpoint, short_timeouts()),
            Err(Error::Deadline {
                stage: Stage::InitialObservation
            })
        ));
        assert!(start.elapsed() < Duration::from_millis(300));
    }
}

#[test]
fn disconnect_is_bounded_even_if_peer_never_acknowledges_close() {
    let (notification, _) = initial();
    let peer = Peer::start(move |mut socket| {
        send(&mut socket, notification);
        // Deliberately never read/acknowledge the client's close frame.
        std::thread::sleep(Duration::from_millis(350));
    });
    let session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
    let start = std::time::Instant::now();
    assert!(matches!(
        session.disconnect(),
        Err(Error::Deadline {
            stage: Stage::Disconnect
        })
    ));
    assert!(start.elapsed() < Duration::from_millis(300));
}

#[test]
fn abrupt_loss_during_disconnect_is_not_a_graceful_acknowledgement() {
    let (notification, _) = initial();
    let peer = Peer::start(move |mut socket| {
        send(&mut socket, notification);
        assert!(matches!(socket.read().unwrap(), Message::Close(_)));
        // Drop TCP without flushing tungstenite's queued close response.
    });
    let session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
    assert!(matches!(
        session.disconnect(),
        Err(Error::Disconnected {
            stage: Stage::Disconnect
        })
    ));
}

#[test]
fn close_truncation_binary_and_oversized_messages_are_not_observations() {
    for mode in [
        "close",
        "truncated",
        "binary",
        "oversized",
        "malformed JSON",
    ] {
        let peer = Peer::start(move |mut socket| match mode {
            "close" => {
                let _ = socket.close(None);
            }
            "truncated" => {
                use std::io::Write;
                socket.get_mut().write_all(&[0x81, 10, b'{']).unwrap();
            }
            "binary" => {
                let _ = socket.send(Message::Binary(vec![0; 20].into()));
            }
            "oversized" => {
                let _ = socket.send(Message::Text("x".repeat(40 * 1024).into()));
            }
            _ => {
                let _ = socket.send(Message::Text("{".into()));
            }
        });
        let result = Session::connect(&peer.endpoint, Timeouts::default());
        if matches!(mode, "close" | "truncated") {
            assert!(
                matches!(
                    result,
                    Err(Error::Disconnected {
                        stage: Stage::InitialObservation
                    })
                ),
                "{mode}"
            );
        } else {
            assert!(
                matches!(
                    result,
                    Err(Error::InvalidProtocol {
                        stage: Stage::InitialObservation,
                        ..
                    })
                ),
                "{mode}"
            );
        }
    }
}

#[test]
fn endpoint_configuration_and_connection_errors_are_distinct() {
    for endpoint in [
        "http://127.0.0.1:1234",
        "wss://127.0.0.1:1234",
        "ws://192.0.2.1:1234",
        "ws://example.org:1234",
        "ws://127.0.0.1",
        "ws://127.0.0.1:0",
        "ws://user@127.0.0.1:1234",
        "bad",
    ] {
        assert!(
            matches!(
                Session::connect(endpoint, short_timeouts()),
                Err(Error::InvalidEndpoint(_))
            ),
            "{endpoint}"
        );
    }
    assert!(matches!(
        Session::connect(
            "ws://127.0.0.1:1",
            Timeouts {
                observation: Duration::ZERO,
                ..short_timeouts()
            }
        ),
        Err(Error::InvalidTimeout)
    ));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    drop(listener);
    assert!(matches!(
        Session::connect(&endpoint, short_timeouts()),
        Err(Error::Connection {
            stage: Stage::Connect,
            ..
        })
    ));
}

#[test]
fn stalled_and_failed_websocket_handshakes_are_bounded_and_classified() {
    for stalled in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let until = std::time::Instant::now() + Duration::from_secs(2);
            let mut stream = loop {
                if let Ok((stream, _)) = listener.accept() {
                    break stream;
                }
                assert!(std::time::Instant::now() < until);
                std::thread::sleep(Duration::from_millis(2));
            };
            stream
                .set_write_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            if stalled {
                std::thread::sleep(Duration::from_millis(350));
            } else {
                use std::io::Write;
                stream
                    .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            }
        });
        let start = std::time::Instant::now();
        let result = Session::connect(&endpoint, short_timeouts());
        if stalled {
            assert!(matches!(
                result,
                Err(Error::Deadline {
                    stage: Stage::Handshake
                })
            ));
        } else {
            assert!(matches!(
                result,
                Err(Error::Connection {
                    stage: Stage::Handshake,
                    ..
                })
            ));
        }
        assert!(start.elapsed() < Duration::from_millis(300));
        thread.join().unwrap();
    }
}
