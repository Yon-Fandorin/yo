use crate::surface::Size;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum InputEvent {
    Key(KeyEvent),
    Paste(String),
    Resize(Size),
}

impl InputEvent {
    pub(crate) fn is_ctrl_c_or_d(&self) -> bool {
        matches!(
            self,
            Self::Key(KeyEvent {
                code: KeyCode::Character('c' | 'd'),
                modifiers,
                ..
            }) if modifiers.contains(KeyModifiers::CONTROL)
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct KeyEvent {
    pub(crate) code: KeyCode,
    pub(crate) modifiers: KeyModifiers,
    pub(crate) action: KeyAction,
    pub(crate) state: KeyState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeyAction {
    Press,
    Repeat,
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeyCode {
    Character(char),
    Enter,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Function(u8),
    Null,
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    Menu,
    KeypadBegin,
    Media(MediaKeyCode),
    Modifier(ModifierKeyCode),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MediaKeyCode {
    Play,
    Pause,
    PlayPause,
    Reverse,
    Stop,
    FastForward,
    Rewind,
    TrackNext,
    TrackPrevious,
    Record,
    LowerVolume,
    RaiseVolume,
    MuteVolume,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ModifierKeyCode {
    LeftShift,
    LeftControl,
    LeftAlt,
    LeftSuper,
    LeftHyper,
    LeftMeta,
    RightShift,
    RightControl,
    RightAlt,
    RightSuper,
    RightHyper,
    RightMeta,
    IsoLevel3Shift,
    IsoLevel5Shift,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct KeyModifiers(u8);

impl KeyModifiers {
    pub(crate) const NONE: Self = Self(0);
    pub(crate) const SHIFT: Self = Self(1 << 0);
    pub(crate) const CONTROL: Self = Self(1 << 1);
    pub(crate) const ALT: Self = Self(1 << 2);
    pub(crate) const SUPER: Self = Self(1 << 3);
    pub(crate) const HYPER: Self = Self(1 << 4);
    pub(crate) const META: Self = Self(1 << 5);

    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct KeyState(u8);

impl KeyState {
    pub(crate) const NONE: Self = Self(0);
    pub(crate) const KEYPAD: Self = Self(1 << 0);
    pub(crate) const CAPS_LOCK: Self = Self(1 << 1);
    pub(crate) const NUM_LOCK: Self = Self(1 << 2);

    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}
