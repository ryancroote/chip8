use super::{Chip8Error, State};

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

pub(crate) fn add_register_with_carry(state: &mut State, x: usize, y: usize) {
    let (value, carried) = state.v[x].overflowing_add(state.v[y]);
    state.v[x] = value;
    state.v[0xF] = u8::from(carried);
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
