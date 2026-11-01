# CHIP-8 Emulator

A CHIP-8 emulator with graphical play, headless control, and a visual game-controller RPC.

## Run a ROM

```sh
cargo run -- path/to/game.ch8
```

This opens the emulator window and uses the local keyboard. For an external
controller such as Jev, start RPC mode instead:

```sh
cargo run -- path/to/game.ch8 --rpc
```

The window remains visible while the game controller connects and plays. To run
without a window, add `--headless`; to select a port, add `--port PORT` (the
default is `8787`):

```sh
cargo run -- path/to/game.ch8 --rpc --headless --port 8790
```

RPC mode serves a loopback-only WebSocket JSON-RPC interface. It sends PNG
images of the CHIP-8 display and accepts bounded keypad actions. The emulator
does not call Jev or interpret natural-language goals: provide the ROM to the
emulator and give the game objective/directions to the separate controller.
See [`docs/agent-harness.md`](docs/agent-harness.md) for the RPC contract and
the retained snapshot/fixture commands.

## Tests

```sh
cargo test
```
