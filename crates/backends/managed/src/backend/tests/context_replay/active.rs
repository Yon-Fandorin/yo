use std::sync::{Arc, Mutex};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    AgentCommand, BackendCommandEvidence, BackendEvent, BackendPoll, ContextPolicyChanged,
    ContextStrategy, ModelConnectorEvent, ModelReplayItem, ModelReplayRole,
    ToolApprovalRequirement, UserInput,
};

use super::{
    super::support::{
        ExactAdmission, MockConnector, MockHost, binding, drain_until_turn, event_rounds, registry,
        turn,
    },
    fixtures::{
        CapSensitiveTokenCounter, SequenceTokenCounter, completed_summary_round,
        completed_text_round, portable_summary, private_summary_event, tool_call_round,
        turn_number,
    },
};
use crate::backend::{
    CompactionState, NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices,
};

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
                [10, 10, 90, 20, 90, 30, 30],
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
    assert_eq!(payloads.lock().unwrap().len(), 7);
}

// 요약 전·중·완료 뒤 보정은 checkpoint에 들어가고, publication 뒤 보정은 swap 뒤 delta에 남습니다.
#[test]
fn active_checkpoint_freezes_only_its_committed_steer_prefix() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let summary = portable_summary();
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![
                completed_text_round("freeze-turn-1", "first"),
                completed_text_round("freeze-turn-2", "second"),
                completed_summary_round("freeze-summary", &summary, 0),
                completed_text_round("freeze-turn-3", "third"),
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
                [10, 10, 90, 20, 90, 30, 30],
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

    assert!(matches!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn_number(3),
                input: UserInput::from("input-3"),
            })
            .unwrap(),
        BackendCommandEvidence::None
    ));
    let steer = |text: &str| AgentCommand::SteerTurn {
        turn: turn_number(3),
        input: UserInput::from(text),
    };
    let before_summary_events = backend
        .execute_command(steer("before summary events"))
        .unwrap();
    assert_eq!(
        before_summary_events,
        BackendCommandEvidence::SubmissionPrepared
    );
    backend.commit_prepared_command().unwrap();

    let mut saw_summary_start = false;
    for _ in 0..100 {
        let _ = backend.poll_event().unwrap();
        if backend.turn.as_ref().is_some_and(|state| {
            matches!(
                &state.compaction,
                Some(CompactionState::Summarizing {
                    response_id: Some(_),
                    ..
                })
            )
        }) {
            saw_summary_start = true;
            break;
        }
    }
    assert!(saw_summary_start, "the summary response did not start");
    let during_summary = backend
        .execute_command(steer("during summary stream"))
        .unwrap();
    assert_eq!(during_summary, BackendCommandEvidence::SubmissionPrepared);
    backend.commit_prepared_command().unwrap();

    for _ in 0..100 {
        let _ = backend.poll_event().unwrap();
        if backend.turn.as_ref().is_some_and(|state| {
            matches!(
                &state.compaction,
                Some(CompactionState::CompletedSummary { .. })
            )
        }) {
            break;
        }
    }
    assert!(backend.turn.as_ref().is_some_and(|state| {
        matches!(
            &state.compaction,
            Some(CompactionState::CompletedSummary { .. })
        )
    }));
    let after_summary = backend
        .execute_command(steer("after summary terminal"))
        .unwrap();
    assert_eq!(after_summary, BackendCommandEvidence::SubmissionPrepared);
    backend.commit_prepared_command().unwrap();

    let BackendPoll::Event(BackendEvent::ContextCheckpointPrepared { proposal }) =
        backend.poll_event().unwrap()
    else {
        panic!("the completed summary did not return its final proposal directly")
    };
    let active_users = proposal
        .active_group()
        .iter()
        .filter_map(|item| match item {
            ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content,
                ..
            } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        active_users,
        [
            "input-3",
            "before summary events",
            "during summary stream",
            "after summary terminal",
        ]
    );
    assert_eq!(requests.lock().unwrap().len(), 3);

    let after_publication = backend
        .execute_command(steer("after checkpoint publication"))
        .unwrap();
    assert_eq!(
        after_publication,
        BackendCommandEvidence::SubmissionPrepared
    );
    backend.commit_prepared_command().unwrap();
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ModelRequestAccepted { turn, .. })
            if turn == turn_number(3)
    ));
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 4);
    let successor_users = captured[3]
        .input()
        .iter()
        .filter_map(|item| match item {
            yo_core::ResponsesInputItem::Message {
                role: yo_core::ResponsesInputRole::User,
                content,
                ..
            } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(successor_users.ends_with(&[
        "input-3",
        "before summary events",
        "during summary stream",
        "after summary terminal",
        "after checkpoint publication",
    ]));
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
                [1, 1, 1, 90, 20, 90, 30, 30],
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
    assert_eq!(payloads.lock().unwrap().len(), 8);
}

// 완료 assistant 뒤의 보정으로 시작한 후속 요청도 압축되며, 선택된 출력 한도와
// 실제 체크포인트 이후 connector 입력을 함께 검증합니다.
#[test]
fn completed_assistant_steer_boundary_compacts_with_the_selected_output_cap() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![
                completed_text_round("assistant-prefix-1", "prior one"),
                completed_text_round("assistant-prefix-2", "prior two"),
                completed_text_round("assistant-steer-first", "first answer"),
                completed_summary_round("assistant-steer-summary", &portable_summary(), 0),
                completed_text_round("assistant-steer-successor", "after checkpoint"),
            ]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(CapSensitiveTokenCounter {
                payloads: Arc::clone(&payloads),
            }),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
        NativeModelBackendConfig {
            context_policy: ContextPolicyChanged::try_new(
                1,
                true,
                ContextStrategy::PortableSummaryV1Alpha1,
                70,
                80,
                Some(10),
                Some(65_536),
            )
            .unwrap(),
            ..NativeModelBackendConfig::default()
        },
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
        BackendCommandEvidence::RequestAccepted(_)
    ));
    assert_eq!(
        backend
            .execute_command(AgentCommand::SteerTurn {
                turn: turn_number(3),
                input: UserInput::from("correction A"),
            })
            .unwrap(),
        BackendCommandEvidence::SubmissionPrepared
    );
    backend.commit_prepared_command().unwrap();

    let proposal = (0..300)
        .find_map(|_| match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ContextCheckpointPrepared { proposal }) => {
                Some(proposal)
            },
            BackendPoll::Event(event @ BackendEvent::TurnFinished { .. }) => {
                panic!("완료 assistant steering 경계가 checkpoint 전에 실패했습니다: {event:?}")
            },
            BackendPoll::Event(_) | BackendPoll::Pending => None,
            BackendPoll::Closed => panic!("backend closed before checkpoint publication"),
        })
        .expect("completed assistant plus a pending steer must be compactable");
    assert_eq!(proposal.input_tokens_before(), 82);
    assert_eq!(proposal.input_tokens_after(), 30);
    assert!(proposal.active_group().iter().any(|item| matches!(
        item,
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content,
            ..
        } if content == "first answer"
    )));
    assert!(proposal.active_group().iter().any(|item| matches!(
        item,
        ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content,
            ..
        } if content == "correction A"
    )));
    assert_eq!(requests.lock().unwrap().len(), 4);

    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ModelRequestAccepted { turn, .. })
            if turn == turn_number(3)
    ));
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 5);
    let successor_messages = captured[4]
        .input()
        .iter()
        .filter_map(|item| match item {
            yo_core::ResponsesInputItem::Message { role, content, .. }
                if matches!(
                    role,
                    yo_core::ResponsesInputRole::User | yo_core::ResponsesInputRole::Assistant
                ) =>
            {
                Some((*role, content.as_str()))
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(successor_messages[0].1.starts_with("# Context Checkpoint"));
    assert_eq!(
        &successor_messages[1..],
        [
            (yo_core::ResponsesInputRole::User, "input-3"),
            (yo_core::ResponsesInputRole::Assistant, "first answer"),
            (yo_core::ResponsesInputRole::User, "correction A"),
        ]
    );
    drop(captured);

    assert!(matches!(
        drain_until_turn(&mut backend),
        BackendEvent::ResumableTurnFinished { .. }
    ));
    let payloads = payloads.lock().unwrap();
    assert_eq!(payloads.len(), 9);
    assert_eq!(payloads[4]["max_output_tokens"], 4);
    assert_eq!(payloads[6]["max_output_tokens"], 4);
    assert_eq!(payloads[7]["max_output_tokens"], 10);
    assert_eq!(payloads[8]["max_output_tokens"], 10);
}
