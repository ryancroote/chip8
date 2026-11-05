//! Synchronous, image-only access to an already running loopback emulator.
//!
//! Connecting observes without advancing time. The caller owns the emulator
//! process; dropping or disconnecting a session never stops or resets it.
use base64::Engine;
use serde::Deserialize;
use std::io::{self, Cursor, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::HandshakeError;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};

const MAX_MESSAGE: usize = 32 * 1024;
const MAX_PNG: usize = 16 * 1024;

/// Finite, total budgets per operation, not per received message.
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    pub connect: Duration,
    pub handshake: Duration,
    pub observation: Duration,
    pub disconnect: Duration,
    /// Added to requested frames / 60 seconds; bounds the entire action I/O.
    pub action_transport: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(3),
            handshake: Duration::from_secs(3),
            observation: Duration::from_secs(3),
            disconnect: Duration::from_secs(3),
            action_transport: Duration::from_secs(3),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Connect,
    Handshake,
    InitialObservation,
    Action,
    Disconnect,
}

/// Local, RPC, and transport failures, separate from image-save I/O errors.
/// An unknown action outcome wraps its structured transport/protocol cause.
#[derive(Debug)]
pub enum Error {
    InvalidEndpoint(String),
    InvalidTimeout,
    InvalidAction(String),
    RequestIdsExhausted,
    Connection {
        stage: Stage,
        message: String,
    },
    InvalidProtocol {
        stage: Stage,
        message: String,
    },
    Disconnected {
        stage: Stage,
    },
    Deadline {
        stage: Stage,
    },
    /// A matching server rejection; execution may have partially occurred.
    Rpc {
        code: i64,
        message: String,
    },
    /// Transmission began, but no valid acknowledgement was confirmed.
    /// The session's last observation is preserved and further actions refused.
    UnknownOutcome {
        source: Box<Error>,
    },
    SessionInvalidated,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnknownOutcome { source } => Some(source.as_ref()),
            _ => None,
        }
    }
}

/// A fully decoded and validated native 64-by-32, 8-bit grayscale PNG.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    png: Vec<u8>,
}

impl Observation {
    pub fn width(&self) -> u32 {
        64
    }
    pub fn height(&self) -> u32 {
        32
    }
    pub fn format(&self) -> &'static str {
        "png"
    }
    /// Original native bytes; inspection never sends a request.
    pub fn png_bytes(&self) -> &[u8] {
        &self.png
    }
    /// Explicit filesystem operation, independent of transport success.
    pub fn save_png(&self, path: impl AsRef<Path>) -> io::Result<()> {
        std::fs::write(path, &self.png)
    }
}

/// One synchronous controller connection. No background actions or retries.
pub struct Session {
    socket: WebSocket<DeadlineStream>,
    observation: Observation,
    timeouts: Timeouts,
    next_id: u64,
    invalidated: bool,
}

impl Session {
    /// Attach to an explicit `ws://` loopback address with an explicit port.
    /// Success includes the validated initial image; no action is sent.
    pub fn connect(endpoint: &str, timeouts: Timeouts) -> Result<Self, Error> {
        for duration in [
            timeouts.connect,
            timeouts.handshake,
            timeouts.observation,
            timeouts.disconnect,
            timeouts.action_transport,
            timeouts
                .action_transport
                .checked_add(Duration::from_secs(1))
                .ok_or(Error::InvalidTimeout)?,
        ] {
            if duration.is_zero() || Instant::now().checked_add(duration).is_none() {
                return Err(Error::InvalidTimeout);
            }
        }
        let (request, address) = endpoint_address(endpoint)?;
        let stream = TcpStream::connect_timeout(&address, timeouts.connect)
            .map_err(|e| transport_error(Stage::Connect, e.into()))?;
        stream
            .set_nonblocking(true)
            .map_err(|e| transport_error(Stage::Connect, e.into()))?;
        let stream = DeadlineStream {
            stream,
            deadline: Instant::now() + timeouts.handshake,
        };
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE))
            .max_frame_size(Some(MAX_MESSAGE));
        let mut handshake = tungstenite::client::client_with_config(request, stream, Some(config));
        let mut socket = loop {
            match handshake {
                Ok((socket, _)) => break socket,
                Err(HandshakeError::Interrupted(mid)) => {
                    pause();
                    handshake = mid.handshake();
                }
                Err(HandshakeError::Failure(e)) => {
                    return Err(transport_error(Stage::Handshake, e));
                }
            }
        };
        socket.get_mut().deadline = Instant::now() + timeouts.observation;
        let observation = loop {
            match read_message(&mut socket, Stage::InitialObservation)? {
                Message::Text(text) => break parse_initial(text.as_str())?,
                Message::Ping(_) | Message::Pong(_) => {}
                Message::Close(_) => {
                    return Err(Error::Disconnected {
                        stage: Stage::InitialObservation,
                    });
                }
                _ => return Err(invalid("expected a text JSON-RPC notification")),
            }
        };
        Ok(Self {
            socket,
            observation,
            timeouts,
            next_id: 1,
            invalidated: false,
        })
    }

    /// Last confirmed image; after a rejection or unknown outcome this may no
    /// longer describe the current game. Reading it never advances emulated time.
    pub fn observation(&self) -> &Observation {
        &self.observation
    }

    /// Execute one simultaneous keypad action and confirm its resulting image.
    /// Keys must be unique numeric values 0..=15; empty is valid. Frames must be
    /// 1..=60. Invalid inputs fail locally, without transmission or advancement.
    /// The total I/O deadline is frames / 60 seconds plus `action_transport`.
    /// No request is retried, and no extra frames are executed. An uncertain
    /// failure returns `UnknownOutcome`, preserving the last confirmed image
    /// and invalidating this session for all subsequent actions. A matching
    /// `Rpc` rejection is not success or a guarantee of unchanged machine state.
    pub fn action(&mut self, keys: &[u8], frames: u32) -> Result<&Observation, Error> {
        if self.invalidated {
            return Err(Error::SessionInvalidated);
        }
        if !(1..=60).contains(&frames) {
            return Err(Error::InvalidAction("frames must be in 1..=60".into()));
        }
        let mut seen = [false; 16];
        for &key in keys {
            let Some(pressed) = seen.get_mut(usize::from(key)) else {
                return Err(Error::InvalidAction("keys must be in 0..=15".into()));
            };
            if *pressed {
                return Err(Error::InvalidAction(
                    "duplicate keys are not allowed".into(),
                ));
            }
            *pressed = true;
        }
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(Error::RequestIdsExhausted)?;
        self.socket.get_mut().deadline = Instant::now()
            + self.timeouts.action_transport
            + Duration::from_secs_f64(f64::from(frames) / 60.0);
        match self.execute_action(keys, frames, id) {
            Ok(observation) => {
                self.observation = observation;
                Ok(&self.observation)
            }
            Err(error @ Error::Rpc { .. }) => Err(error),
            Err(source) => {
                self.invalidated = true;
                // Release ownership promptly; retaining the confirmed image is
                // useful, but this transport must never send another action.
                let _ = self
                    .socket
                    .get_ref()
                    .stream
                    .shutdown(std::net::Shutdown::Both);
                Err(Error::UnknownOutcome {
                    source: Box::new(source),
                })
            }
        }
    }

    fn execute_action(&mut self, keys: &[u8], frames: u32, id: u64) -> Result<Observation, Error> {
        let request = serde_json::json!({"jsonrpc":"2.0", "id":id,
            "method":"chip8.action", "params":{"keys":keys,"frames":frames}});
        match self.socket.send(Message::Text(request.to_string().into())) {
            Ok(()) => {}
            // send() has queued this message. Only flush the same buffered
            // bytes on backpressure; invoking send() again could duplicate it.
            Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => loop {
                match self.socket.flush() {
                    Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {
                        pause()
                    }
                    result => break result.map_err(|e| transport_error(Stage::Action, e))?,
                }
            },
            Err(e) => return Err(transport_error(Stage::Action, e)),
        }
        let observation = loop {
            match read_message(&mut self.socket, Stage::Action)? {
                Message::Text(text) => {
                    if let Some(observation) = parse_response(text.as_str(), id)? {
                        break observation;
                    }
                }
                Message::Ping(_) | Message::Pong(_) => {}
                Message::Close(_) => {
                    return Err(Error::Disconnected {
                        stage: Stage::Action,
                    });
                }
                _ => return Err(action_invalid("expected JSON-RPC text response")),
            }
        };
        Ok(observation)
    }

    /// Complete a bounded close handshake and release only this connection.
    /// Even on failure, consuming the session releases its socket.
    pub fn disconnect(mut self) -> Result<(), Error> {
        self.socket.get_mut().deadline = Instant::now() + self.timeouts.disconnect;
        match self.socket.close(None) {
            Ok(()) => {}
            Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(transport_error(Stage::Disconnect, e)),
        }
        loop {
            match read_message(&mut self.socket, Stage::Disconnect) {
                Ok(Message::Close(_)) => return Ok(()),
                Ok(_) => {}
                Err(e) => return Err(e),
            }
        }
    }
}

fn endpoint_address(
    endpoint: &str,
) -> Result<(tungstenite::handshake::client::Request, SocketAddr), Error> {
    let bad = || {
        Error::InvalidEndpoint(
            "expected ws:// loopback IP (or localhost) with explicit nonzero port".into(),
        )
    };
    let request = endpoint.into_client_request().map_err(|_| bad())?;
    let uri = request.uri();
    if uri.scheme_str() != Some("ws") || uri.authority().is_none_or(|a| a.as_str().contains('@')) {
        return Err(bad());
    }
    let host = uri.host().ok_or_else(bad)?;
    let ip: IpAddr = if host == "localhost" {
        // Deliberately no DNS, endpoint discovery, or alternate-address fallback.
        "127.0.0.1".parse().unwrap()
    } else {
        host.trim_start_matches('[')
            .trim_end_matches(']')
            .parse()
            .map_err(|_| bad())?
    };
    let port = uri.port_u16().filter(|p| *p != 0).ok_or_else(bad)?;
    if !ip.is_loopback() {
        return Err(bad());
    }
    Ok((request, SocketAddr::new(ip, port)))
}

#[derive(Deserialize)]
struct Initial {
    jsonrpc: String,
    method: String,
    params: Frame,
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}
#[derive(Deserialize)]
struct Frame {
    width: u32,
    height: u32,
    format: String,
    data: String,
}

fn invalid(message: impl ToString) -> Error {
    Error::InvalidProtocol {
        stage: Stage::InitialObservation,
        message: message.to_string(),
    }
}

fn parse_initial(text: &str) -> Result<Observation, Error> {
    let initial: Initial = serde_json::from_str(text).map_err(invalid)?;
    if initial.jsonrpc != "2.0"
        || initial.method != "chip8.frame"
        || ["id", "result", "error"]
            .iter()
            .any(|key| initial.other.contains_key(*key))
    {
        return Err(invalid("invalid initial notification envelope"));
    }
    validate_frame(initial.params)
}

fn validate_frame(frame: Frame) -> Result<Observation, Error> {
    if frame.width != 64 || frame.height != 32 || frame.format != "png" {
        return Err(invalid("only 64-by-32 PNG frames are supported"));
    }
    if frame.data.len() > MAX_PNG.div_ceil(3) * 4 {
        return Err(invalid("PNG payload too large"));
    }
    let png = base64::engine::general_purpose::STANDARD
        .decode(frame.data)
        .map_err(invalid)?;
    if png.len() > MAX_PNG {
        return Err(invalid("PNG payload too large"));
    }
    let decoder =
        png::Decoder::new_with_limits(Cursor::new(&png), png::Limits { bytes: 64 * 1024 });
    let mut reader = decoder.read_info().map_err(invalid)?;
    let info = reader.info();
    if info.width != 64
        || info.height != 32
        || info.color_type != png::ColorType::Grayscale
        || info.bit_depth != png::BitDepth::Eight
        || info.animation_control.is_some()
    {
        return Err(invalid(
            "PNG contents do not match supported frame metadata",
        ));
    }
    let mut pixels = [0; 2048];
    reader.next_frame(&mut pixels).map_err(invalid)?;
    reader.finish().map_err(invalid)?;
    drop(reader);
    Ok(Observation { png })
}

fn action_invalid(message: impl ToString) -> Error {
    Error::InvalidProtocol {
        stage: Stage::Action,
        message: message.to_string(),
    }
}

// Explicit null is not absence. Typed fields also reject duplicate envelope,
// rejection, and frame fields rather than silently accepting their last value.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
struct ActionEnvelope {
    jsonrpc: String,
    #[serde(default, deserialize_with = "present")]
    id: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present")]
    method: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present")]
    params: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present")]
    result: Option<Frame>,
    #[serde(default, deserialize_with = "present")]
    error: Option<Rejection>,
}

#[derive(Deserialize)]
struct Rejection {
    code: i64,
    message: String,
}

fn parse_response(text: &str, id: u64) -> Result<Option<Observation>, Error> {
    let response: ActionEnvelope = serde_json::from_str(text).map_err(action_invalid)?;
    // Well-formed unsolicited notifications may be ignored, but can never
    // confirm an action or replace its last confirmed observation.
    if response.jsonrpc == "2.0"
        && response.id.is_none()
        && response.method.as_ref().is_some_and(|m| m.is_string())
        && response.result.is_none()
        && response.error.is_none()
        && response
            .params
            .as_ref()
            .is_none_or(|p| p.is_object() || p.is_array())
    {
        return Ok(None);
    }
    if response.jsonrpc != "2.0"
        || response.id.as_ref().and_then(|id| id.as_u64()) != Some(id)
        || response.method.is_some()
        || response.params.is_some()
        || response.result.is_some() == response.error.is_some()
    {
        return Err(action_invalid("invalid or mismatched response envelope"));
    }
    if let Some(error) = response.error {
        return Err(Error::Rpc {
            code: error.code,
            message: error.message,
        });
    }
    let frame = response
        .result
        .ok_or_else(|| action_invalid("missing action result"))?;
    validate_frame(frame)
        .map(Some)
        .map_err(|error| match error {
            Error::InvalidProtocol { message, .. } => action_invalid(message),
            other => other,
        })
}

// Nonblocking I/O plus a deadline checked on *every* read/write bounds partial
// messages and control floods too, including loops inside tungstenite itself.
#[derive(Debug)]
struct DeadlineStream {
    stream: TcpStream,
    deadline: Instant,
}
impl DeadlineStream {
    fn check(&self) -> io::Result<()> {
        if Instant::now() >= self.deadline {
            Err(io::ErrorKind::TimedOut.into())
        } else {
            Ok(())
        }
    }
}
impl Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        self.stream.read(buf)
    }
}
impl Write for DeadlineStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.check()?;
        self.stream.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.check()?;
        self.stream.flush()
    }
}
fn pause() {
    std::thread::sleep(Duration::from_millis(1));
}
fn read_message(socket: &mut WebSocket<DeadlineStream>, stage: Stage) -> Result<Message, Error> {
    loop {
        socket
            .get_ref()
            .check()
            .map_err(|e| transport_error(stage, e.into()))?;
        match socket.read() {
            Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => pause(),
            result => return result.map_err(|e| transport_error(stage, e)),
        }
    }
}
fn transport_error(stage: Stage, error: tungstenite::Error) -> Error {
    use tungstenite::Error as Ws;
    match error {
        Ws::Io(ref e) if e.kind() == io::ErrorKind::TimedOut => Error::Deadline { stage },
        Ws::ConnectionClosed | Ws::AlreadyClosed => Error::Disconnected { stage },
        Ws::Io(ref e)
            if matches!(
                e.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::BrokenPipe
            ) =>
        {
            Error::Disconnected { stage }
        }
        Ws::Protocol(tungstenite::error::ProtocolError::ResetWithoutClosingHandshake) => {
            Error::Disconnected { stage }
        }
        e if stage == Stage::Connect || stage == Stage::Handshake => Error::Connection {
            stage,
            message: e.to_string(),
        },
        e => Error::InvalidProtocol {
            stage,
            message: e.to_string(),
        },
    }
}
