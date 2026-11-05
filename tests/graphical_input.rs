use chip8::chip8::{KeyInput, State};
use chip8::input::update_keypad_input;
use minifb::Key;

#[test]
fn graphical_keyboard_layout_maps_to_chip8_keys() {
    let mappings = [
        (Key::Key1, KeyInput::One),
        (Key::Key2, KeyInput::Two),
        (Key::Key3, KeyInput::Three),
        (Key::Key4, KeyInput::C),
        (Key::Q, KeyInput::Four),
        (Key::W, KeyInput::Five),
        (Key::E, KeyInput::Six),
        (Key::R, KeyInput::D),
        (Key::A, KeyInput::Seven),
        (Key::S, KeyInput::Eight),
        (Key::D, KeyInput::Nine),
        (Key::F, KeyInput::E),
        (Key::Z, KeyInput::A),
        (Key::X, KeyInput::Zero),
        (Key::C, KeyInput::B),
        (Key::V, KeyInput::F),
    ];

    for (host_key, chip8_key) in mappings {
        let mut state = State::new();
        update_keypad_input(&mut state, true, &[host_key]);
        assert!(
            state.keypad_input[chip8_key.index()],
            "{host_key:?} should press {chip8_key:?}"
        );
        assert_eq!(
            state
                .keypad_input
                .iter()
                .filter(|pressed| **pressed)
                .count(),
            1
        );
    }
}

#[test]
fn graphical_keyboard_tracks_held_and_simultaneous_keys_and_releases_them() {
    let mut state = State::new();

    update_keypad_input(&mut state, true, &[Key::Key1, Key::V]);
    assert!(state.keypad_input[KeyInput::One.index()]);
    assert!(state.keypad_input[KeyInput::F.index()]);

    update_keypad_input(&mut state, true, &[Key::V]);
    assert!(!state.keypad_input[KeyInput::One.index()]);
    assert!(state.keypad_input[KeyInput::F.index()]);

    update_keypad_input(&mut state, false, &[Key::V]);
    assert!(state.keypad_input.iter().all(|pressed| !pressed));
}
