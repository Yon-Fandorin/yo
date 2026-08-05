//! Deterministic parent-swap coverage for directory-handle publication.

#[cfg(unix)]
// 이미 연 parent 디렉터리는 이후 symlink 교체로 리다이렉트되지 않고 원래 디렉터리에 원자적으로
// 기록한다.
#[test]
fn opened_parent_cannot_be_redirected_by_a_later_symlink_swap() {
    use std::{
        fs,
        os::unix::fs::symlink,
        time::{SystemTime, UNIX_EPOCH},
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "methexis-publication-{}-{unique}",
        std::process::id()
    ));
    let outside = root.with_extension("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    let target = root.join("records/item.yaml");
    let lock = super::lock_target(&root, &target).unwrap();

    let opened_parent = root.with_extension("opened-parent");
    fs::rename(root.join("records"), &opened_parent).unwrap();
    symlink(&outside, root.join("records")).unwrap();
    lock.atomic_create(b"trusted\n").unwrap();

    assert_eq!(
        fs::read(opened_parent.join("item.yaml")).unwrap(),
        b"trusted\n"
    );
    assert!(!outside.join("item.yaml").exists());
    drop(lock);
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&opened_parent).unwrap();
    fs::remove_dir_all(&outside).unwrap();
}

#[cfg(unix)]
#[ignore]
// 기본 테스트가 별도 test process로 실행해 SIGKILL 시 kernel lock 해제를 검증한다.
#[test]
fn child_holds_kernel_lock_until_killed() {
    use std::{fs, path::PathBuf, thread, time::Duration};

    let root = PathBuf::from(std::env::var_os("METHEXIS_LOCK_TEST_ROOT").unwrap());
    let target = PathBuf::from(std::env::var_os("METHEXIS_LOCK_TEST_TARGET").unwrap());
    let ready = PathBuf::from(std::env::var_os("METHEXIS_LOCK_TEST_READY").unwrap());
    let _lock = super::lock_target(&root, &target).unwrap();
    fs::write(ready, b"ready\n").unwrap();
    loop {
        thread::park_timeout(Duration::from_secs(30));
    }
}

#[cfg(unix)]
// persistent owner 파일은 남더라도 SIGKILL 뒤 flock은 kernel이 해제해 PID 판정 없이 재획득된다.
#[test]
fn kernel_lock_is_recovered_after_owner_is_killed() {
    use std::{
        fs,
        process::Command,
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "methexis-kill-lock-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let target = root.join("records/item.yaml");
    let ready = root.join("ready");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "publication::tests::child_holds_kernel_lock_until_killed",
            "--ignored",
            "--nocapture",
        ])
        .env("METHEXIS_LOCK_TEST_ROOT", &root)
        .env("METHEXIS_LOCK_TEST_TARGET", &target)
        .env("METHEXIS_LOCK_TEST_READY", &ready)
        .spawn()
        .unwrap();
    for _ in 0..200 {
        if ready.exists() {
            break;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("lock helper exited early: {status}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.exists(), "lock helper did not become ready");
    assert!(matches!(
        super::lock_target(&root, &target),
        Err(super::PublicationError::Locked(_))
    ));
    child.kill().unwrap();
    child.wait().unwrap();
    let recovered = super::lock_target(&root, &target).unwrap();
    drop(recovered);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
// 같은 bytes로 path를 교체해도 capture의 inode identity guard는 변경을 탐지한다.
#[test]
fn captured_file_rejects_same_byte_identity_replacement() {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "methexis-capture-swap-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("records")).unwrap();
    let target = root.join("records/item.yaml");
    fs::write(&target, b"same\n").unwrap();
    let captured = super::capture_file(&root, &target, 32).unwrap();
    fs::write(root.join("records/replacement"), b"same\n").unwrap();
    fs::rename(root.join("records/replacement"), &target).unwrap();
    assert!(captured.revalidate().is_err());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
// bounded capture는 FIFO를 nonblocking으로 열고 regular-file 검증에서 즉시 거부한다.
#[test]
fn captured_file_rejects_fifo_without_blocking() {
    use std::{
        fs,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "methexis-capture-fifo-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("records")).unwrap();
    let target = root.join("records/item.yaml");
    assert!(
        Command::new("mkfifo")
            .arg(&target)
            .status()
            .unwrap()
            .success()
    );
    assert!(super::capture_file(&root, &target, 32).is_err());
    fs::remove_dir_all(root).unwrap();
}

// bounded capture는 상한보다 한 byte만 커도 전체 입력을 보존하지 않고 거부한다.
#[test]
fn captured_file_rejects_oversized_regular_file() {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "methexis-capture-large-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("records")).unwrap();
    let target = root.join("records/item.yaml");
    fs::write(&target, [0_u8; 33]).unwrap();
    assert!(super::capture_file(&root, &target, 32).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
// canonical repository root와 그 root를 가리키는 absolute symlink alias target은 같은
// retained root identity로 해석한다.
#[test]
fn canonical_root_accepts_an_absolute_alias_to_that_exact_root() {
    use std::{
        fs,
        os::unix::fs::symlink,
        time::{SystemTime, UNIX_EPOCH},
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let fixture = std::env::temp_dir().join(format!(
        "methexis-root-alias-{}-{unique}",
        std::process::id()
    ));
    let real = fixture.join("real");
    let alias = fixture.join("alias");
    fs::create_dir_all(real.join("records")).unwrap();
    symlink(&real, &alias).unwrap();
    let canonical_root = fs::canonicalize(&real).unwrap();
    let aliased_target = alias.join("records/item.yaml");

    let lock = super::lock_target(&canonical_root, &aliased_target).unwrap();
    lock.atomic_create(b"trusted\n").unwrap();

    assert_eq!(
        fs::read(real.join("records/item.yaml")).unwrap(),
        b"trusted\n"
    );
    drop(lock);
    fs::remove_dir_all(fixture).unwrap();
}

#[cfg(unix)]
// root alias를 정규화한 뒤에도 root 내부 symlink component는 외부로 따라가지 않는다.
#[test]
fn root_alias_does_not_allow_an_internal_symlink_escape() {
    use std::{
        fs,
        os::unix::fs::symlink,
        time::{SystemTime, UNIX_EPOCH},
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let fixture = std::env::temp_dir().join(format!(
        "methexis-root-alias-escape-{}-{unique}",
        std::process::id()
    ));
    let real = fixture.join("real");
    let alias = fixture.join("alias");
    let outside = fixture.join("outside");
    fs::create_dir_all(&real).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&real, &alias).unwrap();
    symlink(&outside, real.join("records")).unwrap();
    let canonical_root = fs::canonicalize(&real).unwrap();

    assert!(super::lock_target(&canonical_root, &alias.join("records/item.yaml")).is_err());
    assert!(!outside.join("item.yaml").exists());
    fs::remove_dir_all(fixture).unwrap();
}

#[cfg(unix)]
// link commit 뒤 temp cleanup이 실패한 경우의 복구 경계는 target과 hardlink temp를 모두 제거한다.
#[test]
fn created_file_rollback_removes_target_and_temporary_hardlinks() {
    use std::{
        fs,
        os::unix::fs::MetadataExt,
        time::{SystemTime, UNIX_EPOCH},
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "methexis-create-rollback-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let temporary = root.join("temporary");
    let target = root.join("target");
    fs::write(&temporary, b"candidate\n").unwrap();
    fs::hard_link(&temporary, &target).unwrap();
    assert_eq!(fs::metadata(&target).unwrap().nlink(), 2);
    let parent = rustix::fs::open(&root, super::OPEN_DIRECTORY, rustix::fs::Mode::empty()).unwrap();

    super::rollback_created_file(
        &parent,
        target.file_name().unwrap(),
        temporary.file_name().unwrap(),
    )
    .unwrap();

    assert!(!target.exists());
    assert!(!temporary.exists());
    fs::remove_dir(root).unwrap();
}

// ambiguous replacement의 restore 성공/실패는 일반 실패와 recovery-required로 구분된다.
#[test]
fn ambiguous_replacement_failure_seam_preserves_recovery_state() {
    use std::io;

    let restored = super::classify_ambiguous_recovery(io::Error::other("fsync failed"), Ok(()));
    assert!(matches!(restored, super::PublicationError::Io(_)));
    let unresolved = super::classify_ambiguous_recovery(
        io::Error::other("fsync failed"),
        Err(super::PublicationError::Io(io::Error::other(
            "restore failed",
        ))),
    );
    assert!(matches!(
        unresolved,
        super::PublicationError::DurabilityUnknown(_)
    ));
}
