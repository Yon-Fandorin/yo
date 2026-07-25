use std::{fs, os::unix::fs::symlink};

use super::capture_with_hook;
use crate::test_support::TestDirectory;

fn create_supporting_directories(root: &TestDirectory) {
    fs::create_dir_all(root.path().join("methexis/owners")).expect("owner directory");
    fs::create_dir_all(root.path().join("methexis/sources")).expect("source directory");
}

#[test]
fn concurrent_change_is_retryable_and_returns_no_snapshot() {
    let root = TestDirectory::new("concurrent-capture");
    create_supporting_directories(&root);
    let knowledge = root.path().join("methexis/knowledge/domain");
    fs::create_dir_all(&knowledge).expect("knowledge directory");
    let path = knowledge.join("unit.md");
    fs::write(&path, b"first").expect("first version");

    let error = capture_with_hook(root.path(), || {
        fs::write(&path, b"second").expect("concurrent version");
    })
    .err()
    .expect("capture must fail");
    let envelope = error.into_envelope();

    assert_eq!(envelope.error.code, "catalog_changed_during_capture");
    assert!(envelope.error.retryable);
}

#[test]
fn parent_swap_never_reads_through_a_symlink() {
    let root = TestDirectory::new("parent-swap");
    create_supporting_directories(&root);
    let domain = root.path().join("methexis/knowledge/domain");
    fs::create_dir_all(&domain).expect("knowledge directory");
    fs::write(domain.join("unit.md"), b"inside").expect("inside record");
    let external = TestDirectory::new("external-catalog");
    fs::write(external.path().join("unit.md"), b"outside").expect("outside record");

    let error = capture_with_hook(root.path(), || {
        fs::rename(&domain, root.path().join("original-domain")).expect("retain original");
        symlink(external.path(), &domain).expect("replace with symlink");
    })
    .err()
    .expect("capture must fail");
    let envelope = error.into_envelope();

    assert_eq!(envelope.error.code, "catalog_symlink_forbidden");
}

#[test]
fn oversized_record_fails_without_allocating_beyond_the_limit() {
    let root = TestDirectory::new("oversized-record");
    create_supporting_directories(&root);
    let knowledge = root.path().join("methexis/knowledge");
    fs::create_dir_all(&knowledge).expect("knowledge directory");
    fs::write(knowledge.join("large.md"), vec![b'x'; 256 * 1024 + 1]).expect("large record");

    let error = capture_with_hook(root.path(), || {})
        .err()
        .expect("capture must fail");

    assert_eq!(error.into_envelope().error.code, "catalog_record_too_large");
}

#[test]
fn aggregate_catalog_bytes_have_a_structured_limit() {
    let root = TestDirectory::new("aggregate-byte-limit");
    create_supporting_directories(&root);
    let knowledge = root.path().join("methexis/knowledge");
    fs::create_dir_all(&knowledge).expect("knowledge directory");
    for index in 0..17 {
        fs::write(
            knowledge.join(format!("{index}.md")),
            vec![b'x'; 256 * 1024],
        )
        .expect("bounded record");
    }

    let error = capture_with_hook(root.path(), || {})
        .err()
        .expect("capture must fail");

    assert_eq!(error.into_envelope().error.code, "catalog_limit_exceeded");
}

#[test]
fn catalog_file_count_has_a_structured_limit() {
    let root = TestDirectory::new("file-count-limit");
    create_supporting_directories(&root);
    let knowledge = root.path().join("methexis/knowledge");
    fs::create_dir_all(&knowledge).expect("knowledge directory");
    for index in 0..1025 {
        fs::write(knowledge.join(format!("{index}.md")), []).expect("empty record");
    }

    let error = capture_with_hook(root.path(), || {})
        .err()
        .expect("capture must fail");

    assert_eq!(error.into_envelope().error.code, "catalog_limit_exceeded");
}
