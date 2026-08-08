use std::{
    collections::HashSet,
    error::Error,
    fmt, fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Deserializer, de};

use super::{AccountId, ApiCredential, CredentialStore, ProviderId};

const MAX_CREDENTIAL_FILE_BYTES: u64 = 64 * 1024;

/// Loads the immutable account credential snapshot used for one process run.
pub struct LocalCredentialStore;

impl LocalCredentialStore {
    pub fn open(path: impl AsRef<Path>) -> Result<CredentialStore, LocalCredentialStoreError> {
        let path = path.as_ref();
        let mut file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CredentialStore::default());
            },
            Err(source) => return Err(LocalCredentialStoreError::io(path, source)),
        };

        let before = metadata_snapshot(path, &file)?;
        validate_metadata(path, &before)?;

        let mut bytes = Vec::with_capacity(
            usize::try_from(before.len.min(MAX_CREDENTIAL_FILE_BYTES))
                .unwrap_or(MAX_CREDENTIAL_FILE_BYTES as usize),
        );
        file.by_ref()
            .take(MAX_CREDENTIAL_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| LocalCredentialStoreError::io(path, source))?;
        if bytes.len() as u64 > MAX_CREDENTIAL_FILE_BYTES {
            return Err(LocalCredentialStoreError::TooLarge(path.to_owned()));
        }

        let after = metadata_snapshot(path, &file)?;
        validate_stable_metadata(path, &before, &after)?;

        let contents = String::from_utf8(bytes)
            .map_err(|_| LocalCredentialStoreError::InvalidContents(path.to_owned()))?;
        parse(path, &contents)
    }
}

#[derive(Debug)]
pub enum LocalCredentialStoreError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    UnsupportedFileType(PathBuf),
    WrongOwner(PathBuf),
    InsecurePermissions(PathBuf),
    TooLarge(PathBuf),
    Changed(PathBuf),
    InvalidContents(PathBuf),
    UnsupportedVersion {
        path: PathBuf,
        version: u32,
    },
}

impl LocalCredentialStoreError {
    fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }
}

impl fmt::Display for LocalCredentialStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::UnsupportedFileType(path) => {
                write!(
                    formatter,
                    "{} is not a regular credential file",
                    path.display()
                )
            },
            Self::WrongOwner(path) => write!(
                formatter,
                "{} is not owned by the current effective user",
                path.display()
            ),
            Self::InsecurePermissions(path) => write!(
                formatter,
                "{} must not grant group or other permissions",
                path.display()
            ),
            Self::TooLarge(path) => write!(
                formatter,
                "{} exceeds the {MAX_CREDENTIAL_FILE_BYTES}-byte credential-file limit",
                path.display()
            ),
            Self::Changed(path) => write!(
                formatter,
                "{} changed while its credential snapshot was being read",
                path.display()
            ),
            Self::InvalidContents(path) => {
                write!(formatter, "{} contains invalid credentials", path.display())
            },
            Self::UnsupportedVersion { path, version } => write!(
                formatter,
                "{} uses unsupported credential version {version}; expected 1",
                path.display()
            ),
        }
    }
}

impl Error for LocalCredentialStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::UnsupportedFileType(_)
            | Self::WrongOwner(_)
            | Self::InsecurePermissions(_)
            | Self::TooLarge(_)
            | Self::Changed(_)
            | Self::InvalidContents(_)
            | Self::UnsupportedVersion { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MetadataSnapshot {
    device: u64,
    inode: u64,
    mode: u32,
    user: u32,
    group: u32,
    len: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

fn metadata_snapshot(
    path: &Path,
    file: &fs::File,
) -> Result<MetadataSnapshot, LocalCredentialStoreError> {
    let metadata = file
        .metadata()
        .map_err(|source| LocalCredentialStoreError::io(path, source))?;
    Ok(MetadataSnapshot {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        user: metadata.uid(),
        group: metadata.gid(),
        len: metadata.len(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    })
}

fn validate_metadata(
    path: &Path,
    metadata: &MetadataSnapshot,
) -> Result<(), LocalCredentialStoreError> {
    if metadata.mode & libc::S_IFMT != libc::S_IFREG {
        return Err(LocalCredentialStoreError::UnsupportedFileType(
            path.to_owned(),
        ));
    }
    if metadata.user != rustix::process::geteuid().as_raw() {
        return Err(LocalCredentialStoreError::WrongOwner(path.to_owned()));
    }
    if metadata.mode & 0o077 != 0 {
        return Err(LocalCredentialStoreError::InsecurePermissions(
            path.to_owned(),
        ));
    }
    if metadata.len > MAX_CREDENTIAL_FILE_BYTES {
        return Err(LocalCredentialStoreError::TooLarge(path.to_owned()));
    }
    Ok(())
}

fn validate_stable_metadata(
    path: &Path,
    before: &MetadataSnapshot,
    after: &MetadataSnapshot,
) -> Result<(), LocalCredentialStoreError> {
    if before != after {
        return Err(LocalCredentialStoreError::Changed(path.to_owned()));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialFile {
    version: u32,
    #[serde(deserialize_with = "deserialize_providers")]
    providers: Vec<((ProviderId, AccountId), ApiCredential)>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountCredential {
    api_key: String,
}

struct ProviderCredentials(Vec<(AccountId, ApiCredential)>);

impl<'de> Deserialize<'de> for ProviderCredentials {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct AccountsVisitor;

        impl<'de> de::Visitor<'de> for AccountsVisitor {
            type Value = ProviderCredentials;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an AccountId-keyed credential mapping")
            }

            fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut accounts = Vec::with_capacity(mapping.size_hint().unwrap_or(0));
                let mut seen = HashSet::new();
                while let Some(account) = mapping.next_key::<String>()? {
                    let credential = mapping.next_value::<AccountCredential>()?;
                    let account = AccountId::new(account).map_err(de::Error::custom)?;
                    if !seen.insert(account.clone()) {
                        return Err(de::Error::custom("duplicate AccountId inside Provider"));
                    }
                    let credential =
                        ApiCredential::new(credential.api_key).map_err(de::Error::custom)?;
                    accounts.push((account, credential));
                }
                Ok(ProviderCredentials(accounts))
            }
        }

        deserializer.deserialize_map(AccountsVisitor)
    }
}

type ScopedCredential = ((ProviderId, AccountId), ApiCredential);

fn deserialize_providers<'de, D>(deserializer: D) -> Result<Vec<ScopedCredential>, D::Error>
where
    D: Deserializer<'de>,
{
    struct ProvidersVisitor;

    impl<'de> de::Visitor<'de> for ProvidersVisitor {
        type Value = Vec<ScopedCredential>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a ProviderId-keyed credential mapping")
        }

        fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
        where
            A: de::MapAccess<'de>,
        {
            let mut credentials = Vec::new();
            let mut seen = HashSet::new();
            while let Some(provider) = mapping.next_key::<String>()? {
                let provider_accounts = mapping.next_value::<ProviderCredentials>()?;
                let provider = ProviderId::new(provider).map_err(de::Error::custom)?;
                if !seen.insert(provider.clone()) {
                    return Err(de::Error::custom("duplicate ProviderId"));
                }
                credentials.extend(
                    provider_accounts
                        .0
                        .into_iter()
                        .map(|(account, credential)| ((provider.clone(), account), credential)),
                );
            }
            Ok(credentials)
        }
    }

    deserializer.deserialize_map(ProvidersVisitor)
}

fn parse(path: &Path, contents: &str) -> Result<CredentialStore, LocalCredentialStoreError> {
    let decoded: CredentialFile = serde_norway::from_str(contents)
        .map_err(|_| LocalCredentialStoreError::InvalidContents(path.to_owned()))?;
    if decoded.version != 1 {
        return Err(LocalCredentialStoreError::UnsupportedVersion {
            path: path.to_owned(),
            version: decoded.version,
        });
    }
    CredentialStore::new(decoded.providers)
        .map_err(|_| LocalCredentialStoreError::InvalidContents(path.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secure_snapshot() -> MetadataSnapshot {
        MetadataSnapshot {
            device: 1,
            inode: 2,
            mode: libc::S_IFREG | 0o600,
            user: rustix::process::geteuid().as_raw(),
            group: 3,
            len: 100,
            modified_seconds: 4,
            modified_nanoseconds: 5,
            changed_seconds: 6,
            changed_nanoseconds: 7,
        }
    }

    // exact handle의 owner가 현재 effective user와 다르면 다른 metadata가 모두 안전해도
    // WrongOwner로 거절해야 다른 local 계정의 credential을 읽지 않는지 검증합니다.
    #[test]
    fn rejects_metadata_owned_by_another_effective_user() {
        let mut metadata = secure_snapshot();
        metadata.user = metadata.user.wrapping_add(1);
        let path = Path::new("credentials.yaml");

        assert!(matches!(
            validate_metadata(path, &metadata),
            Err(LocalCredentialStoreError::WrongOwner(rejected)) if rejected == path
        ));
    }

    // 같은 handle에서 read 전후 snapshot의 identity·권한·크기·시각 중 하나라도 달라지면
    // captured byte를 사용하지 않고 Changed로 거절하는지 길이 변경 사례로 검증합니다.
    #[test]
    fn rejects_metadata_that_changes_while_the_handle_is_read() {
        let before = secure_snapshot();
        let mut after = before.clone();
        after.len += 1;
        let path = Path::new("credentials.yaml");

        assert!(matches!(
            validate_stable_metadata(path, &before, &after),
            Err(LocalCredentialStoreError::Changed(rejected)) if rejected == path
        ));
    }
}
