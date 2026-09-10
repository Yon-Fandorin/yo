use std::{
    collections::VecDeque,
    num::NonZeroU64,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    ActivityKind, ActivityRequestRef, ActivityResponse, AgentCommand, AgentEvent, AgentIntent,
    AgentSession, ApprovalDecision, BackendCommandEvidence, BackendEvent, BackendFailureKind,
    BackendPoll, CommandAdmission, ContextPolicyChanged, ContextStrategy, JournalSequence,
    ModelConnectorEvent, ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole,
    ProviderPrivateReplayEnvelope, ToolApprovalRequirement, ToolExecution, ToolExecutionError,
    ToolExecutionHost, ToolExecutionRequest, ToolId, TranscriptReader, TranscriptRecord,
    TurnOutcome, TurnRef, UserInput,
};

use super::support::{
    ExactAdmission, FixedTokenCounter, MockConnector, MockHost, backend, binding, completed,
    drain_until_turn, event_rounds, mock_tokenization_payload, registry, turn,
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

struct RecordingTokenCounter {
    input_tokens: u64,
    payloads: Arc<Mutex<Vec<serde_json::Value>>>,
}

fn pressure_at_hard_limit_config() -> NativeModelBackendConfig {
    NativeModelBackendConfig {
        context_policy: ContextPolicyChanged::try_new(
            1,
            true,
            ContextStrategy::PortableSummaryV1Alpha1,
            99,
            100,
            Some(10),
            Some(65_536),
        )
        .unwrap(),
        ..NativeModelBackendConfig::default()
    }
}

fn portable_summary() -> String {
    [
        "# Context Checkpoint",
        "## Current Objective\nContinue the current task.",
        "## Active Constraints\nNone.",
        "## Decisions\nPreserve exact retained history.",
        "## Verified Progress\nTwo prior turns completed.",
        "## Current State\nA new turn is ready.",
        "## Unknown or Unverified\nNone.",
        "## Next Actions\nAnswer the current user input.",
        "## Critical References\nNone.",
    ]
    .join("\n")
}

fn completed_text_round(response_id: &str, text: &str) -> Vec<ModelConnectorEvent> {
    vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: response_id.to_owned(),
        },
        ModelConnectorEvent::TextDelta {
            output_index: 0,
            item_id: format!("{response_id}-item"),
            content_index: 0,
            delta: text.to_owned(),
        },
        ModelConnectorEvent::MessageDone {
            output_index: 0,
            item_id: format!("{response_id}-item"),
        },
        completed(response_id),
    ]
}

fn private_summary_event() -> ModelConnectorEvent {
    ModelConnectorEvent::ProviderPrivateAssistant {
        output_index: 1,
        envelope: ProviderPrivateReplayEnvelope::new(
            "kimi.assistant-message/v1alpha1",
            br#"{"role":"assistant","reasoning_content":"private","content":null}"#.to_vec(),
        )
        .unwrap(),
        visible_projection: Vec::new(),
    }
}

fn turn_number(number: u64) -> TurnRef {
    TurnRef::new(
        turn().session_id(),
        yo_core::TurnId::new(NonZeroU64::new(number).unwrap()),
    )
}

fn wait_for_turn_finish(
    session: &mut AgentSession,
    transcript: &TranscriptReader,
    cursor: &mut Option<JournalSequence>,
    expected_turn_id: u64,
) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let _ = session.poll().expect("the Session remains healthy");
        let slice = transcript.read_after(*cursor);
        if let Some(last) = slice.entries().last() {
            *cursor = Some(last.sequence());
        }
        if slice.entries().iter().any(|entry| {
            matches!(
                entry.record(),
                TranscriptRecord::EventCommitted(AgentEvent::TurnFinished { turn, .. })
                    if turn.turn_id().get().get() == expected_turn_id
            )
        }) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the expected Turn did not finish"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn queue_intent(session: &mut AgentSession, intent: AgentIntent) {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut admission = session.dispatch(intent).unwrap();
    loop {
        match admission {
            CommandAdmission::Queued => return,
            CommandAdmission::Backpressured(pending) => {
                let _ = session.poll().expect("the Session remains healthy");
                admission = session.retry(pending).unwrap();
            },
            CommandAdmission::Rejected { rejection, .. } => {
                panic!("the test intent was rejected: {rejection:?}")
            },
        }
        assert!(Instant::now() < deadline, "the test intent stayed queued");
        thread::sleep(Duration::from_millis(1));
    }
}

struct FailingStartHost;

impl ToolExecutionHost for FailingStartHost {
    fn identity(&self) -> &str {
        "failing-start-host-v1"
    }

    fn is_available(&self, _tool: &ToolId) -> bool {
        true
    }

    fn start(
        &mut self,
        _request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        Err(ToolExecutionError::new("injected start failure"))
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

struct SequenceTokenCounter {
    input_tokens: Mutex<VecDeque<u64>>,
    payloads: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl SequenceTokenCounter {
    fn new(
        input_tokens: impl IntoIterator<Item = u64>,
        payloads: Arc<Mutex<Vec<serde_json::Value>>>,
    ) -> Self {
        Self {
            input_tokens: Mutex::new(input_tokens.into_iter().collect()),
            payloads,
        }
    }
}

impl yo_core::ModelTokenCounter for SequenceTokenCounter {
    fn count_input_tokens(
        &self,
        _tokenizer_profile: &str,
        request: &serde_json::Value,
    ) -> Result<u64, yo_core::ModelTokenCounterError> {
        self.payloads.lock().unwrap().push(request.clone());
        Ok(self
            .input_tokens
            .lock()
            .unwrap()
            .pop_front()
            .expect("the test declared every exact payload count"))
    }
}

fn tool_call_round() -> Vec<ModelConnectorEvent> {
    vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "tool".to_owned(),
        },
        ModelConnectorEvent::FunctionCallStarted {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
        },
        ModelConnectorEvent::FunctionCallDone {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            arguments: r#"{"path":"README.md"}"#.to_owned(),
        },
        completed("tool"),
    ]
}

fn retain_empty_assistant_items(backend: &mut NativeModelBackend, count: usize) {
    backend
        .replay
        .apply(&ModelReplayDelta::new(
            Some(ModelReplayContract::new("system", Vec::new())),
            (0..count)
                .map(|_| ModelReplayItem::Message {
                    role: ModelReplayRole::Assistant,
                    content: String::new(),
                    refusal: None,
                })
                .collect(),
        ))
        .unwrap();
}

fn fill_current_delta_to_item_limit(backend: &mut NativeModelBackend) {
    let delta = &mut backend
        .turn
        .as_mut()
        .expect("the first model request opened a Turn")
        .delta;
    assert_eq!(delta.len(), 1);
    delta.extend((1..4_095).map(|_| ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content: String::new(),
        refusal: None,
    }));
    assert_eq!(delta.len(), 4_095);
}

impl yo_core::ModelTokenCounter for RecordingTokenCounter {
    fn count_input_tokens(
        &self,
        _tokenizer_profile: &str,
        request: &serde_json::Value,
    ) -> Result<u64, yo_core::ModelTokenCounterError> {
        self.payloads.lock().unwrap().push(request.clone());
        Ok(self.input_tokens)
    }
}

// 100-token 입력은 100-token limit 안에 양수 output cap을 하나도 남기지 않으므로 remote
// call 없이 code=context_exhausted인 Failed Turn을 남기고 다음 Turn도 거절합니다.
#[test]
fn context_exhaustion_finishes_non_resumably_and_latches_the_binding() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(Vec::new()),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(100)),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
        pressure_at_hard_limit_config(),
    )
    .unwrap();
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    assert!(matches!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("too much context"),
            })
            .unwrap(),
        BackendCommandEvidence::None
    ));
    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("context exhaustion must finish as a failed non-resumable Turn")
    };
    assert_eq!(failure.code(), Some("context_exhausted"));
    assert!(requests.lock().unwrap().is_empty());

    let next_turn = TurnRef::new(
        turn().session_id(),
        yo_core::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    let error = backend
        .execute_command(AgentCommand::StartTurn {
            turn: next_turn,
            input: UserInput::from("retry"),
        })
        .unwrap_err();
    assert_eq!(error.kind(), BackendFailureKind::ContextExhausted);
}

// 95-token 입력에서 hard max 10은 넘치지만 계산된 cap 5는 정확히 100에 맞으므로, cap 10과
// cap 5 payload를 각각 count한 뒤 최종 cap 5 요청만 connector에 전달합니다.
#[test]
fn recounts_and_dispatches_the_exact_smaller_output_cap() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![Vec::new()]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(RecordingTokenCounter {
                input_tokens: 95,
                payloads: Arc::clone(&payloads),
            }),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
        pressure_at_hard_limit_config(),
    )
    .unwrap();
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();

    assert!(matches!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("exact boundary"),
            })
            .unwrap(),
        BackendCommandEvidence::RequestAccepted(_)
    ));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        mock_tokenization_payload(&requests[0], "qwen3.8max")["max_output_tokens"],
        5
    );
    let payloads = payloads.lock().unwrap();
    assert_eq!(payloads.len(), 2);
    assert_eq!(payloads[0]["max_output_tokens"], 10);
    assert_eq!(payloads[1]["max_output_tokens"], 5);
}

// count 결과가 차례로 95, 100, 99이면 100-token limit과 hard max 10에서 payload cap은
// 10, 5, 1로 엄격히 감소하고, 세 번째 cap 1 payload만 connector에 한 번 전달됩니다.
#[test]
fn bounded_selector_uses_at_most_three_strictly_decreasing_exact_counts() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![Vec::new()]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(SequenceTokenCounter::new(
                [95, 100, 99],
                Arc::clone(&payloads),
            )),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
        pressure_at_hard_limit_config(),
    )
    .unwrap();
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();

    assert!(matches!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("three exact counts"),
            })
            .unwrap(),
        BackendCommandEvidence::RequestAccepted(_)
    ));
    let payloads = payloads.lock().unwrap();
    assert_eq!(payloads.len(), 3);
    assert_eq!(payloads[0]["max_output_tokens"], 10);
    assert_eq!(payloads[1]["max_output_tokens"], 5);
    assert_eq!(payloads[2]["max_output_tokens"], 1);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        mock_tokenization_payload(&requests[0], "qwen3.8max")["max_output_tokens"],
        1
    );
}

// 자동 압축이 한 번만 실행되고 checkpoint commit 전에는 후속 요청을 보내지 않음을 검증합니다.
#[test]
fn pressure_compaction_summarizes_once_then_waits_for_checkpoint_before_dispatch() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let summary = portable_summary();
    let mut summary_round = completed_text_round("summary-1", &summary);
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

// 명시적 idle 압축도 자동 압축과 동일한 bounded summary·checkpoint 파이프라인을 사용합니다.
#[test]
fn explicit_idle_compaction_uses_the_same_bounded_summary_pipeline() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let summary = portable_summary();
    let mut summary_round = completed_text_round("manual-summary", &summary);
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
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![
                completed_text_round("turn-1", "first"),
                completed_text_round("turn-2", "second"),
                summary_round,
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
                [10, 10, 70, 20, 30],
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
        drain_until_turn(&mut backend);
    }

    assert_eq!(
        backend
            .execute_command(AgentCommand::CompactContext {
                guidance: Some("Prioritize unresolved constraints.".to_owned()),
            })
            .unwrap(),
        BackendCommandEvidence::None
    );
    assert_eq!(requests.lock().unwrap().len(), 3);
    let start_error = backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn_number(3),
            input: UserInput::from("must wait for the idle checkpoint"),
        })
        .expect_err("a Turn cannot start while idle compaction is active");
    assert_eq!(start_error.kind(), BackendFailureKind::Session);
    assert_eq!(requests.lock().unwrap().len(), 3);
    assert!(
        payloads.lock().unwrap()[3]
            .to_string()
            .contains("Prioritize unresolved constraints.")
    );
    let proposal = (0..100)
        .find_map(|_| match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ContextCheckpointPrepared { proposal }) => {
                Some(proposal)
            },
            BackendPoll::Event(_) | BackendPoll::Pending => None,
            BackendPoll::Closed => panic!("backend closed before manual checkpoint proposal"),
        })
        .expect("manual summary did not produce a checkpoint proposal");
    assert_eq!(proposal.turn(), None);
    assert!(proposal.active_group().is_empty());
    assert_eq!(proposal.input_tokens_before(), 70);
    assert_eq!(proposal.input_tokens_after(), 30);
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
    assert_eq!(
        backend.replay.items().first(),
        Some(&ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: summary,
            refusal: None,
        })
    );
    assert_eq!(payloads.lock().unwrap().len(), 5);
}

// idle 압축 command를 수락한 순간부터 checkpoint가 durable하게 적용될 때까지 다음
// prompt는 frontend에 보존된다. checkpoint 뒤 worker가 변경 신호를 보내면 같은
// PendingCommand를 재시도해 새 Turn으로 안전하게 진행할 수 있다.
#[test]
fn idle_compaction_backpressures_the_next_submission_until_activation() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let summary = portable_summary();
    let mut summary_round = completed_text_round("manual-summary", &summary);
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
    let backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![
                completed_text_round("turn-1", "first"),
                completed_text_round("turn-2", "second"),
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
                [10, 10, 70, 20, 30, 30],
                Arc::clone(&payloads),
            )),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    let mut session = AgentSession::start(backend).unwrap();
    let transcript = session.transcript_reader();
    let mut cursor = None;

    for number in [1, 2] {
        queue_intent(
            &mut session,
            AgentIntent::submit(format!("input-{number}")).unwrap(),
        );
        wait_for_turn_finish(&mut session, &transcript, &mut cursor, number);
    }

    queue_intent(
        &mut session,
        AgentIntent::CompactContext {
            guidance: Some("Prioritize unresolved constraints.".to_owned()),
        },
    );
    let CommandAdmission::Backpressured(mut pending) = session
        .dispatch(AgentIntent::submit("input-3").unwrap())
        .unwrap()
    else {
        panic!("the following submission must remain at the frontend")
    };

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let _ = session.poll().expect("idle compaction remains nonterminal");
        match session.retry(pending).unwrap() {
            CommandAdmission::Queued => break,
            CommandAdmission::Backpressured(retained) => pending = retained,
            CommandAdmission::Rejected { .. } => {
                panic!("the retained submission cannot become stale while idle")
            },
        }
        assert!(
            Instant::now() < deadline,
            "the checkpoint did not release the retained submission"
        );
        thread::sleep(Duration::from_millis(1));
    }
    wait_for_turn_finish(&mut session, &transcript, &mut cursor, 3);

    assert_eq!(requests.lock().unwrap().len(), 4);
    session.shutdown().unwrap();
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

// 첫 요청은 input 1 + cap 10으로 성공하고 자동 도구 결과까지 누적되지만, 두 번째 요청의
// exact input 100은 양수 cap을 남기지 않습니다. connector request는 1건에서 멈추고 현재
// Turn은 code=context_exhausted로 실패하며 같은 binding의 다음 Turn도 거절됩니다.
#[test]
fn post_tool_round_exhaustion_stops_before_a_second_dispatch_and_latches() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![tool_call_round()]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(SequenceTokenCounter::new([1, 100], Arc::clone(&payloads))),
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
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("run one tool"),
        })
        .unwrap();

    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("the second-round context overflow must fail the current Turn")
    };
    assert_eq!(failure.code(), Some("context_exhausted"));
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(payloads.lock().unwrap().len(), 2);

    let next_turn = TurnRef::new(
        turn().session_id(),
        yo_core::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    assert_eq!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: next_turn,
                input: UserInput::from("retry"),
            })
            .unwrap_err()
            .kind(),
        BackendFailureKind::ContextExhausted
    );
}

// 첫 요청 뒤 current delta를 user 포함 4095 items로 채운 seam에서 model function call은
// 4096에 정확히 맞습니다. 성공한 tool output은 4097번째 item이라 finish_tool이 직접
// ContextExhausted를 반환하고, poll_tool은 typed failure와 latch를 보존합니다.
#[test]
fn successful_tool_output_replay_overflow_uses_the_typed_failure_path() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![tool_call_round()]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
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
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("fill replay with tool output"),
        })
        .unwrap();
    fill_current_delta_to_item_limit(&mut backend);

    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("tool-output replay exhaustion must fail the current Turn")
    };
    assert_eq!(failure.code(), Some("context_exhausted"));
    assert_eq!(requests.lock().unwrap().len(), 1);
    let next_turn = TurnRef::new(
        turn().session_id(),
        yo_core::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    assert_eq!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: next_turn,
                input: UserInput::from("retry"),
            })
            .unwrap_err()
            .kind(),
        BackendFailureKind::ContextExhausted
    );
}

// 첫 요청 뒤 current delta를 user 포함 4095 items로 채우면 function call은 4096에 맞습니다.
// execution host의 동기 start 실패로 backend가 만드는 failed tool output은 4097번째 item이라
// dispatch의 start-tool 오류 경로도 code=context_exhausted인 Failed Turn과 latch를 보존합니다.
#[test]
fn synchronous_tool_start_failure_replay_overflow_is_typed_and_latched() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![tool_call_round()]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(FailingStartHost),
            Box::new(FixedTokenCounter(1)),
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
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("start the failing tool"),
        })
        .unwrap();
    fill_current_delta_to_item_limit(&mut backend);

    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("synchronous tool-start replay exhaustion must fail the Turn")
    };
    assert_eq!(failure.code(), Some("context_exhausted"));
    assert_eq!(requests.lock().unwrap().len(), 1);
    let next_turn = TurnRef::new(
        turn().session_id(),
        yo_core::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    assert_eq!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: next_turn,
                input: UserInput::from("retry"),
            })
            .unwrap_err()
            .kind(),
        BackendFailureKind::ContextExhausted
    );
}

// 첫 요청 뒤 current delta를 user 포함 4095 items로 채우면 approval 대상 function call은
// 4096에 맞습니다. decline tool output은 4097번째 item이라 approval response command가 Turn을
// 잃지 않고 code=context_exhausted와 latch를 기록하며 request는 첫 1건에서 멈춥니다.
#[test]
fn approval_decline_replay_overflow_is_typed_and_latched() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(vec![tool_call_round()]),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Required),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
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
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("request approval"),
        })
        .unwrap();
    fill_current_delta_to_item_limit(&mut backend);
    let request = loop {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ApprovalRequest { request_id },
            }) => break ActivityRequestRef::new(activity, request_id),
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before approval request"),
        }
    };
    assert!(matches!(
        backend
            .execute_command(AgentCommand::RespondToActivity {
                request,
                response: ActivityResponse::Approval(ApprovalDecision::Declined),
            })
            .unwrap(),
        BackendCommandEvidence::None
    ));

    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("approval-decline replay exhaustion must fail the current Turn")
    };
    assert_eq!(failure.code(), Some("context_exhausted"));
    assert_eq!(requests.lock().unwrap().len(), 1);
    let next_turn = TurnRef::new(
        turn().session_id(),
        yo_core::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    assert_eq!(
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: next_turn,
                input: UserInput::from("retry"),
            })
            .unwrap_err()
            .kind(),
        BackendFailureKind::ContextExhausted
    );
}

// output hard max가 unknown인 profile은 99-token payload에서 제한 필드를 생략한 채 한 번
// count하고 dispatch하지만, 같은 payload가 100-token limit과 같으면 remote call 없이 실패합니다.
#[test]
fn unknown_output_cap_uses_strict_input_boundary_and_omits_the_field() {
    for (input_tokens, admitted) in [(99, true), (100, false)] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut backend = NativeModelBackend::with_connector(
            Box::new(MockConnector {
                rounds: event_rounds(if admitted {
                    vec![Vec::new()]
                } else {
                    Vec::new()
                }),
                requests: Arc::clone(&requests),
            }),
            binding(),
            registry(ToolApprovalRequirement::Automatic),
            NativeModelBackendServices::new(
                Box::new(yo_core::admit_standard_complete_binding),
                Some(Box::new(ExactAdmission)),
                Box::new(MockHost::default()),
                Box::new(FixedTokenCounter(input_tokens)),
            ),
            yo_core::ModelContextProfile::with_optional_output_limit(
                100,
                None,
                "test-tokenizer/v1",
            )
            .unwrap(),
            pressure_at_hard_limit_config(),
        )
        .unwrap();
        backend
            .execute_command(AgentCommand::CreateSession {
                session_id: turn().session_id(),
            })
            .unwrap();
        let evidence = backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("unknown cap"),
            })
            .unwrap();

        assert_eq!(
            matches!(evidence, BackendCommandEvidence::RequestAccepted(_)),
            admitted
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), usize::from(admitted));
        if let Some(request) = requests.first() {
            assert!(
                mock_tokenization_payload(request, "qwen3.8max")
                    .get("max_output_tokens")
                    .is_none()
            );
        }
    }
}

// retained prefix가 이미 4096 items이면 새 user item을 더한 누적 replay가 dispatch 전에
// 거절되고 connector request는 0건이며 Turn에는 typed context exhaustion이 남습니다.
#[test]
fn cumulative_replay_capacity_is_checked_before_dispatch() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut backend = NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(Vec::new()),
            requests: Arc::clone(&requests),
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost::default()),
            Box::new(FixedTokenCounter(1)),
        ),
        yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
        NativeModelBackendConfig::default(),
    )
    .unwrap();
    retain_empty_assistant_items(&mut backend, 4_096);
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("overflow before dispatch"),
        })
        .unwrap();

    let BackendEvent::TurnFinished {
        outcome: TurnOutcome::Failed(failure),
        ..
    } = drain_until_turn(&mut backend)
    else {
        panic!("cumulative replay exhaustion must fail the current Turn")
    };
    assert_eq!(failure.code(), Some("context_exhausted"));
    assert!(requests.lock().unwrap().is_empty());
}

// 완료 응답을 replay에 더하는 순간 누적 한도를 넘더라도 실패 기록이나 재개 Anchor를
// 만들지 않고 현재 Turn을 완결한 뒤 같은 binding의 추가 호출을 차단한다.
#[test]
fn replay_exhaustion_finishes_non_resumably_and_latches_the_binding() {
    let starts = Arc::new(Mutex::new(0));
    let mut backend = backend(
        vec![vec![
            ModelConnectorEvent::ResponseCreated {
                response_id: "full".to_owned(),
            },
            ModelConnectorEvent::TextDelta {
                output_index: 0,
                item_id: "message".to_owned(),
                content_index: 0,
                delta: "answer".to_owned(),
            },
            ModelConnectorEvent::MessageDone {
                output_index: 0,
                item_id: "message".to_owned(),
            },
            completed("full"),
        ]],
        ToolApprovalRequirement::Automatic,
        starts,
    );
    retain_empty_assistant_items(&mut backend, 4_095);
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("one item too many"),
        })
        .unwrap();

    assert!(matches!(
        drain_until_turn(&mut backend),
        BackendEvent::TurnFinished {
            outcome: TurnOutcome::Completed,
            ..
        }
    ));

    let next_turn = TurnRef::new(
        turn().session_id(),
        yo_core::TurnId::new(NonZeroU64::new(2).unwrap()),
    );
    let error = backend
        .execute_command(AgentCommand::StartTurn {
            turn: next_turn,
            input: UserInput::from("retry"),
        })
        .unwrap_err();
    assert_eq!(error.kind(), BackendFailureKind::ContextExhausted);
}

// 실제 worker·disk Journal·managed connector 경계를 이어 자동 압축 뒤 재개한 요청이
// checkpoint의 정확한 보존 문맥만 사용하고 요약 원본이나 private summary를 재생하지 않는지
// 검증한다.
#[test]
fn automatic_compaction_survives_disk_resume_with_exact_retained_connector_input() {
    use std::{fs, path::PathBuf};

    use yo_core::{
        ConnectorError, HostWorkspacePath, ModelConnector, ModelConnectorCancellation,
        ModelConnectorInputItem, ModelConnectorInputRole, ModelConnectorRequest,
        ModelConnectorStreamPort, SessionDescriptor, SessionId, WorkspaceHostId,
        session_repository::{
            LocalSessionReader, LocalSessionRepository, SessionRepository, SessionWriterRepository,
            read_stored_session, recover_stored_session_continuation,
        },
    };

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    struct DiskCheckpointConnector {
        inner: MockConnector,
        storage: PathBuf,
        session_id: SessionId,
    }
    impl ModelConnector for DiskCheckpointConnector {
        fn request_url(&self) -> &str {
            self.inner.request_url()
        }
        fn tokenization_payload(
            &self,
            request: &ModelConnectorRequest,
        ) -> Result<serde_json::Value, ConnectorError> {
            self.inner.tokenization_payload(request)
        }
        fn start(
            &self,
            request: ModelConnectorRequest,
            cancellation: ModelConnectorCancellation,
        ) -> Result<Box<dyn ModelConnectorStreamPort>, ConnectorError> {
            if self.inner.requests.lock().unwrap().len() == 3 {
                // The successor cannot cross the connector boundary before the real
                // on-disk checkpoint exists. This reader acquires no Session writer.
                let reader = LocalSessionReader::open(&self.storage).unwrap();
                let history = read_stored_session(&reader, self.session_id).unwrap();
                assert_eq!(
                    history
                        .records()
                        .iter()
                        .filter(|record| matches!(
                            record,
                            TranscriptRecord::ContextCheckpointCommitted(_)
                        ))
                        .count(),
                    1,
                    "successor dispatch preceded durable checkpoint publication"
                );
            }
            self.inner.start(request, cancellation)
        }
    }
    let root = std::env::temp_dir().join(format!(
        "yo-managed-compaction-resume-{}-{}",
        std::process::id(),
        WorkspaceHostId::new().unwrap(),
    ));
    fs::create_dir(&root).unwrap();
    let fixture = Fixture(root);
    let storage = fixture.0.join("repository");
    let descriptor = SessionDescriptor::new(
        WorkspaceHostId::new().unwrap(),
        HostWorkspacePath::normalize_local(&fixture.0).unwrap(),
    )
    .unwrap();
    let session_id = descriptor.session_id();
    let config = NativeModelBackendConfig::default();
    let system_prompt = config.system_prompt.clone();
    let build_backend = |rounds, requests, counts: Vec<u64>| {
        NativeModelBackend::with_connector(
            Box::new(DiskCheckpointConnector {
                inner: MockConnector {
                    rounds: event_rounds(rounds),
                    requests,
                },
                storage: storage.clone(),
                session_id,
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
            config.clone(),
        )
        .unwrap()
    };
    let summary = portable_summary();
    let mut summary_round = completed_text_round("summary-once", &summary);
    summary_round.insert(1, private_summary_event());
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
    let requests = Arc::new(Mutex::new(Vec::new()));
    let backend = build_backend(
        vec![
            completed_text_round("turn-1", "first"),
            completed_text_round("turn-2", "second"),
            summary_round,
            completed_text_round("turn-3", "third"),
        ],
        Arc::clone(&requests),
        vec![10, 10, 90, 20, 30, 30],
    );
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
    repository.acquire_session_writer(session_id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut session = AgentSession::start_cancellable_with_repository(
        backend,
        descriptor.clone(),
        repository,
        || Instant::now() >= deadline,
    )
    .unwrap()
    .expect("initial startup is bounded");
    let transcript = session.transcript_reader();
    let mut cursor = None;
    for number in [1, 2, 3] {
        queue_intent(
            &mut session,
            AgentIntent::submit(format!("input-{number}")).unwrap(),
        );
        wait_for_turn_finish(&mut session, &transcript, &mut cursor, number);
    }
    session.shutdown().unwrap();
    drop(session);
    drop(transcript);
    assert_eq!(
        requests.lock().unwrap().len(),
        4,
        "one summary request, three ordinary requests"
    );

    let reader = LocalSessionReader::open(&storage).unwrap();
    let history = read_stored_session(&reader, session_id).unwrap();
    let checkpoints = history
        .records()
        .iter()
        .filter(|record| matches!(record, TranscriptRecord::ContextCheckpointCommitted(_)))
        .count();
    assert_eq!(checkpoints, 1, "the automatic checkpoint must be durable");
    let completed_turns = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::EventCommitted(AgentEvent::TurnFinished { turn, outcome }) => {
                Some((*turn, outcome.clone()))
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(completed_turns.len(), 3);
    for (index, (turn, outcome)) in completed_turns.iter().enumerate() {
        assert_eq!(turn.session_id(), session_id);
        assert_eq!(turn.turn_id().get().get(), index as u64 + 1);
        assert_eq!(*outcome, TurnOutcome::Completed);
    }
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
    let prefix = repository.read_after(session_id, None, 4096).unwrap();
    assert!(!prefix.is_empty() && prefix.len() < 4096);
    let continuation = recover_stored_session_continuation(&mut repository, session_id).unwrap();
    assert_eq!(continuation.descriptor(), &descriptor);
    assert_eq!(continuation.target().epoch(), 1);
    assert_eq!(continuation.target().context_epoch(), Some(2));
    let original_binding = continuation.target().binding().clone();

    let resumed_requests = Arc::new(Mutex::new(Vec::new()));
    let backend = build_backend(
        vec![completed_text_round("turn-4", "fourth")],
        Arc::clone(&resumed_requests),
        vec![40],
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut session = AgentSession::start_cancellable_with_continuation(
        backend,
        continuation,
        repository,
        || Instant::now() >= deadline,
    )
    .unwrap()
    .expect("resume startup is bounded");
    assert!(
        resumed_requests.lock().unwrap().is_empty(),
        "recovery must not dispatch model work"
    );
    let transcript = session.transcript_reader();
    let mut cursor = transcript.read_after(None).head();
    queue_intent(&mut session, AgentIntent::submit("input-4").unwrap());
    wait_for_turn_finish(&mut session, &transcript, &mut cursor, 4);
    session.shutdown().unwrap();
    drop(session);
    drop(transcript);

    let message = |role, content: &str| ModelConnectorInputItem::Message {
        role,
        content: content.to_owned(),
        refusal: None,
    };
    let expected = vec![
        message(ModelConnectorInputRole::System, &system_prompt),
        message(ModelConnectorInputRole::User, &summary),
        message(ModelConnectorInputRole::User, "input-2"),
        message(ModelConnectorInputRole::Assistant, "second"),
        message(ModelConnectorInputRole::User, "input-3"),
        message(ModelConnectorInputRole::Assistant, "third"),
        message(ModelConnectorInputRole::User, "input-4"),
    ];
    let resumed = resumed_requests.lock().unwrap();
    assert_eq!(
        resumed.len(),
        1,
        "resume must not repeat the summary or prior model requests"
    );
    assert_eq!(resumed[0].input(), expected);
    assert!(!resumed[0].contains_provider_private_input());
    drop(resumed);
    // The live successor already used the same checkpoint; disk recovery adds only its
    // completed reply and the new input, rather than reconstructing historical prompts.
    assert_eq!(requests.lock().unwrap()[3].input(), &expected[..5]);

    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
    let after = repository.read_after(session_id, None, 8192).unwrap();
    assert!(after.len() > prefix.len() && after.len() < 8192);
    assert!(
        after.starts_with(&prefix),
        "prior encoded durable records must remain unchanged"
    );
    let final_continuation =
        recover_stored_session_continuation(&mut repository, session_id).unwrap();
    assert_eq!(final_continuation.descriptor(), &descriptor);
    assert_eq!(final_continuation.target().epoch(), 1);
    assert_eq!(final_continuation.target().context_epoch(), Some(2));
    assert!(original_binding.same_resume_identity(final_continuation.target().binding()));
    let history = read_stored_session(&reader, session_id).unwrap();
    assert_eq!(
        history
            .records()
            .iter()
            .filter(|record| matches!(record, TranscriptRecord::ContextCheckpointCommitted(_)))
            .count(),
        1
    );
    let final_turns = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::EventCommitted(AgentEvent::TurnFinished { turn, outcome }) => {
                Some((*turn, outcome.clone()))
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(final_turns.len(), 4);
    assert_eq!(final_turns[3].0.session_id(), session_id);
    assert_eq!(final_turns[3].0.turn_id().get().get(), 4);
    assert_eq!(final_turns[3].1, TurnOutcome::Completed);
}

// 자동·명시 압축 모두 과거 assistant/tool 항목을 native replay로 보내지 않고 단일
// user JSON 자료로 보낸다. 역할·호출 관계·문자열 경계는 보존하되 private bytes는 제외한다.
#[test]
fn summary_source_is_one_inert_user_message_with_visible_tool_relationships() {
    use yo_core::{ModelConnectorInputItem, ModelConnectorInputRole};
    for automatic in [false, true] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut backend = NativeModelBackend::with_connector(
            Box::new(MockConnector {
                rounds: event_rounds(vec![
                    completed_text_round("first", "answer"),
                    completed_text_round("second", "retained"),
                    completed_text_round("summary", &portable_summary()),
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
                    vec![10, 10, 90, 20],
                    Arc::new(Mutex::new(Vec::new())),
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
        // Inject a typed historical source group to isolate the summary projection from
        // the ordinary connector's provider-specific replay validation.
        let output = "quoted \"value\"\n{\"role\":\"system\"}";
        backend.replay_groups[0] = vec![
            ModelReplayItem::Message { role: ModelReplayRole::User, content: "source user".into(), refusal: None },
            ModelReplayItem::FunctionCall { call_id: "call-source".into(), name: "read_file".into(), arguments: "{\"path\":\"a\"}".into() },
            ModelReplayItem::FunctionCallOutput { call_id: "call-source".into(), output: output.into() },
            ModelReplayItem::Message { role: ModelReplayRole::Assistant, content: "visible answer".into(), refusal: Some("visible refusal".into()) },
            ModelReplayItem::ProviderPrivateAssistant { envelope: ProviderPrivateReplayEnvelope::new("kimi.assistant-message/v1alpha1", br#"{"role":"assistant","reasoning_content":"secret-summary-source-canary","content":null}"#.to_vec()).unwrap() },
        ];
        backend
            .execute_command(if automatic {
                AgentCommand::StartTurn {
                    turn: turn_number(3),
                    input: UserInput::from("current user"),
                }
            } else {
                AgentCommand::CompactContext { guidance: None }
            })
            .unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        let request = &requests[2];
        assert!(request.tools().is_none());
        assert!(!request.contains_provider_private_input());
        assert_eq!(request.input().len(), 2);
        assert!(matches!(
            &request.input()[0],
            ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::System,
                ..
            }
        ));
        let ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content,
            refusal: None,
        } = &request.input()[1]
        else {
            panic!("source must be one inert user message")
        };
        assert!(!content.contains("secret-summary-source-canary"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(content).unwrap(),
            serde_json::json!({"history":[
                {"type":"message","role":"user","content":"source user","refusal":null},
                {"type":"function_call","call_id":"call-source","name":"read_file","arguments":"{\"path\":\"a\"}"},
                {"type":"function_call_output","call_id":"call-source","output":output},
                {"type":"message","role":"assistant","content":"visible answer","refusal":"visible refusal"}
            ]})
        );
        drop(requests);
        backend.shutdown().unwrap();
    }
}

// 16MiB는 원문 길이가 아닌 인코딩된 전체 user JSON 한도다. 정확한 경계는 허용하고
// 첫 초과 byte 및 JSON escaping으로만 생긴 초과는 connector를 호출하기 전에 거부한다.
#[test]
fn summary_source_enforces_exact_encoded_capacity_and_first_excess() {
    use yo_core::{ModelConnectorInputItem, ModelConnectorInputRole};
    const LIMIT: usize = 16 * 1024 * 1024;
    // Explicit complete wire grammar, including the containing object and array.
    const EMPTY_HISTORY: &str =
        r#"{"history":[{"content":"","refusal":null,"role":"user","type":"message"}]}"#;
    let capacity = LIMIT - EMPTY_HISTORY.len();
    for case in ["exact", "one-byte-excess", "escaping-excess"] {
        let text = match case {
            "exact" => "a".repeat(capacity),
            "one-byte-excess" => "a".repeat(capacity + 1),
            "escaping-excess" => format!("{}\"", "a".repeat(capacity - 1)),
            _ => unreachable!(),
        };
        if case == "escaping-excess" {
            assert_eq!(
                text.len(),
                capacity,
                "raw size still fits the exact boundary"
            );
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut backend = NativeModelBackend::with_connector(
            Box::new(MockConnector {
                rounds: event_rounds(vec![completed_text_round("summary", &portable_summary())]),
                requests: Arc::clone(&requests),
            }),
            binding(),
            registry(ToolApprovalRequirement::Automatic),
            NativeModelBackendServices::new(
                Box::new(yo_core::admit_standard_complete_binding),
                Some(Box::new(ExactAdmission)),
                Box::new(MockHost::default()),
                Box::new(FixedTokenCounter(1)),
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
        backend.replay_groups = vec![
            vec![ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: text,
                refusal: None,
            }],
            vec![ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: "retained".into(),
                refusal: None,
            }],
        ];
        let result = backend.execute_command(AgentCommand::CompactContext { guidance: None });
        if case == "exact" {
            result.unwrap();
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            let ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::User,
                content,
                ..
            } = &requests[0].input()[1]
            else {
                panic!("expected inert summary source")
            };
            assert_eq!(content.len(), LIMIT);
            let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
            assert_eq!(
                parsed["history"][0]["content"].as_str().unwrap().len(),
                capacity
            );
        } else {
            let failure = result.unwrap_err();
            assert_eq!(
                failure.kind(),
                BackendFailureKind::ContextExhausted,
                "{case}"
            );
            assert!(failure.message().contains("encoded visible summary source"));
            assert!(
                requests.lock().unwrap().is_empty(),
                "{case} reached the connector"
            );
        }
        backend.shutdown().unwrap();
    }
}

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

// idle 압축의 비축소·형식 거절 직후 종료해도 CompactContext만으로 기존 Anchor를
// 무효화하지 않는다. 실제 disk를 다시 열어 같은 replay로 다음 Turn을 완료한다.
#[test]
fn rejected_idle_compaction_survives_immediate_disk_shutdown_and_resume() {
    use std::{fs, path::PathBuf};

    use yo_core::{
        AgentControlOutcome, AgentSessionPoll, HostWorkspacePath, SessionDescriptor,
        WorkspaceHostId,
        session_repository::{
            LocalSessionReader, LocalSessionRepository, SessionWriterRepository,
            StoredSessionReader, read_stored_session, read_stored_session_continuation,
            recover_stored_session_continuation,
        },
    };
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    for case in ["nonreduction", "wrong-heading", "missing-section"] {
        let root = std::env::temp_dir().join(format!(
            "yo-idle-rejection-resume-{}-{}",
            std::process::id(),
            WorkspaceHostId::new().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Fixture(root);
        let storage = fixture.0.join("repository");
        let descriptor = SessionDescriptor::new(
            WorkspaceHostId::new().unwrap(),
            HostWorkspacePath::normalize_local(&fixture.0).unwrap(),
        )
        .unwrap();
        let session_id = descriptor.session_id();
        let body = match case {
            "nonreduction" => portable_summary(),
            "wrong-heading" => {
                portable_summary().replacen("# Context Checkpoint", "# secret-malformed-canary", 1)
            },
            "missing-section" => portable_summary()
                .replace("\n## Critical References\nNone.", "")
                .replace("Continue the current task.", "secret-malformed-canary"),
            _ => unreachable!(),
        };
        let mut summary_round = completed_text_round("idle-summary", &body);
        let Some(ModelConnectorEvent::Terminal { usage, .. }) = summary_round.last_mut() else {
            unreachable!()
        };
        *usage = yo_core::ResponsesUsage {
            input_tokens: Some(1),
            output_tokens: Some(1),
            total_tokens: Some(2),
            reasoning_tokens: None,
            cache_read_input_tokens: yo_core::CacheReadInputTokens::Unsupported,
        };
        // The fixed one-token counter makes the valid-summary case nonreducing; the other
        // cases reject invalid formats before successor token admission.
        let managed = backend(
            vec![
                completed_text_round("one", "first"),
                completed_text_round("two", "second"),
                summary_round,
            ],
            ToolApprovalRequirement::Automatic,
            Arc::new(Mutex::new(0)),
        );
        let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
        repository.acquire_session_writer(session_id).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut session = AgentSession::start_cancellable_with_repository(
            managed,
            descriptor.clone(),
            repository,
            || Instant::now() >= deadline,
        )
        .unwrap()
        .unwrap();
        let transcript = session.transcript_reader();
        let mut cursor = None;
        for number in [1, 2] {
            queue_intent(
                &mut session,
                AgentIntent::submit(format!("input-{number}")).unwrap(),
            );
            wait_for_turn_finish(&mut session, &transcript, &mut cursor, number);
        }
        let reader = LocalSessionReader::open(&storage).unwrap();
        let before = read_stored_session_continuation(&reader, session_id).unwrap();
        let original_target = before.target().clone();
        let prefix = reader.read_after(session_id, None, 4096).unwrap();
        assert!(!prefix.is_empty() && prefix.len() < 4096);
        queue_intent(&mut session, AgentIntent::CompactContext { guidance: None });
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            assert_ne!(session.poll().unwrap(), AgentSessionPoll::Closed);
            if let Some(AgentControlOutcome::ContextCompactionRejected { detail }) =
                session.take_control_outcome()
            {
                assert!(detail.contains(if case == "nonreduction" {
                    "did not reduce"
                } else {
                    "required summary format"
                }));
                assert!(!detail.contains("secret-malformed-canary"));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "idle rejection was not delivered"
            );
            thread::sleep(Duration::from_millis(1));
        }
        // No intervening ordinary Turn or new Anchor may rescue continuation before this close.
        session.shutdown().unwrap();
        drop(session);
        drop(transcript);
        let history = read_stored_session(&reader, session_id).unwrap();
        assert_eq!(
            history
                .records()
                .iter()
                .filter(|record| matches!(
                    record,
                    TranscriptRecord::CommandCommitted(AgentCommand::CompactContext { .. })
                ))
                .count(),
            1
        );
        assert!(
            !history
                .records()
                .iter()
                .any(|record| matches!(record, TranscriptRecord::ContextCheckpointCommitted(_)))
        );
        let after = reader.read_after(session_id, None, 8192).unwrap();
        assert!(after.len() > prefix.len() && after.len() < 8192);
        assert!(after.starts_with(&prefix));
        let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
        let continuation =
            recover_stored_session_continuation(&mut repository, session_id).unwrap();
        assert_eq!(continuation.descriptor(), &descriptor);
        assert_eq!(
            continuation.target(),
            &original_target,
            "rejected idle compaction must preserve the newest Anchor, binding, epoch and exact replay"
        );
        let managed = backend(
            vec![completed_text_round("three", "third")],
            ToolApprovalRequirement::Automatic,
            Arc::new(Mutex::new(0)),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut session = AgentSession::start_cancellable_with_continuation(
            managed,
            continuation,
            repository,
            || Instant::now() >= deadline,
        )
        .unwrap()
        .unwrap();
        let transcript = session.transcript_reader();
        let mut cursor = transcript.read_after(None).head();
        queue_intent(&mut session, AgentIntent::submit("input-3").unwrap());
        wait_for_turn_finish(&mut session, &transcript, &mut cursor, 3);
        session.shutdown().unwrap();
        drop(session);
        drop(transcript);
        let history = read_stored_session(&reader, session_id).unwrap();
        let completed = history
            .records()
            .iter()
            .filter_map(|record| match record {
                TranscriptRecord::EventCommitted(AgentEvent::TurnFinished { turn, outcome }) => {
                    Some((*turn, outcome.clone()))
                },
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(completed.len(), 3);
        for (index, (turn, outcome)) in completed.iter().enumerate() {
            assert_eq!(turn.session_id(), session_id);
            assert_eq!(turn.turn_id().get().get(), index as u64 + 1);
            assert_eq!(*outcome, TurnOutcome::Completed);
        }
        assert!(
            !history
                .records()
                .iter()
                .any(|record| matches!(record, TranscriptRecord::ContextCheckpointCommitted(_)))
        );
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

// 실제 부모 Journal에서 준비한 child를 새 managed backend로 시작해 inherited input을 한 번만
// 전송한다. 준비는 connector/tool을 호출하지 않고 부모 저장 bytes를 바꾸지 않는다.
#[test]
fn prepared_managed_fork_resumes_exact_context_once_without_touching_parent() {
    use std::{fs, path::PathBuf};

    use yo_core::{
        HostWorkspacePath, ModelConnectorInputItem, ModelConnectorInputRole, ModelContextProfile,
        SessionDescriptor, WorkspaceHostId, admit_standard_complete_binding,
        session_repository::{
            LocalSessionReader, LocalSessionRepository, SessionWriterRepository,
            StoredSessionReader, read_stored_session_continuation,
        },
    };
    struct ForkTestHost(Arc<Mutex<usize>>);
    impl ToolExecutionHost for ForkTestHost {
        fn identity(&self) -> &str {
            "test-host-v1"
        }
        fn is_available(&self, _tool: &ToolId) -> bool {
            true
        }
        fn start(
            &mut self,
            _request: ToolExecutionRequest,
        ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
            *self.0.lock().unwrap() += 1;
            Err(ToolExecutionError::new(
                "unexpected tool execution in text-only fork fixture",
            ))
        }
        fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
            Ok(())
        }
    }
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let root = std::env::temp_dir().join(format!(
        "yo-managed-fork-{}",
        WorkspaceHostId::new().unwrap()
    ));
    fs::create_dir(&root).unwrap();
    let fixture = Fixture(root);
    let storage = fixture.0.join("repository");
    let host = WorkspaceHostId::new().unwrap();
    let workspace = HostWorkspacePath::normalize_local(&fixture.0).unwrap();
    let parent_descriptor = SessionDescriptor::new(host, workspace.clone()).unwrap();
    let parent_id = parent_descriptor.session_id();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let tool_starts = Arc::new(Mutex::new(0));
    let make_backend = |rounds| {
        NativeModelBackend::with_connector(
            Box::new(MockConnector {
                rounds: event_rounds(rounds),
                requests: Arc::clone(&requests),
            }),
            binding(),
            registry(ToolApprovalRequirement::Automatic),
            NativeModelBackendServices::new(
                Box::new(admit_standard_complete_binding),
                Some(Box::new(ExactAdmission)),
                Box::new(ForkTestHost(Arc::clone(&tool_starts))),
                Box::new(FixedTokenCounter(1)),
            ),
            ModelContextProfile::new(1000, 10, "test-tokenizer/v1").unwrap(),
            NativeModelBackendConfig::default(),
        )
        .unwrap()
    };
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
    repository.acquire_session_writer(parent_id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut parent = AgentSession::start_cancellable_with_repository(
        make_backend(vec![completed_text_round("parent", "remembered")]),
        parent_descriptor,
        repository,
        || Instant::now() >= deadline,
    )
    .unwrap()
    .unwrap();
    let transcript = parent.transcript_reader();
    queue_intent(&mut parent, AgentIntent::submit("parent input").unwrap());
    wait_for_turn_finish(&mut parent, &transcript, &mut None, 1);
    parent.shutdown().unwrap();
    drop(parent);
    drop(transcript);
    let reader = LocalSessionReader::open(&storage).unwrap();
    let before = reader.read_session(parent_id).unwrap();
    let continuation = read_stored_session_continuation(&reader, parent_id).unwrap();
    let candidate = make_backend(vec![completed_text_round("child", "child answer")]);
    let child = SessionDescriptor::new(host, workspace).unwrap();
    let child_id = child.session_id();
    let prepared = candidate.prepare_exact_fork(&continuation, child).unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(*tool_starts.lock().unwrap(), 0);
    assert!(candidate.session.is_none());
    assert_eq!(
        prepared.target().model_replay(),
        continuation.target().model_replay()
    );
    assert_ne!(
        prepared.target().binding().session_locator(),
        continuation.target().binding().session_locator()
    );
    // 실제 startup에서 사용 불가능한 candidate를 재사용한 준비도 명확히 거절한다.
    let extra_child = SessionDescriptor::new(
        host,
        HostWorkspacePath::normalize_local(&fixture.0).unwrap(),
    )
    .unwrap();
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
    repository.acquire_session_writer(child_id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut child_session =
        AgentSession::start_cancellable_with_continuation(candidate, prepared, repository, || {
            Instant::now() >= deadline
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "resume must not issue a model request"
    );
    let child_transcript = child_session.transcript_reader();
    queue_intent(
        &mut child_session,
        AgentIntent::submit("child input").unwrap(),
    );
    wait_for_turn_finish(&mut child_session, &child_transcript, &mut None, 1);
    child_session.shutdown().unwrap();
    drop(child_session);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let expected = vec![
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::System,
            content: NativeModelBackendConfig::default().system_prompt,
            refusal: None,
        },
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "parent input".into(),
            refusal: None,
        },
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::Assistant,
            content: "remembered".into(),
            refusal: None,
        },
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "child input".into(),
            refusal: None,
        },
    ];
    assert_eq!(requests[1].input(), expected);
    assert_eq!(*tool_starts.lock().unwrap(), 0);
    assert_eq!(reader.read_session(parent_id).unwrap(), before);
    let child_recovered = read_stored_session_continuation(&reader, child_id).unwrap();
    assert_eq!(child_recovered.target().epoch(), 1);
    assert_eq!(child_recovered.target().model_replay().items().len(), 4);
    let mut closed = make_backend(Vec::new());
    closed.shutdown().unwrap();
    assert!(
        closed
            .prepare_exact_fork(&continuation, extra_child)
            .is_err()
    );
}
