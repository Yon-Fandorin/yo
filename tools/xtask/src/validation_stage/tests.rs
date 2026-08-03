use super::{Authority, StageReport, handle_staged_check_output};

const DRAFT_REPORT: &str = r#"{
    "schema": "methexis.check/v1alpha1",
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
    let prospective: StageReport = serde_json::from_str(
        r#"{"schema":"methexis.prospective-activation/v1alpha1","ok":true,"authority":"prospective","affected_ids":[]}"#,
    )
    .unwrap();
    let ordinary: StageReport = serde_json::from_str(DRAFT_REPORT).unwrap();

    assert_eq!(
        prospective.summary().unwrap().authority,
        Authority::Prospective
    );
    assert_eq!(ordinary.summary().unwrap().authority, Authority::Draft);
}

// 실제 활성화 보고서는 일반 검사 보고서의 checks/units/diagnostics 배열을 갖지
// 않는다. 전용 schema로 이를 구분해 affected_ids 개수만 bounded summary로 내보낸다.
#[test]
fn prospective_activation_uses_its_own_report_shape() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let authority = handle_staged_check_output(
        true,
        "exit status: 0",
        br#"{"schema":"methexis.prospective-activation/v1alpha1","ok":true,"operation":"check_staged_activation","authority":"prospective","affected_ids":["one","two"]}"#,
        b"",
        &mut stdout,
        &mut stderr,
    )
    .unwrap();

    assert_eq!(
        stdout,
        b"{\"schema\":\"yo.methexis-stage-summary/v1\",\"ok\":true,\"authority\":\"prospective\",\"checks\":1,\"units\":2,\"diagnostics\":0}\n"
    );
    assert!(stderr.is_empty());
    assert_eq!(authority, Authority::Prospective);
}

// schema와 authority 조합이 뒤바뀐 보고서는 성공 프로세스라도 신뢰하지 않아
// 도구 버전 불일치나 잘못 연결된 출력이 검증 통과로 오인되지 않게 한다.
#[test]
fn rejects_schema_authority_mismatch() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let error = handle_staged_check_output(
        true,
        "exit status: 0",
        br#"{"schema":"methexis.prospective-activation/v1alpha1","ok":true,"authority":"draft","affected_ids":[]}"#,
        b"",
        &mut stdout,
        &mut stderr,
    )
    .unwrap_err();

    assert_eq!(
        error,
        "staged Methexis report schema and authority disagree"
    );
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
