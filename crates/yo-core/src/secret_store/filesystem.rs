use std::{
    fs::File,
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    path::{Component, Path},
    str,
};

use rustix::{
    fs::{self, Mode, OFlags},
    io::Errno,
    process,
};

use super::{Result, SecretStoreError};

pub(super) fn secure_file(file: &File) -> Result<()> {
    let m = file.metadata()?;
    if !m.is_file()
        || m.nlink() != 1
        || m.uid() != process::geteuid().as_raw()
        || m.mode() & 0o7777 != 0o600
    {
        return Err(SecretStoreError);
    }
    Ok(())
}
fn secure_directory(file: &File) -> Result<()> {
    let m = file.metadata()?;
    if !m.is_dir() || m.uid() != process::geteuid().as_raw() || m.mode() & 0o7777 != 0o700 {
        return Err(SecretStoreError);
    }
    Ok(())
}
pub(super) fn absolute_directory(path: &Path) -> Result<File> {
    if !path.is_absolute()
        || path
            .components()
            .skip(1)
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(SecretStoreError);
    }
    let mut dir = File::from(fs::open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    for component in path.components().skip(1) {
        let Component::Normal(name) = component else {
            return Err(SecretStoreError);
        };
        dir = File::from(fs::openat(
            &dir,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
    }
    secure_directory(&dir)?;
    Ok(dir)
}
pub(super) fn existing_directory(parent: &File, name: &str) -> Result<Option<File>> {
    let fd = match fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let file = File::from(fd);
    secure_directory(&file)?;
    Ok(Some(file))
}
pub(super) fn directory(parent: &File, name: &str, create: bool) -> Result<File> {
    if create {
        match fs::mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
            Ok(()) => parent.sync_all()?,
            Err(Errno::EXIST) => {},
            Err(e) => return Err(e.into()),
        }
    }
    let dir = File::from(fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    secure_directory(&dir)?;
    Ok(dir)
}
pub(super) fn open(parent: &File, name: &str) -> Result<Option<File>> {
    let fd = match fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let file = File::from(fd);
    secure_file(&file)?;
    Ok(Some(file))
}
pub(super) fn create(parent: &File, name: &str, bytes: &[u8]) -> Result<File> {
    let mut file = File::from(fs::openat(
        parent,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )?);
    secure_file(&file)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    parent.sync_all()?;
    Ok(file)
}
pub(super) fn read(parent: &File, name: &str, limit: usize) -> Result<Option<Vec<u8>>> {
    let Some(file) = open(parent, name)? else {
        return Ok(None);
    };
    if file.metadata()?.len() > limit as u64 {
        return Err(SecretStoreError);
    }
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(SecretStoreError);
    }
    Ok(Some(bytes))
}
pub(super) fn rename(parent: &File, from: &str, to: &str) -> Result<()> {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "redox"
    ))]
    {
        fs::renameat_with(parent, from, parent, to, fs::RenameFlags::NOREPLACE)?;
        parent.sync_all()?;
        Ok(())
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "redox"
    )))]
    {
        let _ = (parent, from, to);
        Err(SecretStoreError)
    }
}
pub(super) fn remove(parent: &File, name: &str) -> Result<()> {
    if let Some(file) = open(parent, name)? {
        let identity = file.metadata()?;
        let current = open(parent, name)?.ok_or(SecretStoreError)?.metadata()?;
        if identity.dev() != current.dev() || identity.ino() != current.ino() {
            return Err(SecretStoreError);
        }
        fs::unlinkat(parent, name, fs::AtFlags::empty())?;
    }
    parent.sync_all()?;
    Ok(())
}
pub(super) fn names(parent: &File) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for item in fs::Dir::read_from(parent)? {
        let item = item?;
        let bytes = item.file_name().to_bytes();
        if matches!(bytes, b"." | b"..") {
            continue;
        }
        if names.len() >= 4096 {
            return Err(SecretStoreError);
        }
        names.push(
            str::from_utf8(bytes)
                .map_err(|_| SecretStoreError)?
                .to_owned(),
        );
    }
    Ok(names)
}
pub(super) fn sync(parent: &File, name: &str) -> Result<()> {
    open(parent, name)?.ok_or(SecretStoreError)?.sync_all()?;
    parent.sync_all()?;
    Ok(())
}
