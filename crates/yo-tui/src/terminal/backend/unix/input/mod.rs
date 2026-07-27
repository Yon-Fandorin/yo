use crossterm::event::{
    Event, KeyCode as CrosstermKeyCode, KeyEvent as CrosstermKeyEvent, KeyEventKind, KeyEventState,
    KeyModifiers as CrosstermKeyModifiers, MediaKeyCode as CrosstermMediaKeyCode,
    ModifierKeyCode as CrosstermModifierKeyCode,
};

use crate::{
    input::event::{
        InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState, MediaKeyCode,
        ModifierKeyCode,
    },
    surface::Size,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InputDecodeFailure {
    Unsupported(UnsupportedInputKind),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UnsupportedInputKind {
    FocusGained,
    FocusLost,
    Mouse,
}

pub(super) fn decode_event(event: Event) -> Result<InputEvent, InputDecodeFailure> {
    match event {
        Event::Key(event) => Ok(InputEvent::Key(decode_key(event))),
        Event::Paste(text) => Ok(InputEvent::Paste(text)),
        Event::Resize(width, height) => Ok(InputEvent::Resize(Size::new(width, height))),
        Event::FocusGained => Err(InputDecodeFailure::Unsupported(
            UnsupportedInputKind::FocusGained,
        )),
        Event::FocusLost => Err(InputDecodeFailure::Unsupported(
            UnsupportedInputKind::FocusLost,
        )),
        Event::Mouse(_) => Err(InputDecodeFailure::Unsupported(UnsupportedInputKind::Mouse)),
    }
}

fn decode_key(event: CrosstermKeyEvent) -> KeyEvent {
    KeyEvent {
        code: decode_key_code(event.code),
        modifiers: decode_modifiers(event.modifiers),
        action: match event.kind {
            KeyEventKind::Press => KeyAction::Press,
            KeyEventKind::Repeat => KeyAction::Repeat,
            KeyEventKind::Release => KeyAction::Release,
        },
        state: decode_state(event.state),
    }
}

fn decode_key_code(code: CrosstermKeyCode) -> KeyCode {
    match code {
        CrosstermKeyCode::Backspace => KeyCode::Backspace,
        CrosstermKeyCode::Enter => KeyCode::Enter,
        CrosstermKeyCode::Left => KeyCode::Left,
        CrosstermKeyCode::Right => KeyCode::Right,
        CrosstermKeyCode::Up => KeyCode::Up,
        CrosstermKeyCode::Down => KeyCode::Down,
        CrosstermKeyCode::Home => KeyCode::Home,
        CrosstermKeyCode::End => KeyCode::End,
        CrosstermKeyCode::PageUp => KeyCode::PageUp,
        CrosstermKeyCode::PageDown => KeyCode::PageDown,
        CrosstermKeyCode::Tab => KeyCode::Tab,
        CrosstermKeyCode::BackTab => KeyCode::BackTab,
        CrosstermKeyCode::Delete => KeyCode::Delete,
        CrosstermKeyCode::Insert => KeyCode::Insert,
        CrosstermKeyCode::F(number) => KeyCode::Function(number),
        CrosstermKeyCode::Char(character) => KeyCode::Character(character),
        CrosstermKeyCode::Null => KeyCode::Null,
        CrosstermKeyCode::Esc => KeyCode::Escape,
        CrosstermKeyCode::CapsLock => KeyCode::CapsLock,
        CrosstermKeyCode::ScrollLock => KeyCode::ScrollLock,
        CrosstermKeyCode::NumLock => KeyCode::NumLock,
        CrosstermKeyCode::PrintScreen => KeyCode::PrintScreen,
        CrosstermKeyCode::Pause => KeyCode::Pause,
        CrosstermKeyCode::Menu => KeyCode::Menu,
        CrosstermKeyCode::KeypadBegin => KeyCode::KeypadBegin,
        CrosstermKeyCode::Media(code) => KeyCode::Media(decode_media_key(code)),
        CrosstermKeyCode::Modifier(code) => KeyCode::Modifier(decode_modifier_key(code)),
    }
}

fn decode_media_key(code: CrosstermMediaKeyCode) -> MediaKeyCode {
    match code {
        CrosstermMediaKeyCode::Play => MediaKeyCode::Play,
        CrosstermMediaKeyCode::Pause => MediaKeyCode::Pause,
        CrosstermMediaKeyCode::PlayPause => MediaKeyCode::PlayPause,
        CrosstermMediaKeyCode::Reverse => MediaKeyCode::Reverse,
        CrosstermMediaKeyCode::Stop => MediaKeyCode::Stop,
        CrosstermMediaKeyCode::FastForward => MediaKeyCode::FastForward,
        CrosstermMediaKeyCode::Rewind => MediaKeyCode::Rewind,
        CrosstermMediaKeyCode::TrackNext => MediaKeyCode::TrackNext,
        CrosstermMediaKeyCode::TrackPrevious => MediaKeyCode::TrackPrevious,
        CrosstermMediaKeyCode::Record => MediaKeyCode::Record,
        CrosstermMediaKeyCode::LowerVolume => MediaKeyCode::LowerVolume,
        CrosstermMediaKeyCode::RaiseVolume => MediaKeyCode::RaiseVolume,
        CrosstermMediaKeyCode::MuteVolume => MediaKeyCode::MuteVolume,
    }
}

fn decode_modifier_key(code: CrosstermModifierKeyCode) -> ModifierKeyCode {
    match code {
        CrosstermModifierKeyCode::LeftShift => ModifierKeyCode::LeftShift,
        CrosstermModifierKeyCode::LeftControl => ModifierKeyCode::LeftControl,
        CrosstermModifierKeyCode::LeftAlt => ModifierKeyCode::LeftAlt,
        CrosstermModifierKeyCode::LeftSuper => ModifierKeyCode::LeftSuper,
        CrosstermModifierKeyCode::LeftHyper => ModifierKeyCode::LeftHyper,
        CrosstermModifierKeyCode::LeftMeta => ModifierKeyCode::LeftMeta,
        CrosstermModifierKeyCode::RightShift => ModifierKeyCode::RightShift,
        CrosstermModifierKeyCode::RightControl => ModifierKeyCode::RightControl,
        CrosstermModifierKeyCode::RightAlt => ModifierKeyCode::RightAlt,
        CrosstermModifierKeyCode::RightSuper => ModifierKeyCode::RightSuper,
        CrosstermModifierKeyCode::RightHyper => ModifierKeyCode::RightHyper,
        CrosstermModifierKeyCode::RightMeta => ModifierKeyCode::RightMeta,
        CrosstermModifierKeyCode::IsoLevel3Shift => ModifierKeyCode::IsoLevel3Shift,
        CrosstermModifierKeyCode::IsoLevel5Shift => ModifierKeyCode::IsoLevel5Shift,
    }
}

fn decode_modifiers(modifiers: CrosstermKeyModifiers) -> KeyModifiers {
    let mut decoded = KeyModifiers::NONE;
    for (source, target) in [
        (CrosstermKeyModifiers::SHIFT, KeyModifiers::SHIFT),
        (CrosstermKeyModifiers::CONTROL, KeyModifiers::CONTROL),
        (CrosstermKeyModifiers::ALT, KeyModifiers::ALT),
        (CrosstermKeyModifiers::SUPER, KeyModifiers::SUPER),
        (CrosstermKeyModifiers::HYPER, KeyModifiers::HYPER),
        (CrosstermKeyModifiers::META, KeyModifiers::META),
    ] {
        if modifiers.contains(source) {
            decoded = decoded.union(target);
        }
    }
    decoded
}

fn decode_state(state: KeyEventState) -> KeyState {
    let mut decoded = KeyState::NONE;
    for (source, target) in [
        (KeyEventState::KEYPAD, KeyState::KEYPAD),
        (KeyEventState::CAPS_LOCK, KeyState::CAPS_LOCK),
        (KeyEventState::NUM_LOCK, KeyState::NUM_LOCK),
    ] {
        if state.contains(source) {
            decoded = decoded.union(target);
        }
    }
    decoded
}

#[cfg(test)]
mod tests;
