use std::time::Duration;

use yo_core::{
    SkillAvailability, SkillReference, SkillReferenceCandidate, SkillReferenceScope,
    SkillReferenceSearchStatus, SkillReferenceSearchUpdate,
};

use super::SkillReferenceAssist;
use crate::{
    appearance::AppearanceState,
    input::{
        editor::PromptEditor,
        event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
    },
    overlay::{OverlayBindings, OverlayInputEffect, PromptOverlaySlot, SelectionEntry},
    prompt::workspace_reference::WorkspaceEdit,
    surface::{CellContent, Point, Rect, Size, Surface},
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

fn panel_rows(overlay: &PromptOverlaySlot, width: u16) -> Vec<String> {
    let appearance = AppearanceState::default().pin().snapshot().styles().overlay;
    let prepared = overlay
        .panel()
        .unwrap()
        .prepare(
            Size::new(width, 8),
            appearance,
            &OverlayBindings::default(),
            false,
        )
        .unwrap();
    let size = prepared.size();
    let mut surface = Surface::new(size).unwrap();
    prepared
        .paint(&mut surface.view(Rect::new(Point::new(0, 0), size)).unwrap())
        .unwrap();
    (0..size.height)
        .map(|y| {
            (0..size.width)
                .filter_map(
                    |x| match surface.cell(Point::new(x, y)).unwrap().content() {
                        CellContent::Blank => Some(" ".to_owned()),
                        CellContent::Continuation { .. } => None,
                        CellContent::Grapheme { text, .. } => Some(text.to_string()),
                    },
                )
                .collect()
        })
        .collect()
}

// 같은 이름과 scope를 가진 두 skill도 좁은 pane에서 출처가 먼저 보여야 하며,
// 선택 결과는 표시명이 아니라 해당 row의 typed identity를 유지한다.
#[test]
fn same_name_skill_sources_stay_distinct_in_narrow_panel_and_selection() {
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
    let disabled = SkillReferenceCandidate::new(
        candidate("third", "review", SkillReferenceScope::Workspace)
            .reference()
            .clone(),
        "review",
        "Use review",
        SkillAvailability::Disabled("Disabled by policy".to_owned()),
    );
    assist.observe(
        SkillReferenceSearchUpdate::final_result(
            &request,
            SkillReferenceSearchStatus::Complete,
            vec![
                candidate("one", "review", SkillReferenceScope::Workspace),
                candidate("two", "review", SkillReferenceScope::Workspace),
                disabled,
            ],
        ),
        &mut overlay,
    );
    overlay.set_presented(true);
    assert_eq!(
        overlay.panel().unwrap().entries(),
        &[
            SelectionEntry::enabled_with_context(
                "one",
                "#1 one/SKILL.md · review · Use review",
                None,
                Some("Workspace".to_owned()),
            ),
            SelectionEntry::enabled_with_context(
                "two",
                "#2 two/SKILL.md · review · Use review",
                None,
                Some("Workspace".to_owned()),
            ),
            SelectionEntry::disabled(
                "third",
                "#3 third/SKILL.md · review · Use review",
                Some("Workspace".to_owned()),
                "Disabled by policy",
            ),
        ]
    );
    let rows = panel_rows(&overlay, 32);
    assert!(rows.iter().any(|row| row.contains("#1 one/SKI")));
    assert!(rows.iter().any(|row| row.contains("#2 two/SKI")));
    assert!(rows.iter().any(|row| row.contains("#3 th")), "{rows:?}");
    assert_eq!(
        overlay.handle(&key(KeyCode::Down)),
        OverlayInputEffect::Redraw
    );
    let OverlayInputEffect::Accepted(receipt) = overlay.handle(&key(KeyCode::Enter)) else {
        panic!("the second source should be selected");
    };
    assert_eq!(receipt.identity(), "two");
    assert!(assist.accept(&receipt, &mut editor));
    assert_eq!(
        assist.accepted_reference().unwrap().locator(),
        "/skills/two/SKILL.md"
    );
}

// 긴 공통 디렉터리 이름으로 시작해도 다른 부분이 좁은 pane의 앞쪽에 남는다.
#[test]
fn three_sources_and_long_names_keep_distinct_prefixes() {
    let mut editor = PromptEditor::new();
    editor.handle(
        InputEvent::Paste("$review-security".to_owned()),
        false,
        Duration::ZERO,
    );
    let mut overlay = PromptOverlaySlot::default();
    let mut assist = SkillReferenceAssist::default();
    assist.enable();
    let (request, _) = assist
        .prompt_changed(&editor, &mut overlay, None, true)
        .unwrap();
    let from = |identity, locator| {
        SkillReferenceCandidate::new(
            SkillReference::new(
                identity,
                "local-host:fixture",
                locator,
                "review-security",
                SkillReferenceScope::Workspace,
                1,
                "metadata:1",
            ),
            "review-security",
            "Use review-security",
            SkillAvailability::Enabled,
        )
    };
    assist.observe(
        SkillReferenceSearchUpdate::final_result(
            &request,
            SkillReferenceSearchStatus::Complete,
            vec![
                from("one", "/skills/verylongcommonone/SKILL.md"),
                from("two", "/skills/verylongcommontwo/SKILL.md"),
                from("three", "/skills/other/SKILL.md"),
            ],
        ),
        &mut overlay,
    );
    let rows = panel_rows(&overlay, 32);
    assert!(rows.iter().any(|row| row.contains("#1 …mmono")), "{rows:?}");
    assert!(rows.iter().any(|row| row.contains("#2 …mmont")), "{rows:?}");
    assert!(rows.iter().any(|row| row.contains("#3 other")), "{rows:?}");
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
    overlay.set_presented(true);
    let OverlayInputEffect::Accepted(receipt) = overlay.handle(&key(KeyCode::Enter)) else {
        panic!("the workspace result should remain selectable");
    };
    assert_eq!(receipt.identity(), "repo");
}

// 기존 skill 결과를 보여 주는 중 새 검색이 시작되면 Left/Right도 필터를 바꾸지 않고
// 소비한다. 따라서 새 응답 전에는 이전 목록과 선택이 남고 Enter 역시 accept하지 않는다.
#[test]
fn pending_replacement_blocks_filter_changes_until_matching_skill_results_arrive() {
    let mut editor = PromptEditor::new();
    editor.handle(
        InputEvent::Paste("$review".to_owned()),
        false,
        Duration::ZERO,
    );
    let mut overlay = PromptOverlaySlot::default();
    let mut assist = SkillReferenceAssist::default();
    assist.enable();
    let (initial, _) = assist
        .prompt_changed(&editor, &mut overlay, None, true)
        .unwrap();
    assist.observe(
        SkillReferenceSearchUpdate::final_result(
            &initial,
            SkillReferenceSearchStatus::Complete,
            vec![candidate("repo", "review", SkillReferenceScope::Workspace)],
        ),
        &mut overlay,
    );
    overlay.set_presented(true);

    let old = editor.text().to_owned();
    let old_cursor = editor.cursor_byte_index();
    editor.handle(InputEvent::Paste("x".to_owned()), false, Duration::ZERO);
    let edit = WorkspaceEdit::between(&old, old_cursor, editor.text(), editor.cursor_byte_index());
    let (replacement, refresh_catalog) = assist
        .prompt_changed(&editor, &mut overlay, edit.as_ref(), true)
        .unwrap();
    assert!(!refresh_catalog);

    assert_eq!(
        overlay.handle(&key(KeyCode::Right)),
        OverlayInputEffect::Consumed
    );
    assert_eq!(
        overlay
            .panel()
            .unwrap()
            .selected_identity()
            .unwrap()
            .as_str(),
        "repo"
    );
    assert_eq!(
        overlay.handle(&key(KeyCode::Enter)),
        OverlayInputEffect::Consumed
    );

    assist.observe(
        SkillReferenceSearchUpdate::final_result(
            &replacement,
            SkillReferenceSearchStatus::Complete,
            vec![candidate(
                "repo-x",
                "reviewx",
                SkillReferenceScope::Workspace,
            )],
        ),
        &mut overlay,
    );
    overlay.set_presented(true);
    let OverlayInputEffect::Accepted(receipt) = overlay.handle(&key(KeyCode::Enter)) else {
        panic!("matching replacement results must reopen acceptance");
    };
    assert_eq!(receipt.identity(), "repo-x");
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
