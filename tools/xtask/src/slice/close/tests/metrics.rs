use super::{CloseFixture, close_metrics, output};
use crate::slice::close::{apply, build_plan, identity};

// 표준 close-metrics 파일이 없거나 다른 candidate/accepted commit을 가리키면
// plan이 수치 기록을 추측하거나 cleanup 계획을 발행하기 전에 각각 거부한다.
#[test]
fn plan_requires_metrics_for_the_exact_slice_transition() {
    let missing = CloseFixture::new();
    std::fs::remove_file(&missing.metrics_path).unwrap();
    let missing_error = build_plan(&missing.repository.path, "sample").unwrap_err();
    assert!(missing_error.contains("cannot open Slice close metrics"));

    let stale = CloseFixture::new();
    let candidate = output(&stale.slice_worktree, &["rev-parse", "HEAD"]);
    std::fs::write(
        &stale.metrics_path,
        close_metrics(&candidate, "0000000000000000000000000000000000000000"),
    )
    .unwrap();
    let stale_error = build_plan(&stale.repository.path, "sample").unwrap_err();
    assert!(stale_error.contains("exact candidate and accepted commit"));
}

// Cargo validation을 parallel로 선언하거나 finding 처분 합계가 보고 수와 다르면
// 보기 좋은 JSON이어도 실행 레인과 review 결과가 모순되므로 plan이 거부한다.
#[test]
fn plan_rejects_internally_inconsistent_lane_and_review_metrics() {
    let lane = CloseFixture::new();
    let mut lane_metrics: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&lane.metrics_path).unwrap()).unwrap();
    lane_metrics["execution_lanes"][0]["mode"] = serde_json::json!("parallel");
    lane_metrics["execution_lanes"][0]["max_concurrency"] = serde_json::json!(2);
    std::fs::write(
        &lane.metrics_path,
        serde_json::to_vec_pretty(&lane_metrics).unwrap(),
    )
    .unwrap();
    let lane_error = build_plan(&lane.repository.path, "sample").unwrap_err();
    assert!(
        lane_error.contains("Cargo validation and integration must use serialized execution lanes"),
        "{lane_error}"
    );

    let review = CloseFixture::new();
    let mut review_metrics: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&review.metrics_path).unwrap()).unwrap();
    review_metrics["review"]["findings"]["reported"] = serde_json::json!(1);
    std::fs::write(
        &review.metrics_path,
        serde_json::to_vec_pretty(&review_metrics).unwrap(),
    )
    .unwrap();
    let review_error = build_plan(&review.repository.path, "sample").unwrap_err();
    assert!(review_error.contains("finding totals do not reconcile"));
}

// plan이 고정한 뒤 유효한 metrics에 공백만 추가해도 파일 hash가 달라지므로
// apply는 worktree·contract·branch를 하나도 지우기 전에 중단한다.
#[test]
fn apply_rejects_metrics_byte_drift_before_cleanup() {
    let fixture = CloseFixture::new();
    let plan = fixture.plan();
    fixture.write_plan(&plan);
    let mut metrics = std::fs::read(&fixture.metrics_path).unwrap();
    metrics.push(b'\n');
    std::fs::write(&fixture.metrics_path, metrics).unwrap();

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("metrics changed after planning"));
    assert!(fixture.slice_worktree.exists());
    assert!(fixture.contract_path.exists());
}

// caller가 다른 유효한 metrics 파일로 plan 경로와 identity를 함께 바꿔도 표준
// coordination 경계를 독립적으로 재도출하여 외부 기록으로 cleanup을 승인하지 않는다.
#[test]
fn apply_rejects_rehashed_nonstandard_metrics_path() {
    let fixture = CloseFixture::new();
    let alternate = crate::test_support::unique_path("alternate-close-metrics.json");
    std::fs::copy(&fixture.metrics_path, &alternate).unwrap();
    let mut plan = fixture.plan();
    plan.close_metrics.as_mut().unwrap().path = alternate.clone();
    plan.plan_id = identity(&plan).unwrap();
    fixture.write_plan(&plan);

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("standard coordination path"));
    assert!(fixture.slice_worktree.exists());
    assert!(fixture.contract_path.exists());
    std::fs::remove_file(alternate).unwrap();
}

// 사람의 exact-patch review처럼 packet 발행이 없는 경우에는 packet 합계를 0으로
// 기록할 수 있고, 실행하지 못한 환경 검사는 runs 0과 환경 설명으로 보존한다.
#[test]
fn plan_accepts_packetless_review_and_explicit_unverified_environment() {
    let fixture = CloseFixture::new();
    let mut metrics: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&fixture.metrics_path).unwrap()).unwrap();
    metrics["review_packets"] = serde_json::json!({
        "publication_count": 0,
        "total_managed_tokens": 0,
        "largest_sections": [],
        "reused_inputs": []
    });
    metrics["validation"][0]["runs"] = serde_json::json!(0);
    metrics["validation"][0]["status"] = serde_json::json!("unverified");
    metrics["known_unverified_environments"] = serde_json::json!(["macOS host was unavailable"]);
    std::fs::write(
        &fixture.metrics_path,
        serde_json::to_vec_pretty(&metrics).unwrap(),
    )
    .unwrap();

    let plan = fixture.plan();

    assert_eq!(
        plan.close_metrics.as_ref().unwrap().path,
        fixture.metrics_path
    );
}

// v4 이전에 발행된 v3 plan은 close-metrics 필드가 없어도 당시의 retained 목록과
// identity를 그대로 검증하여 중단된 안전한 cleanup을 계속할 수 있다.
#[test]
fn apply_preserves_legacy_v3_plan_compatibility() {
    let fixture = CloseFixture::new();
    let mut plan = fixture.plan();
    plan.schema = "yo.slice-close-plan/v3".to_owned();
    plan.close_metrics = None;
    plan.retained_coordination_paths = vec![fixture.metrics_path.clone()];
    plan.coordination_cleanup_paths.clear();
    plan.effects.remove_coordination_directory = false;
    plan.plan_id = identity(&plan).unwrap();
    fixture.write_plan(&plan);

    apply(&fixture.repository.path, &fixture.plan_path).unwrap();

    assert!(!fixture.slice_worktree.exists());
    assert!(!fixture.contract_path.exists());
    assert!(fixture.metrics_path.exists());
}

// metrics cutover marker가 accepted commit tree에 있으면 current v1alpha1 plan을 v2나
// v3로 낮추고 identity를 다시 계산해도 legacy provenance가 아니므로 거부한다.
#[test]
fn apply_rejects_rehashed_legacy_downgrade_after_metrics_cutover() {
    for schema in ["yo.slice-close-plan/v2", "yo.slice-close-plan/v3"] {
        let fixture = CloseFixture::new_after_metrics_cutover();
        let mut plan = fixture.plan();
        plan.schema = schema.to_owned();
        plan.close_metrics = None;
        plan.coordination_cleanup_paths.clear();
        plan.effects.remove_coordination_directory = false;
        if schema.ends_with("/v2") {
            plan.retained_coordination_paths.clear();
        } else {
            plan.retained_coordination_paths = vec![fixture.metrics_path.clone()];
        }
        plan.plan_id = identity(&plan).unwrap();
        fixture.write_plan(&plan);

        let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

        assert!(error.contains("at or after the close-metrics cutover require v4 or newer"));
        assert!(fixture.slice_worktree.exists());
        assert!(fixture.contract_path.exists());
    }
}
