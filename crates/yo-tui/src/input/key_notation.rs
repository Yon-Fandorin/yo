//! Familiar terminal key labels shared by input-adjacent UI.

use super::event::{KeyCode, KeyModifiers};

pub(crate) fn interrupt_notation() -> String {
    let escape = key_notation(KeyCode::Escape, KeyModifiers::NONE, false);
    let control_c = key_notation(KeyCode::Character('c'), KeyModifiers::CONTROL, false);
    format!("{escape}/{control_c}")
}

pub(crate) fn key_notation(code: KeyCode, modifiers: KeyModifiers, rich_arrows: bool) -> String {
    if modifiers.contains(KeyModifiers::CONTROL)
        && let KeyCode::Character(character) = code
    {
        return format!("^{}", character.to_uppercase().collect::<String>());
    }
    let mut notation = String::new();
    if modifiers.contains(KeyModifiers::CONTROL) {
        notation.push_str("C-");
    }
    if modifiers.contains(KeyModifiers::ALT) || modifiers.contains(KeyModifiers::META) {
        notation.push_str("M-");
    }
    if modifiers.contains(KeyModifiers::SHIFT) {
        notation.push_str("S-");
    }
    notation.push_str(match code {
        KeyCode::Character(character) => return format!("{notation}{character}"),
        KeyCode::Enter => "Enter",
        KeyCode::Tab => "Tab",
        KeyCode::BackTab => "BackTab",
        KeyCode::Backspace => "Backspace",
        KeyCode::Delete => "Delete",
        KeyCode::Escape => "Esc",
        KeyCode::Up if rich_arrows => "↑",
        KeyCode::Down if rich_arrows => "↓",
        KeyCode::Left if rich_arrows => "←",
        KeyCode::Right if rich_arrows => "→",
        KeyCode::Up => "Up",
        KeyCode::Down => "Down",
        KeyCode::Left => "Left",
        KeyCode::Right => "Right",
        KeyCode::Home => "Home",
        KeyCode::End => "End",
        KeyCode::PageUp => "PageUp",
        KeyCode::PageDown => "PageDown",
        KeyCode::Insert => "Insert",
        KeyCode::Function(_)
        | KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_) => "Key",
    });
    notation
}

#[cfg(test)]
mod tests {
    use super::{interrupt_notation, key_notation};
    use crate::input::event::{KeyCode, KeyModifiers};

    // 상태줄과 overlay가 공유하는 표기는 제어 문자를 caret 표기로, 조합 키를 익숙한
    // modifier-prefix 표기로 유지해 임시 시안이 별도 기호 체계를 만들지 않게 한다.
    #[test]
    fn formats_terminal_control_and_modifier_conventions() {
        assert_eq!(
            key_notation(KeyCode::Character('d'), KeyModifiers::CONTROL, true),
            "^D"
        );
        assert_eq!(
            key_notation(KeyCode::Enter, KeyModifiers::SHIFT, true),
            "S-Enter"
        );
        assert_eq!(
            key_notation(KeyCode::Escape, KeyModifiers::NONE, true),
            "Esc"
        );
        assert_eq!(interrupt_notation(), "Esc/^C");
    }
}
