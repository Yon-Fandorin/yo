use std::sync::{Arc, Mutex};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    AgentCommand, BackendCommandEvidence, BackendEvent, BackendPoll, ModelConnectorEvent,
    ModelReplayItem, ModelReplayRole, ToolApprovalRequirement, UserInput,
};

use super::{
    super::support::{
        ExactAdmission, MockConnector, MockHost, binding, drain_until_turn, event_rounds, registry,
        turn,
    },
    fixtures::{
        SequenceTokenCounter, completed_summary_round, completed_text_round, portable_summary,
        private_summary_event, tool_call_round, turn_number,
    },
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

// 자동 압축이 한 번만 실행되고 checkpoint commit 전에는 후속 요청을 보내지 않음을 검증합니다.
#[test]
fn pressure_compaction_summarizes_once_then_waits_for_checkpoint_before_dispatch() {
    for output_index in [0, 1] {
        check_pressure_compaction_summarizes_once_then_waits_for_checkpoint_before_dispatch(
            output_index,
        );
    }
}

fn check_pressure_compaction_summarizes_once_then_waits_for_checkpoint_before_dispatch(
    output_index: usize,
) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let summary = portable_summary();
    let mut summary_round = completed_summary_round("summary-1", &summary, output_index);
    summary_round.insert(1, private_summary_event());
    let Some(ModelConnectorEvent::Terminal { usage, .. }) = summary_round.last_mut() else {
        unreachable!("a completed text round ends in a terminal event")
    };
    *usage = yo_core::ResponsesUsage {
        input_tokens: Some(20),
        output_tokens: Some(10),
        total_tokens: Some(30),
        reasoning_tokens: None,
        cache_read_input_tokens: yo_core::CacheReadInputTokens::Unsupported,
    };
    let rounds = vec![
        completed_text_round("turn-1", "first"),
        completed_text_round("turn-2", "second"),
        summary_round,
        completed_text_round("turn-3", "third"),
    ];
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(SequenceTokenCounter::new(
                [10, 10, 90, 20, 30, 30],
                Arc::clone(&payloads),
            )),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();

    for number in [1, 2] {
        assert!(matches!(
            backend
                .execute_command(AgentCommand::StartTurn {
                    turn: turn_number(number),
                    input: UserInput::from(format!("input-{number}")),
                })
                .unwrap(),
            BackendCommandEvidence::RequestAccepted(_)
        ));
        assert!(matches!(
            drain_until_turn(&mut backend),
            BackendEvent::ResumableTurnFinished { .. }
        ));
    }

    assert!(matches!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn_number(3),
                input: UserInput::from("input-3"),
            })
            .unwrap(),
        BackendCommandEvidence::None
    ));
    assert_eq!(requests.lock().unwrap().len(), 3);

    let proposal = (0..100)
        .find_map(|_| match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ContextCheckpointPrepared { proposal }) => {
                Some(proposal)
            },
            BackendPoll::Event(_) | BackendPoll::Pending => None,
            BackendPoll::Closed => panic!("backend closed before proposing its checkpoint"),
        })
        .expect("summary did not produce a bounded checkpoint proposal");
    assert_eq!(proposal.input_tokens_before(), 90);
    assert_eq!(proposal.input_tokens_after(), 30);
    assert_eq!(proposal.summarized_groups().len(), 1);
    assert_eq!(proposal.retained_groups().len(), 1);
    assert_eq!(proposal.portable_body(), summary);
    assert_eq!(requests.lock().unwrap().len(), 3);

    let accepted_poll = backend.poll_event().unwrap();
    assert!(
        matches!(
            accepted_poll,
            BackendPoll::Event(BackendEvent::ModelRequestAccepted {
                turn: accepted,
                ..
            }) if accepted == turn_number(3)
        ),
        "unexpected post-checkpoint poll: {accepted_poll:?}"
    );
    assert_eq!(requests.lock().unwrap().len(), 4);
    let BackendEvent::ResumableTurnFinished { evidence, .. } = drain_until_turn(&mut backend)
    else {
        panic!("post-checkpoint request did not finish resumably")
    };
    assert_eq!(
        evidence.model_replay().unwrap().items(),
        &[ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "third".to_owned(),
            refusal: None,
        }]
    );
    assert_eq!(payloads.lock().unwrap().len(), 6);
}

// 도구 결과까지 완전히 닫힌 active suffix는 core 결속 이벤트 뒤 동일한 단일 요약 경로를
// 사용하고, checkpoint 승인 전에는 successor 요청을 보내지 않습니다.
#[test]
fn post_tool_pressure_compacts_only_after_completing_the_active_suffix() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let summary = portable_summary();
    let mut summary_round = completed_text_round("tool-summary", &summary);
    let Some(ModelConnectorEvent::Terminal { usage, .. }) = summary_round.last_mut() else {
        unreachable!("a completed text round ends in a terminal event")
    };
    *usage = yo_core::ResponsesUsage {
        input_tokens: Some(20),
        output_tokens: Some(10),
        total_tokens: Some(30),
        reasoning_tokens: Some(0),
        cache_read_input_tokens: yo_core::CacheReadInputTokens::Unsupported,
    };
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![
                completed_text_round("turn-1", "first"),
                completed_text_round("turn-2", "second"),
                tool_call_round(),
                summary_round,
                completed_text_round("turn-3", "third"),
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
                [1, 1, 1, 90, 20, 30, 30],
                Arc::clone(&payloads),
            )),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
        NativeModelBackendConfig::default(),
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
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn_number(3),
            input: UserInput::from("run one tool"),
        })
        .unwrap();

    let mut saw_closed_suffix = false;
    let proposal = (0..200)
        .find_map(|_| match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ContextActiveSuffixCompleted { items, .. }) => {
                assert!(matches!(
                    items.first(),
                    Some(ModelReplayItem::Message {
                        role: ModelReplayRole::User,
                        ..
                    })
                ));
                assert!(
                    items
                        .iter()
                        .any(|item| matches!(item, ModelReplayItem::FunctionCall { .. }))
                );
                assert!(
                    items
                        .iter()
                        .any(|item| matches!(item, ModelReplayItem::FunctionCallOutput { .. }))
                );
                saw_closed_suffix = true;
                None
            },
            BackendPoll::Event(BackendEvent::ContextCheckpointPrepared { proposal }) => {
                Some(proposal)
            },
            BackendPoll::Event(_) | BackendPoll::Pending => None,
            BackendPoll::Closed => panic!("backend closed before the post-tool checkpoint"),
        })
        .expect("post-tool pressure did not produce a checkpoint proposal");
    assert!(saw_closed_suffix);
    assert_eq!(proposal.summarized_groups().len(), 2);
    assert!(proposal.retained_groups().is_empty());
    assert!(
        proposal
            .active_group()
            .iter()
            .any(|item| matches!(item, ModelReplayItem::FunctionCallOutput { .. }))
    );
    assert_eq!(requests.lock().unwrap().len(), 4);

    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ModelRequestAccepted { turn, .. })
            if turn == turn_number(3)
    ));
    assert_eq!(requests.lock().unwrap().len(), 5);
    let BackendEvent::ResumableTurnFinished { evidence, .. } = drain_until_turn(&mut backend)
    else {
        panic!("post-tool successor request did not finish resumably")
    };
    assert_eq!(
        evidence.model_replay().unwrap().items(),
        &[ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "third".to_owned(),
            refusal: None,
        }]
    );
    assert_eq!(payloads.lock().unwrap().len(), 7);
}
