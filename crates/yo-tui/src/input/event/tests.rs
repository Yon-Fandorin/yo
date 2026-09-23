use super::{KeyModifiers, KeyState};

// enhanced keyboard modifier bit도 Shift·Control·Alt와 같은 방식으로 조합해 보존한다.
#[test]
fn enhanced_modifiers_are_not_dropped() {
    let modifiers = KeyModifiers::SUPER
        .union(KeyModifiers::HYPER)
        .union(KeyModifiers::META);

    assert!(modifiers.contains(KeyModifiers::SUPER));
    assert!(modifiers.contains(KeyModifiers::HYPER));
    assert!(modifiers.contains(KeyModifiers::META));
}

// keypad·Caps Lock·Num Lock 상태도 enhanced key event에서 분리해 보존한다.
#[test]
fn enhanced_key_state_is_not_dropped() {
    let state = KeyState::KEYPAD
        .union(KeyState::CAPS_LOCK)
        .union(KeyState::NUM_LOCK);

    assert!(state.contains(KeyState::KEYPAD));
    assert!(state.contains(KeyState::CAPS_LOCK));
    assert!(state.contains(KeyState::NUM_LOCK));
}
