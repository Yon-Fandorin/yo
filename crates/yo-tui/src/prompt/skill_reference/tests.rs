use std::time::Duration;

use yo_core::{
    SkillAvailability, SkillReference, SkillReferenceCandidate, SkillReferenceScope,
    SkillReferenceSearchStatus, SkillReferenceSearchUpdate,
};

use super::SkillReferenceAssist;
use crate::{
    input::{
        editor::PromptEditor,
        event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
    },
    overlay::{OverlayInputEffect, PromptOverlaySlot},
    prompt::workspace_reference::WorkspaceEdit,
};

fn candidate(identity: &str, name: &str, scope: SkillReferenceScope) -> SkillReferenceCandidate {
    SkillReferenceCandidate::new(
        SkillReference::new(
            identity,
            "local-host:fixture",
            format!("/skills/{identity}/SKILL.md"),
            name,
            scope,
            1,
            "metadata:1",
        ),
        name,
        format!("Use {name}"),
        SkillAvailability::Enabled,
    )
}

fn key(code: KeyCode) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        action: KeyAction::Press,
        state: KeyState::NONE,
    })
}

// 좌우 키는 provider를 다시 호출하지 않고 이미 받은 후보를 scope별로 좁히며,
// 선택 가능한 행도 현재 필터에 포함된 identity로 함께 갱신한다.
#[test]
fn left_and_right_cycle_provenance_filters_over_cached_candidates() {
    let mut editor = PromptEditor::new();
    editor.handle(
        InputEvent::Paste("$review".to_owned()),
        false,
        Duration::ZERO,
    );
    let mut overlay = PromptOverlaySlot::default();
    let mut assist = SkillReferenceAssist::default();
    assist.enable();
    let (request, _) = assist
        .prompt_changed(&editor, &mut overlay, None, true)
        .unwrap();
    assert!(request.refresh_catalog());
    assist.observe(
        SkillReferenceSearchUpdate::final_result(
            &request,
            SkillReferenceSearchStatus::Complete,
            vec![
                candidate("repo", "review", SkillReferenceScope::Workspace),
                candidate("user", "review", SkillReferenceScope::User),
            ],
        ),
        &mut overlay,
    );
    overlay.set_presented(true);

    let OverlayInputEffect::FilterChanged(index) = overlay.handle(&key(KeyCode::Right)) else {
        panic!("right should select the next provenance filter");
    };
    assert_eq!(index, 1);
    assert!(assist.filter_changed(index, &mut overlay));
    let OverlayInputEffect::Accepted(receipt) = overlay.handle(&key(KeyCode::Enter)) else {
        panic!("the workspace result should remain selectable");
    };
    assert_eq!(receipt.identity(), "repo");
}

// 선택은 보이는 `$name`만 편집기에 투영하되 typed identity를 별도로 보존하고,
// V1의 두 번째 명시적 skill trigger는 열지 않는다.
#[test]
fn acceptance_keeps_typed_identity_and_enforces_one_skill_in_v1() {
    let mut editor = PromptEditor::new();
    editor.handle(
        InputEvent::Paste("use $review".to_owned()),
        false,
        Duration::ZERO,
    );
    let mut overlay = PromptOverlaySlot::default();
    let mut assist = SkillReferenceAssist::default();
    assist.enable();
    let (request, _) = assist
        .prompt_changed(&editor, &mut overlay, None, true)
        .unwrap();
    assist.observe(
        SkillReferenceSearchUpdate::final_result(
            &request,
            SkillReferenceSearchStatus::Complete,
            vec![candidate("repo", "review", SkillReferenceScope::Workspace)],
        ),
        &mut overlay,
    );
    overlay.set_presented(true);
    let OverlayInputEffect::Accepted(receipt) = overlay.handle(&key(KeyCode::Enter)) else {
        panic!("the enabled skill should accept");
    };

    assert!(assist.accept(&receipt, &mut editor));
    assert_eq!(editor.text(), "use $review");
    assert!(assist.has_accepted_reference());
    let accepted = assist.accepted_reference().unwrap();
    assert_eq!(accepted.identity(), "repo");
    assert_eq!(
        accepted.execution_environment_identity(),
        "local-host:fixture"
    );
    assert_eq!(accepted.locator(), "/skills/repo/SKILL.md");
    assert_eq!(accepted.scope(), SkillReferenceScope::Workspace);
    assert_eq!(accepted.catalog_generation(), 1);
    assert_eq!(accepted.entry_revision(), "metadata:1");
    let old = editor.text().to_owned();
    let old_cursor = editor.cursor_byte_index();
    editor.handle(
        InputEvent::Paste(" $other".to_owned()),
        false,
        Duration::ZERO,
    );
    let edit = WorkspaceEdit::between(&old, old_cursor, editor.text(), editor.cursor_byte_index());
    assert!(
        assist
            .prompt_changed(&editor, &mut overlay, edit.as_ref(), true)
            .is_none()
    );
}

// provider가 실패 상태와 함께 stale 후보를 실어 보내도 후보를 모두 버리고 acceptance를
// 끄므로, 실패한 catalog snapshot에서 typed reference를 만들 수 없다.
#[test]
fn failed_update_never_accepts_attached_candidates() {
    let mut editor = PromptEditor::new();
    editor.handle(
        InputEvent::Paste("$review".to_owned()),
        false,
        Duration::ZERO,
    );
    let mut overlay = PromptOverlaySlot::default();
    let mut assist = SkillReferenceAssist::default();
    assist.enable();
    let (request, _) = assist
        .prompt_changed(&editor, &mut overlay, None, true)
        .unwrap();
    assist.observe(
        SkillReferenceSearchUpdate::final_result(
            &request,
            SkillReferenceSearchStatus::Failed("catalog unavailable".to_owned()),
            vec![candidate("stale", "review", SkillReferenceScope::User)],
        ),
        &mut overlay,
    );
    overlay.set_presented(true);

    assert_eq!(
        overlay.handle(&key(KeyCode::Enter)),
        OverlayInputEffect::Consumed
    );
    assert!(!assist.has_accepted_reference());
}

// 비활성 skill은 이유와 함께 목록에 남지만 Enter가 typed selection을 만들 수 없다.
#[test]
fn disabled_skill_is_visible_but_not_acceptable() {
    let mut editor = PromptEditor::new();
    editor.handle(InputEvent::Paste("$off".to_owned()), false, Duration::ZERO);
    let mut overlay = PromptOverlaySlot::default();
    let mut assist = SkillReferenceAssist::default();
    assist.enable();
    let (request, _) = assist
        .prompt_changed(&editor, &mut overlay, None, true)
        .unwrap();
    let disabled = SkillReferenceCandidate::new(
        SkillReference::new(
            "off",
            "local-codex:/workspace",
            "/skills/off/SKILL.md",
            "off",
            SkillReferenceScope::User,
            1,
            "metadata:1",
        ),
        "off",
        "Unavailable skill",
        SkillAvailability::Disabled("Disabled by Codex configuration".to_owned()),
    );
    assist.observe(
        SkillReferenceSearchUpdate::final_result(
            &request,
            SkillReferenceSearchStatus::Complete,
            vec![disabled],
        ),
        &mut overlay,
    );
    overlay.set_presented(true);

    assert_eq!(
        overlay.handle(&key(KeyCode::Enter)),
        OverlayInputEffect::Consumed
    );
    assert!(!assist.has_accepted_reference());
}
