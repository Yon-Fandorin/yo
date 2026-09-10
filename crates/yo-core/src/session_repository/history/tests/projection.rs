use super::{
    super::{
        StoredDiscoveryValidation, StoredSessionContinuity, StoredSessionRecovery,
        read_stored_session,
    },
    support::{MemoryReader, activity, finished, record_with_discovery, session, started},
};
use crate::{
    ActivityOutcome, ActivityUpdate, AgentEvent, JournalSequence, TranscriptRecord,
    journal::codec::{
        JournalCommit, JournalRecord, MessageEnded, MessageOutcome, MessageReset, MessageSegment,
        MessageStream, MessageTerminal, SequencedJournalRecord,
    },
    session_repository::{RecordDiscovery, RepositoryEntry, RepositorySequence},
};

fn reader(commits: &[JournalCommit]) -> MemoryReader {
    let descriptor = commits
        .iter()
        .flat_map(JournalCommit::records)
        .find_map(|entry| match entry.record() {
            JournalRecord::SessionDescriptor(descriptor) => Some(descriptor.clone()),
            _ => None,
        })
        .unwrap_or_else(|| crate::fixture_descriptor(session()));
    MemoryReader {
        entries: commits
            .iter()
            .enumerate()
            .map(|(index, commit)| {
                RepositoryEntry::new(
                    RepositorySequence::new(u64::try_from(index).unwrap() + 1),
                    record_with_discovery(commit, RecordDiscovery::new(descriptor.clone())),
                )
            })
            .collect(),
        missing: false,
    }
}

fn inherited_child_snapshot(history: Vec<SequencedJournalRecord>) -> JournalCommit {
    use std::collections::HashMap;

    use crate::{
        BackendBindingEvidence, BackendIdentity, ContextPolicyChanged, ContextStrategy,
        ContinuationStrategy, ModelReplayContract, ModelReplayItem, ModelReplayRole,
        ProviderPrivateReplayEnvelope, ReplayExecutor, ReplayProfile, fixture_descriptor,
        fixture_session,
        journal::codec::{
            BackendBindingOpened, BindingTransition, ForkExactReplay, ForkGroup,
            ForkHistoryCoordinate, ForkHistoryEntry, ForkItemCoordinate, ForkItemOrigin,
            ForkMessagePart, ForkSeed, ForkSource, ForkSourcePoint, InitialForkSeed,
            ReplaySequence, VersionedIdentity,
        },
        provider_private_schema,
    };
    let parent = session();
    let child = fixture_session(2);
    let profile = ReplayProfile::ProviderPrivateLocalPlaintext;
    let strategy = ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient,
        replay_profile: profile,
    };
    let binding = BackendBindingEvidence::new(
        "managed",
        "1",
        BackendIdentity::new("binding/v1", "account"),
        BackendIdentity::new("model/v1", "model"),
        BackendIdentity::new("locator/v1", "parent"),
        strategy,
    );
    let items = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "visible model context".into(),
            refusal: None,
        },
        ModelReplayItem::ProviderPrivateAssistant {
            envelope: ProviderPrivateReplayEnvelope::new(
                provider_private_schema(profile).unwrap(),
                br#"{"reasoning_content":"private-archive-canary","content":"visible model context"}"#.to_vec(),
            )
            .unwrap(),
        },
    ];
    let origins = (0..items.len())
        .map(|index| {
            let coordinate =
                ForkItemCoordinate::new(parent, 1, 1, JournalSequence::new(9), index).unwrap();
            ForkItemOrigin::new(coordinate, coordinate, binding.clone()).unwrap()
        })
        .collect();
    let replay = ForkExactReplay::new(
        ModelReplayContract::new("private-system-canary", vec![]),
        items,
        origins,
        vec![ForkGroup::new(0, 2).unwrap()],
    )
    .unwrap();
    let mut ordinals = HashMap::new();
    let history = history
        .into_iter()
        .map(|record| {
            let source = record.record().session_id().unwrap();
            let coordinate = if let Some(sequence) = record.journal_sequence() {
                ForkHistoryCoordinate::Journal { sequence }
            } else {
                let (activity, part) = match record.record() {
                    JournalRecord::MessageReset(reset) => {
                        (reset.activity(), ForkMessagePart::Reset)
                    },
                    JournalRecord::MessageSegment(segment) => {
                        (segment.activity(), ForkMessagePart::Segment)
                    },
                    JournalRecord::MessageEnded(terminal) => {
                        (terminal.ended().activity(), ForkMessagePart::Ended)
                    },
                    _ => panic!("archive fixture requires a qualified record"),
                };
                let next = ordinals.entry((activity, part)).or_insert(0);
                let ordinal = *next;
                *next += 1;
                ForkHistoryCoordinate::Message {
                    activity,
                    part,
                    ordinal,
                }
            };
            ForkHistoryEntry::new(source, coordinate, record).unwrap()
        })
        .collect();
    let seed = InitialForkSeed::new(
        child,
        parent,
        ForkSource::Anchor(
            ForkSourcePoint::new(
                1,
                1,
                JournalSequence::new(10_001),
                JournalSequence::new(10_000),
                binding,
            )
            .unwrap(),
        ),
        ForkSeed::ExactReplay(replay),
        history,
        16 * 1024 * 1024,
    )
    .unwrap();
    JournalCommit::snapshot_through(
        JournalSequence::new(4),
        vec![
            SequencedJournalRecord::storage(
                ReplaySequence::new(1),
                JournalRecord::SessionDescriptor(fixture_descriptor(child)),
            ),
            SequencedJournalRecord::with_journal_sequence(
                ReplaySequence::new(2),
                JournalSequence::new(1),
                JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id: child }),
            ),
            SequencedJournalRecord::with_journal_sequence(
                ReplaySequence::new(3),
                JournalSequence::new(2),
                JournalRecord::InitialForkSeed(Box::new(seed)),
            ),
            SequencedJournalRecord::with_journal_sequence(
                ReplaySequence::new(4),
                JournalSequence::new(3),
                JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                    1,
                    "managed",
                    "1",
                    VersionedIdentity::new("binding/v1", "account"),
                    VersionedIdentity::new("model/v1", "model"),
                    VersionedIdentity::new("locator/v1", "child"),
                    BindingTransition::initial_fork(JournalSequence::new(2)),
                    strategy,
                )),
            ),
            SequencedJournalRecord::with_journal_sequence(
                ReplaySequence::new(5),
                JournalSequence::new(4),
                JournalRecord::ContextPolicyChanged(
                    ContextPolicyChanged::try_new(
                        1,
                        true,
                        ContextStrategy::PortableSummaryV1Alpha1,
                        85,
                        90,
                        Some(10),
                        Some(65536),
                    )
                    .unwrap(),
                ),
            ),
        ],
    )
}

fn archival_event(sequence: u64, event: AgentEvent) -> SequencedJournalRecord {
    SequencedJournalRecord::new(
        JournalSequence::new(sequence),
        JournalRecord::EventCommitted(event),
    )
}

// child의 seed만으로 원래 source의 첫 등장 순서와 각 source 내부 순서를 복구합니다.
// 마지막 visible record를 exact capture boundary로 오인하지 않으며 private model 본문은 숨깁니다.
#[test]
fn inherited_history_groups_repeated_sources_without_relabeling_or_private_replay() {
    use crate::{
        fixture_session,
        session_repository::{InheritedHistorySource, read_stored_session_continuation},
    };
    let ancestor = fixture_session(3);
    let events = [
        AgentEvent::SessionCreated {
            session_id: session(),
        },
        AgentEvent::SessionCreated {
            session_id: ancestor,
        },
        AgentEvent::TurnStarted {
            turn: activity().turn(),
        },
    ];
    let snapshot = inherited_child_snapshot(vec![
        archival_event(1, events[0].clone()),
        archival_event(1, events[1].clone()),
        archival_event(2, events[2].clone()),
    ]);
    let reader = reader(&[snapshot]);
    let history = read_stored_session(&reader, fixture_session(2)).unwrap();
    let inherited = history.inherited_history().unwrap();
    assert_eq!(inherited.parent_session_id(), session());
    assert_eq!(
        inherited.source(),
        InheritedHistorySource::Anchor {
            record_sequence: JournalSequence::new(10_001),
            journal_boundary: JournalSequence::new(10_000),
        }
    );
    let sections = inherited.sections();
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].source_session_id(), session());
    assert_eq!(sections[1].source_session_id(), ancestor);
    assert_eq!(
        sections[0].last_visible_journal_sequence(),
        Some(JournalSequence::new(2))
    );
    assert_eq!(
        sections[1].last_visible_journal_sequence(),
        Some(JournalSequence::new(1))
    );
    assert_eq!(
        sections[0].records(),
        [
            TranscriptRecord::EventCommitted(events[0].clone()),
            TranscriptRecord::EventCommitted(events[2].clone()),
        ]
    );
    assert_eq!(
        sections[1].records(),
        [TranscriptRecord::EventCommitted(events[1].clone())]
    );
    assert!(matches!(
        history.records(),
        [TranscriptRecord::EventCommitted(AgentEvent::SessionCreated { session_id }),
         TranscriptRecord::ContextPolicyChanged(_)] if *session_id == fixture_session(2)
    ));
    let continuation = read_stored_session_continuation(&reader, fixture_session(2)).unwrap();
    assert_eq!(continuation.inherited_history(), Some(inherited));
    assert_eq!(continuation.target().model_replay().items().len(), 2);
    assert!(!format!("{inherited:?}").contains("private-archive-canary"));
    assert!(!format!("{inherited:?}").contains("private-system-canary"));
    let shared = inherited.clone();
    assert!(std::ptr::eq(shared.sections(), inherited.sections()));
}

// 문서·tool output·usage receipt는 원래 snapshot 그대로 상속 archive에만 남고,
// child의 실행 기록과 request trace 및 usage 집계에는 섞이지 않습니다.
#[test]
fn inherited_history_preserves_typed_snapshots_and_excludes_parent_usage() {
    use crate::{
        ActivityDocument, ActivityId, ActivityKind, ActivityRef, ToolOutput, fixture_session,
        session_repository::SessionUsageProjection,
    };
    let document = ActivityDocument {
        title: "Plan".into(),
        markdown: "# Keep **exact**".into(),
    };
    let output = ToolOutput {
        tool: "read-file".into(),
        server: None,
        arguments: Some(serde_json::json!({"path":"a"})),
        result: Some(serde_json::json!({"content":"unchanged"})),
        content_items: None,
        error: None,
        plain_text: "unchanged".into(),
    };
    let receipt = serde_json::json!({"schema":"yo.model-usage-receipt/v1", "response_id":"parent-response", "round":1,
        "provider":"managed", "account":"account", "model":"model", "connector":"connector", "api_dialect":"dialect",
        "base_url":"https://managed.invalid", "usage":{"input_tokens":17,"output_tokens":2,"total_tokens":19,"reasoning_tokens":null},
        "cache_read_input_tokens":{"availability":"absent","source_profile":"managed.cache-read/v1"}}).to_string();
    let snapshots = [
        document.to_snapshot().unwrap(),
        output.to_snapshot().unwrap(),
        receipt,
    ];
    let mut records = Vec::new();
    for (index, text) in snapshots.iter().enumerate() {
        let activity = ActivityRef::new(
            activity().turn(),
            ActivityId::new((index as u64 + 1).try_into().unwrap()),
        );
        for event in [
            AgentEvent::ActivityStarted {
                activity,
                kind: if index == 1 {
                    ActivityKind::ToolResult
                } else {
                    ActivityKind::ModelWork
                },
            },
            AgentEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text.clone()),
            },
            AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            },
        ] {
            records.push(archival_event(records.len() as u64 + 1, event));
        }
    }
    let history = read_stored_session(
        &reader(&[inherited_child_snapshot(records)]),
        fixture_session(2),
    )
    .unwrap();
    let inherited = history.inherited_history().unwrap().sections()[0].records();
    let projected = inherited
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(projected, snapshots.iter().collect::<Vec<_>>());
    assert_eq!(
        ActivityDocument::from_snapshot(projected[0]),
        Some(document)
    );
    assert_eq!(ToolOutput::from_snapshot(projected[1]), Some(output));
    assert_eq!(
        SessionUsageProjection::from_records(inherited)
            .unwrap()
            .receipts()
            .len(),
        1
    );
    assert!(history.session_usage().unwrap().receipts().is_empty());
    assert_eq!(history.request_trace().len(), 1);
    assert_eq!(history.records().len(), 2);
}

// 세그먼트마다 다른 event 경계가 끼어도 누적 text를 매번 복제하지 않고 terminal의 최종
// snapshot 한 개만 방출하여 상속 archive의 표현 크기를 입력 바이트에 선형으로 유지합니다.
#[test]
fn inherited_segmented_message_emits_one_final_snapshot_across_many_boundaries() {
    use crate::{
        ActivityDocument, ActivityId, ActivityKind, ActivityRef, fixture_session,
        journal::codec::ReplaySequence,
    };
    let progress = ActivityRef::new(activity().turn(), ActivityId::new(4.try_into().unwrap()));
    let mut records = vec![
        started(1),
        archival_event(
            2,
            AgentEvent::ActivityStarted {
                activity: progress,
                kind: ActivityKind::ToolCall,
            },
        ),
    ];
    let document = ActivityDocument {
        title: "Large plan".into(),
        markdown: "x".repeat(2 * 1024 * 1024),
    };
    let text = document.to_snapshot().unwrap();
    let segments = text.as_bytes().chunks(4096).collect::<Vec<_>>();
    let segment_count = u64::try_from(segments.len()).unwrap();
    for (index, segment) in segments.iter().enumerate() {
        let index = u64::try_from(index).unwrap();
        records.push(SequencedJournalRecord::storage(
            ReplaySequence::new(index + 1),
            JournalRecord::MessageSegment(MessageSegment::new(
                activity(),
                MessageStream::Agent,
                index + 1,
                std::str::from_utf8(segment).unwrap().to_owned(),
            )),
        ));
        records.push(archival_event(
            index + 3,
            AgentEvent::ActivityUpdated {
                activity: progress,
                update: ActivityUpdate::TextSnapshot(format!("progress {index}")),
            },
        ));
    }
    records.push(archival_event(
        segment_count + 3,
        AgentEvent::ActivityFinished {
            activity: progress,
            outcome: ActivityOutcome::Completed,
        },
    ));
    records.push(SequencedJournalRecord::storage(
        ReplaySequence::new(segment_count + 2),
        JournalRecord::MessageEnded(MessageTerminal::new(
            None,
            MessageEnded::new(
                activity(),
                MessageStream::Agent,
                MessageOutcome::Completed,
                segment_count,
                u64::try_from(text.len()).unwrap(),
            ),
        )),
    ));
    records.push(finished(segment_count + 4, ActivityOutcome::Completed));
    let history = read_stored_session(
        &reader(&[inherited_child_snapshot(records)]),
        fixture_session(2),
    )
    .unwrap();
    let records = history.inherited_history().unwrap().sections()[0].records();
    let snapshots = records
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
                activity: owner,
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) if *owner == activity() => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0], &text);
    assert_eq!(
        ActivityDocument::from_snapshot(snapshots[0]),
        Some(document)
    );
    assert_eq!(records.len(), segments.len() + 5);
    assert!(matches!(
        records.last(),
        Some(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityFinished {
                outcome: ActivityOutcome::Completed,
                ..
            }
        ))
    ));
}

// wire가 허용한 source-qualified event라도 표시용 activity의 시작이 없으면 부분 archive를
// 내보내지 않고 저장 기록 읽기와 continuation 준비 양쪽에서 typed 오류를 반환합니다.
#[test]
fn inherited_projection_failure_rejects_history_and_continuation() {
    use crate::{
        fixture_session,
        session_repository::{StoredSessionReadError, read_stored_session_continuation},
    };
    let reader = reader(&[inherited_child_snapshot(vec![finished(
        1,
        ActivityOutcome::Completed,
    )])]);
    assert!(matches!(
        read_stored_session(&reader, fixture_session(2)),
        Err(StoredSessionReadError::Invalid { detail }) if detail.contains("finished before it started")
    ));
    assert!(
        read_stored_session_continuation(&reader, fixture_session(2))
            .unwrap_err()
            .to_string()
            .contains("finished before it started")
    );
}

// 여러 physical segment와 그 앞의 오래된 revision은 저장 최적화 경계일 뿐이므로,
// replacement revision의 최종 text snapshot 하나와 종료 event만 frontend에 전달한다.
#[test]
fn coalesces_segments_and_superseded_revisions() {
    let commit = JournalCommit::snapshot(vec![
        SequencedJournalRecord::new(
            JournalSequence::new(1),
            JournalRecord::SessionDescriptor(crate::fixture_descriptor(session())),
        ),
        started(2),
        SequencedJournalRecord::new(
            JournalSequence::new(3),
            JournalRecord::MessageSegment(MessageSegment::new(
                activity(),
                MessageStream::Agent,
                1,
                "old".to_owned(),
            )),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(4),
            JournalRecord::MessageReset(MessageReset::new(activity(), MessageStream::Agent, 2)),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(5),
            JournalRecord::MessageSegment(MessageSegment::for_revision(
                activity(),
                MessageStream::Agent,
                2,
                1,
                "new ".to_owned(),
            )),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(6),
            JournalRecord::MessageEnded(MessageTerminal::new(
                Some(MessageSegment::for_revision(
                    activity(),
                    MessageStream::Agent,
                    2,
                    2,
                    "answer".to_owned(),
                )),
                MessageEnded::for_revision(
                    activity(),
                    MessageStream::Agent,
                    2,
                    MessageOutcome::Completed,
                    2,
                    10,
                ),
            )),
        ),
        finished(7, ActivityOutcome::Completed),
    ]);

    let history = read_stored_session(&reader(&[commit]), session()).unwrap();
    let updates = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { update, .. }) => {
                Some(update)
            },
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        updates,
        [&ActivityUpdate::TextSnapshot("new answer".to_owned())]
    );
    assert!(history.discovery_consistent());
    assert_eq!(
        history.discovery_validation(),
        StoredDiscoveryValidation::Consistent
    );
    assert_eq!(history.recovery(), StoredSessionRecovery::NotRequired);
    assert_eq!(history.continuity(), StoredSessionContinuity::NotObservable);
}

// 이전 revision에 text가 있더라도 다음 revision의 zero-byte terminal은 "빈 답변"이라는
// authoritative snapshot이므로, 저장된 Chat에 오래된 text를 남기지 않습니다.
#[test]
fn empty_terminal_revision_clears_superseded_text() {
    let commit = JournalCommit::snapshot(vec![
        SequencedJournalRecord::new(
            JournalSequence::new(1),
            JournalRecord::SessionDescriptor(crate::fixture_descriptor(session())),
        ),
        started(2),
        SequencedJournalRecord::new(
            JournalSequence::new(3),
            JournalRecord::MessageSegment(MessageSegment::new(
                activity(),
                MessageStream::Agent,
                1,
                "superseded".to_owned(),
            )),
        ),
        SequencedJournalRecord::new(
            JournalSequence::new(4),
            JournalRecord::MessageEnded(MessageTerminal::new(
                None,
                MessageEnded::for_revision(
                    activity(),
                    MessageStream::Agent,
                    2,
                    MessageOutcome::Completed,
                    0,
                    0,
                ),
            )),
        ),
        finished(5, ActivityOutcome::Completed),
    ]);

    let history = read_stored_session(&reader(&[commit]), session()).unwrap();
    let updates = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { update, .. }) => {
                Some(update)
            },
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(updates, [&ActivityUpdate::TextSnapshot(String::new())]);
}

// crash가 열린 message의 terminal을 쓰기 전에 멈추면 recovery seal을 버리지 않고
// 마지막 text 뒤에 interrupted ActivityFinished를 붙여 완료된 대화처럼 보이지 않게 한다.
#[test]
fn projects_an_open_message_as_explicitly_interrupted() {
    let descriptor = JournalCommit::descriptor(crate::fixture_descriptor(session()));
    let open = JournalCommit::incremental(vec![
        started(2),
        SequencedJournalRecord::new(
            JournalSequence::new(3),
            JournalRecord::MessageSegment(MessageSegment::new(
                activity(),
                MessageStream::Agent,
                1,
                "partial".to_owned(),
            )),
        ),
    ]);

    let history = read_stored_session(&reader(&[descriptor, open]), session()).unwrap();

    assert_eq!(history.recovery(), StoredSessionRecovery::Interrupted);
    assert!(matches!(
        history.records().last(),
        Some(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityFinished {
                outcome: ActivityOutcome::Interrupted,
                ..
            }
        ))
    ));
}

// message terminal과 semantic ActivityFinished의 outcome이 다르면 손상된 history를 일부
// 출력하지 않고 두 authority가 충돌한다는 복구 오류로 거부한다.
#[test]
fn rejects_conflicting_message_and_activity_outcomes() {
    let commit = JournalCommit::snapshot(vec![
        SequencedJournalRecord::new(
            JournalSequence::new(1),
            JournalRecord::SessionDescriptor(crate::fixture_descriptor(session())),
        ),
        started(2),
        SequencedJournalRecord::new(
            JournalSequence::new(3),
            JournalRecord::MessageEnded(MessageTerminal::new(
                None,
                MessageEnded::new(
                    activity(),
                    MessageStream::Agent,
                    MessageOutcome::Completed,
                    0,
                    0,
                ),
            )),
        ),
        finished(4, ActivityOutcome::Interrupted),
    ]);

    let error = read_stored_session(&reader(&[commit]), session()).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("conflicting message and activity outcomes")
    );
}
