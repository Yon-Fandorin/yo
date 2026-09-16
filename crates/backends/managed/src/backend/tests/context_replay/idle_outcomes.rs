use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    AgentCommand, BackendEvent, BackendFailureKind, BackendPoll, ModelConnectorEvent,
    ToolApprovalRequirement, UserInput,
};

use super::{
    super::support::{
        ExactAdmission, MockConnector, MockHost, backend, binding, drain_until_turn, event_rounds,
        registry, turn,
    },
    fixtures::{SequenceTokenCounter, completed_text_round, portable_summary, turn_number},
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

// 성공적으로 닫힌 idle 요약이 작아지지 않으면 원래 문맥을 보존하고 다음 Turn을 허용한다.
// 같은 결과가 여전히 pressure 한계에 걸리면 기존 fail-closed 동작을 유지한다.
#[test]
fn idle_nonreducing_summary_rejects_without_losing_original_context() {
    use yo_core::{ModelConnectorInputItem, ModelConnectorInputRole};
    for after in [50, 60, 90] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut counts = vec![10, 10, 50, 20, after];
        if after < 90 {
            counts.push(10);
        }
        let mut summary_round = completed_text_round("nonreducing-summary", &portable_summary());
        let Some(ModelConnectorEvent::Terminal { usage, .. }) = summary_round.last_mut() else {
            unreachable!()
        };
        *usage = yo_core::ResponsesUsage {
            input_tokens: Some(20),
            output_tokens: Some(10),
            total_tokens: Some(30),
            reasoning_tokens: None,
            cache_read_input_tokens: yo_core::CacheReadInputTokens::Unsupported,
        };
        let config = NativeModelBackendConfig::default();
        let system_prompt = config.system_prompt.clone();
        let mut backend = NativeModelBackend::with_connector(
            Box::new(MockConnector {
                rounds: event_rounds(vec![
                    completed_text_round("one", "first"),
                    completed_text_round("two", "second"),
                    summary_round,
                    completed_text_round("three", "third"),
                ]),
                requests: Arc::clone(&requests),
            }),
            binding(),
            registry(ToolApprovalRequirement::Automatic),
            NativeModelBackendServices::new(
                Box::new(yo_core::admit_standard_complete_binding),
                Some(Box::new(ExactAdmission)),
                Box::new(MockHost::default()),
                Box::new(SequenceTokenCounter::new(
                    counts,
                    Arc::new(Mutex::new(Vec::new())),
                )),
            ),
            yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
            config,
        )
        .unwrap();
        backend
            .execute_command(AgentCommand::CreateSession {
                session_id: turn().session_id(),
            })
            .unwrap();
        for number in [1, 2] {
            backend
                .execute_command(AgentCommand::StartTurn {
                    turn: turn_number(number),
                    input: UserInput::from(format!("input-{number}")),
                })
                .unwrap();
            assert!(matches!(
                drain_until_turn(&mut backend),
                BackendEvent::ResumableTurnFinished { .. }
            ));
        }
        let original_replay = backend.replay.clone();
        let original_groups = backend.replay_groups.clone();
        backend
            .execute_command(AgentCommand::CompactContext { guidance: None })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let failure = loop {
            assert!(
                Instant::now() < deadline,
                "summary did not reach its bounded outcome"
            );
            match backend.poll_event() {
                Err(failure) => break failure,
                Ok(BackendPoll::Event(BackendEvent::ContextCheckpointPrepared { .. })) => {
                    panic!("nonreducing summary must not propose a checkpoint")
                },
                Ok(BackendPoll::Closed) => panic!("backend closed before its summary outcome"),
                _ => thread::yield_now(),
            }
        };
        assert_eq!(backend.replay, original_replay);
        assert_eq!(backend.replay_groups, original_groups);
        assert!(backend.idle_compaction.is_none());
        if after == 90 {
            assert_eq!(failure.kind(), BackendFailureKind::ContextExhausted);
            assert!(backend.context_exhausted);
            backend.shutdown().unwrap();
            continue;
        }
        assert_eq!(failure.kind(), BackendFailureKind::CommandRejected);
        assert!(!backend.context_exhausted);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn_number(3),
                input: UserInput::from("input-3"),
            })
            .unwrap();
        assert!(matches!(
            drain_until_turn(&mut backend),
            BackendEvent::ResumableTurnFinished { .. }
        ));
        let message = |role, content: &str| ModelConnectorInputItem::Message {
            role,
            content: content.into(),
            refusal: None,
        };
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        assert_eq!(
            requests[3].input(),
            vec![
                message(ModelConnectorInputRole::System, &system_prompt),
                message(ModelConnectorInputRole::User, "input-1"),
                message(ModelConnectorInputRole::Assistant, "first"),
                message(ModelConnectorInputRole::User, "input-2"),
                message(ModelConnectorInputRole::Assistant, "second"),
                message(ModelConnectorInputRole::User, "input-3"),
            ]
        );
        drop(requests);
        backend.shutdown().unwrap();
    }
}

// 잘못된 summary 형식과 불완전한 usage가 함께 오면 형식 거절로 복구하지 않고
// 기존 fail-closed 경계를 유지하며 provider 본문을 오류에 노출하지 않는다.
#[test]
fn idle_malformed_summary_with_incomplete_usage_remains_fatal() {
    let mut backend = backend(
        vec![
            completed_text_round("one", "first"),
            completed_text_round("two", "second"),
            completed_text_round("malformed", "# secret-malformed-canary"),
        ],
        ToolApprovalRequirement::Automatic,
        Arc::new(Mutex::new(0)),
    );
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    for number in [1, 2] {
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn_number(number),
                input: UserInput::from(format!("input-{number}")),
            })
            .unwrap();
        assert!(matches!(
            drain_until_turn(&mut backend),
            BackendEvent::ResumableTurnFinished { .. }
        ));
    }
    let original_replay = backend.replay.clone();
    backend
        .execute_command(AgentCommand::CompactContext { guidance: None })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let failure = loop {
        assert!(Instant::now() < deadline);
        match backend.poll_event() {
            Err(failure) => break failure,
            Ok(BackendPoll::Event(BackendEvent::ContextCheckpointPrepared { .. })) => {
                panic!("malformed summary must not create a checkpoint")
            },
            Ok(BackendPoll::Closed) => panic!("summary closed without its typed failure"),
            _ => thread::yield_now(),
        }
    };
    assert_eq!(failure.kind(), BackendFailureKind::ContextExhausted);
    assert!(failure.message().contains("usage is incomplete"));
    assert!(!failure.message().contains("secret-malformed-canary"));
    assert!(backend.context_exhausted);
    assert_eq!(backend.replay, original_replay);
    backend.shutdown().unwrap();
}
