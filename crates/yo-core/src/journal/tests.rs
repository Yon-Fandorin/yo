use std::num::NonZeroU64;

use super::{SemanticRecord, SessionJournal};
use crate::{AgentCommand, AgentEvent, SessionId, TurnId, TurnRef, UserInput};

fn session(value: u64) -> SessionId {
    SessionId::new(NonZeroU64::new(value).unwrap())
}

fn turn(session_id: SessionId, value: u64) -> TurnRef {
    TurnRef::new(session_id, TurnId::new(NonZeroU64::new(value).unwrap()))
}

// 한 명령이 만든 의미 이벤트를 함께 기록하면 명령이 먼저 나오고 이어지는 이벤트들이
// 같은 batch 순서를 유지해야 replay가 원래의 원인과 결과를 복원할 수 있다.
#[test]
fn appends_a_committed_command_before_its_committed_events() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let command = AgentCommand::StartTurn {
        turn,
        input: UserInput::new("inspect the repository"),
    };
    let events = vec![AgentEvent::TurnStarted { turn }];
    let mut journal = SessionJournal::new();

    journal.append_committed_command(command.clone(), &events);

    assert_eq!(journal.entries().len(), 2);
    assert_eq!(
        journal.entries()[0].record(),
        &SemanticRecord::CommandCommitted(command)
    );
    assert_eq!(
        journal.entries()[1].record(),
        &SemanticRecord::EventCommitted(events[0].clone())
    );
}

// 서로 다른 append에서 추가된 기록도 1부터 시작하는 하나의 연속 sequence를 공유해야
// frontend와 저장소가 누락이나 중복 없이 suffix를 요청할 수 있다.
#[test]
fn assigns_one_contiguous_sequence_across_append_boundaries() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let mut journal = SessionJournal::new();

    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );
    journal.append_events(&[AgentEvent::TurnStarted { turn }]);

    let sequences = journal
        .entries()
        .iter()
        .map(|entry| entry.sequence().get())
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![1, 2, 3]);
}

// 빈 event batch는 sequence를 소비하거나 가짜 기록을 만들지 않아야 다음 실제 의미
// 이벤트의 번호가 이전 기록 바로 다음으로 이어진다.
#[test]
fn empty_event_batches_do_not_create_observations_or_sequence_gaps() {
    let session_id = session(1);
    let mut journal = SessionJournal::new();

    journal.append_events(&[]);
    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );

    let sequences = journal
        .entries()
        .iter()
        .map(|entry| entry.sequence().get())
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![1, 2]);
}
