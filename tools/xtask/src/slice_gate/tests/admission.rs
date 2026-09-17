use super::*;

// 검증 증거가 아직 없으면 리뷰나 승인을 다시 요구하지 않고 검증만 다음 행동으로
// 선택하여 coordinator가 가장 이른 미충족 게이트부터 진행할 수 있게 한다.
#[test]
fn missing_validation_reports_validate() {
    let mut fixture = Fixture::new();
    fixture.request["validation_evidence"] = json!([]);

    assert_eq!(fixture.evaluate().unwrap().next_action, "validate");
}

// 필요한 렌즈 중 하나가 빠진 정확 후보는 검증이 통과했어도 준비 완료가 아니며,
// 기존에 끝난 렌즈를 반복하지 않고 리뷰만 다음 행동으로 표시한다.
#[test]
fn missing_review_reports_review() {
    let mut fixture = Fixture::new();
    fixture.request["review_evidence"]
        .as_array_mut()
        .unwrap()
        .pop();

    assert_eq!(fixture.evaluate().unwrap().next_action, "review");
}

// 검증과 리뷰가 완전해도 승인 객체가 없으면 human-attention 후보를 통합 가능으로
// 오인하지 않고 승인 하나만 남았음을 표시한다.
#[test]
fn missing_approval_reports_approve() {
    let mut fixture = Fixture::new();
    fixture.request["approval"] = Value::Null;

    assert_eq!(fixture.evaluate().unwrap().next_action, "approve");
}

// human approval은 정확히 두 segment인 human/<identity>만 받아 빈 identity나
// 추가 path segment가 standing/exact authorization으로 오인되지 않게 한다.
#[test]
fn malformed_human_approval_authority_fails_closed() {
    for authority in ["human/", "human/a/b"] {
        let mut fixture = Fixture::new();
        fixture.request["approval"]["authority"] = json!(authority);

        assert!(
            fixture
                .evaluate()
                .unwrap_err()
                .contains("exactly human/<identity>")
        );
    }
}

// request의 최상위 후보가 현재 clean HEAD와 다르면 하위 증거를 읽기 전에 거부하여
// 이전 후보의 녹색 결과가 새 후보로 승계되지 않게 한다.
#[test]
fn stale_candidate_fails_closed() {
    let mut fixture = Fixture::new();
    fixture.request["candidate_commit"] = json!("0000000000000000000000000000000000000000");

    assert!(fixture.evaluate().unwrap_err().contains("request is stale"));
}

// tools Rust 변경이 요구하는 path-derived 최소 렌즈를 planner가 request에서 빼도
// 게이트가 기존 impact 규칙을 재사용해 누락을 즉시 거부한다.
#[test]
fn request_cannot_omit_path_derived_lens() {
    let mut fixture = Fixture::new();
    fixture.request["required_lenses"] = json!(["fresh-context"]);

    assert!(fixture.evaluate().unwrap_err().contains("code-quality"));
}

// 이미 허용된 기계적 후속 작업은 명시된 human-origin standing authorization으로
// 준비 완료가 될 수 있어 exact merge 승인 문구를 불필요하게 다시 만들지 않는다.
#[test]
fn routine_candidate_accepts_standing_authorization() {
    let mut fixture = Fixture::new();
    fixture.request["risk"] = json!({
        "classification": "routine",
        "rationale": "mechanical follow-through under an accepted contract"
    });
    fixture.request["approval"] = json!({
        "kind": "standing_routine",
        "authority": "human/yon",
        "scope": "routine exact-contract implementation",
        "candidate_commit": null,
        "diff_hash": null
    });

    assert_eq!(fixture.evaluate().unwrap().next_action, "integrate");
}

// 실행하지 못한 필수 환경이 남은 후보는 routine 자동 경로로 흘리지 않고
// human-attention으로 분류해 사람이 그 공백을 명시적으로 판단하게 한다.
#[test]
fn routine_candidate_rejects_unverified_environment() {
    let mut fixture = Fixture::new();
    fixture.request["risk"] = json!({
        "classification": "routine",
        "rationale": "mechanical follow-through"
    });
    fixture.request["known_unverified_environments"] = json!(["registered macOS runner"]);

    assert!(
        fixture
            .evaluate()
            .unwrap_err()
            .contains("routine risk cannot retain known unverified")
    );
}
