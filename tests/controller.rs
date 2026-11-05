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
fn invalid_actions_are_local_and_valid_chords_have_distinct_response_ids() {
    let (notification, _) = initial();
    let peer = Peer::start(move |mut socket| {
        send(&mut socket, notification.clone());
        let mut previous = None;
        for (keys, frames) in [(json!([10, 15]), 60), (json!([]), 1)] {
            let Message::Text(text) = socket.read().unwrap() else {
                panic!("action text")
            };
            let request: Value = serde_json::from_str(text.as_str()).unwrap();
            assert_eq!(request["method"], "chip8.action");
            assert_eq!(request["params"], json!({"keys":keys,"frames":frames}));
            let id = request["id"].as_u64().unwrap();
            assert_ne!(Some(id), previous);
            previous = Some(id);
            send(
                &mut socket,
                json!({"jsonrpc":"2.0","id":id,"result":notification["params"]}),
            );
        }
        assert!(matches!(socket.read().unwrap(), Message::Close(_)));
        let _ = socket.flush();
    });
    let mut session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
    let before = session.observation().clone();
    for (keys, frames) in [
        (vec![16], 1),
        (vec![255], 1),
        (vec![1, 1], 1),
        (vec![], 0),
        (vec![], 61),
        (vec![], u32::MAX),
    ] {
        assert!(matches!(
            session.action(&keys, frames),
            Err(Error::InvalidAction(_))
        ));
        assert_eq!(session.observation(), &before);
    }
    session.action(&[10, 15], 60).unwrap();
    session.action(&[], 1).unwrap();
    session.disconnect().unwrap();
}

#[test]
fn explicit_settling_is_prevalidated_and_ordered_after_confirmed_primary() {
    let (notification, _) = initial();
    let peer = Peer::start(move |mut socket| {
        send(&mut socket, notification.clone());
        for (keys, frames) in [(json!([10, 15]), 60), (json!([]), 60), (json!([1]), 1)] {
            let Message::Text(text) = socket.read().unwrap() else {
                panic!("action")
            };
            let request: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(request["params"], json!({"keys":keys,"frames":frames}));
            // No following stage may arrive before this acknowledgement.
            socket
                .get_mut()
                .set_read_timeout(Some(Duration::from_millis(20)))
                .unwrap();
            assert!(socket.read().is_err());
            socket
                .get_mut()
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            send(
                &mut socket,
                json!({"jsonrpc":"2.0","id":request["id"],"result":notification["params"]}),
            );
        }
        assert!(matches!(socket.read().unwrap(), Message::Close(_)));
        let _ = socket.flush();
    });
    let mut session = Session::connect(&peer.endpoint, Timeouts::default()).unwrap();
    for (keys, frames, settling) in [
        (vec![16], 1, 1),
        (vec![1, 1], 1, 1),
        (vec![], 0, 1),
        (vec![], 61, 1),
        (vec![1], 1, 61),
        (vec![1], 1, u32::MAX),
    ] {
        assert!(matches!(
            session.action_with_settling(&keys, frames, settling),
            Err(Error::InvalidAction(_))
        ));
    }
    session.action_with_settling(&[10, 15], 60, 60).unwrap();
    session.action_with_settling(&[1], 1, 0).unwrap();
    session.disconnect().unwrap();
}

#[test]
fn primary_failure_never_sends_settling() {
    for mode in ["rejection", "disconnect", "protocol", "deadline"] {
        let (notification, _) = initial();
        let peer = Peer::start(move |mut socket| {
            send(&mut socket, notification.clone());
            assert!(matches!(socket.read().unwrap(), Message::Text(_)));
            match mode {
                "rejection" => send(
                    &mut socket,
                    json!({"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"primary fault"}}),
                ),
                "protocol" => send(
                    &mut socket,
                    json!({"jsonrpc":"2.0","id":999,"result":notification["params"]}),
                ),
                "disconnect" => return,
                _ => {}
            }
            assert!(
                !matches!(socket.read(), Ok(Message::Text(_))),
                "settling after failed primary"
            );
        });
        let mut session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
        let before = session.observation().clone();
        let error = session.action_with_settling(&[10], 1, 1).unwrap_err();
        if mode == "rejection" {
            assert!(matches!(error, Error::Rpc { code: -1, .. }));
        } else {
            assert!(matches!(error, Error::UnknownOutcome { .. }));
        }
        assert_eq!(session.observation(), &before);
        drop(session);
    }
}

#[test]
fn settling_has_its_own_duration_aware_deadline_and_returns_final_image() {
    let (notification, primary_bytes) = initial();
    let mut final_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut final_bytes, 64, 32);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[0; 2048])
            .unwrap();
    }
    let expected = final_bytes.clone();
    let peer = Peer::start(move |mut socket| {
        send(&mut socket, notification.clone());
        for stage in 1..=2 {
            let Message::Text(text) = socket.read().unwrap() else {
                panic!("action")
            };
            let request: Value = serde_json::from_str(&text).unwrap();
            let mut frame = notification["params"].clone();
            if stage == 2 {
                // Exceeds transport allowance, but not settling's 60-frame budget.
                std::thread::sleep(Duration::from_millis(200));
                frame["data"] =
                    json!(base64::engine::general_purpose::STANDARD.encode(&final_bytes));
            }
            send(
                &mut socket,
                json!({"jsonrpc":"2.0","id":request["id"],"result":frame}),
            );
        }
    });
    let mut session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
    let final_frame = session.action_with_settling(&[10], 1, 60).unwrap();
    assert_eq!(final_frame.png_bytes(), expected);
    assert_ne!(final_frame.png_bytes(), primary_bytes);
}

#[test]
fn settling_failure_preserves_completed_primary_without_replaying_it() {
    for mode in ["rejection", "disconnect", "deadline", "wrong-id"] {
        let (notification, bytes) = initial();
        let mut blank = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut blank, 64, 32);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[0; 2048])
                .unwrap();
        }
        let mut initial_notification = notification.clone();
        initial_notification["params"]["data"] =
            json!(base64::engine::general_purpose::STANDARD.encode(&blank));
        let peer = Peer::start(move |mut socket| {
            send(&mut socket, initial_notification);
            let Message::Text(primary) = socket.read().unwrap() else {
                panic!("primary")
            };
            let primary: Value = serde_json::from_str(&primary).unwrap();
            assert_eq!(primary["params"], json!({"keys":[10],"frames":1}));
            send(
                &mut socket,
                json!({"jsonrpc":"2.0","id":primary["id"],"result":notification["params"]}),
            );
            let Message::Text(settling) = socket.read().unwrap() else {
                panic!("settling")
            };
            let settling: Value = serde_json::from_str(&settling).unwrap();
            assert_eq!(settling["params"], json!({"keys":[],"frames":60}));
            assert_ne!(primary["id"], settling["id"]);
            match mode {
                "rejection" => send(
                    &mut socket,
                    json!({"jsonrpc":"2.0","id":settling["id"],"error":{"code":-32000,"message":"settling fault"}}),
                ),
                "wrong-id" => send(
                    &mut socket,
                    json!({"jsonrpc":"2.0","id":primary["id"],"result":notification["params"]}),
                ),
                "deadline" => { /* wait for the caller to close its uncertain transport */ }
                _ => return,
            }
            assert!(
                !matches!(socket.read(), Ok(Message::Text(_))),
                "unexpected retry"
            );
        });
        let mut session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
        assert_ne!(session.observation().png_bytes(), bytes);
        let error = session.action_with_settling(&[10], 1, 60).unwrap_err();
        let Error::SettlingFailed {
            primary,
            primary_frames,
            settling_frames,
            outcome_unknown,
            source,
        } = error
        else {
            panic!("{mode}: {error}")
        };
        assert_eq!(primary.png_bytes(), bytes);
        assert_eq!((primary_frames, settling_frames), (1, 60));
        assert_eq!(outcome_unknown, mode != "rejection");
        match mode {
            "rejection" => assert!(
                matches!(*source, Error::Rpc { code: -32000, ref message } if message == "settling fault")
            ),
            "deadline" => assert!(
                matches!(*source, Error::UnknownOutcome { source } if matches!(*source, Error::Deadline { stage: Stage::Action }))
            ),
            "wrong-id" => assert!(
                matches!(*source, Error::UnknownOutcome { source } if matches!(*source, Error::InvalidProtocol { stage: Stage::Action, .. }))
            ),
            _ => assert!(
                matches!(*source, Error::UnknownOutcome { source } if matches!(*source, Error::Disconnected { stage: Stage::Action }))
            ),
        }
        assert_eq!(session.observation().png_bytes(), bytes);
        if mode != "rejection" {
            assert!(matches!(
                session.action(&[], 1),
                Err(Error::SessionInvalidated)
            ));
        }
        drop(session);
    }
}

#[test]
fn received_action_without_acknowledgement_is_unknown_and_never_replayed() {
    for delayed in [false, true] {
        let (notification, _) = initial();
        let (release_reply, await_release) = std::sync::mpsc::channel();
        let peer = Peer::start(move |mut socket| {
            send(&mut socket, notification.clone());
            assert!(matches!(socket.read().unwrap(), Message::Text(_)));
            if delayed {
                // Withhold acknowledgement until the caller's deadline expires,
                // then attempt a late reply. It must never revive the session.
                if await_release.recv_timeout(Duration::from_secs(2)).is_ok() {
                    let _ = socket.send(Message::Text(
                        json!({"jsonrpc":"2.0","id":1,
                        "result":notification["params"]})
                        .to_string()
                        .into(),
                    ));
                }
                // Any second text request is an unsafe retry, even if explicit.
                loop {
                    match socket.read() {
                        Ok(Message::Text(text)) => panic!("replayed uncertain action: {text}"),
                        Ok(Message::Ping(_) | Message::Pong(_)) => {}
                        _ => break,
                    }
                }
            }
        });
        let mut session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
        let before = session.observation().clone();
        let start = std::time::Instant::now();
        let error = session.action(&[1], 1).unwrap_err();
        assert!(
            matches!(error, Error::UnknownOutcome { ref source }
            if if delayed { matches!(**source, Error::Deadline { stage: Stage::Action }) }
            else { matches!(**source, Error::Disconnected { stage: Stage::Action }) }),
            "{error}"
        );
        assert!(start.elapsed() < Duration::from_millis(400));
        assert_eq!(session.observation(), &before);
        let error = session.action(&[1], 1).unwrap_err();
        assert!(matches!(error, Error::SessionInvalidated), "{error}");
        let _ = release_reply.send(());
    }
}

#[test]
fn matching_rpc_rejections_preserve_code_and_message_without_claiming_success() {
    let (notification, _) = initial();
    let peer = Peer::start(move |mut socket| {
        send(&mut socket, notification.clone());
        for rejection in [true, false] {
            let Message::Text(text) = socket.read().unwrap() else {
                panic!("action")
            };
            let request: Value = serde_json::from_str(&text).unwrap();
            if rejection {
                send(
                    &mut socket,
                    json!({"jsonrpc":"2.0","id":request["id"],
                    "error":{"code":-32000,"message":"execution fault after partial progress","data":{}}}),
                );
            } else {
                send(
                    &mut socket,
                    json!({"jsonrpc":"2.0","id":request["id"],"result":notification["params"]}),
                );
            }
        }
    });
    let mut session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
    let before = session.observation().clone();
    assert!(
        matches!(session.action(&[], 1), Err(Error::Rpc {code:-32000, message})
        if message == "execution fault after partial progress")
    );
    assert_eq!(session.observation(), &before);
    session.action(&[], 1).unwrap();
}

#[test]
fn action_responses_require_matching_valid_envelopes_and_validated_frames() {
    let (notification, _) = initial();
    let valid = json!({"jsonrpc":"2.0","id":1,"result":notification["params"]});
    let mut cases = vec![
        json!(null),
        json!([]),
        json!({}),
        json!({"jsonrpc":"2.0","id":1}),
        json!({"jsonrpc":"2.0","id":1,"error":{"code":"bad","message":"no"}}),
        json!({"jsonrpc":"2.0","id":1,"error":{"code":-1}}),
    ];
    for (path, value) in [
        (vec!["id"], json!(2)),
        (vec!["id"], json!("1")),
        (vec!["id"], json!(null)),
        (vec!["jsonrpc"], json!("1.0")),
        (
            vec!["error"],
            json!({"code":-1,"message":"both result and error"}),
        ),
        (vec!["method"], json!("chip8.frame")),
        (vec!["params"], json!({})),
        (vec!["result"], json!(null)),
        (vec!["result", "width"], json!(128)),
        (vec!["result", "format"], json!("jpeg")),
        (vec!["result", "data"], json!("%%%")),
        (vec!["result", "data"], json!("bm90IFBORw==")),
        (vec!["result", "data"], json!("A".repeat(24 * 1024))),
    ] {
        let mut bad = valid.clone();
        let mut field = &mut bad;
        for key in path {
            field = &mut field[key];
        }
        *field = value;
        cases.push(bad);
    }
    for response in cases {
        let notification = notification.clone();
        let peer = Peer::start(move |mut socket| {
            send(&mut socket, notification);
            assert!(matches!(socket.read().unwrap(), Message::Text(_)));
            send(&mut socket, response);
            // No later action may be sent on an uncertain session.
            assert!(!matches!(socket.read(), Ok(Message::Text(_))));
        });
        let mut session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
        let before = session.observation().clone();
        assert!(
            matches!(session.action(&[1], 1), Err(Error::UnknownOutcome {source})
            if matches!(*source, Error::InvalidProtocol {stage:Stage::Action,..}))
        );
        assert_eq!(session.observation(), &before);
        assert!(matches!(
            session.action(&[], 1),
            Err(Error::SessionInvalidated)
        ));
    }
}

#[test]
fn stale_notifications_and_control_traffic_are_not_action_results_or_new_deadlines() {
    let (notification, _) = initial();
    let peer = Peer::start(move |mut socket| {
        send(&mut socket, notification.clone());
        assert!(matches!(socket.read().unwrap(), Message::Text(_)));
        let until = std::time::Instant::now() + Duration::from_millis(350);
        while std::time::Instant::now() < until {
            if socket
                .send(Message::Text(notification.to_string().into()))
                .is_err()
            {
                break;
            }
            if socket.send(Message::Pong(vec![1].into())).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    let mut session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
    let start = std::time::Instant::now();
    assert!(
        matches!(session.action(&[], 1), Err(Error::UnknownOutcome {source})
        if matches!(*source, Error::Deadline {stage:Stage::Action}))
    );
    assert!(start.elapsed() >= Duration::from_millis(90));
    assert!(start.elapsed() < Duration::from_millis(300));
}

#[test]
fn control_frames_and_notifications_can_precede_only_a_correlated_success() {
    let (notification, _) = initial();
    let peer = Peer::start(move |mut socket| {
        send(&mut socket, notification.clone());
        let Message::Text(text) = socket.read().unwrap() else {
            panic!("action")
        };
        let request: Value = serde_json::from_str(&text).unwrap();
        socket.send(Message::Ping(vec![7].into())).unwrap();
        send(&mut socket, notification.clone());
        socket.send(Message::Pong(vec![8].into())).unwrap();
        send(
            &mut socket,
            json!({"jsonrpc":"2.0","id":request["id"],"result":notification["params"]}),
        );
        // Read the second action, skipping control replies. An old response ID
        // must invalidate the session, not return the first action's image.
        loop {
            if matches!(socket.read().unwrap(), Message::Text(_)) {
                break;
            }
        }
        send(
            &mut socket,
            json!({"jsonrpc":"2.0","id":request["id"],"result":notification["params"]}),
        );
    });
    let mut session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
    session.action(&[], 1).unwrap();
    let confirmed = session.observation().clone();
    let dir = tempfile::tempdir().unwrap();
    assert!(confirmed.save_png(dir.path()).is_err());
    assert_eq!(session.observation(), &confirmed);
    assert!(
        matches!(session.action(&[],1), Err(Error::UnknownOutcome { source })
        if matches!(*source, Error::InvalidProtocol { stage:Stage::Action,.. }))
    );
    assert!(matches!(
        session.action(&[], 1),
        Err(Error::SessionInvalidated)
    ));
}

#[test]
fn requested_execution_time_is_added_to_the_transport_allowance() {
    let (notification, _) = initial();
    let peer = Peer::start(move |mut socket| {
        send(&mut socket, notification.clone());
        assert!(matches!(socket.read().unwrap(), Message::Text(_)));
        // Longer than the configured 100ms allowance, but within the
        // 60-frame execution budget plus allowance. The real-process test
        // separately verifies actual 60-frame execution and server pacing.
        std::thread::sleep(Duration::from_millis(200));
        send(
            &mut socket,
            json!({"jsonrpc":"2.0","id":1,"result":notification["params"]}),
        );
    });
    let mut session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
    session.action(&[], 60).unwrap();
}

#[test]
fn duplicate_response_fields_are_malformed_even_when_the_last_value_matches() {
    for field in ["id", "jsonrpc", "result"] {
        let (notification, _) = initial();
        let peer = Peer::start(move |mut socket| {
            send(&mut socket, notification.clone());
            assert!(matches!(socket.read().unwrap(), Message::Text(_)));
            let result = notification["params"].to_string();
            let extra = match field {
                "id" => "\"id\":999,",
                "jsonrpc" => "\"jsonrpc\":\"1.0\",",
                _ => "\"result\":null,",
            };
            let text = format!("{{{extra}\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{result}}}");
            let _ = socket.send(Message::Text(text.into()));
        });
        let mut session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
        assert!(
            matches!(session.action(&[], 1), Err(Error::UnknownOutcome {source})
            if matches!(*source, Error::InvalidProtocol {stage:Stage::Action,..})),
            "duplicate {field}"
        );
    }
}

#[test]
fn malformed_or_interrupted_action_messages_are_unknown_not_success() {
    for mode in [
        "close",
        "truncated",
        "binary",
        "oversized",
        "malformed JSON",
    ] {
        let (notification, _) = initial();
        let peer = Peer::start(move |mut socket| {
            send(&mut socket, notification);
            assert!(matches!(socket.read().unwrap(), Message::Text(_)));
            match mode {
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
            }
        });
        let mut session = Session::connect(&peer.endpoint, short_timeouts()).unwrap();
        let before = session.observation().clone();
        let Err(Error::UnknownOutcome { source }) = session.action(&[], 1) else {
            panic!("{mode}")
        };
        if matches!(mode, "close" | "truncated") {
            assert!(
                matches!(
                    *source,
                    Error::Disconnected {
                        stage: Stage::Action
                    }
                ),
                "{mode}: {source}"
            );
        } else {
            assert!(
                matches!(
                    *source,
                    Error::InvalidProtocol {
                        stage: Stage::Action,
                        ..
                    }
                ),
                "{mode}: {source}"
            );
        }
        assert_eq!(session.observation(), &before);
        assert!(matches!(
            session.action(&[], 1),
            Err(Error::SessionInvalidated)
        ));
    }
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
        action_transport: Duration::from_millis(100),
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
