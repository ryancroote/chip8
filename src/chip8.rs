const FONT: [u8; 80] = [
    0xF0, 0x90, 0x90, 0x90, 0xF0, // 0
    0x20, 0x60, 0x20, 0x20, 0x70, // 1
    0xF0, 0x10, 0xF0, 0x80, 0xF0, // 2
    0xF0, 0x10, 0xF0, 0x10, 0xF0, // 3
    0x90, 0x90, 0xF0, 0x10, 0x10, // 4
    0xF0, 0x80, 0xF0, 0x10, 0xF0, // 5
    0xF0, 0x80, 0xF0, 0x90, 0xF0, // 6
    0xF0, 0x10, 0x20, 0x40, 0x40, // 7
    0xF0, 0x90, 0xF0, 0x90, 0xF0, // 8
    0xF0, 0x90, 0xF0, 0x10, 0xF0, // 9
    0xF0, 0x90, 0xF0, 0x90, 0x90, // A
    0xE0, 0x90, 0xE0, 0x90, 0xE0, // B
    0xF0, 0x80, 0x80, 0x80, 0xF0, // C
    0xE0, 0x90, 0x90, 0x90, 0xE0, // D
    0xF0, 0x80, 0xF0, 0x80, 0xF0, // E
    0xF0, 0x80, 0xF0, 0x80, 0x80, // F
];

const FONT_STARTING_LOCATION: usize = 80;
const PC_STARTING_LOCATION: u16 = 0x200;

pub type Opcode = u16;
pub type KeypadInput = [bool; 16];

#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub enum KeyInput {
    Zero = 0x0,
    One = 0x1,
    Two = 0x2,
    Three = 0x3,
    Four = 0x4,
    Five = 0x5,
    Six = 0x6,
    Seven = 0x7,
    Eight = 0x8,
    Nine = 0x9,
    A = 0xA,
    B = 0xB,
    C = 0xC,
    D = 0xD,
    E = 0xE,
    F = 0xF,
}

impl KeyInput {
    /// Index into the 16-entry keypad array (0x0..=0xF).
    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct State {
    pub pc: u16,
    pub display: [[bool; 64]; 32],
    pub i: u16,
    pub v: [u8; 16],
    pub delay: u8,
    pub sound: u8,
    pub memory: [u8; 4096],
    pub keypad_input: KeypadInput,
    pub stack: Vec<u16>,
    random_state: u32,
}

impl State {
    /// Complete, read-only machine snapshot for deterministic trace inspection.
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "pc": self.pc, "i": self.i, "v": self.v,
            "memory": self.memory.as_slice(),
            "display": self.display.iter().map(|row| row.as_slice()).collect::<Vec<_>>(),
            "stack": self.stack, "delay": self.delay, "sound": self.sound,
            "keypad_input": self.keypad_input, "random_state": self.random_state
        })
    }

    pub fn new() -> Self {
        let mut memory = [0; 4096];
        memory[FONT_STARTING_LOCATION..FONT_STARTING_LOCATION + FONT.len()].copy_from_slice(&FONT);

        Self {
            pc: PC_STARTING_LOCATION,
            display: [[false; 64]; 32],
            i: 0,
            v: [0; 16],
            delay: 0,
            sound: 0,
            memory,
            keypad_input: [false; 16],
            stack: Vec::new(),
            random_state: 0xA341_316C,
        }
    }

    /// Copy `bytes` into memory starting at address `at`.
    pub fn load_program(&mut self, bytes: &[u8], at: u16) {
        let start = at as usize;
        let end = start + bytes.len();
        assert!(end <= self.memory.len(), "program write out of bounds");
        self.memory[start..end].copy_from_slice(bytes);
    }

    /// Simulate a keypad transition for one key.
    pub fn set_key(&mut self, key: KeyInput, pressed: bool) {
        self.keypad_input[key.index()] = pressed;
    }

    /// Set the seed used by the deterministic random-byte generator.
    pub fn seed_random(&mut self, seed: u32) {
        self.random_state = if seed == 0 { 1 } else { seed };
    }

    fn random_byte(&mut self) -> u8 {
        // xorshift32; keep the zero state unreachable because it never changes.
        let mut value = self.random_state;
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        self.random_state = value;
        value as u8
    }

    /// Advance the delay and sound timers by one tick.
    pub fn tick_timers(&mut self) {
        self.delay = self.delay.saturating_sub(1);
        self.sound = self.sound.saturating_sub(1);
    }

    pub fn step(&mut self) -> Result<(), Chip8Error> {
        let opcode = self.fetch();
        instructions::bump_pointer(self);
        self.execute(opcode)
    }

    fn fetch(&self) -> Opcode {
        let high_bits = self.memory[self.pc as usize] as u16;
        let low_bits = self.memory[self.pc as usize + 1] as u16;
        (high_bits << 8) | low_bits
    }

    fn execute(&mut self, opcode: Opcode) -> Result<(), Chip8Error> {
        let first_nibble = (opcode >> 12) & 0xF;
        let second_nibble = (opcode >> 8) & 0xF;
        let third_nibble = (opcode >> 4) & 0xF;
        let fourth_nibble = opcode & 0xF;
        let nn = (opcode & 0x00FF) as u8;
        let nnn = opcode & 0x0FFF;

        match (first_nibble, second_nibble, third_nibble, fourth_nibble) {
            (0x0, 0x0, 0xE, 0x0) => instructions::clear_screen(self),
            (0x0, 0x0, 0xE, 0xE) => instructions::return_from_subroutine(self)?,
            (0x1, _, _, _) => instructions::jump(self, nnn),
            (0x2, _, _, _) => instructions::call_subroutine(self, nnn),
            (0x3, x, _, _) => instructions::skip_if_register_equals_n(self, x as usize, nn),
            (0x4, x, _, _) => instructions::skip_if_register_not_equals_n(self, x as usize, nn),
            (0x5, x, y, 0x0) => {
                instructions::skip_if_x_and_y_are_equal(self, x as usize, y as usize)
            }
            (0x6, x, _, _) => instructions::set_register(self, x as usize, nn),
            (0x7, x, _, _) => instructions::add_value_to_register(self, x as usize, nn),
            (0x8, x, y, 0x0) => {
                instructions::set_register_from_register(self, x as usize, y as usize)
            }
            (0x8, x, y, 0x1) => instructions::or_registers(self, x as usize, y as usize),
            (0x8, x, y, 0x2) => instructions::and_registers(self, x as usize, y as usize),
            (0x8, x, y, 0x3) => instructions::xor_registers(self, x as usize, y as usize),
            (0x8, x, y, 0x4) => instructions::add_register_with_carry(self, x as usize, y as usize),
            (0x8, x, y, 0x5) => instructions::subtract_registers(self, x as usize, y as usize),
            (0x8, x, _, 0x6) => instructions::shift_right(self, x as usize),
            (0x8, x, y, 0x7) => {
                instructions::subtract_registers_reverse(self, x as usize, y as usize)
            }
            (0x8, x, _, 0xE) => instructions::shift_left(self, x as usize),
            (0x9, x, y, 0x0) => {
                instructions::skip_if_x_and_y_are_not_equal(self, x as usize, y as usize)
            }
            (0xA, _, _, _) => instructions::set_index_register(self, nnn),
            (0xB, _, _, _) => instructions::jump_with_offset(self, nnn),
            (0xC, x, _, _) => instructions::random_mask(self, x as usize, nn),
            (0xD, x, y, n) => instructions::draw(self, x as usize, y as usize, n),
            (0xE, x, 0x9, 0xE) => instructions::skip_if_key_pressed(self, x as usize),
            (0xE, x, 0xA, 0x1) => instructions::skip_if_key_not_pressed(self, x as usize),
            (0xF, x, 0x0, 0x7) => instructions::read_delay_timer(self, x as usize),
            (0xF, x, 0x0, 0xA) => instructions::wait_for_key(self, x as usize),
            (0xF, x, 0x1, 0xE) => instructions::add_to_index(self, x as usize),
            (0xF, x, 0x2, 0x9) => instructions::set_index_to_font(self, x as usize),
            (0xF, x, 0x3, 0x3) => instructions::store_bcd(self, x as usize),
            (0xF, x, 0x1, 0x5) => instructions::set_delay_timer(self, x as usize),
            (0xF, x, 0x1, 0x8) => instructions::set_sound_timer(self, x as usize),
            (0xF, x, 0x5, 0x5) => instructions::store_registers(self, x as usize),
            (0xF, x, 0x6, 0x5) => instructions::set_registers(self, x as usize),
            _ => return Err(Chip8Error::InvalidOpcode(opcode)),
        }

        Ok(())
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum Chip8Error {
    InvalidOpcode(Opcode),
    StackUnderflow,
}

pub(crate) mod instructions;

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn state_can_initialize() {
        State::new();
    }

    #[test]
    fn state_can_be_cloned() {
        let state = State::new();
        assert_eq!(state, state.clone());
    }
}
