use std::io::Write;

use sha2::{Digest, Sha256};

use super::{
    publish_new_or_exact, publish_new_or_exact_with, publish_new_or_exact_with_hooks,
    remove_regular_matching_sha256, remove_regular_matching_sha256_with_hooks,
};
use crate::test_support;

// exact hash가 일치하는 singly-linked regular file만 제거하고, 재실행 때 이미
// 없는 target은 성공적인 수렴 상태로 보고한다.
#[test]
fn exact_hash_removal_is_bounded_and_idempotent() {
    let directory = test_support::unique_path("bounded-file-remove-exact");
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("contract.json");
    let bytes = b"exact\n";
    std::fs::write(&target, bytes).unwrap();
    let hash = format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );

    assert!(remove_regular_matching_sha256(&target, &hash, 1024, "test file").unwrap());
    assert!(!remove_regular_matching_sha256(&target, &hash, 1024, "test file").unwrap());
    std::fs::remove_dir_all(directory).unwrap();
}

// plan에 묶인 hash와 현재 bytes가 다르면 삭제하지 않아 사람이 변경 원인을
// 조사할 수 있게 파일을 그대로 보존한다.
#[test]
fn hash_mismatch_preserves_the_target() {
    let directory = test_support::unique_path("bounded-file-remove-mismatch");
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("contract.json");
    std::fs::write(&target, b"changed\n").unwrap();

    let error = remove_regular_matching_sha256(
        &target,
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        1024,
        "test file",
    )
    .unwrap_err();

    assert!(error.contains("hash changed"));
    assert!(target.exists());
    std::fs::remove_dir_all(directory).unwrap();
}

// initial hash 확인 직후 pathname bytes가 바뀌어도 atomic claim 뒤 다시
// 검증하므로 바뀐 file은 삭제하지 않고 원래 이름으로 복구한다.
#[test]
fn replacement_between_hash_and_claim_is_preserved() {
    let directory = test_support::unique_path("bounded-file-remove-race");
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("contract.json");
    let bytes = b"exact\n";
    std::fs::write(&target, bytes).unwrap();
    let hash = format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );

    let error = remove_regular_matching_sha256_with_hooks(
        &target,
        &hash,
        1024,
        "test file",
        || std::fs::write(&target, b"changed\n").map_err(|error| error.to_string()),
        |parent| super::sync_directory(parent, "test file"),
    )
    .unwrap_err();

    assert!(error.contains("hash changed"));
    assert_eq!(std::fs::read(&target).unwrap(), b"changed\n");
    std::fs::remove_dir_all(directory).unwrap();
}

// claim 직후 parent sync가 실패해도 hash-addressed claimed name이 남아
// 재실행이 같은 inode를 검증하고 삭제까지 수렴한다.
#[test]
fn retry_finishes_an_unsynced_claim() {
    let directory = test_support::unique_path("bounded-file-remove-claim-sync");
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("contract.json");
    let bytes = b"exact\n";
    std::fs::write(&target, bytes).unwrap();
    let hash = format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );

    let error = remove_regular_matching_sha256_with_hooks(
        &target,
        &hash,
        1024,
        "test file",
        || Ok(()),
        |_| Err("injected claim sync failure".to_owned()),
    )
    .unwrap_err();
    assert!(error.contains("injected claim sync failure"));
    assert!(!target.exists());

    assert!(remove_regular_matching_sha256(&target, &hash, 1024, "test file").unwrap());
    assert!(!target.exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
// initial hash 뒤 symlink로 바뀐 target도 claim 후 검증에서 거절하고 원래
// pathname으로 복구하며 link target은 건드리지 않는다.
#[test]
fn symlink_replacement_during_claim_is_restored() {
    use std::os::unix::fs::symlink;

    let directory = test_support::unique_path("bounded-file-remove-symlink-race");
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("contract.json");
    let outside = directory.join("outside.json");
    let bytes = b"exact\n";
    std::fs::write(&target, bytes).unwrap();
    std::fs::write(&outside, b"outside\n").unwrap();
    let hash = format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );

    let error = remove_regular_matching_sha256_with_hooks(
        &target,
        &hash,
        1024,
        "test file",
        || {
            std::fs::remove_file(&target).map_err(|error| error.to_string())?;
            symlink(&outside, &target).map_err(|error| error.to_string())
        },
        |parent| super::sync_directory(parent, "test file"),
    )
    .unwrap_err();

    assert!(error.contains("cannot open test file"));
    assert!(
        std::fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside\n");
    std::fs::remove_file(&target).unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

// unlink 뒤 parent sync가 실패해도 재실행은 absent 상태에서 parent를 다시
// sync한 뒤 성공하므로 삭제 내구성까지 수렴한다.
#[test]
fn retry_resyncs_an_unlinked_target() {
    let directory = test_support::unique_path("bounded-file-remove-resync");
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("contract.json");
    let bytes = b"exact\n";
    std::fs::write(&target, bytes).unwrap();
    let hash = format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let mut syncs = 0;

    let error = remove_regular_matching_sha256_with_hooks(
        &target,
        &hash,
        1024,
        "test file",
        || Ok(()),
        |_| {
            syncs += 1;
            if syncs == 2 {
                Err("injected parent sync failure".to_owned())
            } else {
                Ok(())
            }
        },
    )
    .unwrap_err();
    assert!(error.contains("injected parent sync failure"));
    assert!(!target.exists());

    let mut retry_syncs = 0;
    assert!(
        !remove_regular_matching_sha256_with_hooks(
            &target,
            &hash,
            1024,
            "test file",
            || Ok(()),
            |_| {
                retry_syncs += 1;
                Ok(())
            },
        )
        .unwrap()
    );
    assert_eq!(retry_syncs, 1);
    std::fs::remove_dir_all(directory).unwrap();
}

// 이전 실행이 write 중 중단되어 partial prepared file을 남겨도 고유한 새
// 임시 파일을 사용해 exact target을 발행하고 stale artifact에 막히지 않는다.
#[test]
fn retry_ignores_a_partial_prepared_file() {
    let directory = test_support::unique_path("bounded-file-recovery");
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("contract.json");
    let prepared = directory.join(".contract.json.yo-prepare-stale");
    std::fs::write(&prepared, b"part").unwrap();

    assert!(publish_new_or_exact(&target, b"exact\n", 1024, "test file").unwrap());
    assert_eq!(std::fs::read(&target).unwrap(), b"exact\n");
    assert_eq!(std::fs::read(&prepared).unwrap(), b"part");
    std::fs::remove_dir_all(directory).unwrap();
}

// 실제 write가 일부 bytes 뒤 실패해 helper-owned temp가 남은 경우에도
// 다음 호출은 새 temp를 사용하고 exact target으로 수렴한다.
#[test]
fn retry_converges_after_an_injected_partial_write_failure() {
    let directory = test_support::unique_path("bounded-file-write-failure");
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("contract.json");
    let error = publish_new_or_exact_with(&target, b"exact\n", 1024, "test file", |file, bytes| {
        file.write_all(&bytes[..2]).unwrap();
        Err("injected write failure".to_owned())
    })
    .unwrap_err();
    assert!(error.contains("injected write failure"));

    assert!(publish_new_or_exact(&target, b"exact\n", 1024, "test file").unwrap());
    assert_eq!(std::fs::read(&target).unwrap(), b"exact\n");
    std::fs::remove_dir_all(directory).unwrap();
}

// complete bytes를 쓴 뒤 sync 단계가 실패한 것처럼 중단되어도 그 temp를
// 승격하지 않고 다음 호출이 새로 쓰고 sync한 target만 발행한다.
#[test]
fn retry_converges_after_an_injected_sync_failure() {
    let directory = test_support::unique_path("bounded-file-sync-failure");
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("contract.json");
    let error = publish_new_or_exact_with(&target, b"exact\n", 1024, "test file", |file, bytes| {
        file.write_all(bytes).unwrap();
        Err("injected sync failure".to_owned())
    })
    .unwrap_err();
    assert!(error.contains("injected sync failure"));

    assert!(publish_new_or_exact(&target, b"exact\n", 1024, "test file").unwrap());
    assert_eq!(std::fs::read(&target).unwrap(), b"exact\n");
    std::fs::remove_dir_all(directory).unwrap();
}

// rename 뒤 parent directory sync가 실패하면 target은 보일 수 있지만 durable
// 여부는 미정이다. 재실행은 exact target에서도 parent sync를 다시 수행한다.
#[test]
fn retry_resyncs_parent_after_an_injected_post_rename_failure() {
    let directory = test_support::unique_path("bounded-file-parent-sync-failure");
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("contract.json");
    let error = publish_new_or_exact_with_hooks(
        &target,
        b"exact\n",
        1024,
        "test file",
        |file, bytes| {
            file.write_all(bytes).unwrap();
            file.sync_all().map_err(|sync| sync.to_string())
        },
        |_| Err("injected parent sync failure".to_owned()),
    )
    .unwrap_err();
    assert!(error.contains("injected parent sync failure"));
    assert_eq!(std::fs::read(&target).unwrap(), b"exact\n");

    let mut resynced = false;
    assert!(
        !publish_new_or_exact_with_hooks(
            &target,
            b"exact\n",
            1024,
            "test file",
            |_, _| panic!("exact target reuse must not write another temporary"),
            |_| {
                resynced = true;
                Ok(())
            },
        )
        .unwrap()
    );
    assert!(resynced);
    std::fs::remove_dir_all(directory).unwrap();
}

// 경쟁 publisher가 exact target을 먼저 rename한 EEXIST 경로도 현재 호출이
// parent를 직접 sync하여 다른 호출의 durability 결과에 의존하지 않는다.
#[test]
fn exact_rename_collision_syncs_the_parent_before_reuse() {
    let directory = test_support::unique_path("bounded-file-rename-collision");
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("contract.json");
    let mut synced = false;

    let created = publish_new_or_exact_with_hooks(
        &target,
        b"exact\n",
        1024,
        "test file",
        |file, bytes| {
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
            std::fs::write(&target, bytes).unwrap();
            Ok(())
        },
        |_| {
            synced = true;
            Ok(())
        },
    )
    .unwrap();

    assert!(!created);
    assert!(synced);
    assert_eq!(std::fs::read(&target).unwrap(), b"exact\n");
    std::fs::remove_dir_all(directory).unwrap();
}

// target 자체가 다른 bytes면 새 prepared artifact를 만들거나 기존 계약을
// 덮어쓰지 않고 충돌을 그대로 보고한다.
#[test]
fn retry_rejects_conflicting_target_bytes() {
    let directory = test_support::unique_path("bounded-file-conflict");
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("contract.json");
    std::fs::write(&target, b"other\n").unwrap();

    let error = publish_new_or_exact(&target, b"exact\n", 1024, "test file").unwrap_err();

    assert!(error.contains("already contains different bytes"));
    assert_eq!(std::fs::read(&target).unwrap(), b"other\n");
    std::fs::remove_dir_all(directory).unwrap();
}
