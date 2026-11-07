# CHIP-8 Emulator

A CHIP-8 emulator with graphical play and a deterministic snapshot interface.

## Run a ROM

```sh
cargo run -- path/to/game.ch8
```

This opens the emulator window and uses the local keyboard. See
[`docs/agent-harness.md`](docs/agent-harness.md) for the snapshot/fixture
commands available under `chip8 agent`.

## Tests

```sh
cargo test
```
