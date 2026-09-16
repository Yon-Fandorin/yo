use std::{
    env, fs,
    os::unix::{fs as unix_fs, fs::PermissionsExt},
    process, slice,
    time::{SystemTime, UNIX_EPOCH},
};

use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountCredits,
    AccountId, ProviderId,
};

use super::{
    DIRECTORY_MODE, FILE_MODE, LEGACY_SCHEMA, SCHEMA, StorageError,
    codec::{WireCacheEntry, WireCacheFile},
    load, upsert,
};
use crate::command::account::domain::AccountCapacityReport;

// cache는 snapshot, account label, 관측 시각과 count 기반 window를 저장 후 다시 복원합니다.
#[test]
fn cache_round_trips_snapshot_and_observation_time() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "yo-account-capacity-cache-{}-{nonce}",
        process::id()
    ));
    let path = root.join("account-capacity.yaml");
    let snapshot = AccountCapacitySnapshot::new(
        ProviderId::new("kimi").unwrap(),
        AccountId::new("default").unwrap(),
        vec![AccountCapacityBucket::new(
            Some("weekly".to_owned()),
            None,
            Some("Kimi Code".to_owned()),
            Some(AccountCapacityWindow::from_usage_ratio(1, 3, Some(10_080), None).unwrap()),
            None,
            Some(AccountCredits::new(Some("4".to_owned()), true, false)),
            None,
        )],
    );
    let report = AccountCapacityReport::plain(snapshot)
        .with_account_label("kimi-default")
        .with_observed_at("2026-09-03T01:02:03Z".to_owned());

    upsert(&path, slice::from_ref(&report)).unwrap();
    let loaded = load(&path).unwrap();

    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].snapshot(), report.snapshot());
    assert_eq!(loaded[0].account_label(), "kimi-default");
    assert_eq!(loaded[0].observed_at(), Some("2026-09-03T01:02:03Z"));
    assert_eq!(
        loaded[0].snapshot().buckets()[0]
            .primary()
            .unwrap()
            .reported_usage(),
        Some((1, 3))
    );
    assert!(!fs::metadata(&path).unwrap().permissions().readonly());
    let _ = fs::remove_dir_all(root);
}

// cache label은 AccountId와 같은 256-byte 경계를 round-trip하고 그 이상은 저장하지 않습니다.
#[test]
fn enforces_the_account_label_boundary_before_cache_publication() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "yo-account-capacity-label-boundary-{}-{nonce}",
        process::id()
    ));
    let path = root.join("account-capacity.yaml");
    let boundary_path = root.join("account-capacity-boundary.yaml");
    let snapshot = AccountCapacitySnapshot::new(
        ProviderId::new("codex").unwrap(),
        AccountId::new("stable-account").unwrap(),
        Vec::new(),
    );
    let report = AccountCapacityReport::plain(snapshot)
        .with_account_label("a".repeat(256))
        .with_observed_at("2026-09-03T01:02:03Z".to_owned());

    upsert(&boundary_path, slice::from_ref(&report)).unwrap();
    let loaded = load(&boundary_path).unwrap();
    assert_eq!(loaded[0].account_label().len(), 256);

    let too_long = report.with_account_label("a".repeat(257));
    assert!(matches!(
        upsert(&path, slice::from_ref(&too_long)),
        Err(StorageError::InvalidContents(_))
    ));
    assert!(!path.exists());
    let _ = fs::remove_dir_all(root);
}

// 동일한 Provider·Account 좌표가 중복되면 마지막 값으로 덮지 않고 cache 전체를 거부합니다.
#[test]
fn rejects_duplicate_cache_coordinates_instead_of_last_write_wins() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "yo-account-capacity-duplicate-{}-{nonce}",
        process::id()
    ));
    let path = root.join("account-capacity.yaml");
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(DIRECTORY_MODE)).unwrap();

    let snapshot = AccountCapacitySnapshot::new(
        ProviderId::new("kimi").unwrap(),
        AccountId::new("default").unwrap(),
        Vec::new(),
    );
    let report =
        AccountCapacityReport::plain(snapshot).with_observed_at("2026-09-03T01:02:03Z".to_owned());
    let entry = WireCacheEntry::from_report(&report).unwrap();
    let encoded = yo_yaml::to_string(&WireCacheFile {
        schema: SCHEMA.to_owned(),
        entries: vec![entry.clone(), entry],
    })
    .unwrap();
    fs::write(&path, encoded).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(FILE_MODE)).unwrap();

    assert!(matches!(load(&path), Err(StorageError::InvalidContents(_))));
    let _ = fs::remove_dir_all(root);
}

// 이전 cache schema에서 account label이 없어도 stable AccountId를 표시 label로 사용해 읽습니다.
#[test]
fn reads_the_legacy_cache_shape_without_an_account_label() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "yo-account-capacity-legacy-{}-{nonce}",
        process::id()
    ));
    let path = root.join("account-capacity.yaml");
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(DIRECTORY_MODE)).unwrap();

    let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
        ProviderId::new("codex").unwrap(),
        AccountId::new("0123456789abcdef").unwrap(),
        Vec::new(),
    ))
    .with_observed_at("2026-09-03T01:02:03Z".to_owned());
    let mut entry = WireCacheEntry::from_report(&report).unwrap();
    entry.account_label = None;
    entry.provider_data = None;
    let encoded = yo_yaml::to_string(&WireCacheFile {
        schema: LEGACY_SCHEMA.to_owned(),
        entries: vec![entry],
    })
    .unwrap();
    fs::write(&path, encoded).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(FILE_MODE)).unwrap();

    let loaded = load(&path).unwrap();

    assert_eq!(loaded[0].account_label(), "0123456789abcdef");
    let _ = fs::remove_dir_all(root);
}

// host 계정이 바뀌면 이전 host cache를 남기지 않고 새 identity 하나로 교체합니다.
#[test]
fn replaces_old_host_cache_identity_when_a_new_host_is_observed() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "yo-account-capacity-host-switch-{}-{nonce}",
        process::id()
    ));
    let path = root.join("account-capacity.yaml");
    let old = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
        ProviderId::new("grok").unwrap(),
        AccountId::new("old-host-account").unwrap(),
        Vec::new(),
    ))
    .with_observed_at("2026-09-03T01:02:03Z".to_owned());
    let new = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
        ProviderId::new("grok").unwrap(),
        AccountId::new("new-host-account").unwrap(),
        Vec::new(),
    ))
    .with_observed_at("2026-09-03T01:03:03Z".to_owned());

    upsert(&path, slice::from_ref(&old)).unwrap();
    upsert(&path, slice::from_ref(&new)).unwrap();

    let loaded = load(&path).unwrap();

    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].snapshot().account().as_str(), "new-host-account");
    let _ = fs::remove_dir_all(root);
}

// cache 디렉터리에 group 또는 other 쓰기 권한이 있으면 저장하지 않습니다.
#[test]
fn refuses_a_group_or_other_writable_state_directory() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "yo-account-capacity-insecure-{}-{nonce}",
        process::id()
    ));
    let path = root.join("account-capacity.yaml");
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
    let snapshot = AccountCapacitySnapshot::new(
        ProviderId::new("kimi").unwrap(),
        AccountId::new("default").unwrap(),
        Vec::new(),
    );
    let report =
        AccountCapacityReport::plain(snapshot).with_observed_at("2026-09-03T01:02:03Z".to_owned());

    assert!(matches!(
        upsert(&path, slice::from_ref(&report)),
        Err(StorageError::InsecurePermissions(_))
    ));
    let _ = fs::set_permissions(&root, fs::Permissions::from_mode(DIRECTORY_MODE));
    let _ = fs::remove_dir_all(root);
}

// cache 경로가 symlink이면 대상 파일을 따라가지 않고 안전하게 거부합니다.
#[test]
fn refuses_a_symlink_cache_without_following_its_target() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "yo-account-capacity-symlink-{}-{nonce}",
        process::id()
    ));
    let path = root.join("account-capacity.yaml");
    let target = root.join("outside.yaml");
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(DIRECTORY_MODE)).unwrap();
    fs::write(&target, b"outside").unwrap();
    unix_fs::symlink(&target, &path).unwrap();
    let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
        ProviderId::new("kimi").unwrap(),
        AccountId::new("default").unwrap(),
        Vec::new(),
    ))
    .with_observed_at("2026-09-03T01:02:03Z".to_owned());

    assert!(matches!(
        upsert(&path, slice::from_ref(&report)),
        Err(StorageError::UnsupportedFileType(found)) if found == path
    ));
    assert_eq!(fs::read(&target).unwrap(), b"outside");
    let _ = fs::remove_dir_all(root);
}

// cache 경로가 regular file이 아닌 directory이면 읽기 대상으로 사용하지 않습니다.
#[test]
fn rejects_a_directory_cache_as_non_regular() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "yo-account-capacity-directory-{}-{nonce}",
        process::id()
    ));
    let path = root.join("account-capacity.yaml");
    fs::create_dir_all(&path).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(DIRECTORY_MODE)).unwrap();

    assert!(matches!(
        load(&path),
        Err(StorageError::UnsupportedFileType(found)) if found == path
    ));
    let _ = fs::remove_dir_all(root);
}

// 기존 cache 파일에 group 또는 other 권한이 있으면 secret-adjacent state로 읽지 않습니다.
#[test]
fn rejects_an_insecure_existing_cache_file() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "yo-account-capacity-file-mode-{}-{nonce}",
        process::id()
    ));
    let path = root.join("account-capacity.yaml");
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(DIRECTORY_MODE)).unwrap();
    let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
        ProviderId::new("kimi").unwrap(),
        AccountId::new("default").unwrap(),
        Vec::new(),
    ))
    .with_observed_at("2026-09-03T01:02:03Z".to_owned());
    let entry = WireCacheEntry::from_report(&report).unwrap();
    let encoded = yo_yaml::to_string(&WireCacheFile {
        schema: SCHEMA.to_owned(),
        entries: vec![entry],
    })
    .unwrap();
    fs::write(&path, encoded).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

    assert!(matches!(
        load(&path),
        Err(StorageError::InsecurePermissions(found)) if found == path
    ));
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(FILE_MODE));
    let _ = fs::remove_dir_all(root);
}
