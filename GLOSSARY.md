# CHIP-8 Emulator

This project emulates the CHIP-8 virtual machine, executing programs encoded as two-byte instructions against registers, memory, timers, keypad input, and a monochrome display.

## Machine state

**V register**:
One of sixteen 8-bit general-purpose registers named `V0` through `VF`.
_Avoid_: data register

**VF**:
The sixteenth V register (`V[15]`), used by instructions for flags such as carry, borrow, and sprite collision. Instructions that write a result register and `VF` follow the implementation's documented write order when the result register itself is `VF`.
_Avoid_: flag register (when referring to an independent register)

**I register**:
The 16-bit index register, commonly used to hold a memory address.
_Avoid_: address register

**Program counter (PC)**:
The address of the next two-byte instruction to execute.
_Avoid_: instruction pointer

**Keypad key**:
One of the sixteen CHIP-8 keys, identified by a hexadecimal value from `0` through `F`.
_Avoid_: keyboard key

**Machine snapshot**:
A recorded representation of the complete emulator state at a point in a test trace, including architectural state and implementation state needed to reproduce execution.

**Test trace**:
An ordered record of control actions and the resulting machine snapshots, used to replay a scenario and compare selected checkpoints with reviewed expectations.

## External control

**Agent**:
A separate process that receives a machine snapshot and chooses the next action.
_Avoid_: bot, player

**Game controller**:
A separate process that receives images of the CHIP-8 display and chooses keypad inputs for the running game.
_Avoid_: Agent (when referring to visual game control)

## Instructions and profile

**Opcode**:
A 16-bit CHIP-8 instruction, conventionally written as four hexadecimal digits. Its nibbles encode an operation and operands such as register indices, immediate values, or memory addresses.
_Avoid_: instruction byte

**CHIP-8 profile**:
The specific set of variant-dependent CHIP-8 behaviors this emulator implements. This project uses Vx for shifts, V0 for `BNNN`, preserves `I` after `FX55`/`FX65`, leaves `VF` unchanged for `8XY1`–`8XY3`, wraps sprite drawing at display edges, and waits for `FX0A` by re-executing it until a key is pressed (choosing the lowest-numbered held key).
_Avoid_: dialect

**Sprite collision**:
The event during `DXYN` when a set sprite bit overlaps a lit display pixel; the pixel is erased and `VF` is set to 1.
_Avoid_: screen collision
