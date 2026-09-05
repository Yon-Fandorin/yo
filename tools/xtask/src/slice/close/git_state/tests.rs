use super::accepted_commit_requires_close_metrics;
use crate::test_support::TestRepository;

// accepted commit의 marker가 재배치 전후 어느 경로에 있든 metrics 의무를 유지하고,
// marker가 없는 cutover 이전 commit만 legacy close plan을 허용한다.
#[test]
fn close_metrics_cutover_survives_marker_relocation() {
    for marker in [
        None,
        Some("tools/xtask/src/slice_close/metrics-cutover"),
        Some("tools/xtask/src/slice/close/metrics-cutover"),
    ] {
        let repository = TestRepository::new("close-metrics-marker-relocation");
        repository.write("base.txt", "base\n");
        repository.git(["add", "base.txt"]);
        if let Some(path) = marker {
            repository.write(path, "yo.slice-close-metrics/v1\n");
            repository.git(["add", path]);
        }
        repository.git(["commit", "--quiet", "-m", "test: metrics cutover"]);

        assert_eq!(
            accepted_commit_requires_close_metrics(&repository.path, "HEAD").unwrap(),
            marker.is_some(),
            "marker: {marker:?}"
        );
    }
}
