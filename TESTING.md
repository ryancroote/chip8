# CHIP-8 Test Suite

The tests use `State::step()` directly, so they do not open a window, read the clock, play audio, or read a physical keyboard. Timer ticks and key input are controlled explicitly. Expected instruction behavior is documented in [`docs/chip8-opcodes.md`](docs/chip8-opcodes.md); the project's terminology is in [`GLOSSARY.md`](GLOSSARY.md).

## Running tests

```sh
cargo test
cargo test -- --test-threads=1 # ordered output
```

All instruction tests are active; none are ignored as blocked on `step()`.

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
