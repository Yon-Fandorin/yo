use super::{SecretEditor, SecretEditorEffect, SecretPublicState};
use crate::input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState};

fn key(code: KeyCode, modifiers: KeyModifiers) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code,
        modifiers,
        action: KeyAction::Press,
        state: KeyState::NONE,
    })
}

// 붙여넣기의 개행과 슬래시는 명령으로 해석되지 않고 비밀 값에 그대로 들어간다.
#[test]
fn paste_is_literal_and_public_state_is_fixed() {
    let mut editor = SecretEditor::new();
    assert!(!editor.ready());
    editor.mark_ready();
    assert!(editor.ready());

    assert_eq!(
        editor.handle(InputEvent::Paste("/token\n$HOME".into())),
        SecretEditorEffect::Changed
    );
    assert_eq!(editor.public_state(), SecretPublicState::Entered);
    assert_eq!(editor.public_text(), "Entered");
    assert!(!editor.public_text().contains("token"));
}

// 복구된 secret row는 값을 되살리지 않고 고정된 재입력 상태만 노출한다.
#[test]
fn recovered_secret_requires_reentry_without_becoming_ready() {
    let editor = SecretEditor::reentry_required();
    assert_eq!(editor.public_state(), SecretPublicState::ReentryRequired);
    assert_eq!(editor.public_text(), "Re-entry required");
    assert!(!editor.ready());
}

// 결합 문자와 이모지는 한 grapheme 단위로 삭제되고 Enter만 제출을 발생시킨다.
#[test]
fn deletion_and_explicit_submit_preserve_unicode() {
    let mut editor = SecretEditor::new();
    editor.mark_ready();
    assert_eq!(
        editor.handle(InputEvent::Paste("e\u{301}👨‍👩‍👧".into())),
        SecretEditorEffect::Changed
    );
    assert_eq!(
        editor.handle(key(KeyCode::Backspace, KeyModifiers::NONE)),
        SecretEditorEffect::Changed
    );
    assert_eq!(
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE)),
        SecretEditorEffect::Submitted(
            yo_core::SecretInput::new("e\u{301}").expect("bounded secret")
        )
    );
    assert_eq!(editor.public_state(), SecretPublicState::NotEntered);
}

// 용량을 넘는 붙여넣기는 첫 초과 바이트에서 전체가 거부되고 기존 값은 보존된다.
#[test]
fn oversized_paste_is_rejected_without_truncation() {
    let mut editor = SecretEditor::new();
    editor.mark_ready();
    assert_eq!(
        editor.handle(InputEvent::Paste(
            "a".repeat(yo_core::SecretInput::MAX_BYTES)
        )),
        SecretEditorEffect::Changed
    );
    assert_eq!(
        editor.handle(InputEvent::Paste("b".into())),
        SecretEditorEffect::Rejected
    );
    assert_eq!(
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE)),
        SecretEditorEffect::Submitted(
            yo_core::SecretInput::new("a".repeat(yo_core::SecretInput::MAX_BYTES))
                .expect("bounded secret")
        )
    );
}

// Ctrl-U는 kill/yank 상태를 만들지 않고 현재 줄의 앞부분만 비운다.
#[test]
fn ctrl_u_clears_without_kill_or_yank_state() {
    let mut editor = SecretEditor::new();
    editor.mark_ready();
    editor.handle(InputEvent::Paste("first\nsecond".into()));
    assert_eq!(
        editor.handle(key(KeyCode::Character('u'), KeyModifiers::CONTROL)),
        SecretEditorEffect::Changed
    );
    assert_eq!(
        editor.handle(key(KeyCode::Enter, KeyModifiers::NONE)),
        SecretEditorEffect::Submitted(
            yo_core::SecretInput::new("first\n").expect("bounded secret")
        )
    );
}

// 첫 Ctrl-R은 저장 경계를 먼저 보여 주도록 요청하고, 별도 두 번째 Ctrl-R만 현재 값을
// 저장 후보로 내보낸다. 복구도 버퍼를 표시하지 않고 fresh Enter 전까지 보내지 않는다.
#[test]
fn recovery_requires_disclosure_then_a_second_action_and_fresh_submit() {
    let mut editor = SecretEditor::new();
    editor.mark_ready();
    editor.handle(InputEvent::Paste("vault-value".into()));

    assert_eq!(
        editor.handle(key(KeyCode::Character('r'), KeyModifiers::CONTROL)),
        SecretEditorEffect::RecoveryDisclosureRequested
    );
    editor.mark_recovery_disclosed();
    assert_eq!(
        editor.handle(key(KeyCode::Character('r'), KeyModifiers::CONTROL)),
        SecretEditorEffect::StoreRecovery(
            yo_core::SecretInput::new("vault-value").expect("bounded secret")
        )
    );
    editor.mark_recovery_available();
    assert_eq!(editor.public_text(), "Recovery available");

    let mut recovered = SecretEditor::new();
    recovered.mark_ready();
    recovered.mark_recovery_available();
    assert_eq!(
        recovered.handle(key(KeyCode::Character('r'), KeyModifiers::CONTROL)),
        SecretEditorEffect::RecoverRequested
    );
    recovered.restore(yo_core::SecretInput::new("vault-value").unwrap());
    assert_eq!(recovered.public_text(), "Recovered");
    assert_eq!(
        recovered.handle(key(KeyCode::Enter, KeyModifiers::NONE)),
        SecretEditorEffect::Submitted(yo_core::SecretInput::new("vault-value").unwrap())
    );
}
