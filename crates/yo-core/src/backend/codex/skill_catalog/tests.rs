use std::sync::mpsc;

use super::{SkillInterface, SkillMetadata, WireScope, candidate_from_wire, newest_request};
use crate::{SkillAvailability, SkillReferenceScope, SkillReferenceSearchRequest};

// Codex wire scope `repo`는 경로 추측 없이 Workspace 출처로 매핑되고,
// interface의 짧은 이름과 설명이 일반 메타데이터보다 우선한다.
#[test]
fn wire_metadata_maps_to_honest_workspace_provenance() {
    let candidate = candidate_from_wire(
        "local-host:fixture",
        SkillMetadata {
            name: "raw-name".to_owned(),
            description: "Long description".to_owned(),
            short_description: Some("Legacy description".to_owned()),
            path: "/workspace/.agents/skills/review/SKILL.md".to_owned(),
            scope: WireScope::Repo,
            enabled: true,
            interface: Some(SkillInterface {
                display_name: Some("Review".to_owned()),
                short_description: Some("Review changes".to_owned()),
            }),
        },
        3,
        Ok("sha256:exact".to_owned()),
    );

    assert_eq!(
        candidate.reference().scope(),
        SkillReferenceScope::Workspace
    );
    assert_eq!(candidate.display_name(), "Review");
    assert_eq!(candidate.description(), "Review changes");
    assert_eq!(
        candidate.reference().execution_environment_identity(),
        "local-host:fixture"
    );
    assert_eq!(candidate.reference().catalog_generation(), 3);
    assert_eq!(candidate.availability(), &SkillAvailability::Enabled);
}

// 비활성 Codex 항목은 목록에는 남지만 선택 불가 이유를 함께 보존한다.
#[test]
fn disabled_wire_skill_remains_visible_with_a_reason() {
    let candidate = candidate_from_wire(
        "local-host:fixture",
        SkillMetadata {
            name: "review".to_owned(),
            description: "Review changes".to_owned(),
            short_description: None,
            path: "/skills/review/SKILL.md".to_owned(),
            scope: WireScope::User,
            enabled: false,
            interface: None,
        },
        1,
        Ok("sha256:exact".to_owned()),
    );

    assert!(
        matches!(candidate.availability(), SkillAvailability::Disabled(reason) if reason == "Disabled by Codex configuration")
    );
}

// Codex가 enabled로 보고해도 exact revision을 읽을 수 없으면 선택을 허용하지 않아,
// 제출 시점 재검증이 비교할 수 없는 reference가 UI에서 만들어지지 않는다.
#[test]
fn missing_revision_disables_an_otherwise_enabled_skill() {
    let candidate = candidate_from_wire(
        "local-host:test",
        SkillMetadata {
            name: "review".to_owned(),
            description: "Review changes".to_owned(),
            short_description: None,
            path: "/missing/review/SKILL.md".to_owned(),
            scope: WireScope::User,
            enabled: true,
            interface: None,
        },
        1,
        Err("Skill revision unavailable: missing".to_owned()),
    );

    assert!(
        matches!(candidate.availability(), SkillAvailability::Disabled(reason) if reason == "Skill revision unavailable: missing")
    );
    assert_eq!(candidate.reference().entry_revision(), "unavailable");
}

// 새 overlay의 refresh 요청 뒤 연속 입력 요청이 queue에서 합쳐져도 최신 query를 쓰면서
// refresh 의도는 보존해, 오래된 catalog가 우연히 재사용되지 않는다.
#[test]
fn request_coalescing_preserves_catalog_refresh_intent() {
    let (sender, receiver) = mpsc::channel();
    sender
        .send(SkillReferenceSearchRequest::new(
            2,
            2,
            3,
            0..3,
            "$re",
            "re",
            false,
        ))
        .unwrap();
    let first = SkillReferenceSearchRequest::new(1, 1, 1, 0..1, "$", "", true);

    let (latest, refresh) = newest_request(first, &receiver);

    assert_eq!(latest.request_id(), 2);
    assert_eq!(latest.query(), "re");
    assert!(refresh);
}
