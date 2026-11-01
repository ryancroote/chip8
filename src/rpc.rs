//! Local WebSocket interface for visual game controllers.
//!
//! Each connection receives the current display, then submits bounded keypad
//! actions. The emulator remains paused while the controller is thinking.

use crate::chip8::{KeyInput, State};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Cursor;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tungstenite::protocol::WebSocket;
use tungstenite::{Message, accept};

pub type SharedState = Arc<Mutex<State>>;

const WIDTH: usize = 64;
const HEIGHT: usize = 32;
const INSTRUCTIONS_PER_FRAME: usize = 11;
const FRAMES_PER_SECOND: u32 = 60;
const MAX_ACTION_FRAMES: u32 = 60;
const SOCKET_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    params: ActionParams,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionParams {
    keys: Vec<u8>,
    frames: u32,
}

pub struct RpcServer {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RpcServer {
    /// Start a loopback-only WebSocket JSON-RPC server.
    pub fn start(state: SharedState, address: SocketAddr) -> Result<Self, String> {
        if !address.ip().is_loopback() {
            return Err("RPC server must bind to a loopback address".into());
        }
        let listener = TcpListener::bind(address).map_err(|error| error.to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || serve(listener, state, thread_stop));
        Ok(Self {
            address,
            stop,
            thread: Some(thread),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }
}

impl Drop for RpcServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(listener: TcpListener, state: SharedState, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = serve_client(stream, &state, &stop) {
                    eprintln!("RPC client disconnected: {error}");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                eprintln!("RPC listener failed: {error}");
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn serve_client(stream: TcpStream, state: &SharedState, stop: &AtomicBool) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| error.to_string())?;
    let mut socket = accept(stream).map_err(|error| error.to_string())?;
    socket
        .get_ref()
        .set_read_timeout(Some(SOCKET_TIMEOUT))
        .map_err(|error| error.to_string())?;
    send_frame(&mut socket, state)?;

    while !stop.load(Ordering::Relaxed) {
        match socket.read() {
            Ok(Message::Text(text)) => {
                let result = handle_request(text.as_str(), state);
                match result {
                    Ok(response) => socket
                        .send(Message::Text(response.to_string().into()))
                        .map_err(|error| error.to_string())?,
                    Err((id, code, message)) => socket
                        .send(Message::Text(
                            json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
                                .to_string()
                                .into(),
                        ))
                        .map_err(|error| error.to_string())?,
                }
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(Message::Ping(data)) => socket
                .send(Message::Pong(data))
                .map_err(|error| error.to_string())?,
            Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
            Ok(Message::Binary(_)) => {
                socket
                    .send(Message::Text(
                        json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"text JSON-RPC messages are required"}})
                            .to_string()
                            .into(),
                    ))
                    .map_err(|error| error.to_string())?;
            }
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(());
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let _ = socket.close(None);
    Ok(())
}

fn handle_request(text: &str, state: &SharedState) -> Result<Value, (Value, i32, String)> {
    let request: RpcRequest = serde_json::from_str(text)
        .map_err(|error| (Value::Null, -32600, format!("invalid request: {error}")))?;
    if request.jsonrpc != "2.0" {
        return Err((request.id, -32600, "jsonrpc must be 2.0".into()));
    }
    if request.method != "chip8.action" {
        return Err((request.id, -32601, "method not found".into()));
    }
    if !(1..=MAX_ACTION_FRAMES).contains(&request.params.frames) {
        return Err((
            request.id,
            -32602,
            format!("frames must be in 1..={MAX_ACTION_FRAMES}"),
        ));
    }
    let mut keys = [false; 16];
    for key in request.params.keys {
        let Some(pressed) = keys.get_mut(usize::from(key)) else {
            return Err((request.id, -32602, "keys must be in 0..=15".into()));
        };
        if *pressed {
            return Err((request.id, -32602, "duplicate keys are not allowed".into()));
        }
        *pressed = true;
    }

    let result = advance(state, keys, request.params.frames);
    if let Err(error) = result {
        return Err((request.id, -32000, error));
    }
    match frame_data(state) {
        Ok(frame) => Ok(json!({"jsonrpc":"2.0","id":request.id,"result":frame})),
        Err(error) => Err((request.id, -32000, error)),
    }
}

fn advance(state: &SharedState, keys: [bool; 16], frames: u32) -> Result<(), String> {
    let frame_duration = Duration::from_nanos(1_000_000_000 / u64::from(FRAMES_PER_SECOND));
    for _ in 0..frames {
        let started = Instant::now();
        {
            let mut state = state
                .lock()
                .map_err(|_| "machine state lock poisoned".to_string())?;
            for (index, pressed) in keys.iter().copied().enumerate() {
                state.set_key(key_from_index(index), pressed);
            }
            for _ in 0..INSTRUCTIONS_PER_FRAME {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.step())) {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        release_keys(&mut state);
                        return Err(format!("{error:?}"));
                    }
                    Err(_) => {
                        release_keys(&mut state);
                        return Err("emulator fault during instruction execution".into());
                    }
                }
            }
            state.tick_timers();
        }
        thread::sleep(frame_duration.saturating_sub(started.elapsed()));
    }
    if let Ok(mut state) = state.lock() {
        release_keys(&mut state);
    }
    Ok(())
}

fn key_from_index(index: usize) -> KeyInput {
    const KEYS: [KeyInput; 16] = [
        KeyInput::Zero,
        KeyInput::One,
        KeyInput::Two,
        KeyInput::Three,
        KeyInput::Four,
        KeyInput::Five,
        KeyInput::Six,
        KeyInput::Seven,
        KeyInput::Eight,
        KeyInput::Nine,
        KeyInput::A,
        KeyInput::B,
        KeyInput::C,
        KeyInput::D,
        KeyInput::E,
        KeyInput::F,
    ];
    KEYS[index]
}

fn release_keys(state: &mut State) {
    for index in 0..16 {
        state.set_key(key_from_index(index), false);
    }
}

fn send_frame(socket: &mut WebSocket<TcpStream>, state: &SharedState) -> Result<(), String> {
    let frame = frame_data(state)?;
    socket
        .send(Message::Text(
            json!({"jsonrpc":"2.0","method":"chip8.frame","params":frame})
                .to_string()
                .into(),
        ))
        .map_err(|error| error.to_string())
}

fn frame_data(state: &SharedState) -> Result<Value, String> {
    let state = state
        .lock()
        .map_err(|_| "machine state lock poisoned".to_string())?;
    let mut samples = Vec::with_capacity(WIDTH * HEIGHT);
    for row in &state.display {
        for pixel in row {
            samples.push(if *pixel { 255 } else { 0 });
        }
    }
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut png), WIDTH as u32, HEIGHT as u32);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("failed to encode display frame: {error}"))?;
        writer
            .write_image_data(&samples)
            .map_err(|error| format!("failed to encode display frame: {error}"))?;
    }
    Ok(json!({
        "width": WIDTH,
        "height": HEIGHT,
        "format": "png",
        "data": base64::engine::general_purpose::STANDARD.encode(png)
    }))
}

#[cfg(test)]
mod tests {
    use super::{advance, handle_request};
    use crate::chip8::State;
    use std::sync::{Arc, Mutex};

    #[test]
    fn action_rejects_out_of_range_key_and_frame_count() {
        let state = Arc::new(Mutex::new(State::new()));
        let key_error = handle_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"chip8.action","params":{"keys":[16],"frames":1}}"#,
            &state,
        )
        .unwrap_err();
        assert_eq!(key_error.1, -32602);
        let frame_error = handle_request(
            r#"{"jsonrpc":"2.0","id":2,"method":"chip8.action","params":{"keys":[],"frames":61}}"#,
            &state,
        )
        .unwrap_err();
        assert_eq!(frame_error.1, -32602);
    }

    #[test]
    fn failed_action_releases_held_keys() {
        let mut machine = State::new();
        machine.pc = 4095;
        let state = Arc::new(Mutex::new(machine));
        let mut keys = [false; 16];
        keys[5] = true;

        assert!(advance(&state, keys, 1).is_err());
        assert!(
            state
                .lock()
                .unwrap()
                .keypad_input
                .iter()
                .all(|pressed| !pressed)
        );
    }
}
