use std::{
    env,
    io::Cursor,
    process, thread,
    time::{Duration, Instant},
};

use super::{
    DEFAULT_MAX_JSONL_MESSAGE_BYTES,
    config::StdioJsonlConfig,
    peer::{JsonlPoll, StdioJsonlPeer},
    reader::read_jsonl_message,
};
use crate::BackendFailureKind;

// newline 없는 oversized 출력도 제한보다 한 byte만 읽고 실패해 child process가 reader
// memory를 무제한으로 늘리지 못하게 합니다.
#[test]
fn rejects_an_oversized_message_while_reading() {
    let input = vec![b'x'; DEFAULT_MAX_JSONL_MESSAGE_BYTES + 1];
    let error = read_jsonl_message(
        &mut Cursor::new(input),
        "fixture",
        DEFAULT_MAX_JSONL_MESSAGE_BYTES,
    )
    .unwrap_err();

    assert!(error.contains("exceeds"));
    assert!(error.contains(&DEFAULT_MAX_JSONL_MESSAGE_BYTES.to_string()));
}

// LF와 CRLF delimiter는 payload 제한에 포함되지 않으므로 정확히 limit 크기인 JSON은
// 두 줄바꿈 형식 모두에서 허용되어야 합니다.
#[test]
fn accepts_a_json_payload_exactly_at_the_message_limit() {
    let maximum_message_bytes = 32;
    let empty_payload = r#"{"value":""}"#;
    let padding = "x".repeat(maximum_message_bytes - empty_payload.len());
    let payload = format!(r#"{{"value":"{padding}"}}"#);
    assert_eq!(payload.len(), maximum_message_bytes);

    for delimiter in ["\n", "\r\n"] {
        let mut input = Cursor::new(format!("{payload}{delimiter}"));
        let value = read_jsonl_message(&mut input, "fixture", maximum_message_bytes)
            .unwrap()
            .unwrap();

        assert_eq!(value["value"], padding);
    }
}

// blank JSONL 행을 건너뛰어도 다음 bounded JSON object가 손실되지 않는지 검증합니다.
#[test]
fn skips_blank_lines_without_losing_the_next_message() {
    let mut input = Cursor::new(b" \r\n{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n");

    let value = read_jsonl_message(&mut input, "fixture", DEFAULT_MAX_JSONL_MESSAGE_BYTES)
        .unwrap()
        .unwrap();

    assert_eq!(value["id"], 1);
}

// JSONL read limit에 CRLF와 sentinel byte를 더할 수 없는 설정은 spawn 전에 거부되어야
// release build에서도 size 계산이 wrap되지 않습니다.
#[test]
fn rejects_a_maximum_message_size_that_cannot_add_a_sentinel_byte() {
    let config = StdioJsonlConfig::new("fixture", "fixture", "/bin/false", "/")
        .with_maximum_message_bytes(usize::MAX);

    let error = config.validate().unwrap_err();

    assert_eq!(error.kind(), BackendFailureKind::Initialization);
}

#[cfg(unix)]
mod unix {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_SCRIPT: AtomicU64 = AtomicU64::new(1);

    fn spawn_fixture(label: &str, body: &str) -> (PathBuf, StdioJsonlPeer) {
        let suffix = NEXT_SCRIPT.fetch_add(1, Ordering::Relaxed);
        let directory = env::temp_dir().join(format!(
            "yo-backend-stdio-jsonl-{label}-{}-{suffix}",
            process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let script = directory.join("fake-backend");
        fs::write(&script, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let config = StdioJsonlConfig::new("fixture", "fixture", &script, &directory)
            .with_shutdown_timeout(Duration::from_secs(1));
        (directory, StdioJsonlPeer::spawn(config).unwrap())
    }

    fn remove_fixture(directory: &PathBuf) {
        let _ = fs::remove_file(directory.join("fake-backend"));
        let _ = fs::remove_dir(directory);
    }

    // 실제 pipe-backed child가 stdin request를 받은 뒤 stdout JSONL response를
    // 돌려주는 왕복 경계를 검증해 protocol fixture가 transport를 우회하지 않게 합니다.
    #[test]
    fn exchanges_one_json_message_with_a_child_process() {
        let (directory, mut peer) = spawn_fixture(
            "round-trip",
            "IFS= read -r request\nprintf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}'\n",
        );

        peer.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {},
        }))
        .unwrap();
        let message = peer.receive(Duration::from_secs(5)).unwrap();

        peer.shutdown().unwrap();
        remove_fixture(&directory);
        assert_eq!(
            message,
            JsonlPoll::Message(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {},
            }))
        );
    }

    // bounded queue가 가득 차 stdout reader가 대기해도 shutdown이 queue를 비우고 child를
    // 교착 없이 회수하는지 검증합니다.
    #[test]
    fn shutdown_drains_a_reader_blocked_on_the_full_queue() {
        let (directory, mut peer) = spawn_fixture(
            "full-queue",
            "i=0\nwhile [ \"$i\" -lt 300 ]; do\n  printf '%s\\n' '{\"method\":\"warning\",\"params\":{}}'\n  i=$((i + 1))\ndone\ncat >/dev/null\n",
        );
        thread::sleep(Duration::from_millis(50));

        let result = peer.shutdown();

        remove_fixture(&directory);
        result.unwrap();
    }

    // 명시적 shutdown 전 child가 exit 0으로 끝나도 정상 close로 숨기지 않고 stderr를
    // 포함한 ProcessExit로 분류해 상위 runtime이 Turn 실패를 관찰하게 합니다.
    #[test]
    fn unexpected_clean_exit_is_a_process_failure_with_stderr() {
        let (directory, mut peer) = spawn_fixture(
            "unexpected-clean-exit",
            "printf '%s\\n' 'clean-exit-diagnostic' >&2\n",
        );
        let deadline = Instant::now() + Duration::from_secs(1);
        while !peer
            .stderr_tail
            .lock()
            .unwrap()
            .contains("clean-exit-diagnostic")
            && Instant::now() < deadline
        {
            thread::yield_now();
        }

        let failure = peer.receive(Duration::from_secs(1)).unwrap_err();

        assert_eq!(failure.kind(), BackendFailureKind::ProcessExit);
        assert!(failure.message().contains("clean-exit-diagnostic"));
        peer.shutdown().unwrap();
        remove_fixture(&directory);
    }

    // 첫 write 전에 종료한 child의 안전한 진단은 shutdown이 stderr를 join한 뒤에도
    // 보존되며, 임의로 캡처한 내용은 오류 메시지에 노출하지 않는다.
    #[test]
    fn safe_stderr_diagnostics_survive_a_deterministic_broken_pipe() {
        let (directory, mut peer) = spawn_fixture(
            "safe-broken-pipe",
            "printf '%s\\n' 'known-cause credential-canary /private/secret-path' >&2\nexit 1\n",
        );
        peer.stderr_diagnostic =
            Some(|tail| tail.contains("known-cause").then_some("safe startup cause"));
        peer.process.child.lock().unwrap().wait().unwrap();

        let write_failure = peer
            .send(&serde_json::json!({"method": "initialize"}))
            .unwrap_err();
        assert_eq!(write_failure.kind(), BackendFailureKind::ProcessExit);
        let cleanup_failure = peer.shutdown().unwrap_err();
        assert!(cleanup_failure.message().contains("safe startup cause"));
        for failure in [&write_failure, &cleanup_failure] {
            assert!(!failure.message().contains("credential-canary"));
            assert!(!failure.message().contains("/private/secret-path"));
        }
        assert_eq!(
            peer.shutdown().unwrap_err().message(),
            cleanup_failure.message()
        );
        remove_fixture(&directory);
    }

    // stop handle은 진행 중 poll을 깨우지만 explicit shutdown 전의 child 종료는 기존
    // 계약대로 ProcessExit이며, 뒤이은 shutdown만 정상 cleanup으로 처리합니다.
    #[test]
    fn stop_handle_exit_is_a_process_failure_until_shutdown() {
        let (directory, mut peer) = spawn_fixture("stop-handle", "cat >/dev/null\n");

        peer.stop_handle().request_stop();
        let failure = peer.receive(Duration::from_secs(1)).unwrap_err();

        assert_eq!(failure.kind(), BackendFailureKind::ProcessExit);
        peer.shutdown().unwrap();
        remove_fixture(&directory);
    }

    // explicit shutdown 결과가 기록된 뒤에는 reader channel이 닫혀도 반복 poll이
    // ProcessExit로 되돌아가지 않고 안정적으로 Closed를 반환합니다.
    #[test]
    fn poll_after_explicit_shutdown_remains_closed() {
        let (directory, mut peer) = spawn_fixture("post-shutdown", "cat >/dev/null\n");

        peer.shutdown().unwrap();

        assert_eq!(peer.try_receive().unwrap(), JsonlPoll::Closed);
        remove_fixture(&directory);
    }
}
