# Deterministic CHIP-8 Test Harness

The harness lives entirely under `tests/` and depends only on the emulator's
small public API. It never draws a window, reads the clock, plays audio, reads
the real keyboard, or runs an async loop.

## Running the tests

```sh
cargo test                     # fast suite (harness + implemented behavior)
cargo test -- --ignored        # instruction tests blocked on Chip8::step()
cargo test --no-fail-fast -- --ignored   # run every blocked binary
cargo test -- --test-threads=1 # fully ordered output
```

`cargo test` currently passes. The `#[ignore]` tests fail only because
`Chip8::step()` is still a no-op; remove each `#[ignore]` line as the
corresponding instruction is implemented.

## How a test is structured

Every test calls `common::run(TestCase)`:

```rust
mod common;

use common::new_case;

#[test]
fn add_with_carry_sets_vf() {
    let mut case = new_case("8XY4 sets VF=1 on carry");
    case.initial.registers.v[0] = 0xF0;   // set up only what matters
    case.initial.registers.v[1] = 0x20;
    case.program = vec![0x8014];          // opcodes, big-endian
    case.steps = 1;                       // execute exactly one instruction
    case.expected.registers = vec![(0, 0x10), (0xF, 0x01)];
    case.expected.pc = Some(0x202);
    common::run(case);
}
```

`TestCase` fields:

| Field | Meaning |
|---|---|
| `name` | Human name used in failure reports |
| `initial` | Starting `Snapshot` (default: `Chip8::new().snapshot_state()`, PC = `common::PROGRAM_START`) |
| `program` | `Vec<u16>` opcodes written big-endian at `load_at` |
| `load_at` | Address the program is written to (default `0x200`) |
| `steps` | Exact number of `Chip8::step()` calls |
| `timer_ticks` | Exact number of `Chip8::tick_timers()` calls after stepping |
| `expected` | Only the fields to verify; everything else is ignored |

`Expected` supports `registers`, `i`, `pc`, `stack`, `memory`,
`memory_ranges`, `pixels_on`, `pixels_off`, `delay`, `sound`, and `keys`.

Snapshot types are plain public-field DTOs, so seed memory (sprites, a
subroutine), pixels, registers, or timers by writing fields directly on
`case.initial`:

```rust
case.initial.memory.bytes[0x300] = 0xF0;            // sprite byte
case.initial.display.pixels[0][0] = true;           // lit pixel
case.initial.registers.i = 0x300;                   // index register
case.initial.registers.v[0] = 0xF0;                 // V0
case.initial.timers.delay = 30;
```

The program itself is written afterwards by the harness via `load_program`.

## Adding a new test

1. Pick the category file (`registers.rs`, `control_flow.rs`, `memory.rs`,
   `display.rs`, `timers.rs`, `input.rs`) or add a new `tests/*.rs` file that
   starts with `mod common;`.
2. Derive the expected value from documented CHIP-8 semantics or the project's
   pinned quirk (see `.kiro/specs/chip8-display/requirements.md`), never from
   running the emulator.
3. Set only the `expected` fields the test is about.
4. If the test needs `step()`, add:
   `#[ignore = "blocked: Chip8::step() is a no-op; implement fetch/decode/execute"]`
   and remove it when the instruction lands.

## Determinism rules

- Timers advance only via `case.timer_ticks` (no wall clock).
- Input is set via `case.initial.keypad_input[KeyInput::A.index()]` or
  `Chip8::set_key(KeyInput::A, pressed)`; `Expected.keys` takes `(KeyInput, bool)`
  pairs.
- Randomness is **not yet controllable**: when `CXNN` is implemented, the
  emulator needs a fixed-seed or injectable RNG hook before `RND` can be
  tested. No `RND` tests exist yet on purpose.
- Initial memory/registers/display come from `Chip8::new().snapshot_state()` or
  are fully specified by the test.

## Quirks not yet pinned

`BNNN` (offset register), `FX55`/`FX65` (I increment), shift ops, and logic-op
VF behavior vary between CHIP-8 variants. Tests touching these assert only the
parts the project has documented and are marked with a comment.

## Failure output

A failed comparison panics with the test name, loaded program, step count, the
instruction associated with the failure, expected-vs-actual lines, and a state
dump:

```
FAIL: Addition with carry
Program (loaded at 0x200):
  [0] 0x200: 0x8014
Instructions executed: 1
Instruction associated with failure: 0x8014 at 0x200 (step 1)

Expected vs actual:
  V0: expected 0x00, actual 0xFF
  V15: expected 0x01, actual 0x00

Machine state:
  PC=0x200  I=0x000  SP=0  delay=0  sound=0
  V= FF 01 00 ...
```
