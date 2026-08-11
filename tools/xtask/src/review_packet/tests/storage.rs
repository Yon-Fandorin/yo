use super::super::storage;

// packet과 manifest는 한 directory rename으로 함께 나타나며, 같은 bytes는 재사용하고
// extra artifact가 있는 ReviewId directory는 교체하지 않고 corruption으로 거부한다.
#[test]
fn artifact_set_is_atomic_exact_and_reusable() {
    let root = crate::test_support::unique_path("slice-review-artifacts");
    let directory = root.join("review-id");
    let packet = b"packet\n";
    let manifest = b"manifest\n";

    assert_eq!(
        storage::publish(&directory, packet, manifest, || Ok(())).unwrap(),
        "created"
    );
    assert_eq!(std::fs::read(directory.join("packet.md")).unwrap(), packet);
    assert_eq!(
        std::fs::read(directory.join("manifest.json")).unwrap(),
        manifest
    );
    assert_eq!(
        storage::publish(&directory, packet, manifest, || Ok(())).unwrap(),
        "reused"
    );

    std::fs::write(directory.join("extra.txt"), b"unexpected\n").unwrap();
    let error = storage::publish(&directory, packet, manifest, || Ok(())).unwrap_err();
    assert!(error.contains("differs from the exact artifact set"));
    assert_eq!(std::fs::read(directory.join("packet.md")).unwrap(), packet);
    std::fs::remove_dir_all(root).unwrap();
}

// final revalidation 실패는 준비된 temporary sibling까지 정리해 packet이나
// manifest 어느 한쪽도 eligible output으로 남기지 않는다.
#[test]
fn final_revalidation_failure_publishes_no_partial_set() {
    let root = crate::test_support::unique_path("slice-review-final-guard");
    let directory = root.join("review-id");

    let error = storage::publish(&directory, b"packet", b"manifest", || {
        Err("candidate changed".to_owned())
    })
    .unwrap_err();

    assert_eq!(error, "candidate changed");
    assert!(!directory.exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

// prepared files가 모두 sync된 뒤 rename 직전 guard가 입력 변경을 발견하면 temporary
// sibling까지 정리하고 eligible ReviewId directory를 게시하지 않는다.
#[test]
fn rename_boundary_mutation_cleans_the_prepared_set() {
    let root = crate::test_support::unique_path("slice-review-rename-guard");
    let directory = root.join("review-id");

    let error = storage::publish_with_test_hook(
        &directory,
        b"packet",
        b"manifest",
        || Ok(()),
        || Err("captured evidence changed".to_owned()),
    )
    .unwrap_err();

    assert_eq!(error, "captured evidence changed");
    assert!(!directory.exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

// rename 경합에서 다른 writer의 exact artifact set이 먼저 나타나면 그 winner를
// reuse하기 전에 authority guard를 다시 실행하고 exact bytes를 재검증한다.
#[test]
fn concurrent_exact_winner_is_revalidated_before_reuse() {
    use std::{cell::Cell, rc::Rc};

    let root = crate::test_support::unique_path("slice-review-concurrent-winner");
    let directory = root.join("review-id");
    let packet = b"packet";
    let manifest = b"manifest";
    let calls = Rc::new(Cell::new(0));
    let observed = Rc::clone(&calls);
    let winner = directory.clone();

    let status = storage::publish_with_test_hook(
        &directory,
        packet,
        manifest,
        move || {
            observed.set(observed.get() + 1);
            Ok(())
        },
        move || {
            std::fs::create_dir(&winner).unwrap();
            std::fs::write(winner.join("packet.md"), packet).unwrap();
            std::fs::write(winner.join("manifest.json"), manifest).unwrap();
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(status, "reused");
    assert_eq!(calls.get(), 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
// ReviewId target 자체가 symlink이면 외부 directory의 그럴듯한 packet을 따라가
// reuse하지 않고 target entry를 corruption으로 거부한다.
#[test]
fn symlinked_review_directory_is_never_reused() {
    let root = crate::test_support::unique_path("slice-review-symlink");
    let outside = crate::test_support::unique_path("slice-review-symlink-outside");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("packet.md"), b"packet").unwrap();
    std::fs::write(outside.join("manifest.json"), b"manifest").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("review-id")).unwrap();

    let error =
        storage::publish(&root.join("review-id"), b"packet", b"manifest", || Ok(())).unwrap_err();

    assert!(error.contains("differs from the exact artifact set"));
    assert_eq!(std::fs::read(outside.join("packet.md")).unwrap(), b"packet");
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}
