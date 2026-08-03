use super::*;

// 정식 릴리스 전에 현재 Journal 계약을 v1 기준점으로 고정해, 개발 과정에서 사용한
// 임시 버전 번호가 새 저장소의 호환성 약속으로 남지 않게 한다.
#[test]
fn writes_the_release_baseline_as_semantic_journal_v1() {
    let commit = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::CommandCommitted(
            AgentCommand::CreateSession {
                session_id: activity().session_id(),
            },
        )],
    ));
    let wire = serde_json::from_str::<serde_json::Value>(&encode(&commit).unwrap()).unwrap();

    assert_eq!(wire["schema"], "yo.semantic-journal-commit/v1");
}

// v2~v4는 공개된 과거 형식이 아니라 개발 중간 산출물이므로, 새 v1 reader가 이를
// 지원한다고 오해하지 않도록 명시적인 unsupported-schema 오류로 거부한다.
#[test]
fn rejects_pre_release_semantic_journal_versions() {
    let commit = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::CommandCommitted(
            AgentCommand::CreateSession {
                session_id: activity().session_id(),
            },
        )],
    ));
    let encoded = encode(&commit).unwrap();

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

// v1 envelope가 비어 있으면 Session ID를 찾는 과정에서 panic하지 않고 손상된 commit을
// 설명하는 typed 오류를 반환해야 한다.
#[test]
fn rejects_an_empty_v1_commit_without_panicking() {
    let payload = serde_json::json!({
        "schema": "yo.semantic-journal-commit/v1",
        "kind": "incremental",
        "journal_cutoff": 1,
        "first_sequence": 1,
        "records": []
    });

    let error = decode(&payload.to_string()).expect_err("empty commits are invalid");

    assert!(error.to_string().contains("at least one Journal record"));
}

// v1 record와 그 안쪽 command에 정의되지 않은 필드를 조용히 무시하면 같은 schema가
// 서로 다른 의미로 해석될 수 있으므로 각 nested wire 경계를 fail-closed로 거부해야 한다.
#[test]
fn rejects_unknown_fields_at_each_v1_record_boundary() {
    let commit = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::CommandCommitted(AgentCommand::StartTurn {
            turn: activity().turn(),
            input: crate::UserInput::new("strict"),
        })],
    ));
    let encoded = encode(&commit).unwrap();

    for location in ["record", "command"] {
        let mut wire = serde_json::from_str::<serde_json::Value>(&encoded).unwrap();
        if location == "record" {
            wire["records"][0]["future_field"] = true.into();
        } else {
            wire["records"][0]["command"]["future_field"] = true.into();
        }

        let error = decode(&wire.to_string()).expect_err("unknown nested fields are unsupported");

        assert!(error.to_string().contains("unknown field"));
    }
}

// 현재 semantic v1은 plain input 문자열로 고정되어 있으므로 구조화된 reference를
// 조용히 문자열로 낮추지 않는다. 새 wire shape의 SOT 승인이 있기 전에는 encode가 막힌다.
#[test]
fn fixed_v1_rejects_structured_input_instead_of_dropping_reference_identity() {
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
    let input = crate::UserInput::with_references(
        "use @src/lib.rs with $review",
        vec![
            crate::InputReference::workspace(4..15, workspace),
            crate::InputReference::skill(21..28, skill),
        ],
    )
    .unwrap();
    let command = AgentCommand::StartTurn {
        turn: activity().turn(),
        input,
    };
    let commit = JournalCommit::incremental(sequenced(
        1,
        [JournalRecord::CommandCommitted(command.clone())],
    ));

    let error = encode(&commit).expect_err("fixed semantic v1 cannot lose typed references");

    assert!(error.to_string().contains("structured input"), "{error}");
}

// Activity response도 같은 UserInput domain을 쓰므로 fixed v1 writer가 reference를
// 문자열로 낮추지 않고 Start/Steer와 같은 실패-폐쇄 경계를 적용한다.
#[test]
fn fixed_v1_also_rejects_structured_activity_user_input() {
    let workspace = crate::WorkspaceReference::new(
        "workspace:src/lib.rs",
        "host:one",
        "workspace:one",
        "root:one",
        "src/lib.rs",
        crate::WorkspaceReferenceKind::File,
    )
    .unwrap();
    let input = crate::UserInput::with_references(
        "@src/lib.rs",
        vec![crate::InputReference::workspace(0..11, workspace)],
    )
    .unwrap();
    let command = AgentCommand::RespondToActivity {
        request: crate::ActivityRequestRef::new(
            activity(),
            crate::RequestId::new(std::num::NonZeroU64::new(9).unwrap()),
        ),
        response: crate::ActivityResponse::UserInput(input),
    };
    let commit =
        JournalCommit::incremental(sequenced(1, [JournalRecord::CommandCommitted(command)]));

    let error = encode(&commit).expect_err("fixed semantic v1 cannot lose response references");

    assert!(error.to_string().contains("structured input"), "{error}");
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
