use std::{
    ffi::OsString,
    fs::File,
    io::{Read, Write},
    os::fd::OwnedFd,
    path::{Component, Path},
    sync::atomic::{AtomicU64, Ordering},
};

use rustix::{
    fs::{
        AtFlags, FileType, Mode, OFlags, RenameFlags, fstat, mkdirat, open, openat, renameat_with,
        unlinkat,
    },
    io::Errno,
};
use sha2::{Digest, Sha256};

const READ_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NONBLOCK)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const CREATE_FLAGS: OFlags = OFlags::WRONLY
    .union(OFlags::CREATE)
    .union(OFlags::EXCL)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(1);

pub(crate) fn read_regular(path: &Path, limit: usize, label: &str) -> Result<Vec<u8>, String> {
    let parent_path = path
        .parent()
        .ok_or_else(|| format!("{label} path {} has no parent", path.display()))?;
    let target = path
        .file_name()
        .ok_or_else(|| format!("{label} path {} has no file name", path.display()))?;
    let parent = open_directory(parent_path, label)?;
    read_regular_at(&parent, target, path, limit, label)?
        .ok_or_else(|| format!("cannot open {label} {}: {}", path.display(), Errno::NOENT))
}

pub(crate) fn read_regular_at(
    parent: &OwnedFd,
    name: &std::ffi::OsStr,
    display_path: &Path,
    limit: usize,
    label: &str,
) -> Result<Option<Vec<u8>>, String> {
    let fd = match openat(parent, name, READ_FLAGS, Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot open {label} {}: {error}",
                display_path.display()
            ));
        },
    };
    let stat = fstat(&fd)
        .map_err(|error| format!("cannot inspect {label} {}: {error}", display_path.display()))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile || stat.st_nlink != 1 {
        return Err(format!("{label} must be a singly linked regular file"));
    }
    let declared =
        usize::try_from(stat.st_size).map_err(|_| format!("{label} has an unsupported size"))?;
    if declared > limit {
        return Err(format!("{label} exceeds the {limit}-byte limit"));
    }
    let mut bytes = Vec::with_capacity(declared.min(limit));
    File::from(fd)
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {label} {}: {error}", display_path.display()))?;
    if bytes.len() > limit {
        return Err(format!("{label} exceeds the {limit}-byte limit"));
    }
    Ok(Some(bytes))
}

pub(crate) fn publish_new_or_exact(
    path: &Path,
    expected: &[u8],
    limit: usize,
    label: &str,
) -> Result<bool, String> {
    publish_new_or_exact_with(path, expected, limit, label, |file, bytes| {
        file.write_all(bytes).map_err(|error| {
            format!("cannot write prepared {label} {}: {error}", path.display())
        })?;
        file.sync_all()
            .map_err(|error| format!("cannot sync prepared {label} {}: {error}", path.display()))
    })
}

pub(crate) fn remove_regular_matching_sha256(
    path: &Path,
    expected_hash: &str,
    limit: usize,
    label: &str,
) -> Result<bool, String> {
    remove_regular_matching_sha256_with_hooks(
        path,
        expected_hash,
        limit,
        label,
        || Ok(()),
        |parent| sync_directory(parent, label),
    )
}

fn remove_regular_matching_sha256_with_hooks(
    path: &Path,
    expected_hash: &str,
    limit: usize,
    label: &str,
    mut before_claim: impl FnMut() -> Result<(), String>,
    mut sync_parent: impl FnMut(&OwnedFd) -> Result<(), String>,
) -> Result<bool, String> {
    let parent_path = path
        .parent()
        .ok_or_else(|| format!("{label} path {} has no parent", path.display()))?;
    let target = path
        .file_name()
        .ok_or_else(|| format!("{label} path {} has no file name", path.display()))?;
    let hash_suffix = expected_hash
        .strip_prefix("sha256:")
        .filter(|suffix| {
            suffix.len() == 64
                && suffix
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
        .ok_or_else(|| format!("{label} expected hash is not canonical SHA-256"))?;
    let mut claimed = OsString::from(".");
    claimed.push(target);
    claimed.push(format!(".yo-remove-{hash_suffix}"));
    let parent = open_directory(parent_path, label)?;
    let claimed_path = parent_path.join(&claimed);
    let mut removed = false;

    loop {
        let claimed_bytes = match read_regular_at(&parent, &claimed, &claimed_path, limit, label) {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(restore_claimed(
                    &parent,
                    &claimed,
                    target,
                    &claimed_path,
                    path,
                    label,
                    &mut sync_parent,
                    error,
                ));
            },
        };
        if let Some(bytes) = claimed_bytes {
            // A previous attempt may have stopped after the atomic claim. Establish that
            // directory state before deleting the claimed inode.
            sync_parent(&parent)?;
            if let Err(error) = exact_sha256(path, expected_hash, &bytes, label) {
                return Err(restore_claimed(
                    &parent,
                    &claimed,
                    target,
                    &claimed_path,
                    path,
                    label,
                    &mut sync_parent,
                    error,
                ));
            }
            unlinkat(&parent, &claimed, AtFlags::empty()).map_err(|error| {
                format!(
                    "cannot remove claimed {label} {}: {error}",
                    claimed_path.display()
                )
            })?;
            sync_parent(&parent)?;
            removed = true;
            continue;
        }

        let Some(bytes) = read_regular_at(&parent, target, path, limit, label)? else {
            // This also makes a preceding unlink durable when its parent sync failed.
            sync_parent(&parent)?;
            return Ok(removed);
        };
        if let Err(error) = exact_sha256(path, expected_hash, &bytes, label) {
            sync_parent(&parent)?;
            return Err(error);
        }

        before_claim()?;
        match renameat_with(&parent, target, &parent, &claimed, RenameFlags::NOREPLACE) {
            Ok(()) => {},
            Err(Errno::NOENT | Errno::EXIST) => continue,
            Err(error) => {
                return Err(format!(
                    "cannot claim {label} {} for removal: {error}",
                    path.display()
                ));
            },
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn restore_claimed(
    parent: &OwnedFd,
    claimed: &std::ffi::OsStr,
    target: &std::ffi::OsStr,
    claimed_path: &Path,
    path: &Path,
    label: &str,
    sync_parent: &mut impl FnMut(&OwnedFd) -> Result<(), String>,
    error: String,
) -> String {
    match renameat_with(parent, claimed, parent, target, RenameFlags::NOREPLACE) {
        Ok(()) => match sync_parent(parent) {
            Ok(()) => error,
            Err(sync_error) => format!(
                "{error}; restored {label} {} but cannot sync its parent: {sync_error}",
                path.display()
            ),
        },
        Err(Errno::EXIST) => format!(
            "{error}; preserved the claimed file at {} because {} was recreated",
            claimed_path.display(),
            path.display()
        ),
        Err(restore_error) => format!(
            "{error}; cannot restore {label} {}: {restore_error}",
            path.display()
        ),
    }
}

fn exact_sha256(path: &Path, expected_hash: &str, bytes: &[u8], label: &str) -> Result<(), String> {
    let actual_hash = format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    if actual_hash == expected_hash {
        Ok(())
    } else {
        Err(format!(
            "{label} {} hash changed: expected {expected_hash}, found {actual_hash}",
            path.display()
        ))
    }
}

fn publish_new_or_exact_with(
    path: &Path,
    expected: &[u8],
    limit: usize,
    label: &str,
    write_prepared: impl FnOnce(&mut File, &[u8]) -> Result<(), String>,
) -> Result<bool, String> {
    publish_new_or_exact_with_hooks(path, expected, limit, label, write_prepared, |parent| {
        sync_directory(parent, label)
    })
}

fn publish_new_or_exact_with_hooks(
    path: &Path,
    expected: &[u8],
    limit: usize,
    label: &str,
    write_prepared: impl FnOnce(&mut File, &[u8]) -> Result<(), String>,
    mut sync_parent: impl FnMut(&OwnedFd) -> Result<(), String>,
) -> Result<bool, String> {
    if expected.len() > limit {
        return Err(format!("{label} exceeds the {limit}-byte limit"));
    }
    let parent_path = path
        .parent()
        .ok_or_else(|| format!("{label} path {} has no parent", path.display()))?;
    let target = path
        .file_name()
        .ok_or_else(|| format!("{label} path {} has no file name", path.display()))?;
    let parent = open_directory(parent_path, label)?;
    if let Some(actual) = read_regular_at(&parent, target, path, limit, label)? {
        exact_bytes(path, expected, &actual, label)?;
        sync_parent(&parent)?;
        return Ok(false);
    }

    let (temporary, fd) = create_temporary(&parent, target, path, label)?;
    let mut file = File::from(fd);
    write_prepared(&mut file, expected)?;
    drop(file);

    match renameat_with(&parent, &temporary, &parent, target, RenameFlags::NOREPLACE) {
        Ok(()) => {
            sync_parent(&parent)?;
            Ok(true)
        },
        Err(Errno::EXIST) => {
            let actual =
                read_regular_at(&parent, target, path, limit, label)?.ok_or_else(|| {
                    format!("{label} {} disappeared during publication", path.display())
                })?;
            exact_bytes(path, expected, &actual, label)?;
            let _ = unlinkat(&parent, &temporary, AtFlags::empty());
            sync_parent(&parent)?;
            Ok(false)
        },
        Err(error) => Err(format!(
            "cannot publish {label} {}: {error}",
            path.display()
        )),
    }
}

fn create_temporary(
    parent: &OwnedFd,
    target: &std::ffi::OsStr,
    display_path: &Path,
    label: &str,
) -> Result<(OsString, OwnedFd), String> {
    for _ in 0..1024 {
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let mut temporary = OsString::from(".");
        temporary.push(target);
        temporary.push(format!(".yo-prepare-{}-{sequence}", std::process::id()));
        match openat(parent, &temporary, CREATE_FLAGS, Mode::from_raw_mode(0o600)) {
            Ok(fd) => return Ok((temporary, fd)),
            Err(Errno::EXIST) => continue,
            Err(error) => {
                return Err(format!(
                    "cannot prepare {label} {}: {error}",
                    display_path.display()
                ));
            },
        }
    }
    Err(format!(
        "cannot allocate a unique prepared {label} for {}",
        display_path.display()
    ))
}

pub(crate) fn open_directory(path: &Path, label: &str) -> Result<OwnedFd, String> {
    let mut directory = if path.is_absolute() {
        open(Path::new("/"), DIRECTORY_FLAGS, Mode::empty())
    } else {
        open(Path::new("."), DIRECTORY_FLAGS, Mode::empty())
    }
    .map_err(|error| format!("cannot open {label} directory anchor: {error}"))?;
    for component in path.components() {
        let name = match component {
            Component::RootDir | Component::CurDir => continue,
            Component::ParentDir => std::ffi::OsStr::new(".."),
            Component::Normal(name) => name,
            Component::Prefix(_) => {
                return Err(format!(
                    "cannot open {label} directory {} with a platform prefix",
                    path.display()
                ));
            },
        };
        directory = openat(&directory, name, DIRECTORY_FLAGS, Mode::empty()).map_err(|error| {
            format!(
                "cannot open {label} directory {} without symlinks: {error}",
                path.display()
            )
        })?;
    }
    Ok(directory)
}

pub(crate) fn ensure_directory(path: &Path, label: &str) -> Result<(), String> {
    let mut directory = if path.is_absolute() {
        open(Path::new("/"), DIRECTORY_FLAGS, Mode::empty())
    } else {
        open(Path::new("."), DIRECTORY_FLAGS, Mode::empty())
    }
    .map_err(|error| format!("cannot open {label} directory anchor: {error}"))?;
    for component in path.components() {
        let name = match component {
            Component::RootDir | Component::CurDir => continue,
            Component::ParentDir => std::ffi::OsStr::new(".."),
            Component::Normal(name) => name,
            Component::Prefix(_) => {
                return Err(format!(
                    "cannot create {label} directory {} with a platform prefix",
                    path.display()
                ));
            },
        };
        directory = match openat(&directory, name, DIRECTORY_FLAGS, Mode::empty()) {
            Ok(next) => next,
            Err(Errno::NOENT) if !matches!(component, Component::ParentDir) => {
                match mkdirat(&directory, name, Mode::from_raw_mode(0o777)) {
                    Ok(()) | Err(Errno::EXIST) => {},
                    Err(error) => {
                        return Err(format!(
                            "cannot create {label} directory {}: {error}",
                            path.display()
                        ));
                    },
                }
                openat(&directory, name, DIRECTORY_FLAGS, Mode::empty()).map_err(|error| {
                    format!(
                        "cannot open created {label} directory {} without symlinks: {error}",
                        path.display()
                    )
                })?
            },
            Err(error) => {
                return Err(format!(
                    "cannot open {label} directory {} without symlinks: {error}",
                    path.display()
                ));
            },
        };
    }
    Ok(())
}

pub(crate) fn sync_directory(directory: &OwnedFd, label: &str) -> Result<(), String> {
    File::from(
        rustix::io::dup(directory)
            .map_err(|error| format!("cannot retain {label} parent for sync: {error}"))?,
    )
    .sync_all()
    .map_err(|error| format!("cannot sync {label} parent: {error}"))
}

fn exact_bytes(path: &Path, expected: &[u8], actual: &[u8], label: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{label} {} already contains different bytes",
            path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
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
        let error =
            publish_new_or_exact_with(&target, b"exact\n", 1024, "test file", |file, bytes| {
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
        let error =
            publish_new_or_exact_with(&target, b"exact\n", 1024, "test file", |file, bytes| {
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
}
