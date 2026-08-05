use super::{CorpusRepository, methexis};

// summary 성공 출력은 전체 KnowledgeUnit 목록을 생략해 agent context를 아끼고,
// 검사 상태·authority·진단 개수만 남긴다.
#[test]
fn check_summary_returns_a_bounded_success_report() {
    let repository = CorpusRepository::without_active_checkpoint();
    let output = methexis()
        .current_dir(&repository.path)
        .args(["check", "--summary"])
        .output()
        .expect("run summarized check");

    assert!(output.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("summary output is JSON");
    assert_eq!(report["schema"], "methexis.check-summary/v1alpha1");
    assert_eq!(report["ok"], true);
    assert_eq!(report["authority"], "draft");
    assert_eq!(report["checks"].as_array().map(Vec::len), Some(4));
    assert_eq!(report["diagnostic_count"], 0);
    assert_eq!(report["units"], serde_json::json!([]));
    assert!(report.get("diagnostics").is_none());
}

// --unit은 지정한 KnowledgeUnit 한 개만 성공 보고서에 남겨 revision을 별도 탐색 없이
// 전달하며, --only와 --summary를 함께 사용해도 검사 계획은 그대로 유지한다.
#[test]
fn check_unit_keeps_only_the_requested_unit_in_a_summary() {
    let repository = CorpusRepository::without_active_checkpoint();
    let output = methexis()
        .current_dir(&repository.path)
        .args([
            "check",
            "--only",
            "authority",
            "--summary",
            "--unit=tui.surface.blank-cell",
        ])
        .output()
        .expect("run unit summary check");

    assert!(output.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("summary output is JSON");
    assert_eq!(report["requested_checks"], serde_json::json!(["authority"]));
    assert_eq!(
        report["executed_checks"],
        serde_json::json!(["records", "relations", "authority"])
    );
    let units = report["units"].as_array().expect("units are an array");
    assert_eq!(units.len(), 1);
    assert_eq!(units[0]["id"], "tui.surface.blank-cell");
    assert!(units[0]["revision"].as_str().is_some());
}

// 존재하지 않는 --unit 값과 중복된 --unit은 조용히 빈 성공 결과가 되지 않고
// 각각 구조화된 오류로 거부되어 잘못된 revision 전달을 막는다.
#[test]
fn check_unit_rejects_unknown_and_duplicate_ids() {
    let repository = CorpusRepository::without_active_checkpoint();
    let unknown = methexis()
        .current_dir(&repository.path)
        .args(["check", "--summary", "--unit", "tui.missing"])
        .output()
        .expect("run unknown unit check");
    assert_eq!(unknown.status.code(), Some(2));
    let error: serde_json::Value =
        serde_json::from_slice(&unknown.stderr).expect("error output is JSON");
    assert_eq!(error["error"]["code"], "unknown_check_unit");
    assert_eq!(
        error["error"]["affected_ids"],
        serde_json::json!(["tui.missing"])
    );

    let duplicate = methexis()
        .args(["check", "--unit", "tui.one", "--unit=tui.two"])
        .output()
        .expect("run duplicate unit check");
    assert_eq!(duplicate.status.code(), Some(2));
    let error: serde_json::Value =
        serde_json::from_slice(&duplicate.stderr).expect("error output is JSON");
    assert_eq!(error["error"]["code"], "invalid_check_selector");
}

// --unit은 단위의 approval·eligibility가 계산되는 bounded summary에서만 의미가 있으므로,
// 일반 전체 보고서나 records-only 검사와 섞으면 실행 전에 명시적인 사용 오류가 된다.
#[test]
fn check_unit_rejects_unbounded_and_pre_authority_combinations() {
    for args in [
        &["check", "--unit", "tui.surface.blank-cell"][..],
        &[
            "check",
            "--only",
            "records",
            "--summary",
            "--unit",
            "tui.surface.blank-cell",
        ][..],
        &[
            "check",
            "--only",
            "relations",
            "--summary",
            "--unit",
            "tui.surface.blank-cell",
        ][..],
    ] {
        let output = methexis()
            .args(args)
            .output()
            .expect("run incompatible unit check");

        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let error: serde_json::Value =
            serde_json::from_slice(&output.stderr).expect("error output is JSON");
        assert_eq!(error["error"]["code"], "invalid_check_selector");
        assert_eq!(
            error["error"]["affected_ids"],
            serde_json::json!(["tui.surface.blank-cell"])
        );
    }
}

// 실패 때는 --summary나 --unit이 진단을 숨기지 않고 기존 전체 보고서를 stderr에 보존한다.
#[test]
fn check_summary_preserves_the_full_failure_report() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("local-invalid");
    let output = methexis()
        .current_dir(fixture)
        .args(["check", "--summary", "--unit", "tui.any"])
        .output()
        .expect("run failing summarized check");

    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("failure output is JSON");
    assert_eq!(report["schema"], "methexis.check/v1alpha1");
    assert_eq!(report["ok"], false);
    assert!(
        report["diagnostics"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
}
