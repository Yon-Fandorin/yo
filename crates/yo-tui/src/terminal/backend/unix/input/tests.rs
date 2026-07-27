use std::collections::VecDeque;

use crossterm::event::{
    Event, KeyCode as CrosstermKeyCode, KeyEvent as CrosstermKeyEvent, KeyEventKind, KeyEventState,
    KeyModifiers as CrosstermKeyModifiers, MediaKeyCode as CrosstermMediaKeyCode,
    ModifierKeyCode as CrosstermModifierKeyCode, MouseButton, MouseEvent, MouseEventKind,
};

use super::{
    CrosstermEventSource, EventSource, EventSourceAcquireFailure, InputDecodeFailure,
    InputReadFailure, InputReader, UnsupportedInputKind, decode_event,
};
use crate::{
    input::event::{
        InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState, MediaKeyCode,
        ModifierKeyCode,
    },
    surface::Size,
};

// top-level key code를 각각 대응하는 yo identity로 옮긴다.
#[test]
fn every_top_level_key_code_maps_one_to_one() {
    let cases = [
        (CrosstermKeyCode::Backspace, KeyCode::Backspace),
        (CrosstermKeyCode::Enter, KeyCode::Enter),
        (CrosstermKeyCode::Left, KeyCode::Left),
        (CrosstermKeyCode::Right, KeyCode::Right),
        (CrosstermKeyCode::Up, KeyCode::Up),
        (CrosstermKeyCode::Down, KeyCode::Down),
        (CrosstermKeyCode::Home, KeyCode::Home),
        (CrosstermKeyCode::End, KeyCode::End),
        (CrosstermKeyCode::PageUp, KeyCode::PageUp),
        (CrosstermKeyCode::PageDown, KeyCode::PageDown),
        (CrosstermKeyCode::Tab, KeyCode::Tab),
        (CrosstermKeyCode::BackTab, KeyCode::BackTab),
        (CrosstermKeyCode::Delete, KeyCode::Delete),
        (CrosstermKeyCode::Insert, KeyCode::Insert),
        (CrosstermKeyCode::F(12), KeyCode::Function(12)),
        (CrosstermKeyCode::Char('한'), KeyCode::Character('한')),
        (CrosstermKeyCode::Null, KeyCode::Null),
        (CrosstermKeyCode::Esc, KeyCode::Escape),
        (CrosstermKeyCode::CapsLock, KeyCode::CapsLock),
        (CrosstermKeyCode::ScrollLock, KeyCode::ScrollLock),
        (CrosstermKeyCode::NumLock, KeyCode::NumLock),
        (CrosstermKeyCode::PrintScreen, KeyCode::PrintScreen),
        (CrosstermKeyCode::Pause, KeyCode::Pause),
        (CrosstermKeyCode::Menu, KeyCode::Menu),
        (CrosstermKeyCode::KeypadBegin, KeyCode::KeypadBegin),
        (
            CrosstermKeyCode::Media(CrosstermMediaKeyCode::Play),
            KeyCode::Media(MediaKeyCode::Play),
        ),
        (
            CrosstermKeyCode::Modifier(CrosstermModifierKeyCode::LeftShift),
            KeyCode::Modifier(ModifierKeyCode::LeftShift),
        ),
    ];

    for (source, expected) in cases {
        assert_eq!(
            decode_key(CrosstermKeyEvent::new(source, CrosstermKeyModifiers::NONE)).code,
            expected
        );
    }
}

// 모든 media subcode는 방향이나 기능이 뒤바뀌지 않고 그대로 대응한다.
#[test]
fn every_media_key_maps_one_to_one() {
    let cases = [
        (CrosstermMediaKeyCode::Play, MediaKeyCode::Play),
        (CrosstermMediaKeyCode::Pause, MediaKeyCode::Pause),
        (CrosstermMediaKeyCode::PlayPause, MediaKeyCode::PlayPause),
        (CrosstermMediaKeyCode::Reverse, MediaKeyCode::Reverse),
        (CrosstermMediaKeyCode::Stop, MediaKeyCode::Stop),
        (
            CrosstermMediaKeyCode::FastForward,
            MediaKeyCode::FastForward,
        ),
        (CrosstermMediaKeyCode::Rewind, MediaKeyCode::Rewind),
        (CrosstermMediaKeyCode::TrackNext, MediaKeyCode::TrackNext),
        (
            CrosstermMediaKeyCode::TrackPrevious,
            MediaKeyCode::TrackPrevious,
        ),
        (CrosstermMediaKeyCode::Record, MediaKeyCode::Record),
        (
            CrosstermMediaKeyCode::LowerVolume,
            MediaKeyCode::LowerVolume,
        ),
        (
            CrosstermMediaKeyCode::RaiseVolume,
            MediaKeyCode::RaiseVolume,
        ),
        (CrosstermMediaKeyCode::MuteVolume, MediaKeyCode::MuteVolume),
    ];

    for (source, expected) in cases {
        assert_eq!(
            decode_key(CrosstermKeyEvent::new(
                CrosstermKeyCode::Media(source),
                CrosstermKeyModifiers::NONE,
            ))
            .code,
            KeyCode::Media(expected)
        );
    }
}

// 모든 modifier subcode는 좌우와 ISO level identity를 그대로 대응한다.
#[test]
fn every_modifier_key_maps_one_to_one() {
    let cases = [
        (
            CrosstermModifierKeyCode::LeftShift,
            ModifierKeyCode::LeftShift,
        ),
        (
            CrosstermModifierKeyCode::LeftControl,
            ModifierKeyCode::LeftControl,
        ),
        (CrosstermModifierKeyCode::LeftAlt, ModifierKeyCode::LeftAlt),
        (
            CrosstermModifierKeyCode::LeftSuper,
            ModifierKeyCode::LeftSuper,
        ),
        (
            CrosstermModifierKeyCode::LeftHyper,
            ModifierKeyCode::LeftHyper,
        ),
        (
            CrosstermModifierKeyCode::LeftMeta,
            ModifierKeyCode::LeftMeta,
        ),
        (
            CrosstermModifierKeyCode::RightShift,
            ModifierKeyCode::RightShift,
        ),
        (
            CrosstermModifierKeyCode::RightControl,
            ModifierKeyCode::RightControl,
        ),
        (
            CrosstermModifierKeyCode::RightAlt,
            ModifierKeyCode::RightAlt,
        ),
        (
            CrosstermModifierKeyCode::RightSuper,
            ModifierKeyCode::RightSuper,
        ),
        (
            CrosstermModifierKeyCode::RightHyper,
            ModifierKeyCode::RightHyper,
        ),
        (
            CrosstermModifierKeyCode::RightMeta,
            ModifierKeyCode::RightMeta,
        ),
        (
            CrosstermModifierKeyCode::IsoLevel3Shift,
            ModifierKeyCode::IsoLevel3Shift,
        ),
        (
            CrosstermModifierKeyCode::IsoLevel5Shift,
            ModifierKeyCode::IsoLevel5Shift,
        ),
    ];

    for (source, expected) in cases {
        assert_eq!(
            decode_key(CrosstermKeyEvent::new(
                CrosstermKeyCode::Modifier(source),
                CrosstermKeyModifiers::NONE,
            ))
            .code,
            KeyCode::Modifier(expected)
        );
    }
}

// press·repeat·release action을 각각 같은 의미로 대응한다.
#[test]
fn every_key_action_maps_one_to_one() {
    for (source, expected) in [
        (KeyEventKind::Press, KeyAction::Press),
        (KeyEventKind::Repeat, KeyAction::Repeat),
        (KeyEventKind::Release, KeyAction::Release),
    ] {
        let event = CrosstermKeyEvent::new_with_kind(
            CrosstermKeyCode::Char('x'),
            CrosstermKeyModifiers::NONE,
            source,
        );
        assert_eq!(decode_key(event).action, expected);
    }
}

// modifier와 enhanced state의 각 bit를 독립적으로 보존한다.
#[test]
fn every_modifier_and_state_bit_maps_one_to_one() {
    for (source, expected) in [
        (CrosstermKeyModifiers::SHIFT, KeyModifiers::SHIFT),
        (CrosstermKeyModifiers::CONTROL, KeyModifiers::CONTROL),
        (CrosstermKeyModifiers::ALT, KeyModifiers::ALT),
        (CrosstermKeyModifiers::SUPER, KeyModifiers::SUPER),
        (CrosstermKeyModifiers::HYPER, KeyModifiers::HYPER),
        (CrosstermKeyModifiers::META, KeyModifiers::META),
    ] {
        let event = CrosstermKeyEvent::new(CrosstermKeyCode::Char('x'), source);
        assert_eq!(decode_key(event).modifiers, expected);
    }
    for (source, expected) in [
        (KeyEventState::KEYPAD, KeyState::KEYPAD),
        (KeyEventState::CAPS_LOCK, KeyState::CAPS_LOCK),
        (KeyEventState::NUM_LOCK, KeyState::NUM_LOCK),
    ] {
        let event = CrosstermKeyEvent::new_with_kind_and_state(
            CrosstermKeyCode::Char('x'),
            CrosstermKeyModifiers::NONE,
            KeyEventKind::Press,
            source,
        );
        assert_eq!(decode_key(event).state, expected);
    }
}

// bracketed paste의 control 문자와 줄바꿈은 command가 아닌 한 text payload로 유지한다.
#[test]
fn paste_is_decoded_as_one_text_payload() {
    let source = Event::Paste("first\n\u{3}\u{4}second".to_owned());

    assert_eq!(
        decode_event(source).unwrap(),
        InputEvent::Paste("first\n\u{3}\u{4}second".to_owned())
    );
}

// resize는 columns·rows를 이미 해석된 Surface width·height로 전달한다.
#[test]
fn resize_is_decoded_to_resolved_geometry() {
    assert_eq!(
        decode_event(Event::Resize(120, 40)).unwrap(),
        InputEvent::Resize(Size::new(120, 40))
    );
}

// 활성화하지 않은 focus와 mouse event는 편집기를 건드리지 않는 구조화 실패다.
#[test]
fn unsupported_events_are_explicit_failures() {
    let mouse = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 3,
        row: 4,
        modifiers: CrosstermKeyModifiers::NONE,
    });

    assert_eq!(
        decode_event(Event::FocusGained),
        Err(InputDecodeFailure::Unsupported(
            UnsupportedInputKind::FocusGained
        ))
    );
    assert_eq!(
        decode_event(Event::FocusLost),
        Err(InputDecodeFailure::Unsupported(
            UnsupportedInputKind::FocusLost
        ))
    );
    assert_eq!(
        decode_event(mouse),
        Err(InputDecodeFailure::Unsupported(UnsupportedInputKind::Mouse))
    );
}

struct RecordingSource {
    events: VecDeque<Result<Event, &'static str>>,
}

impl EventSource for RecordingSource {
    type Error = &'static str;

    fn read(&mut self) -> Result<Event, Self::Error> {
        self.events.pop_front().unwrap()
    }
}

// reader는 source가 전달한 정상 event만 semantic input으로 반환한다.
#[test]
fn reader_decodes_one_complete_source_event() {
    let source = RecordingSource {
        events: VecDeque::from([Ok(Event::Resize(120, 40))]),
    };
    let mut reader = InputReader::new(source);

    assert_eq!(
        reader.read().unwrap(),
        InputEvent::Resize(Size::new(120, 40))
    );
}

// Crossterm이 event를 만들기 전에 실패하면 원인을 추측하지 않고 source failure로 보존한다.
#[test]
fn source_failure_returns_no_partial_input_event() {
    let source = RecordingSource {
        events: VecDeque::from([Err("read failure"), Ok(Event::Paste("after".to_owned()))]),
    };
    let mut reader = InputReader::new(source);

    assert_eq!(reader.read(), Err(InputReadFailure::Source("read failure")));
    assert_eq!(
        reader.read().unwrap(),
        InputEvent::Paste("after".to_owned())
    );
}

// 정상적으로 읽힌 비활성 event는 source 오류와 구분된 decode failure다.
#[test]
fn unsupported_source_event_is_a_decode_failure() {
    let source = RecordingSource {
        events: VecDeque::from([Ok(Event::FocusGained)]),
    };
    let mut reader = InputReader::new(source);

    assert_eq!(
        reader.read(),
        Err(InputReadFailure::Decode(InputDecodeFailure::Unsupported(
            UnsupportedInputKind::FocusGained
        )))
    );
}

// production source는 하나만 소유하며 최초 reader thread 밖의 재획득도 거절한다.
#[test]
fn production_source_enforces_single_thread_ownership() {
    let source = CrosstermEventSource::acquire().unwrap();
    assert!(matches!(
        CrosstermEventSource::acquire(),
        Err(EventSourceAcquireFailure::AlreadyOwned)
    ));
    let _reader = InputReader::new(source);
    drop(_reader);

    let cross_thread = std::thread::spawn(|| match CrosstermEventSource::acquire() {
        Err(failure) => failure,
        Ok(_) => panic!("a different thread must not acquire the source"),
    })
    .join()
    .unwrap();
    assert_eq!(cross_thread, EventSourceAcquireFailure::DifferentThread);
}

fn decode_key(source: CrosstermKeyEvent) -> KeyEvent {
    let InputEvent::Key(decoded) = decode_event(Event::Key(source)).unwrap() else {
        panic!("a key event is expected");
    };
    decoded
}
