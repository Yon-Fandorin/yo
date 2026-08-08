use std::num::NonZeroU64;

use uuid::Builder;

use super::*;
use crate::{
    AgentCommand, AgentEvent, JournalSequence, SubmissionId, TurnId, TurnOutcome, TurnRef,
    UserInput,
    journal::{
        CommittedCommand,
        codec::{
            BackendBindingClosed, BackendBindingOpened, BackendExchangeObserved,
            BackendRequestAccepted, BackendResumableOutcome, BindingCloseReason, BindingTransition,
            CacheState, ContinuationAnchor, DetailAvailability, ExchangeDirection, ExchangeKind,
            JournalCommit, JournalRecord, OperationId, ReplaySequence, SequencedJournalRecord,
            TransitionMode, VersionedIdentity, recover,
        },
    },
    request_trace::projection::{
        cache_state, close_reason, detail_availability, exchange_direction, exchange_kind,
        transition_mode,
    },
};

fn semantic(sequence: u64, record: JournalRecord) -> SequencedJournalRecord {
    SequencedJournalRecord::with_journal_sequence(
        ReplaySequence::new(sequence + 1),
        JournalSequence::new(sequence),
        record,
    )
}

fn identity(name: &str) -> VersionedIdentity {
    VersionedIdentity::new(format!("yo.test.{name}/v1"), format!("{name}:value"))
}

fn submission() -> SubmissionId {
    SubmissionId::from_uuid(Builder::from_random_bytes([11; 16]).into_uuid()).unwrap()
}

fn assert_identity(identity: &crate::BackendIdentity, schema: &str, value: &str) {
    assert_eq!(identity.schema(), schema);
    assert_eq!(identity.value(), value);
}

fn recovered_correlation_history() -> crate::journal::codec::RecoveredJournal {
    let session_id = crate::fixture_session(1);
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(1).unwrap()));
    let submission_id = submission();
    let operation_id = OperationId::from(submission_id);
    let descriptor = JournalCommit::descriptor(crate::fixture_descriptor(session_id));
    let records = JournalCommit::incremental_through(
        JournalSequence::new(9),
        vec![
            semantic(
                1,
                JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id }),
            ),
            semantic(
                2,
                JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                    1,
                    "codex",
                    "1.0.0",
                    identity("binding"),
                    identity("model"),
                    identity("session"),
                    BindingTransition::new(
                        TransitionMode::Initial,
                        CacheState::NotApplicable,
                        None,
                    ),
                    crate::ContinuationStrategy::BackendManagedState,
                )),
            ),
            semantic(
                3,
                JournalRecord::CommandCommitted(
                    CommittedCommand::submission(
                        AgentCommand::StartTurn {
                            turn,
                            input: UserInput::new("inspect"),
                        },
                        submission_id,
                    )
                    .unwrap(),
                ),
            ),
            semantic(
                4,
                JournalRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    1,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    "codex.request/v1",
                    None,
                    Some(identity("exchange")),
                    DetailAvailability::Redacted,
                )),
            ),
            semantic(
                5,
                JournalRecord::BackendRequestAccepted(BackendRequestAccepted::new(
                    1,
                    turn.turn_id(),
                    operation_id,
                    JournalSequence::new(4),
                    identity("request"),
                )),
            ),
            semantic(
                6,
                JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                    turn,
                    outcome: TurnOutcome::Completed,
                }),
            ),
            semantic(
                7,
                JournalRecord::BackendResumableOutcome(BackendResumableOutcome::new(
                    1,
                    turn.turn_id(),
                    JournalSequence::new(5),
                    Some(identity("outcome")),
                    None,
                )),
            ),
            semantic(
                8,
                JournalRecord::ContinuationAnchor(ContinuationAnchor::new(
                    1,
                    JournalSequence::new(5),
                    JournalSequence::new(7),
                    JournalSequence::new(7),
                )),
            ),
            semantic(
                9,
                JournalRecord::BackendBindingClosed(BackendBindingClosed::new(
                    1,
                    BindingCloseReason::Revoked,
                )),
            ),
        ],
    );

    recover(&[descriptor, records]).unwrap()
}

// 검증된 Journal에서 Request 진단 레코드만 원래 의미 순서로 추출하고, command/event와
// payload 본문은 제외하면서 redaction·상관 좌표·binding 종료 원인을 타입으로 보존한다.
#[test]
fn projects_the_complete_payload_free_request_trace_in_journal_order() {
    let trace = project(&recovered_correlation_history());
    let expected_operation = OperationId::from(submission()).as_uuid();

    assert_eq!(
        trace
            .iter()
            .map(|entry| entry.sequence().get())
            .collect::<Vec<_>>(),
        [2, 4, 5, 7, 8, 9]
    );
    let StoredRequestTraceRecord::BindingOpened {
        epoch,
        backend_kind,
        backend_version,
        binding_identity,
        model_identity,
        session_locator,
        transition,
        ..
    } = trace[0].record()
    else {
        panic!("first trace record must open the backend binding");
    };
    assert_eq!(*epoch, 1);
    assert_eq!(backend_kind, "codex");
    assert_eq!(backend_version, "1.0.0");
    assert_identity(binding_identity, "yo.test.binding/v1", "binding:value");
    assert_identity(model_identity, "yo.test.model/v1", "model:value");
    assert_identity(session_locator, "yo.test.session/v1", "session:value");
    assert_eq!(transition.mode(), StoredBindingTransitionMode::Initial);
    assert_eq!(transition.cache(), StoredBindingCacheState::NotApplicable);
    assert_eq!(transition.source_anchor_sequence(), None);

    let StoredRequestTraceRecord::ExchangeObserved {
        epoch,
        operation_id,
        kind,
        direction,
        payload_schema,
        correlation_sequence,
        exchange_identity,
        detail_availability,
    } = trace[1].record()
    else {
        panic!("second trace record must describe the outbound exchange");
    };
    assert_eq!(*epoch, 1);
    assert_eq!(*operation_id, expected_operation);
    assert_eq!(*kind, StoredExchangeKind::Request);
    assert_eq!(*direction, StoredExchangeDirection::YoToBackend);
    assert_eq!(payload_schema, "codex.request/v1");
    assert_eq!(*correlation_sequence, None);
    assert_identity(
        exchange_identity.as_ref().unwrap(),
        "yo.test.exchange/v1",
        "exchange:value",
    );
    assert_eq!(
        *detail_availability,
        StoredRequestDetailAvailability::Redacted
    );

    let StoredRequestTraceRecord::RequestAccepted {
        epoch,
        turn_id,
        operation_id,
        exchange_sequence,
        request_identity,
    } = trace[2].record()
    else {
        panic!("third trace record must accept the request");
    };
    assert_eq!(*epoch, 1);
    assert_eq!(turn_id.get(), NonZeroU64::new(1).unwrap());
    assert_eq!(*operation_id, expected_operation);
    assert_eq!(*exchange_sequence, JournalSequence::new(4));
    assert_identity(request_identity, "yo.test.request/v1", "request:value");

    let StoredRequestTraceRecord::ResumableOutcome {
        epoch,
        turn_id,
        accepted_request_sequence,
        outcome_identity,
        ..
    } = trace[3].record()
    else {
        panic!("fourth trace record must describe the resumable outcome");
    };
    assert_eq!(*epoch, 1);
    assert_eq!(turn_id.get(), NonZeroU64::new(1).unwrap());
    assert_eq!(*accepted_request_sequence, JournalSequence::new(5));
    assert_identity(
        outcome_identity.as_ref().unwrap(),
        "yo.test.outcome/v1",
        "outcome:value",
    );

    let StoredRequestTraceRecord::ContinuationAnchor {
        epoch,
        accepted_request_sequence,
        resumable_outcome_sequence,
        journal_boundary,
    } = trace[4].record()
    else {
        panic!("fifth trace record must close the resumable correlation chain");
    };
    assert_eq!(*epoch, 1);
    assert_eq!(*accepted_request_sequence, JournalSequence::new(5));
    assert_eq!(*resumable_outcome_sequence, JournalSequence::new(7));
    assert_eq!(*journal_boundary, JournalSequence::new(7));

    let StoredRequestTraceRecord::BindingClosed { epoch, reason } = trace[5].record() else {
        panic!("last trace record must close the backend binding");
    };
    assert_eq!(*epoch, 1);
    assert_eq!(*reason, StoredBindingCloseReason::Revoked);
}

// 저장 codec의 닫힌 enum 값이 공개 Request 모델의 같은 의미 값으로 모두 변환되어,
// 새 variant 추가나 비슷한 이름 사이의 오매핑을 컴파일 또는 비교 단계에서 드러냅니다.
#[test]
fn maps_every_closed_request_trace_enum_without_semantic_drift() {
    assert_eq!(
        [
            ExchangeKind::Request,
            ExchangeKind::Response,
            ExchangeKind::Notification,
            ExchangeKind::ServerRequest,
            ExchangeKind::Retry,
            ExchangeKind::TerminalOutcome,
        ]
        .map(exchange_kind),
        [
            StoredExchangeKind::Request,
            StoredExchangeKind::Response,
            StoredExchangeKind::Notification,
            StoredExchangeKind::ServerRequest,
            StoredExchangeKind::Retry,
            StoredExchangeKind::TerminalOutcome,
        ]
    );
    assert_eq!(
        [
            ExchangeDirection::YoToBackend,
            ExchangeDirection::BackendToYo,
        ]
        .map(exchange_direction),
        [
            StoredExchangeDirection::YoToBackend,
            StoredExchangeDirection::BackendToYo,
        ]
    );
    assert_eq!(
        [
            DetailAvailability::Persisted,
            DetailAvailability::Volatile,
            DetailAvailability::Missing,
            DetailAvailability::Unsupported,
            DetailAvailability::Unpersisted,
            DetailAvailability::Redacted,
        ]
        .map(detail_availability),
        [
            StoredRequestDetailAvailability::Persisted,
            StoredRequestDetailAvailability::Volatile,
            StoredRequestDetailAvailability::Missing,
            StoredRequestDetailAvailability::Unsupported,
            StoredRequestDetailAvailability::Unpersisted,
            StoredRequestDetailAvailability::Redacted,
        ]
    );
    assert_eq!(
        [
            TransitionMode::Initial,
            TransitionMode::ExactReplay,
            TransitionMode::LossyHandoff,
        ]
        .map(transition_mode),
        [
            StoredBindingTransitionMode::Initial,
            StoredBindingTransitionMode::ExactReplay,
            StoredBindingTransitionMode::LossyHandoff,
        ]
    );
    assert_eq!(
        [
            CacheState::NotApplicable,
            CacheState::Lost,
            CacheState::Unknown,
        ]
        .map(cache_state),
        [
            StoredBindingCacheState::NotApplicable,
            StoredBindingCacheState::Lost,
            StoredBindingCacheState::Unknown,
        ]
    );
    assert_eq!(
        [
            BindingCloseReason::Replaced,
            BindingCloseReason::Revoked,
            BindingCloseReason::Exhausted,
        ]
        .map(close_reason),
        [
            StoredBindingCloseReason::Replaced,
            StoredBindingCloseReason::Revoked,
            StoredBindingCloseReason::Exhausted,
        ]
    );
}
