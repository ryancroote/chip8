pub struct State {
    pub pc: u16,
    pub display: [[bool; 64]; 32],
    pub i: u16,
    pub v: [u8; 16],
    pub delay: u8,
    pub sound: u8,
    pub memory: [u8; 4096],
    pub keypad_input: [bool; 16],
    pub stack: Vec<u16>,
}

pub(crate) mod instructions {
    fn clear_screen(state: &mut State) {}
    fn jump(state: &mut State) {}
    fn set_register(state: &mut State) {}
    fn set_index_register(state: &mut State) {}
}
