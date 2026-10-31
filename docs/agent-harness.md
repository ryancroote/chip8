# Agent testing harness

`chip8-agent` is a headless process that uses the same emulator operations as
instruction tests. It opens no window and does not read the clock or a physical
keyboard. The graphical executable remains `chip8`.

## Live control

```sh
cargo build --bin chip8-agent
printf '%s\n' \
  '{"action":{"type":"load","at":512,"bytes":[96,5]}}' \
  '{"action":{"type":"step"}}' \
  '{"action":{"type":"inspect"}}' | target/debug/chip8-agent
```

Send **one JSON request per line** on stdin. Each response is a single JSON line
on stdout, flushed immediately. An agent can read a response, inspect its display
and machine state, and choose the next request while keeping stdin open. EOF ends
the trace successfully. One process is one trace; launch another process for an
independent trace. Diagnostic text goes to stderr.

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

Actions run sequentially. Each response describes the state **after** its action.
There are no implicit timer ticks, frames, delays, or combined input/step actions.
Broader scheduling/timing semantics remain outside this interface.

Success responses contain `index` (zero-based), `action`, and `snapshot`.
Machine snapshots include PC, I register, all V registers, all 4096 memory bytes,
the 32-by-64 boolean display, stack, both timers, all keypad held states, and RNG
state. Snapshot keys are `pc`, `i`, `v`, `memory`, `display`, `stack`, `delay`,
`sound`, `keypad_input`, and `random_state`. Every field is reproducible and is
compared at checkpoints; there is no field-exclusion mechanism.

An action failure or checkpoint mismatch emits an error response and exits
nonzero without executing later actions. Reports include the action index,
error, and `last_snapshot` (null before the first successful action). Checkpoint
mismatches also include the actual snapshot and field-level expected/actual
differences. A core fault can leave partial internal changes, so execution never
continues after one. Malformed JSON reports the raw request. File/CLI setup
failures are reported on stderr with a nonzero exit.

## Fixtures and explicit checkpoints

A version-1 fixture is a JSON object containing `version: 1` and `actions`, an
array of the same requests used in live mode. Set `checkpoint: true` on selected
requests. Replay requires their complete `expected` snapshot; expectations on
non-checkpoints are rejected. Each recorded request also has a `snapshot` for
trace inspection. Replay recomputes observations rather than trusting recorded
snapshots. Live requests may also include checkpoint expectations.

```sh
target/debug/chip8-agent replay approved.json > replay.jsonl
```

Replay emits every resulting snapshot, including non-checkpoint observations,
and stops at the first failure. Recorded observations are not assertions unless
an explicit checkpoint supplies an expected snapshot.

## Record, review, promote

The checked-in example is a **scenario for recording**, not an approved golden
fixture. It presses key 5, waits for input, sets both timers to 5, releases the
key, ticks the timers to 4, then generates a seeded random byte (33 in V1).

```sh
target/debug/chip8-agent record examples/agent-scenario.json candidate.json > recording.jsonl
target/debug/chip8-agent review candidate.json approved.json > review.json
# Read candidate.json and review.json; independently check the expected state.
target/debug/chip8-agent promote candidate.json approved.json --replace
target/debug/chip8-agent replay approved.json > replay.jsonl
```

Record mode captures every observation and fills expected values at explicit
checkpoints. Its output path **must not exist**: even existing candidates are
never overwritten. Failed execution removes the incomplete candidate. To record
an updated approved scenario, use it as the source and choose a new candidate
path. Record mode ignores old checkpoint values, not checkpoint selection.

Review reports field-level differences, or the full initial candidate if no
approved fixture exists. Promotion requires `--replace`, a complete recorded
candidate, and at least one checkpoint. It atomically replaces the approved
fixture; the candidate remains available. This explicit command represents the
human's approval—the tool cannot determine whether someone actually reviewed it.
Commit approved fixtures after review; source control retains their history.

There is **no independent correctness oracle** here. Recording alone proves
neither emulator nor harness correctness. Human-reviewed expectations provide
regression checks; independent instruction tests remain important. Tests of the
promotion command do not imply human approval of generated snapshots. Only the
repository's documented [CHIP-8 profile](chip8-opcodes.md) is supported; no
third-party reference emulator or autonomous gameplay strategy is included.

## Tests

Run `cargo test`. Subprocess tests cover the public JSON interface and fixture
workflow; existing direct-state tests cover instruction semantics.

If the local macOS linker rejects the selected SDK's `arm64e.x1` stubs, an
installed compatible SDK can be selected without changing the repository:

```sh
SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk cargo test
```
