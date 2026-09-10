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

#[cfg(unix)]
mod codex_receive_bound {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    use yo_backend::transport::{JsonMessagePeer, JsonlPoll};
    use yo_core::BackendFailureKind;

    use super::super::super::{CodexBackendConfig, StdioPeer};

    const CODEX_MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;
    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(payload_bytes: usize) -> Self {
            let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "yo-codex-receive-bound-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir(&directory).unwrap();
            let executable = directory.join("fake-codex");
            let script = format!(
                r#"#!/bin/sh
printf '%s' '{{"value":"'
head -c {payload_bytes} /dev/zero | tr '\000' x
printf '%s\n' '"}}'
"#
            );
            fs::write(&executable, script).unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            Self(directory)
        }

        fn config(&self) -> CodexBackendConfig {
            CodexBackendConfig::new(&self.0)
                .with_executable(self.0.join("fake-codex"))
                .with_shutdown_timeout(Duration::from_secs(2))
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // Codex의 큰 echo payload는 기존 1 MiB 기본 transport보다 크지만, adapter가 지정한
    // 32 MiB inbound envelope 안에서는 JSON object로 끝까지 수신되어야 합니다.
    #[test]
    fn accepts_a_codex_payload_above_the_default_receive_bound() {
        let fixture = Fixture::new(1024 * 1024 + 1);
        let mut peer = StdioPeer::spawn(&fixture.config()).unwrap();
        let message = peer.receive(Duration::from_secs(5)).unwrap();
        let JsonlPoll::Message(message) = message else {
            panic!("the bounded Codex peer must receive the large JSON payload");
        };
        assert_eq!(message["value"].as_str().unwrap().len(), 1024 * 1024 + 1);
        peer.shutdown().unwrap();
    }

    // 32 MiB payload의 첫 초과 byte는 JSON 값으로 publish되지 않고 reader protocol failure로
    // 닫혀야 하며, Codex adapter가 1 MiB 기본값으로 되돌아가지 않았는지도 함께 확인합니다.
    #[test]
    fn rejects_the_first_byte_above_the_codex_receive_bound() {
        let prefix = br#"{"value":""#;
        let suffix = br#""}"#;
        let payload_bytes = CODEX_MAX_MESSAGE_BYTES - prefix.len() - suffix.len() + 1;
        let fixture = Fixture::new(payload_bytes);
        let mut peer = StdioPeer::spawn(&fixture.config()).unwrap();
        let failure = peer
            .receive(Duration::from_secs(10))
            .expect_err("the first byte above the Codex JSONL bound must fail");
        assert_eq!(failure.kind(), BackendFailureKind::Protocol);
        let _ = peer.shutdown();
    }
}
