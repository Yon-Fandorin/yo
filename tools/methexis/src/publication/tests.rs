//! Deterministic parent-swap coverage for directory-handle publication.

#[cfg(unix)]
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
