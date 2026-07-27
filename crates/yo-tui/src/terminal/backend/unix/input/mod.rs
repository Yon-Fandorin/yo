//! Exclusive Crossterm event boundary for one process-affine reader thread.
//!
//! All yo calls to Crossterm `read`, `poll`, or `EventStream` must remain in
//! this module so the dependency's single-thread precondition stays enforced.

use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{Mutex, MutexGuard, TryLockError},
    thread::{self, ThreadId},
};

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

#[derive(Debug, Eq, PartialEq)]
pub(super) enum InputReadFailure<SourceError> {
    Source(SourceError),
    Decode(InputDecodeFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EventSourceAcquireFailure {
    AlreadyOwned,
    DifferentThread,
    OwnershipPoisoned,
}

pub(super) trait EventSource {
    type Error;

    fn read(&mut self) -> Result<Event, Self::Error>;
}

pub(super) struct InputReader<S> {
    source: S,
}

impl<S> InputReader<S>
where
    S: EventSource,
{
    pub(super) fn new(source: S) -> Self {
        Self { source }
    }

    pub(super) fn read(&mut self) -> Result<InputEvent, InputReadFailure<S::Error>> {
        let event = self.source.read().map_err(InputReadFailure::Source)?;
        decode_event(event).map_err(InputReadFailure::Decode)
    }
}

static CROSSTERM_EVENT_OWNER: Mutex<Option<ThreadId>> = Mutex::new(None);

pub(super) struct CrosstermEventSource {
    _ownership: MutexGuard<'static, Option<ThreadId>>,
    _thread_affinity: PhantomData<Rc<()>>,
}

impl CrosstermEventSource {
    pub(super) fn acquire() -> Result<Self, EventSourceAcquireFailure> {
        let mut ownership = match CROSSTERM_EVENT_OWNER.try_lock() {
            Ok(ownership) => ownership,
            Err(TryLockError::WouldBlock) => {
                return Err(EventSourceAcquireFailure::AlreadyOwned);
            },
            Err(TryLockError::Poisoned(_)) => {
                return Err(EventSourceAcquireFailure::OwnershipPoisoned);
            },
        };
        let current = thread::current().id();
        if ownership.as_ref().is_some_and(|owner| *owner != current) {
            return Err(EventSourceAcquireFailure::DifferentThread);
        }
        ownership.get_or_insert(current);

        Ok(Self {
            _ownership: ownership,
            _thread_affinity: PhantomData,
        })
    }
}

impl EventSource for CrosstermEventSource {
    type Error = std::io::Error;

    fn read(&mut self) -> Result<Event, Self::Error> {
        crossterm::event::read()
    }
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
