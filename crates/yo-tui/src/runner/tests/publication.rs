use std::time::Duration;

use yo_core::{
    ActivityKind, ActivityOutcome, AgentCommand, AgentEvent, TranscriptRecord, UserInput,
};

use super::{activity, key, turn};
use crate::{
    appearance::AppearanceState,
    input::event::{KeyCode, KeyModifiers},
    runner::{PresentationMode, state::TuiState},
    surface::{CellContent, Point, Size, Surface},
};

// Final user 항목 뒤 Streaming assistant가 있으면 준비 단계는 Final prefix만 persistent
// 후보로 만들고 cursor를 전진시키지 않는다. receipt acknowledgement 뒤에만 종료 suffix가
// Streaming 항목부터 시작한다.
#[test]
fn final_prefix_waits_for_acknowledgement_and_stops_before_streaming() {
    let mut state = TuiState::new();
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("question"),
            },
        ))
        .unwrap();
    let work = activity(1);
    state
        .observe(AgentEvent::ActivityStarted {
            activity: work,
            kind: ActivityKind::ModelWork,
        })
        .unwrap();
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("later final"),
            },
        ))
        .unwrap();
    let appearance = AppearanceState::default().pin();

    let frame = state
        .prepare_frame_for_geometry(Size::new(32, 20), &appearance, Duration::ZERO, 7)
        .unwrap();
    let publication = frame
        .publication
        .as_ref()
        .expect("the Final user item is eligible");

    assert_eq!(plain_surface(&publication.surface), "❯ question");
    assert!(plain_surface(&frame.surface).contains("• Thinking…"));
    assert!(plain_surface(&frame.surface).contains("❯ later final"));
    assert!(frame.surface.size().height < 20);
    assert_eq!(publication.observed_terminal_size, Size::new(32, 20));
    assert_eq!(publication.geometry_epoch, 7);
    assert_eq!(
        state.session_output(&appearance).unwrap().unwrap(),
        "❯ question\n\n• Thinking…\n\n\n❯ later final\n"
    );

    assert!(state.acknowledge_publication(&frame));
    assert!(!state.acknowledge_publication(&frame));
    assert_eq!(
        state.session_output(&appearance).unwrap().unwrap(),
        "\n• Thinking…\n\n\n❯ later final\n"
    );
    let blocked = state
        .prepare_frame_for_geometry(Size::new(32, 20), &appearance, Duration::ZERO, 7)
        .unwrap();
    assert!(
        blocked.publication.is_none(),
        "the acknowledged item must not be selected again across a Streaming barrier"
    );

    state
        .observe(AgentEvent::ActivityFinished {
            activity: work,
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();
    let next = state
        .prepare_frame_for_geometry(Size::new(32, 20), &appearance, Duration::ZERO, 7)
        .unwrap();
    assert_eq!(
        plain_surface(&next.publication.as_ref().unwrap().surface),
        "\n• Thinking…\n\n\n❯ later final"
    );
}

// Chat를 tail에서 떼어 검토하면 publication을 중지하고 terminal 전체 높이를 계속 사용한다.
// 같은 상태가 다음 frame에도 유지되어 review 중 과거 항목이 native history로 빠지지 않는다.
#[test]
fn detached_chat_freezes_publication_and_keeps_the_review_viewport() {
    let mut state = TuiState::new();
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from(
                    (0..20)
                        .map(|row| format!("row {row}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            },
        ))
        .unwrap();
    let appearance = AppearanceState::default().pin();
    let initial = state.prepare_frame(Size::new(20, 8), &appearance).unwrap();
    state.commit_frame(&initial);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();

    let detached = state
        .prepare_frame_for_geometry(Size::new(20, 8), &appearance, Duration::ZERO, 1)
        .unwrap();
    assert!(detached.publication.is_none());
    assert_eq!(detached.surface.size(), Size::new(20, 8));
    state.commit_frame(&detached);

    let retained = state
        .prepare_frame_for_geometry(Size::new(20, 8), &appearance, Duration::ZERO, 1)
        .unwrap();
    assert!(retained.publication.is_none());
    assert_eq!(retained.surface.size(), Size::new(20, 8));

    state
        .handle(key(KeyCode::PageDown, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let reaches_tail = state
        .prepare_frame_for_geometry(Size::new(20, 8), &appearance, Duration::ZERO, 1)
        .unwrap();
    assert!(reaches_tail.publication.is_none());
    assert!(reaches_tail.reprepare_for_publication);
    state.commit_frame(&reaches_tail);
    let compact = state
        .prepare_frame_for_geometry(Size::new(20, 8), &appearance, Duration::ZERO, 1)
        .unwrap();
    assert!(compact.publication.is_some());
}

// Inline에서 이미 acknowledge한 항목도 Fullscreen으로 전환하면 complete semantic history에서
// 다시 그리며 Inline cursor를 더 전진시키거나 compact live height를 사용하지 않는다.
#[test]
fn fullscreen_ignores_the_inline_publication_cursor() {
    let mut state = TuiState::new();
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("published"),
            },
        ))
        .unwrap();
    let appearance = AppearanceState::default().pin();
    let inline = state
        .prepare_frame_for_geometry(Size::new(24, 12), &appearance, Duration::ZERO, 0)
        .unwrap();
    assert!(state.acknowledge_publication(&inline));

    state.set_presentation_mode(PresentationMode::Fullscreen);
    let fullscreen = state
        .prepare_frame_for_geometry(Size::new(24, 12), &appearance, Duration::ZERO, 0)
        .unwrap();

    assert!(fullscreen.publication.is_none());
    assert_eq!(fullscreen.surface.size(), Size::new(24, 12));
    assert!(plain_surface(&fullscreen.surface).contains("❯ published"));
}

// Transcript와 Request는 읽기 전용 전체 화면이므로 Inline publication cursor를 사용하지
// 않는다. 두 view를 왕복해도 cursor는 전진하지 않으며 Chat으로 돌아왔을 때 같은 Final
// 항목이 처음으로 persistent 후보가 된다.
#[test]
fn read_only_views_freeze_publication_until_chat_returns() {
    let mut state = TuiState::new();
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("still unpublished"),
            },
        ))
        .unwrap();
    let appearance = AppearanceState::default().pin();

    for mode in [2, 3] {
        state
            .handle(
                key(KeyCode::Function(mode), KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap();
        let frame = state
            .prepare_frame_for_geometry(Size::new(28, 10), &appearance, Duration::ZERO, 0)
            .unwrap();
        assert!(frame.publication.is_none());
        assert_eq!(frame.surface.size(), Size::new(28, 10));
        state.commit_frame(&frame);
    }

    state
        .handle(
            key(KeyCode::Function(1), KeyModifiers::NONE),
            Duration::ZERO,
        )
        .unwrap();
    let chat = state
        .prepare_frame_for_geometry(Size::new(28, 10), &appearance, Duration::ZERO, 0)
        .unwrap();
    assert_eq!(
        plain_surface(&chat.publication.as_ref().unwrap().surface),
        "❯ still unpublished"
    );
}

fn plain_surface(surface: &Surface) -> String {
    (0..surface.size().height)
        .map(|row| {
            (0..surface.size().width)
                .map(|column| {
                    match surface
                        .cell(Point::new(column, row))
                        .expect("the point is inside the Surface")
                        .content()
                    {
                        CellContent::Blank | CellContent::Continuation { .. } => " ",
                        CellContent::Grapheme { text, .. } => text,
                    }
                })
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}
