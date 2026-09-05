use std::{
    ffi::OsString,
    fs::File,
    os::{fd::OwnedFd, unix::fs::MetadataExt},
    sync::{Mutex, OnceLock},
};

use nix::{
    fcntl::{OFlag, openat},
    sys::stat::{Mode, SFlag, fstat},
};
use yo_core::ToolExecutionError;

static NEW_FILE_MODE: OnceLock<u32> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FileIdentity {
    pub(super) device: u64,
    pub(super) inode: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OpenRegularError {
    Unavailable,
    NotRegular,
}

pub(super) fn initialize_process_file_mode() {
    NEW_FILE_MODE.get_or_init(capture_new_file_mode);
}

pub(super) fn new_file_mode() -> u32 {
    *NEW_FILE_MODE.get_or_init(capture_new_file_mode)
}

pub(super) fn open_beneath(
    workspace: &File,
    components: &[OsString],
    final_flags: OFlag,
) -> Result<OwnedFd, ToolExecutionError> {
    let mut current: OwnedFd = workspace
        .try_clone()
        .map_err(|_| ToolExecutionError::new("workspace handle is unavailable"))?
        .into();
    for (index, component) in components.iter().enumerate() {
        let is_final = index + 1 == components.len();
        let flags = OFlag::O_CLOEXEC
            | OFlag::O_NOFOLLOW
            | if is_final {
                final_flags
            } else {
                OFlag::O_RDONLY | OFlag::O_DIRECTORY
            };
        current = openat(&current, component.as_os_str(), flags, Mode::empty())
            .map_err(|_| ToolExecutionError::new("tool path is unavailable"))?;
    }
    Ok(current)
}

pub(super) fn open_regular_file(
    workspace: &File,
    components: &[OsString],
    denied_credential: Option<FileIdentity>,
) -> Result<File, OpenRegularError> {
    let descriptor = open_beneath(workspace, components, OFlag::O_RDONLY | OFlag::O_NONBLOCK)
        .map_err(|_| OpenRegularError::Unavailable)?;
    let metadata = fstat(&descriptor).map_err(|_| OpenRegularError::Unavailable)?;
    if !SFlag::from_bits_truncate(metadata.st_mode).contains(SFlag::S_IFREG) {
        return Err(OpenRegularError::NotRegular);
    }
    if denied_credential
        == Some(FileIdentity {
            device: normalize_device_id(metadata.st_dev),
            inode: metadata.st_ino,
        })
    {
        return Err(OpenRegularError::Unavailable);
    }
    Ok(File::from(descriptor))
}

pub(super) fn file_identity(file: &File) -> Option<FileIdentity> {
    file.metadata().ok().map(|metadata| FileIdentity {
        device: normalize_device_id(metadata.dev()),
        inode: metadata.ino(),
    })
}

fn capture_new_file_mode() -> u32 {
    static UMASK_LOCK: Mutex<()> = Mutex::new(());
    let _guard = UMASK_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let current = nix::sys::stat::umask(Mode::empty());
    nix::sys::stat::umask(current);
    permission_mode_u32(0o666 & !current.bits())
}

pub(super) fn permission_mode_u32(mode: impl Into<u32>) -> u32 {
    mode.into()
}

#[cfg(target_vendor = "apple")]
pub(super) const fn normalize_device_id(device: libc::dev_t) -> u64 {
    device as u64
}

#[cfg(not(target_vendor = "apple"))]
pub(super) const fn normalize_device_id(device: libc::dev_t) -> u64 {
    device
}

#[cfg(test)]
mod tests {
    #[cfg(target_vendor = "apple")]
    use super::normalize_device_id;

    // Apple의 signed dev_t가 high bit를 가진 경우에도 MetadataExt::dev와 같은 u64
    // 비트 표현을 보존해 credential identity 비교가 정상 파일을 거절하지 않습니다.
    #[cfg(target_vendor = "apple")]
    // signed dev_t의 high bit가 설정된 조건에서 u64 전체 비트가 보존되는 결과를 확인해 credential
    // identity 비교 경계를 보호합니다.
    #[test]
    fn apple_device_identity_preserves_the_complete_signed_domain() {
        assert_eq!(normalize_device_id(-1), u64::MAX);
        assert_eq!(normalize_device_id(i32::MIN), i32::MIN as u64);
    }
}
