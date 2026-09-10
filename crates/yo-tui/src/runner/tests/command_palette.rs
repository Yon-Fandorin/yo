use std::time::Duration;

use yo_core::{
    AgentEvent, JournalDurability, SubmissionOutcome, SubmissionRejection, SubmissionRejectionKind,
    session_repository::RepositorySequence,
};

use super::{key, rendered_row, turn};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyCode, KeyModifiers},
    overlay::{PanelSnapshot, SelectionEntry, SlotError},
    runner::{
        AgentAction,
        state::{StateEffect, TuiState},
    },
    surface::Size,
};

fn present_palette(state: &mut TuiState, size: Size) -> String {
    let frame = state
        .prepare_frame(size, &AppearanceState::default().pin())
        .unwrap();
    assert!(frame.overlay_presented);
    state.commit_frame(&frame);
    (0..size.height)
        .map(|row| rendered_row(state, size, row))
        .collect::<Vec<_>>()
        .join("\n")
}

// 첫 slash 입력은 agent 제출 없이 prompt 위에 현재 로컬 명령 전체를 표시한다.
#[test]
fn slash_opens_the_local_command_palette() {
    let mut state = TuiState::new();
    assert_eq!(
        state
            .handle(InputEvent::Paste("/".to_owned()), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );

    let rendered = present_palette(&mut state, Size::new(80, 16));
    assert!(rendered.contains("Commands"), "{rendered}");
    assert!(rendered.contains("/help"), "{rendered}");
    assert!(rendered.contains("/model"), "{rendered}");
    assert!(rendered.contains("/compact"), "{rendered}");
    assert!(rendered.contains("/exit"), "{rendered}");
    assert_eq!(state.editor().text(), "/");
    assert!(state.transcript().items().is_empty());
}

// 선택 지침이 있는 `/compact`가 idle control intent 하나로 정확히 변환됨을 검증합니다.
#[test]
fn compact_with_guidance_dispatches_one_idle_control_intent() {
    let mut state = TuiState::new();
    state
        .handle(
            InputEvent::Paste("/compact preserve unresolved constraints".to_owned()),
            Duration::ZERO,
        )
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::CompactContext {
            guidance: Some("preserve unresolved constraints".to_owned()),
        })
    );
    assert!(state.editor().text().is_empty());
}

// slash 접두어는 일치하는 명령만 남기고 editor draft와 cursor 소유권을 유지한다.
#[test]
fn slash_query_filters_the_palette() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/m".to_owned()), Duration::ZERO)
        .unwrap();

    let rendered = present_palette(&mut state, Size::new(80, 16));
    assert!(rendered.contains("/model"), "{rendered}");
    assert!(!rendered.contains("/help"), "{rendered}");
    assert!(!rendered.contains("/exit"), "{rendered}");
    assert_eq!(state.editor().text(), "/m");
}

// 이미 보인 palette를 query로 좁힌 직후 redraw 전 Enter가 와도 새 선택을 실행하거나
// 부분 command를 agent에 제출하지 않고, 그 snapshot이 표시된 뒤에만 acceptance한다.
#[test]
fn query_refinement_remains_acceptable_before_the_next_frame() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/".to_owned()), Duration::ZERO)
        .unwrap();
    present_palette(&mut state, Size::new(80, 16));

    assert_eq!(
        state
            .handle(InputEvent::Paste("e".to_owned()), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Unchanged
    );
    assert_eq!(state.editor().text(), "/e");
    assert!(state.transcript().items().is_empty());

    let rendered = present_palette(&mut state, Size::new(80, 16));
    assert!(rendered.contains("/exit"), "{rendered}");
    assert!(!rendered.contains("/help"), "{rendered}");
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Exit
    );
}

// cursor 뒤에 다른 draft가 남아 있으면 앞쪽 slash 접두어는 command 입력 소유권을
// 얻지 않아 acceptance가 suffix를 지울 수 없다.
#[test]
fn cursor_prefix_does_not_open_over_a_trailing_draft() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/h keep".to_owned()), Duration::ZERO)
        .unwrap();
    for _ in 0..5 {
        state
            .handle(key(KeyCode::Left, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
    }

    let frame = state
        .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
        .unwrap();
    assert!(!frame.overlay_presented);
    assert_eq!(state.editor().text(), "/h keep");
    assert!(state.transcript().items().is_empty());
}

// cursor 뒤 text 때문에 palette가 소유하지 않는 known/unknown slash draft는 Enter 시점에도
// 다시 command로 분류되지 않고 현재 ordinary prompt owner에 그대로 제출된다.
#[test]
fn cursor_ineligible_slash_drafts_remain_ordinary_submissions() {
    for draft in ["/help", "/foo"] {
        let mut state = TuiState::new();
        state
            .handle(InputEvent::Paste(draft.to_owned()), Duration::ZERO)
            .unwrap();
        state
            .handle(key(KeyCode::Left, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();

        let frame = state
            .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
            .unwrap();
        assert!(!frame.overlay_presented);

        let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap()
        else {
            panic!("cursor-ineligible slash draft must use ordinary submission");
        };
        assert_eq!(submission.input().as_str(), draft);
        assert!(state.transcript().items().is_empty());
    }
}

// Esc는 현재 draft를 지우거나 agent 요청을 만들지 않고 command overlay만 닫는다.
#[test]
fn escape_closes_the_palette_without_submitting() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/".to_owned()), Duration::ZERO)
        .unwrap();
    present_palette(&mut state, Size::new(80, 16));

    assert_eq!(
        state
            .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), "/");
    assert!(state.transcript().items().is_empty());
    assert!(
        !state
            .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
            .unwrap()
            .overlay_presented
    );
}

// 선택한 /help는 agent로 제출하지 않고 로컬 notice를 남긴 뒤 prompt를 비운다.
#[test]
fn selected_help_runs_locally() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/h".to_owned()), Duration::ZERO)
        .unwrap();
    present_palette(&mut state, Size::new(80, 16));

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.editor().text().is_empty());
    assert_eq!(state.transcript().items().len(), 1);
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("Available commands"), "{output}");
    assert!(output.contains("/model"), "{output}");
}

// exact command는 palette frame을 기다리지 않아도 registry에서 로컬 실행된다.
#[test]
fn exact_help_runs_locally_before_palette_presentation() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/help".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.editor().text().is_empty());
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("Available commands"), "{output}");
}

// 앞쪽 공백과 ASCII 대소문자가 있는 exact command도 query scanner가 인정한 같은
// invocation이므로, 첫 palette frame 전후에 관계없이 동일한 로컬 효과를 실행한다.
#[test]
fn normalized_exact_help_is_frame_timing_independent() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("  /HELP".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.editor().text().is_empty());
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("Available commands"), "{output}");
}

// 아직 표시되지 않은 exact /model이 로컬 admission에 실패해도 제출 과정에서 비워진
// editor 대신 원래 command draft를 복원해 사용자가 그대로 수정할 수 있게 한다.
#[test]
fn unpresented_model_admission_failure_restores_the_exact_draft() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/model".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), "/model");
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("No configured model catalog"), "{output}");
}

// 목록 이동 뒤 선택한 /exit는 transcript를 추가하지 않고 기존 runner 종료 효과를 반환한다.
#[test]
fn selected_exit_uses_the_existing_runner_exit_boundary() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/ex".to_owned()), Duration::ZERO)
        .unwrap();
    present_palette(&mut state, Size::new(80, 16));

    assert_eq!(
        state
            .handle(key(KeyCode::Down, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Exit
    );
    assert!(state.transcript().items().is_empty());
}

// 아직 한 번도 표시되지 않은 부분 command의 Enter는 agent 제출로 빠지지 않고 로컬
// unknown 결과를 남기며, 사용자가 수정할 수 있도록 원문 draft를 보존한다.
#[test]
fn unpresented_partial_command_is_a_local_unknown() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/e".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), "/e");
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("Unknown command `/e`"), "{output}");
}

// 표시된 palette에 일치 항목이 없는 상태에서 Enter를 누르면 panel을 닫고 로컬 unknown을
// 알리되, draft를 agent나 Activity로 보내지 않는다.
#[test]
fn visible_unknown_command_closes_locally_and_preserves_the_draft() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/foo".to_owned()), Duration::ZERO)
        .unwrap();
    present_palette(&mut state, Size::new(80, 16));

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), "/foo");
    assert!(
        !state
            .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
            .unwrap()
            .overlay_presented
    );
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("Unknown command `/foo`"), "{output}");
}

// 실제로 보인 palette를 Esc로 닫으면 정확히 그 unchanged draft의 다음 Enter 한 번만
// ordinary agent 제출로 통과한다. 그 admission이 끝난 뒤 같은 draft는 다시 local unknown이다.
#[test]
fn escape_arms_one_exact_agent_submission() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/foo".to_owned()), Duration::ZERO)
        .unwrap();
    present_palette(&mut state, Size::new(80, 16));
    assert_eq!(
        state
            .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );

    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("dismissed command draft must pass through once");
    };
    assert_eq!(submission.input().as_str(), "/foo");
    state
        .observe_submission_outcome(SubmissionOutcome::Rejected {
            id: submission.id(),
            rejection: SubmissionRejection::new(
                SubmissionRejectionKind::StaleReference,
                "fixture rejection",
            ),
        })
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("Unknown command `/foo`"), "{output}");
}

// open됐지만 아직 frame에 표시되지 않은 command palette는 Esc를 소유하지 않는다.
// active Turn의 기존 interrupt 규칙이 그대로 우선한다.
#[test]
fn unpresented_palette_does_not_claim_active_turn_escape() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    state
        .handle(InputEvent::Paste("/foo".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::Interrupt)
    );
}

// 실제 /help는 펼쳐진 읽기 전용 문서로 렌더링하고 좁은 폭의 처음·끝 이동에서도 조작 안내를
// 보존한다.
#[test]
fn help_document_is_expanded_and_scrollable_without_starting_work() {
    use crate::surface::{CellContent, Point};
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/help".into()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    assert!(!state.turn_active());
    let pin = AppearanceState::default().pin();
    let source = state.session_output(&pin).unwrap().unwrap();
    for width in [80, 24, 80] {
        for code in [KeyCode::Home, KeyCode::End] {
            state
                .handle(key(code, KeyModifiers::NONE), Duration::ZERO)
                .unwrap();
            let frame = state.prepare_frame(Size::new(width, 30), &pin).unwrap();
            let text: String = (0..frame.surface.size().height)
                .flat_map(|y| (0..width).map(move |x| Point::new(x, y)))
                .filter_map(|point| match frame.surface.cell(point).unwrap().content() {
                    CellContent::Grapheme { text, .. } => Some(text.as_ref()),
                    _ => None,
                })
                .collect();
            let flat = text.split_whitespace().collect::<String>();
            if code == KeyCode::Home {
                assert!(flat.contains("Availablecommandsandkeyboardhelp"), "{flat}");
            } else {
                assert!(flat.contains("thisguide'sstart"), "{flat}");
            }
            assert!(!flat.contains("rowshidden"), "{flat}");
            state.commit_frame(&frame);
            assert_eq!(state.session_output(&pin).unwrap().unwrap(), source);
        }
    }
}

// /new는 idle에서만 host lifecycle 요청으로 끝나며 실행 중·예약 입력은 버리지 않는다.
#[test]
fn new_session_command_requires_idle_and_preserves_queued_work() {
    use yo_core::session_repository::RepositorySequence;
    for busy in [
        "idle",
        "active",
        "queued",
        "starting",
        "memory-only",
        "compacting",
    ] {
        let mut state = TuiState::new();
        if busy != "memory-only" {
            state
                .observe_durability(JournalDurability::Durable {
                    journal_sequence: None,
                    repository_sequence: RepositorySequence::new(1),
                })
                .unwrap();
        }
        match busy {
            "compacting" => {
                state
                    .handle(
                        InputEvent::Paste("/compact keep context".to_owned()),
                        Duration::ZERO,
                    )
                    .unwrap();
                assert!(matches!(
                    state
                        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                        .unwrap(),
                    StateEffect::Dispatch(AgentAction::CompactContext { .. })
                ));
            },
            "active" => {
                state
                    .observe(AgentEvent::TurnStarted { turn: turn() })
                    .unwrap();
            },
            "queued" => {
                state
                    .handle(InputEvent::Paste("later".to_owned()), Duration::ZERO)
                    .unwrap();
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::ALT), Duration::ZERO)
                    .unwrap();
            },
            "starting" => {
                state
                    .handle(InputEvent::Paste("work".to_owned()), Duration::ZERO)
                    .unwrap();
                let StateEffect::Dispatch(AgentAction::Submit(input)) = state
                    .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap()
                else {
                    panic!("submission");
                };
                state
                    .observe_submission_outcome(SubmissionOutcome::Accepted { id: input.id() })
                    .unwrap();
            },
            _ => {},
        }
        state
            .handle(InputEvent::Paste("/new".to_owned()), Duration::ZERO)
            .unwrap();
        present_palette(&mut state, Size::new(100, 24));
        let outcome = state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        assert_eq!(state.take_new_session_request(), busy == "idle");
        if busy == "idle" {
            assert_eq!(outcome, StateEffect::Exit);
            assert_eq!(state.editor().text(), "");
        } else {
            assert_eq!(outcome, StateEffect::Redraw);
            assert_eq!(state.editor().text(), "/new");
        }
        if busy == "compacting" {
            state
                .observe_control_outcome(yo_core::AgentControlOutcome::ContextCompactionRejected {
                    detail: "unsupported".to_owned(),
                })
                .unwrap();
            state
                .handle(
                    key(KeyCode::Character('u'), KeyModifiers::CONTROL),
                    Duration::ZERO,
                )
                .unwrap();
            state
                .handle(InputEvent::Paste("/new".to_owned()), Duration::ZERO)
                .unwrap();
            present_palette(&mut state, Size::new(100, 24));
            assert_eq!(
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap(),
                StateEffect::Exit
            );
            assert!(state.take_new_session_request());
        }
        if busy == "queued" {
            let Some(AgentAction::Submit(input)) = state.next_follow_up().unwrap() else {
                panic!("queued input was lost");
            };
            assert_eq!(input.input().as_str(), "later");
        }
    }
}

// /resume는 전체 UUID 또는 picker 요청으로만 전달되고 잘못된 ID는 draft를 보존한다.
#[test]
fn resume_command_routes_full_identity_or_picker_without_submitting_text() {
    let target: yo_core::SessionId = "01890f00-0000-7000-8000-000000000009".parse().unwrap();
    for (text, expected) in [
        ("/resume".to_owned(), Some(None)),
        (format!("/resume {target}"), Some(Some(target))),
        ("/resume not-a-session".to_owned(), None),
    ] {
        let mut state = TuiState::new();
        state
            .observe_durability(JournalDurability::Durable {
                journal_sequence: None,
                repository_sequence: RepositorySequence::new(1),
            })
            .unwrap();
        state
            .handle(InputEvent::Paste(text.clone()), Duration::ZERO)
            .unwrap();
        if text == "/resume" {
            present_palette(&mut state, Size::new(100, 24));
        }
        let result = state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        assert_eq!(state.take_resume_session_request(), expected);
        assert_eq!(
            result,
            if expected.is_some() {
                StateEffect::Exit
            } else {
                StateEffect::Redraw
            }
        );
        assert_eq!(
            state.editor().text(),
            if expected.is_some() { "" } else { &text }
        );
        assert!(state.next_follow_up().unwrap().is_none());
    }
}

// 저장 목록의 unavailable 항목은 건너뛰며 표시 이후 작성한 draft는 선택으로 지우지 않는다.
#[test]
fn resume_picker_skips_disabled_rows_and_preserves_newer_drafts() {
    use crate::overlay::{PanelSnapshot, SelectionEntry};
    let target: yo_core::SessionId = "01890f00-0000-7000-8000-000000000009".parse().unwrap();
    for newer_draft in [false, true] {
        let mut state = TuiState::new();
        state
            .observe_durability(JournalDurability::Durable {
                journal_sequence: None,
                repository_sequence: RepositorySequence::new(1),
            })
            .unwrap();
        state
            .show_resume_picker(
                PanelSnapshot::new(
                    "Resume saved session",
                    vec![
                        SelectionEntry::status("unavailable", "Cannot resume"),
                        SelectionEntry::enabled_with_context(
                            target.to_string(),
                            target.to_string(),
                            None,
                            None,
                        ),
                    ],
                )
                .unwrap(),
            )
            .unwrap();
        if newer_draft {
            state
                .handle(
                    InputEvent::Paste("keep this draft".to_owned()),
                    Duration::ZERO,
                )
                .unwrap();
        }
        present_palette(&mut state, Size::new(100, 24));
        let result = state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        if newer_draft {
            assert_eq!(result, StateEffect::Redraw);
            assert_eq!(state.editor().text(), "keep this draft");
            assert_eq!(state.take_resume_session_request(), None);
        } else {
            assert_eq!(result, StateEffect::Exit);
            assert_eq!(state.take_resume_session_request(), Some(Some(target)));
        }
    }
}

// /new와 /resume는 같은 idle guard를 쓰므로 대기 중인 후속 입력을 전환으로 버리지 않는다.
#[test]
fn resume_command_preserves_queued_work() {
    let mut state = TuiState::new();
    state
        .observe_durability(JournalDurability::Durable {
            journal_sequence: None,
            repository_sequence: RepositorySequence::new(1),
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("later".to_owned()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::ALT), Duration::ZERO)
        .unwrap();
    let text = "/resume 01890f00-0000-7000-8000-000000000009";
    state
        .handle(InputEvent::Paste(text.to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), text);
    assert_eq!(state.take_resume_session_request(), None);
    let Some(AgentAction::Submit(input)) = state.next_follow_up().unwrap() else {
        panic!("queue lost");
    };
    assert_eq!(input.input().as_str(), "later");
}

// /fork는 durable idle 상태에서만 terminal 전환을 요청하고 진행 중인 작업과 draft를 보존합니다.
#[test]
fn fork_command_requires_idle_and_preserves_queued_work() {
    use yo_core::session_repository::RepositorySequence;
    for busy in [
        "idle",
        "active",
        "queued",
        "starting",
        "memory-only",
        "compacting",
    ] {
        let mut state = TuiState::new();
        if busy != "memory-only" {
            state
                .observe_durability(JournalDurability::Durable {
                    journal_sequence: None,
                    repository_sequence: RepositorySequence::new(1),
                })
                .unwrap();
        }
        match busy {
            "compacting" => {
                state
                    .handle(
                        InputEvent::Paste("/compact keep context".to_owned()),
                        Duration::ZERO,
                    )
                    .unwrap();
                assert!(matches!(
                    state
                        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                        .unwrap(),
                    StateEffect::Dispatch(AgentAction::CompactContext { .. })
                ));
            },
            "active" => {
                state
                    .observe(AgentEvent::TurnStarted { turn: turn() })
                    .unwrap();
            },
            "queued" => {
                state
                    .handle(InputEvent::Paste("later".to_owned()), Duration::ZERO)
                    .unwrap();
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::ALT), Duration::ZERO)
                    .unwrap();
            },
            "starting" => {
                state
                    .handle(InputEvent::Paste("work".to_owned()), Duration::ZERO)
                    .unwrap();
                let StateEffect::Dispatch(AgentAction::Submit(input)) = state
                    .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap()
                else {
                    panic!("submission");
                };
                state
                    .observe_submission_outcome(SubmissionOutcome::Accepted { id: input.id() })
                    .unwrap();
            },
            _ => {},
        }
        state
            .handle(InputEvent::Paste("/fork".to_owned()), Duration::ZERO)
            .unwrap();
        present_palette(&mut state, Size::new(100, 24));
        let outcome = state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        assert_eq!(state.take_fork_session_request(), busy == "idle");
        if busy == "idle" {
            assert_eq!(outcome, StateEffect::Exit);
            assert_eq!(state.editor().text(), "");
        } else {
            assert_eq!(outcome, StateEffect::Redraw);
            assert_eq!(state.editor().text(), "/fork");
        }
        if busy == "compacting" {
            state
                .observe_control_outcome(yo_core::AgentControlOutcome::ContextCompactionRejected {
                    detail: "unsupported".to_owned(),
                })
                .unwrap();
            state
                .handle(
                    key(KeyCode::Character('u'), KeyModifiers::CONTROL),
                    Duration::ZERO,
                )
                .unwrap();
            state
                .handle(InputEvent::Paste("/fork".to_owned()), Duration::ZERO)
                .unwrap();
            present_palette(&mut state, Size::new(100, 24));
            assert_eq!(
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap(),
                StateEffect::Exit
            );
            assert!(state.take_fork_session_request());
        }
        if busy == "queued" {
            let Some(AgentAction::Submit(input)) = state.next_follow_up().unwrap() else {
                panic!("queued input was lost");
            };
            assert_eq!(input.input().as_str(), "later");
        }
    }
}

// 승인 대기와 잘못된 인수는 fork나 빈 child를 만들지 않으며 원래 draft를 유지합니다.
#[test]
fn fork_rejects_pending_requests_and_arguments_without_dispatch() {
    use yo_core::{ActivityKind, RequestId, session_repository::RepositorySequence};
    for (text, pending) in [
        ("/fork", true),
        ("/fork --empty", false),
        ("/fork other", false),
        ("/fork at extra", false),
        ("/fork 10", false),
    ] {
        let mut state = TuiState::new();
        state
            .observe_durability(JournalDurability::Durable {
                journal_sequence: None,
                repository_sequence: RepositorySequence::new(1),
            })
            .unwrap();
        if pending {
            state
                .observe(AgentEvent::ActivityStarted {
                    activity: super::activity(1),
                    kind: ActivityKind::ApprovalRequest {
                        request_id: RequestId::new(super::nonzero(7)),
                    },
                })
                .unwrap();
        }
        state
            .handle(InputEvent::Paste(text.to_owned()), Duration::ZERO)
            .unwrap();
        let result = state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        assert_eq!(result, StateEffect::Redraw);
        assert_eq!(state.editor().text(), text);
        assert!(!state.take_fork_session_request());
        assert!(!state.take_new_session_request());
        assert!(state.next_follow_up().unwrap().is_none());
    }
}

fn historical_fork_state() -> TuiState {
    let mut state = TuiState::new();
    state
        .observe_durability(JournalDurability::Durable {
            journal_sequence: None,
            repository_sequence: RepositorySequence::new(1),
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("/fork at".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Exit
    );
    assert!(state.take_fork_picker_request());
    assert!(!state.take_fork_session_request());
    assert!(state.editor().text().is_empty());
    state
}

fn historical_fork_panel() -> PanelSnapshot {
    PanelSnapshot::new(
        "Fork from an earlier point",
        vec![
            SelectionEntry::enabled("0", "Newest turn", None),
            SelectionEntry::enabled("1", "Earlier checkpoint", None),
            SelectionEntry::status("truncated", "Older boundaries were omitted"),
        ],
    )
    .unwrap()
}

// /fork at은 catalog만 요청하고 선택은 exact picker token과 row index로 host에 넘긴다.
#[test]
fn historical_fork_selection_requests_the_row_from_its_exact_picker() {
    let mut state = historical_fork_state();
    let picker = state.show_fork_picker(historical_fork_panel(), 2).unwrap();
    let output = present_palette(&mut state, Size::new(100, 24));
    assert!(output.contains("Newest turn"));
    assert!(output.contains("Older boundaries were omitted"));
    state
        .handle(key(KeyCode::Down, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    present_palette(&mut state, Size::new(100, 24));
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Exit
    );
    assert_eq!(state.take_fork_boundary_request(), Some((picker, 1)));
    assert_eq!(state.take_fork_boundary_request(), None);
    assert!(!state.take_fork_session_request());
    assert_eq!(state.editor().text(), "");
    state.report_fork_failure("historical model is unavailable".to_owned());
    assert_eq!(state.editor().text(), "/fork at");
}

// 취소·준비 실패는 원래 draft를 복원하고 picker가 열린 뒤 입력한 새 draft는 덮어쓰지 않는다.
#[test]
fn historical_fork_cancel_and_failure_preserve_original_or_newer_drafts() {
    for action in [
        "cancel",
        "capture-failure",
        "newer-cancel",
        "newer-selection",
        "newer-failure",
    ] {
        let mut state = historical_fork_state();
        if action != "capture-failure" {
            state.show_fork_picker(historical_fork_panel(), 2).unwrap();
            present_palette(&mut state, Size::new(100, 24));
        }
        let newer = action.starts_with("newer");
        if newer {
            state
                .handle(
                    InputEvent::Paste("keep this newer draft".to_owned()),
                    Duration::ZERO,
                )
                .unwrap();
        }
        match action {
            "cancel" | "newer-cancel" => {
                state
                    .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap();
            },
            "newer-selection" => {
                assert_eq!(
                    state
                        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                        .unwrap(),
                    StateEffect::Redraw
                );
            },
            _ => state.report_fork_failure("capture is no longer available".to_owned()),
        }
        assert_eq!(
            state.editor().text(),
            if newer {
                "keep this newer draft"
            } else {
                "/fork at"
            }
        );
        assert_eq!(state.take_fork_boundary_request(), None);
        assert!(!state.take_fork_session_request());
    }
}

// overlay가 교체되면 이전 token으로 닫거나 갱신할 수 없고 선택은 새 catalog generation에 묶인다.
#[test]
fn historical_fork_picker_replacement_invalidates_the_old_overlay_token() {
    let mut state = historical_fork_state();
    let first = state.show_fork_picker(historical_fork_panel(), 2).unwrap();
    present_palette(&mut state, Size::new(100, 24));
    let second = state.show_fork_picker(historical_fork_panel(), 2).unwrap();
    assert_ne!(first, second);
    assert_eq!(state.close_overlay(first.0), Err(SlotError::StaleToken));
    assert_eq!(
        state.refresh_overlay(first.0, historical_fork_panel()),
        Err(SlotError::StaleToken)
    );
    present_palette(&mut state, Size::new(100, 24));
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Exit
    );
    assert_eq!(state.take_fork_boundary_request(), Some((second, 0)));
}

// catalog 표시 후 active·queue·durability가 바뀌어도 기존 idle guard를 다시 검사한다.
#[test]
fn historical_fork_selection_rechecks_idle_and_durable_parent() {
    for change in ["active", "queued", "memory-only"] {
        let mut state = historical_fork_state();
        state.show_fork_picker(historical_fork_panel(), 2).unwrap();
        present_palette(&mut state, Size::new(100, 24));
        match change {
            "active" => {
                state
                    .observe(AgentEvent::TurnStarted { turn: turn() })
                    .unwrap();
            },
            "queued" => {
                state
                    .handle(InputEvent::Paste("later".to_owned()), Duration::ZERO)
                    .unwrap();
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::ALT), Duration::ZERO)
                    .unwrap();
            },
            _ => {
                state
                    .observe_durability(JournalDurability::MemoryOnly)
                    .unwrap();
            },
        }
        present_palette(&mut state, Size::new(100, 24));
        assert_eq!(
            state
                .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Redraw
        );
        assert_eq!(state.take_fork_boundary_request(), None);
        assert_eq!(state.editor().text(), "/fork at");
        if change == "queued" {
            let Some(AgentAction::Submit(input)) = state.next_follow_up().unwrap() else {
                panic!("queued input was lost");
            };
            assert_eq!(input.input().as_str(), "later");
        }
    }
}

// catalog 결과가 돌아오기 전에 생긴 새 draft나 durable 상태 변화도 표시 전에 다시 확인한다.
#[test]
fn historical_fork_picker_rechecks_state_before_displaying_a_capture() {
    for newer in [false, true] {
        let mut state = historical_fork_state();
        if newer {
            state
                .handle(InputEvent::Paste("newer draft".to_owned()), Duration::ZERO)
                .unwrap();
        } else {
            state
                .observe_durability(JournalDurability::MemoryOnly)
                .unwrap();
        }
        assert!(state.show_fork_picker(historical_fork_panel(), 2).is_err());
        assert_eq!(
            state.editor().text(),
            if newer { "newer draft" } else { "/fork at" }
        );
        assert_eq!(state.take_fork_boundary_request(), None);
    }
}

// /tree는 전환과 같은 durable idle guard를 지켜 대기 작업과 draft를 버리지 않습니다.
#[test]
fn tree_command_requires_idle_and_preserves_queued_work() {
    use yo_core::session_repository::RepositorySequence;
    for busy in [
        "idle",
        "active",
        "queued",
        "starting",
        "memory-only",
        "compacting",
    ] {
        let mut state = TuiState::new();
        if busy != "memory-only" {
            state
                .observe_durability(JournalDurability::Durable {
                    journal_sequence: None,
                    repository_sequence: RepositorySequence::new(1),
                })
                .unwrap();
        }
        match busy {
            "compacting" => {
                state
                    .handle(
                        InputEvent::Paste("/compact keep context".to_owned()),
                        Duration::ZERO,
                    )
                    .unwrap();
                assert!(matches!(
                    state
                        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                        .unwrap(),
                    StateEffect::Dispatch(AgentAction::CompactContext { .. })
                ));
            },
            "active" => {
                state
                    .observe(AgentEvent::TurnStarted { turn: turn() })
                    .unwrap();
            },
            "queued" => {
                state
                    .handle(InputEvent::Paste("later".to_owned()), Duration::ZERO)
                    .unwrap();
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::ALT), Duration::ZERO)
                    .unwrap();
            },
            "starting" => {
                state
                    .handle(InputEvent::Paste("work".to_owned()), Duration::ZERO)
                    .unwrap();
                let StateEffect::Dispatch(AgentAction::Submit(input)) = state
                    .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap()
                else {
                    panic!("submission");
                };
                state
                    .observe_submission_outcome(SubmissionOutcome::Accepted { id: input.id() })
                    .unwrap();
            },
            _ => {},
        }
        state
            .handle(InputEvent::Paste("/tree".to_owned()), Duration::ZERO)
            .unwrap();
        present_palette(&mut state, Size::new(100, 24));
        let outcome = state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        assert_eq!(state.take_session_tree_request(), busy == "idle");
        if busy == "idle" {
            assert_eq!(outcome, StateEffect::Exit);
            assert_eq!(state.editor().text(), "");
        } else {
            assert_eq!(outcome, StateEffect::Redraw);
            assert_eq!(state.editor().text(), "/tree");
        }
        if busy == "compacting" {
            state
                .observe_control_outcome(yo_core::AgentControlOutcome::ContextCompactionRejected {
                    detail: "unsupported".to_owned(),
                })
                .unwrap();
            state
                .handle(
                    key(KeyCode::Character('u'), KeyModifiers::CONTROL),
                    Duration::ZERO,
                )
                .unwrap();
            state
                .handle(InputEvent::Paste("/tree".to_owned()), Duration::ZERO)
                .unwrap();
            present_palette(&mut state, Size::new(100, 24));
            assert_eq!(
                state
                    .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap(),
                StateEffect::Exit
            );
            assert!(state.take_session_tree_request());
        }
        if busy == "queued" {
            let Some(AgentAction::Submit(input)) = state.next_follow_up().unwrap() else {
                panic!("queued input was lost");
            };
            assert_eq!(input.input().as_str(), "later");
        }
    }
}

// /tree 인수와 승인 대기는 실제 Session 선택이나 모델 요청으로 해석하지 않습니다.
#[test]
fn tree_rejects_pending_requests_and_arguments_without_dispatch() {
    use yo_core::{ActivityKind, RequestId, session_repository::RepositorySequence};
    for (text, pending) in [
        ("/tree", true),
        ("/tree --empty", false),
        ("/tree other", false),
    ] {
        let mut state = TuiState::new();
        state
            .observe_durability(JournalDurability::Durable {
                journal_sequence: None,
                repository_sequence: RepositorySequence::new(1),
            })
            .unwrap();
        if pending {
            state
                .observe(AgentEvent::ActivityStarted {
                    activity: super::activity(1),
                    kind: ActivityKind::ApprovalRequest {
                        request_id: RequestId::new(super::nonzero(7)),
                    },
                })
                .unwrap();
        }
        state
            .handle(InputEvent::Paste(text.to_owned()), Duration::ZERO)
            .unwrap();
        let result = state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        assert_eq!(result, StateEffect::Redraw);
        assert_eq!(state.editor().text(), text);
        assert!(!state.take_session_tree_request());
        assert!(!state.take_new_session_request());
        assert!(state.next_follow_up().unwrap().is_none());
    }
}
