use std::{
    ffi::OsString,
    fs::File,
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use rustix::{
    fs::{
        AtFlags, Dir, Mode, OFlags, RenameFlags, fstat, mkdirat, openat, renameat_with, statat,
        unlinkat,
    },
    io::Errno,
};

use crate::bounded_file;

const FILE_LIMIT: usize = 32 * 1024 * 1024;
const FILE_FLAGS: OFlags = OFlags::WRONLY
    .union(OFlags::CREATE)
    .union(OFlags::EXCL)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(1);

pub(super) fn publish(
    directory: &Path,
    packet: &[u8],
    manifest: &[u8],
    mut final_revalidate: impl FnMut() -> Result<(), String>,
) -> Result<&'static str, String> {
    publish_with_hook(
        directory,
        packet,
        manifest,
        &mut final_revalidate,
        || Ok(()),
    )
}

fn publish_with_hook(
    directory: &Path,
    packet: &[u8],
    manifest: &[u8],
    final_revalidate: &mut impl FnMut() -> Result<(), String>,
    mut before_rename: impl FnMut() -> Result<(), String>,
) -> Result<&'static str, String> {
    let parent_path = directory
        .parent()
        .ok_or_else(|| "review packet directory has no parent".to_owned())?;
    let target = directory
        .file_name()
        .ok_or_else(|| "review packet directory has no name".to_owned())?;
    bounded_file::ensure_directory(parent_path, "Slice review packet")?;

    match inspect(directory, packet, manifest)? {
        Existing::Exact => {
            final_revalidate()?;
            require_exact(directory, packet, manifest)?;
            return Ok("reused");
        },
        Existing::Different => {
            return Err(format!(
                "existing ReviewId directory {} differs from the exact artifact set",
                directory.display()
            ));
        },
        Existing::Missing => {},
    }

    let parent = bounded_file::open_directory(parent_path, "Slice review packet")?;
    let temporary = create_temporary(&parent, target, directory)?;
    let temporary_directory = match openat(&parent, &temporary, DIRECTORY_FLAGS, Mode::empty()) {
        Ok(directory) => directory,
        Err(error) => {
            let _ = unlinkat(&parent, &temporary, AtFlags::REMOVEDIR);
            return Err(format!(
                "cannot open prepared review packet directory: {error}"
            ));
        },
    };
    if let Err(error) = write_file(&temporary_directory, "packet.md", packet)
        .and_then(|()| write_file(&temporary_directory, "manifest.json", manifest))
        .and_then(|()| bounded_file::sync_directory(&temporary_directory, "review packet"))
    {
        cleanup(&parent, &temporary, &temporary_directory);
        return Err(error);
    }
    drop(temporary_directory);
    if let Err(error) = final_revalidate().and_then(|()| before_rename()) {
        cleanup_path(&parent, &temporary);
        return Err(error);
    }

    let status = match renameat_with(&parent, &temporary, &parent, target, RenameFlags::NOREPLACE) {
        Ok(()) => "created",
        Err(Errno::EXIST) => {
            cleanup_path(&parent, &temporary);
            require_exact(directory, packet, manifest)?;
            final_revalidate()?;
            require_exact(directory, packet, manifest)?;
            "reused"
        },
        Err(error) => {
            cleanup_path(&parent, &temporary);
            return Err(format!("cannot publish review packet atomically: {error}"));
        },
    };
    bounded_file::sync_directory(&parent, "review packet")?;
    require_exact(directory, packet, manifest)?;
    Ok(status)
}

enum Existing {
    Missing,
    Exact,
    Different,
}

fn inspect(directory: &Path, packet: &[u8], manifest: &[u8]) -> Result<Existing, String> {
    let parent_path = directory
        .parent()
        .ok_or_else(|| "review packet directory has no parent".to_owned())?;
    let target = directory
        .file_name()
        .ok_or_else(|| "review packet directory has no name".to_owned())?;
    let parent = bounded_file::open_directory(parent_path, "Slice review packet")?;
    let opened = match openat(&parent, target, DIRECTORY_FLAGS, Mode::empty()) {
        Ok(opened) => opened,
        Err(Errno::NOENT) => return Ok(Existing::Missing),
        Err(Errno::LOOP | Errno::NOTDIR) => return Ok(Existing::Different),
        Err(error) => return Err(format!("cannot open ReviewId directory: {error}")),
    };
    let mut reader = Dir::read_from(&opened)
        .map_err(|error| format!("cannot enumerate ReviewId directory: {error}"))?;
    let mut names = Vec::new();
    while let Some(entry) = reader.read() {
        let entry =
            entry.map_err(|error| format!("cannot enumerate ReviewId directory: {error}"))?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            names.push(name.to_vec());
        }
    }
    names.sort();
    if names != [b"manifest.json".to_vec(), b"packet.md".to_vec()] {
        return Ok(Existing::Different);
    }
    let actual_packet = bounded_file::read_regular_at(
        &opened,
        "packet.md".as_ref(),
        &directory.join("packet.md"),
        FILE_LIMIT,
        "Slice review packet",
    )?
    .ok_or_else(|| "ReviewId packet disappeared during inspection".to_owned())?;
    let actual_manifest = bounded_file::read_regular_at(
        &opened,
        "manifest.json".as_ref(),
        &directory.join("manifest.json"),
        FILE_LIMIT,
        "Slice review manifest",
    )?
    .ok_or_else(|| "ReviewId manifest disappeared during inspection".to_owned())?;
    let opened_stat = fstat(&opened)
        .map_err(|error| format!("cannot inspect opened ReviewId directory: {error}"))?;
    let target_stat = statat(&parent, target, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("cannot revalidate ReviewId directory entry: {error}"))?;
    if opened_stat.st_dev != target_stat.st_dev || opened_stat.st_ino != target_stat.st_ino {
        return Err("ReviewId directory changed during inspection".to_owned());
    }
    if actual_packet == packet && actual_manifest == manifest {
        Ok(Existing::Exact)
    } else {
        Ok(Existing::Different)
    }
}

#[cfg(test)]
pub(super) fn publish_with_test_hook(
    directory: &Path,
    packet: &[u8],
    manifest: &[u8],
    mut final_revalidate: impl FnMut() -> Result<(), String>,
    before_rename: impl FnMut() -> Result<(), String>,
) -> Result<&'static str, String> {
    publish_with_hook(
        directory,
        packet,
        manifest,
        &mut final_revalidate,
        before_rename,
    )
}

fn require_exact(directory: &Path, packet: &[u8], manifest: &[u8]) -> Result<(), String> {
    match inspect(directory, packet, manifest)? {
        Existing::Exact => Ok(()),
        Existing::Missing | Existing::Different => {
            Err("published ReviewId artifact set is missing, extra, or byte-mismatched".to_owned())
        },
    }
}

fn create_temporary(
    parent: &std::os::fd::OwnedFd,
    target: &std::ffi::OsStr,
    directory: &Path,
) -> Result<OsString, String> {
    for _ in 0..1024 {
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let mut temporary = OsString::from(".");
        temporary.push(target);
        temporary.push(format!(".yo-prepare-{}-{sequence}", std::process::id()));
        match mkdirat(parent, &temporary, Mode::from_raw_mode(0o700)) {
            Ok(()) => return Ok(temporary),
            Err(Errno::EXIST) => continue,
            Err(error) => {
                return Err(format!(
                    "cannot prepare review packet {}: {error}",
                    directory.display()
                ));
            },
        }
    }
    Err("cannot allocate a unique prepared review packet directory".to_owned())
}

fn write_file(directory: &std::os::fd::OwnedFd, name: &str, bytes: &[u8]) -> Result<(), String> {
    let fd = openat(directory, name, FILE_FLAGS, Mode::from_raw_mode(0o600))
        .map_err(|error| format!("cannot create prepared review {name}: {error}"))?;
    let mut file = File::from(fd);
    file.write_all(bytes)
        .map_err(|error| format!("cannot write prepared review {name}: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("cannot sync prepared review {name}: {error}"))
}

fn cleanup(
    parent: &std::os::fd::OwnedFd,
    temporary: &std::ffi::OsStr,
    directory: &std::os::fd::OwnedFd,
) {
    let _ = unlinkat(directory, "packet.md", AtFlags::empty());
    let _ = unlinkat(directory, "manifest.json", AtFlags::empty());
    let _ = unlinkat(parent, temporary, AtFlags::REMOVEDIR);
}

fn cleanup_path(parent: &std::os::fd::OwnedFd, temporary: &std::ffi::OsStr) {
    if let Ok(directory) = openat(parent, temporary, DIRECTORY_FLAGS, Mode::empty()) {
        cleanup(parent, temporary, &directory);
    }
}
