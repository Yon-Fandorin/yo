use yo_core::{
    AgentCommand, DurabilityGapCause, JournalDurability, TranscriptRecord, UserInput,
    session_repository::DurableCutoff,
};

use super::turn;
use crate::runner::state::{StateEffect, StateError, TuiState};

// 저장 공간 압력의 구체적인 화면 표현은 별도 SOT가 소유한다. 이 단계에서는 typed cutoff를
// 잃지 않고 TUI 상태까지 전달해 이후 presenter가 Chat·status·banner 정책을 선택할 수 있다.
#[test]
fn retains_storage_pressure_for_a_future_presentation_policy() {
    let mut state = TuiState::new();
    let durability = JournalDurability::Gap {
        durable_cutoff: DurableCutoff::KnownEmpty,
        cause: DurabilityGapCause::Capacity,
    };

    assert_eq!(
        state.observe_durability(durability).unwrap(),
        StateEffect::Unchanged
    );
    assert_eq!(state.durability(), Some(durability));
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
