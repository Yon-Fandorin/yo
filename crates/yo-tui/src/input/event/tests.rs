use super::{
    InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState, MediaKeyCode, ModifierKeyCode,
};
use crate::surface::Size;

// key event는 terminal byte가 아니라 code·modifier·action이 해석된 값이다.
#[test]
fn key_event_preserves_semantic_parts() {
    let event = InputEvent::Key(KeyEvent {
        code: KeyCode::Character('c'),
        modifiers: KeyModifiers::CONTROL.union(KeyModifiers::SHIFT),
        action: KeyAction::Press,
        state: KeyState::NONE,
    });

    let InputEvent::Key(key) = event else {
        panic!("a key event is expected");
    };
    assert_eq!(key.code, KeyCode::Character('c'));
    assert!(key.modifiers.contains(KeyModifiers::CONTROL));
    assert!(key.modifiers.contains(KeyModifiers::SHIFT));
    assert!(!key.modifiers.contains(KeyModifiers::ALT));
    assert_eq!(key.action, KeyAction::Press);
    assert_eq!(key.state, KeyState::NONE);
}

// paste 안의 줄바꿈과 control 문자는 별도 key command가 아닌 하나의 text payload다.
#[test]
fn paste_is_one_payload_instead_of_embedded_key_events() {
    let event = InputEvent::Paste("first\n\u{3}\u{4}second".to_owned());

    assert_eq!(
        event,
        InputEvent::Paste("first\n\u{3}\u{4}second".to_owned())
    );
}

// resize는 화면 계층이 다시 조회하지 않도록 해석이 끝난 width·height를 전달한다.
#[test]
fn resize_contains_resolved_surface_size() {
    let event = InputEvent::Resize(Size::new(120, 40));

    assert_eq!(event, InputEvent::Resize(Size::new(120, 40)));
}

// action과 navigation code는 press 외의 최신 terminal key 상태도 잃지 않는다.
#[test]
fn non_character_key_keeps_repeat_and_release_actions() {
    let repeated = KeyEvent {
        code: KeyCode::Left,
        modifiers: KeyModifiers::NONE,
        action: KeyAction::Repeat,
        state: KeyState::NONE,
    };
    let released = KeyEvent {
        action: KeyAction::Release,
        ..repeated
    };

    assert_eq!(repeated.action, KeyAction::Repeat);
    assert_eq!(released.action, KeyAction::Release);
}

// key vocabulary는 선택한 modern terminal decoder의 top-level code를 손실 없이 구분한다.
#[test]
fn key_vocabulary_keeps_distinct_decoded_codes() {
    let codes = [
        KeyCode::Enter,
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Backspace,
        KeyCode::Delete,
        KeyCode::Escape,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Insert,
        KeyCode::Function(12),
        KeyCode::Null,
        KeyCode::CapsLock,
        KeyCode::ScrollLock,
        KeyCode::NumLock,
        KeyCode::PrintScreen,
        KeyCode::Pause,
        KeyCode::Menu,
        KeyCode::KeypadBegin,
        KeyCode::Media(MediaKeyCode::Play),
        KeyCode::Modifier(ModifierKeyCode::LeftShift),
    ];

    assert_eq!(codes.len(), 26);
    assert!(codes.windows(2).all(|pair| pair[0] != pair[1]));
}

// media key 종류는 editor가 사용하지 않아도 terminal layer에서 버리지 않는다.
#[test]
fn media_key_vocabulary_is_lossless_for_the_selected_decoder() {
    let codes = [
        MediaKeyCode::Play,
        MediaKeyCode::Pause,
        MediaKeyCode::PlayPause,
        MediaKeyCode::Reverse,
        MediaKeyCode::Stop,
        MediaKeyCode::FastForward,
        MediaKeyCode::Rewind,
        MediaKeyCode::TrackNext,
        MediaKeyCode::TrackPrevious,
        MediaKeyCode::Record,
        MediaKeyCode::LowerVolume,
        MediaKeyCode::RaiseVolume,
        MediaKeyCode::MuteVolume,
    ];

    assert_eq!(codes.len(), 13);
    assert!(codes.windows(2).all(|pair| pair[0] != pair[1]));
}

// 좌우 modifier와 ISO shift 종류도 policy 판단 전에 원래 identity를 유지한다.
#[test]
fn modifier_key_vocabulary_is_lossless_for_the_selected_decoder() {
    let codes = [
        ModifierKeyCode::LeftShift,
        ModifierKeyCode::LeftControl,
        ModifierKeyCode::LeftAlt,
        ModifierKeyCode::LeftSuper,
        ModifierKeyCode::LeftHyper,
        ModifierKeyCode::LeftMeta,
        ModifierKeyCode::RightShift,
        ModifierKeyCode::RightControl,
        ModifierKeyCode::RightAlt,
        ModifierKeyCode::RightSuper,
        ModifierKeyCode::RightHyper,
        ModifierKeyCode::RightMeta,
        ModifierKeyCode::IsoLevel3Shift,
        ModifierKeyCode::IsoLevel5Shift,
    ];

    assert_eq!(codes.len(), 14);
    assert!(codes.windows(2).all(|pair| pair[0] != pair[1]));
}

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
