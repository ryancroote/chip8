# Agent interface

The project builds one executable, `chip8`, with two ways to use it:

- `chip8 [ROM]` opens the graphical emulator. A ROM runs with local keyboard input; without a ROM, the window shows a blank display.
- `chip8 agent [ARGS...]` runs the snapshot-based JSON-lines interface and fixture workflow described below.

## Snapshot-based headless interface

The original deterministic JSON-lines interface remains available as a `chip8` subcommand. It opens no window and does not read the clock or physical keyboard:

```sh
cargo run -- agent
printf '%s\n' \
  '{"action":{"type":"load","at":512,"bytes":[96,5]}}' \
  '{"action":{"type":"step"}}' \
  '{"action":{"type":"inspect"}}' | target/debug/chip8 agent
```

Send one JSON request per line on stdin. Each response is one JSON line on stdout, flushed immediately. One process is one trace; EOF ends the trace successfully. Diagnostic text goes to stderr.

Actions (numeric fields use decimal JSON numbers):

| Action | Fields | Effect |
| --- | --- | --- |
| `load` | `at`: u16, `bytes`: byte array | Copy bytes into memory; does not change PC. Reset starts PC at 512. |
| `step` | none | Attempt one instruction. A waiting `FX0A` is still one attempt. |
| `inspect` | none | Observe without changing machine state. |
| `reset` | none | Fresh machine state, including the default RNG seed. Does not erase the trace. |
| `seed` | `seed`: u32 | Seed the RNG; zero is normalized to one. |
| `key` | `key`: 0–15, `pressed`: boolean | Set a keypad key's held state. |
| `tick` | none | Decrement delay and sound timers once, saturating at zero. |

Actions run sequentially. Each response describes the state after its action.
Machine snapshots include PC, I register, all V registers, all 4096 memory
bytes, the 32-by-64 boolean display, stack, both timers, all keypad held states,
and RNG state. Snapshot keys are `pc`, `i`, `v`, `memory`, `display`, `stack`,
`delay`, `sound`, `keypad_input`, and `random_state`. Every field is reproducible
and is compared at checkpoints; there is no field-exclusion mechanism.

An action failure or checkpoint mismatch emits an error response and exits
nonzero without executing later actions. Reports include the action index,
error, and `last_snapshot` (null before the first successful action).
Checkpoint mismatches also include the actual snapshot and field-level
expected/actual differences. A core fault can leave partial internal changes,
so execution never continues after one. Malformed JSON reports the raw request.
File/CLI setup failures are reported on stderr with a nonzero exit.

## Fixtures and explicit checkpoints

A version-1 fixture is a JSON object containing `version: 1` and `actions`, an
array of the same requests used in live mode. Set `checkpoint: true` on selected
requests. Replay requires their complete `expected` snapshot; expectations on
non-checkpoints are rejected. Each recorded request also has a `snapshot` for
trace inspection. Replay recomputes observations rather than trusting recorded
snapshots. Live requests may also include checkpoint expectations.

```sh
target/debug/chip8 agent replay approved.json > replay.jsonl
```

Replay emits every resulting snapshot, including non-checkpoint observations,
and stops at the first failure. Recorded observations are not assertions unless
an explicit checkpoint supplies an expected snapshot.

## Record, review, promote

The checked-in example is a scenario for recording, not an approved golden
fixture. It presses key 5, waits for input, sets both timers to 5, releases the
key, ticks the timers to 4, then generates a seeded random byte (33 in V1).

```sh
target/debug/chip8 agent record examples/agent-scenario.json candidate.json > recording.jsonl
target/debug/chip8 agent review candidate.json approved.json > review.json
target/debug/chip8 agent promote candidate.json approved.json --replace
target/debug/chip8 agent replay approved.json > replay.jsonl
```

Record mode captures every observation and fills expected values at explicit
checkpoints. Its output path must not exist: even existing candidates are never
overwritten. Failed execution removes the incomplete candidate. To record an
updated approved scenario, use it as the source and choose a new candidate
path. Record mode ignores old checkpoint values, not checkpoint selection.

Review reports field-level differences, or the full initial candidate if no
approved fixture exists. Promotion requires `--replace`, a complete recorded
candidate, and at least one checkpoint. It atomically replaces the approved
fixture; the candidate remains available. This explicit command represents the
human's approval—the tool cannot determine whether someone actually reviewed
it. Commit approved fixtures after review; source control retains their history.

There is no independent correctness oracle here. Recording alone proves neither
the emulator nor the harness correct. Human-reviewed expectations provide
regression checks; independent instruction tests remain important. Only the
repository's documented [CHIP-8 profile](chip8-opcodes.md) is supported.

## Tests

Run `cargo test`. Instruction tests exercise the emulator directly;
`tests/agent.rs` preserves the snapshot JSON-lines and fixture workflow through
the unified executable, and `tests/cli.rs` checks the command line. The visible
window itself still requires a manual check.
