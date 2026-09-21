use std::{
    fs::{File, TryLockError},
    path::PathBuf,
};

use serde::{Serialize, de::DeserializeOwned};
use zeroize::Zeroizing;

use super::{
    Result, RetentionPolicy, SecretDestination, SecretMetadata, SecretStoreError,
    crypto::{self, Key},
    filesystem as fs,
    model::{self, Barrier, Generation, GenerationState, Operation},
};
use crate::SecretInput;

const GENERATION_FORMAT: &str = "yo.secret-metadata/v1";
const BARRIER_FORMAT: &str = "yo.secret-transition/v1";
const PUBLIC_LIMIT: usize = 65536;

/// 작업별 배타적 임대를 획득하는 별도 암호화 저장소입니다.
#[derive(Debug)]
pub struct SecretStore {
    state_root: PathBuf,
    key_path: PathBuf,
}
struct Lease {
    _lock: File,
    root: File,
    entries: File,
    metadata: File,
}

impl SecretStore {
    /// 기존 절대 상태 루트와 키 부모의 소유권·모드를 검증합니다. 키는 저장 시에만 생성합니다.
    pub fn open(state_root: PathBuf, key_path: PathBuf) -> Result<Self> {
        if !cfg!(any(
            target_os = "linux",
            target_os = "android",
            target_vendor = "apple",
            target_os = "redox"
        )) {
            return Err(SecretStoreError);
        }
        let _ = fs::absolute_directory(&state_root)?;
        if !key_path.is_absolute() || key_path.file_name().is_none() {
            return Err(SecretStoreError);
        }
        let _ = fs::absolute_directory(key_path.parent().ok_or(SecretStoreError)?)?;
        Ok(Self {
            state_root,
            key_path,
        })
    }

    fn lease(&self, create: bool) -> Result<Option<Lease>> {
        let root = fs::absolute_directory(&self.state_root)?;
        let home = if create {
            fs::directory(&root, "secret-store", true)?
        } else {
            let Some(home) = fs::existing_directory(&root, "secret-store")? else {
                return Ok(None);
            };
            home
        };
        let lock = match fs::open(&home, "lease")? {
            Some(file) => file,
            None if create => fs::create(&home, "lease", &[])?,
            None => return Err(SecretStoreError),
        };
        match lock.try_lock() {
            Ok(()) => {},
            Err(TryLockError::WouldBlock) | Err(TryLockError::Error(_)) => {
                return Err(SecretStoreError);
            },
        }
        let entries = fs::directory(&home, "entries", create)?;
        let metadata = fs::directory(&home, "metadata", create)?;
        Ok(Some(Lease {
            _lock: lock,
            root,
            entries,
            metadata,
        }))
    }

    fn key(&self, lease: &Lease, create: bool) -> Result<Option<Key>> {
        let parent = fs::absolute_directory(self.key_path.parent().ok_or(SecretStoreError)?)?;
        let name = self
            .key_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(SecretStoreError)?;
        if let Some(bytes) = fs::read(&parent, name, 32)? {
            let bytes = Zeroizing::new(bytes);
            if bytes.len() != 32 {
                return Err(SecretStoreError);
            }
            let mut value = Zeroizing::new([0; 32]);
            value.copy_from_slice(&bytes);
            return Ok(Some(value));
        }
        if !fs::names(&lease.entries)?.is_empty() || !fs::names(&lease.metadata)?.is_empty() {
            return Err(SecretStoreError);
        }
        if fs::names(&lease.root)?
            .iter()
            .any(|n| n == "secret-recovery")
            && !fs::names(&fs::directory(&lease.root, "secret-recovery", false)?)?.is_empty()
        {
            return Err(SecretStoreError);
        }
        if !create {
            return Ok(None);
        }
        let mut key = Zeroizing::new([0; 32]);
        getrandom::fill(key.as_mut()).map_err(|_| SecretStoreError)?;
        fs::create(&parent, name, key.as_ref())?;
        Ok(Some(key))
    }

    /// 선택한 정책을 현재 시각에 고정하고 새 암호문과 인증된 세대를 순서대로 발행합니다.
    pub fn save(
        &self,
        destination: &SecretDestination,
        scope: &str,
        title: &str,
        retention: RetentionPolicy,
        secret: &SecretInput,
        now: u64,
    ) -> Result<SecretMetadata> {
        let public = SecretMetadata::new(destination, scope, title, retention, now)?;
        let lease = self.lease(true)?.ok_or(SecretStoreError)?;
        let key = self.key(&lease, true)?.ok_or(SecretStoreError)?;
        let bucket = fs::directory(&lease.metadata, &bucket_name(destination, scope)?, true)?;
        let prior = recover_bucket(&lease, &bucket, &key, &bucket_name(destination, scope)?)?;
        let successor = Generation {
            format: GENERATION_FORMAT.to_owned(),
            state: GenerationState::Entry,
            generation: model::new_id()?,
            entry: Some(model::new_id()?),
            predecessor_generation: prior.as_ref().map(|g| g.generation.clone()),
            predecessor_entry: prior.as_ref().and_then(|g| g.entry.clone()),
            destination: destination.clone(),
            scope: scope.to_owned(),
            title: title.to_owned(),
            retention: Some(public.clone()),
        };
        fs::create(
            &lease.entries,
            &entry_name(&successor)?,
            &crypto::encrypt(&key, &successor, secret)?,
        )?;
        write_record(&bucket, &temporary_name(&successor), &key, &successor)?;
        let barrier = Barrier {
            format: BARRIER_FORMAT.to_owned(),
            operation: if prior.is_some() {
                Operation::Replace
            } else {
                Operation::Create
            },
            destination: destination.clone(),
            scope: scope.to_owned(),
            prior,
            successor: successor.clone(),
        };
        publish_barrier(&bucket, &key, &barrier)?;
        if barrier.prior.is_some() {
            fs::rename(&bucket, "current", "previous")?;
        }
        fs::rename(&bucket, &temporary_name(&successor), "current")?;
        recover_bucket(&lease, &bucket, &key, &bucket_name(destination, scope)?)?;
        Ok(public)
    }

    /// 인증된 공개 메타데이터만 반환하며 만료 항목은 삭제 프로토콜로 제거합니다.
    pub fn list(&self, now: u64) -> Result<Vec<SecretMetadata>> {
        let Some(lease) = self.lease(false)? else {
            return Ok(Vec::new());
        };
        let Some(key) = self.key(&lease, false)? else {
            return Ok(Vec::new());
        };
        let mut result = Vec::new();
        for name in fs::names(&lease.metadata)? {
            validate_bucket_name(&name)?;
            let bucket = fs::directory(&lease.metadata, &name, false)?;
            if let Some(generation) = recover_bucket(&lease, &bucket, &key, &name)? {
                if name != bucket_name(&generation.destination, &generation.scope)? {
                    return Err(SecretStoreError);
                }
                let public = generation.retention.as_ref().ok_or(SecretStoreError)?;
                if public.expires_at().is_some_and(|expiry| now >= expiry) {
                    delete_generation(&lease, &bucket, &key, generation)?;
                } else {
                    result.push(public.clone());
                }
            }
        }
        Ok(result)
    }

    /// 명시적 복구 동작에서만 일치하는 값의 복호문을 반환합니다.
    pub fn recover(
        &self,
        destination: &SecretDestination,
        scope: &str,
        now: u64,
    ) -> Result<Option<SecretInput>> {
        if !model::valid_scope(scope) {
            return Err(SecretStoreError);
        }
        let Some(lease) = self.lease(false)? else {
            return Ok(None);
        };
        let Some(key) = self.key(&lease, false)? else {
            return Ok(None);
        };
        let name = bucket_name(destination, scope)?;
        if !fs::names(&lease.metadata)?.contains(&name) {
            return Ok(None);
        }
        let bucket = fs::directory(&lease.metadata, &name, false)?;
        let Some(generation) = recover_bucket(&lease, &bucket, &key, &name)? else {
            return Ok(None);
        };
        if &generation.destination != destination || generation.scope != scope {
            return Err(SecretStoreError);
        }
        if generation
            .retention
            .as_ref()
            .ok_or(SecretStoreError)?
            .expires_at()
            .is_some_and(|expiry| now >= expiry)
        {
            delete_generation(&lease, &bucket, &key, generation)?;
            return Ok(None);
        }
        let bytes = fs::read(
            &lease.entries,
            &entry_name(&generation)?,
            crypto::ENTRY_BYTES,
        )?
        .ok_or(SecretStoreError)?;
        crypto::decrypt(&key, &generation, &bytes).map(Some)
    }

    /// 인증된 삭제 장벽을 먼저 발행하며 완료 전에는 성공을 반환하지 않습니다.
    pub fn delete(&self, destination: &SecretDestination, scope: &str) -> Result<()> {
        if !model::valid_scope(scope) {
            return Err(SecretStoreError);
        }
        let Some(lease) = self.lease(false)? else {
            return Ok(());
        };
        let Some(key) = self.key(&lease, false)? else {
            return Ok(());
        };
        let name = bucket_name(destination, scope)?;
        if !fs::names(&lease.metadata)?.contains(&name) {
            return Ok(());
        }
        let bucket = fs::directory(&lease.metadata, &name, false)?;
        if let Some(generation) = recover_bucket(&lease, &bucket, &key, &name)? {
            if &generation.destination != destination || generation.scope != scope {
                return Err(SecretStoreError);
            }
            delete_generation(&lease, &bucket, &key, generation)?;
        }
        Ok(())
    }

    /// 제한된 시작·유지보수 범위 안에서 전이를 완료하고 만료 항목을 삭제합니다.
    pub fn maintain(&self, now: u64) -> Result<()> {
        self.list(now).map(|_| ())
    }
}

fn bucket_name(destination: &SecretDestination, scope: &str) -> Result<String> {
    model::destination_scope_identity(destination, scope)
}
fn validate_bucket_name(name: &str) -> Result<()> {
    if name.len() != 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        Err(SecretStoreError)
    } else {
        Ok(())
    }
}
fn entry_name(g: &Generation) -> Result<String> {
    Ok(format!(
        "{}.entry",
        g.entry
            .as_deref()
            .filter(|id| model::valid_id(id))
            .ok_or(SecretStoreError)?
    ))
}
fn temporary_name(g: &Generation) -> String {
    format!("{}.pending", g.generation)
}
fn write_record<T: Serialize>(directory: &File, name: &str, key: &Key, value: &T) -> Result<()> {
    fs::create(directory, name, &crypto::sign(key, value)?)?;
    Ok(())
}
fn record<T: Serialize + DeserializeOwned>(
    directory: &File,
    name: &str,
    key: &Key,
) -> Result<Option<T>> {
    fs::read(directory, name, PUBLIC_LIMIT)?
        .map(|bytes| crypto::verify(key, &bytes))
        .transpose()
}
fn generation(directory: &File, name: &str, key: &Key) -> Result<Option<Generation>> {
    let value: Option<Generation> = record(directory, name, key)?;
    if let Some(value) = &value {
        validate_generation(value)?;
    }
    Ok(value)
}
fn validate_generation(g: &Generation) -> Result<()> {
    if g.format != GENERATION_FORMAT
        || !model::valid_id(&g.generation)
        || !model::valid_scope(&g.scope)
        || g.title.is_empty()
        || g.title.len() > 80
        || g.title.chars().any(char::is_control)
        || g.predecessor_generation
            .as_deref()
            .is_some_and(|id| !model::valid_id(id))
        || g.predecessor_entry
            .as_deref()
            .is_some_and(|id| !model::valid_id(id))
        || g.predecessor_generation.is_some() != g.predecessor_entry.is_some()
    {
        return Err(SecretStoreError);
    }
    match g.state {
        GenerationState::Entry => {
            let p = g.retention.as_ref().ok_or(SecretStoreError)?;
            p.validate()?;
            if g.entry.as_deref().is_none_or(|id| !model::valid_id(id))
                || p.destination() != &g.destination
                || p.scope() != g.scope
                || p.title() != g.title
            {
                return Err(SecretStoreError);
            }
        },
        GenerationState::Tombstone => {
            if g.entry.is_some() || g.retention.is_some() {
                return Err(SecretStoreError);
            }
        },
    }
    Ok(())
}
fn ciphertext(lease: &Lease, key: &Key, g: &Generation) -> Result<bool> {
    if g.state == GenerationState::Tombstone {
        return Ok(false);
    }
    let Some(bytes) = fs::read(&lease.entries, &entry_name(g)?, crypto::ENTRY_BYTES)? else {
        return Ok(false);
    };
    crypto::authenticate(key, g, &bytes)?;
    Ok(true)
}
fn publish_barrier(bucket: &File, key: &Key, barrier: &Barrier) -> Result<()> {
    let name = format!("{}.barrier", model::new_id()?);
    write_record(bucket, &name, key, barrier)?;
    fs::rename(bucket, &name, "transition")
}
fn delete_generation(lease: &Lease, bucket: &File, key: &Key, prior: Generation) -> Result<()> {
    let successor = Generation {
        format: GENERATION_FORMAT.to_owned(),
        state: GenerationState::Tombstone,
        generation: model::new_id()?,
        entry: None,
        predecessor_generation: Some(prior.generation.clone()),
        predecessor_entry: prior.entry.clone(),
        destination: prior.destination.clone(),
        scope: prior.scope.clone(),
        title: prior.title.clone(),
        retention: None,
    };
    write_record(bucket, &temporary_name(&successor), key, &successor)?;
    let barrier = Barrier {
        format: BARRIER_FORMAT.to_owned(),
        operation: Operation::Delete,
        destination: prior.destination.clone(),
        scope: prior.scope.clone(),
        prior: Some(prior),
        successor,
    };
    publish_barrier(bucket, key, &barrier)?;
    recover_bucket(
        lease,
        bucket,
        key,
        &bucket_name(&barrier.destination, &barrier.scope)?,
    )?;
    Ok(())
}

fn recover_bucket(
    lease: &Lease,
    bucket: &File,
    key: &Key,
    expected_bucket: &str,
) -> Result<Option<Generation>> {
    let barrier: Option<Barrier> = record(bucket, "transition", key)?;
    let current = generation(bucket, "current", key)?;
    let previous = generation(bucket, "previous", key)?;
    for g in current.iter().chain(previous.iter()) {
        if bucket_name(&g.destination, &g.scope)? != expected_bucket {
            return Err(SecretStoreError);
        }
    }
    if let Some(b) = &barrier
        && bucket_name(&b.destination, &b.scope)? != expected_bucket
    {
        return Err(SecretStoreError);
    }
    let Some(barrier) = barrier else {
        if previous.is_some() {
            return Err(SecretStoreError);
        }
        if let Some(current) = &current
            && (current.state != GenerationState::Entry || !ciphertext(lease, key, current)?)
        {
            return Err(SecretStoreError);
        }
        return Ok(current);
    };
    validate_barrier(&barrier)?;
    let successor = &barrier.successor;
    let prior = barrier.prior.as_ref();
    let temporary = generation(bucket, &temporary_name(successor), key)?;
    if temporary.as_ref().is_some_and(|g| g != successor) {
        return Err(SecretStoreError);
    }
    let successor_cipher = ciphertext(lease, key, successor)?;
    let prior_cipher = prior
        .map(|g| ciphertext(lease, key, g))
        .transpose()?
        .unwrap_or(false);
    if temporary.is_some() && successor.state == GenerationState::Entry && !successor_cipher {
        return Err(SecretStoreError);
    }
    match barrier.operation {
        Operation::Create => {
            if previous.is_some() {
                return Err(SecretStoreError);
            }
            if current.is_none() {
                abort_successor(lease, bucket, successor)?;
                return Ok(None);
            }
            if current.as_ref() != Some(successor) || !successor_cipher {
                return Err(SecretStoreError);
            }
            resolve_successor(lease, bucket, successor)?;
            fs::remove(bucket, &temporary_name(successor))?;
            fs::remove(bucket, "transition")?;
            Ok(Some(successor.clone()))
        },
        Operation::Replace => {
            if current.as_ref() == prior && previous.is_none() && prior_cipher {
                abort_successor(lease, bucket, successor)?;
                return Ok(prior.cloned());
            }
            if current.is_none() && previous.as_ref() == prior && prior_cipher {
                fs::rename(bucket, "previous", "current")?;
                abort_successor(lease, bucket, successor)?;
                return Ok(prior.cloned());
            }
            if current.as_ref() != Some(successor)
                || !successor_cipher
                || (previous.is_some() && (previous.as_ref() != prior || !prior_cipher))
            {
                return Err(SecretStoreError);
            }
            resolve_successor(lease, bucket, successor)?;
            fs::remove(bucket, "previous")?;
            if let Some(prior) = prior {
                fs::remove(&lease.entries, &entry_name(prior)?)?;
            }
            fs::remove(bucket, &temporary_name(successor))?;
            fs::remove(bucket, "transition")?;
            Ok(Some(successor.clone()))
        },
        Operation::Delete => {
            let prior = prior.ok_or(SecretStoreError)?;
            let initial = current.as_ref() == Some(prior) && previous.is_none();
            let moved = current.is_none() && previous.as_ref() == Some(prior);
            let published = current.as_ref() == Some(successor)
                && (previous.is_none() || previous.as_ref() == Some(prior));
            let cleared = current.is_none() && previous.is_none();
            if !(initial || moved || published || cleared)
                || ((initial || previous.is_some()) && !prior_cipher)
            {
                return Err(SecretStoreError);
            }
            if initial {
                fs::rename(bucket, "current", "previous")?;
            }
            if initial || moved {
                if temporary.is_none() {
                    write_record(bucket, &temporary_name(successor), key, successor)?;
                }
                fs::rename(bucket, &temporary_name(successor), "current")?;
            }
            if !cleared {
                fs::sync(bucket, "current")?;
            }
            fs::remove(bucket, "previous")?;
            fs::remove(&lease.entries, &entry_name(prior)?)?;
            fs::remove(bucket, &temporary_name(successor))?;
            fs::remove(bucket, "current")?;
            fs::remove(bucket, "transition")?;
            Ok(None)
        },
    }
}
fn validate_barrier(b: &Barrier) -> Result<()> {
    if b.format != BARRIER_FORMAT
        || b.destination != b.successor.destination
        || b.scope != b.successor.scope
    {
        return Err(SecretStoreError);
    }
    validate_generation(&b.successor)?;
    if let Some(prior) = &b.prior {
        validate_generation(prior)?;
        if prior.state != GenerationState::Entry
            || prior.destination != b.destination
            || prior.scope != b.scope
            || b.successor.predecessor_generation.as_ref() != Some(&prior.generation)
            || b.successor.predecessor_entry != prior.entry
            || b.successor.generation == prior.generation
            || (b.successor.entry.is_some() && b.successor.entry == prior.entry)
        {
            return Err(SecretStoreError);
        }
    } else if b.successor.predecessor_generation.is_some()
        || b.successor.predecessor_entry.is_some()
    {
        return Err(SecretStoreError);
    }
    match b.operation {
        Operation::Create if b.prior.is_none() && b.successor.state == GenerationState::Entry => {
            Ok(())
        },
        Operation::Replace if b.prior.is_some() && b.successor.state == GenerationState::Entry => {
            Ok(())
        },
        Operation::Delete
            if b.prior.is_some() && b.successor.state == GenerationState::Tombstone =>
        {
            Ok(())
        },
        _ => Err(SecretStoreError),
    }
}
fn resolve_successor(lease: &Lease, bucket: &File, g: &Generation) -> Result<()> {
    fs::sync(&lease.entries, &entry_name(g)?)?;
    fs::sync(bucket, "current")
}
fn abort_successor(lease: &Lease, bucket: &File, g: &Generation) -> Result<()> {
    fs::remove(bucket, &temporary_name(g))?;
    fs::remove(&lease.entries, &entry_name(g)?)?;
    fs::remove(bucket, "transition")
}
