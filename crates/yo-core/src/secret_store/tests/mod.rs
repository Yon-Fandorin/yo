use std::{
    env, fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

use super::{
    crypto::{self, Key},
    model::{Barrier, Generation, GenerationState, Operation},
    *,
};
use crate::SecretInput;

struct Fixture {
    root: PathBuf,
    state: PathBuf,
    key: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = fs::canonicalize(env::temp_dir())
            .unwrap()
            .join(format!("yo-secret-store-{}", model::new_id().unwrap()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let state = root.join("state");
        let config = root.join("config");
        for path in [&state, &config] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self {
            root,
            state,
            key: config.join("secret-recovery.key"),
        }
    }
    fn open(&self) -> SecretStore {
        SecretStore::open(self.state.clone(), self.key.clone()).unwrap()
    }
    fn destination(&self) -> SecretDestination {
        SecretDestination::from_live_authentication(
            &crate::ProviderId::new("provider").unwrap(),
            &crate::ModelId::new("model").unwrap(),
            &LiveAuthenticatedAccount::from_live_observation("authenticated-user".into()).unwrap(),
        )
        .unwrap()
    }
    fn seed(&self) {
        self.open()
            .save(
                &self.destination(),
                "test.token",
                "Token",
                RetentionPolicy::UntilDeleted,
                &SecretInput::new("prior-secret").unwrap(),
                100,
            )
            .unwrap();
    }
    fn bucket(&self) -> PathBuf {
        fs::read_dir(self.state.join("secret-store/metadata"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
    }
    fn key_bytes(&self) -> Key {
        zeroize::Zeroizing::new(fs::read(&self.key).unwrap().try_into().unwrap())
    }
    fn current(&self) -> Generation {
        crypto::verify(
            &self.key_bytes(),
            &fs::read(self.bucket().join("current")).unwrap(),
        )
        .unwrap()
    }
    fn value(&self) -> Option<String> {
        self.open()
            .recover(&self.destination(), "test.token", 100)
            .unwrap()
            .map(|v| v.expose().to_owned())
    }
    fn transition(&self, operation: Operation) -> Barrier {
        let prior = self.current();
        let successor = Generation {
            format: prior.format.clone(),
            state: if operation == Operation::Delete {
                GenerationState::Tombstone
            } else {
                GenerationState::Entry
            },
            generation: model::new_id().unwrap(),
            entry: if operation == Operation::Delete {
                None
            } else {
                Some(model::new_id().unwrap())
            },
            predecessor_generation: if operation == Operation::Create {
                None
            } else {
                Some(prior.generation.clone())
            },
            predecessor_entry: if operation == Operation::Create {
                None
            } else {
                prior.entry.clone()
            },
            destination: prior.destination.clone(),
            scope: prior.scope.clone(),
            title: prior.title.clone(),
            retention: if operation == Operation::Delete {
                None
            } else {
                prior.retention.clone()
            },
        };
        if let Some(id) = &successor.entry {
            write(
                &self
                    .state
                    .join("secret-store/entries")
                    .join(format!("{id}.entry")),
                &crypto::encrypt(
                    &self.key_bytes(),
                    &successor,
                    &SecretInput::new("successor-secret").unwrap(),
                )
                .unwrap(),
            );
        }
        write(
            &self
                .bucket()
                .join(format!("{}.pending", successor.generation)),
            &crypto::sign(&self.key_bytes(), &successor).unwrap(),
        );
        let barrier = Barrier {
            format: "yo.secret-transition/v1".to_owned(),
            operation,
            destination: prior.destination.clone(),
            scope: prior.scope.clone(),
            prior: if operation == Operation::Create {
                None
            } else {
                Some(prior)
            },
            successor,
        };
        write(
            &self.bucket().join("transition"),
            &crypto::sign(&self.key_bytes(), &barrier).unwrap(),
        );
        barrier
    }
    fn publish(&self, b: &Barrier) {
        fs::rename(
            self.bucket()
                .join(format!("{}.pending", b.successor.generation)),
            self.bucket().join("current"),
        )
        .unwrap();
    }
    fn assert_unchanged_failure(&self) {
        let before = snapshot(&self.root);
        assert!(self.open().list(100).is_err());
        assert_eq!(snapshot(&self.root), before);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
fn snapshot(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut result = Vec::new();
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(snapshot(&path));
        } else {
            result.push((path.clone(), fs::read(path).unwrap()));
        }
    }
    result.sort();
    result
}

// 저장·교체·재시작·명시적 삭제가 공개 메타데이터와 정확한 값만 유지한다.
#[test]
fn save_replace_restart_and_delete() {
    let fixture = Fixture::new();
    let store = fixture.open();
    assert!(!fixture.key.exists());
    assert!(store.list(0).unwrap().is_empty());
    let d = fixture.destination();
    store
        .save(
            &d,
            "test.token",
            "Token",
            RetentionPolicy::UntilDeleted,
            &SecretInput::new("first").unwrap(),
            0,
        )
        .unwrap();
    assert_eq!(fixture.value().as_deref(), Some("first"));
    store
        .save(
            &d,
            "test.token",
            "New title",
            RetentionPolicy::ForDays(1),
            &SecretInput::new("next\n한글").unwrap(),
            100,
        )
        .unwrap();
    assert_eq!(fixture.value().as_deref(), Some("next\n한글"));
    let public = store.list(100).unwrap();
    assert_eq!(public.len(), 1);
    assert_eq!(public[0].expires_at(), Some(86500));
    assert_eq!(
        fs::read_dir(fixture.state.join("secret-store/entries"))
            .unwrap()
            .count(),
        1
    );
    store.delete(&d, "test.token").unwrap();
    assert!(store.list(100).unwrap().is_empty());
    assert!(fixture.value().is_none());
}

// 만료 경계에서 값은 복구되지 않고 인증된 삭제가 완료된다.
#[test]
fn timed_expiry_and_policy_bounds() {
    let f = Fixture::new();
    let s = f.open();
    let d = f.destination();
    for days in [0, 366] {
        assert!(
            s.save(
                &d,
                "test.token",
                "Token",
                RetentionPolicy::ForDays(days),
                &SecretInput::new("value").unwrap(),
                0
            )
            .is_err()
        );
    }
    assert!(
        s.save(
            &d,
            "test.token",
            "Token",
            RetentionPolicy::ForDays(1),
            &SecretInput::new("value").unwrap(),
            u64::MAX
        )
        .is_err()
    );
    s.save(
        &d,
        "test.token",
        "Token",
        RetentionPolicy::ForDays(1),
        &SecretInput::new("value").unwrap(),
        0,
    )
    .unwrap();
    assert!(s.recover(&d, "test.token", 86399).unwrap().is_some());
    assert!(s.recover(&d, "test.token", 86400).unwrap().is_none());
    assert_eq!(
        fs::read_dir(f.state.join("secret-store/entries"))
            .unwrap()
            .count(),
        0
    );
}

// 암호문의 크기는 비밀 길이를 드러내지 않고 공개 파일에는 비밀이 없다.
#[test]
fn fixed_ciphertext_and_no_public_plaintext() {
    let f = Fixture::new();
    f.seed();
    let s = f.open();
    s.save(
        &f.destination(),
        "large",
        "Large",
        RetentionPolicy::UntilDeleted,
        &SecretInput::new("x".repeat(SecretInput::MAX_BYTES)).unwrap(),
        0,
    )
    .unwrap();
    for entry in fs::read_dir(f.state.join("secret-store/entries")).unwrap() {
        assert_eq!(
            entry.unwrap().metadata().unwrap().len(),
            crypto::ENTRY_BYTES as u64
        );
    }
    for (_, bytes) in snapshot(&f.state.join("secret-store/metadata")) {
        assert!(
            !bytes
                .windows(b"prior-secret".len())
                .any(|v| v == b"prior-secret")
        );
    }
    let other = SecretDestination::from_live_authentication(
        &crate::ProviderId::new("provider").unwrap(),
        &crate::ModelId::new("other").unwrap(),
        &LiveAuthenticatedAccount::from_live_observation("authenticated-user".into()).unwrap(),
    )
    .unwrap();
    assert!(s.recover(&other, "test.token", 0).unwrap().is_none());
}

// 모든 허용된 교체 중단 상태는 이전 값 또는 완료된 후속 값 하나만 복구한다.
#[test]
fn replace_recovery_states() {
    for state in [
        "unstarted",
        "moved",
        "published",
        "previous_removed",
        "ciphertext_removed",
    ] {
        let f = Fixture::new();
        f.seed();
        let b = f.transition(Operation::Replace);
        if state != "unstarted" {
            fs::rename(f.bucket().join("current"), f.bucket().join("previous")).unwrap();
        }
        if matches!(
            state,
            "published" | "previous_removed" | "ciphertext_removed"
        ) {
            f.publish(&b);
        }
        if matches!(state, "previous_removed" | "ciphertext_removed") {
            fs::remove_file(f.bucket().join("previous")).unwrap();
        }
        if state == "ciphertext_removed" {
            fs::remove_file(f.state.join("secret-store/entries").join(format!(
                "{}.entry",
                b.prior.as_ref().unwrap().entry.as_ref().unwrap()
            )))
            .unwrap();
        }
        assert_eq!(
            f.value().as_deref(),
            Some(if matches!(state, "unstarted" | "moved") {
                "prior-secret"
            } else {
                "successor-secret"
            }),
            "{state}"
        );
        assert!(!f.bucket().join("transition").exists());
        assert_eq!(
            fs::read_dir(f.state.join("secret-store/entries"))
                .unwrap()
                .count(),
            1
        );
    }
}

// 첫 생성은 current가 없으면 폐기하고 존재하면 내구성을 재확인한다.
#[test]
fn create_recovery_states() {
    for published in [false, true] {
        let f = Fixture::new();
        f.seed();
        let prior = f.current();
        let b = f.transition(Operation::Create);
        fs::remove_file(f.bucket().join("current")).unwrap();
        fs::remove_file(
            f.state
                .join("secret-store/entries")
                .join(format!("{}.entry", prior.entry.unwrap())),
        )
        .unwrap();
        if published {
            f.publish(&b);
        }
        assert_eq!(
            f.value().as_deref(),
            if published {
                Some("successor-secret")
            } else {
                None
            }
        );
        assert!(!f.bucket().join("transition").exists());
    }
}

// 삭제의 모든 중단 상태는 이전 값을 복원하지 않고 정확한 삭제를 완료한다.
#[test]
fn delete_recovery_states() {
    for state in [
        "unstarted",
        "moved",
        "published",
        "previous_removed",
        "ciphertext_removed",
        "tombstone_removed",
    ] {
        let f = Fixture::new();
        f.seed();
        let b = f.transition(Operation::Delete);
        if state != "unstarted" {
            fs::rename(f.bucket().join("current"), f.bucket().join("previous")).unwrap();
        }
        if !matches!(state, "unstarted" | "moved") {
            f.publish(&b);
        }
        if matches!(
            state,
            "previous_removed" | "ciphertext_removed" | "tombstone_removed"
        ) {
            fs::remove_file(f.bucket().join("previous")).unwrap();
        }
        if matches!(state, "ciphertext_removed" | "tombstone_removed") {
            fs::remove_file(f.state.join("secret-store/entries").join(format!(
                "{}.entry",
                b.prior.as_ref().unwrap().entry.as_ref().unwrap()
            )))
            .unwrap();
        }
        if state == "tombstone_removed" {
            fs::remove_file(f.bucket().join("current")).unwrap();
        }
        assert_eq!(f.value(), None, "{state}");
        assert!(!f.bucket().join("transition").exists());
        assert_eq!(
            fs::read_dir(f.state.join("secret-store/entries"))
                .unwrap()
                .count(),
            0
        );
    }
}

// 교체 중 두 슬롯 소실과 잘못된 현재·이전 슬롯은 어떤 파일도 바꾸지 않는다.
#[test]
fn invalid_transition_states_preserve_every_byte() {
    for state in [
        "both_absent",
        "wrong_previous",
        "missing_successor",
        "invalid_temp",
        "invalid_barrier",
        "invalid_current",
    ] {
        let f = Fixture::new();
        f.seed();
        let b = f.transition(Operation::Replace);
        match state {
            "both_absent" => fs::remove_file(f.bucket().join("current")).unwrap(),
            "wrong_previous" => {
                write(
                    &f.bucket().join("previous"),
                    &crypto::sign(&f.key_bytes(), &b.successor).unwrap(),
                );
            },
            "missing_successor" => fs::remove_file(
                f.state
                    .join("secret-store/entries")
                    .join(format!("{}.entry", b.successor.entry.as_ref().unwrap())),
            )
            .unwrap(),
            "invalid_temp" => write(
                &f.bucket()
                    .join(format!("{}.pending", b.successor.generation)),
                b"invalid",
            ),
            "invalid_barrier" => write(&f.bucket().join("transition"), b"invalid"),
            "invalid_current" => write(&f.bucket().join("current"), b"invalid"),
            _ => unreachable!(),
        }
        f.assert_unchanged_failure();
    }
}

// 장벽 없는 이전 슬롯·손상된 메타데이터·암호문·분실 키는 그대로 보존한다.
#[test]
fn damaged_stable_storage_is_unavailable_and_untouched() {
    for state in [
        "previous",
        "metadata",
        "ciphertext",
        "key_missing",
        "key_mode",
        "hard_link",
    ] {
        let f = Fixture::new();
        f.seed();
        match state {
            "previous" => {
                fs::rename(f.bucket().join("current"), f.bucket().join("previous")).unwrap()
            },
            "metadata" => write(&f.bucket().join("current"), b"invalid"),
            "ciphertext" => {
                let path = fs::read_dir(f.state.join("secret-store/entries"))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                let mut bytes = fs::read(&path).unwrap();
                bytes[40] ^= 1;
                write(&path, &bytes);
            },
            "key_missing" => fs::remove_file(&f.key).unwrap(),
            "key_mode" => fs::set_permissions(&f.key, fs::Permissions::from_mode(0o644)).unwrap(),
            "hard_link" => fs::hard_link(&f.key, f.root.join("linked-key")).unwrap(),
            _ => unreachable!(),
        }
        f.assert_unchanged_failure();
    }
}

// 잘못된 경로와 심볼릭 링크는 저장소 경계를 통과하지 않는다.
#[test]
fn nofollow_paths_and_scope_boundaries() {
    let f = Fixture::new();
    assert!(SecretStore::open(PathBuf::from("relative"), f.key.clone()).is_err());
    let link = f.root.join("linked-state");
    symlink(&f.state, &link).unwrap();
    assert!(SecretStore::open(link, f.key.clone()).is_err());
    let s = f.open();
    for scope in ["", ".token", "token-", "../escape", "Upper", "한글"] {
        assert!(
            s.save(
                &f.destination(),
                scope,
                "Title",
                RetentionPolicy::UntilDeleted,
                &SecretInput::new("x").unwrap(),
                0
            )
            .is_err()
        );
    }
    assert!(
        s.save(
            &f.destination(),
            &"a".repeat(128),
            "Title",
            RetentionPolicy::UntilDeleted,
            &SecretInput::new("x").unwrap(),
            0
        )
        .is_ok()
    );
    assert!(
        s.save(
            &f.destination(),
            &"a".repeat(129),
            "Title",
            RetentionPolicy::UntilDeleted,
            &SecretInput::new("x").unwrap(),
            0
        )
        .is_err()
    );
}

// 목적지가 다른 인증된 세대를 다른 디렉터리에 옮겨도 복구나 정리를 하지 않는다.
#[test]
fn authenticated_mapping_in_wrong_bucket_is_untouched() {
    let f = Fixture::new();
    f.seed();
    let bucket = f.bucket();
    fs::rename(&bucket, bucket.parent().unwrap().join("f".repeat(64))).unwrap();
    f.assert_unchanged_failure();
}

// 원자적 발행은 기존 목적지를 덮어쓰지 않고 두 파일을 모두 보존한다.
#[test]
fn publication_never_overwrites_occupied_destination() {
    let f = Fixture::new();
    let parent = filesystem::absolute_directory(&f.state).unwrap();
    filesystem::create(&parent, "transition", b"existing").unwrap();
    filesystem::create(&parent, "candidate", b"new").unwrap();
    assert!(filesystem::rename(&parent, "candidate", "transition").is_err());
    assert_eq!(fs::read(f.state.join("transition")).unwrap(), b"existing");
    assert_eq!(fs::read(f.state.join("candidate")).unwrap(), b"new");
}

// 다른 소유자가 임대를 유지하는 동안 목록·저장 작업은 비차단으로 실패한다.
#[test]
fn concurrent_repository_lease_rejects_operations() {
    let f = Fixture::new();
    f.seed();
    let store = f.open();
    let lock = fs::File::open(f.state.join("secret-store/lease")).unwrap();
    lock.try_lock().unwrap();
    assert!(store.list(0).is_err());
    assert!(
        store
            .save(
                &f.destination(),
                "test.token",
                "Title",
                RetentionPolicy::UntilDeleted,
                &SecretInput::new("value").unwrap(),
                0
            )
            .is_err()
    );
    assert_eq!(f.current().scope, "test.token");
}

// 키가 분실된 구형 암호문은 새 저장소의 키 재생성으로 훼손하지 않는다.
#[test]
fn missing_shared_key_never_replaces_legacy_ciphertext_authority() {
    let f = Fixture::new();
    let legacy = f.state.join("secret-recovery");
    fs::create_dir(&legacy).unwrap();
    fs::set_permissions(&legacy, fs::Permissions::from_mode(0o700)).unwrap();
    write(&legacy.join("legacy.entry"), b"surviving ciphertext");
    assert!(
        f.open()
            .save(
                &f.destination(),
                "test.token",
                "Title",
                RetentionPolicy::UntilDeleted,
                &SecretInput::new("value").unwrap(),
                0
            )
            .is_err()
    );
    assert!(!f.key.exists());
    assert_eq!(
        fs::read(legacy.join("legacy.entry")).unwrap(),
        b"surviving ciphertext"
    );
}

// 제안 없는 시작·목록·조회·삭제는 상태 디렉터리에 아무것도 기록하지 않는다.
#[test]
fn use_once_store_access_makes_no_durable_writes() {
    let f = Fixture::new();
    let before = snapshot(&f.root);
    let store = f.open();
    assert!(store.list(0).unwrap().is_empty());
    assert!(
        store
            .recover(&f.destination(), "test.token", 0)
            .unwrap()
            .is_none()
    );
    store.delete(&f.destination(), "test.token").unwrap();
    store.maintain(0).unwrap();
    assert_eq!(snapshot(&f.root), before);
    assert!(!f.state.join("secret-store").exists());
}

// 인증된 null 필드도 봉투에서 생략하면 동일한 서명으로 허용하지 않는다.
#[test]
fn authenticated_envelope_requires_explicit_absent_values() {
    let f = Fixture::new();
    f.seed();
    let path = f.bucket().join("current");
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["envelope"]
        .as_object_mut()
        .unwrap()
        .remove("predecessor_generation");
    write(&path, &serde_json::to_vec(&record).unwrap());
    f.assert_unchanged_failure();
}

// 선택 뒤 제출이 늦어져도 공개된 만료 시각을 그대로 기록한다.
#[test]
fn selected_expiry_survives_delayed_save_and_rejects_invalid_deadlines() {
    let f = Fixture::new();
    let store = f.open();
    let destination = f.destination();
    let selected_expiry = 100 + 86400;
    let metadata = store
        .save(
            &destination,
            "test.token",
            "Token",
            RetentionPolicy::ForDaysAt {
                days: 1,
                expires_at: selected_expiry,
            },
            &SecretInput::new("value").unwrap(),
            160,
        )
        .unwrap();
    assert_eq!(metadata.expires_at(), Some(selected_expiry));
    assert_eq!(
        store.list(160).unwrap()[0].expires_at(),
        Some(selected_expiry)
    );
    let before = snapshot(&f.root);
    for (days, expires_at, now) in [
        (1, selected_expiry, selected_expiry),
        (1, selected_expiry, selected_expiry + 1),
        (1, selected_expiry, 99),
        (0, selected_expiry, 160),
        (366, selected_expiry, 160),
        (1, u64::MAX, u64::MAX - 1),
    ] {
        assert!(
            store
                .save(
                    &destination,
                    "test.token",
                    "Token",
                    RetentionPolicy::ForDaysAt { days, expires_at },
                    &SecretInput::new("replacement").unwrap(),
                    now,
                )
                .is_err()
        );
        assert_eq!(snapshot(&f.root), before);
    }
    assert!(
        store
            .recover(&destination, "test.token", selected_expiry - 1)
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .recover(&destination, "test.token", selected_expiry)
            .unwrap()
            .is_none()
    );
}
