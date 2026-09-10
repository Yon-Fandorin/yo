use super::{activity, engine_with_active_turn, id, session, turn};
use crate::{
    ActivityKind, ActivityOutcome, ActivityResponse, AgentCommand, AgentEngine, AgentEvent,
    AgentRejection, ApprovalDecision, Failure, RequestId, TurnOutcome, UserInput,
};

// 세션과 Turn을 시작하면 상태가 보존되고 프런트엔드가 그대로 전달할 상관관계 이벤트가 생성되는지
// 확인한다.
#[test]
fn creates_a_session_and_starts_its_first_turn() {
    let session_id = session(1);
    let first_turn = turn(session_id, 1);
    let mut engine = AgentEngine::new();

    let created = engine
        .handle_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    let started = engine
        .handle_command(AgentCommand::StartTurn {
            turn: first_turn,
            input: UserInput::from("inspect"),
        })
        .unwrap();

    assert_eq!(created, vec![AgentEvent::SessionCreated { session_id }]);
    assert_eq!(started, vec![AgentEvent::TurnStarted { turn: first_turn }]);
    assert_eq!(engine.session_id(), Some(session_id));
    assert_eq!(engine.active_turn(), Some(first_turn));
    assert_eq!(
        engine.active_turn_input().map(UserInput::as_str),
        Some("inspect")
    );
    assert_eq!(engine.turn_count(), 1);
}

// Activity의 시작·증분·종료가 같은 참조를 유지하고 종료 뒤 추가 증분은 거절되는지 확인한다.
#[test]
fn tracks_an_activity_through_its_lifecycle() {
    let (mut engine, active_turn) = engine_with_active_turn();
    let activity = activity(active_turn, 1);

    let started = engine
        .start_activity(activity, ActivityKind::AgentMessage)
        .unwrap();
    let updated = engine
        .update_activity(
            activity,
            crate::ActivityUpdate::TextDelta("working".to_owned()),
        )
        .unwrap();
    let finished = engine
        .finish_activity(activity, ActivityOutcome::Completed)
        .unwrap();
    let rejection = engine
        .update_activity(
            activity,
            crate::ActivityUpdate::TextDelta("late".to_owned()),
        )
        .unwrap_err();

    assert_eq!(
        started,
        AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        }
    );
    assert_eq!(
        updated,
        AgentEvent::ActivityUpdated {
            activity,
            update: crate::ActivityUpdate::TextDelta("working".to_owned()),
        }
    );
    assert_eq!(
        finished,
        AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        }
    );
    assert_eq!(rejection, AgentRejection::ActivityNotActive { activity });
}

// 정상 완료는 요청 응답 명령과 상관관계 응답 Activity가 모두 끝난 뒤에만 허용하는지 단계별로
// 확인한다.
#[test]
fn completes_a_turn_only_after_its_request_response_cycle_closes() {
    let (mut engine, active_turn) = engine_with_active_turn();
    let request_activity = activity(active_turn, 1);
    let response_activity = activity(active_turn, 2);
    let request_id = RequestId::new(id(1));
    let request = crate::ActivityRequestRef::new(request_activity, request_id);
    engine
        .start_activity(
            request_activity,
            ActivityKind::ApprovalRequest { request_id },
        )
        .unwrap();
    engine
        .finish_activity(request_activity, ActivityOutcome::Completed)
        .unwrap();

    let unanswered = engine
        .finish_turn(active_turn, TurnOutcome::Completed)
        .unwrap_err();
    engine
        .handle_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Approved),
        })
        .unwrap();
    let not_recorded = engine
        .finish_turn(active_turn, TurnOutcome::Completed)
        .unwrap_err();
    engine
        .start_activity(
            response_activity,
            ActivityKind::ApprovalResponse { request_id },
        )
        .unwrap();
    engine
        .finish_activity(response_activity, ActivityOutcome::Completed)
        .unwrap();
    let finished = engine
        .finish_turn(active_turn, TurnOutcome::Completed)
        .unwrap();

    assert_eq!(
        unanswered,
        AgentRejection::RequestStillUnanswered { request }
    );
    assert_eq!(
        not_recorded,
        AgentRejection::ResponseNotRecorded { request }
    );
    assert_eq!(
        finished,
        AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Completed,
        }
    );
    assert_eq!(engine.active_turn(), None);
}

// 실패 종료는 백엔드 단절처럼 응답을 받을 수 없는 상황을 표현하므로 미해결 요청을 남기고도 닫히는지
// 확인한다.
#[test]
fn failed_turn_may_abandon_an_unanswered_request() {
    let (mut engine, active_turn) = engine_with_active_turn();
    let request_activity = activity(active_turn, 1);
    let request_id = RequestId::new(id(1));
    engine
        .start_activity(
            request_activity,
            ActivityKind::ApprovalRequest { request_id },
        )
        .unwrap();
    engine
        .finish_activity(request_activity, ActivityOutcome::Completed)
        .unwrap();
    let failure = Failure::new("backend disconnected");

    let finished = engine
        .finish_turn(active_turn, TurnOutcome::Failed(failure.clone()))
        .unwrap();

    assert_eq!(
        finished,
        AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Failed(failure),
        }
    );
    assert_eq!(engine.active_turn(), None);
}

// InterruptTurn은 요청만 기록하고 backend가 Activity와 Turn의 실제 중단을 알린 뒤에야 닫히는지
// 확인한다.
#[test]
fn interruption_waits_for_backend_terminal_events() {
    let (mut engine, active_turn) = engine_with_active_turn();
    let first = activity(active_turn, 1);
    let already_finished = activity(active_turn, 2);
    engine
        .start_activity(first, ActivityKind::ModelWork)
        .unwrap();
    engine
        .start_activity(already_finished, ActivityKind::ToolResult)
        .unwrap();
    engine
        .finish_activity(already_finished, ActivityOutcome::Completed)
        .unwrap();

    let immediate = engine
        .handle_command(AgentCommand::InterruptTurn { turn: active_turn })
        .unwrap();
    assert!(immediate.is_empty());
    assert_eq!(engine.active_turn(), Some(active_turn));

    let activity_finished = engine
        .finish_activity(first, ActivityOutcome::Interrupted)
        .unwrap();
    let turn_finished = engine
        .finish_turn(active_turn, TurnOutcome::Interrupted)
        .unwrap();

    assert_eq!(
        activity_finished,
        AgentEvent::ActivityFinished {
            activity: first,
            outcome: ActivityOutcome::Interrupted,
        }
    );
    assert_eq!(
        turn_finished,
        AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Interrupted,
        }
    );
    assert_eq!(engine.active_turn(), None);
}

// 실제 fork bootstrap을 codec으로 복구한 뒤에도 parent의 Turn이나 합성 명령 없이 child의
// 첫 실제 입력을 실행하며, 원본 Journal은 그대로 유지합니다.
#[test]
fn restores_fork_bootstrap_and_starts_a_real_child_turn() {
    let recovered = fork_bootstrap();
    let entries = recovered.semantic_entries();
    let before = entries.clone();
    let child = recovered.descriptor().unwrap().session_id();
    let mut engine = AgentEngine::from_journal(&entries, false).unwrap();
    assert_eq!(engine.session_id(), Some(child));
    assert_eq!(engine.turn_count(), 0);
    assert_eq!(engine.active_turn(), None);
    assert!(entries.iter().all(|entry| !matches!(
        entry.record(),
        crate::journal::SemanticRecord::CommandCommitted(_)
    )));

    let first = turn(child, 1);
    let events = engine
        .handle_command(AgentCommand::StartTurn {
            turn: first,
            input: UserInput::from("continue in the child"),
        })
        .unwrap();
    assert_eq!(events, vec![AgentEvent::TurnStarted { turn: first }]);
    assert_eq!(engine.active_turn(), Some(first));
    assert_eq!(engine.turn_count(), 1);
    assert_eq!(
        engine.active_turn_input().map(UserInput::as_str),
        Some("continue in the child")
    );
    assert_eq!(entries, before);
}

// seed나 initial binding이 빠진 SessionCreated, 또는 seed의 parent를 child로 가장한 이벤트는
// 일반 lifecycle 명령 없이 엔진 상태를 생성할 수 없습니다.
#[test]
fn rejects_unmatched_or_incomplete_fork_session_creation() {
    let recovered = fork_bootstrap();
    let entries = recovered.semantic_entries();
    for incomplete in [&entries[..1], &entries[..2]] {
        assert!(
            AgentEngine::from_journal(incomplete, false)
                .unwrap_err()
                .contains("unexpected lifecycle event")
        );
    }
    let mut foreign = crate::journal::SessionJournal::new();
    foreign.append_events(&[AgentEvent::SessionCreated {
        session_id: session(81),
    }]);
    let mut foreign_entries = foreign.semantic_entries();
    foreign_entries.extend_from_slice(&entries[1..]);
    assert!(
        AgentEngine::from_journal(&foreign_entries, false)
            .unwrap_err()
            .contains("unexpected lifecycle event")
    );

    let mut ordinary = crate::journal::SessionJournal::new();
    ordinary.append_committed_command(
        AgentCommand::CreateSession {
            session_id: session(82),
        },
        &[AgentEvent::SessionCreated {
            session_id: session(82),
        }],
    );
    let engine = AgentEngine::from_journal(&ordinary.semantic_entries(), false).unwrap();
    assert_eq!(engine.session_id(), Some(session(82)));
    assert_eq!(engine.turn_count(), 0);
}

fn fork_bootstrap() -> crate::journal::codec::RecoveredJournal {
    use crate::{
        BackendBindingEvidence, BackendIdentity, ContinuationStrategy, JournalSequence,
        ModelReplayContract, ModelReplayItem, ModelReplayRole, ReplayExecutor, ReplayProfile,
        journal::codec::{
            BackendBindingOpened, BindingTransition, ContextPolicyChanged, ContextStrategy,
            ForkExactReplay, ForkGroup, ForkItemCoordinate, ForkItemOrigin, ForkSeed, ForkSource,
            ForkSourcePoint, InitialForkSeed, JournalCommit, JournalRecord, ReplaySequence,
            SequencedJournalRecord, VersionedIdentity, decode, encode, recover,
        },
    };
    let parent = session(81);
    let child = session(82);
    let source = BackendBindingEvidence::new(
        "managed",
        "1.0.0",
        BackendIdentity::new("binding/v1", "account"),
        BackendIdentity::new("model/v1", "model"),
        BackendIdentity::new("locator/v1", "parent"),
        ContinuationStrategy::ExactReplay {
            executor: ReplayExecutor::LocalClient,
            replay_profile: ReplayProfile::SemanticOnly,
        },
    );
    let origin = ForkItemCoordinate::new(parent, 3, 2, JournalSequence::new(7), 0).unwrap();
    let exact = ForkExactReplay::new(
        ModelReplayContract::new("system", vec![]),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "inherited context".into(),
            refusal: None,
        }],
        vec![ForkItemOrigin::new(origin, origin, source.clone()).unwrap()],
        vec![ForkGroup::new(0, 1).unwrap()],
    )
    .unwrap();
    let seed = InitialForkSeed::new(
        child,
        parent,
        ForkSource::Anchor(
            ForkSourcePoint::new(
                3,
                2,
                JournalSequence::new(9),
                JournalSequence::new(8),
                source.clone(),
            )
            .unwrap(),
        ),
        ForkSeed::ExactReplay(exact),
        vec![],
        2,
    )
    .unwrap();
    let records = vec![
        JournalRecord::SessionDescriptor(crate::fixture_descriptor(child)),
        JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id: child }),
        JournalRecord::InitialForkSeed(Box::new(seed)),
        JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
            1,
            source.backend_kind(),
            source.backend_version(),
            VersionedIdentity::new("binding/v1", "account"),
            VersionedIdentity::new("model/v1", "model"),
            VersionedIdentity::new("locator/v1", "child"),
            BindingTransition::initial_fork(JournalSequence::new(2)),
            source.continuation_strategy(),
        )),
        JournalRecord::ContextPolicyChanged(
            ContextPolicyChanged::try_new(
                1,
                true,
                ContextStrategy::PortableSummaryV1Alpha1,
                85,
                90,
                Some(10),
                Some(65_536),
            )
            .unwrap(),
        ),
    ];
    let records = records
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            let replay_sequence = ReplaySequence::new(index as u64 + 1);
            if index == 0 {
                SequencedJournalRecord::storage(replay_sequence, record)
            } else {
                SequencedJournalRecord::with_journal_sequence(
                    replay_sequence,
                    JournalSequence::new(index as u64),
                    record,
                )
            }
        })
        .collect();
    let commit = JournalCommit::incremental_through(JournalSequence::new(4), records);
    recover(&[decode(&encode(&commit).unwrap()).unwrap()]).unwrap()
}
