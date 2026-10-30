use super::{Chip8Error, State};

const FONT_STARTING_LOCATION: u16 = 80;

pub(crate) fn clear_screen(state: &mut State) {
    state.display = [[false; 64]; 32];
}

pub(crate) fn jump(state: &mut State, to: u16) {
    state.pc = to;
}

pub(crate) fn set_register(state: &mut State, x: usize, value: u8) {
    state.v[x] = value;
}

pub(crate) fn add_value_to_register(state: &mut State, x: usize, value: u8) {
    state.v[x] = state.v[x].wrapping_add(value);
}

pub(crate) fn set_register_from_register(state: &mut State, x: usize, y: usize) {
    state.v[x] = state.v[y];
}

pub(crate) fn or_registers(state: &mut State, x: usize, y: usize) {
    state.v[x] |= state.v[y];
}

pub(crate) fn and_registers(state: &mut State, x: usize, y: usize) {
    state.v[x] &= state.v[y];
}

pub(crate) fn xor_registers(state: &mut State, x: usize, y: usize) {
    state.v[x] ^= state.v[y];
}

pub(crate) fn add_register_with_carry(state: &mut State, x: usize, y: usize) {
    let (value, carried) = state.v[x].overflowing_add(state.v[y]);
    state.v[x] = value;
    state.v[0xF] = u8::from(carried);
}

pub(crate) fn subtract_registers(state: &mut State, x: usize, y: usize) {
    let (value, borrowed) = state.v[x].overflowing_sub(state.v[y]);
    state.v[x] = value;
    state.v[0xF] = u8::from(!borrowed);
}

pub(crate) fn subtract_registers_reverse(state: &mut State, x: usize, y: usize) {
    let (value, borrowed) = state.v[y].overflowing_sub(state.v[x]);
    state.v[x] = value;
    state.v[0xF] = u8::from(!borrowed);
}

pub(crate) fn shift_right(state: &mut State, x: usize) {
    let value = state.v[x];
    state.v[x] = value >> 1;
    state.v[0xF] = value & 1;
}

pub(crate) fn shift_left(state: &mut State, x: usize) {
    let value = state.v[x];
    state.v[x] = value << 1;
    state.v[0xF] = value >> 7;
}

pub(crate) fn set_index_register(state: &mut State, value: u16) {
    state.i = value;
}

pub(crate) fn draw(state: &mut State, vx: usize, vy: usize, n: u16) {
    let x = (state.v[vx] & 63) as usize;
    let y = (state.v[vy] & 31) as usize;

    state.v[0xF] = 0;
    let base = state.i as usize;
    for row in 0..n as usize {
        let sprite = state.memory[base + row];
        for col in 0..8 {
            let row_index = (y + row) % 32;
            let col_index = (x + col) % 64;
            let old = state.display[row_index][col_index];
            let val = ((sprite >> (7 - col)) & 1) == 1;
            if old && val {
                state.v[0xF] = 1;
            }
            state.display[row_index][col_index] = val != old;
        }
    }
}

pub(crate) fn store_registers(state: &mut State, x: usize) {
    let base = state.i as usize;
    for idx in 0..=x {
        state.memory[base + idx] = state.v[idx];
    }
}

pub(crate) fn set_registers(state: &mut State, x: usize) {
    let base = state.i as usize;
    for idx in 0..=x {
        state.v[idx] = state.memory[base + idx];
    }
}

pub(crate) fn return_from_subroutine(state: &mut State) -> Result<(), Chip8Error> {
    let to = state.stack.pop().ok_or(Chip8Error::StackUnderflow)?;
    jump(state, to);
    Ok(())
}

pub(crate) fn call_subroutine(state: &mut State, value: u16) {
    state.stack.push(state.pc);
    jump(state, value);
}

pub(crate) fn bump_pointer(state: &mut State) {
    state.pc += 2;
}

pub(crate) fn skip_if_register_equals_n(state: &mut State, x: usize, n: u8) {
    if state.v[x] == n {
        bump_pointer(state);
    }
}

pub(crate) fn skip_if_register_not_equals_n(state: &mut State, x: usize, n: u8) {
    if state.v[x] != n {
        bump_pointer(state);
    }
}

pub(crate) fn skip_if_x_and_y_are_equal(state: &mut State, x: usize, y: usize) {
    if state.v[x] == state.v[y] {
        bump_pointer(state);
    }
}

pub(crate) fn skip_if_x_and_y_are_not_equal(state: &mut State, x: usize, y: usize) {
    if state.v[x] != state.v[y] {
        bump_pointer(state);
    }
}

pub(crate) fn jump_with_offset(state: &mut State, to: u16) {
    jump(state, to + state.v[0] as u16);
}

pub(crate) fn read_delay_timer(state: &mut State, x: usize) {
    state.v[x] = state.delay;
}

pub(crate) fn wait_for_key(state: &mut State, x: usize) {
    if let Some(key) = state.keypad_input.iter().position(|pressed| *pressed) {
        state.v[x] = key as u8;
    } else {
        state.pc = state.pc.wrapping_sub(2);
    }
}

pub(crate) fn add_to_index(state: &mut State, x: usize) {
    state.i = state.i.wrapping_add(state.v[x] as u16);
}

pub(crate) fn set_index_to_font(state: &mut State, x: usize) {
    state.i = FONT_STARTING_LOCATION + (state.v[x] as u16 & 0x0F) * 5;
}

pub(crate) fn store_bcd(state: &mut State, x: usize) {
    let value = state.v[x];
    let base = state.i as usize;
    state.memory[base] = value / 100;
    state.memory[base + 1] = (value / 10) % 10;
    state.memory[base + 2] = value % 10;
}

pub(crate) fn set_delay_timer(state: &mut State, x: usize) {
    state.delay = state.v[x];
}

pub(crate) fn set_sound_timer(state: &mut State, x: usize) {
    state.sound = state.v[x];
}

pub(crate) fn skip_if_key_pressed(state: &mut State, x: usize) {
    if state.keypad_input[state.v[x] as usize] {
        bump_pointer(state);
    }
}

pub(crate) fn skip_if_key_not_pressed(state: &mut State, x: usize) {
    if !state.keypad_input[state.v[x] as usize] {
        bump_pointer(state);
    }
}
