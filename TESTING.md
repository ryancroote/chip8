# CHIP-8 Test Suite

The tests use `State::step()` directly, so they do not open a window, read the clock, play audio, or read a physical keyboard. Timer ticks and key input are controlled explicitly. Expected instruction behavior is documented in [`docs/chip8-opcodes.md`](docs/chip8-opcodes.md); the project's terminology is in [`GLOSSARY.md`](GLOSSARY.md).

## Control interfaces

The WebSocket visual-controller RPC and the snapshot-based JSON-lines interface,
including checkpoints and fixture review/promotion, are documented in
[`docs/agent-harness.md`](docs/agent-harness.md). `tests/rpc.rs`, `tests/cli.rs`,
and `tests/agent.rs` exercise these public process boundaries.

## Running tests

```sh
cargo test
cargo test -- --test-threads=1 # ordered output
```

All instruction tests are active; none are ignored as blocked on `step()`.

### Graphical RPC regression probe

On a logged-in desktop, run:

```sh
cargo test --test graphical_rpc -- --include-ignored --test-threads=1
```

The graphical probe opens a real native window, waits for its render loop to
start, then uses bounded WebSocket connections and actions to toggle a known
pixel repeatedly. Connecting immediately during window construction can hide
render-loop lock starvation. A headless control runs the same ROM and requests.
Child processes are reaped even on assertion failure. Normal `cargo test` skips
native-window checks, which cannot run reliably on headless CI.

This probe verifies native-window/RPC coexistence and returned pixels, **not**
actual window pixels, focus handling, or clicking the close button. Manual
verification on macOS is required for those properties:

1. Start a graphical RPC instance and connect a controller to its printed URL.
   Do not start a second headless instance. Use the probe's toggle ROM or another
   deterministic display-changing ROM.
2. After at least three display-changing actions, compare each returned image
   against the window within one second of the response (ignore borders and
   account for scaling). Test both focused and unfocused windows.
3. While the controller thinks, verify repainting stays responsive but the game
   does not advance. Physical keyboard input must not affect RPC gameplay.

If native screenshot capture is unavailable, record this as an unverified manual
check rather than treating RPC image assertions as a window-pixel comparison.
On this development machine the default macOS 27 SDK's text stubs are rejected
by the installed linker; tests can be run with
`SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk` as a local
command environment override (no project toolchain change is required).

## Test harness

The helpers in `tests/common/mod.rs` provide `new_case` and `run`. A case starts from `State::new()` with the program counter at `0x200`. Set initial public state, provide opcodes as `Vec<u16>`, set the number of steps, and assert only the state fields relevant to the behavior under test:

```rust
let mut case = new_case("6XNN loads an immediate value into VX");
case.program = vec![0x6005];
case.steps = 1;
case.expected.registers = vec![(0, 0x05)];
case.expected.pc = Some(0x202);
common::run(case);
```

`TestCase` supports initial state, program, load address, step count, timer tick count, and expected register, index, PC, stack, memory, display, timer, and keypad values. Programs are loaded big-endian. See `tests/common/mod.rs` for the assertion types and failure report.

For `CXNN`, seed the deterministic random-byte generator using `State::seed_random(u32)` before stepping. Seed zero is normalized to one so the generator cannot enter its absorbing zero state. A known seed and opcode make the result reproducible.

## Profile-dependent behavior

CHIP-8 variants disagree on some instruction details. This project pins those choices in `docs/chip8-opcodes.md`; tests should assert the documented profile rather than importing assumptions from a different variant. In particular, the profile specifies Vx-based shifts, `BNNN` using `V0`, unchanged `I` after `FX55`/`FX65`, unchanged `VF` for logic instructions, and wrapping sprite drawing.
