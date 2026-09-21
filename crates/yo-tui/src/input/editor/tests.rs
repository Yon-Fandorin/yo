use std::{num::NonZeroU16, time::Duration};

use super::{
    EditorEffect, MAX_UNDO_BYTES, MAX_UNDO_STATES, PromptEditor,
    binding::{NewlineBinding, NewlineBindingError},
};
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

fn undo() -> InputEvent {
    key(
        KeyCode::Character('7'),
        KeyModifiers::CONTROL,
        KeyAction::Press,
    )
}

// 단어 입력은 공백과 다음 단어를 한 단위로 묶고 커서를 복원하며 legacy Ctrl+7을 수용한다.
#[test]
fn undo_restores_word_boundaries_and_cursor() {
    let mut editor = PromptEditor::new();
    for character in "가나 🦀".chars() {
        editor.handle(press(KeyCode::Character(character)), false, NOW);
    }
    assert_eq!(editor.text(), "가나 🦀");
    assert_eq!(
        editor.handle(undo(), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(editor.text(), "가나");
    assert_eq!(editor.cursor_byte_index(), "가나".len());
    assert_eq!(
        editor.handle(
            key(
                KeyCode::Character('-'),
                KeyModifiers::CONTROL,
                KeyAction::Press,
            ),
            false,
            NOW,
        ),
        EditorEffect::BufferChanged
    );
    assert!(editor.text().is_empty());
    assert_eq!(editor.cursor_byte_index(), 0);
    assert_eq!(editor.handle(undo(), false, NOW), EditorEffect::NoChange);
}

// 붙여넣기·한 글자 삭제·Ctrl+C 초안 삭제는 각각 복원하고 이동은 입력 묶음을 끊는다.
#[test]
fn undo_recovers_paste_delete_and_cleared_multiline_draft() {
    let mut editor = PromptEditor::new();
    let source = "첫 줄\n가👨‍👩‍👧";
    editor.handle(InputEvent::Paste(source.to_owned()), false, NOW);
    editor.handle(press(KeyCode::Backspace), false, NOW);
    assert_eq!(editor.text(), "첫 줄\n가");
    assert_eq!(
        editor.handle(undo(), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(editor.text(), source);
    assert_eq!(editor.cursor_byte_index(), source.len());
    editor.handle(
        key(
            KeyCode::Character('a'),
            KeyModifiers::CONTROL,
            KeyAction::Press,
        ),
        false,
        NOW,
    );
    let line_start = "첫 줄\n".len();
    assert_eq!(editor.cursor_byte_index(), line_start);
    editor.handle(
        key(
            KeyCode::Character('c'),
            KeyModifiers::CONTROL,
            KeyAction::Press,
        ),
        false,
        NOW,
    );
    assert!(editor.text().is_empty());
    assert_eq!(
        editor.handle(undo(), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(editor.text(), source);
    assert_eq!(editor.cursor_byte_index(), line_start);
    assert_eq!(
        editor.handle(undo(), false, NOW),
        EditorEffect::BufferChanged
    );
    assert!(editor.text().is_empty());
}

// 사용자 완료 치환만 실행 취소하며 제출 뒤와 프로그램이 교체한 초안은 이전 기록을 버린다.
#[test]
fn undo_completion_does_not_cross_programmatic_or_submit_boundary() {
    let mut editor = PromptEditor::new();
    editor.handle(InputEvent::Paste("@src".to_owned()), false, NOW);
    editor.replace_range_undoable(0..4, "@src/main.rs");
    assert_eq!(
        editor.handle(undo(), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(editor.text(), "@src");
    editor.replace_range(0..4, "new draft");
    assert_eq!(editor.handle(undo(), false, NOW), EditorEffect::NoChange);
    assert_eq!(
        editor.handle(press(KeyCode::Enter), false, NOW),
        EditorEffect::Submitted("new draft".to_owned())
    );
    assert_eq!(editor.handle(undo(), false, NOW), EditorEffect::NoChange);
}

// 최대 상태 수 뒤의 첫 추가 편집은 가장 오래된 기록만 버리고 최신 64개를 복원한다.
#[test]
fn undo_evicts_first_excess_state() {
    let mut editor = PromptEditor::new();
    for _ in 0..=MAX_UNDO_STATES {
        editor.handle(InputEvent::Paste("x".to_owned()), false, NOW);
    }
    for _ in 0..MAX_UNDO_STATES {
        assert_eq!(
            editor.handle(undo(), false, NOW),
            EditorEffect::BufferChanged
        );
    }
    assert_eq!(editor.text(), "x");
    assert_eq!(editor.handle(undo(), false, NOW), EditorEffect::NoChange);
}

// 저장할 이전 상태가 한도를 첫 초과하면 오래된 실행 취소가 새 편집에 잘못 적용되지 않는다.
#[test]
fn undo_clears_history_on_first_excess_source_byte() {
    let mut editor = PromptEditor::new();
    editor.handle(InputEvent::Paste("x".repeat(MAX_UNDO_BYTES)), false, NOW);
    editor.handle(press(KeyCode::Backspace), false, NOW);
    assert_eq!(
        editor.handle(undo(), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(editor.text().len(), MAX_UNDO_BYTES);
    editor.handle(press(KeyCode::Character('y')), false, NOW);
    editor.handle(press(KeyCode::Backspace), false, NOW);
    assert_eq!(editor.handle(undo(), false, NOW), EditorEffect::NoChange);
}

// 위아래 이동은 실제 줄과 자동 줄바꿈을 따르며 짧은 줄을 거쳐도 원래 셀 열을 복원한다.
#[test]
fn vertical_navigation_preserves_preferred_cell_column() {
    let mut editor = PromptEditor::new();
    editor.set_layout_width(NonZeroU16::new(8).unwrap());
    editor.handle(InputEvent::Paste("abcdef\nx\nabcdef".into()), false, NOW);
    assert_eq!(editor.cursor_byte_index(), editor.text().len());

    for (code, expected) in [
        (KeyCode::Up, "abcdef\nx".len()),
        (KeyCode::Up, "abcdef".len()),
        (KeyCode::Down, "abcdef\nx".len()),
        (KeyCode::Down, "abcdef\nx\nabcdef".len()),
    ] {
        assert_eq!(
            editor.handle(press(code), false, NOW),
            EditorEffect::BufferChanged
        );
        assert_eq!(editor.cursor_byte_index(), expected);
        assert_eq!(
            editor.handle(
                key(code, KeyModifiers::NONE, KeyAction::Release),
                false,
                NOW
            ),
            EditorEffect::Unhandled
        );
    }
    assert_eq!(
        editor.handle(press(KeyCode::Down), false, NOW),
        EditorEffect::NoChange
    );
    assert_eq!(editor.text(), "abcdef\nx\nabcdef");
}

// 하나의 탭이나 제어 표기가 여러 화면 줄을 차지해도 커서를 놓을 수 없는 중간 줄에서 멈추지 않는다.
#[test]
fn vertical_navigation_skips_rows_inside_one_expanded_grapheme() {
    for source in ["\tX", "\u{0085}X"] {
        let mut editor = PromptEditor::new();
        editor.set_layout_width(NonZeroU16::new(2).unwrap());
        editor.handle(InputEvent::Paste(source.into()), false, NOW);
        assert_eq!(
            editor.handle(press(KeyCode::Up), false, NOW),
            EditorEffect::BufferChanged
        );
        assert_eq!(editor.cursor_byte_index(), 0, "{source:?}");
        assert_eq!(
            editor.handle(press(KeyCode::Down), false, NOW),
            EditorEffect::BufferChanged
        );
        assert_eq!(editor.cursor_byte_index(), source.len(), "{source:?}");
    }
}

// 좁은 폭에서 접힌 한 줄도 여러 화면 줄로 이동하고, 폭 변경은 이전 선호 열을 버린다.
#[test]
fn vertical_navigation_uses_wrapped_rows_and_current_width() {
    let mut editor = PromptEditor::new();
    editor.set_layout_width(NonZeroU16::new(4).unwrap());
    editor.handle(InputEvent::Paste("abcdefghij".into()), false, NOW);
    assert_eq!(
        editor.handle(press(KeyCode::Up), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(editor.cursor_byte_index(), 6);
    assert_eq!(
        editor.handle(press(KeyCode::Up), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(editor.cursor_byte_index(), 2);
    editor.set_layout_width(NonZeroU16::new(5).unwrap());
    assert_eq!(
        editor.handle(press(KeyCode::Down), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(editor.cursor_byte_index(), 7);
}

// 한글·결합 문자·이모지를 통과해도 커서는 grapheme 경계에만 놓이고 원문은 그대로다.
#[test]
fn vertical_navigation_preserves_unicode_grapheme_boundaries() {
    let source = "ab\n👨‍👩‍👧c\n가e\u{301}";
    let mut editor = PromptEditor::new();
    editor.set_layout_width(NonZeroU16::new(8).unwrap());
    editor.handle(InputEvent::Paste(source.into()), false, NOW);
    for (code, expected) in [
        (KeyCode::Up, "ab\n👨‍👩‍👧c".len()),
        (KeyCode::Up, "ab".len()),
        (KeyCode::Down, "ab\n👨‍👩‍👧c".len()),
        (KeyCode::Down, source.len()),
    ] {
        assert_eq!(
            editor.handle(press(code), false, NOW),
            EditorEffect::BufferChanged
        );
        assert_eq!(editor.cursor_byte_index(), expected);
    }
    assert_eq!(editor.text(), source);
}

// 단어 삭제는 공백·줄바꿈을 넘되 결합 문자와 이모지를 쪼개지 않으며 연속 삭제를 복원한다.
#[test]
fn word_editing_preserves_unicode_and_yanks_consecutive_deletions() {
    let mut editor = PromptEditor::new();
    let source = "first e\u{301} 👨‍👩‍👧\n한글  ";
    editor.handle(InputEvent::Paste(source.into()), false, NOW);
    for expected in ["first e\u{301} 👨‍👩‍👧\n", "first e\u{301} "] {
        assert_eq!(
            editor.handle(
                key(
                    KeyCode::Character('w'),
                    KeyModifiers::CONTROL,
                    KeyAction::Press
                ),
                false,
                NOW
            ),
            EditorEffect::BufferChanged
        );
        assert_eq!(editor.text(), expected);
    }
    editor.handle(
        key(
            KeyCode::Character('y'),
            KeyModifiers::CONTROL,
            KeyAction::Press,
        ),
        false,
        NOW,
    );
    assert_eq!(editor.text(), source);
    for (code, prefix) in [
        (KeyCode::Left, "first e\u{301} 👨‍👩‍👧\n"),
        (KeyCode::Left, "first e\u{301} "),
        (KeyCode::Right, "first e\u{301} 👨‍👩‍👧"),
        (KeyCode::Right, "first e\u{301} 👨‍👩‍👧\n한글"),
    ] {
        assert_eq!(
            editor.handle(
                key(code, KeyModifiers::CONTROL, KeyAction::Press),
                false,
                NOW
            ),
            EditorEffect::BufferChanged
        );
        assert_eq!(editor.cursor_byte_index(), prefix.len());
        assert_eq!(editor.text(), source);
    }
    assert_eq!(
        editor.handle(
            key(
                KeyCode::Character('w'),
                KeyModifiers::CONTROL,
                KeyAction::Release
            ),
            false,
            NOW
        ),
        EditorEffect::Unhandled
    );
    assert_eq!(editor.text(), source);
}

// 빈 입력의 단어 편집은 종료·제출하지 않고 공백만 있는 입력도 안전하게 이동·삭제한다.
#[test]
fn word_editing_handles_empty_and_whitespace_only_input() {
    let mut editor = PromptEditor::new();
    for code in [KeyCode::Left, KeyCode::Right, KeyCode::Character('w')] {
        assert_eq!(
            editor.handle(
                key(code, KeyModifiers::CONTROL, KeyAction::Press),
                false,
                NOW
            ),
            EditorEffect::NoChange
        );
    }
    editor.handle(InputEvent::Paste(" \t\n\u{3000}".into()), false, NOW);
    editor.handle(
        key(KeyCode::Left, KeyModifiers::CONTROL, KeyAction::Press),
        false,
        NOW,
    );
    assert_eq!(editor.cursor_byte_index(), 0);
    editor.handle(
        key(KeyCode::Right, KeyModifiers::CONTROL, KeyAction::Press),
        false,
        NOW,
    );
    assert_eq!(editor.cursor_byte_index(), editor.text().len());
    editor.handle(
        key(
            KeyCode::Character('w'),
            KeyModifiers::CONTROL,
            KeyAction::Press,
        ),
        false,
        NOW,
    );
    assert!(editor.text().is_empty());
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

// 줄바꿈 modifier는 내부 바인딩으로 교체할 수 있고 기본 Shift는 더 이상 가로채지 않는다.
#[test]
fn custom_newline_binding_replaces_shift() {
    let binding = NewlineBinding::new(KeyModifiers::ALT).unwrap();
    let mut editor = PromptEditor::with_newline_binding(binding);
    editor.handle(InputEvent::Paste("첫 줄".into()), false, NOW);

    assert_eq!(
        editor.handle(
            key(KeyCode::Enter, KeyModifiers::SHIFT, KeyAction::Press),
            false,
            NOW
        ),
        EditorEffect::Unhandled
    );
    assert_eq!(
        editor.handle(
            key(KeyCode::Enter, KeyModifiers::ALT, KeyAction::Press),
            false,
            NOW
        ),
        EditorEffect::BufferChanged
    );
    assert_eq!(editor.text(), "첫 줄\n");
}

// modifier 없는 Enter는 제출 전용이므로 줄바꿈 바인딩으로 설정할 수 없다.
#[test]
fn newline_binding_cannot_replace_plain_enter_submission() {
    assert_eq!(
        NewlineBinding::new(KeyModifiers::NONE),
        Err(NewlineBindingError::ConflictsWithSubmit)
    );
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

// 줄 편집은 실제 개행을 기준으로 하며 한글·결합 문자·이모지를 쪼개지 않는다.
// 연속 삭제를 방향에 맞게 합쳐 Ctrl+Y로 복원하고 release·추가 modifier는 편집하지 않는다.
#[test]
fn line_editing_preserves_unicode_and_restores_consecutive_kills() {
    let mut editor = PromptEditor::new();
    let original = "첫줄\n가e\u{301}👨‍👩‍👧뒤\n마지막";
    editor.handle(InputEvent::Paste(original.into()), false, NOW);
    let prefix = "첫줄\n가e\u{301}👨‍👩‍👧";
    while editor.cursor_byte_index() > prefix.len() {
        editor.handle(press(KeyCode::Left), false, NOW);
    }
    let control = |character| {
        key(
            KeyCode::Character(character),
            KeyModifiers::CONTROL,
            KeyAction::Press,
        )
    };
    assert_eq!(
        editor.handle(control('u'), false, NOW),
        EditorEffect::BufferChanged
    );
    assert_eq!(editor.text(), "첫줄\n뒤\n마지막");
    assert_eq!(editor.cursor_byte_index(), "첫줄\n".len());
    editor.handle(control('y'), false, NOW);
    assert_eq!(editor.text(), original);
    assert_eq!(editor.cursor_byte_index(), prefix.len());
    for _ in 0..3 {
        editor.handle(control('k'), false, NOW);
    }
    assert_eq!(editor.text(), prefix);
    assert_eq!(
        editor.handle(control('k'), false, NOW),
        EditorEffect::NoChange
    );
    editor.handle(control('y'), false, NOW);
    assert_eq!(editor.text(), original);
    editor.handle(control('a'), false, NOW);
    assert_eq!(editor.cursor_byte_index(), "첫줄\n가e\u{301}👨‍👩‍👧뒤\n".len());
    editor.handle(control('u'), false, NOW);
    assert_eq!(editor.text(), "첫줄\n가e\u{301}👨‍👩‍👧뒤마지막");
    editor.handle(control('u'), false, NOW);
    assert_eq!(editor.text(), "첫줄\n마지막");
    editor.handle(control('y'), false, NOW);
    assert_eq!(editor.text(), original);
    editor.handle(control('e'), false, NOW);
    assert_eq!(editor.cursor_byte_index(), original.len());
    for input in [
        key(
            KeyCode::Character('u'),
            KeyModifiers::CONTROL,
            KeyAction::Release,
        ),
        key(
            KeyCode::Character('k'),
            KeyModifiers::CONTROL.union(KeyModifiers::ALT),
            KeyAction::Press,
        ),
    ] {
        assert_eq!(editor.handle(input, false, NOW), EditorEffect::Unhandled);
        assert_eq!(editor.text(), original);
    }
}

// CRLF 줄 경계는 하나의 grapheme으로 제거하며 화면 폭 변경이 논리 줄 삭제 범위를 바꾸지 않는다.
#[test]
fn line_edits_keep_crlf_atomic_and_ignore_visual_wrapping() {
    for width in [24, 80] {
        let mut editor = PromptEditor::new();
        let line = "가".repeat(60);
        editor.handle(
            InputEvent::Paste(format!("before\r\n{line}\r\nafter")),
            false,
            NOW,
        );
        let control = |character| {
            key(
                KeyCode::Character(character),
                KeyModifiers::CONTROL,
                KeyAction::Press,
            )
        };
        editor.handle(control('a'), false, NOW);
        editor.handle(control('u'), false, NOW);
        assert_eq!(editor.text(), format!("before\r\n{line}after"));
        editor.handle(control('y'), false, NOW);
        assert_eq!(editor.text(), format!("before\r\n{line}\r\nafter"));
        editor.handle(press(KeyCode::Left), false, NOW);
        // Cursor moves over the whole CRLF to the preceding line's end.
        assert_eq!(editor.cursor_byte_index(), "before\r\n".len() + line.len());
        editor.layout(NonZeroU16::new(width).unwrap()).unwrap();
        editor.handle(control('u'), false, NOW);
        assert_eq!(editor.text(), "before\r\n\r\nafter");
        editor.handle(control('y'), false, NOW);
        assert_eq!(editor.text(), format!("before\r\n{line}\r\nafter"));
        editor.handle(control('k'), false, NOW);
        assert_eq!(editor.text(), format!("before\r\n{line}after"));
    }
}
