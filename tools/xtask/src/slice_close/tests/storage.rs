use std::process::Command;

use super::super::storage;
use crate::test_support;

#[cfg(unix)]
// apply 입력은 다른 파일로 바뀔 수 있는 symlink를 따라가지 않아, 검토한 plan과
// 실제로 읽는 plan 사이의 파일 치환이 정리 권한으로 이어지지 않는다.
#[test]
fn rejects_symlinks() {
    use std::os::unix::fs::symlink;

    let target = test_support::unique_path("slice-close-plan-target");
    let link = test_support::unique_path("slice-close-plan-link");
    std::fs::write(&target, b"{}").unwrap();
    symlink(&target, &link).unwrap();

    let error = storage::read_plan(&link).unwrap_err();

    assert!(error.contains("cannot open Slice close plan"));
    std::fs::remove_file(link).unwrap();
    std::fs::remove_file(target).unwrap();
}

// 최대 크기보다 큰 regular file은 JSON parser에 넘기지 않아 plan 입력이
// 무제한 메모리를 소비하거나 뒤쪽 데이터를 숨기지 못하게 한다.
#[test]
fn rejects_oversized_plan_files() {
    let path = test_support::unique_path("slice-close-plan-oversized");
    std::fs::write(&path, vec![b'x'; 64 * 1024 + 1]).unwrap();

    let error = storage::read_plan(&path).unwrap_err();

    assert!(error.contains("65536-byte limit"));
    std::fs::remove_file(path).unwrap();
}

#[cfg(unix)]
// FIFO는 writer를 기다리며 개발 명령을 멈추지 않고 nonblocking open 뒤
// regular-file 검사를 통해 즉시 거절한다.
#[test]
fn rejects_fifo_without_blocking() {
    let path = test_support::unique_path("slice-close-plan-fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );

    let error = storage::read_plan(&path).unwrap_err();

    assert!(error.contains("regular file"));
    std::fs::remove_file(path).unwrap();
}
