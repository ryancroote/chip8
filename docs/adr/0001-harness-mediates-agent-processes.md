# Exploratory runs mediate two child processes over JSON

_Status: superseded by [ADR-0002](0002-unified-emulator-and-visual-rpc.md)_

Exploratory runs exercise the emulator through the same line-delimited JSON
protocol that `chip8-agent` already exposes to fixture replay, rather than
linking the emulator and the agent into one process. A harness process owns the
run: it starts `chip8-agent`, loads the ROM itself, passes each resulting
machine snapshot to a separate agent process, and forwards the agent's chosen
action back to the emulator. Every observation therefore crosses the real
process boundary, and replacing the bundled random agent with an LLM-driven one
requires no change to the emulator or the harness.

`chip8-agent` was chosen as the boundary because it already contains the
containment this run needs: it catches core faults during instruction execution
and refuses to continue a trace with partially mutated state, which an
in-process loop would have to reimplement.

## Consequences

Each step costs a JSON round trip and every snapshot is materialized in full —
including all 4096 memory bytes — which bounds how many actions fit in a
reasonable run and inflates trace artifacts. This is accepted because
exploratory runs are bounded and artifact-rich by design; throughput was
traded for process-level fidelity and agent replaceability.

The agent boundary is deliberately narrow: an agent sees a snapshot and returns
one action, and owns no process lifecycle. Anyone adding actions to what an
agent may choose must widen the emulator protocol first, so the headless and
exploratory paths cannot silently diverge.
