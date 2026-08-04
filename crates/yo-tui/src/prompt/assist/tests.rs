use std::time::Duration;

use yo_core::{
    SkillAvailability, SkillReference, SkillReferenceCandidate, SkillReferenceScope,
    SkillReferenceSearchStatus, SkillReferenceSearchUpdate, WorkspaceReferenceSearchStatus,
    WorkspaceReferenceSearchUpdate,
};

use super::{PromptAssistController, PromptAssistRequest};
use crate::{
    input::{
        editor::PromptEditor,
        event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
    },
    overlay::{OverlayInputEffect, PromptOverlaySlot},
    prompt::workspace_reference::WorkspaceEdit,
};

// 한 controller의 request ID와 editor revision은 @에서 $로 trigger 종류가 바뀌어도
// 계속 증가해, 이전 provider 결과가 새 overlay에 적용될 수 없다.
#[test]
fn one_controller_fences_updates_across_trigger_kind_changes() {
    let mut controller = PromptAssistController::default();
    controller.enable_workspace();
    controller.enable_skill();
    let mut editor = PromptEditor::new();
    let mut overlay = PromptOverlaySlot::default();
    editor.handle(InputEvent::Paste("@src".to_owned()), false, Duration::ZERO);
    let PromptAssistRequest::Workspace(workspace) = controller
        .prompt_changed(&editor, &mut overlay, None, true)
        .unwrap()
    else {
        panic!("@ should create a workspace request");
    };

    let old = editor.text().to_owned();
    let old_cursor = editor.cursor_byte_index();
    editor.replace_range(0..old.len(), "$review");
    let edit = WorkspaceEdit::between(&old, old_cursor, editor.text(), editor.cursor_byte_index());
    let PromptAssistRequest::Skill(skill) = controller
        .prompt_changed(&editor, &mut overlay, edit.as_ref(), true)
        .unwrap()
    else {
        panic!("$ should create a skill request");
    };

    assert!(skill.request_id() > workspace.request_id());
    assert!(skill.editor_revision() > workspace.editor_revision());
    assert!(!controller.observe_workspace(
        WorkspaceReferenceSearchUpdate::final_result(
            &workspace,
            WorkspaceReferenceSearchStatus::Complete,
            Vec::new(),
        ),
        &mut overlay,
    ));
}

// accept된 `$name` 자체에 cursor가 있어도 같은 span을 raw trigger로 다시 열지 않아,
// cardinality 오류는 별도의 두 번째 token에만 사용된다.
#[test]
fn accepted_skill_span_is_not_rescanned_into_an_overlay() {
    let mut controller = PromptAssistController::default();
    controller.enable_skill();
    let mut editor = PromptEditor::new();
    editor.handle(
        InputEvent::Paste("$review".to_owned()),
        false,
        Duration::ZERO,
    );
    let mut overlay = PromptOverlaySlot::default();
    let PromptAssistRequest::Skill(request) = controller
        .prompt_changed(&editor, &mut overlay, None, true)
        .unwrap()
    else {
        panic!("$ should create a skill request");
    };
    let reference = SkillReference::new(
        "review-id",
        "local-host:fixture",
        "/skills/review/SKILL.md",
        "review",
        SkillReferenceScope::User,
        2,
        "sha256:exact",
    );
    assert!(controller.observe_skill(
        SkillReferenceSearchUpdate::final_result(
            &request,
            SkillReferenceSearchStatus::Complete,
            vec![SkillReferenceCandidate::new(
                reference,
                "review",
                "Review changes",
                SkillAvailability::Enabled,
            )],
        ),
        &mut overlay,
    ));
    overlay.set_presented(true);
    let OverlayInputEffect::Accepted(receipt) = overlay.handle(&InputEvent::Key(KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::NONE,
        action: KeyAction::Press,
        state: KeyState::NONE,
    })) else {
        panic!("the enabled skill should accept");
    };
    assert!(controller.accept(&receipt, &mut editor));

    assert!(
        controller
            .prompt_changed(&editor, &mut overlay, None, true)
            .is_none()
    );
    assert!(!overlay.is_open());
}
