//! Typed presentation-policy bindings for switching observability views.

use super::event::{InputEvent, KeyAction, KeyCode, KeyModifiers};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ViewSwitchTarget {
    Chat,
    Transcript,
    Request,
    Usage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ViewSwitchBinding {
    code: KeyCode,
    target: ViewSwitchTarget,
}

impl ViewSwitchBinding {
    pub(crate) const fn new(code: KeyCode, target: ViewSwitchTarget) -> Self {
        Self { code, target }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ViewSwitchBindings {
    bindings: [ViewSwitchBinding; 4],
}

impl ViewSwitchBindings {
    pub(crate) const fn new(bindings: [ViewSwitchBinding; 4]) -> Self {
        Self { bindings }
    }

    pub(crate) fn target(self, input: &InputEvent) -> Option<ViewSwitchTarget> {
        let InputEvent::Key(key) = input else {
            return None;
        };
        if key.action == KeyAction::Release || key.modifiers != KeyModifiers::NONE {
            return None;
        }
        self.bindings
            .into_iter()
            .find(|binding| binding.code == key.code)
            .map(|binding| binding.target)
    }
}

impl Default for ViewSwitchBindings {
    fn default() -> Self {
        Self::new([
            ViewSwitchBinding::new(KeyCode::Function(1), ViewSwitchTarget::Chat),
            ViewSwitchBinding::new(KeyCode::Function(2), ViewSwitchTarget::Transcript),
            ViewSwitchBinding::new(KeyCode::Function(3), ViewSwitchTarget::Request),
            ViewSwitchBinding::new(KeyCode::Function(4), ViewSwitchTarget::Usage),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::{ViewSwitchBindings, ViewSwitchTarget};
    use crate::input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState};

    fn key(code: KeyCode, modifiers: KeyModifiers, action: KeyAction) -> InputEvent {
        InputEvent::Key(KeyEvent {
            code,
            modifiers,
            action,
            state: KeyState::NONE,
        })
    }

    // 기본 view binding은 modifier 없는 F1/F2/F3/F4 press를 각각 네 관측 view로 정확히
    // 매핑해 editor 문자 입력과 겹치지 않는다.
    #[test]
    fn default_bindings_map_plain_function_key_presses_exactly() {
        let bindings = ViewSwitchBindings::default();

        assert_eq!(
            bindings.target(&key(
                KeyCode::Function(1),
                KeyModifiers::NONE,
                KeyAction::Press
            )),
            Some(ViewSwitchTarget::Chat)
        );
        assert_eq!(
            bindings.target(&key(
                KeyCode::Function(2),
                KeyModifiers::NONE,
                KeyAction::Press
            )),
            Some(ViewSwitchTarget::Transcript)
        );
        assert_eq!(
            bindings.target(&key(
                KeyCode::Function(3),
                KeyModifiers::NONE,
                KeyAction::Press
            )),
            Some(ViewSwitchTarget::Request)
        );
        assert_eq!(
            bindings.target(&key(
                KeyCode::Function(4),
                KeyModifiers::NONE,
                KeyAction::Press
            )),
            Some(ViewSwitchTarget::Usage)
        );
    }

    // release와 modifier가 붙은 function key는 기본 정책의 switch가 아니므로 terminal
    // decoder가 보낸 다른 의미의 입력을 mode 전환으로 오인하지 않는다.
    #[test]
    fn default_bindings_ignore_releases_and_modified_function_keys() {
        let bindings = ViewSwitchBindings::default();

        assert_eq!(
            bindings.target(&key(
                KeyCode::Function(2),
                KeyModifiers::NONE,
                KeyAction::Release
            )),
            None
        );
        assert_eq!(
            bindings.target(&key(
                KeyCode::Function(2),
                KeyModifiers::SHIFT,
                KeyAction::Press
            )),
            None
        );
    }
}
