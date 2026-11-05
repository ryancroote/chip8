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
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(3),
            handshake: Duration::from_secs(3),
            observation: Duration::from_secs(3),
            disconnect: Duration::from_secs(3),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Connect,
    Handshake,
    InitialObservation,
    Disconnect,
}

/// Transport/initialization failures are separate from image-save I/O errors.
#[derive(Debug)]
pub enum Error {
    InvalidEndpoint(String),
    InvalidTimeout,
    Connection { stage: Stage, message: String },
    InvalidProtocol { stage: Stage, message: String },
    Disconnected { stage: Stage },
    Deadline { stage: Stage },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

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
        })
    }

    pub fn observation(&self) -> &Observation {
        &self.observation
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
    let frame = initial.params;
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
