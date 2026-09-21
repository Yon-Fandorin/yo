use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use yo_core::{
    InputImage, InputImageSnapshot, InputReference, SkillReference, SkillReferenceScope, UserInput,
    WorkspaceReference, WorkspaceReferenceKind, skill_reference_projection,
    workspace_reference_projection,
};

use super::*;
use crate::{
    input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
    overlay::{PanelSnapshot, SelectionEntry},
    runner::{ExternalEditorImportError, state::StateEffect},
};

fn key(code: KeyCode, modifiers: KeyModifiers, action: KeyAction) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code,
        modifiers,
        action,
        state: KeyState::NONE,
    })
}

fn ctrl_g(action: KeyAction) -> InputEvent {
    key(KeyCode::Character('g'), KeyModifiers::CONTROL, action)
}

fn request(state: &mut TuiState, text: &str) -> ExternalEditorSnapshot {
    state
        .handle(InputEvent::Paste(text.to_owned()), Duration::ZERO)
        .unwrap();
    request_current(state)
}

fn request_current(state: &mut TuiState) -> ExternalEditorSnapshot {
    assert_eq!(
        state
            .handle(ctrl_g(KeyAction::Press), Duration::ZERO)
            .unwrap(),
        StateEffect::Exit
    );
    state
        .external_editor_snapshot()
        .expect("external editor snapshot")
}

// Ctrl+G는 press 한 번만 소비하며, 외부 편집기 요청은 기존 초안을 지우지 않는다.
#[test]
fn ctrl_g_requests_exactly_on_press_and_keeps_draft() {
    let mut state = TuiState::new();
    let snapshot = request(&mut state, "draft");

    assert_eq!(snapshot.text(), "draft");
    assert_eq!(state.editor.text(), "draft");
    assert_eq!(
        state
            .handle(ctrl_g(KeyAction::Repeat), Duration::ZERO)
            .unwrap(),
        StateEffect::Unchanged
    );
    assert_eq!(
        state
            .handle(ctrl_g(KeyAction::Release), Duration::ZERO)
            .unwrap(),
        StateEffect::Unchanged
    );
    assert!(state.take_external_editor_request());
}

// Chat prompt overlay가 살아 있는 동안에는 Ctrl+G가 overlay나 draft를 우회하지 않는다.
#[test]
fn ctrl_g_is_ignored_while_prompt_overlay_is_open() {
    let mut state = TuiState::new();
    state
        .open_overlay(
            PanelSnapshot::new(
                "Commands",
                vec![SelectionEntry::enabled("one", "One", None)],
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        state
            .handle(ctrl_g(KeyAction::Press), Duration::ZERO)
            .unwrap(),
        StateEffect::Unchanged
    );
    assert!(state.external_editor_snapshot().is_none());
}

// 외부 편집 결과는 제출하지 않고 하나의 undo 단위로만 현재 draft를 교체한다.
#[test]
fn import_replaces_plain_draft_as_one_undo_without_submit() {
    let mut state = TuiState::new();
    let snapshot = request(&mut state, "before");

    state
        .import_external_editor_result(&snapshot, "after".to_owned())
        .unwrap();
    assert_eq!(state.editor.text(), "after");
    assert!(state.pending_submissions.is_empty());
    assert!(state.external_editor_snapshot().is_none());

    assert_eq!(
        state
            .handle(
                key(
                    KeyCode::Character('-'),
                    KeyModifiers::CONTROL,
                    KeyAction::Press,
                ),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor.text(), "before");
}

// 결과가 요청 이후의 더 새로운 draft에 도착하면 오래된 결과는 적용되지 않는다.
#[test]
fn stale_import_preserves_newer_draft() {
    let mut state = TuiState::new();
    let snapshot = request(&mut state, "before");
    state
        .handle(InputEvent::Paste(" newer".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state.import_external_editor_result(&snapshot, "external".to_owned()),
        Err(ExternalEditorImportError::StaleDraft)
    );
    assert_eq!(state.editor.text(), "before newer");
}

// 기존 typed reference의 바깥 삽입은 span을 옮겨 보존하고, span 내부 교체는 거부한다.
#[test]
fn import_maps_outside_reference_and_rejects_ambiguous_change() {
    let mut state = TuiState::new();
    let reference = WorkspaceReference::new(
        "workspace-file",
        "execution",
        "workspace",
        "root",
        "src/main.rs",
        WorkspaceReferenceKind::File,
    )
    .unwrap();
    let projection = workspace_reference_projection(&reference);
    let input = UserInput::with_references(
        projection.clone(),
        vec![InputReference::workspace(0..projection.len(), reference)],
    )
    .unwrap();
    state.editor.replace_range(0..0, &projection);
    state
        .prompt_assist
        .restore_input(&input, &mut state.overlay);
    let snapshot = request_current(&mut state);
    state
        .import_external_editor_result(&snapshot, format!("prefix {projection}"))
        .unwrap();
    let mapped = state.prompt_assist.input(state.editor.text()).unwrap();
    assert_eq!(mapped.references()[0].span(), &(7..7 + projection.len()));

    let snapshot = request_current(&mut state);
    assert_eq!(
        state.import_external_editor_result(&snapshot, "prefix @changed".to_owned()),
        Err(ExternalEditorImportError::AmbiguousAnnotations)
    );
    assert_eq!(state.editor.text(), format!("prefix {projection}"));
}

// Skill annotation도 표시 token을 다시 검색해 대체하지 않고 동일한 안전 경계를 따른다.
#[test]
fn import_rejects_skill_annotation_replacement() {
    let mut state = TuiState::new();
    let reference = SkillReference::new(
        "skill-id",
        "execution",
        "skill://review",
        "review",
        SkillReferenceScope::Workspace,
        1,
        "revision",
    );
    let projection = skill_reference_projection(&reference);
    let input = UserInput::with_references(
        projection.clone(),
        vec![InputReference::skill(0..projection.len(), reference)],
    )
    .unwrap();
    state.editor.replace_range(0..0, &projection);
    state
        .prompt_assist
        .restore_input(&input, &mut state.overlay);
    let snapshot = request_current(&mut state);

    assert_eq!(
        state.import_external_editor_result(&snapshot, "$changed".to_owned()),
        Err(ExternalEditorImportError::AmbiguousAnnotations)
    );
    assert_eq!(state.editor.text(), projection);
}

// 준비된 image occurrence도 marker 바깥 삽입에서만 이동하며 marker 교체는 거부한다.
#[test]
fn import_preserves_image_annotation_or_rejects_marker_edit() {
    let png = STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==")
        .unwrap();
    let image = InputImage::new(
        0..InputImage::PROJECTION.len(),
        10,
        InputImageSnapshot::new(1, 1, png).unwrap(),
    )
    .unwrap();
    let input = UserInput::new(InputImage::PROJECTION)
        .with_images(vec![image])
        .unwrap();
    let mut state = TuiState::new();
    state.editor.replace_range(0..0, InputImage::PROJECTION);
    state
        .prompt_assist
        .restore_input(&input, &mut state.overlay);
    let snapshot = request_current(&mut state);

    state
        .import_external_editor_result(&snapshot, "x[image]".to_owned())
        .unwrap();
    let mapped = state.prompt_assist.input(state.editor.text()).unwrap();
    assert_eq!(mapped.images()[0].span(), &(1..8));

    let snapshot = request_current(&mut state);
    assert_eq!(
        state.import_external_editor_result(&snapshot, "x[photo]".to_owned()),
        Err(ExternalEditorImportError::AmbiguousAnnotations)
    );
    assert_eq!(state.editor.text(), "x[image]");
}
