use super::{Authority, CheckReport, handle_staged_check_output};

const DRAFT_REPORT: &str = r#"{
    "ok": true,
    "authority": "draft",
    "checks": [{"check": "records", "status": "passed"}],
    "units": [{"id": "secretly-large-unit", "body": "content must not escape"}],
    "diagnostics": []
}"#;

// 보고서의 authority 문자열이 draft와 prospective의 서로 다른 타입 값으로
// 역직렬화되는지 직접 비교해 호출자가 두 상태를 혼동하지 않게 한다.
#[test]
fn trusts_only_the_authority_in_the_single_methexis_report() {
    let prospective: CheckReport = serde_json::from_str(
        r#"{"ok":true,"authority":"prospective","checks":[],"units":[],"diagnostics":[]}"#,
    )
    .unwrap();
    let ordinary: CheckReport = serde_json::from_str(DRAFT_REPORT).unwrap();

    assert_eq!(prospective.authority, Authority::Prospective);
    assert_eq!(ordinary.authority, Authority::Draft);
}

// 성공 처리 경계가 전체 unit 본문과 식별자를 전달하지 않고 authority와 개수만
// stdout에 쓰며, 성공 stderr는 그대로 전달하는지 바이트 단위로 확인한다.
#[test]
fn successful_check_forwards_only_bounded_summary_and_stderr() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let authority = handle_staged_check_output(
        true,
        "exit status: 0",
        DRAFT_REPORT.as_bytes(),
        b"compiler warning\n",
        &mut stdout,
        &mut stderr,
    )
    .unwrap();

    assert_eq!(
        stdout,
        b"{\"schema\":\"yo.methexis-stage-summary/v1\",\"ok\":true,\"authority\":\"draft\",\"checks\":1,\"units\":1,\"diagnostics\":0}\n"
    );
    assert_eq!(stderr, b"compiler warning\n");
    assert_eq!(authority, Authority::Draft);
    assert!(
        !stdout
            .windows(b"secretly-large-unit".len())
            .any(|bytes| bytes == b"secretly-large-unit")
    );
}

// 실패 처리 경계가 Methexis의 stdout과 stderr 진단을 생략하거나 요약으로 바꾸지
// 않고 그대로 전달한 뒤 원래 종료 상태가 포함된 오류를 반환하는지 확인한다.
#[test]
fn failed_check_preserves_both_diagnostic_streams() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let error = handle_staged_check_output(
        false,
        "exit status: 2",
        b"structured stdout\n",
        b"structured stderr\n",
        &mut stdout,
        &mut stderr,
    )
    .unwrap_err();

    assert_eq!(stdout, b"structured stdout\n");
    assert_eq!(stderr, b"structured stderr\n");
    assert_eq!(
        error,
        "staged Methexis validation failed with exit status: 2"
    );
}
