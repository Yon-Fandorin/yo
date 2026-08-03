use crate::input::event::{InputEvent, KeyAction, KeyCode, KeyModifiers};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OverlayAction {
    Dismiss,
    Previous,
    Next,
    Accept,
    Interrupt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BindingMatch {
    pub(super) action: OverlayAction,
    pub(super) key_action: KeyAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BindingHint {
    physical: &'static str,
    caption: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OverlayBindings {
    bindings: Vec<ResolvedBinding>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedBinding {
    action: OverlayAction,
    code: KeyCode,
    modifiers: KeyModifiers,
    physical: &'static str,
}

impl OverlayBindings {
    pub(super) fn classify(&self, input: &InputEvent) -> Option<BindingMatch> {
        let InputEvent::Key(key) = input else {
            return None;
        };
        if key.action == KeyAction::Release {
            return None;
        }
        self.bindings
            .iter()
            .find(|binding| binding.code == key.code && binding.modifiers == key.modifiers)
            .map(|binding| BindingMatch {
                action: binding.action,
                key_action: key.action,
            })
    }

    pub(super) fn hints(&self, turn_active: bool) -> Vec<BindingHint> {
        let mut actions = vec![OverlayAction::Dismiss];
        if turn_active {
            actions.push(OverlayAction::Interrupt);
        }
        actions
            .into_iter()
            .filter_map(|action| {
                self.bindings
                    .iter()
                    .find(|binding| binding.action == action)
            })
            .map(|binding| BindingHint {
                physical: binding.physical,
                caption: binding.action.caption(),
            })
            .collect()
    }
}

impl BindingHint {
    pub(super) const fn physical(&self) -> &'static str {
        self.physical
    }

    pub(super) const fn caption(&self) -> &'static str {
        self.caption
    }
}

impl OverlayAction {
    const fn caption(self) -> &'static str {
        match self {
            Self::Dismiss => "close",
            Self::Previous => "previous",
            Self::Next => "next",
            Self::Accept => "select",
            Self::Interrupt => "interrupt",
        }
    }
}

impl Default for OverlayBindings {
    fn default() -> Self {
        Self {
            bindings: vec![
                ResolvedBinding {
                    action: OverlayAction::Dismiss,
                    code: KeyCode::Escape,
                    modifiers: KeyModifiers::NONE,
                    physical: "Esc",
                },
                ResolvedBinding {
                    action: OverlayAction::Previous,
                    code: KeyCode::Up,
                    modifiers: KeyModifiers::NONE,
                    physical: "↑",
                },
                ResolvedBinding {
                    action: OverlayAction::Next,
                    code: KeyCode::Down,
                    modifiers: KeyModifiers::NONE,
                    physical: "↓",
                },
                ResolvedBinding {
                    action: OverlayAction::Accept,
                    code: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                    physical: "Enter",
                },
                ResolvedBinding {
                    action: OverlayAction::Accept,
                    code: KeyCode::Tab,
                    modifiers: KeyModifiers::NONE,
                    physical: "Tab",
                },
                ResolvedBinding {
                    action: OverlayAction::Interrupt,
                    code: KeyCode::Character('c'),
                    modifiers: KeyModifiers::CONTROL,
                    physical: "Ctrl+C",
                },
                ResolvedBinding {
                    action: OverlayAction::Interrupt,
                    code: KeyCode::Character('C'),
                    modifiers: KeyModifiers::CONTROL,
                    physical: "Ctrl+C",
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OverlayAction, OverlayBindings};
    use crate::input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> InputEvent {
        InputEvent::Key(KeyEvent {
            code,
            modifiers,
            action: KeyAction::Press,
            state: KeyState::NONE,
        })
    }

    // 화면 hint와 입력 routing은 같은 resolved binding 목록에서 Esc close와 active-Turn
    // Ctrl+C interrupt를 도출한다.
    #[test]
    fn one_resolved_map_drives_mandatory_hints_and_commands() {
        let bindings = OverlayBindings::default();

        assert_eq!(
            bindings
                .hints(true)
                .iter()
                .map(|hint| (hint.physical(), hint.caption()))
                .collect::<Vec<_>>(),
            vec![("Esc", "close"), ("Ctrl+C", "interrupt")]
        );
        assert_eq!(
            bindings
                .classify(&key(KeyCode::Escape, KeyModifiers::NONE))
                .unwrap()
                .action,
            OverlayAction::Dismiss
        );
        assert_eq!(
            bindings
                .classify(&key(KeyCode::Character('c'), KeyModifiers::CONTROL))
                .unwrap()
                .action,
            OverlayAction::Interrupt
        );
    }
}
