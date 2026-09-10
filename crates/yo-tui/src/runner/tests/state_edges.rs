use yo_core::{
    AgentCommand, AgentSession, BackendScriptStep, DurabilityGapCause, JournalDurability,
    ScriptedBackend, TranscriptRecord, UserInput,
    session_repository::{DurableCutoff, RepositorySequence},
};

use super::turn;
use crate::{
    runner::state::{StateEffect, StateError, TuiState},
    transcript::TranscriptBody,
};

// 저장 실패를 화면에서 숨기지 않고 원인·확실한 저장 경계를 알린다. 같은 알림을 중복하지
// 않으며 실제 Durable 복구 뒤에만 저장 완료를 알리고 정상 append마다 알림을 만들지 않는다.
#[test]
fn storage_failure_discloses_its_cutoff_until_authoritative_recovery() {
    let session_id = turn().session_id();
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession { session_id }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut session = AgentSession::start_cancellable_with_id(backend, session_id, || false)
        .unwrap()
        .unwrap();
    let reader = session.transcript_reader();
    let entries = reader.read_after(None);
    assert_eq!(entries.entries().len(), 2);
    let saved_sequence = entries.entries()[0].sequence();
    let recovered_sequence = reader.head_sequence().unwrap();
    session.shutdown().unwrap();
    assert!(saved_sequence < recovered_sequence);
    let saved_notice = format!(
        "Saved through event {} (storage record 3).",
        saved_sequence.get()
    );
    for (cause, cutoff, reason, saved) in [
        (
            DurabilityGapCause::Capacity,
            DurableCutoff::KnownEmpty,
            "History storage is full.",
            "No session history has been saved.",
        ),
        (
            DurabilityGapCause::Storage,
            DurableCutoff::Known {
                journal_sequence: Some(saved_sequence),
                repository_sequence: RepositorySequence::new(3),
            },
            "History storage is unavailable.",
            saved_notice.as_str(),
        ),
        (
            DurabilityGapCause::Storage,
            DurableCutoff::Known {
                journal_sequence: None,
                repository_sequence: RepositorySequence::new(1),
            },
            "History storage is unavailable.",
            "Only session metadata is saved (storage record 1).",
        ),
        (
            DurabilityGapCause::Integrity,
            DurableCutoff::Unknown,
            "A history record failed validation.",
            "The last saved point could not be verified.",
        ),
    ] {
        let mut state = TuiState::new();
        let gap = JournalDurability::Gap {
            durable_cutoff: cutoff,
            cause,
        };
        assert_eq!(state.observe_durability(gap).unwrap(), StateEffect::Redraw);
        assert_eq!(state.durability(), Some(gap));
        assert_eq!(state.transcript().items().len(), 1);
        let TranscriptBody::Message(notice) = state.transcript().items()[0].body();
        assert!(notice.text().contains(reason));
        assert!(notice.text().contains(saved));
        assert!(notice.text().contains("New activity stays in memory"));
        assert_eq!(
            state.observe_durability(gap).unwrap(),
            StateEffect::Unchanged
        );
        assert_eq!(state.transcript().items().len(), 1);
        let recovered = JournalDurability::Durable {
            journal_sequence: Some(recovered_sequence),
            repository_sequence: RepositorySequence::new(9),
        };
        assert_eq!(
            state.observe_durability(recovered).unwrap(),
            StateEffect::Redraw
        );
        assert_eq!(state.transcript().items().len(), 2);
        let TranscriptBody::Message(notice) = state.transcript().items()[1].body();
        assert!(
            notice
                .text()
                .contains("The complete session has been saved")
        );
        assert_eq!(
            state
                .observe_durability(JournalDurability::Durable {
                    journal_sequence: Some(recovered_sequence),
                    repository_sequence: RepositorySequence::new(10),
                })
                .unwrap(),
            StateEffect::Unchanged
        );
        assert_eq!(state.transcript().items().len(), 2);
    }
}

// 저널에 확정된 사용자 명령을 표시할 transcript ID가 더는 증가할 수 없으면 중복 ID로
// 일부만 넣지 않고 실패한다.
#[test]
fn item_id_overflow_preserves_empty_transcript() {
    let mut state = TuiState::new();
    state.set_next_item_id(u64::MAX);

    assert_eq!(
        state.observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("질문"),
            },
        )),
        Err(StateError::ItemIdOverflow)
    );
    assert!(state.transcript().items().is_empty());
}
