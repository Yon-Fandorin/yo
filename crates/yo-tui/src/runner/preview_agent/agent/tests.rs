use super::*;

// 인터뷰는 첫 답변 뒤 새 요청을 발행하고 두 번째 답변 뒤에만 Turn을 끝낸다.
#[test]
fn interview_preserves_both_answers_and_correlations() {
    let mut agent = TestAgent::new();
    agent
        .dispatch(AgentAction::submit("interview").unwrap())
        .unwrap();
    let Some(PreviewRequest::Interview { request: first, .. }) = agent.request else {
        panic!("first question missing")
    };
    agent
        .dispatch(AgentAction::RespondToUserInput {
            request: first,
            input: "reading".into(),
        })
        .unwrap();
    let Some(PreviewRequest::Interview {
        request: second, ..
    }) = agent.request
    else {
        panic!("second question missing")
    };
    assert_ne!(first, second);
    agent
        .dispatch(AgentAction::RespondToUserInput {
            request: first,
            input: "stale".into(),
        })
        .unwrap();
    assert!(
        matches!(agent.request, Some(PreviewRequest::Interview { request, .. }) if request == second)
    );
    agent
        .dispatch(AgentAction::RespondToUserInput {
            request: second,
            input: "compact".into(),
        })
        .unwrap();
    assert!(agent.request.is_none());
    assert!(agent.active.is_none());
    assert!(agent.ready.iter().any(|event| matches!(event,
            AgentPoll::Record(TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text), ..
            })) if text.contains("Priority: reading") && text.contains("Density: compact") && !text.contains("stale"))));
}

// 첫 질문 취소와 부분 답변 뒤 취소 모두 실제 기록 수만 한 번 표시한다.
#[test]
fn interrupted_interview_reports_recorded_and_unanswered_questions_once() {
    for recorded in [false, true] {
        let mut agent = TestAgent::new();
        agent
            .dispatch(AgentAction::submit("interview").unwrap())
            .unwrap();
        if recorded {
            let Some(PreviewRequest::Interview { request, .. }) = agent.request else {
                panic!("question missing")
            };
            agent
                .dispatch(AgentAction::RespondToUserInput {
                    request,
                    input: "1".into(),
                })
                .unwrap();
        }
        agent.dispatch(AgentAction::Interrupt).unwrap();
        let summaries = agent
            .ready
            .iter()
            .filter_map(|event| {
                if let AgentPoll::Record(TranscriptRecord::EventCommitted(
                    AgentEvent::ActivityUpdated {
                        update: ActivityUpdate::TextSnapshot(text),
                        ..
                    },
                )) = event
                {
                    ActivityNotice::from_snapshot(text)
                        .filter(|notice| notice.title == "Interview incomplete")
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].level, NoticeLevel::Warning);
        assert!(summaries[0].message.contains(if recorded {
            "1/2 answers recorded · 1 unanswered"
        } else {
            "0/2 answers recorded · 2 unanswered"
        }));
        assert!(
            summaries[0]
                .message
                .contains("2. Unanswered: How much detail should stay visible?")
        );
        assert!(summaries[0].message.contains("Offline preview"));
        assert!(agent.request.is_none());
        assert!(agent.active.is_none());
        let before = agent.ready.len();
        agent.dispatch(AgentAction::Interrupt).unwrap();
        assert_eq!(agent.ready.len(), before);
    }
}

// idle preview는 polling timer를 요구하지 않는다. 합성 출력이 있을 때만 깨우고,
// 중단 후 종료 observation을 소비하면 다시 무기한 입력 대기로 돌아간다.
#[test]
fn preview_deadlines_exist_only_while_events_are_pending() {
    let mut agent = TestAgent::new();
    assert_eq!(agent.next_deadline(), None);
    agent
        .dispatch(AgentAction::submit("long").unwrap())
        .unwrap();
    assert!(agent.next_deadline().is_some());
    while !agent.ready.is_empty() {
        agent.poll().unwrap();
    }
    agent.next = Instant::now() + Duration::from_secs(1);
    assert_eq!(agent.next_deadline(), Some(agent.next));
    agent.dispatch(AgentAction::Interrupt).unwrap();
    while !agent.ready.is_empty() {
        agent.poll().unwrap();
    }
    assert_eq!(agent.next_deadline(), None);
}
// 세션 경고 예시는 입력만 수락하고 가짜 journal record·Turn·stream을 생성하지 않는다.
#[test]
fn session_notice_preview_has_no_turn_or_journal_record() {
    for (input, title) in [
        ("warning", "Codex configuration warning"),
        ("deprecation", "Codex deprecation notice"),
        ("approval-warning", "Codex approval warning"),
    ] {
        let mut agent = TestAgent::new();
        agent.dispatch(AgentAction::submit(input).unwrap()).unwrap();
        assert!(matches!(agent.poll().unwrap(), AgentPoll::Submission(_)));
        let AgentPoll::Notice(notice) = agent.poll().unwrap() else {
            panic!("missing session notice");
        };
        assert_eq!(notice.title, title);
        assert!(notice.message.starts_with("Preview only:"));
        assert_eq!(agent.poll().unwrap(), AgentPoll::Pending);
        assert_eq!(agent.turns, 0);
        assert!(agent.stream.is_empty());
        assert!(agent.active.is_none());
    }
}

// 입력을 수락하고 실제 문자열을 응답에 반영하며, 중단 후 새 대화를 받을 수 있어야 한다.
#[test]
fn accepts_interrupts_and_accepts_again() {
    let mut agent = TestAgent::new();
    agent
        .dispatch(AgentAction::submit("hello").unwrap())
        .unwrap();
    assert!(matches!(
        agent.poll().unwrap(),
        AgentPoll::Submission(SubmissionOutcome::Accepted { .. })
    ));
    assert!(
        agent
            .stream
            .iter()
            .any(|event| matches!(event, AgentEvent::ActivityUpdated {
            update: ActivityUpdate::TextDelta(text), .. } if text.contains("hello")))
    );
    agent.dispatch(AgentAction::Interrupt).unwrap();
    assert!(agent.stream.is_empty());
    assert!(agent.active.is_none());
    assert_eq!(
        agent
            .dispatch(AgentAction::submit("again").unwrap())
            .unwrap(),
        DispatchOutcome::Queued
    );
}
// 작업 중 입력은 조용히 유실시키지 않고 거절하여 frontend의 draft 보존 경로로 돌린다.
#[test]
fn busy_input_is_rejected() {
    let mut agent = TestAgent::new();
    agent
        .dispatch(AgentAction::submit("long").unwrap())
        .unwrap();
    assert!(matches!(
        agent
            .dispatch(AgentAction::submit("keep draft").unwrap())
            .unwrap(),
        DispatchOutcome::Rejected { .. }
    ));
}

// 모의 실패는 연결을 닫지 않고 Turn을 끝내 다음 입력으로 복구할 수 있어야 한다.
#[test]
fn failed_turn_finishes_and_allows_recovery() {
    let mut agent = TestAgent::new();
    agent
        .dispatch(AgentAction::submit("error").unwrap())
        .unwrap();
    let mut failed = false;
    for _ in 0..200 {
        agent.next = Instant::now();
        if matches!(
            agent.poll().unwrap(),
            AgentPoll::Record(TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                outcome: TurnOutcome::Failed(_),
                ..
            }))
        ) {
            failed = true;
            break;
        }
    }
    assert!(failed);
    assert!(agent.active.is_none());
    assert_eq!(
        agent
            .dispatch(AgentAction::submit("recover").unwrap())
            .unwrap(),
        DispatchOutcome::Queued
    );
}
// 구조화 이미지 예시는 불완전 JSON delta 대신 한 개의 완전한 snapshot으로 전달한다.
#[test]
fn tool_image_preview_emits_one_structured_snapshot() {
    let mut agent = TestAgent::new();
    agent
        .dispatch(AgentAction::submit("mcp-image").unwrap())
        .unwrap();
    assert!(agent.stream.is_empty());
    let updates = agent
        .ready
        .iter()
        .filter_map(|event| match event {
            AgentPoll::Record(TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
                update,
                ..
            })) => Some(update),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(updates.len(), 1);
    let ActivityUpdate::TextSnapshot(text) = updates[0] else {
        panic!("complete snapshot required")
    };
    let output = ToolOutput::from_snapshot(text).unwrap();
    assert_eq!(output.tool, "capture");
    assert!(
        output
            .content_blocks()
            .any(|block| block.get("type").is_some_and(|kind| kind == "image"))
    );
}
// 상태 예시는 전체 snapshot을 갱신·삭제하되 Turn이나 대화 기록을 만들지 않는다.
#[test]
fn host_status_preview_stays_outside_conversation_events() {
    let mut agent = TestAgent::new();
    for (command, expected) in [
        (
            "status",
            "Demo branch: chat-ui · Demo checks: 18 passed · Demo worker: idle",
        ),
        (
            "status-update",
            "Demo checks: 20 passed · Demo worker: ready",
        ),
        ("status-clear", ""),
    ] {
        agent
            .dispatch(AgentAction::submit(command).unwrap())
            .unwrap();
        assert!(matches!(
            agent.poll().unwrap(),
            AgentPoll::Submission(SubmissionOutcome::Accepted { .. })
        ));
        let AgentPoll::StatusLine(status) = agent.poll().unwrap() else {
            panic!("status snapshot missing")
        };
        assert_eq!(status.as_str(), expected);
        assert_eq!(agent.poll().unwrap(), AgentPoll::Pending);
        assert!(agent.active.is_none());
        assert_eq!(agent.turns, 0);
    }
}
// 승인은 응답 전 대기하며 다른 요청의 결정은 무시하고 구조화된 거절도 정확한 결과 제목을
// 표시한다.
#[test]
fn approval_preview_waits_for_exact_request_and_labels_offered_decline() {
    for command in ["approval", "approval-scopes", "approval-diff"] {
        for declined in [false, true] {
            let mut agent = TestAgent::new();
            agent
                .dispatch(AgentAction::submit(command).unwrap())
                .unwrap();
            let Some(PreviewRequest::Approval(request, kind)) = agent.request else {
                panic!("approval request missing")
            };
            let decision = match kind.profile() {
                Some(profile) => ApprovalDecision::Offered(if declined {
                    profile.decline_choice.unwrap()
                } else {
                    1
                }),
                None => {
                    if declined {
                        ApprovalDecision::Declined
                    } else {
                        ApprovalDecision::Approved
                    }
                },
            };
            while !agent.ready.is_empty() {
                agent.poll().unwrap();
            }
            for _ in 0..3 {
                assert_eq!(agent.poll().unwrap(), AgentPoll::Pending);
            }
            assert!(agent.stream.is_empty());
            let stale = ActivityRequestRef::new(
                request.activity(),
                RequestId::new(NonZeroU64::new(999).unwrap()),
            );
            agent
                .dispatch(AgentAction::RespondToApproval {
                    request: stale,
                    decision,
                })
                .unwrap();
            assert!(
                matches!(agent.request, Some(PreviewRequest::Approval(current, _)) if current == request)
            );
            assert_eq!(agent.poll().unwrap(), AgentPoll::Pending);
            agent
                .dispatch(AgentAction::RespondToApproval { request, decision })
                .unwrap();
            assert!(agent.request.is_none());
            let snapshots: Vec<_> = agent
                .ready
                .iter()
                .filter_map(|event| match event {
                    AgentPoll::Record(TranscriptRecord::EventCommitted(
                        AgentEvent::ActivityUpdated {
                            update: ActivityUpdate::TextSnapshot(text),
                            ..
                        },
                    )) => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(
                snapshots
                    .iter()
                    .filter(|text| text.starts_with("Decision:"))
                    .count(),
                1
            );
            assert_eq!(
                snapshots.iter().any(|text| text.starts_with("## Declined")),
                declined
            );
            assert_eq!(
                snapshots
                    .iter()
                    .any(|text| text.starts_with("## Approval recorded")),
                !declined
            );
            let count = agent.ready.len();
            agent
                .dispatch(AgentAction::RespondToApproval { request, decision })
                .unwrap();
            assert_eq!(agent.ready.len(), count);
        }
    }
}
// 세션 문서 예시는 사용자 제출 수락과 문서만 내보내고 Turn·저널 record를 만들지 않는다.
#[test]
fn session_document_preview_emits_only_host_document() {
    let mut agent = TestAgent::new();
    agent
        .dispatch(AgentAction::submit("session-document").unwrap())
        .unwrap();
    assert!(matches!(
        agent.poll().unwrap(),
        AgentPoll::Submission(SubmissionOutcome::Accepted { .. })
    ));
    let AgentPoll::Document(document) = agent.poll().unwrap() else {
        panic!("host document missing")
    };
    let profile = ActivityDocument::from_snapshot(document.snapshot()).unwrap();
    assert_eq!(profile.title, "Workspace guide");
    assert!(profile.markdown.contains("| Action | Shortcut |"));
    assert_eq!(agent.poll().unwrap(), AgentPoll::Pending);
    assert_eq!(agent.turns, 0);
    assert!(agent.active.is_none());
}
