use std::num::NonZeroU64;

use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    AgentEvent, HostWorkspacePath, SessionDescriptor, SessionId, TranscriptRecord, TurnId, TurnRef,
    UserInput, WorkspaceHostId,
    session_repository::{StoredSessionContinuity, StoredSessionRecovery},
};

use super::{project_chat, project_transcript_parts};
use crate::GlyphProfile;

fn history() -> (SessionDescriptor, Vec<TranscriptRecord>) {
    let session_id: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let host: WorkspaceHostId = "10000000-0000-4000-8000-000000000001".parse().unwrap();
    let descriptor = SessionDescriptor::for_session(
        session_id,
        host,
        HostWorkspacePath::normalize_local(std::env::current_dir().unwrap()).unwrap(),
    );
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(1).unwrap()));
    let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    (
        descriptor,
        vec![
            TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
                turn,
                input: UserInput::new("질문"),
            }),
            TranscriptRecord::EventCommitted(AgentEvent::TurnStarted { turn }),
            TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted {
                activity,
                kind: ActivityKind::AgentMessage,
            }),
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot("답변".to_owned()),
            }),
            TranscriptRecord::EventCommitted(AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            }),
        ],
    )
}

// 같은 저장 history를 Chat으로 보면 사용자/에이전트 본문만 간결하게 나오고 저장 schema나
// 물리 record 진단은 섞이지 않아 stdout을 그대로 읽거나 pipe로 넘길 수 있다.
#[test]
fn archived_chat_reuses_the_plain_chat_projection() {
    let (_, records) = history();
    let output = project_chat(&records, GlyphProfile::Rich).unwrap();

    assert!(output.contains("질문"));
    assert!(output.contains("답변"));
    assert!(!output.contains("journal_cutoff"));
}

// Transcript view는 live reader의 "metadata unavailable" 문구를 재사용하지 않고, 저장
// 스냅샷에서 실제로 검증한 cutoff/recovery/discovery와 각 semantic record를 함께 보여준다.
#[test]
fn archived_transcript_labels_the_durable_observation_boundary() {
    let (descriptor, records) = history();
    let output = project_transcript_parts(
        &descriptor,
        None,
        StoredSessionRecovery::NotRequired,
        StoredSessionContinuity::NotObservable,
        true,
        &records,
    );

    assert!(output.contains("journal_cutoff=descriptor-only"));
    assert!(output.contains("message_recovery=not-required"));
    assert!(output.contains("durability_continuity=not-observable"));
    assert!(output.contains("discovery=consistent"));
    assert!(output.contains("[#001] command.start_turn"));
    assert!(!output.contains("metadata, and Request Audit detail are unavailable"));
}
