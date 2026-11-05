use base64::Engine;
use chip8::chip8::State;
use chip8::rpc::RpcServer;
use serde_json::{Value, json};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket, connect};

fn frame(message: Message) -> Value {
    let Message::Text(text) = message else {
        panic!("expected a JSON text message, got {message:?}");
    };
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["jsonrpc"], "2.0");
    value
}

fn set_read_timeout(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>) {
    let MaybeTlsStream::Plain(stream) = socket.get_mut() else {
        unreachable!("test client uses plain WebSocket")
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
}

fn assert_png(frame: &Value) {
    assert_eq!(frame["width"], 64);
    assert_eq!(frame["height"], 32);
    assert_eq!(frame["format"], "png");
    let data = base64::engine::general_purpose::STANDARD
        .decode(frame["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(&data[..8], b"\x89PNG\r\n\x1a\n");
    let decoder = png::Decoder::new(std::io::Cursor::new(data));
    let reader = decoder.read_info().unwrap();
    assert_eq!(reader.info().width, 64);
    assert_eq!(reader.info().height, 32);
}

#[test]
fn controller_can_send_its_handshake_after_tcp_connection_is_accepted() {
    let state = Arc::new(Mutex::new(State::new()));
    let server = RpcServer::start(state, "127.0.0.1:0".parse().unwrap()).unwrap();
    let address = server.local_addr();
    let stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    // Let the server accept TCP before the WebSocket upgrade bytes arrive.
    std::thread::sleep(Duration::from_millis(200));
    let (mut socket, _) = tungstenite::client(format!("ws://{address}"), stream)
        .expect("a bounded handshake must tolerate delayed upgrade bytes");
    let initial = frame(socket.read().unwrap());
    assert_eq!(initial["method"], "chip8.frame");
    assert_png(&initial["params"]);
}

#[test]
fn rpc_shutdown_is_bounded_with_idle_active_and_incomplete_connections() {
    for mode in ["idle", "active", "incomplete handshake"] {
        let mut machine = State::new();
        machine.load_program(&[0x12, 0x00], 0x200);
        let state = Arc::new(Mutex::new(machine));
        let server = RpcServer::start(state, "127.0.0.1:0".parse().unwrap()).unwrap();
        let address = server.local_addr();
        let mut client = None;
        let mut incomplete = None;
        if mode == "incomplete handshake" {
            incomplete = Some(TcpStream::connect(address).unwrap());
        } else {
            let (mut socket, _) = connect(format!("ws://{address}")).unwrap();
            set_read_timeout(&mut socket);
            frame(socket.read().unwrap());
            if mode == "active" {
                socket
                    .send(Message::Text(
                        json!({
                            "jsonrpc":"2.0", "id":1, "method":"chip8.action",
                            "params":{"keys":[5],"frames":60}
                        })
                        .to_string()
                        .into(),
                    ))
                    .unwrap();
            }
            client = Some(socket);
        }
        std::thread::sleep(Duration::from_millis(100));
        let (tx, rx) = std::sync::mpsc::channel();
        let started = Instant::now();
        let shutdown = std::thread::spawn(move || {
            drop(server);
            tx.send(()).unwrap();
        });
        rx.recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| panic!("shutdown hung with {mode}"));
        assert!(started.elapsed() < Duration::from_secs(5));
        shutdown.join().unwrap();
        drop(client);
        drop(incomplete);
    }
}

#[test]
fn rpc_sends_an_initial_display_and_the_next_display_after_a_bounded_action() {
    let mut machine = State::new();
    machine.load_program(&[0x12, 0x00], 0x200);
    let state = Arc::new(Mutex::new(machine));
    let server = RpcServer::start(Arc::clone(&state), "127.0.0.1:0".parse().unwrap()).unwrap();
    let address = server.local_addr();
    let (mut client, _) = connect(format!("ws://{address}")).unwrap();
    set_read_timeout(&mut client);

    let initial = frame(client.read().unwrap());
    assert_eq!(initial["method"], "chip8.frame");
    assert_png(&initial["params"]);

    client
        .send(Message::Text(
            json!({
                "jsonrpc":"2.0",
                "id":1,
                "method":"chip8.action",
                "params":{"keys":[5,10],"frames":60}
            })
            .to_string()
            .into(),
        ))
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    {
        let state = state.lock().unwrap();
        assert!(state.keypad_input[5]);
        assert!(state.keypad_input[10]);
    }
    let response = frame(client.read().unwrap());
    assert_eq!(response["id"], 1);
    assert_png(&response["result"]);
    assert!(
        state
            .lock()
            .unwrap()
            .keypad_input
            .iter()
            .all(|pressed| !pressed)
    );

    client
        .send(Message::Text(
            json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"chip8.action",
                "params":{"keys":[],"frames":61}
            })
            .to_string()
            .into(),
        ))
        .unwrap();
    let rejected = frame(client.read().unwrap());
    assert_eq!(rejected["id"], 2);
    assert!(rejected["error"].is_object());

    client.close(None).unwrap();
    drop(client);
    std::thread::sleep(Duration::from_millis(50));

    let (mut reconnected, _) = connect(format!("ws://{address}")).unwrap();
    set_read_timeout(&mut reconnected);
    let resumed = frame(reconnected.read().unwrap());
    assert_eq!(resumed["method"], "chip8.frame");
    assert_png(&resumed["params"]);
}
