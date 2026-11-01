use base64::Engine;
use chip8::chip8::State;
use chip8::rpc::RpcServer;
use serde_json::{Value, json};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;
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
