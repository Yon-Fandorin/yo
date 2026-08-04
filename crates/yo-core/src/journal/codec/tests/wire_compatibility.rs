use super::*;

fn committed(command: AgentCommand, submission_id: Option<crate::SubmissionId>) -> JournalRecord {
    let committed = match submission_id {
        Some(submission_id) => crate::journal::CommittedCommand::submission(command, submission_id),
        None => crate::journal::CommittedCommand::uncorrelated(command),
    }
    .expect("the fixture command uses the matching correlation shape");
    JournalRecord::CommandCommitted(committed)
}

fn structured_input() -> crate::UserInput {
    let workspace = crate::WorkspaceReference::new(
        "workspace:src/lib.rs",
        "host:one",
        "workspace:one",
        "root:one",
        "src/lib.rs",
        crate::WorkspaceReferenceKind::File,
    )
    .unwrap();
    let skill = crate::SkillReference::new(
        "skill:review",
        "host:one",
        "/skills/review/SKILL.md",
        "review",
        crate::SkillReferenceScope::User,
        7,
        "sha256:exact",
    );
    crate::UserInput::with_references(
        "use @src/lib.rs with $review",
        vec![
            crate::InputReference::workspace(4..15, workspace),
            crate::InputReference::skill(21..28, skill),
        ],
    )
    .unwrap()
}

// 현재 공개 후보의 schema뿐 아니라 같은 schema 아래 형식 세대를 구분하는 discriminator도
// descriptor-only commit을 포함한 모든 payload에 기록해야 이전 개발 v1과 섞이지 않는다.
#[test]
fn writes_the_anchored_session_release_baseline() {
    let commit = JournalCommit::descriptor(descriptor_with_path(b"/workspace".to_vec()));
    let wire = serde_json::from_str::<serde_json::Value>(&encode(&commit).unwrap()).unwrap();

    assert_eq!(wire["schema"], "yo.semantic-journal-commit/v1");
    assert_eq!(wire["format"], "anchored-session");
}

// v2~v4는 공개된 과거 형식이 아니라 개발 중간 산출물이므로, 새 v1 reader가 이를
// 지원한다고 오해하지 않도록 명시적인 unsupported-schema 오류로 거부한다.
#[test]
fn rejects_pre_release_semantic_journal_versions() {
    let encoded = encode(&JournalCommit::descriptor(descriptor_with_path(
        b"/workspace".to_vec(),
    )))
    .unwrap();

    for schema in [
        "yo.semantic-journal-commit/v2",
        "yo.semantic-journal-commit/v3",
        "yo.semantic-journal-commit/v4",
    ] {
        let mut wire = serde_json::from_str::<serde_json::Value>(&encoded).unwrap();
        wire["schema"] = schema.into();

        let error = decode(&wire.to_string()).expect_err("pre-release schema is unsupported");
        assert!(
            error
                .to_string()
                .contains("unsupported Journal commit schema")
        );
    }
}

// 같은 v1 tag를 사용했던 이전 문자열 형식은 format discriminator가 없으므로 현재
// reader가 구조를 추측하지 않고 JSON 경계에서 바로 실패해야 한다.
#[test]
fn rejects_the_displaced_string_input_v1() {
    let payload = serde_json::json!({
        "schema": "yo.semantic-journal-commit/v1",
        "kind": "incremental",
        "journal_cutoff": 1,
        "first_sequence": 1,
        "records": [{
            "type": "command_committed",
            "command": {
                "type": "start_turn",
                "turn": {
                    "session_id": activity().session_id().to_string(),
                    "turn_id": activity().turn().turn_id().get().get()
                },
                "input": "old"
            }
        }]
    });

    let error = decode(&payload.to_string()).expect_err("old v1 input shape is displaced");

    assert!(
        error.to_string().contains("expected struct WireUserInput"),
        "{error}"
    );
}

// v1 envelope가 비어 있으면 Session ID를 찾는 과정에서 panic하지 않고 손상된 commit을
// 설명하는 typed 오류를 반환해야 한다.
#[test]
fn rejects_an_empty_v1_commit_without_panicking() {
    let payload = serde_json::json!({
        "schema": "yo.semantic-journal-commit/v1",
        "format": "anchored-session",
        "kind": "incremental",
        "journal_cutoff": 1,
        "first_sequence": 1,
        "records": []
    });

    let error = decode(&payload.to_string()).expect_err("empty commits are invalid");

    assert!(error.to_string().contains("at least one Journal record"));
}

// StartTurn은 입력 text, 순서가 있는 typed occurrences, 그리고 frontend가 부여한
// SubmissionId를 함께 왕복해야 replay가 표시 문자열을 다시 파싱하지 않는다.
#[test]
fn round_trips_structured_start_input_and_submission_identity() {
    let input = structured_input();
    let command = AgentCommand::StartTurn {
        turn: activity().turn(),
        input: input.clone(),
    };
    let commit = JournalCommit::incremental(sequenced(
        1,
        [committed(command.clone(), Some(submission(4)))],
    ));

    let encoded = encode(&commit).expect("structured input encodes");
    let decoded = decode(&encoded).expect("structured input decodes");
    let JournalRecord::CommandCommitted(decoded_command) = decoded.records()[0].record() else {
        panic!("the command record remains committed");
    };

    assert_eq!(decoded_command.command(), &command);
    assert_eq!(decoded_command.submission_id(), Some(submission(4)));
    let wire = serde_json::from_str::<serde_json::Value>(&encoded).unwrap();
    assert_eq!(
        wire["records"][0]["command"]["input"]["profile"],
        "yo.structured-input/v1"
    );
    assert_eq!(
        wire["records"][0]["command"]["input"]["references"][0]["type"],
        "workspace"
    );
    assert_eq!(
        wire["records"][0]["command"]["input"]["references"][1]["type"],
        "skill"
    );
}

// Activity user-input response는 별도 SubmissionId를 만들지 않지만 Start/Steer와 같은
// structured input object를 사용해 typed identity를 손실 없이 보존해야 한다.
#[test]
fn round_trips_structured_activity_user_input_without_submission_identity() {
    let command = AgentCommand::RespondToActivity {
        request: crate::ActivityRequestRef::new(
            activity(),
            crate::RequestId::new(std::num::NonZeroU64::new(9).unwrap()),
        ),
        response: crate::ActivityResponse::UserInput(structured_input()),
    };
    let commit = JournalCommit::incremental(sequenced(1, [committed(command.clone(), None)]));

    let decoded = decode(&encode(&commit).unwrap()).unwrap();
    let JournalRecord::CommandCommitted(decoded_command) = decoded.records()[0].record() else {
        panic!("the response remains a committed command");
    };

    assert_eq!(decoded_command.command(), &command);
    assert_eq!(decoded_command.submission_id(), None);
}

// Start/Steer의 correlation identity가 wire에 없으면 reader가 임의 ID를 만들거나
// 무관한 command identity를 재사용하지 않고 JSON 경계에서 실패해야 한다.
#[test]
fn rejects_a_submission_command_without_submission_identity() {
    let commit = JournalCommit::incremental(sequenced(
        1,
        [committed(
            AgentCommand::StartTurn {
                turn: activity().turn(),
                input: crate::UserInput::new("missing"),
            },
            Some(submission(12)),
        )],
    ));
    let mut wire = serde_json::from_str::<serde_json::Value>(&encode(&commit).unwrap()).unwrap();
    wire["records"][0]["command"]
        .as_object_mut()
        .unwrap()
        .remove("submission_id");

    let error = decode(&wire.to_string()).expect_err("StartTurn requires correlation identity");

    assert!(
        error.to_string().contains("missing field `submission_id`"),
        "{error}"
    );
}

// UUID parser가 허용하는 축약형이나 대문자 표현까지 wire에서 받으면 같은 identity가
// 여러 byte 표현을 가지므로 canonical lowercase hyphenated UUIDv4만 수용한다.
#[test]
fn rejects_a_noncanonical_submission_identity_representation() {
    let commit = JournalCommit::incremental(sequenced(
        1,
        [committed(
            AgentCommand::StartTurn {
                turn: activity().turn(),
                input: crate::UserInput::new("inspect"),
            },
            Some(submission(10)),
        )],
    ));
    let mut wire = serde_json::from_str::<serde_json::Value>(&encode(&commit).unwrap()).unwrap();
    wire["records"][0]["command"]["submission_id"] = "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA".into();

    let error = decode(&wire.to_string()).expect_err("noncanonical UUID text is rejected");

    assert!(error.to_string().contains("canonical lowercase"), "{error}");
}

// descriptor-only commit도 format generation을 명시해야 하며 같은 schema 아래 다른
// generation 문자열은 현재 structured-input reader가 의미를 추측하지 않는다.
#[test]
fn rejects_missing_or_unknown_format_discriminators() {
    let encoded = encode(&JournalCommit::descriptor(descriptor_with_path(
        b"/workspace".to_vec(),
    )))
    .unwrap();

    let mut missing = serde_json::from_str::<serde_json::Value>(&encoded).unwrap();
    missing.as_object_mut().unwrap().remove("format");
    let missing_error = decode(&missing.to_string()).expect_err("format is mandatory");
    assert!(missing_error.to_string().contains("missing field `format`"));

    let mut unknown = serde_json::from_str::<serde_json::Value>(&encoded).unwrap();
    unknown["format"] = "future".into();
    let unknown_error = decode(&unknown.to_string()).expect_err("future format is unsupported");
    assert!(
        unknown_error
            .to_string()
            .contains("unsupported Journal commit format")
    );
}

// command, input, occurrence의 닫힌 경계에서 미래 필드를 무시하면 같은 v1 bytes가 서로
// 다른 의미가 될 수 있으므로 각 레이어가 unknown field를 거부해야 한다.
#[test]
fn rejects_unknown_fields_at_each_structured_input_boundary() {
    let commit = JournalCommit::incremental(sequenced(
        1,
        [committed(
            AgentCommand::StartTurn {
                turn: activity().turn(),
                input: structured_input(),
            },
            Some(submission(5)),
        )],
    ));
    let encoded = encode(&commit).unwrap();

    for location in ["record", "command", "input", "occurrence"] {
        let mut wire = serde_json::from_str::<serde_json::Value>(&encoded).unwrap();
        let target = match location {
            "record" => &mut wire["records"][0],
            "command" => &mut wire["records"][0]["command"],
            "input" => &mut wire["records"][0]["command"]["input"],
            "occurrence" => &mut wire["records"][0]["command"]["input"]["references"][0],
            _ => unreachable!(),
        };
        target["future_field"] = true.into();

        let error = decode(&wire.to_string()).expect_err("unknown fields are unsupported");
        assert!(
            error.to_string().contains("unknown field"),
            "{location}: {error}"
        );
    }
}

// persisted profile은 live helper의 향후 정책과 분리된 닫힌 계약이므로 잘못된 profile,
// span/projection, path, skill generation을 decoder가 각각 typed domain으로 들이지 않는다.
#[test]
fn rejects_invalid_persisted_reference_invariants() {
    let commit = JournalCommit::incremental(sequenced(
        1,
        [committed(
            AgentCommand::StartTurn {
                turn: activity().turn(),
                input: structured_input(),
            },
            Some(submission(11)),
        )],
    ));
    let original = serde_json::from_str::<serde_json::Value>(&encode(&commit).unwrap()).unwrap();

    for (case, expected, mutate) in [
        ("profile", "unsupported structured input profile", 0_u8),
        ("projection", "invalid structured input", 1),
        ("path", "invalid workspace reference", 2),
        ("generation", "invalid structured input", 3),
    ] {
        let mut wire = original.clone();
        match mutate {
            0 => wire["records"][0]["command"]["input"]["profile"] = "future".into(),
            1 => {
                wire["records"][0]["command"]["input"]["references"][0]["projection"] =
                    "mismatch".into()
            },
            2 => {
                wire["records"][0]["command"]["input"]["references"][0]["relative_path"] =
                    "../escape".into()
            },
            3 => {
                wire["records"][0]["command"]["input"]["references"][1]["catalog_generation"] =
                    0.into()
            },
            _ => unreachable!(),
        }

        let error = decode(&wire.to_string()).expect_err("invalid persisted metadata is rejected");
        assert!(error.to_string().contains(expected), "{case}: {error}");
    }
}

// 저장된 projection은 당시 승인된 bytes이므로 decoder가 현재 display helper로 다시
// 계산하지 않고 text span 일치와 typed metadata만 검증해야 한다.
#[test]
fn replay_preserves_an_historical_projection_without_reinterpreting_it() {
    let commit = JournalCommit::incremental(sequenced(
        1,
        [committed(
            AgentCommand::StartTurn {
                turn: activity().turn(),
                input: crate::UserInput::new("placeholder"),
            },
            Some(submission(6)),
        )],
    ));
    let mut wire = serde_json::from_str::<serde_json::Value>(&encode(&commit).unwrap()).unwrap();
    wire["records"][0]["command"]["input"] = serde_json::json!({
        "profile": "yo.structured-input/v1",
        "text": "historic",
        "references": [{
            "type": "workspace",
            "start": 0,
            "end": 8,
            "projection": "historic",
            "identity": "workspace:historic",
            "execution_environment_identity": "host:one",
            "workspace_identity": "workspace:one",
            "root_identity": "root:one",
            "relative_path": "src/lib.rs",
            "kind": "file"
        }]
    });

    let decoded = decode(&wire.to_string()).expect("stored projection remains authoritative bytes");
    let JournalRecord::CommandCommitted(committed) = decoded.records()[0].record() else {
        panic!("the historical input remains a command");
    };
    let AgentCommand::StartTurn { input, .. } = committed.command() else {
        panic!("the command remains StartTurn");
    };

    assert_eq!(input.references()[0].projection(), "historic");
}

// workspace path의 tagged 표현도 encoding과 value 외의 필드를 허용하면 미래 의미를
// 현재 v1 reader가 버릴 수 있으므로 descriptor 내부 경계까지 엄격하게 검증해야 한다.
#[test]
fn rejects_an_unknown_v1_workspace_path_field() {
    let descriptor = descriptor_with_path(b"/workspace".to_vec());
    let mut wire = serde_json::from_str::<serde_json::Value>(
        &encode(&JournalCommit::descriptor(descriptor)).unwrap(),
    )
    .unwrap();
    wire["records"][0]["descriptor"]["workspace_path"]["future_field"] = true.into();

    let error = decode(&wire.to_string()).expect_err("unknown workspace path field is unsupported");

    assert!(error.to_string().contains("future_field"), "{error}");
}
