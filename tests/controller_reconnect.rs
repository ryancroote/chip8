//! Explicit recovery through the public session interface and bounded wire peers.
#[path = "common/controller_peer.rs"]
mod controller_peer;

use base64::Engine;
use chip8::controller::{Error, Session, Stage, Timeouts};
use controller_peer::{Peer, accept};
use serde_json::{Value, json};
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

fn frame(pixel: u8) -> Value {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 64, 32);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut pixels = [0; 2048];
        pixels[0] = pixel;
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
    json!({"width":64,"height":32,"format":"png",
        "data":base64::engine::general_purpose::STANDARD.encode(bytes)})
}
fn send(socket: &mut WebSocket<TcpStream>, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .unwrap();
}
fn initial(socket: &mut WebSocket<TcpStream>, pixel: u8) {
    send(
        socket,
        json!({"jsonrpc":"2.0","method":"chip8.frame","params":frame(pixel)}),
    );
}
fn request(socket: &mut WebSocket<TcpStream>, keys: Value) -> Value {
    let Message::Text(text) = socket.read().unwrap() else {
        panic!("expected action")
    };
    let request: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(request["method"], "chip8.action");
    assert_eq!(request["params"], json!({"keys":keys,"frames":1}));
    request
}
fn timeouts() -> Timeouts {
    Timeouts {
        connect: Duration::from_millis(150),
        handshake: Duration::from_millis(150),
        observation: Duration::from_millis(150),
        action_transport: Duration::from_millis(100),
        disconnect: Duration::from_millis(150),
    }
}

#[test]
fn lost_or_delayed_ack_recovers_only_after_fresh_initial_without_replay() {
    for delayed in [false, true] {
        let (release, wait) = std::sync::mpsc::channel();
        let peer = Peer::start(move |listener| {
            let mut socket = accept(&listener);
            initial(&mut socket, 0);
            let old = request(&mut socket, json!([10]));
            if delayed {
                wait.recv_timeout(Duration::from_secs(2)).unwrap();
                let _ = socket.send(Message::Text(
                    json!({"jsonrpc":"2.0", "id":old["id"],
                    "result":frame(0)})
                    .to_string()
                    .into(),
                ));
                // No second request may arrive on the invalidated transport.
                assert!(!matches!(socket.read(), Ok(Message::Text(_))));
            }
            drop(socket);
            let mut socket = accept(&listener);
            // A failed recovery must not restore an uncertain session.
            send(
                &mut socket,
                json!({"jsonrpc":"2.0","method":"wrong","params":frame(255)}),
            );
            assert!(!matches!(socket.read(), Ok(Message::Text(_))));
            drop(socket);
            let mut socket = accept(&listener);
            initial(&mut socket, 255);
            // The only action on the new connection is the caller's NEW input.
            let new = request(&mut socket, json!([15]));
            send(
                &mut socket,
                json!({"jsonrpc":"2.0","id":new["id"],"result":frame(0)}),
            );
            assert!(matches!(socket.read().unwrap(), Message::Close(_)));
            let _ = socket.flush();
        });
        let mut session = Session::connect(&peer.endpoint, timeouts()).unwrap();
        let previous = session.observation().clone();
        assert!(matches!(
            session.action(&[10], 1),
            Err(Error::UnknownOutcome { .. })
        ));
        assert!(matches!(
            session.action(&[15], 1),
            Err(Error::SessionInvalidated)
        ));
        assert_eq!(session.observation(), &previous);
        let _ = release.send(());
        assert!(matches!(
            session.reconnect(),
            Err(Error::InvalidProtocol {
                stage: Stage::InitialObservation,
                ..
            })
        ));
        assert_eq!(session.observation(), &previous);
        assert!(matches!(
            session.action(&[15], 1),
            Err(Error::SessionInvalidated)
        ));
        let recovered = session.reconnect().unwrap().clone();
        assert_ne!(recovered, previous, "cached image presented as fresh");
        session.action(&[15], 1).unwrap();
        assert_eq!(session.observation(), &previous);
        session.disconnect().unwrap();
    }
}

#[test]
fn failed_reconnect_from_live_session_preserves_history_but_refuses_actions() {
    let peer = Peer::start(move |listener| {
        let mut socket = accept(&listener);
        initial(&mut socket, 255);
        // Reconnect releases the old transport without any action.
        assert!(!matches!(socket.read(), Ok(Message::Text(_))));
        drop(socket);
        let mut socket = accept(&listener);
        // A valid handshake is NOT a recovered session: initial data is required.
        assert!(!matches!(socket.read(), Ok(Message::Text(_))));
        drop(socket);
        let mut socket = accept(&listener);
        send(
            &mut socket,
            json!({"jsonrpc":"2.0","method":"chip8.frame","params":frame(0)["data"]}),
        );
        assert!(!matches!(socket.read(), Ok(Message::Text(_))));
        drop(socket);
        let mut socket = accept(&listener);
        initial(&mut socket, 0);
        let new = request(&mut socket, json!([]));
        send(
            &mut socket,
            json!({"jsonrpc":"2.0","id":new["id"],"result":frame(255)}),
        );
        assert!(matches!(socket.read().unwrap(), Message::Close(_)));
        let _ = socket.flush();
    });
    let mut session = Session::connect(&peer.endpoint, timeouts()).unwrap();
    let previous = session.observation().clone();
    let start = Instant::now();
    assert!(matches!(
        session.reconnect(),
        Err(Error::Deadline {
            stage: Stage::InitialObservation
        })
    ));
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(session.observation(), &previous);
    assert!(matches!(
        session.action(&[], 1),
        Err(Error::SessionInvalidated)
    ));
    assert!(matches!(
        session.reconnect(),
        Err(Error::InvalidProtocol {
            stage: Stage::InitialObservation,
            ..
        })
    ));
    assert_eq!(session.observation(), &previous);
    assert!(matches!(
        session.action(&[], 1),
        Err(Error::SessionInvalidated)
    ));
    assert_ne!(session.reconnect().unwrap(), &previous);
    session.action(&[], 1).unwrap();
    assert_eq!(session.observation(), &previous);
    session.disconnect().unwrap();
}

#[test]
fn unavailable_endpoint_reconnect_is_bounded_and_never_restores_actionability() {
    let peer = Peer::start(move |listener| {
        let mut socket = accept(&listener);
        drop(listener); // Same selected endpoint is now unavailable.
        initial(&mut socket, 255);
        assert!(!matches!(socket.read(), Ok(Message::Text(_))));
    });
    let mut session = Session::connect(&peer.endpoint, timeouts()).unwrap();
    let previous = session.observation().clone();
    for _ in 0..2 {
        let start = Instant::now();
        assert!(matches!(
            session.reconnect(),
            Err(Error::Connection {
                stage: Stage::Connect,
                ..
            })
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(session.observation(), &previous);
        assert!(matches!(
            session.action(&[], 1),
            Err(Error::SessionInvalidated)
        ));
    }
}
