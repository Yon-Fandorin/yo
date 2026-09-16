use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    AgentCommand, AgentIntent, AgentSession, BackendCommandEvidence, BackendEvent,
    BackendFailureKind, BackendPoll, CommandAdmission, ModelConnectorEvent, ModelReplayItem,
    ModelReplayRole, ToolApprovalRequirement, UserInput,
};

use super::{
    super::support::{
        ExactAdmission, MockConnector, MockHost, binding, drain_until_turn, event_rounds, registry,
        turn,
    },
    fixtures::{
        SequenceTokenCounter, completed_summary_round, completed_text_round, portable_summary,
        private_summary_event, queue_intent, turn_number, wait_for_turn_finish,
    },
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

// 명시적 idle 압축도 자동 압축과 동일한 bounded summary·checkpoint 파이프라인을 사용합니다.
#[test]
fn explicit_idle_compaction_uses_the_same_bounded_summary_pipeline() {
    for output_index in [0, 1] {
        check_explicit_idle_compaction_uses_the_same_bounded_summary_pipeline(output_index);
    }
}

fn check_explicit_idle_compaction_uses_the_same_bounded_summary_pipeline(output_index: usize) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let summary = portable_summary();
    let mut summary_round = completed_summary_round("manual-summary", &summary, output_index);
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
