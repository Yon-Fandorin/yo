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

// 뒤쪽에 이미 선택한 참조가 있어도 앞쪽 후보를 긴 Unicode 경로로 바꾸면 두 신원과
// byte span을 함께 보존한다. 이전 제출 snapshot은 이후 편집으로 변하지 않는다.
#[test]
fn accepting_an_earlier_reference_preserves_later_selected_identity_and_snapshot() {
    use yo_core::{WorkspaceReference, WorkspaceReferenceCandidate, WorkspaceReferenceKind};
    let mut controller = PromptAssistController::default();
    controller.enable_workspace();
    let mut editor = PromptEditor::new();
    let mut overlay = PromptOverlaySlot::default();
    editor.handle(InputEvent::Paste("@a".to_owned()), false, Duration::ZERO);
    let mut old_snapshot = None;
    for (index, path) in ["src/a.rs", "src/한글 이름.rs"].into_iter().enumerate() {
        let edit = if index == 1 {
            old_snapshot = Some(controller.input(editor.text()).unwrap());
            editor.handle(
                InputEvent::Key(KeyEvent {
                    code: KeyCode::Character('a'),
                    modifiers: KeyModifiers::CONTROL,
                    action: KeyAction::Press,
                    state: KeyState::NONE,
                }),
                false,
                Duration::ZERO,
            );
            let before = editor.text().to_owned();
            let cursor = editor.cursor_byte_index();
            editor.handle(InputEvent::Paste("@b ".to_owned()), false, Duration::ZERO);
            let edit =
                WorkspaceEdit::between(&before, cursor, editor.text(), editor.cursor_byte_index());
            editor.handle(
                InputEvent::Key(KeyEvent {
                    code: KeyCode::Left,
                    modifiers: KeyModifiers::NONE,
                    action: KeyAction::Press,
                    state: KeyState::NONE,
                }),
                false,
                Duration::ZERO,
            );
            edit
        } else {
            None
        };
        let PromptAssistRequest::Workspace(request) = controller
            .prompt_changed(&editor, &mut overlay, edit.as_ref(), true)
            .unwrap()
        else {
            panic!("workspace request")
        };
        let reference = WorkspaceReference::new(
            format!("file:{index}"),
            "host:one",
            "workspace:one",
            "root:one",
            path,
            WorkspaceReferenceKind::File,
        )
        .unwrap();
        assert!(controller.observe_workspace(
            WorkspaceReferenceSearchUpdate::final_result(
                &request,
                WorkspaceReferenceSearchStatus::Complete,
                vec![WorkspaceReferenceCandidate::new(reference)]
            ),
            &mut overlay
        ));
        overlay.set_presented(true);
        let OverlayInputEffect::Accepted(receipt) = overlay.handle(&InputEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            action: KeyAction::Press,
            state: KeyState::NONE,
        })) else {
            panic!("candidate acceptance")
        };
        assert!(controller.accept(&receipt, &mut editor));
    }
    let input = controller.input(editor.text()).unwrap();
    assert_eq!(input.as_str(), "@src/한글 이름.rs @src/a.rs");
    assert_eq!(input.references().len(), 2);
    assert_eq!(
        input.references()[0]
            .workspace_reference()
            .unwrap()
            .relative_path(),
        "src/한글 이름.rs"
    );
    assert_eq!(
        input.references()[1]
            .workspace_reference()
            .unwrap()
            .relative_path(),
        "src/a.rs"
    );
    let old = old_snapshot.unwrap();
    assert_eq!(old.as_str(), "@src/a.rs");
    assert_eq!(old.references().len(), 1);
    controller.prompt_cleared(&mut overlay);
    assert!(
        controller
            .input(input.as_str())
            .unwrap()
            .references()
            .is_empty()
    );
    controller.restore_input(&input, &mut overlay);
    assert_eq!(controller.input(input.as_str()).unwrap(), input);
}
