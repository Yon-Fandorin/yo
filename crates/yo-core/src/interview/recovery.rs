use std::{
    ffi::OsStr,
    fs::File,
    io::{Error, ErrorKind, Read, Write},
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    str,
    time::{Duration, SystemTime},
};

use chacha20::{
    XChaCha20,
    cipher::{KeyIvInit, StreamCipher},
};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce, aead::AeadInPlace};
use poly1305::{Poly1305, universal_hash::UniversalHash};
use rustix::{
    fs::{self, Dir, Mode, OFlags},
    io::Errno,
    process,
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::{InterviewError, invalid, working_copy::valid_id};
use crate::{AccountId, HostId, ModelId, ProviderId, SecretInput};

const FORMAT: &str = "yo.secret-recovery-entry/v1";
const MAGIC: &[u8; 8] = b"YOSRVLT1";
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
const LENGTH_BYTES: usize = 4;
const TAG_BYTES: usize = 16;
const PLAINTEXT_BYTES: usize = LENGTH_BYTES + SecretInput::MAX_BYTES;
pub(super) const ENTRY_BYTES: usize = MAGIC.len() + NONCE_BYTES + PLAINTEXT_BYTES + TAG_BYTES;
pub(super) const EXPIRY: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Stable, non-secret live destination evidence used by the recovery binding.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SecretRecoveryDestination {
    Managed {
        provider: String,
        model: String,
        authenticated_account: String,
    },
    Delegated {
        host: String,
        provider: String,
        model: String,
        authenticated_account: String,
    },
}

impl SecretRecoveryDestination {
    /// Callers may use this only for an account identity observed from live authentication.
    #[must_use]
    pub fn managed(
        provider: &ProviderId,
        model: &ModelId,
        authenticated_account: &AccountId,
    ) -> Self {
        Self::Managed {
            provider: provider.as_str().to_owned(),
            model: model.as_str().to_owned(),
            authenticated_account: authenticated_account.as_str().to_owned(),
        }
    }

    /// The account and Provider must come from the same live delegated binding evidence.
    #[must_use]
    pub fn delegated(
        host: &HostId,
        provider: &ProviderId,
        model: &ModelId,
        authenticated_account: &AccountId,
    ) -> Self {
        Self::Delegated {
            host: host.as_str().to_owned(),
            provider: provider.as_str().to_owned(),
            model: model.as_str().to_owned(),
            authenticated_account: authenticated_account.as_str().to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SecretRecoveryReference {
    pub(super) question_id: String,
    pub(super) entry_id: String,
    pub(super) state: SecretRecoveryState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SecretRecoveryState {
    RecoveryAvailable,
}

impl SecretRecoveryReference {
    pub(super) fn new(question_id: String, entry_id: String) -> Self {
        Self {
            question_id,
            entry_id,
            state: SecretRecoveryState::RecoveryAvailable,
        }
    }

    pub(super) fn validate(&self) -> Result<(), InterviewError> {
        if self.question_id.is_empty() || !valid_id(&self.entry_id) {
            return Err(invalid("invalid secret recovery reference"));
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct AssociatedData<'a> {
    format: &'static str,
    entry_id: &'a str,
    copy_id: &'a str,
    batch_fingerprint: &'a str,
    question_id: &'a str,
    destination: &'a SecretRecoveryDestination,
}

pub(super) struct RecoveryBinding<'a> {
    pub(super) entry_id: &'a str,
    pub(super) copy_id: &'a str,
    pub(super) batch_fingerprint: &'a str,
    pub(super) question_id: &'a str,
    pub(super) destination: &'a SecretRecoveryDestination,
}

impl RecoveryBinding<'_> {
    fn aad(&self) -> Result<Vec<u8>, InterviewError> {
        serde_json::to_vec(&AssociatedData {
            format: FORMAT,
            entry_id: self.entry_id,
            copy_id: self.copy_id,
            batch_fingerprint: self.batch_fingerprint,
            question_id: self.question_id,
            destination: self.destination,
        })
        .map_err(|error| invalid(format!("cannot encode secret recovery binding: {error}")))
    }
}

#[derive(Debug)]
pub(super) struct RecoveryStore {
    vault_path: PathBuf,
    key_path: PathBuf,
}

pub(super) struct WrittenEntry {
    pub(super) id: String,
    device: u64,
    inode: u64,
}

impl RecoveryStore {
    pub(super) fn new(vault_path: PathBuf, key_path: PathBuf) -> Result<Self, InterviewError> {
        validate_absolute(&vault_path, "secret recovery vault")?;
        validate_absolute(&key_path, "secret recovery key")?;
        if vault_path == key_path || key_path.file_name().is_none() {
            return Err(invalid("invalid secret recovery storage paths"));
        }
        Ok(Self {
            vault_path,
            key_path,
        })
    }

    pub(super) fn boundary(&self) -> String {
        format!(
            "encrypted vault {} with its dedicated key {}",
            self.vault_path.display(),
            self.key_path.display()
        )
    }

    pub(super) fn write(
        &self,
        copy_id: &str,
        batch_fingerprint: &str,
        question_id: &str,
        destination: &SecretRecoveryDestination,
        secret: &SecretInput,
    ) -> Result<WrittenEntry, InterviewError> {
        let vault = open_absolute_directory(&self.vault_path, true, "secret recovery vault")?;
        let key = self.read_key(true, Some(&vault))?;
        let mut nonce = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce)
            .map_err(|error| invalid(format!("generating a recovery nonce failed: {error}")))?;

        let bytes = secret.expose().as_bytes();
        let mut plaintext = Zeroizing::new(vec![0_u8; PLAINTEXT_BYTES]);
        plaintext[..LENGTH_BYTES].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
        plaintext[LENGTH_BYTES..LENGTH_BYTES + bytes.len()].copy_from_slice(bytes);
        let id = super::working_copy::new_id()?;
        let aad = RecoveryBinding {
            entry_id: &id,
            copy_id,
            batch_fingerprint,
            question_id,
            destination,
        }
        .aad()?;
        let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
            .map_err(|_| invalid("invalid secret recovery key"))?;
        let tag = cipher
            .encrypt_in_place_detached(XNonce::from_slice(&nonce), &aad, plaintext.as_mut())
            .map_err(|_| invalid("encrypting secret recovery entry failed"))?;
        let name = entry_name(&id);
        let mut file = File::from(
            fs::openat(
                &vault,
                name.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(Error::from)?,
        );
        let created = file.metadata()?;
        let result = (|| {
            secure_file(&file)?;
            file.write_all(MAGIC)?;
            file.write_all(&nonce)?;
            file.write_all(plaintext.as_ref())?;
            file.write_all(tag.as_slice())?;
            file.sync_all()?;
            secure_file(&file)?;
            let metadata = file.metadata()?;
            vault.sync_all()?;
            Ok(WrittenEntry {
                id,
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        })();
        if result.is_err() {
            let _ = delete_named_if_same(
                &vault,
                OsStr::new(&name),
                created.dev(),
                created.ino(),
                "secret recovery entry",
            );
        }
        result
    }

    pub(super) fn read(&self, binding: RecoveryBinding<'_>) -> Result<SecretInput, InterviewError> {
        let value = self.decrypt(binding)?;
        let value = String::from_utf8(value.to_vec())
            .map_err(|_| invalid("secret recovery entry is not UTF-8"))?;
        SecretInput::new(value).map_err(|error| invalid(error.to_string()))
    }

    fn decrypt(&self, binding: RecoveryBinding<'_>) -> Result<Zeroizing<Vec<u8>>, InterviewError> {
        let vault = open_absolute_directory(&self.vault_path, false, "secret recovery vault")?;
        let key = self.read_key(false, Some(&vault))?;
        let mut bytes = self.read_entry(&vault, binding.entry_id)?;
        if &bytes[..MAGIC.len()] != MAGIC {
            return Err(invalid("secret recovery entry has an unsupported format"));
        }
        let nonce_start = MAGIC.len();
        let body_start = nonce_start + NONCE_BYTES;
        let tag_start = body_start + PLAINTEXT_BYTES;
        let nonce_bytes: [u8; NONCE_BYTES] = bytes[nonce_start..body_start]
            .try_into()
            .expect("fixed nonce length");
        let tag_bytes: [u8; TAG_BYTES] = bytes[tag_start..].try_into().expect("fixed tag length");
        let nonce = XNonce::from_slice(&nonce_bytes);
        let tag = chacha20poly1305::Tag::from_slice(&tag_bytes);
        let aad = binding.aad()?;
        let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
            .map_err(|_| invalid("invalid secret recovery key"))?;
        if cipher
            .decrypt_in_place_detached(nonce, &aad, &mut bytes[body_start..tag_start], tag)
            .is_err()
        {
            return Err(invalid(
                "secret recovery authentication or destination matching failed",
            ));
        }
        let length = u32::from_be_bytes(
            bytes[body_start..body_start + LENGTH_BYTES]
                .try_into()
                .expect("fixed length prefix"),
        ) as usize;
        if length > SecretInput::MAX_BYTES {
            return Err(invalid("secret recovery entry has an invalid length"));
        }
        let start = body_start + LENGTH_BYTES;
        let value = Zeroizing::new(bytes[start..start + length].to_vec());
        str::from_utf8(value.as_ref())
            .map_err(|_| invalid("secret recovery entry is not UTF-8"))?;
        Ok(value)
    }

    pub(super) fn available(&self, id: &str) -> Result<bool, InterviewError> {
        let vault = match open_absolute_directory(&self.vault_path, false, "secret recovery vault")
        {
            Ok(vault) => vault,
            Err(InterviewError::Io(error)) if error.kind() == ErrorKind::NotFound => {
                return Ok(false);
            },
            Err(error) => return Err(error),
        };
        let _key = self.read_key(false, Some(&vault))?;
        let file = match open_entry(&vault, id) {
            Ok(file) => file,
            Err(InterviewError::Io(error)) if error.kind() == ErrorKind::NotFound => {
                return Ok(false);
            },
            Err(error) => return Err(error),
        };
        Ok(!expired(&file)? && file.metadata()?.len() == ENTRY_BYTES as u64)
    }

    /// Authenticates the complete entry and its exact live binding without
    /// returning the plaintext to the caller.
    pub(super) fn authenticate(&self, binding: RecoveryBinding<'_>) -> Result<(), InterviewError> {
        let vault = open_absolute_directory(&self.vault_path, false, "secret recovery vault")?;
        let key = self.read_key(false, Some(&vault))?;
        let bytes = self.read_entry(&vault, binding.entry_id)?;
        if &bytes[..MAGIC.len()] != MAGIC {
            return Err(invalid("secret recovery entry has an unsupported format"));
        }
        let nonce_start = MAGIC.len();
        let body_start = nonce_start + NONCE_BYTES;
        let tag_start = body_start + PLAINTEXT_BYTES;
        let nonce = chacha20::XNonce::from_slice(&bytes[nonce_start..body_start]);
        let tag = chacha20poly1305::Tag::from_slice(&bytes[tag_start..]);
        let aad = binding.aad()?;
        authenticate_xchacha20poly1305(
            key.as_ref(),
            nonce,
            &aad,
            &bytes[body_start..tag_start],
            tag,
        )
    }

    pub(super) fn expired(&self, id: &str) -> Result<bool, InterviewError> {
        let vault = open_absolute_directory(&self.vault_path, false, "secret recovery vault")?;
        expired(&open_entry(&vault, id)?)
    }

    pub(super) fn delete(&self, id: &str) -> Result<(), InterviewError> {
        let vault = open_absolute_directory(&self.vault_path, false, "secret recovery vault")?;
        let file = match open_entry(&vault, id) {
            Ok(file) => file,
            Err(InterviewError::Io(error)) if error.kind() == ErrorKind::NotFound => {
                return Ok(());
            },
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        delete_if_same(&vault, id, metadata.dev(), metadata.ino())
    }

    pub(super) fn delete_written(&self, entry: &WrittenEntry) -> Result<(), InterviewError> {
        let vault = open_absolute_directory(&self.vault_path, false, "secret recovery vault")?;
        delete_if_same(&vault, &entry.id, entry.device, entry.inode)
    }

    fn read_entry(&self, vault: &File, id: &str) -> Result<Zeroizing<Vec<u8>>, InterviewError> {
        let file = open_entry(vault, id)?;
        if expired(&file)? {
            return Err(invalid("secret recovery entry expired after seven days"));
        }
        if file.metadata()?.len() != ENTRY_BYTES as u64 {
            return Err(invalid("secret recovery entry has an invalid size"));
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(ENTRY_BYTES));
        file.take((ENTRY_BYTES + 1) as u64)
            .read_to_end(bytes.as_mut())?;
        if bytes.len() != ENTRY_BYTES {
            return Err(invalid("secret recovery entry has an invalid size"));
        }
        Ok(bytes)
    }

    fn read_key(
        &self,
        create: bool,
        vault: Option<&File>,
    ) -> Result<Zeroizing<[u8; KEY_BYTES]>, InterviewError> {
        let parent_path = self
            .key_path
            .parent()
            .ok_or_else(|| invalid("secret recovery key has no parent directory"))?;
        let parent = open_absolute_directory(parent_path, create, "secret recovery key parent")?;
        let name = self
            .key_path
            .file_name()
            .ok_or_else(|| invalid("secret recovery key has no filename"))?;
        let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
        let fd = match fs::openat(&parent, name, flags, Mode::empty()) {
            Err(Errno::NOENT) if create => {
                if let Some(vault) = vault
                    && vault_has_entries(vault)?
                {
                    return Err(invalid(
                        "secret recovery key is missing while vault entries survive",
                    ));
                }
                return create_key(&parent, name);
            },
            value => value.map_err(Error::from)?,
        };
        read_existing_key(File::from(fd))
    }
}

/// Verifies the RFC 8439/XChaCha20-Poly1305 tag over ciphertext and AAD.
///
/// The standard AEAD decrypt API verifies first and then applies the message
/// keystream in-place. Availability must stop at the first step so plaintext
/// cannot exist before the user's explicit recovery action.
fn authenticate_xchacha20poly1305(
    key: &[u8],
    nonce: &chacha20::XNonce,
    associated_data: &[u8],
    ciphertext: &[u8],
    expected_tag: &chacha20poly1305::Tag,
) -> Result<(), InterviewError> {
    if ciphertext.len() / 64 >= u32::MAX as usize {
        return Err(invalid("secret recovery ciphertext is too large"));
    }
    let mut stream = XChaCha20::new(chacha20::Key::from_slice(key), nonce);
    let mut mac_key = poly1305::Key::default();
    stream.apply_keystream(mac_key.as_mut_slice());
    let mut mac = Poly1305::new(&mac_key);
    mac_key.as_mut_slice().zeroize();
    mac.update_padded(associated_data);
    mac.update_padded(ciphertext);

    let aad_len = u64::try_from(associated_data.len())
        .map_err(|_| invalid("secret recovery associated data is too large"))?;
    let ciphertext_len = u64::try_from(ciphertext.len())
        .map_err(|_| invalid("secret recovery ciphertext is too large"))?;
    let mut lengths = poly1305::Block::default();
    lengths[..8].copy_from_slice(&aad_len.to_le_bytes());
    lengths[8..].copy_from_slice(&ciphertext_len.to_le_bytes());
    mac.update(&[lengths]);
    mac.verify(expected_tag)
        .map_err(|_| invalid("secret recovery authentication or destination matching failed"))
}

fn create_key(parent: &File, name: &OsStr) -> Result<Zeroizing<[u8; KEY_BYTES]>, InterviewError> {
    let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
    getrandom::fill(key.as_mut())
        .map_err(|error| invalid(format!("generating a recovery key failed: {error}")))?;
    let mut file = File::from(
        fs::openat(
            parent,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(Error::from)?,
    );
    let created = file.metadata()?;
    let result = (|| {
        secure_file(&file)?;
        file.write_all(key.as_ref())?;
        file.sync_all()?;
        secure_file(&file)?;
        parent.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = delete_named_if_same(
            parent,
            name,
            created.dev(),
            created.ino(),
            "secret recovery key",
        );
        return result.map(|()| key);
    }
    Ok(key)
}

fn read_existing_key(mut file: File) -> Result<Zeroizing<[u8; KEY_BYTES]>, InterviewError> {
    secure_file(&file)?;
    if file.metadata()?.len() != KEY_BYTES as u64 {
        return Err(invalid("secret recovery key must contain exactly 32 bytes"));
    }
    let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
    file.read_exact(key.as_mut())?;
    let mut extra = [0_u8; 1];
    if file.read(&mut extra)? != 0 {
        return Err(invalid("secret recovery key must contain exactly 32 bytes"));
    }
    Ok(key)
}

fn open_entry(vault: &File, id: &str) -> Result<File, InterviewError> {
    if !valid_id(id) {
        return Err(invalid("invalid secret recovery entry identity"));
    }
    let file = File::from(
        fs::openat(
            vault,
            entry_name(id),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(Error::from)?,
    );
    secure_file(&file)?;
    Ok(file)
}

fn delete_if_same(vault: &File, id: &str, device: u64, inode: u64) -> Result<(), InterviewError> {
    delete_named_if_same(
        vault,
        OsStr::new(&entry_name(id)),
        device,
        inode,
        "secret recovery entry",
    )
}

fn delete_named_if_same(
    parent: &File,
    name: &OsStr,
    device: u64,
    inode: u64,
    label: &str,
) -> Result<(), InterviewError> {
    let current = match fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(file) => File::from(file),
        Err(Errno::NOENT) => return Ok(()),
        Err(error) => return Err(Error::from(error).into()),
    };
    secure_file(&current)?;
    let metadata = current.metadata()?;
    if metadata.dev() != device || metadata.ino() != inode {
        return Err(invalid(format!("{label} identity changed before deletion")));
    }
    fs::unlinkat(parent, name, fs::AtFlags::empty()).map_err(Error::from)?;
    parent.sync_all()?;
    Ok(())
}

fn entry_name(id: &str) -> String {
    format!("{id}.entry")
}

fn expired(file: &File) -> Result<bool, InterviewError> {
    let modified = file.metadata()?.modified()?;
    Ok(SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age >= EXPIRY))
}

fn vault_has_entries(vault: &File) -> Result<bool, InterviewError> {
    for (index, entry) in Dir::read_from(vault).map_err(Error::from)?.enumerate() {
        if index >= 4096 {
            return Err(invalid("secret recovery vault exceeds its read limit"));
        }
        let entry = entry.map_err(Error::from)?;
        let name = entry.file_name();
        if name.to_bytes() != b"." && name.to_bytes() != b".." {
            // Any surviving object makes key regeneration unsafe. Unknown and
            // malformed entries remain untouched rather than being interpreted
            // as an empty vault.
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_absolute(path: &Path, label: &str) -> Result<(), InterviewError> {
    if !path.is_absolute()
        || path
            .components()
            .skip(1)
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid(format!(
            "{label} path must be absolute and normalized"
        )));
    }
    Ok(())
}

fn open_absolute_directory(
    path: &Path,
    create_final: bool,
    label: &str,
) -> Result<File, InterviewError> {
    validate_absolute(path, label)?;
    let components = path.components().collect::<Vec<_>>();
    let mut directory = File::from(
        fs::open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(Error::from)?,
    );
    for (index, component) in components.iter().enumerate().skip(1) {
        let Component::Normal(name) = component else {
            return Err(invalid(format!("invalid {label} path")));
        };
        let last = index + 1 == components.len();
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let fd = match fs::openat(&directory, *name, flags, Mode::empty()) {
            Err(Errno::NOENT) if create_final && last => {
                match fs::mkdirat(&directory, *name, Mode::from_raw_mode(0o700)) {
                    Ok(()) | Err(Errno::EXIST) => {},
                    Err(error) => return Err(Error::from(error).into()),
                }
                let fd =
                    fs::openat(&directory, *name, flags, Mode::empty()).map_err(Error::from)?;
                directory.sync_all()?;
                fd
            },
            value => value.map_err(Error::from)?,
        };
        directory = File::from(fd);
    }
    secure_directory(&directory)?;
    Ok(directory)
}

fn secure_directory(file: &File) -> Result<(), InterviewError> {
    let metadata = file.metadata()?;
    if !metadata.is_dir()
        || metadata.uid() != process::geteuid().as_raw()
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err(invalid(
            "secret recovery directories must be current-user-owned mode 0700 directories",
        ));
    }
    Ok(())
}

fn secure_file(file: &File) -> Result<(), InterviewError> {
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != process::geteuid().as_raw()
        || metadata.mode() & 0o7777 != 0o600
    {
        return Err(invalid(
            "secret recovery files must be current-user-owned one-link mode 0600 regular files",
        ));
    }
    Ok(())
}
