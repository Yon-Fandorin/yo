use crate::input::{
    event::{InputEvent, KeyAction, KeyCode, KeyModifiers},
    key_notation::key_notation,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OverlayAction {
    Dismiss,
    Previous,
    Next,
    FilterPrevious,
    FilterNext,
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
    physical: String,
    caption: &'static str,
    optional: bool,
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

    pub(super) fn hints(&self, turn_active: bool, rich_keys: bool) -> Vec<BindingHint> {
        let mut hints = Vec::new();
        let previous = self.binding(OverlayAction::Previous);
        let next = self.binding(OverlayAction::Next);
        if let (Some(previous), Some(next)) = (previous, next) {
            hints.push(BindingHint {
                physical: if rich_keys {
                    format!(
                        "{}{}",
                        binding_notation(previous, true),
                        binding_notation(next, true)
                    )
                } else {
                    format!(
                        "{}/{}",
                        binding_notation(previous, false),
                        binding_notation(next, false)
                    )
                },
                caption: "move",
                optional: true,
            });
        }
        if let Some(accept) = self.binding(OverlayAction::Accept) {
            hints.push(BindingHint {
                physical: binding_notation(accept, rich_keys),
                caption: "insert",
                optional: true,
            });
        }
        let mut actions = vec![OverlayAction::Dismiss];
        if turn_active {
            actions.push(OverlayAction::Interrupt);
        }
        hints.extend(actions.into_iter().filter_map(|action| {
            self.binding(action).map(|binding| BindingHint {
                physical: binding_notation(binding, rich_keys),
                caption: binding.action.caption(),
                optional: false,
            })
        }));
        hints
    }

    fn binding(&self, action: OverlayAction) -> Option<&ResolvedBinding> {
        self.bindings
            .iter()
            .find(|binding| binding.action == action)
    }
}

impl BindingHint {
    pub(super) fn physical(&self) -> &str {
        &self.physical
    }

    pub(super) const fn caption(&self) -> &'static str {
        self.caption
    }

    pub(super) const fn is_optional(&self) -> bool {
        self.optional
    }
}

impl OverlayAction {
    const fn caption(self) -> &'static str {
        match self {
            Self::Dismiss => "close",
            Self::Previous => "previous",
            Self::Next => "next",
            Self::FilterPrevious => "previous filter",
            Self::FilterNext => "next filter",
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
                },
                ResolvedBinding {
                    action: OverlayAction::Previous,
                    code: KeyCode::Up,
                    modifiers: KeyModifiers::NONE,
                },
                ResolvedBinding {
                    action: OverlayAction::Next,
                    code: KeyCode::Down,
                    modifiers: KeyModifiers::NONE,
                },
                ResolvedBinding {
                    action: OverlayAction::FilterPrevious,
                    code: KeyCode::Left,
                    modifiers: KeyModifiers::NONE,
                },
                ResolvedBinding {
                    action: OverlayAction::FilterNext,
                    code: KeyCode::Right,
                    modifiers: KeyModifiers::NONE,
                },
                ResolvedBinding {
                    action: OverlayAction::Accept,
                    code: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                },
                ResolvedBinding {
                    action: OverlayAction::Accept,
                    code: KeyCode::Tab,
                    modifiers: KeyModifiers::NONE,
                },
                ResolvedBinding {
                    action: OverlayAction::Interrupt,
                    code: KeyCode::Character('c'),
                    modifiers: KeyModifiers::CONTROL,
                },
                ResolvedBinding {
                    action: OverlayAction::Interrupt,
                    code: KeyCode::Character('C'),
                    modifiers: KeyModifiers::CONTROL,
                },
            ],
        }
    }
}

fn binding_notation(binding: &ResolvedBinding, rich_keys: bool) -> String {
    key_notation(binding.code, binding.modifiers, rich_keys)
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
                .hints(true, true)
                .iter()
                .map(|hint| (hint.physical(), hint.caption()))
                .collect::<Vec<_>>(),
            vec![
                ("↑↓", "move"),
                ("Enter", "insert"),
                ("Esc", "close"),
                ("^C", "interrupt"),
            ]
        );
        assert_eq!(
            bindings
                .hints(false, false)
                .iter()
                .map(|hint| (hint.physical(), hint.caption()))
                .collect::<Vec<_>>(),
            vec![("Up/Down", "move"), ("Enter", "insert"), ("Esc", "close"),]
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
