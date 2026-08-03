use std::time::Duration;

use yo_core::{
    WorkspaceReference, WorkspaceReferenceCandidate, WorkspaceReferenceKind,
    WorkspaceReferenceSearchStatus, WorkspaceReferenceSearchUpdate,
};

use super::{
    AcceptedAnnotation, AssistTriggerKind, WorkspaceEdit, WorkspaceReferenceAssist,
    scan_prompt_trigger,
};
use crate::{
    input::{editor::PromptEditor, event::InputEvent},
    overlay::{OverlayInputEffect, PanelSnapshot, PromptOverlaySlot, SelectionEntry},
};

// 입력 시작이나 Unicode 공백 뒤의 @ 토큰만 전체 치환 범위와 query로 인식한다.
#[test]
fn scanner_finds_the_cursor_token_only_at_an_eligible_boundary() {
    let trigger = scan_prompt_trigger("질문 \t@src/ma 뒤", "질문 \t@src/ma".len()).unwrap();
    assert_eq!(trigger.kind, AssistTriggerKind::Workspace);
    assert_eq!(trigger.query, "src/ma");
    assert_eq!(&"질문 \t@src/ma 뒤"[trigger.span], "@src/ma");
    assert!(scan_prompt_trigger("mail@example.com", "mail@example".len()).is_none());
    assert_eq!(
        scan_prompt_trigger("use $rust", "use $rust".len())
            .unwrap()
            .kind,
        AssistTriggerKind::Skill
    );
}

// provider 결과를 고르면 @ 토큰 전체가 공백이 든 경로로 한 번에 바뀌고,
// 같은 철자의 typed annotation 위에서는 새 raw trigger가 다시 열리지 않는다.
#[test]
fn acceptance_replaces_the_whole_trigger_and_preserves_typed_identity() {
    let mut editor = PromptEditor::new();
    editor.handle(
        InputEvent::Paste("inspect @src/ma".to_owned()),
        false,
        Duration::ZERO,
    );
    let mut overlay = PromptOverlaySlot::default();
    let mut assist = WorkspaceReferenceAssist::default();
    assist.enable();
    let (request, show_loading) = assist
        .prompt_changed(&editor, &mut overlay, None, true)
        .unwrap();
    assert!(show_loading);
    let candidate = WorkspaceReferenceCandidate::new(
        WorkspaceReference::new(
            "path-1",
            "environment-1",
            "workspace-1",
            "root-1",
            "src/main file.rs",
            WorkspaceReferenceKind::Directory,
        )
        .unwrap(),
    );
    assert!(assist.observe(
        WorkspaceReferenceSearchUpdate::final_result(
            &request,
            WorkspaceReferenceSearchStatus::Complete,
            vec![candidate],
        ),
        &mut overlay,
    ));
    overlay.set_presented(true);
    let OverlayInputEffect::Accepted(receipt) =
        overlay.handle(&InputEvent::Key(crate::input::event::KeyEvent {
            code: crate::input::event::KeyCode::Enter,
            modifiers: crate::input::event::KeyModifiers::NONE,
            action: crate::input::event::KeyAction::Press,
            state: crate::input::event::KeyState::NONE,
        }))
    else {
        panic!("the enabled workspace row should accept");
    };

    assert!(assist.accept(&receipt, &mut editor));
    assert_eq!(editor.text(), "inspect @src/main file.rs/");
    assert!(
        assist
            .prompt_changed(&editor, &mut overlay, None, true)
            .is_none()
    );
    assert!(assist.has_accepted_references());
}

// annotation 시작·끝 경계의 삽입은 identity를 보존하고 span만 필요한 만큼 옮기지만,
// 내부 삽입은 보이는 text만 남긴 채 typed 의미를 제거한다.
#[test]
fn annotation_transforms_use_the_actual_editor_insertion_boundary() {
    let reference = WorkspaceReference::new(
        "path-1",
        "environment-1",
        "workspace-1",
        "root-1",
        "src",
        WorkspaceReferenceKind::Directory,
    )
    .unwrap();
    for (new, cursor, expected_span, retained) in [
        ("x !@src y", 2, 3..7, true),
        ("x @src! y", 6, 2..6, true),
        ("x @s!rc y", 4, 0..0, false),
    ] {
        let mut assist = WorkspaceReferenceAssist {
            last_text: "x @src y".to_owned(),
            accepted: vec![AcceptedAnnotation {
                span: 2..6,
                projection: "@src".to_owned(),
                reference: reference.clone(),
            }],
            ..WorkspaceReferenceAssist::default()
        };
        let new_cursor = if new.len() > "x @src y".len() {
            cursor + new.len() - "x @src y".len()
        } else {
            cursor
        };
        let edit = WorkspaceEdit::between("x @src y", cursor, new, new_cursor).unwrap();
        assist.transform_annotations(new, Some(&edit));
        assert_eq!(assist.accepted.len(), usize::from(retained));
        if retained {
            assert_eq!(assist.accepted[0].span, expected_span);
        }
    }
}

// 같은 글자가 경계 양쪽에 반복되어도 Backspace와 Delete의 실제 cursor 위치를 사용해
// annotation 밖 삭제는 보존하고 내부 삭제만 typed identity를 제거한다.
#[test]
fn annotation_deletions_follow_the_editor_cursor_in_repeated_text() {
    let reference = WorkspaceReference::new(
        "path-1",
        "environment-1",
        "workspace-1",
        "root-1",
        "aaa",
        WorkspaceReferenceKind::File,
    )
    .unwrap();
    for (new, old_cursor, new_cursor, expected_span, retained) in [
        ("a @aaa y", 2, 1, 2..6, true),
        ("aa @aaa ", 8, 8, 3..7, true),
        ("aa @aa y", 7, 6, 0..0, false),
        ("aa @aa y", 6, 6, 0..0, false),
    ] {
        let mut assist = WorkspaceReferenceAssist {
            last_text: "aa @aaa y".to_owned(),
            accepted: vec![AcceptedAnnotation {
                span: 3..7,
                projection: "@aaa".to_owned(),
                reference: reference.clone(),
            }],
            ..WorkspaceReferenceAssist::default()
        };
        let edit = WorkspaceEdit::between("aa @aaa y", old_cursor, new, new_cursor).unwrap();
        assist.transform_annotations(new, Some(&edit));
        assert_eq!(assist.accepted.len(), usize::from(retained));
        if retained {
            assert_eq!(assist.accepted[0].span, expected_span);
        }
    }
}

// 첫 trigger만 loading frame을 요청하고 연속 입력은 기존 화면을 유지한 채
// 새 결과가 도착할 때 한 번만 redraw하여 키 입력마다 panel이 깜빡이지 않게 한다.
#[test]
fn consecutive_queries_do_not_request_an_intermediate_loading_redraw() {
    let mut editor = PromptEditor::new();
    editor.handle(InputEvent::Paste("@s".to_owned()), false, Duration::ZERO);
    let mut overlay = PromptOverlaySlot::default();
    let mut assist = WorkspaceReferenceAssist::default();
    assist.enable();
    let (first_request, first_loading) = assist
        .prompt_changed(&editor, &mut overlay, None, true)
        .unwrap();
    assert!(first_loading);
    let candidate = WorkspaceReferenceCandidate::new(
        WorkspaceReference::new(
            "src",
            "environment",
            "workspace",
            "root",
            "src",
            WorkspaceReferenceKind::Directory,
        )
        .unwrap(),
    );
    assert!(assist.observe(
        WorkspaceReferenceSearchUpdate::final_result(
            &first_request,
            WorkspaceReferenceSearchStatus::Complete,
            vec![candidate],
        ),
        &mut overlay,
    ));
    let visible_results = overlay.panel().cloned();

    let old = editor.text().to_owned();
    let old_cursor = editor.cursor_byte_index();
    editor.handle(InputEvent::Paste("r".to_owned()), false, Duration::ZERO);
    let edit = WorkspaceEdit::between(&old, old_cursor, editor.text(), editor.cursor_byte_index());
    let (_, second_loading) = assist
        .prompt_changed(&editor, &mut overlay, edit.as_ref(), true)
        .unwrap();
    assert!(!second_loading);
    assert_eq!(overlay.panel(), visible_results.as_ref());
    overlay.set_presented(true);
    assert_eq!(
        overlay.handle(&InputEvent::Key(crate::input::event::KeyEvent {
            code: crate::input::event::KeyCode::Enter,
            modifiers: crate::input::event::KeyModifiers::NONE,
            action: crate::input::event::KeyAction::Press,
            state: crate::input::event::KeyState::NONE,
        })),
        OverlayInputEffect::Consumed
    );
    assert!(overlay.is_open());
}

// 늦게 도착한 workspace 결과는 그 사이 열린 다른 overlay의 선택 가능 상태를 바꾸지 않는다.
#[test]
fn stale_workspace_update_cannot_disable_a_replacement_overlay() {
    let mut editor = PromptEditor::new();
    editor.handle(InputEvent::Paste("@src".to_owned()), false, Duration::ZERO);
    let mut overlay = PromptOverlaySlot::default();
    let mut assist = WorkspaceReferenceAssist::default();
    assist.enable();
    let (request, _) = assist
        .prompt_changed(&editor, &mut overlay, None, true)
        .unwrap();
    overlay
        .open(
            PanelSnapshot::new(
                "Commands",
                vec![SelectionEntry::enabled("command", "Run command", None)],
            )
            .unwrap(),
        )
        .unwrap();
    overlay.set_presented(true);

    assert!(!assist.observe(
        WorkspaceReferenceSearchUpdate::final_result(
            &request,
            WorkspaceReferenceSearchStatus::Complete,
            Vec::new(),
        ),
        &mut overlay,
    ));
    assert!(matches!(
        overlay.handle(&InputEvent::Key(crate::input::event::KeyEvent {
            code: crate::input::event::KeyCode::Enter,
            modifiers: crate::input::event::KeyModifiers::NONE,
            action: crate::input::event::KeyAction::Press,
            state: crate::input::event::KeyState::NONE,
        })),
        OverlayInputEffect::Accepted(_)
    ));
}

// 파일명의 제어문자는 typed identity에는 그대로 남지만 입력창에는 안전한 가시 표기로 삽입된다.
#[test]
fn acceptance_projects_control_characters_safely() {
    let mut editor = PromptEditor::new();
    editor.handle(InputEvent::Paste("@bad".to_owned()), false, Duration::ZERO);
    let mut overlay = PromptOverlaySlot::default();
    let mut assist = WorkspaceReferenceAssist::default();
    assist.enable();
    let (request, _) = assist
        .prompt_changed(&editor, &mut overlay, None, true)
        .unwrap();
    let candidate = WorkspaceReferenceCandidate::new(
        WorkspaceReference::new(
            "control-path",
            "environment",
            "workspace",
            "root",
            "bad\nname.rs",
            WorkspaceReferenceKind::File,
        )
        .unwrap(),
    );
    assert!(assist.observe(
        WorkspaceReferenceSearchUpdate::final_result(
            &request,
            WorkspaceReferenceSearchStatus::Complete,
            vec![candidate],
        ),
        &mut overlay,
    ));
    overlay.set_presented(true);
    let OverlayInputEffect::Accepted(receipt) =
        overlay.handle(&InputEvent::Key(crate::input::event::KeyEvent {
            code: crate::input::event::KeyCode::Enter,
            modifiers: crate::input::event::KeyModifiers::NONE,
            action: crate::input::event::KeyAction::Press,
            state: crate::input::event::KeyState::NONE,
        }))
    else {
        panic!("the safe projection should remain selectable");
    };
    assert!(assist.accept(&receipt, &mut editor));
    assert_eq!(editor.text(), "@bad\\u{A}name.rs");
}

// Chat 입력을 받을 수 없는 상태에서는 trigger가 있어도 overlay를 열지 않는다.
#[test]
fn ineligible_prompt_slot_does_not_start_workspace_search() {
    let mut editor = PromptEditor::new();
    editor.handle(InputEvent::Paste("@src".to_owned()), false, Duration::ZERO);
    let mut overlay = PromptOverlaySlot::default();
    let mut assist = WorkspaceReferenceAssist::default();
    assist.enable();

    assert!(
        assist
            .prompt_changed(&editor, &mut overlay, None, false)
            .is_none()
    );
    assert!(!overlay.is_open());
}

// provider 오류는 제어문자를 안전하게 표시하고 terminal 상태가 된 뒤 반복 redraw하지 않는다.
#[test]
fn provider_failure_is_safe_and_only_redraws_once() {
    let mut editor = PromptEditor::new();
    editor.handle(InputEvent::Paste("@src".to_owned()), false, Duration::ZERO);
    let mut overlay = PromptOverlaySlot::default();
    let mut assist = WorkspaceReferenceAssist::default();
    assist.enable();
    assist
        .prompt_changed(&editor, &mut overlay, None, true)
        .unwrap();

    assert!(assist.provider_failed("bad\nprovider".to_owned(), &mut overlay));
    assert!(!assist.provider_failed("again".to_owned(), &mut overlay));
}
