use std::time::Duration;

use serde_json::json;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityUpdate, AgentCommand, AgentEvent, DurabilityGapCause,
    JournalDurability, ToolOutput, TranscriptRecord, UserInput,
    session_repository::{DurableCutoff, RepositorySequence},
};

use super::{
    super::{activity, key, turn},
    support::{function, render_and_commit},
};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyAction, KeyCode, KeyModifiers},
    runner::{
        session::TuiStatusLine,
        state::{StateEffect, TuiState},
        view::ObservabilityView,
    },
    surface::{CellContent, Point, Size},
};

// 저장 실패 경고는 이후 대화와 host status 갱신에 밀려 사라지지 않는다. 좁은 화면에서도
// 상태 줄에 유지되고, 실제 Durable 복구를 관찰한 뒤에만 최신 host status가 다시 보인다.
#[test]
fn storage_failure_status_survives_new_chat_and_narrow_resize() {
    let mut state = TuiState::new();
    state
        .observe_durability(JournalDurability::Gap {
            durable_cutoff: DurableCutoff::Unknown,
            cause: DurabilityGapCause::Integrity,
        })
        .unwrap();
    for index in 0..20 {
        state
            .observe_record(TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn: turn(),
                    input: UserInput::from(format!("later activity {index}")),
                },
            ))
            .unwrap();
    }
    state.set_status_line(TuiStatusLine::new([("git", "branch-main")]).unwrap());
    for width in [20, 40] {
        state
            .handle(function(1, KeyAction::Press), Duration::ZERO)
            .unwrap();
        let screen = render_and_commit(&mut state, Size::new(width, 12));
        assert!(
            screen
                .lines()
                .rev()
                .take(3)
                .any(|line| line.trim() == "History not saved"),
            "{screen}"
        );
        assert!(!screen.contains("branch-main"), "{screen}");
        for mode in [2, 3] {
            state
                .handle(function(mode, KeyAction::Press), Duration::ZERO)
                .unwrap();
            for height in [1, 12] {
                let screen = render_and_commit(&mut state, Size::new(width, height));
                assert_eq!(
                    screen.lines().next().unwrap().trim(),
                    "History not saved",
                    "{screen}"
                );
            }
        }
    }
    state
        .observe_durability(JournalDurability::Durable {
            journal_sequence: None,
            repository_sequence: RepositorySequence::new(12),
        })
        .unwrap();
    for mode in [2, 3] {
        state
            .handle(function(mode, KeyAction::Press), Duration::ZERO)
            .unwrap();
        let screen = render_and_commit(&mut state, Size::new(20, 12));
        assert!(
            !screen.lines().next().unwrap().contains("History not saved"),
            "{screen}"
        );
    }
    state
        .handle(function(1, KeyAction::Press), Duration::ZERO)
        .unwrap();
    let screen = render_and_commit(&mut state, Size::new(20, 12));
    assert!(
        screen
            .lines()
            .rev()
            .take(3)
            .any(|line| line.trim() == "branch-main"),
        "{screen}"
    );
    assert!(
        !screen
            .lines()
            .rev()
            .take(3)
            .any(|line| line.trim() == "History not saved"),
        "{screen}"
    );
}

// 한 frame 사이에 들어온 방향키는 마지막 키로 덮지 않고 순서대로 적용한다.
// 경계에서의 반대 방향 이동과 실패 후 재시도도 개별 frame 처리와 같아야 한다.
#[test]
fn coalesced_navigation_preserves_every_key_and_boundary_order() {
    fn seeded() -> TuiState {
        let mut state = TuiState::new();
        for index in 0..30 {
            state
                .observe_record(TranscriptRecord::CommandCommitted(
                    AgentCommand::StartTurn {
                        turn: turn(),
                        input: UserInput::from(format!("question {index}: 한글 wrapping")),
                    },
                ))
                .unwrap();
        }
        state
    }
    let size = Size::new(40, 12);
    for mode in [1, 2] {
        for sequence in [
            vec![KeyCode::Up; 5],
            vec![KeyCode::Home, KeyCode::Up, KeyCode::Down],
            vec![KeyCode::Up, KeyCode::Up, KeyCode::Down],
        ] {
            let mut burst = seeded();
            let mut serial = seeded();
            for state in [&mut burst, &mut serial] {
                state
                    .handle(function(mode, KeyAction::Press), Duration::ZERO)
                    .unwrap();
                state
                    .handle(key(KeyCode::End, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap();
                render_and_commit(state, size);
            }
            let before = burst.views().view_positions();
            for code in sequence {
                serial
                    .handle(key(code, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap();
                render_and_commit(&mut serial, size);
                burst
                    .handle(key(code, KeyModifiers::NONE), Duration::ZERO)
                    .unwrap();
            }
            assert!(
                burst
                    .prepare_frame(Size::new(40, 0), &AppearanceState::default().pin())
                    .is_err()
            );
            assert_eq!(burst.views().view_positions(), before);
            let actual = render_and_commit(&mut burst, size);
            let expected = render_and_commit(&mut serial, size);
            assert_eq!(actual, expected);
            assert_eq!(
                burst.views().view_positions(),
                serial.views().view_positions()
            );
            assert_eq!(
                render_and_commit(&mut burst, size),
                actual,
                "keys must be consumed only once"
            );
        }
    }
}

// Chat, Transcript, Request에서 각각 분리된 viewport를 움직인 뒤 mode를 왕복하면 같은
// anchor일 때 각 first-visible-row가 복원되어 다른 view의 scroll이 덮어쓰지 않는다.
#[test]
fn switching_restores_each_view_local_scroll_state() {
    let mut state = TuiState::new();
    for index in 0..12 {
        state
            .observe_record(TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn: turn(),
                    input: UserInput::from(format!("question {index}")),
                },
            ))
            .unwrap();
    }
    let size = Size::new(12, 5);
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);

    state
        .handle(function(2, KeyAction::Press), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);

    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);
    let detached = state.views().view_positions();
    assert!(detached.0 > 0);
    assert!(detached.1 > 0);
    assert!(detached.2 > 0);

    for mode in [1, 2, 3] {
        state
            .handle(function(mode, KeyAction::Press), Duration::ZERO)
            .unwrap();
        render_and_commit(&mut state, size);
    }
    assert_eq!(state.views().view_positions(), detached);
}

// 유효한 frame을 commit한 뒤 준비가 실패해도 committed viewport와 미소비 scroll 의도는 그대로
// 남아, 재시도한 유효 frame에서만 다음 viewport가 적용된다.
#[test]
fn failed_frame_preparation_keeps_committed_viewport_and_pending_scroll() {
    let mut state = TuiState::new();
    for index in 0..12 {
        state
            .observe_record(TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn: turn(),
                    input: UserInput::from(format!("question {index}")),
                },
            ))
            .unwrap();
    }
    let size = Size::new(18, 5);
    let first = state
        .prepare_frame(size, &AppearanceState::default().pin())
        .unwrap();
    state.commit_frame(&first);
    let committed = state.views().view_positions();

    assert_eq!(
        state.handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO),
        Ok(StateEffect::Redraw)
    );
    assert!(state.views().chat_has_pending_scroll());
    assert!(
        state
            .prepare_frame(Size::new(size.width, 0), &AppearanceState::default().pin(),)
            .is_err()
    );
    assert_eq!(state.views().view_positions(), committed);
    assert!(state.views().chat_has_pending_scroll());

    let retry = state
        .prepare_frame(size, &AppearanceState::default().pin())
        .unwrap();
    state.commit_frame(&retry);
    assert!(state.views().view_positions().0 < committed.0);
}

// 빠른 휠 왕복은 모든 줄 이동을 반영하고 작성 중인 프롬프트와 화면을 그대로 복원한다.
#[test]
fn wheel_bursts_restore_view_and_preserve_draft() {
    let mut state = TuiState::new();
    for index in 0..30 {
        state
            .observe_record(TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn: turn(),
                    input: UserInput::from(format!("question {index}")),
                },
            ))
            .unwrap();
    }
    state
        .handle(InputEvent::Paste("unfinished draft".into()), Duration::ZERO)
        .unwrap();
    let size = Size::new(40, 12);
    let before = render_and_commit(&mut state, size);
    for _ in 0..5 {
        state
            .handle(InputEvent::MouseScroll(-3), Duration::ZERO)
            .unwrap();
    }
    assert_ne!(render_and_commit(&mut state, size), before);
    for _ in 0..5 {
        state
            .handle(InputEvent::MouseScroll(3), Duration::ZERO)
            .unwrap();
    }
    assert_eq!(render_and_commit(&mut state, size), before);
}

// /changes는 실제 FileChange만 파일별로 펼쳐 보여주고 읽기 입력이 모델 제출로 새지 않는다.
// 좁은 폭에서도 복귀 키를 알 수 있고 F1은 기존 대화 위치로 돌아간다.
#[test]
fn changes_review_navigates_files_without_dispatching_input() {
    let mut state = TuiState::new();
    for (id, kind, text) in [
        (
            1,
            ActivityKind::AgentMessage,
            "add: pretend.rs\n+not a file change".to_owned(),
        ),
        (
            2,
            ActivityKind::FileChange,
            format!(
                "update: first.rs\ndiff --git a/first.rs b/first.rs\n@@ -0,0 +1,20 @@\n{}\nadd: second.rs\n+second file\n",
                (1..=20)
                    .map(|n| format!("+line {n:02}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        ),
    ] {
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(id),
                kind,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(id),
                update: ActivityUpdate::TextSnapshot(text),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(id),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
    }
    let size = Size::new(40, 12);
    let chat = render_and_commit(&mut state, size);
    state
        .handle(InputEvent::Paste("/changes".into()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    let first = render_and_commit(&mut state, size);
    assert!(first.contains("1/2"), "{first}");
    assert!(first.contains("first.rs"), "{first}");
    assert!(!first.contains("pretend.rs"));
    assert!(!first.contains("rows hidden"));
    state
        .handle(InputEvent::MouseScroll(6), Duration::ZERO)
        .unwrap();
    let scrolled = render_and_commit(&mut state, size);
    assert_ne!(scrolled, first);
    assert_eq!(scrolled.lines().next(), first.lines().next());
    state
        .handle(key(KeyCode::Right, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let second = render_and_commit(&mut state, Size::new(20, 12));
    assert!(
        second.lines().next().unwrap().contains("2/2 second.rs F1"),
        "{second}"
    );
    assert!(second.contains("second.rs"), "{second}");
    for event in [
        InputEvent::Paste("do not submit".into()),
        key(KeyCode::Enter, KeyModifiers::NONE),
    ] {
        assert!(!matches!(
            state.handle(event, Duration::ZERO).unwrap(),
            StateEffect::Dispatch(_)
        ));
    }
    assert!(state.editor().text().is_empty());
    state
        .handle(key(KeyCode::Left, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    assert_eq!(render_and_commit(&mut state, size), first);
    state
        .handle(function(1, KeyAction::Press), Duration::ZERO)
        .unwrap();
    assert_eq!(render_and_commit(&mut state, size), chat);
}

// 긴 파일 경로는 고정 안내에서만 줄이며 폭 변경과 스크롤 뒤에도 파일명을 유지한다.
#[test]
fn changes_header_preserves_unicode_filename_while_scrolling() {
    let mut state = TuiState::new();
    let path = "src/a/very/long/directory/한글-e\u{301}.rs";
    let body = format!("update: {path}\n{}", "+changed line\n".repeat(30));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::FileChange,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(body),
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("/changes".into()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let wide = render_and_commit(&mut state, Size::new(100, 12));
    assert!(
        wide.lines()
            .next()
            .unwrap()
            .contains("directory/한 글 -e.rs"),
        "{wide}"
    );
    let narrow = render_and_commit(&mut state, Size::new(24, 12));
    let header = narrow.lines().next().unwrap();
    assert!(header.starts_with("1/1 …"), "{narrow}");
    assert!(header.ends_with("한 글 -e.rs"), "{narrow}");
    let frame = state
        .prepare_frame(Size::new(24, 12), &AppearanceState::default().pin())
        .unwrap();
    let title: String = (0..24)
        .filter_map(
            |x| match frame.surface.cell(Point::new(x, 0)).unwrap().content() {
                CellContent::Grapheme { text, .. } => Some(text.as_ref()),
                _ => None,
            },
        )
        .collect();
    assert!(title.ends_with("한글-e\u{301}.rs"), "{title}");

    state
        .handle(key(KeyCode::End, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let scrolled = render_and_commit(&mut state, Size::new(24, 12));
    assert_eq!(scrolled.lines().next(), Some(header));
    assert_ne!(scrolled, narrow);
    for width in [1, 4, 6, 12, 24, 100] {
        render_and_commit(&mut state, Size::new(width, 12));
    }
}

// 앞서 시작한 병렬 변경에 파일이 추가되어 전체 순번이 바뀌어도 보고 있던 Activity의 파일을
// 유지한다.
#[test]
fn live_changes_keep_the_selected_activity_when_earlier_files_arrive() {
    let mut state = TuiState::new();
    for (id, body) in [
        (1, "update: earlier.rs\n+first"),
        (2, "update: selected.rs\n+second"),
    ] {
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(id),
                kind: ActivityKind::FileChange,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(id),
                update: ActivityUpdate::TextSnapshot(body.into()),
            })
            .unwrap();
    }
    state
        .handle(InputEvent::Paste("/changes".into()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, Size::new(40, 12));
    state
        .handle(key(KeyCode::Right, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    assert!(render_and_commit(&mut state, Size::new(40, 12)).contains("selected.rs"));
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(
                "update: earlier.rs\n+first\nadd: arrived.rs\n+late".into(),
            ),
        })
        .unwrap();
    let frame = render_and_commit(&mut state, Size::new(40, 12));
    assert!(frame.contains("3/3"), "{frame}");
    assert!(frame.contains("selected.rs"), "{frame}");
    assert!(!frame.contains("arrived.rs"));
}

// /output은 마지막 도구의 보관된 원문을 전체 높이 제한 없이 탐색하고 입력을 실행하지 않는다.
// 휠·페이지·도구 전환과 좁은 화면 왕복 뒤에도 F1은 기존 Chat으로 돌아간다.
#[test]
fn retained_output_pages_large_tools_and_keeps_input_local() {
    let mut state = TuiState::new();
    for (id, kind, source) in [
        (1, ActivityKind::AgentMessage, "not tool output".to_owned()),
        (2, ActivityKind::ToolCall, "first tool body".to_owned()),
        (
            3,
            ActivityKind::ToolResult,
            (0..70_000)
                .map(|index| format!("row {index:05}\n"))
                .collect::<String>()
                + "last 한글",
        ),
    ] {
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(id),
                kind,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(id),
                update: ActivityUpdate::TextSnapshot(source),
            })
            .unwrap();
    }
    state
        .handle(InputEvent::Paste("/output".into()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    let size = Size::new(80, 12);
    let first = render_and_commit(&mut state, size);
    assert!(first.contains("Output 2/2"), "{first}");
    assert!(first.contains("row 00000"), "{first}");
    state
        .handle(key(KeyCode::PageDown, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let page = render_and_commit(&mut state, size);
    assert!(page.contains("row 00011"), "{page}");
    state
        .handle(InputEvent::MouseScroll(-1), Duration::ZERO)
        .unwrap();
    let page = render_and_commit(&mut state, size);
    assert!(page.contains("row 00010"), "{page}");
    state
        .handle(key(KeyCode::End, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    for columns in [80, 24, 80] {
        let last = render_and_commit(&mut state, Size::new(columns, 12));
        assert!(last.contains("last 한 글"), "{last}");
        assert!(!last.contains("HeightOverflow"));
    }
    state
        .handle(key(KeyCode::Left, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let previous = render_and_commit(&mut state, size);
    assert!(previous.contains("first tool body"), "{previous}");
    assert!(!previous.contains("not tool output"));
    state
        .handle(key(KeyCode::Right, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    assert_eq!(render_and_commit(&mut state, size), first);
    for event in [
        InputEvent::Paste("do not submit".into()),
        key(KeyCode::Enter, KeyModifiers::NONE),
    ] {
        assert!(!matches!(
            state.handle(event, Duration::ZERO).unwrap(),
            StateEffect::Dispatch(_)
        ));
    }
    assert!(state.editor().text().is_empty());
    state
        .handle(function(1, KeyAction::Press), Duration::ZERO)
        .unwrap();
    assert_eq!(state.views().active(), ObservabilityView::Chat);
}

// 구조화 도구는 JSON envelope 대신 보관된 plain_text를 읽고 마지막 행을 넘는 입력도
// frame 승인 전에는 소비하지 않는다. 표시 불가능한 셀은 ASCII escape로 탐색 가능하다.
#[test]
fn output_decodes_retained_profiles_and_retries_navigation_after_failed_frames() {
    let mut state = TuiState::new();
    let output = ToolOutput {
        tool: "external".into(),
        server: None,
        arguments: None,
        result: None,
        content_items: None,
        error: None,
        plain_text: "first\nsecond\nthird\nlast".into(),
    };
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolResult,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("/output".into()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let size = Size::new(24, 3);
    let first = render_and_commit(&mut state, size);
    assert!(first.contains("first\nsecond"), "{first}");
    assert!(!first.contains("yo.tool-output"));
    for code in [KeyCode::End, KeyCode::Down, KeyCode::Up] {
        state
            .handle(key(code, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
    }
    assert!(
        state
            .prepare_frame(Size::new(24, 0), &AppearanceState::default().pin())
            .is_err()
    );
    let page = render_and_commit(&mut state, size);
    assert!(page.contains("second\nthird"), "{page}");
    assert_eq!(render_and_commit(&mut state, size), page);
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot("\u{301}한글".into()),
        })
        .unwrap();
    let escaped = render_and_commit(&mut state, Size::new(80, 5));
    assert!(escaped.contains("Escaped output"), "{escaped}");
    assert!(escaped.contains("\\u{301}"), "{escaped}");
    render_and_commit(&mut state, Size::new(1, 5));
}

// 도구 원문을 한 칸 폭으로 읽어도 폭 복원 시 같은 줄로 돌아오며 줄바꿈을 평탄화하지 않는다.
#[test]
fn output_escaped_resize_preserves_the_reading_position() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolResult,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot("first\n한글\nlast\nend".into()),
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("/output".into()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let size = Size::new(24, 3);
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::Down, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let before = render_and_commit(&mut state, size);
    assert!(before.contains("한 글\nlast"), "{before}");
    render_and_commit(&mut state, Size::new(1, 3));
    assert_eq!(render_and_commit(&mut state, size), before);
    render_and_commit(&mut state, Size::new(1, 3));
    state
        .handle(key(KeyCode::Down, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, Size::new(1, 3));
    assert_eq!(render_and_commit(&mut state, size), before);
}

// 생략 관측은 끝까지 스크롤하거나 폭을 줄여도 헤더에 남고, 새 snapshot의 false·미상은
// 이전 경고를 지운다. 경로·문자열만으로 생략이나 로컬 원문 접근 가능성을 추정하지 않는다.
#[test]
fn output_keeps_reported_truncation_visible_while_paging() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolResult,
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("/output".into()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    for (result, partial) in [
        (Some(json!({"truncated": true})), true),
        (Some(json!({"truncated": false})), false),
        (
            Some(json!({"truncated": true,"retainedOutput":{"truncated":false}})),
            false,
        ),
        (
            Some(json!({"truncated": false,"retainedOutput":{"truncated":true}})),
            true,
        ),
        (
            Some(json!({"details": {"truncation": {"truncated": true}}})),
            true,
        ),
        (
            Some(json!({"truncated": "true", "details": {"fullOutputPath": "/untrusted/output"}})),
            false,
        ),
        (None, false),
    ] {
        let output = ToolOutput {
            tool: "bash".into(),
            server: None,
            arguments: None,
            result,
            content_items: None,
            error: None,
            plain_text: "first\nsecond\nthird\nlast".into(),
        };
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let wide = render_and_commit(&mut state, Size::new(80, 3));
        assert_eq!(wide.contains("Partial output"), partial, "{wide}");
        state
            .handle(key(KeyCode::End, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        let tail = render_and_commit(&mut state, Size::new(24, 3));
        assert!(tail.contains("last"), "{tail}");
        assert_eq!(tail.contains("Partial"), partial, "{tail}");
        let tiny = render_and_commit(&mut state, Size::new(1, 3));
        assert_eq!(tiny.contains('!'), partial, "{tiny}");
        let restored = render_and_commit(&mut state, Size::new(80, 3));
        assert_eq!(restored.contains("Partial output"), partial, "{restored}");
        assert!(!restored.contains("yo.tool-output"), "{restored}");
    }
}

// 누적 diff 안내를 가짜 파일로 세지 않고 두 파일을 탐색하며 빈 갱신은 이전 파일을 제거한다.
#[test]
fn aggregate_diff_preamble_does_not_create_a_spurious_file_section() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::FileChange,
        })
        .unwrap();
    state.observe(AgentEvent::ActivityUpdated { activity: activity(1), update: ActivityUpdate::TextSnapshot("Turn aggregate diff\ndiff --git a/a.rs b/a.rs\n-old\n+new\ndiff --git a/b.rs b/b.rs\n-b\n+c\n".into()) }).unwrap();
    state
        .handle(InputEvent::Paste("/changes".into()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let first = render_and_commit(&mut state, Size::new(80, 15));
    assert!(first.contains("1/2"), "{first}");
    assert!(first.contains("Turn aggregate diff"), "{first}");
    assert!(first.contains("a/a.rs"), "{first}");
    state
        .handle(key(KeyCode::Right, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let second = render_and_commit(&mut state, Size::new(24, 15));
    assert!(second.contains("2/2"), "{second}");
    assert!(second.contains("a/b.rs"), "{second}");
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(
                "Turn aggregate diff\nNo remaining changes reported for this turn.".into(),
            ),
        })
        .unwrap();
    let empty = render_and_commit(&mut state, Size::new(80, 15));
    assert!(empty.contains("No remaining changes"), "{empty}");
    assert!(!empty.contains("a/b.rs"));
}

// 채팅의 짧은 결과와 보존 원문을 구분하고, 뷰어에서 모델 결과에 없는 중간·마지막 행을 읽는다.
#[test]
fn retained_output_view_reads_the_capture_instead_of_the_model_preview() {
    let mut state = TuiState::new();
    let output = ToolOutput {
        tool: "run_command".into(),
        server: None,
        arguments: None,
        result: Some(
            json!({"content":[{"type":"text","text":"short model preview"}],"truncated":true,"retainedOutput":{"truncated":false}}),
        ),
        content_items: None,
        error: None,
        plain_text: (1..=120)
            .map(|row| format!("retained row {row:03}"))
            .collect::<Vec<_>>()
            .join("\n"),
    };
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolResult,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
        })
        .unwrap();
    let chat = render_and_commit(&mut state, Size::new(80, 25));
    assert!(chat.contains("short model preview"), "{chat}");
    assert!(chat.contains("Retained output"), "{chat}");
    assert!(chat.contains("/output"), "{chat}");
    assert!(!chat.contains("retained row 060"));
    state
        .handle(InputEvent::Paste("/output".into()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let first = render_and_commit(&mut state, Size::new(24, 4));
    assert!(first.contains("retained row 001"), "{first}");
    assert!(!first.contains("Partial"), "{first}");
    for _ in 0..20 {
        state
            .handle(key(KeyCode::PageDown, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
    }
    let middle = render_and_commit(&mut state, Size::new(24, 4));
    assert!(middle.contains("retained row 061"), "{middle}");
    state
        .handle(key(KeyCode::End, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let last = render_and_commit(&mut state, Size::new(80, 4));
    assert!(last.contains("retained row 120"), "{last}");
    assert!(!last.contains("Partial"), "{last}");
}

// 7만 행 diff는 Chat을 깨뜨리지 않고 전용 뷰어에서 마지막 행까지 읽으며, 실패한 frame 뒤에도
// 이동이 남고 파일 전환·새 snapshot의 끝 따라가기가 각각 유지된다.
#[test]
fn changes_pages_beyond_u16_and_preserves_file_and_snapshot_navigation() {
    let mut state = TuiState::new();
    let first = format!(
        "update: huge.rs\n@@ -0,0 +1,70000 @@\n{}",
        (0..70000)
            .map(|row| format!("+retained {row:05}\n"))
            .collect::<String>()
    );
    let source = format!("{first}update: second.rs\n+second file\n");
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::FileChange,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(source),
        })
        .unwrap();
    let chat = render_and_commit(&mut state, Size::new(80, 20));
    assert!(chat.contains("Large diff"), "{chat}");
    assert!(chat.contains("/changes"), "{chat}");
    state
        .handle(InputEvent::Paste("/changes".into()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let top = render_and_commit(&mut state, Size::new(80, 12));
    assert!(top.contains("1/2"), "{top}");
    assert!(top.contains("huge.rs"), "{top}");
    state
        .handle(key(KeyCode::End, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    assert!(
        state
            .prepare_frame(Size::new(80, 0), &AppearanceState::default().pin())
            .is_err()
    );
    let tail = render_and_commit(&mut state, Size::new(24, 12));
    assert!(tail.contains("+retained 69999"), "{tail}");
    let updated = format!("{first}+new appended row\nupdate: second.rs\n+second file\n");
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(updated),
        })
        .unwrap();
    let appended = render_and_commit(&mut state, Size::new(80, 12));
    assert!(appended.contains("+new appended row"), "{appended}");
    state
        .handle(key(KeyCode::Right, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let second = render_and_commit(&mut state, Size::new(24, 12));
    assert!(second.contains("2/2"), "{second}");
    assert!(second.contains("+second file"), "{second}");
    state
        .handle(key(KeyCode::Left, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let restored = render_and_commit(&mut state, Size::new(80, 12));
    assert!(restored.contains("huge.rs"), "{restored}");
    assert!(!restored.contains("new appended row"));
}

// inline 예약 공간을 제외한 마지막 허용 행은 접힌 미리보기를 유지하고, 첫 초과 행부터
// 전체 원문을 버리지 않는 /changes 안내로 전환한다.
#[test]
fn inline_diff_capacity_keeps_the_last_row_and_routes_the_first_excess_to_review() {
    for (rows, large) in [
        (usize::from(u16::MAX) - 256, false),
        (usize::from(u16::MAX) - 255, true),
    ] {
        let mut state = TuiState::new();
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::FileChange,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot("+x\n".repeat(rows)),
            })
            .unwrap();
        let frame = render_and_commit(&mut state, Size::new(80, 12));
        assert_eq!(frame.contains("Large diff"), large, "{frame}");
        if large {
            assert!(frame.contains("/changes"), "{frame}");
        } else {
            assert!(frame.contains("+x"), "{frame}");
        }
    }
}

// 두 diff를 전체 펼쳤을 때 누적 8만 행을 넘겨도 실제 Chat frame·history 위치·End가 유지된다.
#[test]
fn expanded_chat_keeps_large_accumulated_diffs_scrollable() {
    let mut state = TuiState::new();
    for number in 1..=2 {
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(number),
                kind: ActivityKind::FileChange,
            })
            .unwrap();
        let source = format!(
            "update: file{number}.rs\n{}",
            (0..40000)
                .map(|row| format!("+file{number} row{row:05}\n"))
                .collect::<String>()
        );
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(number),
                update: ActivityUpdate::TextSnapshot(source),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(number),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
    }
    render_and_commit(&mut state, Size::new(80, 20));
    state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::CONTROL),
            Duration::ZERO,
        )
        .unwrap();
    let expanded = render_and_commit(&mut state, Size::new(80, 20));
    assert!(expanded.contains("file2 row39999"), "{expanded}");
    assert!(state.views().view_positions().0 > usize::from(u16::MAX));
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let detached = render_and_commit(&mut state, Size::new(24, 20));
    assert!(detached.contains("History"), "{detached}");
    assert!(!detached.contains("row39999"), "{detached}");
    state
        .handle(key(KeyCode::End, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let tail = render_and_commit(&mut state, Size::new(80, 20));
    assert!(tail.contains("file2 row39999"), "{tail}");
}

// Alt 방향키를 한 frame에 모아도 항목 이동 순서와 프롬프트 초안을 보존한다.
#[test]
fn alt_arrows_navigate_chat_items_without_editing_the_prompt() {
    fn seeded() -> TuiState {
        let mut state = TuiState::new();
        for index in 1..=4 {
            state
                .observe(AgentEvent::ActivityStarted {
                    activity: activity(index),
                    kind: ActivityKind::ToolCall,
                })
                .unwrap();
            state
                .observe(AgentEvent::ActivityUpdated {
                    activity: activity(index),
                    update: ActivityUpdate::TextSnapshot(format!(
                        "ITEM{index}\none\ntwo\nthree\nfour\nfive\nsix"
                    )),
                })
                .unwrap();
        }
        state
            .handle(InputEvent::Paste("keep draft 한글".into()), Duration::ZERO)
            .unwrap();
        state
    }
    for width in [80, 24] {
        let size = Size::new(width, 12);
        let mut burst = seeded();
        let mut serial = seeded();
        for state in [&mut burst, &mut serial] {
            state
                .handle(key(KeyCode::Home, KeyModifiers::NONE), Duration::ZERO)
                .unwrap();
            render_and_commit(state, size);
        }
        for code in [KeyCode::Down, KeyCode::Down, KeyCode::Up] {
            for state in [&mut burst, &mut serial] {
                assert_eq!(
                    state
                        .handle(key(code, KeyModifiers::ALT), Duration::ZERO)
                        .unwrap(),
                    StateEffect::Redraw
                );
            }
            render_and_commit(&mut serial, size);
        }
        let actual = render_and_commit(&mut burst, size);
        assert_eq!(actual, render_and_commit(&mut serial, size));
        assert!(actual.contains("ITEM2"), "{actual}");
        assert_eq!(burst.editor().text(), "keep draft 한글");
    }
}
