use std::time::Duration;

use super::{EditorEffect, PromptEditor};
use crate::{
    input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
    surface::Size,
};

const NOW: Duration = Duration::from_secs(10);

fn key(code: KeyCode, modifiers: KeyModifiers, action: KeyAction) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code,
        modifiers,
        action,
        state: KeyState::NONE,
    })
}

fn press(code: KeyCode) -> InputEvent {
    key(code, KeyModifiers::NONE, KeyAction::Press)
}

// 일반 문자와 Shift로 확정된 문자는 현재 커서에 삽입한다.
#[test]
fn inserts_resolved_plain_characters() {
    let mut editor = PromptEditor::new();

    assert_eq!(
        editor.handle(press(KeyCode::Character('가')), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(
        editor.handle(
            key(
                KeyCode::Character('A'),
                KeyModifiers::SHIFT,
                KeyAction::Press
            ),
            false,
            NOW
        ),
        EditorEffect::BufferChanged
    );

    assert_eq!(editor.text(), "가A");
    assert_eq!(editor.cursor_byte_index(), "가A".len());
}

// bracketed paste는 줄바꿈과 control 문자를 실행하지 않고 한 payload로 삽입한다.
#[test]
fn inserts_paste_payload_without_executing_its_contents() {
    let mut editor = PromptEditor::new();

    let effect = editor.handle(InputEvent::Paste("a\n\u{3}b".into()), false, NOW);

    assert_eq!(effect, EditorEffect::BufferChanged);
    assert_eq!(editor.text(), "a\n\u{3}b");
}

// 좌우 이동과 Backspace 및 Delete는 grapheme 한 개씩만 편집한다.
#[test]
fn edits_one_grapheme_per_navigation_command() {
    let mut editor = PromptEditor::new();
    editor.handle(InputEvent::Paste("A가B".into()), false, NOW);

    assert_eq!(
        editor.handle(press(KeyCode::Left), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(
        editor.handle(press(KeyCode::Left), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(
        editor.handle(press(KeyCode::Right), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(
        editor.handle(press(KeyCode::Backspace), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(editor.text(), "AB");
    assert_eq!(
        editor.handle(press(KeyCode::Delete), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(editor.text(), "A");
}

// 키 repeat는 일반 편집을 반복하지만 release는 텍스트를 바꾸지 않는다.
#[test]
fn repeats_edits_and_ignores_releases() {
    let mut editor = PromptEditor::new();

    assert_eq!(
        editor.handle(
            key(
                KeyCode::Character('x'),
                KeyModifiers::NONE,
                KeyAction::Repeat
            ),
            false,
            NOW
        ),
        EditorEffect::BufferChanged
    );
    assert_eq!(
        editor.handle(
            key(
                KeyCode::Character('x'),
                KeyModifiers::NONE,
                KeyAction::Release
            ),
            false,
            NOW
        ),
        EditorEffect::Unhandled
    );
    assert_eq!(editor.text(), "x");
}

// Alt 문자와 modifier가 추가된 Enter는 선택된 편집 계약이 아니므로 그대로 돌려준다.
#[test]
fn leaves_unselected_commands_unhandled() {
    let mut editor = PromptEditor::new();
    editor.handle(InputEvent::Paste("x".into()), false, NOW);

    assert_eq!(
        editor.handle(
            key(KeyCode::Character('x'), KeyModifiers::ALT, KeyAction::Press),
            false,
            NOW
        ),
        EditorEffect::Unhandled
    );
    assert_eq!(
        editor.handle(
            key(KeyCode::Left, KeyModifiers::SHIFT, KeyAction::Press),
            false,
            NOW
        ),
        EditorEffect::Unhandled
    );
    assert_eq!(
        editor.handle(
            key(KeyCode::Delete, KeyModifiers::ALT, KeyAction::Press),
            false,
            NOW
        ),
        EditorEffect::Unhandled
    );
    assert_eq!(
        editor.handle(
            key(KeyCode::Enter, KeyModifiers::CONTROL, KeyAction::Press),
            false,
            NOW
        ),
        EditorEffect::Unhandled
    );
    assert_eq!(editor.text(), "x");
}

// Enter는 현재 입력을 소유한 제출 효과로 넘기고 편집 버퍼를 비운다.
#[test]
fn enter_submits_owned_text_and_resets_the_editor() {
    let mut editor = PromptEditor::new();
    editor.handle(InputEvent::Paste("질문".into()), false, NOW);

    assert_eq!(
        editor.handle(press(KeyCode::Enter), false, NOW),
        EditorEffect::Submitted("질문".into())
    );
    assert!(editor.text().is_empty());
    assert_eq!(editor.cursor_byte_index(), 0);
}

// 빈 입력의 Enter는 빈 요청을 제출하지 않고 상태를 그대로 둔다.
#[test]
fn enter_does_not_submit_empty_text() {
    let mut editor = PromptEditor::new();

    assert_eq!(
        editor.handle(press(KeyCode::Enter), false, NOW),
        EditorEffect::NoChange
    );
    assert!(editor.text().is_empty());
}

// Shift+Enter는 제출하지 않고 편집 중인 문자열에 줄바꿈을 추가한다.
#[test]
fn shift_enter_inserts_a_newline() {
    let mut editor = PromptEditor::new();
    editor.handle(InputEvent::Paste("첫 줄".into()), false, NOW);

    assert_eq!(
        editor.handle(
            key(KeyCode::Enter, KeyModifiers::SHIFT, KeyAction::Press),
            false,
            NOW
        ),
        EditorEffect::BufferChanged
    );
    editor.handle(InputEvent::Paste("둘째 줄".into()), false, NOW);

    assert_eq!(editor.text(), "첫 줄\n둘째 줄");
}

// Enter release는 무시하고 repeat 제출은 한 번만 일어나며 Shift+Enter repeat는 줄바꿈을 반복한다.
#[test]
fn enter_actions_preserve_release_and_repeat_semantics() {
    let mut editor = PromptEditor::new();
    editor.handle(InputEvent::Paste("질문".into()), false, NOW);

    assert_eq!(
        editor.handle(
            key(KeyCode::Enter, KeyModifiers::NONE, KeyAction::Release),
            false,
            NOW
        ),
        EditorEffect::Unhandled
    );
    assert_eq!(editor.text(), "질문");
    assert_eq!(
        editor.handle(
            key(KeyCode::Enter, KeyModifiers::NONE, KeyAction::Repeat),
            false,
            NOW
        ),
        EditorEffect::Submitted("질문".into())
    );
    assert_eq!(
        editor.handle(
            key(KeyCode::Enter, KeyModifiers::NONE, KeyAction::Repeat),
            false,
            NOW
        ),
        EditorEffect::NoChange
    );
    assert_eq!(
        editor.handle(
            key(KeyCode::Enter, KeyModifiers::SHIFT, KeyAction::Repeat),
            false,
            NOW
        ),
        EditorEffect::BufferChanged
    );
    assert_eq!(
        editor.handle(
            key(KeyCode::Enter, KeyModifiers::SHIFT, KeyAction::Repeat),
            false,
            NOW
        ),
        EditorEffect::BufferChanged
    );
    assert_eq!(editor.text(), "\n\n");
}

// Ctrl+C/D 결과는 실제 프로세스 동작 없이 명시적인 요청으로 전달한다.
#[test]
fn forwards_control_policy_as_editor_effects() {
    let mut editor = PromptEditor::new();
    let ctrl_c = key(
        KeyCode::Character('c'),
        KeyModifiers::CONTROL,
        KeyAction::Press,
    );
    let ctrl_d = key(
        KeyCode::Character('d'),
        KeyModifiers::CONTROL,
        KeyAction::Press,
    );

    assert_eq!(
        editor.handle(ctrl_c, true, NOW),
        EditorEffect::InterruptTask
    );
    assert_eq!(editor.handle(ctrl_d, false, NOW), EditorEffect::Exit);
}

// 빈 paste도 연속 입력 사이에 오면 Ctrl+C 두 번 종료 조건을 끊는다.
#[test]
fn paste_breaks_the_empty_ctrl_c_sequence() {
    let mut editor = PromptEditor::new();
    let ctrl_c = key(
        KeyCode::Character('c'),
        KeyModifiers::CONTROL,
        KeyAction::Press,
    );

    assert_eq!(
        editor.handle(ctrl_c.clone(), false, NOW),
        EditorEffect::ExitArmed
    );
    assert_eq!(
        editor.handle(InputEvent::Paste(String::new()), false, NOW),
        EditorEffect::NoChange
    );
    assert_eq!(editor.handle(ctrl_c, false, NOW), EditorEffect::ExitArmed);
}

// 화면 크기 변화는 편집 내용을 바꾸거나 Ctrl+C 종료 준비를 취소하지 않는다.
#[test]
fn resize_is_not_an_editing_command() {
    let mut editor = PromptEditor::new();
    let ctrl_c = key(
        KeyCode::Character('c'),
        KeyModifiers::CONTROL,
        KeyAction::Press,
    );

    editor.handle(ctrl_c.clone(), false, NOW);
    assert_eq!(
        editor.handle(
            InputEvent::Resize(Size::new(80, 24)),
            false,
            NOW + Duration::from_millis(100)
        ),
        EditorEffect::Unhandled
    );
    assert_eq!(
        editor.handle(ctrl_c, false, NOW + Duration::from_millis(200)),
        EditorEffect::Exit
    );
}
