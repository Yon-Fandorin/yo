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

fn read_regular_at(
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

fn open_directory(path: &Path, label: &str) -> Result<OwnedFd, String> {
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

fn sync_directory(directory: &OwnedFd, label: &str) -> Result<(), String> {
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

    use super::{publish_new_or_exact, publish_new_or_exact_with, publish_new_or_exact_with_hooks};
    use crate::test_support;

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
