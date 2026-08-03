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
