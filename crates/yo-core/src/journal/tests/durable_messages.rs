use super::*;

// 백엔드 adapter가 semantic ModelWork 본문으로 공개한 text는 live 화면에서만 보였다가
// 사라지지 않고 segment와 terminal seal로 저장되어 재개 가능한 Transcript가 되어야 한다.
#[test]
fn model_work_text_is_persisted_as_a_sealed_message() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_events(&[AgentEvent::ActivityStarted {
        activity,
        kind: ActivityKind::ModelWork,
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity,
        update: ActivityUpdate::TextDelta("inspect the journal".to_owned()),
    }]);
    journal.append_events(&[AgentEvent::ActivityFinished {
        activity,
        outcome: ActivityOutcome::Completed,
    }]);

    let records = observed
        .lock()
        .unwrap()
        .entries
        .iter()
        .flat_map(|entry| decode(entry.record().payload()).unwrap().records().to_vec())
        .collect::<Vec<_>>();
    let terminal = records
        .iter()
        .find_map(|entry| match entry.record() {
            super::super::codec::JournalRecord::MessageEnded(terminal) => Some(terminal),
            _ => None,
        })
        .expect("the observable ModelWork text is durably sealed");
    assert_eq!(
        terminal.final_segment().unwrap().text(),
        "inspect the journal"
    );
    assert_eq!(terminal.ended().segment_count(), 1);
    assert_eq!(terminal.ended().utf8_bytes(), 19);
}

// live TextDelta와 권위 있는 TextSnapshot은 frontend Journal에는 즉시 남지만 저장소에는
// raw update로 기록하지 않고, Activity 종료 시 최종 revision과 seal을 한 envelope로 남긴다.
#[test]
fn persistent_journal_stores_the_final_message_revision_instead_of_raw_updates() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );
    journal.append_events(&[AgentEvent::ActivityStarted {
        activity,
        kind: ActivityKind::AgentMessage,
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity,
        update: ActivityUpdate::TextDelta("draft".to_owned()),
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity,
        update: ActivityUpdate::TextSnapshot("final answer".to_owned()),
    }]);
    journal.append_events(&[AgentEvent::ActivityFinished {
        activity,
        outcome: ActivityOutcome::Completed,
    }]);

    let state = observed.lock().unwrap();
    let records = state
        .entries
        .iter()
        .flat_map(|entry| decode(entry.record().payload()).unwrap().records().to_vec())
        .collect::<Vec<_>>();
    assert!(records.iter().all(|entry| {
        !matches!(
            entry.record(),
            super::super::codec::JournalRecord::EventCommitted(AgentEvent::ActivityUpdated { .. })
        )
    }));
    let terminal = records
        .iter()
        .find_map(|entry| match entry.record() {
            super::super::codec::JournalRecord::MessageEnded(terminal) => Some(terminal),
            _ => None,
        })
        .expect("the final message revision is sealed");
    assert_eq!(terminal.ended().revision(), 2);
    assert_eq!(terminal.final_segment().unwrap().text(), "final answer");
}

// 짧은 text가 아직 메모리 buffer에 있을 때 다른 Activity가 시작되면 text segment를 먼저
// 강제 저장하고 그 다음 사건을 기록해야 한다. 이때 durable cutoff는 segment 개수가 아니라
// frontend가 보는 마지막 semantic JournalSequence와 정확히 같아야 한다.
#[test]
fn forces_buffered_text_before_a_non_text_boundary_with_one_semantic_cutoff() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let message = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let tool = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(2).unwrap()));
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );
    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: message,
        kind: ActivityKind::AgentMessage,
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextDelta("before tool".to_owned()),
    }]);
    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: tool,
        kind: ActivityKind::ToolCall,
    }]);

    let state = observed.lock().unwrap();
    let last = state
        .entries
        .last()
        .expect("the boundary commit is durable");
    let commit = decode(last.record().payload()).unwrap();
    assert!(matches!(
        commit.records()[0].record(),
        super::super::codec::JournalRecord::MessageSegment(segment)
            if segment.text() == "before tool"
    ));
    assert!(matches!(
        commit.records()[1].record(),
        super::super::codec::JournalRecord::EventCommitted(
            AgentEvent::ActivityStarted { activity, .. }
        ) if *activity == tool
    ));
    assert_eq!(
        last.record().journal_cutoff(),
        journal.transcript_reader().head_sequence()
    );
}

// size bound에서 이미 segment가 durable해진 message 뒤에 다른 사건이 와도 전체 commit을
// 다시 재생할 수 있어야 한다. 이는 동시 Activity가 있는 실제 provider 흐름이 영구적인
// integrity gap으로 굳지 않는지를 검증한다.
#[test]
fn recovers_after_a_size_forced_segment_and_an_unrelated_event() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let message = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let tool = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(2).unwrap()));
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );
    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: message,
        kind: ActivityKind::AgentMessage,
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextDelta("x".repeat(16 * 1024)),
    }]);
    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: tool,
        kind: ActivityKind::ToolCall,
    }]);

    let commits = observed
        .lock()
        .unwrap()
        .entries
        .iter()
        .map(|entry| decode(entry.record().payload()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).expect("the interleaved durable history recovers");
    assert!(recovered.recovery_commit().is_some());
}

// text update가 전혀 없는 message도 ActivityFinished가 관찰되면 zero-byte MessageEnded를
// 남겨야 완료와 crash 중단을 구분할 수 있다. 다시 읽었을 때 recovery seal이 필요 없어야
// 정상 종료가 durable authority로 확정되었음을 증명한다.
#[test]
fn empty_message_is_sealed_with_a_zero_byte_terminal() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let message = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: message,
        kind: ActivityKind::AgentMessage,
    }]);
    journal.append_events(&[AgentEvent::ActivityFinished {
        activity: message,
        outcome: ActivityOutcome::Completed,
    }]);

    let commits = observed
        .lock()
        .unwrap()
        .entries
        .iter()
        .map(|entry| decode(entry.record().payload()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).unwrap();
    assert!(recovered.recovery_commit().is_none());
    let terminal = recovered
        .records()
        .iter()
        .find_map(|entry| match entry.record() {
            super::super::codec::JournalRecord::MessageEnded(terminal) => Some(terminal.ended()),
            _ => None,
        })
        .unwrap();
    assert_eq!(terminal.segment_count(), 0);
    assert_eq!(terminal.utf8_bytes(), 0);
}

// revision 1 text가 이미 durable한 뒤 권위 있는 empty snapshot이 오면 그 옛 본문을
// 되살리지 않는다. revision 2의 zero-byte seal을 저장하고 integrity gap 없이 복구한다.
#[test]
fn durable_message_can_be_authoritatively_replaced_with_empty_text() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let message = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: message,
        kind: ActivityKind::AgentMessage,
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextDelta("x".repeat(16 * 1024)),
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextSnapshot(String::new()),
    }]);
    journal.append_events(&[AgentEvent::ActivityFinished {
        activity: message,
        outcome: ActivityOutcome::Completed,
    }]);

    assert!(matches!(
        journal.transcript_reader().durability(),
        JournalDurability::Durable { .. }
    ));
    let commits = observed
        .lock()
        .unwrap()
        .entries
        .iter()
        .map(|entry| decode(entry.record().payload()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).unwrap();
    let terminal = recovered
        .records()
        .iter()
        .find_map(|entry| match entry.record() {
            super::super::codec::JournalRecord::MessageEnded(terminal) => Some(terminal.ended()),
            _ => None,
        })
        .unwrap();

    assert_eq!(terminal.revision(), 2);
    assert_eq!(terminal.segment_count(), 0);
    assert_eq!(terminal.utf8_bytes(), 0);
    assert!(recovered.recovery_commit().is_none());
}

// 이미 저장된 본문을 empty snapshot으로 교체한 뒤 unrelated event가 경계를 만들면,
// 종료 전 crash가 나도 옛 본문이 되살아나지 않도록 empty revision을 먼저 저장해야 한다.
#[test]
fn empty_replacement_is_durable_before_a_later_non_text_event() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let message = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let tool = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(2).unwrap()));
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: message,
        kind: ActivityKind::AgentMessage,
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextDelta("x".repeat(16 * 1024)),
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextSnapshot(String::new()),
    }]);
    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: tool,
        kind: ActivityKind::ToolCall,
    }]);

    let commits = observed
        .lock()
        .unwrap()
        .entries
        .iter()
        .map(|entry| decode(entry.record().payload()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).unwrap();
    assert!(recovered.records().iter().any(|entry| {
        matches!(
            entry.record(),
            super::super::codec::JournalRecord::MessageReset(reset)
                if reset.activity() == message && reset.revision() == 2
        )
    }));
    let seal = recovered
        .recovery_commit()
        .expect("the still-open activities receive recovery seals");
    assert!(seal.records().iter().any(|entry| {
        matches!(
            entry.record(),
            super::super::codec::JournalRecord::MessageEnded(terminal)
                if terminal.ended().activity() == message
                    && terminal.ended().revision() == 2
                    && terminal.ended().utf8_bytes() == 0
        )
    }));
}

// 명시적 도구 출력 profile은 기존 message segment·seal로 저장되고 복구 후 원본 JSON과 바이트를
// 보존한다.
#[test]
fn structured_tool_output_survives_durable_message_recovery() {
    use serde_json::json;

    use crate::{ToolOutput, journal::codec::JournalRecord};

    let output = ToolOutput {
        tool: "capture".to_owned(),
        server: Some("browser".to_owned()),
        arguments: Some(json!({"target":"한글"})),
        result: Some(
            json!({"content":[{"type":"image","mimeType":"image/png","data":"abcd".repeat(20000)}],"_meta":{"original":true}}),
        ),
        content_items: None,
        error: None,
        plain_text: "Captured image".to_owned(),
    };
    let retained = ToolOutput {
        tool: "run_command".to_owned(),
        server: None,
        arguments: None,
        result: Some(
            json!({"content":[{"type":"text","text":"small model result"}],"truncated":true,"retainedOutput":{"truncated":false}}),
        ),
        content_items: None,
        error: None,
        plain_text: format!(
            "{}retained final row",
            "retained middle row\n".repeat(20000)
        ),
    };
    for output in [output, retained] {
        let wire = output.to_snapshot().unwrap();
        let message = ActivityRef::new(
            turn(session(1), 1),
            ActivityId::new(NonZeroU64::new(1).unwrap()),
        );
        let repository = SharedRepository::default();
        let observed = Arc::clone(&repository.state);
        let mut journal = SessionJournal::with_repository(Box::new(repository));
        journal.append_events(&[AgentEvent::ActivityStarted {
            activity: message,
            kind: ActivityKind::ToolCall,
        }]);
        journal.append_events(&[AgentEvent::ActivityUpdated {
            activity: message,
            update: ActivityUpdate::TextSnapshot(wire.clone()),
        }]);
        journal.append_events(&[AgentEvent::ActivityFinished {
            activity: message,
            outcome: ActivityOutcome::Completed,
        }]);
        let commits = observed
            .lock()
            .unwrap()
            .entries
            .iter()
            .map(|entry| decode(entry.record().payload()).unwrap())
            .collect::<Vec<_>>();
        let recovered = recover(&commits).unwrap();
        assert!(recovered.recovery_commit().is_none());
        let text = recovered
            .records()
            .iter()
            .filter_map(|entry| match entry.record() {
                JournalRecord::MessageSegment(segment) => Some(segment.text()),
                JournalRecord::MessageEnded(terminal) => {
                    terminal.final_segment().map(|segment| segment.text())
                },
                _ => None,
            })
            .collect::<String>();
        assert_eq!(text.len(), wire.len());
        assert!(text == wire, "recovered profile bytes changed");
        assert_eq!(ToolOutput::from_snapshot(&text), Some(output));
    }
}

// 미지 profile·필드·빈 도구 이름과 첫 초과 바이트는 일반 텍스트로 남고 타입으로 오인하지 않는다.
#[test]
fn structured_tool_output_profile_admission_is_exact_and_bounded() {
    use crate::ToolOutput;

    let mut output = ToolOutput {
        tool: "read".to_owned(),
        server: None,
        arguments: None,
        result: None,
        content_items: None,
        error: None,
        plain_text: String::new(),
    };
    let wire = output.to_snapshot().unwrap();
    assert!(
        ToolOutput::from_snapshot(&wire.replace(ToolOutput::SCHEMA, "yo.tool-output/v2")).is_none()
    );
    assert!(ToolOutput::from_snapshot(&wire.replacen("{", "{\"unknown\":true,", 1)).is_none());
    assert!(
        ToolOutput::from_snapshot(&wire.replace("\"tool\":\"read\"", "\"tool\":\"\"")).is_none()
    );
    output.plain_text = "x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - wire.len());
    let boundary = output.to_snapshot().unwrap();
    assert_eq!(boundary.len(), ToolOutput::MAX_SNAPSHOT_BYTES);
    assert_eq!(ToolOutput::from_snapshot(&boundary), Some(output.clone()));
    output.plain_text.push('x');
    assert!(output.to_snapshot().is_none());
    assert!(ToolOutput::from_snapshot(&(boundary + " ")).is_none());
}

// 안내 profile의 severity와 원문은 roundtrip되며 잘못된 schema·수준·추가 필드는 거절한다.
#[test]
fn activity_notice_profile_is_explicit_and_roundtrips() {
    use crate::{ActivityNotice, NoticeLevel};

    let notice = ActivityNotice {
        title: "Retry announced".to_owned(),
        message: "원문\nmessage".to_owned(),
        level: NoticeLevel::Warning,
    };
    let wire = notice.to_snapshot().unwrap();
    assert_eq!(ActivityNotice::from_snapshot(&wire), Some(notice));
    for invalid in [
        wire.replace(ActivityNotice::SCHEMA, "yo.activity-notice/v2"),
        wire.replace("\"warning\"", "\"fatal\""),
        wire.replacen("{", "{\"extra\":true,", 1),
        wire.replace("Retry announced", ""),
    ] {
        assert!(ActivityNotice::from_snapshot(&invalid).is_none());
    }
}

// 요약 profile은 종류·수치·필드를 검증하고 원문과 정확한 크기 상한을 보존한다.
#[test]
fn activity_summary_profile_rejects_unknown_fields_and_excess_bytes() {
    use crate::{ActivitySummary, SummaryKind, ToolOutput};

    let mut summary = ActivitySummary {
        kind: SummaryKind::Compaction,
        summary: "원문 **summary**".to_owned(),
        tokens_before: Some(12345),
    };
    let wire = summary.to_snapshot().unwrap();
    assert_eq!(ActivitySummary::from_snapshot(&wire), Some(summary.clone()));
    for invalid in [
        wire.replace(ActivitySummary::SCHEMA, "yo.activity-summary/v2"),
        wire.replace("compaction", "unknown"),
        wire.replace("compaction", "branch"),
        wire.replace("compaction", "reasoning"),
        wire.replace("12345", "-1"),
        wire.replacen("{", "{\"extra\":true,", 1),
    ] {
        assert!(ActivitySummary::from_snapshot(&invalid).is_none());
    }
    summary.kind = SummaryKind::Branch;
    assert!(summary.to_snapshot().is_none());
    summary.tokens_before = None;
    summary.kind = SummaryKind::Reasoning;
    assert_eq!(
        ActivitySummary::from_snapshot(&summary.to_snapshot().unwrap()),
        Some(summary.clone())
    );
    summary.kind = SummaryKind::Branch;
    summary.summary.clear();
    assert_eq!(
        ActivitySummary::from_snapshot(&summary.to_snapshot().unwrap()),
        Some(summary.clone())
    );
    let overhead = summary.to_snapshot().unwrap().len();
    summary.summary = "x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - overhead);
    let boundary = summary.to_snapshot().unwrap();
    assert_eq!(boundary.len(), ToolOutput::MAX_SNAPSHOT_BYTES);
    assert_eq!(
        ActivitySummary::from_snapshot(&boundary),
        Some(summary.clone())
    );
    summary.summary.push('x');
    assert!(summary.to_snapshot().is_none());
    assert!(ActivitySummary::from_snapshot(&(boundary + " ")).is_none());
}

// 계획 profile은 원문·빈 계획을 보존하고 미지 상태·필드·과도한 snapshot을 타입으로 오인하지 않는다.
#[test]
fn activity_plan_profile_preserves_status_and_rejects_invalid_payloads() {
    use crate::{ActivityPlan, PlanStep, PlanStepStatus, ToolOutput};
    let plan = ActivityPlan {
        explanation: Some("변경 이유\n**literal**".to_owned()),
        steps: vec![PlanStep {
            text: "검증".to_owned(),
            status: PlanStepStatus::InProgress,
        }],
    };
    let wire = plan.to_snapshot().unwrap();
    assert_eq!(ActivityPlan::from_snapshot(&wire), Some(plan));
    for invalid in [
        wire.replace(ActivityPlan::SCHEMA, "yo.activity-plan/v2"),
        wire.replace("in_progress", "unknown"),
        wire.replacen("{", "{\"extra\":true,", 1),
    ] {
        assert!(ActivityPlan::from_snapshot(&invalid).is_none());
    }
    let empty = ActivityPlan {
        explanation: None,
        steps: Vec::new(),
    };
    assert_eq!(
        ActivityPlan::from_snapshot(&empty.to_snapshot().unwrap()),
        Some(empty)
    );
    assert!(ActivityPlan::from_snapshot(&" ".repeat(ToolOutput::MAX_SNAPSHOT_BYTES + 1)).is_none());
}

// 문서 profile은 제목과 Markdown을 보존하고 빈 제목·미지 필드·다른 schema를 거절한다.
#[test]
fn activity_document_profile_preserves_source_and_exact_identity() {
    use crate::ActivityDocument;
    let document = ActivityDocument {
        title: "Proposed plan".to_owned(),
        markdown: "## 계획\n\n```rust\nuse std::fmt;\n```".to_owned(),
    };
    let wire = document.to_snapshot().unwrap();
    assert_eq!(ActivityDocument::from_snapshot(&wire), Some(document));
    for invalid in [
        wire.replace(ActivityDocument::SCHEMA, "yo.activity-document/v2"),
        wire.replace("Proposed plan", ""),
        wire.replacen("{", "{\"extra\":true,", 1),
    ] {
        assert!(ActivityDocument::from_snapshot(&invalid).is_none());
    }
    assert!(
        ActivityDocument {
            title: String::new(),
            markdown: String::new()
        }
        .to_snapshot()
        .is_none()
    );
}

// 질문 프로필은 선택지 64개와 정확한 스냅샷 바이트 한도까지만 허용하고 다른 스키마를 거부한다.
#[test]
fn question_presentation_profile_preserves_choices_and_exact_bounds() {
    use serde_json::json;

    use crate::{ActivityQuestion, QuestionChoice, ToolOutput};
    let choice = QuestionChoice {
        label: "Runtime".into(),
        description: "Events".into(),
    };
    for count in [64, 65] {
        let question = ActivityQuestion {
            allow_notes: false,
            previous_question: false,
            draft: None,
            draft_choice: None,
            plain_text: "Question".into(),
            choices: vec![choice.clone(); count],
        };
        assert_eq!(question.to_snapshot().is_some(), count == 64);
        let unchecked = json!({"schema": ActivityQuestion::SCHEMA, "output": question}).to_string();
        assert_eq!(
            ActivityQuestion::from_snapshot(&unchecked).is_some(),
            count == 64
        );
    }
    let mut question = ActivityQuestion {
        allow_notes: false,
        previous_question: false,
        draft: None,
        draft_choice: None,
        plain_text: "q".into(),
        choices: vec![choice],
    };
    let wire = question.to_snapshot().unwrap();
    assert_eq!(
        ActivityQuestion::from_snapshot(&wire),
        Some(question.clone())
    );
    assert!(
        ActivityQuestion::from_snapshot(
            &wire.replace(ActivityQuestion::SCHEMA, "yo.activity-question/v2")
        )
        .is_none()
    );
    let mut legacy = json!({"schema": ActivityQuestion::SCHEMA, "output": question});
    legacy["output"]
        .as_object_mut()
        .unwrap()
        .remove("allow_notes");
    assert!(
        !ActivityQuestion::from_snapshot(&legacy.to_string())
            .unwrap()
            .allow_notes
    );
    legacy["output"]["allow_notes"] = json!(true);
    assert!(
        ActivityQuestion::from_snapshot(&legacy.to_string())
            .unwrap()
            .allow_notes
    );
    legacy["output"]["allow_notes"] = json!("true");
    assert!(ActivityQuestion::from_snapshot(&legacy.to_string()).is_none());
    let overhead = wire.len() - 1;
    question.plain_text = "x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - overhead);
    let wire = question.to_snapshot().unwrap();
    assert_eq!(wire.len(), ToolOutput::MAX_SNAPSHOT_BYTES);
    assert_eq!(
        ActivityQuestion::from_snapshot(&wire),
        Some(question.clone())
    );
    question.plain_text.push('x');
    assert!(question.to_snapshot().is_none());
    assert!(ActivityQuestion::from_snapshot(&format!("{wire} ")).is_none());
}

// 이전 질문 기능은 이전 프로필에서 기본 비활성이며 복원 선택은 실제 선택지·메모 지원과 일치해야
// 한다.
#[test]
fn question_navigation_profile_validates_restored_choice_and_legacy_defaults() {
    use serde_json::json;

    use crate::ActivityQuestion;
    let legacy = json!({"schema":ActivityQuestion::SCHEMA,"output":{
        "plain_text":"Question", "choices":[{"label":"A","description":"a"}]
    }});
    let original = ActivityQuestion::from_snapshot(&legacy.to_string()).unwrap();
    assert!(!original.previous_question);
    assert!(original.draft.is_none());
    assert!(original.draft_choice.is_none());
    for (choice, notes, draft, valid) in [
        (None, false, None, true),
        (None, false, Some("/exit\n한글"), true),
        (Some(1), true, Some(""), true),
        (Some(0), true, Some(""), false),
        (Some(2), true, Some(""), false),
        (Some(1), false, Some(""), false),
        (Some(1), true, None, false),
    ] {
        let mut profile = original.clone();
        profile.previous_question = true;
        profile.draft_choice = choice;
        profile.allow_notes = notes;
        profile.draft = draft.map(str::to_owned);
        assert_eq!(profile.to_snapshot().is_some(), valid);
        let unchecked = json!({"schema":ActivityQuestion::SCHEMA,"output":profile}).to_string();
        assert_eq!(ActivityQuestion::from_snapshot(&unchecked).is_some(), valid);
        if valid {
            assert_eq!(ActivityQuestion::from_snapshot(&unchecked), Some(profile));
        }
    }
}

// 추론 원문 profile은 공개 요약과 구분되고 임의 content 및 정확한 크기 경계를 보존한다.
#[test]
fn reasoning_profile_preserves_content_and_enforces_boundary() {
    use crate::{ActivityReasoning, ToolOutput};
    for content in [
        serde_json::json!("한글\nreasoning"),
        serde_json::json!({"type":"future", "data":[1,null]}),
    ] {
        let reasoning = ActivityReasoning { content };
        let wire = reasoning.to_snapshot().unwrap();
        assert_eq!(ActivityReasoning::from_snapshot(&wire), Some(reasoning));
        assert!(
            ActivityReasoning::from_snapshot(
                &wire.replace(ActivityReasoning::SCHEMA, "yo.activity-summary/v1")
            )
            .is_none()
        );
        assert!(
            ActivityReasoning::from_snapshot(
                &wire.replace("\"content\":", "\"extra\":0,\"content\":")
            )
            .is_none()
        );
    }
    let mut reasoning = ActivityReasoning {
        content: serde_json::json!(""),
    };
    let overhead = reasoning.to_snapshot().unwrap().len();
    reasoning.content = serde_json::json!("x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - overhead));
    let wire = reasoning.to_snapshot().unwrap();
    assert_eq!(wire.len(), ToolOutput::MAX_SNAPSHOT_BYTES);
    assert_eq!(
        ActivityReasoning::from_snapshot(&wire),
        Some(reasoning.clone())
    );
    reasoning.content = serde_json::json!(format!("{}x", reasoning.content.as_str().unwrap()));
    assert!(reasoning.to_snapshot().is_none());
    assert!(ActivityReasoning::from_snapshot(&(wire + " ")).is_none());
}

// 실제 저장 경계에서 승인 뒤 연속 질문의 선택·빈 notes·한글 응답을 처리해도
// Integrity gap 없이 마지막 턴까지 보존하고, 재복구 시 추가 seal이 필요 없어야 합니다.
#[test]
fn approval_then_interview_responses_remain_durable_through_completion() {
    use crate::{
        ActivityQuestion, ActivityRequestRef, ActivityResponse, ApprovalDecision, QuestionChoice,
        RequestId, journal::codec::JournalRecord,
    };

    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));
    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );
    let question = ActivityQuestion {
        plain_text: "Question 1 of 2\nWhich area?".into(),
        choices: vec![
            QuestionChoice {
                label: "UI".into(),
                description: "Layout".into(),
            },
            QuestionChoice {
                label: "Runtime".into(),
                description: "Events".into(),
            },
        ],
        allow_notes: true,
        previous_question: false,
        draft: None,
        draft_choice: None,
    }
    .to_snapshot()
    .unwrap();
    let responses = [
        ActivityResponse::Approval(ApprovalDecision::Approved),
        ActivityResponse::QuestionAnswer {
            choice: 2,
            notes: UserInput::from(""),
        },
        ActivityResponse::UserInput(UserInput::from("Keep 한글 intact")),
    ];
    for (index, response) in responses.iter().enumerate() {
        let number = NonZeroU64::new(index as u64 + 1).unwrap();
        let activity = ActivityRef::new(active_turn, ActivityId::new(number));
        let request_id = RequestId::new(number);
        journal.append_events(&[AgentEvent::ActivityStarted {
            activity,
            kind: if index == 0 {
                ActivityKind::ApprovalRequest { request_id }
            } else {
                ActivityKind::UserInputRequest { request_id }
            },
        }]);
        journal.append_events(&[AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(match index {
                0 => "Command approval: offline-noop".into(),
                1 => question.clone(),
                _ => "Question 2 of 2\nWhat must remain intact?".into(),
            }),
        }]);
        journal.append_committed_command(
            AgentCommand::RespondToActivity {
                request: ActivityRequestRef::new(activity, request_id),
                response: response.clone(),
            },
            &[],
        );
        assert!(
            matches!(
                journal.transcript_reader().durability(),
                JournalDurability::Durable { .. }
            ),
            "response {index} lost durability"
        );
        journal.append_events(&[AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        }]);
    }
    let finished = AgentEvent::TurnFinished {
        turn: active_turn,
        outcome: TurnOutcome::Completed,
    };
    journal.append_events(std::slice::from_ref(&finished));
    let commits = observed
        .lock()
        .unwrap()
        .entries
        .iter()
        .map(|entry| decode(entry.record().payload()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).unwrap();
    assert!(recovered.recovery_commit().is_none());
    let persisted = recovered
        .records()
        .iter()
        .filter_map(|entry| match entry.record() {
            JournalRecord::CommandCommitted(command) => match command.command() {
                AgentCommand::RespondToActivity { response, .. } => Some(response.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(persisted, responses);
    assert!(recovered.records().iter().any(
        |entry| matches!(entry.record(), JournalRecord::EventCommitted(event) if event == &finished)
    ));
}
