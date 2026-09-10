use std::ops::Range;

use uuid::{Builder, Uuid};

use super::{InputReference, SubmissionId, UserInput, UserInputError};
use crate::{SkillReference, SkillReferenceScope, WorkspaceReference, WorkspaceReferenceKind};

fn workspace(path: &str, kind: WorkspaceReferenceKind) -> WorkspaceReference {
    WorkspaceReference::new(
        format!("workspace:{path}"),
        "host:one",
        "workspace:one",
        "root:one",
        path,
        kind,
    )
    .unwrap()
}

fn skill(name: &str) -> SkillReference {
    SkillReference::new(
        format!("skill:{name}"),
        "host:one",
        format!("/skills/{name}/SKILL.md"),
        name,
        SkillReferenceScope::User,
        1,
        "sha256:exact",
    )
}

// UUIDv4가 아닌 외부 값은 재개나 원격 응답에서 submission identity로 오인되지 않는다.
#[test]
fn submission_identity_accepts_only_uuid_v4() {
    assert!(SubmissionId::from_uuid(Builder::from_random_bytes([7_u8; 16]).into_uuid()).is_ok());
    assert!(SubmissionId::from_uuid(Uuid::now_v7()).is_err());
}

// visible text와 구조화된 reference는 같은 byte span을 가리켜야 하며 한 목록에서
// draft 순서를 보존하므로 Backend나 Journal이 문자열을 다시 해석할 필요가 없다.
#[test]
fn ordered_workspace_and_skill_references_preserve_exact_projections() {
    let input = UserInput::with_references(
        "read @src/lib.rs with $review",
        vec![
            InputReference::workspace(
                Range { start: 5, end: 16 },
                workspace("src/lib.rs", WorkspaceReferenceKind::File),
            ),
            InputReference::skill(Range { start: 22, end: 29 }, skill("review")),
        ],
    )
    .unwrap();

    assert_eq!(input.as_str(), "read @src/lib.rs with $review");
    assert_eq!(input.references().len(), 2);
    assert!(input.references()[0].workspace_reference().is_some());
    assert!(input.references()[1].skill_reference().is_some());
}

// 디렉터리 reference의 trailing slash도 identity가 아니라 projection 계약의 일부이므로
// 빠진 문자열은 typed reference로 승인되지 않는다.
#[test]
fn directory_projection_requires_its_trailing_slash() {
    let error = UserInput::with_references(
        "inspect @src",
        vec![InputReference::workspace(
            8..12,
            workspace("src", WorkspaceReferenceKind::Directory),
        )],
    )
    .unwrap_err();

    assert_eq!(error, UserInputError::ProjectionMismatch { index: 0 });
}

// V1 skill cardinality는 UI만의 편의가 아니므로 core input 생성 단계에서도 두 번째
// skill을 거부하고 기존 reference를 임의로 버리지 않는다.
#[test]
fn version_one_rejects_a_second_explicit_skill() {
    let error = UserInput::with_references(
        "$one $two",
        vec![
            InputReference::skill(0..4, skill("one")),
            InputReference::skill(5..9, skill("two")),
        ],
    )
    .unwrap_err();

    assert_eq!(error, UserInputError::TooManySkills);
}

// reference span은 UTF-8 grapheme의 내부 byte를 자르거나 앞 reference와 겹칠 수 없다.
#[test]
fn invalid_utf8_boundaries_and_overlaps_fail_closed() {
    let boundary = UserInput::with_references(
        "가 @src/lib.rs",
        vec![InputReference::workspace(
            1..13,
            workspace("src/lib.rs", WorkspaceReferenceKind::File),
        )],
    )
    .unwrap_err();
    assert_eq!(
        boundary,
        UserInputError::InvalidReferenceBoundary { index: 0 }
    );

    let overlap = UserInput::with_references(
        "@src/lib.rs",
        vec![
            InputReference::workspace(0..11, workspace("src/lib.rs", WorkspaceReferenceKind::File)),
            InputReference::workspace(0..11, workspace("src/lib.rs", WorkspaceReferenceKind::File)),
        ],
    )
    .unwrap_err();
    assert_eq!(overlap, UserInputError::ReferenceOrder { index: 1 });
}

// 화면에 escape된 token은 raw filesystem path와 달라도 occurrence가 보존한 정확한
// projection과 일치하면 유효하다. admission은 별도 typed identity를 다시 검증한다.
#[test]
fn escaped_visible_projection_does_not_reinterpret_the_raw_reference_path() {
    let input = UserInput::with_references(
        "read @line\\u{A}break",
        vec![InputReference::workspace(
            5..20,
            workspace("line\nbreak", WorkspaceReferenceKind::File),
        )],
    )
    .unwrap();

    assert_eq!(input.references()[0].span(), &(5..20));
    assert_eq!(
        input.references()[0]
            .workspace_reference()
            .unwrap()
            .relative_path(),
        "line\nbreak"
    );
}

// caller가 보이는 label만 다른 대상으로 바꿀 수 없도록 projection은 typed reference에서
// core가 생성한다. 같은 span에 그럴듯한 다른 이름을 놓아도 결합은 실패한다.
#[test]
fn visible_projection_cannot_be_mislabeled_for_another_reference() {
    let error = UserInput::with_references(
        "@harmless",
        vec![InputReference::workspace(
            0..9,
            workspace("secret", WorkspaceReferenceKind::File),
        )],
    )
    .unwrap_err();

    assert_eq!(error, UserInputError::ProjectionMismatch { index: 0 });
}

// public enum variant를 직접 만들어 helper constructor를 우회해도 validation이 typed
// reference에서 canonical Projection을 다시 계산하므로 spoofed label은 승인되지 않는다.
#[test]
fn direct_variant_construction_cannot_bypass_projection_binding() {
    let error = UserInput::with_references(
        "@harmless",
        vec![InputReference::Workspace {
            span: 0..9,
            projection: "@harmless".to_owned(),
            reference: workspace("secret", WorkspaceReferenceKind::File),
        }],
    )
    .unwrap_err();

    assert_eq!(error, UserInputError::ProjectionMismatch { index: 0 });
}

// 영속 기록이나 원격 frontend가 필수 selector metadata가 빈 reference를 만들더라도
// semantic input 경계가 이를 typed identity로 승인하지 않는다.
#[test]
fn missing_reference_identity_metadata_fails_closed() {
    let invalid = WorkspaceReference::new(
        "",
        "host:one",
        "workspace:one",
        "root:one",
        "src/lib.rs",
        WorkspaceReferenceKind::File,
    )
    .unwrap();
    let error = UserInput::with_references(
        "@src/lib.rs",
        vec![InputReference::workspace(0..11, invalid)],
    )
    .unwrap_err();

    assert_eq!(error, UserInputError::InvalidReferenceMetadata { index: 0 });
}

// 표시용 text/span은 그대로 두고 검증한 지침만 모델 입력에 포함한다. 이름·경로·제어문자도
// JSON 값으로 보존하며 명시적 스킬이 없는 입력은 기존 byte를 바꾸지 않는다.
#[test]
fn resolved_skill_preserves_visible_text_and_exact_model_instructions() {
    use super::ResolvedSkill;
    let selected = skill("review");
    let input = UserInput::with_references(
        "use $review",
        vec![InputReference::skill(4..11, selected.clone())],
    )
    .unwrap();
    let instructions = "# Review\nKeep \"quotes\" and 한글\u{0} intact";
    let resolved = input
        .clone()
        .with_resolved_skill(ResolvedSkill::new(selected, instructions).unwrap())
        .unwrap();
    assert_eq!(resolved.as_str(), input.as_str());
    assert_eq!(resolved.references(), input.references());
    assert_eq!(input.model_input(), input.as_str());
    let (visible, json) = resolved
        .model_input()
        .split_once("\n\nExplicit skill instructions (yo.skill-instructions/v1):\n")
        .unwrap();
    assert_eq!(visible, "use $review");
    assert_eq!(
        json,
        r##"{"instructions":"# Review\nKeep \"quotes\" and 한글\u0000 intact","name":"review","source":"/skills/review/SKILL.md"}"##
    );
    let snapshot: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(snapshot["instructions"], instructions);
    assert_eq!(snapshot["source"], "/skills/review/SKILL.md");
    assert_eq!(resolved.clone().into_string(), input.as_str());
    assert_eq!(resolved.clone().into_model_input(), resolved.model_input());
    assert_eq!(UserInput::new("plain\r\n").model_input(), "plain\r\n");
}

// 지침은 256 KiB까지 온전히 보존하고 첫 초과 byte·빈 본문·다른 스킬·중복 snapshot은 거절한다.
#[test]
fn resolved_skill_rejects_first_excess_and_wrong_selected_identity() {
    use super::ResolvedSkill;
    let selected = skill("review");
    let full = ResolvedSkill::new(
        selected.clone(),
        "x".repeat(ResolvedSkill::MAX_INSTRUCTION_BYTES),
    )
    .unwrap();
    assert_eq!(
        full.instructions().len(),
        ResolvedSkill::MAX_INSTRUCTION_BYTES
    );
    for text in [
        "x".repeat(ResolvedSkill::MAX_INSTRUCTION_BYTES + 1),
        " \n".to_owned(),
    ] {
        assert_eq!(
            ResolvedSkill::new(selected.clone(), text).unwrap_err(),
            UserInputError::InvalidSkillInstructions
        );
    }
    let input = UserInput::with_references(
        "$review",
        vec![InputReference::skill(0..7, selected.clone())],
    )
    .unwrap();
    let other = ResolvedSkill::new(skill("other"), "other instructions").unwrap();
    assert_eq!(
        input.clone().with_resolved_skill(other).unwrap_err(),
        UserInputError::SkillSnapshotMismatch
    );
    let resolved = input.with_resolved_skill(full.clone()).unwrap();
    assert_eq!(
        resolved.with_resolved_skill(full).unwrap_err(),
        UserInputError::SkillSnapshotMismatch
    );
}

// 저장 규약의 공백 집합 전체는 빈 지침으로 거절하며, 경계 밖의 zero-width space는
// 지침 원문으로 보존한다. Unicode 라이브러리의 공백 분류 변화에 의존하지 않는다.
#[test]
fn resolved_skill_uses_the_frozen_whitespace_profile() {
    use super::ResolvedSkill;
    let blank = "\u{9}\u{a}\u{b}\u{c}\u{d}\u{20}\u{85}\u{a0}\u{1680}\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}";
    assert_eq!(
        ResolvedSkill::new(skill("review"), blank).unwrap_err(),
        UserInputError::InvalidSkillInstructions
    );
    assert_eq!(
        ResolvedSkill::new(skill("review"), "\u{200b}")
            .unwrap()
            .instructions(),
        "\u{200b}"
    );
}
