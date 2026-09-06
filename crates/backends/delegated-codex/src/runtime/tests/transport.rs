use yo_core::BackendFailureKind;

// Codex 실행 파일이 없으면 일반 protocol 오류가 아니라 설치 또는 PATH 문제로 대응할 수
// 있도록 Unavailable 실패를 즉시 반환하는지 확인한다.
#[test]
fn missing_codex_binary_is_reported_as_unavailable() {
    let config = super::super::CodexBackendConfig::new("/tmp")
        .with_executable("/definitely/missing/yo-codex");

    let failure = match super::super::CodexBackend::spawn(config) {
        Ok(_) => panic!("a missing executable must not initialize"),
        Err(failure) => failure,
    };

    assert_eq!(failure.kind(), BackendFailureKind::Unavailable);
}

// 상대 경로나 존재하지 않는 작업 디렉토리는 child process에 넘겨 모호한 spawn 오류로
// 바꾸지 않고 adapter 설정의 Initialization 실패로 먼저 설명하는지 확인한다.
#[test]
fn invalid_working_directory_is_rejected_before_spawn() {
    let config = super::super::CodexBackendConfig::new("relative/path");

    let failure = match super::super::CodexBackend::spawn(config) {
        Ok(_) => panic!("an invalid working directory must not initialize"),
        Err(failure) => failure,
    };

    assert_eq!(failure.kind(), BackendFailureKind::Initialization);
}
