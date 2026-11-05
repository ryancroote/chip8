# Visual controller client

The library's `chip8::controller::Session` attaches to an **already running**
emulator through visual RPC (ADR-0002). It never discovers, launches, loads,
resets, or stops a game, and does not inspect machine snapshots or interpret ROMs.
The caller owns emulator startup/shutdown and display interpretation.

Start the emulator separately, graphically or headlessly:

```sh
cargo run -- path/to/game.ch8 --headless --rpc --port 0
# Prints: RPC listening at ws://127.0.0.1:... (use that exact endpoint)
cargo run --example observe -- ws://127.0.0.1:PRINTED_PORT display.png
```

The command above attaches, saves the initial display, and disconnects without
sending an action. To execute a caller-chosen action and save its result:

```sh
# Hold hexadecimal keypad A and F simultaneously for 6 frames:
cargo run --example observe -- ws://127.0.0.1:PRINTED_PORT initial.png result.png a,f 6
# Explicitly execute 60 frames with no keys:
cargo run --example observe -- ws://127.0.0.1:PRINTED_PORT initial.png result.png - 60
```

Connection, thinking time, reading `observation()`, and saving all advance
**zero emulated frames**. Disconnecting releases only the controller connection;
the running game stays paused and another controller may attach. Dropping a
session releases its TCP socket without a graceful WebSocket handshake.

```rust,no_run
use chip8::controller::{Session, Timeouts};
use std::time::Duration;
# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut session = Session::connect("ws://127.0.0.1:9000", Timeouts {
    observation: Duration::from_secs(2),
    ..Timeouts::default()
})?;
let frame = session.observation();
assert_eq!((frame.width(), frame.height(), frame.format()), (64, 32, "png"));
frame.save_png("initial.png")?;
let result = session.action(&[0xa, 0xf], 6)?;
result.save_png("result.png")?; // independent filesystem error, not action failure
session.disconnect()?;
# Ok(()) }
```

## Endpoint and deadlines

Only `ws://` endpoints with an explicit nonzero port and a loopback IP are
accepted (IPv4 or bracketed IPv6). `localhost` means exactly `127.0.0.1`, without
DNS lookup or fallback. Use the emulator's printed endpoint, including dynamic
ports. No connection attempts are retried, and an occupied server is never
replaced or stolen from another controller.

`Timeouts` has independent total budgets for TCP `connect`, WebSocket `handshake`,
initial `observation`, and graceful `disconnect`: **3 seconds each by default**.
All must be positive and representable as an `Instant` deadline. The total
connection budget is at most the sum of the first three stages. Partial messages
and ping/pong traffic do not restart these deadlines. Transport is synchronous,
with bounded nonblocking polling and no asynchronous runtime or background work.

`Error` distinguishes invalid endpoints/configuration, connection/handshake
failures, invalid protocol/frame data, disconnection, and deadline expiry;
network errors carry a `Stage`. Missing initial data expires the initial-observation
deadline. WebSocket close is not a successful observation. Failed connection
attempts release their socket. `disconnect` consumes the session even on failure.

## Image semantics and limits

Success requires a JSON-RPC 2.0 `chip8.frame` notification with no response/request
ID, containing base64 PNG data. Unknown harmless fields are ignored. The PNG must
fully decode as a nonanimated, 64-by-32, 8-bit grayscale image matching its
metadata. The original PNG bytes are authoritative, available via `png_bytes()`;
they are not enlarged or re-encoded. The image is confirmed at connection time,
not a continuously refreshed view. There is no hidden polling or execution.

WebSocket messages and individual frames are limited to 32 KiB, decoded PNG data
to 16 KiB, and PNG decoder allocations to 64 KiB. Pixel decoding uses a fixed
2048-byte buffer. The WebSocket dependency also bounds handshake headers.
`save_png(path)` writes exactly the confirmed bytes and returns `std::io::Result`,
independent of transport status. A write failure neither sends a request nor
changes the observation; writes are ordinary filesystem writes, not atomic saves.

## Routine verification

```sh
cargo test --test controller --test controller_process
cargo build --example observe
```

The process test generates its own tiny display-toggle ROM, uses a dynamic
headless RPC port, and reaps the emulator even on failure. Controlled peers
exercise invalid envelopes/images, size limits, incomplete data, control traffic,
and deadlines through the same public session interface. No downloaded ROM,
screen capture, or desktop automation is required. On macOS machines whose default
SDK is newer than the installed linker, prefix Cargo commands with
`SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk`.

## Keypad actions and timing

`Session::action(&[u8], frames: u32) -> Result<&Observation, Error>` executes one
synchronous request. The mutable session permits at most one in-flight action.
Keys must be unique numeric CHIP-8 keypad values `0..=15` (hexadecimal `0..=F`):
`0xa` is numeric 10, not a host keyboard label. Keys are held **simultaneously**;
`&[]` explicitly runs with no keys. Durations must be `1..=60` frames. Invalid
keys, duplicates, or durations return `Error::InvalidAction` before any request
is sent. Nothing is clamped, deduplicated, split, or converted into host input.

The server owns key release at the end of the action, 60-Hz pacing, and instruction
execution. There are no startup/menu inputs, synthetic release messages, polling
actions, automatic waits, or hidden extra frames. A completed action replaces
`observation()` with its validated resulting image. Equal image bytes can be a
valid result: freshness is established by the matching request ID, not by a pixel
change. Unsolicited notifications never confirm an action or replace its image.

`Timeouts::action_transport` defaults to **3 seconds** and is configurable at
connection time. Each action gets one total deadline of **frames / 60 seconds +
transport allowance**, beginning before transmission and bounding both writes and
the entire response wait. Partial messages, notifications, and ping/pong traffic
do not extend it. The allowance must be positive and large enough for transport,
image encoding, and server scheduling overhead; very small allowances can expire
even for a normally functioning emulator. All I/O stays synchronous and bounded.

## Failures and ambiguity

- `Error::InvalidAction` means nothing was transmitted or advanced by that call.
- `Error::Rpc { code, message }` preserves a matching JSON-RPC rejection. It is
  **not success**, nor a promise of unchanged machine state: an execution fault
  can occur after partial progress. The connection remains usable. Its previous
  confirmed image is historical, not evidence that a rejected action did nothing.
- Once transmission starts, a deadline, disconnection, malformed frame/envelope,
  or mismatched response ID becomes `Error::UnknownOutcome { source }`. The
  source distinguishes `Deadline`, `Disconnected`, and `InvalidProtocol` at
  `Stage::Action`. The action **may already have executed**. No action is ever
  automatically resent, reconnected, or replayed.
- An unknown outcome immediately releases the transport and invalidates the
  session. Later `action` calls return `Error::SessionInvalidated` locally.
  `observation()` still exposes the last confirmed image for diagnosis, **not a
  current observation of the uncertain game**. Explicitly `reconnect()` to recover,
  drop the session, or consume it with `disconnect`; graceful disconnect may fail
  on its closed transport.
- Request IDs are distinct within a connection. The theoretical counter-exhaustion
  case returns `Error::RequestIdsExhausted` locally instead of reusing an ID.
- `Observation::save_png` returns an independent `std::io::Result`. If saving a
  successful action's image fails, the move is still confirmed. Retry the file
  write if appropriate, never the move. A cloned observation can retain its image.

## Explicit recovery and controller handoff

`Session::reconnect(&mut self) -> Result<&Observation, Error>` releases the old
transport and attaches to the **original supplied endpoint**, using the original
`Timeouts`. It sends no action, never replays an uncertain input, and never resets
or loads a ROM. Success requires a fresh valid initial frame, replaces
`observation()`, and permits new actions even after `UnknownOutcome`. Pixels need
not differ: confirmation comes from the new connection's initial notification.

A failed attempt returns the connection/initial-observation error with its stage,
preserves the previous confirmed image in the session as **historical**, and
leaves actions locally refused with `SessionInvalidated`. There is no stale
success or transparent retry loop; the caller may explicitly attempt recovery
again. For example, after deciding to recover from an unknown outcome:

```rust,no_run
# use chip8::controller::{Session, Timeouts};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let mut session = Session::connect("ws://127.0.0.1:9000", Timeouts::default())?;
let historical = session.observation().clone();
let current = session.reconnect()?; // no old action is resent
current.save_png("recovered.png")?;
// Reinterpret the current display before choosing a NEW action.
# let _ = historical;
# session.disconnect()?;
# Ok(()) }
```

For graceful handoff, consume the first controller with bounded `disconnect()`;
another controller uses `Session::connect` at the same endpoint and observes the
paused game's current progress. Dropping a session also releases ownership,
without a close handshake. An occupied endpoint may accept TCP but its WebSocket
handshake waits for the active controller; the configured handshake deadline
bounds that wait without stealing ownership or disrupting the active controller.
Failed connection attempts release their sockets. Neither recovery nor handoff
starts or stops the caller-owned emulator.

**Endpoint identity limitation:** the existing protocol cannot prove that the
same emulator process still owns an endpoint. If another emulator later binds
that address, explicit reconnect observes that selected endpoint, not a verified
original instance. There is no discovery, fallback address, or invented identity
guarantee. ROM controls, planning, and success detection remain outside the client.
Settling is handled separately from explicit session recovery.
