use std::{ffi::OsString, fs, process::ExitCode};

use super::run;
use crate::test_support::TestDirectory;

// 형식이 깨진 요청은 stdout을 비워 둔 채 stderr로 malformed_request 구조화 실패를 보고한다.
#[test]
fn malformed_request_leaves_stdout_empty() {
    let directory = TestDirectory::new("malformed-request");
    let request = directory.path().join("request.json");
    fs::write(&request, b"{").expect("malformed request");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let code = run(
        ["discover".into(), request.as_os_str().to_owned()],
        &mut stdout,
        &mut stderr,
    )
    .expect("CLI completes");

    assert_failure(code, &stdout, &stderr, "malformed_request");
}

// query도 anchor도 없는 빈 요청은 catalog 수집 전에 empty_discovery_request로 실패한다.
#[test]
fn empty_request_fails_before_catalog_capture() {
    let directory = TestDirectory::new("empty-request");
    let request = directory.path().join("request.json");
    fs::write(
        &request,
        br#"{"schema":"librarian.discovery-request/v1alpha1"}"#,
    )
    .expect("empty request");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let code = run(
        ["discover".into(), request.as_os_str().to_owned()],
        &mut stdout,
        &mut stderr,
    )
    .expect("CLI completes");

    assert_failure(code, &stdout, &stderr, "empty_discovery_request");
}

// 깨진 catalog 레코드가 있으면 부분 candidate를 보내지 않고 invalid_catalog_record로 실패한다.
#[test]
fn invalid_catalog_never_emits_partial_candidates() {
    let directory = TestDirectory::new("invalid-catalog");
    let knowledge = directory.path().join("methexis/knowledge");
    fs::create_dir_all(&knowledge).expect("knowledge directory");
    fs::create_dir_all(directory.path().join("methexis/owners")).expect("owner directory");
    fs::create_dir_all(directory.path().join("methexis/sources")).expect("source directory");
    fs::write(knowledge.join("broken.md"), b"not frontmatter").expect("broken record");
    let request = directory.path().join("request.json");
    fs::write(
        &request,
        br#"{"schema":"librarian.discovery-request/v1alpha1","query":"anything"}"#,
    )
    .expect("request");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let arguments = vec![
        OsString::from("discover"),
        OsString::from("--repository"),
        directory.path().as_os_str().to_owned(),
        request.into_os_string(),
    ];

    let code = run(arguments, &mut stdout, &mut stderr).expect("CLI completes");

    assert_failure(code, &stdout, &stderr, "invalid_catalog_record");
}

// 크기 상한을 초과한 요청은 request_too_large 코드의 구조화된 실패로 거부된다.
#[test]
fn oversized_request_is_a_structured_failure() {
    let directory = TestDirectory::new("oversized-request");
    let request = directory.path().join("request.json");
    fs::write(&request, vec![b'x'; 256 * 1024 + 1]).expect("oversized request");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let code = run(
        ["discover".into(), request.as_os_str().to_owned()],
        &mut stdout,
        &mut stderr,
    )
    .expect("CLI completes");

    assert_failure(code, &stdout, &stderr, "request_too_large");
}

// 알 수 없는 명령어도 unknown_command 코드의 구조화된 실패로 응답한다.
#[test]
fn unknown_command_is_structured() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let code = run(["wat"], &mut stdout, &mut stderr).expect("CLI completes");

    assert_failure(code, &stdout, &stderr, "unknown_command");
}

fn assert_failure(code: ExitCode, stdout: &[u8], stderr: &[u8], expected_code: &str) {
    assert_eq!(code, ExitCode::from(2));
    assert!(stdout.is_empty());
    let failure: serde_json::Value = serde_json::from_slice(stderr).expect("failure JSON");
    assert_eq!(failure["error"]["code"], expected_code);
}
