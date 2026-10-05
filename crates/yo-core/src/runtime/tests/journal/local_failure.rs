use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::{
    journal::codec::{self, JournalCommit, JournalRecord, LocalFailureSource},
    session_repository::{SessionWriterRepository, recover_stored_session_continuation},
};

#[derive(Clone, Default)]
struct LocalFailureRepository {
    entries: Arc<Mutex<Vec<RepositoryEntry>>>,
    fail_append: Arc<AtomicBool>,
}

impl SessionRepository for LocalFailureRepository {
    fn append(
        &mut self,
        _: crate::SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        if self.fail_append.load(Ordering::SeqCst) {
            return Err(AppendError::Repository(RepositoryError::Unavailable {
                message: "injected settlement failure".into(),
            }));
        }
        let mut entries = self.entries.lock().unwrap();
        let sequence = RepositorySequence::new(entries.len() as u64 + 1);
        entries.push(RepositoryEntry::new(sequence, record));
        Ok(AppendReceipt::new(sequence))
    }
    fn read_after(
        &self,
        _: crate::SessionId,
        after: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| entry.sequence().get() > after.map_or(0, RepositorySequence::get))
            .take(limit)
            .cloned()
            .collect())
    }
}

impl SessionWriterRepository for LocalFailureRepository {
    fn acquire_session_writer(&mut self, _: crate::SessionId) -> Result<(), RepositoryError> {
        Ok(())
    }
}

fn local_failure_runtime() -> (
    AgentRuntime<ScriptedBackend>,
    crate::TurnRef,
    LocalFailureRepository,
    Vec<ModelReplayItem>,
) {
    local_failure_runtime_with_steps(Vec::new())
}

fn local_failure_runtime_with_steps(
    extra: Vec<BackendScriptStep>,
) -> (
    AgentRuntime<ScriptedBackend>,
    crate::TurnRef,
    LocalFailureRepository,
    Vec<ModelReplayItem>,
) {
    let session_id = session(71);
    let active = turn(session_id, 1);
    let create = AgentCommand::CreateSession { session_id };
    let start = AgentCommand::StartTurn {
        turn: active,
        input: UserInput::new("keep exact context"),
    };
    let policy = ContextPolicyChanged::try_new(
        1,
        true,
        ContextStrategy::PortableSummaryV1Alpha1,
        85,
        90,
        Some(10),
        Some(65_536),
    )
    .unwrap();
    let mut steps = vec![
        BackendScriptStep::AcceptCommandWithEvidence {
            command: create.clone(),
            evidence: BackendCommandEvidence::BindingOpened(exact_replay_binding_evidence()),
        },
        BackendScriptStep::Emit(BackendEvent::ContextPolicyChanged { policy }),
        BackendScriptStep::AcceptCommandWithEvidence {
            command: start.clone(),
            evidence: BackendCommandEvidence::RequestAccepted(request_evidence()),
        },
    ];
    steps.extend(extra);
    steps.push(BackendScriptStep::Shutdown(Ok(())));
    let backend =
        ScriptedBackend::new(steps).with_capabilities(BackendCapabilities::none().with_steer());
    let repository = LocalFailureRepository::default();
    let mut journal = SessionJournal::with_repository_and_descriptor(
        Box::new(repository.clone()),
        crate::fixture_descriptor(session_id),
    );
    journal.initialize_durability();
    let mut runtime = AgentRuntime::with_journal(backend, journal);
    runtime.execute_command(create).unwrap();
    runtime.execute_submission(start, submission(71)).unwrap();
    let items = vec![UserInput::new("keep exact context").model_replay_item()];
    (runtime, active, repository, items)
}

fn close_local_failure_group(
    runtime: &mut AgentRuntime<ScriptedBackend>,
    turn: crate::TurnRef,
    index: u64,
    items: &mut Vec<ModelReplayItem>,
) {
    let call = activity(turn, index);
    for event in [
        BackendEvent::ActivityStarted {
            activity: call,
            kind: ActivityKind::ToolCall,
        },
        BackendEvent::ActivityUpdated {
            activity: call,
            update: ActivityUpdate::TextSnapshot(format!("tool-{index}")),
        },
        BackendEvent::ActivityFinished {
            activity: call,
            outcome: ActivityOutcome::Completed,
        },
    ] {
        runtime.apply_backend_event(event).unwrap();
    }
    items.extend([
        ModelReplayItem::FunctionCall {
            call_id: format!("call-{index}"),
            name: "read_file".into(),
            arguments: format!("{{\"path\":\"file-{index}\"}}"),
        },
        ModelReplayItem::FunctionCallOutput {
            call_id: format!("call-{index}"),
            output: format!("result-{index}"),
        },
    ]);
    runtime
        .apply_backend_event(BackendEvent::ContextActiveSuffixCompleted {
            turn,
            items: items.clone(),
        })
        .unwrap();
}

fn local_failure_proposal(turn: crate::TurnRef, items: Vec<ModelReplayItem>) -> BackendEvent {
    BackendEvent::LocalArgumentRejectionPrepared {
        turn,
        failure: crate::Failure::new("typed argument rejection"),
        replay: Some(ModelReplayDelta::new(
            Some(ModelReplayContract::new("system", Vec::new())),
            items,
        )),
    }
}

fn local_failure_commits(repository: &LocalFailureRepository) -> Vec<JournalCommit> {
    repository
        .entries
        .lock()
        .unwrap()
        .iter()
        .map(|entry| codec::decode(entry.record().payload()).unwrap())
        .collect()
}

// 여러 완료 묶음을 보존해도 실패 상태와 최신 실패 요청을 유지하고 실제 재개 소비자에게 같은 문맥을
// 전달합니다.
#[test]
fn local_failure_settlement_preserves_cumulative_context_and_cold_resume_payload() {
    let (mut runtime, active, mut repository, mut items) = local_failure_runtime();
    close_local_failure_group(&mut runtime, active, 1, &mut items);
    runtime
        .apply_backend_event(BackendEvent::ModelRequestAccepted {
            turn: active,
            evidence: request_evidence(),
        })
        .unwrap();
    close_local_failure_group(&mut runtime, active, 2, &mut items);
    let source_end = runtime.journal().last_sequence().unwrap();
    runtime
        .apply_backend_event(BackendEvent::ModelRequestAccepted {
            turn: active,
            evidence: request_evidence(),
        })
        .unwrap();
    let failed_request = runtime.accepted_requests[&active];
    assert!(source_end < failed_request);
    let partial = activity(active, 3);
    for event in [
        BackendEvent::ActivityStarted {
            activity: partial,
            kind: ActivityKind::AgentMessage,
        },
        BackendEvent::ActivityUpdated {
            activity: partial,
            update: ActivityUpdate::TextSnapshot("visible partial response".into()),
        },
        BackendEvent::ActivityFinished {
            activity: partial,
            outcome: ActivityOutcome::Failed(crate::Failure::new("argument rejection")),
        },
    ] {
        runtime.apply_backend_event(event).unwrap();
    }
    let before = repository.entries.lock().unwrap().len();
    let observed = runtime
        .apply_backend_event(local_failure_proposal(active, items.clone()))
        .unwrap();
    assert!(matches!(
        observed,
        crate::RuntimePoll::Event(crate::AgentEvent::TurnFinished {
            outcome: TurnOutcome::Failed(_),
            ..
        })
    ));
    assert_eq!(repository.entries.lock().unwrap().len(), before + 1);
    assert_eq!(runtime.model_replay.items(), items);
    assert!(runtime.journal().entries().iter().any(|entry| matches!(entry.record(), SemanticRecord::EventCommitted(crate::AgentEvent::ActivityUpdated { activity, update: ActivityUpdate::TextSnapshot(text) }) if *activity == partial && text == "visible partial response")));
    let commits = local_failure_commits(&repository);
    let terminal = commits.last().unwrap();
    assert_eq!(terminal.records().len(), 4);
    let JournalRecord::BackendResumableOutcome(outcome) = terminal.records()[2].record() else {
        panic!("settlement record");
    };
    assert_eq!(outcome.accepted_request_sequence(), failed_request);
    assert!(outcome.outcome_identity().is_none());
    let wire = codec::encode(terminal).unwrap();
    assert!(wire.contains("\"status\":\"failed\""));
    assert!(wire.contains("\"profile\":\"yo.local-failure-context/v1\""));
    assert!(!wire.contains("outcome_identity"));
    let recovered = codec::recover(&commits).unwrap();
    assert_eq!(recovered.model_replay().items(), items);
    let continuation =
        recover_stored_session_continuation(&mut repository, active.session_id()).unwrap();
    assert_eq!(continuation.target().model_replay().items(), items);
    let target = continuation.target().clone();
    let backend = ScriptedBackend::new([
        BackendScriptStep::Resume {
            target: Box::new(target.clone()),
            evidence: exact_replay_binding_evidence(),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let journal =
        SessionJournal::with_repository_and_continuation(Box::new(repository), &continuation);
    let mut resumed = AgentRuntime::with_journal(backend, journal);
    resumed.initialize_resume(&target).unwrap();
    assert_eq!(resumed.backend().remaining_steps(), 1);
    assert_eq!(resumed.model_replay.items(), items);
    resumed.shutdown().unwrap();
    runtime.shutdown().unwrap();
}

// 보존 후보의 변조나 내구성 추기 실패 뒤에는 공개 poll 전에 직접 StartTurn도 backend에 도달하지
// 않습니다.
#[test]
fn local_failure_settlement_failure_latches_every_public_continuation_entry() {
    for corrupt in [false, true] {
        let (mut runtime, active, repository, mut items) = local_failure_runtime();
        close_local_failure_group(&mut runtime, active, 1, &mut items);
        runtime
            .apply_backend_event(BackendEvent::ModelRequestAccepted {
                turn: active,
                evidence: request_evidence(),
            })
            .unwrap();
        if corrupt {
            items.pop();
        } else {
            repository.fail_append.store(true, Ordering::SeqCst);
        }
        let before_replay = runtime.model_replay.clone();
        assert!(
            runtime
                .apply_backend_event(local_failure_proposal(active, items))
                .is_err()
        );
        let remaining = runtime.backend().remaining_steps();
        assert!(
            runtime
                .execute_submission(
                    AgentCommand::StartTurn {
                        turn: turn(active.session_id(), 2),
                        input: UserInput::new("must not dispatch")
                    },
                    submission(72)
                )
                .is_err()
        );
        assert!(
            runtime
                .execute_command(AgentCommand::InterruptTurn { turn: active })
                .is_err()
        );
        assert!(runtime.poll_event().is_err());
        assert_eq!(runtime.backend().remaining_steps(), remaining);
        assert_eq!(runtime.model_replay, before_replay);
        assert!(!local_failure_commits(&repository).iter().flat_map(|commit| commit.records()).any(|entry| matches!(entry.record(), JournalRecord::BackendResumableOutcome(outcome) if outcome.local_failure_source().is_some())));
    }
}

// 입력만 있는 첫 요청에는 보존할 완료 묶음이 없으며 비밀 제출 뒤에도 로컬 실패 정산을 만들 수
// 없습니다.
#[test]
fn local_failure_settlement_rejects_input_only_and_secret_barriers() {
    for secret in [false, true] {
        let (mut runtime, active, repository, mut items) = local_failure_runtime();
        if secret {
            close_local_failure_group(&mut runtime, active, 1, &mut items);
            runtime.secret_input_terminal = true;
        }
        runtime
            .apply_backend_event(BackendEvent::ModelRequestAccepted {
                turn: active,
                evidence: request_evidence(),
            })
            .unwrap();
        assert!(
            runtime
                .apply_backend_event(local_failure_proposal(active, items))
                .is_err()
        );
        assert!(
            codec::recover(&local_failure_commits(&repository))
                .unwrap()
                .continuation_anchor()
                .is_none()
        );
    }
}

// 새 닫힌 객체의 누락·null·중복·알 수 없는 값과 분리된 terminal chain은 복구 권한을 만들지
// 않습니다.
#[test]
fn local_failure_wire_and_atomic_chain_fail_closed() {
    let (mut runtime, active, repository, mut items) = local_failure_runtime();
    close_local_failure_group(&mut runtime, active, 1, &mut items);
    runtime
        .apply_backend_event(BackendEvent::ModelRequestAccepted {
            turn: active,
            evidence: request_evidence(),
        })
        .unwrap();
    runtime
        .apply_backend_event(local_failure_proposal(active, items))
        .unwrap();
    let commits = local_failure_commits(&repository);
    let wire = codec::encode(commits.last().unwrap()).unwrap();
    for (from, to) in [
        ("\"status\":\"failed\"", "\"status\":\"completed\""),
        (
            "\"profile\":\"yo.local-failure-context/v1\"",
            "\"profile\":null",
        ),
        ("\"cleanup\":\"succeeded\"", "\"cleanup\":\"unknown\""),
        (
            "\"cause\":\"tool_argument_semantic_admission_rejected\"",
            "\"cause\":\"other\"",
        ),
        (
            "\"kind\":\"active_suffix\"",
            "\"kind\":\"active_suffix\",\"extra\":1",
        ),
        (
            "\"kind\":\"active_suffix\"",
            "\"kind\":\"active_suffix\",\"kind\":\"active_suffix\"",
        ),
        (
            "\"cleanup\":\"succeeded\"",
            "\"cleanup\":\"succeeded\",\"cleanup\":\"succeeded\"",
        ),
        (
            "\"status\":\"failed\"",
            "\"status\":\"failed\",\"outcome_identity\":null",
        ),
    ] {
        assert!(
            codec::decode(&wire.replace(from, to)).is_err(),
            "accepted mutation {to}"
        );
    }
    for (field, canonical) in [
        ("status", "failed"),
        ("profile", "yo.local-failure-context/v1"),
        ("cause", "tool_argument_semantic_admission_rejected"),
        ("cleanup", "succeeded"),
    ] {
        let from = format!("\"{field}\":\"{canonical}\"");
        for wrong_type in [
            format!("{{\"{canonical}\":null}}"),
            format!("[\"{canonical}\"]"),
            "null".into(),
            "true".into(),
            "1".into(),
        ] {
            let to = format!("\"{field}\":{wrong_type}");
            assert!(wire.contains(&from));
            assert!(
                codec::decode(&wire.replace(&from, &to)).is_err(),
                "accepted {to}"
            );
        }
    }
    // 기존 Completed reader의 unit-map 수용과 canonical writer 바이트는 유지합니다.
    let mut completed_wire: serde_json::Value = serde_json::from_str(&wire).unwrap();
    completed_wire["records"][0]["event"]["outcome"] = serde_json::json!({"type":"completed"});
    completed_wire["records"][2]["status"] = "completed".into();
    completed_wire["records"][2]
        .as_object_mut()
        .unwrap()
        .remove("settlement");
    let completed = codec::decode(&completed_wire.to_string()).unwrap();
    let canonical_completed = codec::encode(&completed).unwrap();
    let historical_completed = canonical_completed.replace(
        "\"status\":\"completed\"",
        "\"status\":{\"completed\":null}",
    );
    assert_eq!(codec::decode(&historical_completed).unwrap(), completed);
    assert_eq!(
        codec::encode(&codec::decode(&historical_completed).unwrap()).unwrap(),
        canonical_completed
    );
    let terminal = commits.last().unwrap();
    for count in 2..4 {
        let records = terminal.records()[..count].to_vec();
        let cutoff = records.last().unwrap().journal_sequence().unwrap();
        let mut truncated = commits[..commits.len() - 1].to_vec();
        truncated.push(JournalCommit::incremental_through(cutoff, records));
        assert!(codec::recover(&truncated).is_err());
    }
}

// 완료 묶음 뒤에 이미 수락한 조향은 마지막 도구 경계로 잘리지 않고 정확한 suffix에 남습니다.
#[test]
fn local_failure_settlement_keeps_committed_steering_after_closed_groups() {
    let active = turn(session(71), 1);
    let steer = AgentCommand::SteerTurn {
        turn: active,
        input: UserInput::new("accepted correction"),
    };
    let (mut runtime, active, repository, mut items) =
        local_failure_runtime_with_steps(vec![BackendScriptStep::AcceptCommand(steer.clone())]);
    close_local_failure_group(&mut runtime, active, 1, &mut items);
    runtime.execute_submission(steer, submission(73)).unwrap();
    items.push(UserInput::new("accepted correction").model_replay_item());
    runtime
        .apply_backend_event(BackendEvent::ContextActiveSuffixCompleted {
            turn: active,
            items: items.clone(),
        })
        .unwrap();
    runtime
        .apply_backend_event(BackendEvent::ModelRequestAccepted {
            turn: active,
            evidence: request_evidence(),
        })
        .unwrap();
    runtime
        .apply_backend_event(local_failure_proposal(active, items.clone()))
        .unwrap();
    assert_eq!(
        codec::recover(&local_failure_commits(&repository))
            .unwrap()
            .model_replay()
            .items(),
        items
    );
}

fn local_failure_checkpoint_proposal(
    turn: crate::TurnRef,
    summarized: Vec<ModelReplayItem>,
    active: Vec<ModelReplayItem>,
) -> ContextCheckpointProposal {
    ContextCheckpointProposal::new(Some(turn), 1, 100, 90, 40, ModelReplayContract::new("system", Vec::new()),
        "# Context Checkpoint\n## Current Objective\nContinue.\n## Active Constraints\nNone.\n## Decisions\nRetain closed context.\n## Verified Progress\nPrior turn done.\n## Current State\nTool groups closed.\n## Unknown or Unverified\nNone.\n## Next Actions\nContinue.\n## Critical References\nNone.",
        vec![summarized], Vec::new(), active,
        serde_json::json!({ "schema": "yo.model-usage-receipt/v1", "response_id": "failure-summary", "round": 1,
            "provider": "test", "account": "default", "model": "test-model", "connector": "openai-responses",
            "api_dialect": "openai-responses", "base_url": "https://example.invalid/",
            "usage": { "input_tokens": 90, "output_tokens": 10, "total_tokens": 100, "reasoning_tokens": 0 },
            "cache_read_input_tokens": { "availability": "unsupported" } })).unwrap()
}

// 현재 Turn의 체크포인트 뒤 빈 suffix는 delta 없이, 조향만 있는 suffix는 그 입력만 한 번
// 기록합니다.
#[test]
fn local_failure_checkpoint_root_preserves_empty_and_steering_only_suffixes() {
    for with_steer in [false, true] {
        let active = turn(session(71), 2);
        let start = AgentCommand::StartTurn {
            turn: active,
            input: UserInput::new("active after baseline"),
        };
        let steer = AgentCommand::SteerTurn {
            turn: active,
            input: UserInput::new("checkpoint correction"),
        };
        let mut extra = vec![BackendScriptStep::AcceptCommandWithEvidence {
            command: start.clone(),
            evidence: BackendCommandEvidence::RequestAccepted(request_evidence()),
        }];
        if with_steer {
            extra.push(BackendScriptStep::AcceptCommand(steer.clone()));
        }
        let (mut runtime, first_turn, mut repository, mut first_group) =
            local_failure_runtime_with_steps(extra);
        first_group.push(ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "baseline finished".into(),
            refusal: None,
        });
        runtime
            .apply_backend_event(BackendEvent::ResumableTurnFinished {
                turn: first_turn,
                evidence: BackendOutcomeEvidence::without_identity().with_replay(
                    ModelReplayDelta::new(
                        Some(ModelReplayContract::new("system", Vec::new())),
                        first_group.clone(),
                    ),
                ),
            })
            .unwrap();
        runtime.execute_submission(start, submission(74)).unwrap();
        let mut active_group = vec![UserInput::new("active after baseline").model_replay_item()];
        close_local_failure_group(&mut runtime, active, 1, &mut active_group);
        runtime
            .apply_backend_event(BackendEvent::ContextCheckpointPrepared {
                proposal: local_failure_checkpoint_proposal(active, first_group, active_group),
            })
            .unwrap();
        let checkpoint = runtime.resume_source.unwrap().sequence();
        let mut expected = runtime.model_replay.items().to_vec();
        let replay = if with_steer {
            runtime.execute_submission(steer, submission(75)).unwrap();
            let suffix = vec![UserInput::new("checkpoint correction").model_replay_item()];
            runtime
                .apply_backend_event(BackendEvent::ContextActiveSuffixCompleted {
                    turn: active,
                    items: suffix.clone(),
                })
                .unwrap();
            expected.extend(suffix.iter().cloned());
            Some(ModelReplayDelta::new(None, suffix))
        } else {
            None
        };
        // pressure 관찰은 새 semantic group이 아니므로 보존된 경계를 무효화하지 않습니다.
        let pressure = activity(active, 10);
        for event in [
            BackendEvent::ActivityStarted {
                activity: pressure,
                kind: ActivityKind::ModelWork,
            },
            BackendEvent::ActivityFinished {
                activity: pressure,
                outcome: ActivityOutcome::Completed,
            },
        ] {
            runtime.apply_backend_event(event).unwrap();
        }
        runtime
            .apply_backend_event(BackendEvent::ModelRequestAccepted {
                turn: active,
                evidence: request_evidence(),
            })
            .unwrap();
        let before = repository.entries.lock().unwrap().len();
        runtime
            .apply_backend_event(BackendEvent::LocalArgumentRejectionPrepared {
                turn: active,
                failure: crate::Failure::new("typed rejection"),
                replay,
            })
            .unwrap();
        assert_eq!(repository.entries.lock().unwrap().len(), before + 1);
        let commits = local_failure_commits(&repository);
        let terminal = commits.last().unwrap();
        assert_eq!(terminal.records().len(), if with_steer { 4 } else { 3 });
        let JournalRecord::BackendResumableOutcome(outcome) =
            terminal.records()[terminal.records().len() - 2].record()
        else {
            panic!("outcome");
        };
        if !with_steer {
            assert_eq!(
                outcome.local_failure_source(),
                Some(LocalFailureSource::Checkpoint {
                    checkpoint_sequence: checkpoint
                })
            );
        }
        let continuation =
            recover_stored_session_continuation(&mut repository, active.session_id()).unwrap();
        assert_eq!(continuation.target().model_replay().items(), expected);
        assert_eq!(
            codec::recover(&[continuation.snapshot()])
                .unwrap()
                .model_replay()
                .items(),
            expected
        );
        runtime.shutdown().unwrap();
    }
}

// 이전 요청·다른 root·실패 요청 경계·누락된 도구 묶음으로 고친 저장 바이트는 재개 권한이 되지
// 않습니다.
#[test]
fn local_failure_recovery_rejects_forged_source_and_request_coordinates() {
    let (mut runtime, active, repository, mut items) = local_failure_runtime();
    let earlier_request = runtime.accepted_requests[&active].get();
    close_local_failure_group(&mut runtime, active, 1, &mut items);
    runtime
        .apply_backend_event(BackendEvent::ModelRequestAccepted {
            turn: active,
            evidence: request_evidence(),
        })
        .unwrap();
    close_local_failure_group(&mut runtime, active, 2, &mut items);
    runtime
        .apply_backend_event(BackendEvent::ModelRequestAccepted {
            turn: active,
            evidence: request_evidence(),
        })
        .unwrap();
    let failed_request = runtime.accepted_requests[&active].get();
    runtime
        .apply_backend_event(local_failure_proposal(active, items))
        .unwrap();
    let commits = local_failure_commits(&repository);
    let wire: serde_json::Value =
        serde_json::from_str(&codec::encode(commits.last().unwrap()).unwrap()).unwrap();
    for case in 0..6 {
        let mut changed = wire.clone();
        let records = changed["records"].as_array_mut().unwrap();
        match case {
            0 => {
                for record in records.iter_mut().skip(1) {
                    record["accepted_request_sequence"] = earlier_request.into();
                }
            },
            1 => {
                records[2]["settlement"]["source"]["last_sequence"] = failed_request.into();
            },
            2 => {
                records[2]["settlement"]["source"]["first_sequence"] = 1.into();
            },
            3 => {
                for record in records.iter_mut().skip(1) {
                    record["context_epoch"] = 2.into();
                }
            },
            4 => {
                let items = records[1]["items"].as_array_mut().unwrap();
                items.drain(1..3);
            },
            5 => {
                records[1]["items"][2]["call_id"] = "foreign-call".into();
            },
            _ => unreachable!(),
        }
        let mut history = commits[..commits.len() - 1].to_vec();
        match codec::decode(&changed.to_string()) {
            Err(_) => {},
            Ok(commit) => {
                history.push(commit);
                assert!(
                    codec::recover(&history).is_err(),
                    "accepted forged case {case}"
                );
            },
        }
    }
}

// 누적 재개 문맥의 4096번째 항목은 보존하고 첫 초과인 4097번째에서는 이전 Anchor로 돌아가지
// 않습니다.
#[test]
fn local_failure_settlement_enforces_first_excess_cumulative_item() {
    for baseline_count in [4093, 4094] {
        let active = turn(session(71), 2);
        let start = AgentCommand::StartTurn {
            turn: active,
            input: UserInput::new("bounded continuation"),
        };
        let (mut runtime, first, repository, mut baseline) =
            local_failure_runtime_with_steps(vec![BackendScriptStep::AcceptCommandWithEvidence {
                command: start.clone(),
                evidence: BackendCommandEvidence::RequestAccepted(request_evidence()),
            }]);
        baseline.extend((1..baseline_count).map(|_| ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "x".into(),
            refusal: None,
        }));
        runtime
            .apply_backend_event(BackendEvent::ResumableTurnFinished {
                turn: first,
                evidence: BackendOutcomeEvidence::without_identity().with_replay(
                    ModelReplayDelta::new(
                        Some(ModelReplayContract::new("system", Vec::new())),
                        baseline,
                    ),
                ),
            })
            .unwrap();
        runtime.execute_submission(start, submission(78)).unwrap();
        let mut suffix = vec![UserInput::new("bounded continuation").model_replay_item()];
        close_local_failure_group(&mut runtime, active, 1, &mut suffix);
        runtime
            .apply_backend_event(BackendEvent::ModelRequestAccepted {
                turn: active,
                evidence: request_evidence(),
            })
            .unwrap();
        let result = runtime.apply_backend_event(BackendEvent::LocalArgumentRejectionPrepared {
            turn: active,
            failure: crate::Failure::new("typed rejection"),
            replay: Some(ModelReplayDelta::new(None, suffix)),
        });
        let recovered = codec::recover(&local_failure_commits(&repository)).unwrap();
        if baseline_count == 4093 {
            result.unwrap();
            assert_eq!(recovered.model_replay().items().len(), 4096);
            assert!(recovered.continuation_anchor().is_some());
        } else {
            assert!(result.is_err());
            assert_eq!(recovered.model_replay().items().len(), 4094);
            assert!(recovered.continuation_anchor().is_none());
            assert!(runtime.poll_event().is_err());
        }
    }
}
