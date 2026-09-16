use std::ffi::OsString;

use super::parse_prepare_approval;

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

// 분리형 --reviewer 뒤의 구조 옵션이나 알 수 없는 장기 옵션은 reviewer 값이 아니라
// 누락된 값과 독립 옵션으로 취급해 인자 단계에서 닫습니다.
#[test]
fn separated_reviewer_rejects_flag_like_values() {
    for values in [
        &["manifest.json", "--reviewer", "--replace-current"][..],
        &["manifest.json", "--reviewer", "--reviewer=owner"][..],
        &["manifest.json", "--reviewer", "--unknown"][..],
    ] {
        assert!(parse_prepare_approval(&args(values)).is_err(), "{values:?}");
    }
}

// 선행 dash를 실제 OwnerId 문자로 전달해야 하는 명시적 equals form은 파서에서
// 보존하고, 존재 여부는 기존 prepare 서비스 검증에 맡깁니다.
#[test]
fn equals_reviewer_preserves_a_leading_dash_literal() {
    let parsed =
        parse_prepare_approval(&args(&["manifest.json", "--reviewer=--literal-owner"])).unwrap();

    assert_eq!(parsed.reviewer, "--literal-owner");
    assert!(!parsed.replace_current);
}

// 동일 옵션의 분리형·equals form 혼용은 마지막 값을 덮어쓰지 않고 기존 중복 오류를
// 유지합니다.
#[test]
fn duplicate_reviewer_forms_remain_invalid() {
    assert!(
        parse_prepare_approval(&args(&[
            "manifest.json",
            "--reviewer",
            "owner",
            "--reviewer=other",
        ]))
        .is_err()
    );
}

// canonical form은 ID와 revision을 한 쌍으로 요구하고 positional manifest와 섞이지 않는다.
#[test]
fn canonical_target_requires_an_exact_revision_and_no_manifest() {
    let parsed = parse_prepare_approval(&args(&[
        "--canonical",
        "tui.relocated",
        "--revision=sha256:1234",
        "--reviewer",
        "tui-architecture",
    ]))
    .unwrap();
    assert_eq!(
        parsed.target,
        super::PrepareApprovalTarget::Canonical {
            knowledge_id: "tui.relocated".to_owned(),
            revision: "sha256:1234".to_owned(),
        }
    );
    assert!(
        parse_prepare_approval(&args(&[
            "manifest.json",
            "--canonical=tui.relocated",
            "--revision=sha256:1234",
            "--reviewer=owner",
        ]))
        .is_err()
    );
}
