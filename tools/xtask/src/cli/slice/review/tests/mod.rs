use super::{
    cost_report_usage, review_continuation_preflight_usage, review_delivery_usage,
    review_delta_usage, review_egress_usage, review_packet_usage, review_prepare_usage,
    review_target_admission_usage,
};
use crate::cli::run;

// 비용 owner report는 source request와 새 output 경로를 모두 요구하며 추가 인자를
// 무시한 채 다른 artifact를 만들지 않는다.
#[test]
fn cost_report_requires_request_and_output() {
    let missing = run(["slice", "cost-report", "request.json"].map(Into::into)).unwrap_err();
    let extra = run([
        "slice",
        "cost-report",
        "request.json",
        "output.json",
        "extra.json",
    ]
    .map(Into::into))
    .unwrap_err();

    assert_eq!(missing, cost_report_usage());
    assert_eq!(extra, cost_report_usage());
}

// review packet 생성도 정확히 한 versioned request만 받아 입력 일부가
// 누락되거나 추가 인자가 조용히 무시된 채 다른 review identity가 생기지 않는다.
#[test]
fn review_packet_requires_exactly_one_request() {
    let missing = run(["slice", "review-packet"].map(Into::into)).unwrap_err();
    let extra =
        run(["slice", "review-packet", "request.json", "extra.json"].map(Into::into)).unwrap_err();

    assert_eq!(missing, review_packet_usage());
    assert_eq!(extra, review_packet_usage());
}

// 통합 review preparation도 한 versioned request만 받아 semantic 입력과 target을
// 추가 argv로 교체하거나 두 번째 준비 경로를 암묵적으로 만들지 않습니다.
#[test]
fn review_prepare_requires_exactly_one_request() {
    let missing = run(["slice", "review-prepare"].map(Into::into)).unwrap_err();
    let extra =
        run(["slice", "review-prepare", "request.json", "extra.json"].map(Into::into)).unwrap_err();

    assert_eq!(missing, review_prepare_usage());
    assert_eq!(extra, review_prepare_usage());
}

// preflight도 publication과 같은 하나의 versioned request를 요구하여, request가
// 빠지거나 추가 입력이 있는 호출을 준비 완료로 오인하지 않는다.
#[test]
fn review_packet_preflight_requires_exactly_one_request() {
    let missing = run(["slice", "review-packet", "--preflight"].map(Into::into)).unwrap_err();
    let extra = run([
        "slice",
        "review-packet",
        "--preflight",
        "request.json",
        "extra.json",
    ]
    .map(Into::into))
    .unwrap_err();

    assert_eq!(missing, review_packet_usage());
    assert_eq!(extra, review_packet_usage());
}

// readiness도 publication과 같은 request 하나만 받아, 빠진 입력이나 서로 겹친 mode
// flag를 준비 완료로 처리하지 않는다.
#[test]
fn review_packet_readiness_requires_exactly_one_request() {
    let missing = run(["slice", "review-packet", "--check-readiness"].map(Into::into)).unwrap_err();
    let extra = run([
        "slice",
        "review-packet",
        "--check-readiness",
        "request.json",
        "extra.json",
    ]
    .map(Into::into))
    .unwrap_err();
    let overlapping = run([
        "slice",
        "review-packet",
        "--check-readiness",
        "--preflight",
        "request.json",
    ]
    .map(Into::into))
    .unwrap_err();

    assert_eq!(missing, review_packet_usage());
    assert_eq!(extra, review_packet_usage());
    assert_eq!(overlapping, review_packet_usage());
}

// finding-resolution delta도 prior review identity와 disposition을 담은 정확히 한
// versioned request만 받아 누락되거나 추가된 인자를 조용히 무시하지 않는다.
#[test]
fn review_delta_requires_exactly_one_request() {
    let missing = run(["slice", "review-delta"].map(Into::into)).unwrap_err();
    let extra =
        run(["slice", "review-delta", "request.json", "extra.json"].map(Into::into)).unwrap_err();

    assert_eq!(missing, review_delta_usage());
    assert_eq!(extra, review_delta_usage());
}

// external review egress preflight도 packet과 standing authorization을 결속한 정확히
// 한 request만 받아 추가 입력으로 route나 권한을 넓히지 않는다.
#[test]
fn review_egress_requires_exactly_one_request() {
    let missing = run(["slice", "review-egress"].map(Into::into)).unwrap_err();
    let extra =
        run(["slice", "review-egress", "request.json", "extra.json"].map(Into::into)).unwrap_err();

    assert_eq!(missing, review_egress_usage());
    assert_eq!(extra, review_egress_usage());
}

// target admission도 정확히 한 versioned request만 받아 호출자가 argv로 다른
// target이나 상태 경로를 덧붙이지 못하게 합니다.
#[test]
fn review_target_admission_requires_exactly_one_request() {
    let missing = run(["slice", "review-target-admission"].map(Into::into)).unwrap_err();
    let extra = run([
        "slice",
        "review-target-admission",
        "request.json",
        "extra.json",
    ]
    .map(Into::into))
    .unwrap_err();

    assert_eq!(missing, review_target_admission_usage());
    assert_eq!(extra, review_target_admission_usage());
}

// 실제 외부 effect를 소유하는 review-deliver도 versioned request 하나만 받아
// 추가 argv가 route, packet, retry 또는 output 경계를 넓히지 못하게 한다.
#[test]
fn review_delivery_requires_exactly_one_request() {
    let missing = run(["slice", "review-deliver"].map(Into::into)).unwrap_err();
    let extra =
        run(["slice", "review-deliver", "request.json", "extra.json"].map(Into::into)).unwrap_err();

    assert_eq!(missing, review_delivery_usage());
    assert_eq!(extra, review_delivery_usage());
    let finalize_missing =
        run(["slice", "review-deliver", "finalize"].map(Into::into)).unwrap_err();
    assert_eq!(finalize_missing, review_delivery_usage());
}

// finding-resolution preflight도 egress와 Session root를 담은 closed request 하나만
// 받아 추가 argv가 terminal input이나 resume 대상을 넓히지 못하게 합니다.
#[test]
fn review_continuation_preflight_requires_exactly_one_request() {
    let missing = run(["slice", "review-continuation-preflight"].map(Into::into)).unwrap_err();
    let extra = run([
        "slice",
        "review-continuation-preflight",
        "request.json",
        "extra.json",
    ]
    .map(Into::into))
    .unwrap_err();

    assert_eq!(missing, review_continuation_preflight_usage());
    assert_eq!(extra, review_continuation_preflight_usage());
}
