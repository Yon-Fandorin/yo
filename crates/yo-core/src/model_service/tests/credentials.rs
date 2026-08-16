use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::super::{
    AccountId, ApiCredential, CredentialStore, LocalCredentialStore, LocalCredentialStoreError,
    ProviderId,
};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock is after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "yo-model-service-{}-{name}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_credential_file(path: &Path, contents: &str, mode: u32) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

// resolved secret은 connector가 내부적으로 읽을 수 있어도 Debug와 Display projection에는
// 원문이나 일부 prefix가 나타나지 않고 store Debug도 계정 수만 보고하는지 검증합니다.
#[test]
fn redacts_api_credentials_from_diagnostics() {
    let secret = ApiCredential::new("sk-sensitive-value").unwrap();
    let store = CredentialStore::new([(
        (
            ProviderId::new("qwencloud").unwrap(),
            AccountId::new("token-plan").unwrap(),
        ),
        secret.clone(),
    )])
    .unwrap();

    assert_eq!(secret.expose_secret(), "sk-sensitive-value");
    assert_eq!(format!("{secret:?}"), "ApiCredential([REDACTED])");
    assert_eq!(secret.to_string(), "[REDACTED]");
    assert_eq!(
        format!("{store:?}"),
        "CredentialStore { credential_count: 1 }"
    );
    assert!(!format!("{store:?}").contains("sensitive"));
    assert!(store.contains_secret_material("prefix sk-sensitive-value suffix"));
    assert!(!store.contains_secret_material("ordinary tool output"));
}

// credential 파일이 아직 없는 첫 실행은 파일이나 부모 디렉터리를 만들지 않고 빈
// immutable snapshot을 반환해야 읽기 동작이 사용자 상태를 암묵적으로 바꾸지 않습니다.
#[test]
fn missing_local_credential_file_is_an_empty_non_creating_store() {
    let directory = TestDirectory::new("missing");
    let path = directory.path().join("nested/credentials.yaml");

    let store = LocalCredentialStore::open(&path).unwrap();

    assert!(store.is_empty());
    assert!(!path.exists());
    assert!(!path.parent().unwrap().exists());
}

// 현재 사용자가 소유한 0600 regular file의 pre-version Provider/Account mapping은 지정된
// exact pair에서만 원문 API key를 resolve하는 시작 시점 snapshot으로 읽히는지 검증합니다.
#[test]
fn loads_a_user_only_provider_account_credential_snapshot() {
    let directory = TestDirectory::new("valid");
    let path = directory.path().join("credentials.yaml");
    write_credential_file(
        &path,
        "providers:\n  qwencloud:\n    token-plan:\n      api_key: sk-sensitive-value\n",
        0o600,
    );

    let store = LocalCredentialStore::open(&path).unwrap();
    let credential = store
        .resolve(
            &ProviderId::new("qwencloud").unwrap(),
            &AccountId::new("token-plan").unwrap(),
        )
        .unwrap();

    assert_eq!(credential.expose_secret(), "sk-sensitive-value");
    assert_eq!(store.len(), 1);
}

// 서로 다른 Provider는 같은 AccountId를 독립적으로 사용할 수 있어야 하며 exact pair가
// 없을 때 다른 Provider의 같은 이름 credential로 fall through하지 않는지 검증합니다.
#[test]
fn isolates_the_same_account_id_between_providers() {
    let directory = TestDirectory::new("provider-scope");
    let path = directory.path().join("credentials.yaml");
    write_credential_file(
        &path,
        "providers:\n  openrouter:\n    default:\n      api_key: sk-openrouter\n  qwencloud:\n    default:\n      api_key: sk-qwencloud\n",
        0o600,
    );

    let store = LocalCredentialStore::open(&path).unwrap();
    let account = AccountId::new("default").unwrap();

    assert_eq!(
        store
            .resolve(&ProviderId::new("openrouter").unwrap(), &account)
            .unwrap()
            .expose_secret(),
        "sk-openrouter"
    );
    assert_eq!(
        store
            .resolve(&ProviderId::new("qwencloud").unwrap(), &account)
            .unwrap()
            .expose_secret(),
        "sk-qwencloud"
    );
    assert!(
        store
            .resolve(&ProviderId::new("missing-provider").unwrap(), &account)
            .is_none()
    );
}

// group 또는 other permission이 하나라도 열린 credential 파일은 내용이 올바르더라도
// 읽기 전에 거절해야 로컬의 다른 계정에 API key가 노출되는 설정을 허용하지 않습니다.
#[test]
fn rejects_a_credential_file_with_group_or_other_permissions() {
    let directory = TestDirectory::new("permissions");
    let path = directory.path().join("credentials.yaml");
    write_credential_file(&path, "providers: {}\n", 0o640);

    let error = LocalCredentialStore::open(&path).unwrap_err();

    assert!(matches!(
        error,
        LocalCredentialStoreError::InsecurePermissions(ref rejected) if rejected == &path
    ));
}

// 최종 경로의 symlink는 target이 안전한 0600 regular file이어도 no-follow open에서
// 거절되어 경로 확인과 실제 credential read 사이의 대상을 바꿀 수 없음을 검증합니다.
#[test]
fn rejects_a_symlink_instead_of_following_it_to_credentials() {
    let directory = TestDirectory::new("symlink");
    let target = directory.path().join("actual.yaml");
    let alias = directory.path().join("credentials.yaml");
    write_credential_file(&target, "providers: {}\n", 0o600);
    symlink(&target, &alias).unwrap();

    assert!(matches!(
        LocalCredentialStore::open(&alias),
        Err(LocalCredentialStoreError::Io { .. })
    ));
}

// regular file이 아닌 directory는 nonblocking handle로 열릴 수 있는 플랫폼에서도
// 같은 handle의 metadata 검증에서 거절되어 무제한 또는 특수 입력으로 읽히지 않습니다.
#[test]
fn rejects_non_regular_credential_sources() {
    let directory = TestDirectory::new("directory");

    assert!(matches!(
        LocalCredentialStore::open(directory.path()),
        Err(LocalCredentialStoreError::UnsupportedFileType(ref rejected))
            if rejected == directory.path()
    ));
}

// 허용 한도보다 큰 regular file은 YAML parse 전에 typed size failure로 거절되어
// credential 입력이 시작 시점 메모리를 무제한 소비하지 않는지 검증합니다.
#[test]
fn rejects_an_oversized_credential_file_before_parsing() {
    let directory = TestDirectory::new("oversized");
    let path = directory.path().join("credentials.yaml");
    write_credential_file(&path, &"x".repeat(64 * 1024 + 1), 0o600);

    assert!(matches!(
        LocalCredentialStore::open(&path),
        Err(LocalCredentialStoreError::TooLarge(ref rejected)) if rejected == &path
    ));
}

// malformed 또는 중복 Provider/Account YAML에 secret-looking 원문이 들어 있어도 외부 오류에는
// 경로와 일반 사유만 남겨 credential 값이나 parser 문맥이 유출되지 않는지 검증합니다.
#[test]
fn redacts_secret_material_from_invalid_credential_diagnostics() {
    let directory = TestDirectory::new("invalid");
    let path = directory.path().join("credentials.yaml");
    write_credential_file(
        &path,
        "providers:\n  qwencloud:\n    duplicate:\n      api_key: sk-first-secret\n    duplicate:\n      api_key: sk-second-secret\n",
        0o600,
    );

    let error = LocalCredentialStore::open(&path).unwrap_err();
    let diagnostic = format!("{error:?} {error}");

    assert!(matches!(
        error,
        LocalCredentialStoreError::InvalidContents(_)
    ));
    assert!(!diagnostic.contains("sk-first-secret"));
    assert!(!diagnostic.contains("sk-second-secret"));
}
