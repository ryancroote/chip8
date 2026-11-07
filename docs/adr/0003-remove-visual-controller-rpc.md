# Remove the visual controller RPC ahead of a new game-control approach

_Status: accepted. Supersedes the RPC portion of [ADR-0002](0002-unified-emulator-and-visual-rpc.md)._

The loopback WebSocket JSON-RPC interface from ADR-0002, together with its `--rpc`, `--headless` and `--port` modes, is removed, along with the Rust controller client (`chip8::controller`) built on it. Game control is moving to a new approach, and the RPC's turn-based contract (PNG frames out, bounded keypad actions in, emulation paused between them) would otherwise shape that design before it is made. Removing it now leaves a clean base: the graphical loop owns the machine state directly instead of sharing it behind a lock for a server thread, the executable opens no network listener, and the networking and image dependencies go with it.

ADR-0002's other decisions still stand. The emulator remains one `chip8` executable, with `chip8 [ROM]` for graphical play and `chip8 agent [ARGS...]` for the deterministic snapshot JSON-lines and fixture workflow. The **Game controller** and **Continuous control** terms stay in the glossary because they describe game control itself, not the transport the RPC used.

## Consequences

Until the new approach lands, no external game controller can drive the emulator; graphical play is keyboard-only. Scripts that pass the removed flags fail with the standard `unknown option` error rather than a deprecation message. ADR-0002's reasoning for showing a controller display images instead of ROM-specific memory semantics is not overturned, and the new approach may reuse it.
