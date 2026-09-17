use super::*;

// 모든 증거와 정확 승인이 같은 후보와 diff에 결속되면 실행을 반복하지 않고
// 통합이 유일한 다음 행동임을 보고하고, 커밋에 넣을 exact trailer도 함께 만든다.
#[test]
fn ready_candidate_reports_integrate_and_exact_trailers() {
    let fixture = Fixture::new();
    let result = fixture.evaluate().unwrap();

    assert_eq!(result.next_action, "integrate");
    assert_eq!(result.status, "ready");
    assert_eq!(result.candidate_commit, fixture.candidate);
    assert_eq!(result.diff_hash, fixture.diff_hash);
    assert_eq!(result.commit_trailers.len(), 4);
    assert!(
        result
            .commit_trailers
            .iter()
            .all(|trailer| !trailer.starts_with("Review-Coverage:")
                || trailer.ends_with(&fixture.diff_hash))
    );
}
