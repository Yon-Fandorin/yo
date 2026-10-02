use std::num::NonZeroU64;

use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityResponse,
    ActivityUpdate, AgentCommand, AgentEvent, ApprovalDecision, Failure, RequestId, SessionId,
    TurnId, TurnOutcome, TurnRef, UserInput,
};

fn id(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn references() -> (SessionId, TurnRef, ActivityRef) {
    let session_id = crate::fixture_session(1);
    let turn = TurnRef::new(session_id, TurnId::new(id(2)));
    let activity = ActivityRef::new(turn, ActivityId::new(id(3)));
    (session_id, turn, activity)
}

// 프런트엔드가 세션 내부를 직접 수정하지 않고 명시적인 도메인 명령으로 의도를 전달함을 확인한다.
#[test]
fn commands_carry_the_identity_of_their_target() {
    let (session_id, turn, _) = references();
    let input = UserInput::from("inspect the repository");

    let AgentCommand::CreateSession {
        session_id: observed_session,
    } = (AgentCommand::CreateSession { session_id })
    else {
        panic!("expected a create-session command");
    };
    let AgentCommand::StartTurn {
        turn: observed_turn,
        input: observed_input,
    } = (AgentCommand::StartTurn {
        turn,
        input: input.clone(),
    })
    else {
        panic!("expected a start-turn command");
    };

    assert_eq!(observed_session, session_id);
    assert_eq!(observed_turn, turn);
    assert_eq!(observed_input, input);
    assert_ne!(
        AgentCommand::InterruptTurn { turn },
        AgentCommand::InterruptTurn {
            turn: TurnRef::new(crate::fixture_session(9), TurnId::new(id(9))),
        }
    );
}

// 활동의 시작·갱신·종료 이벤트가 모두 같은 ActivityRef를 가져 상관관계를 잃지 않음을 확인한다.
#[test]
fn every_activity_phase_carries_the_same_correlation_identity() {
    let (_, _, activity) = references();
    let events = [
        AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        },
        AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextDelta("hello".to_owned()),
        },
        AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        },
    ];

    for event in events {
        let observed = match event {
            AgentEvent::ActivityStarted { activity, .. }
            | AgentEvent::ActivityUpdated { activity, .. }
            | AgentEvent::ActivityFinished { activity, .. } => activity,
            _ => panic!("expected an activity event"),
        };
        assert_eq!(observed, activity);
    }
}

// TextDelta는 기존 텍스트 뒤에 이어 붙일 조각이고 TextSnapshot은 지금까지의 텍스트를
// 통째로 교체할 최종 상태여서 frontend가 둘을 추측 없이 구분할 수 있는지 확인한다.
#[test]
fn text_updates_distinguish_append_from_replacement() {
    assert_ne!(
        ActivityUpdate::TextDelta("answer".to_owned()),
        ActivityUpdate::TextSnapshot("answer".to_owned())
    );
}

// 승인 응답 명령과 응답 Activity가 원래 요청의 Activity와 request ID를 함께 가리킴을 확인한다.
#[test]
fn activity_response_keeps_its_request_correlation() {
    let (_, _, activity) = references();
    let request_id = RequestId::new(id(4));
    let request = ActivityRequestRef::new(activity, request_id);
    let command = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::Approval(ApprovalDecision::Approved),
    };
    let event = AgentEvent::ActivityStarted {
        activity: ActivityRef::new(activity.turn(), ActivityId::new(id(5))),
        kind: ActivityKind::ApprovalResponse { request_id },
    };

    let AgentCommand::RespondToActivity {
        request: observed_request,
        ..
    } = command
    else {
        panic!("expected an activity-response command");
    };
    let AgentEvent::ActivityStarted {
        kind:
            ActivityKind::ApprovalResponse {
                request_id: observed_request_id,
            },
        ..
    } = event
    else {
        panic!("expected an approval-response activity");
    };

    assert_eq!(observed_request.activity(), activity);
    assert_eq!(observed_request.request_id(), request_id);
    assert_eq!(observed_request_id, request_id);
}

// Turn 종료가 정상 완료·사용자 중단·실패를 구분해 프런트엔드가 문자열을 해석하지 않아도 됨을
// 확인한다.
#[test]
fn turn_outcomes_distinguish_completion_interruption_and_failure() {
    let (_, turn, _) = references();
    let failure = Failure::new("backend disconnected");
    let outcomes = [
        TurnOutcome::Completed,
        TurnOutcome::Interrupted,
        TurnOutcome::Failed(failure.clone()),
    ];

    assert!(matches!(outcomes[0], TurnOutcome::Completed));
    assert!(matches!(outcomes[1], TurnOutcome::Interrupted));
    assert!(matches!(
        &outcomes[2],
        TurnOutcome::Failed(observed) if observed == &failure
    ));
    assert_ne!(outcomes[0], outcomes[1]);
    assert_ne!(outcomes[1], outcomes[2]);

    let AgentEvent::TurnFinished {
        turn: observed_turn,
        outcome: observed_outcome,
    } = (AgentEvent::TurnFinished {
        turn,
        outcome: outcomes[2].clone(),
    })
    else {
        panic!("expected a turn-finished event");
    };
    assert_eq!(observed_turn, turn);
    assert_eq!(observed_outcome, TurnOutcome::Failed(failure));
}

// 승인 표시 프로필은 64개 경계와 거절 위치를 검증하고 잘못된 스키마·비활성 기본값을 받지 않는다.
#[test]
fn approval_profile_bounds_choices_and_decline_identity() {
    use crate::{ActivityApproval, ApprovalChoice, ToolOutput};
    let choice = ApprovalChoice {
        label: "Decline".into(),
        description: "No grant".into(),
        enabled: true,
    };
    let mut profile = ActivityApproval {
        related_change: None,
        plain_text: "Review".into(),
        choices: vec![choice; 64],
        decline_choice: Some(64),
    };
    profile.related_change = Some(0);
    assert!(profile.to_snapshot().is_none());
    profile.related_change = Some(12);
    let linked = profile.to_snapshot().unwrap();
    assert_eq!(
        ActivityApproval::from_snapshot(&linked),
        Some(profile.clone())
    );
    profile.related_change = None;
    let snapshot = profile.to_snapshot().unwrap();
    assert!(!snapshot.contains("related_change"));
    assert_eq!(
        ActivityApproval::from_snapshot(&snapshot),
        Some(profile.clone())
    );
    assert!(
        ActivityApproval::from_snapshot(
            &snapshot.replace(ActivityApproval::SCHEMA, "yo.activity-approval/v2")
        )
        .is_none()
    );
    profile.choices.push(profile.choices[0].clone());
    assert!(profile.to_snapshot().is_none());
    profile.choices.pop();
    for invalid in [0, 65, u32::MAX] {
        profile.decline_choice = Some(invalid);
        assert!(profile.to_snapshot().is_none());
    }
    profile.decline_choice = Some(1);
    profile.choices[0].enabled = false;
    assert!(profile.to_snapshot().is_none());
    profile.decline_choice = None;
    profile.choices.clear();
    assert!(profile.to_snapshot().is_some());
    let overhead = profile.to_snapshot().unwrap().len() - profile.plain_text.len();
    profile.plain_text = "x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - overhead);
    let exact = profile.to_snapshot().unwrap();
    assert_eq!(exact.len(), ToolOutput::MAX_SNAPSHOT_BYTES);
    assert_eq!(
        ActivityApproval::from_snapshot(&exact),
        Some(profile.clone())
    );
    profile.plain_text.push('x');
    assert!(profile.to_snapshot().is_none());
}

// 메시지 콘텐츠는 알 수 없는 필드까지 보존하며 정확한 크기 상한만 허용한다.
#[test]
fn message_content_round_trip_and_first_excess() {
    use serde_json::json;

    use crate::{MessageContent, ToolOutput};
    let mut content = MessageContent {
        block: json!({"type":"future","data":"","_meta":{"revision":3}}),
    };
    let small = content.to_snapshot().unwrap();
    assert_eq!(MessageContent::from_snapshot(&small), Some(content.clone()));
    assert!(
        MessageContent::from_snapshot(&small.replace(MessageContent::SCHEMA, "other")).is_none()
    );
    content.block["data"] = "x"
        .repeat(ToolOutput::MAX_SNAPSHOT_BYTES - small.len())
        .into();
    let exact = content.to_snapshot().unwrap();
    assert_eq!(exact.len(), ToolOutput::MAX_SNAPSHOT_BYTES);
    assert_eq!(MessageContent::from_snapshot(&exact), Some(content.clone()));
    let data = content.block["data"].as_str().unwrap().to_owned() + "x";
    content.block["data"] = data.into();
    assert!(content.to_snapshot().is_none());
    assert!(MessageContent::from_snapshot(&(exact + " ")).is_none());
}

// 게시 결과 profile은 원문을 보존하고 버전, state, 전체 크기 경계를 엄격히 확인한다.
#[test]
fn file_publication_evidence_round_trip_and_closed_bounds() {
    use serde_json::Value;

    use crate::{
        FilePublicationEvidence, FilePublicationEvidenceState,
        FilePublicationEvidenceUnavailableReason,
    };

    let before = "fn main() { println!(\"old\\path\"); }\n";
    let after = "fn main() { println!(\"새 값\\path\"); }\n";
    let complete = FilePublicationEvidence::complete("src/main.rs", before, after).unwrap();
    let snapshot = complete.to_snapshot().unwrap();
    let value: Value = serde_json::from_str(&snapshot).unwrap();
    assert_eq!(value["schema"], FilePublicationEvidence::SCHEMA);
    assert_eq!(value["output"]["path"], "src/main.rs");
    assert_eq!(value["output"]["state"]["status"], "complete");
    assert_eq!(value["output"]["state"]["before"], before);
    assert_eq!(value["output"]["state"]["after"], after);
    assert_eq!(
        FilePublicationEvidence::from_snapshot(&snapshot),
        Some(complete)
    );

    assert!(
        FilePublicationEvidence::from_snapshot(&snapshot.replace(
            FilePublicationEvidence::SCHEMA,
            "yo.file-publication-evidence/v2"
        ))
        .is_none()
    );
    let mut unknown_field: Value = serde_json::from_str(&snapshot).unwrap();
    unknown_field["output"]["extra"] = Value::Bool(true);
    assert!(FilePublicationEvidence::from_snapshot(&unknown_field.to_string()).is_none());
    let mut unknown_state: Value = serde_json::from_str(&snapshot).unwrap();
    unknown_state["output"]["state"]["status"] = Value::String("future".to_owned());
    assert!(FilePublicationEvidence::from_snapshot(&unknown_state.to_string()).is_none());

    let unavailable = FilePublicationEvidence::unavailable(
        "src/main.rs",
        FilePublicationEvidenceUnavailableReason::Disabled,
    )
    .unwrap();
    let unavailable_snapshot = unavailable.to_snapshot().unwrap();
    assert_eq!(
        FilePublicationEvidence::from_snapshot(&unavailable_snapshot),
        Some(unavailable)
    );
    assert_eq!(
        serde_json::from_str::<Value>(&unavailable_snapshot).unwrap()["output"]["state"]["reason"],
        "disabled"
    );
    assert!(!unavailable_snapshot.contains("before"));
    assert!(!unavailable_snapshot.contains("after"));

    let empty = FilePublicationEvidence::complete("a", "", "").unwrap();
    let empty_snapshot = empty.to_snapshot().unwrap();
    let exact_after =
        "x".repeat(FilePublicationEvidence::MAX_SNAPSHOT_BYTES - empty_snapshot.len());
    let exact = FilePublicationEvidence::complete("a", "", exact_after).unwrap();
    let exact_snapshot = exact.to_snapshot().unwrap();
    assert_eq!(
        exact_snapshot.len(),
        FilePublicationEvidence::MAX_SNAPSHOT_BYTES
    );
    assert_eq!(
        FilePublicationEvidence::from_snapshot(&exact_snapshot),
        Some(exact)
    );
    assert!(FilePublicationEvidence::from_snapshot(&(exact_snapshot + "x")).is_none());

    assert!(FilePublicationEvidence::complete("bad\npath", "a", "b").is_none());
    assert!(matches!(
        FilePublicationEvidence::from_snapshot(&snapshot)
            .unwrap()
            .state(),
        FilePublicationEvidenceState::Complete { .. }
    ));
}
