# CHIP-8 Opcode Reference

Opcodes are 16-bit values fetched in big-endian byte order. `X` and `Y` are 4-bit V-register indices, `NN` is an 8-bit immediate, `NNN` is a 12-bit address, and `N` is a 4-bit value. `VF` means `V[15]`.

This reference documents the profile implemented by this emulator. The classic CHIP-8 instruction set is represented below; `CXNN` uses the emulator's deterministic xorshift32 byte generator, initialized to a fixed nonzero seed and controllable through `State::seed_random`.

| Opcode | Meaning | Profile details |
| --- | --- | --- |
| `00E0` | Clear the display. | All pixels become off. |
| `00EE` | Return from a subroutine. | Returns an error on stack underflow. |
| `1NNN` | Jump to `NNN`. | Sets `PC = NNN`. |
| `2NNN` | Call subroutine at `NNN`. | Pushes the address after this instruction, then jumps. |
| `3XNN` | Skip next instruction if `VX == NN`. | Skips two bytes. |
| `4XNN` | Skip next instruction if `VX != NN`. | Skips two bytes. |
| `5XY0` | Skip next instruction if `VX == VY`. | Other low nibbles are invalid. |
| `6XNN` | Set `VX = NN`. | |
| `7XNN` | Set `VX = VX + NN`. | Addition wraps; `VF` is unchanged. |
| `8XY0` | Set `VX = VY`. | |
| `8XY1` | Set `VX = VX OR VY`. | `VF` is unchanged. |
| `8XY2` | Set `VX = VX AND VY`. | `VF` is unchanged. |
| `8XY3` | Set `VX = VX XOR VY`. | `VF` is unchanged. |
| `8XY4` | Set `VX = VX + VY`. | `VF = 1` on carry, otherwise `0`. |
| `8XY5` | Set `VX = VX - VY`. | `VF = 1` when no borrow, otherwise `0`. |
| `8XY6` | Shift `VX` right by one. | `VF` receives the least-significant bit; `Y` is ignored. |
| `8XY7` | Set `VX = VY - VX`. | `VF = 1` when no borrow, otherwise `0`. |
| `8XYE` | Shift `VX` left by one. | `VF` receives the most-significant bit; `Y` is ignored. |
| `9XY0` | Skip next instruction if `VX != VY`. | Other low nibbles are invalid. |
| `ANNN` | Set `I = NNN`. | |
| `BNNN` | Jump to `NNN + V0`. | Uses `V0` as the offset register. |
| `CXNN` | Set `VX = random_byte AND NN`. | Seedable deterministic xorshift32 generator; see `State::seed_random`. |
| `DXYN` | Draw an `N`-byte sprite from memory at `I`, positioned at `(VX, VY)`. | XOR drawing; coordinates wrap at screen edges; `VF` is 1 if any lit pixel is erased, otherwise 0. |
| `EX9E` | Skip next instruction if the key whose value is `VX` is pressed. | |
| `EXA1` | Skip next instruction if the key whose value is `VX` is not pressed. | |
| `FX07` | Set `VX` to the delay timer. | |
| `FX0A` | Wait for a pressed key and store its value in `VX`. | Re-executes while no key is pressed; if multiple keys are held, chooses the lowest-numbered key. |
| `FX15` | Set the delay timer to `VX`. | |
| `FX18` | Set the sound timer to `VX`. | |
| `FX1E` | Add `VX` to `I`. | Wraps as a 16-bit value; does not set `VF`. |
| `FX29` | Set `I` to the 5-byte font sprite for the hexadecimal digit in `VX`. | Uses the low nibble of `VX`; font begins at `0x50`. |
| `FX33` | Store the hundreds, tens, and ones digits of `VX` at `I`, `I+1`, and `I+2`. | |
| `FX55` | Store `V0` through `VX` in memory starting at `I`. | `I` is unchanged. |
| `FX65` | Load `V0` through `VX` from memory starting at `I`. | `I` is unchanged. |

`00NN` instructions other than `00E0` and `00EE` are not supported by this profile and return an invalid-opcode error. Super-CHIP and XO-CHIP extensions are not included.
