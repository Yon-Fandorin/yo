use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    super::{AccountId, ApiCredential, ProviderId},
    *,
};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock is after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "yo-credential-repository-{}-{name}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn repository(name: &str) -> (TestDirectory, LocalCredentialRepository) {
    let directory = TestDirectory::new(name);
    let repository = LocalCredentialRepository::new(directory.0.join("nested/credentials.yaml"));
    (directory, repository)
}

fn pair(provider: &str, account: &str) -> (ProviderId, AccountId) {
    (
        ProviderId::new(provider).unwrap(),
        AccountId::new(account).unwrap(),
    )
}

fn secret(value: &str) -> ApiCredential {
    ApiCredential::new(value).unwrap()
}

// 파일이 없는 capture는 부모를 만들지 않은 absent snapshot이어야 하며, set 준비 뒤에도
// credential 본문은 생기지 않아 중단된 준비가 secret 저장으로 오인되지 않는지 검증합니다.
#[test]
fn missing_capture_and_abandoned_prepare_do_not_publish_credentials() {
    let (_directory, repository) = repository("missing");
    let (provider, account) = pair("openrouter", "default");

    let captured = repository.capture().unwrap();
    assert!(captured.revision().is_absent());
    assert!(captured.is_empty());
    assert!(!repository.path().parent().unwrap().exists());

    let mutation = repository.prepare_set(&provider, &account).unwrap();
    assert_eq!(mutation.action(), CredentialMutationAction::Add);
    assert!(!mutation.planned_revision().is_absent());
    assert!(!repository.path().exists());
}

// 첫 add는 candidate를 mutation이나 진단에 넣지 않은 채 mode 0600 snapshot 하나를
// 게시하고, 같은 mutation과 candidate 재시도는 exact planned revision으로 인식합니다.
#[test]
fn first_add_is_secure_redacted_and_idempotent() {
    let (_directory, repository) = repository("first-add");
    let (provider, account) = pair("openrouter", "default");
    let candidate = secret("sk-first-sensitive");
    let mutation = repository.prepare_set(&provider, &account).unwrap();

    assert!(!format!("{mutation:?}").contains(candidate.expose_secret()));
    assert_eq!(
        repository.commit(&mutation, Some(&candidate)).unwrap(),
        CredentialCommit::Committed
    );
    assert_eq!(
        repository.commit(&mutation, Some(&candidate)).unwrap(),
        CredentialCommit::AlreadyCommitted
    );

    let metadata = fs::metadata(repository.path()).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert!(
        !fs::read_to_string(repository.path())
            .unwrap()
            .contains("version:")
    );
    let captured = repository.capture().unwrap();
    assert_eq!(captured.revision(), mutation.planned_revision());
    assert_eq!(
        captured
            .resolve(&provider, &account)
            .unwrap()
            .expose_secret(),
        candidate.expose_secret()
    );
    assert_eq!(
        LocalCredentialStore::open(repository.path())
            .unwrap()
            .resolve(&provider, &account)
            .unwrap()
            .expose_secret(),
        candidate.expose_secret()
    );
    assert!(!format!("{:?}", captured.revision()).contains("crev-"));
}

// 같은 expected revision에서 준비된 두 add 중 winner가 게시된 뒤 loser는 conflict로
// 실패하고 winner의 API key와 planned revision을 덮어쓰지 않는지 검증합니다.
#[test]
fn stale_add_preserves_the_concurrent_winner() {
    let (_directory, repository) = repository("concurrent-winner");
    let (provider, winner_account) = pair("qwencloud", "winner");
    let loser_account = AccountId::new("loser").unwrap();
    let winner = repository.prepare_set(&provider, &winner_account).unwrap();
    let loser = repository.prepare_set(&provider, &loser_account).unwrap();

    repository
        .commit(&winner, Some(&secret("sk-winner")))
        .unwrap();
    let error = repository
        .commit(&loser, Some(&secret("sk-loser")))
        .unwrap_err();

    assert!(matches!(error, LocalCredentialStoreError::Conflict(_)));
    let current = repository.capture().unwrap();
    assert_eq!(current.revision(), winner.planned_revision());
    assert_eq!(
        current
            .resolve(&provider, &winner_account)
            .unwrap()
            .expose_secret(),
        "sk-winner"
    );
    assert!(current.resolve(&provider, &loser_account).is_none());
}

// revision 없는 현재 pre-version 파일의 exact pair를 replace하면 기존 account는 보존되고
// 새 private revision이 추가되어 다음 재시작부터 stored CAS snapshot이 되는지 검증합니다.
#[test]
fn replacing_one_pair_materializes_a_revision_for_the_current_snapshot() {
    let (directory, repository) = repository("revisionless-replace");
    fs::create_dir_all(repository.path().parent().unwrap()).unwrap();
    fs::write(
        repository.path(),
        concat!(
            "",
            "providers:\n",
            "  openrouter:\n",
            "    default:\n",
            "      api_key: sk-old\n",
            "  qwencloud:\n",
            "    retained:\n",
            "      api_key: sk-retained\n",
        ),
    )
    .unwrap();
    fs::set_permissions(repository.path(), fs::Permissions::from_mode(0o600)).unwrap();
    let (provider, account) = pair("openrouter", "default");
    let retained_provider = ProviderId::new("qwencloud").unwrap();
    let retained_account = AccountId::new("retained").unwrap();

    let mutation = repository.prepare_set(&provider, &account).unwrap();
    assert_eq!(mutation.action(), CredentialMutationAction::Replace);
    repository
        .commit(&mutation, Some(&secret("sk-replacement")))
        .unwrap();

    let contents = fs::read_to_string(repository.path()).unwrap();
    assert!(contents.contains("revision: crev-"));
    let current = repository.capture().unwrap();
    assert_eq!(
        current
            .resolve(&provider, &account)
            .unwrap()
            .expose_secret(),
        "sk-replacement"
    );
    assert_eq!(
        current
            .resolve(&retained_provider, &retained_account)
            .unwrap()
            .expose_secret(),
        "sk-retained"
    );
    drop(directory);
}

// 같은 account record의 account_session은 API key와 독립적으로 추가·재시도·보존되며,
// local tool redaction store에도 포함되고 마지막 pair 제거 때 함께 사라집니다.
#[test]
fn account_session_round_trips_without_replacing_the_model_credential() {
    let (_directory, repository) = repository("account-session");
    let (provider, account) = pair("qwencloud", "default");
    let add = repository.prepare_set(&provider, &account).unwrap();
    repository
        .commit(&add, Some(&secret("sk-sp-model")))
        .unwrap();

    let session = secret("cna=x; login_qwencloud_ticket=session-secret");
    let set_session = repository
        .prepare_set_account_session(&provider, &account)
        .unwrap();
    assert!(!format!("{set_session:?}").contains(session.expose_secret()));
    assert_eq!(
        repository
            .commit_account_session(&set_session, &session)
            .unwrap(),
        CredentialCommit::Committed
    );
    assert_eq!(
        repository
            .commit_account_session(&set_session, &session)
            .unwrap(),
        CredentialCommit::AlreadyCommitted
    );

    let replace_api = repository.prepare_set(&provider, &account).unwrap();
    repository
        .commit(&replace_api, Some(&secret("sk-sp-rotated")))
        .unwrap();
    let captured = repository.capture().unwrap();
    assert_eq!(
        captured
            .resolve(&provider, &account)
            .unwrap()
            .expose_secret(),
        "sk-sp-rotated"
    );
    assert_eq!(
        captured
            .resolve_account_session(&provider, &account)
            .unwrap()
            .expose_secret(),
        session.expose_secret()
    );
    assert!(
        captured
            .credentials()
            .contains_secret_material(session.expose_secret())
    );
    assert!(
        fs::read_to_string(repository.path())
            .unwrap()
            .contains("account_session:")
    );

    let remove = repository
        .prepare_remove(&provider, &account)
        .unwrap()
        .unwrap();
    repository.commit(&remove, None).unwrap();
    let captured = repository.capture().unwrap();
    assert!(captured.resolve(&provider, &account).is_none());
    assert!(
        captured
            .resolve_account_session(&provider, &account)
            .is_none()
    );
}

// account_session은 기존 Provider-and-Account API credential의 보조 field이므로 단독으로
// 새 account record를 만들지 못하고 저장소 byte도 게시하지 않습니다.
#[test]
fn account_session_requires_an_existing_model_credential() {
    let (_directory, repository) = repository("account-session-without-model");
    let (provider, account) = pair("qwencloud", "default");

    let error = repository
        .prepare_set_account_session(&provider, &account)
        .unwrap_err();

    assert!(matches!(error, LocalCredentialStoreError::InvalidMutation));
    assert!(!repository.path().exists());
}

// 별도 format classifier가 없으므로 top-level version은 일반 invalid-content이며
// secret을 진단에 노출하거나 원문을 자동 변환하지 않습니다.
#[test]
fn unknown_top_level_version_is_invalid_credential_state() {
    let (_directory, repository) = repository("unknown-version");
    fs::create_dir_all(repository.path().parent().unwrap()).unwrap();
    let stale = concat!(
        "version: 1\n",
        "providers:\n",
        "  openrouter:\n",
        "    default:\n",
        "      api_key: sk-stale-secret\n",
    );
    fs::write(repository.path(), stale).unwrap();
    fs::set_permissions(repository.path(), fs::Permissions::from_mode(0o600)).unwrap();

    let error = repository.capture().unwrap_err();
    let diagnostic = format!("{error:?} {error}");

    assert!(matches!(
        error,
        LocalCredentialStoreError::InvalidContents(_)
    ));
    assert!(!diagnostic.contains("sk-stale-secret"));
    assert_eq!(fs::read_to_string(repository.path()).unwrap(), stale);
}

// account 내부의 version 오타도 secret을 진단에 노출하지 않고 일반 invalid-content로 남깁니다.
#[test]
fn nested_version_field_is_invalid_credential_state() {
    let (_directory, repository) = repository("nested-version");
    fs::create_dir_all(repository.path().parent().unwrap()).unwrap();
    let malformed = concat!(
        "providers:\n",
        "  openrouter:\n",
        "    default:\n",
        "      api_key: sk-nested-secret\n",
        "      version: 1\n",
    );
    fs::write(repository.path(), malformed).unwrap();
    fs::set_permissions(repository.path(), fs::Permissions::from_mode(0o600)).unwrap();

    let error = repository.capture().unwrap_err();
    let diagnostic = format!("{error:?} {error}");
    assert!(matches!(
        error,
        LocalCredentialStoreError::InvalidContents(_)
    ));
    assert!(!diagnostic.contains("sk-nested-secret"), "{diagnostic}");
    assert_eq!(fs::read_to_string(repository.path()).unwrap(), malformed);
}

// 마지막 pair removal은 파일을 다시 absent로 지우지 않고 non-absent planned revision의
// revisioned empty snapshot을 게시하며 동일 removal 재시도를 완료로 인식하는지 검증합니다.
#[test]
fn removing_the_final_pair_keeps_a_revisioned_empty_snapshot() {
    let (_directory, repository) = repository("final-remove");
    let (provider, account) = pair("openrouter", "default");
    let add = repository.prepare_set(&provider, &account).unwrap();
    repository
        .commit(&add, Some(&secret("sk-removable")))
        .unwrap();
    let remove = repository
        .prepare_remove(&provider, &account)
        .unwrap()
        .unwrap();

    assert_eq!(remove.action(), CredentialMutationAction::Remove);
    assert_eq!(
        repository.commit(&remove, None).unwrap(),
        CredentialCommit::Committed
    );
    assert_eq!(
        repository.commit(&remove, None).unwrap(),
        CredentialCommit::AlreadyCommitted
    );
    let current = repository.capture().unwrap();
    assert!(!current.revision().is_absent());
    assert!(current.is_empty());
    assert!(repository.path().exists());
    assert!(
        repository
            .prepare_remove(&provider, &account)
            .unwrap()
            .is_none()
    );
}

// add·replace에는 candidate가 필수이고 remove에는 금지되어야 하며, API 오용이 발생해도
// candidate 원문이나 private revision을 오류와 Debug projection에 노출하지 않습니다.
#[test]
fn rejects_candidate_shape_mismatches_without_secret_or_revision_disclosure() {
    let (_directory, repository) = repository("candidate-shape");
    let (provider, account) = pair("openrouter", "default");
    let add = repository.prepare_set(&provider, &account).unwrap();

    let missing = repository.commit(&add, None).unwrap_err();
    let diagnostic = format!("{missing:?} {missing} {add:?}");
    assert!(matches!(
        missing,
        LocalCredentialStoreError::InvalidMutation
    ));
    assert!(!diagnostic.contains("crev-"));

    repository
        .commit(&add, Some(&secret("sk-shape-sensitive")))
        .unwrap();
    let remove = repository
        .prepare_remove(&provider, &account)
        .unwrap()
        .unwrap();
    let unexpected = repository
        .commit(&remove, Some(&secret("sk-must-not-appear")))
        .unwrap_err();
    let diagnostic = format!("{unexpected:?} {unexpected} {remove:?}");
    assert!(!diagnostic.contains("sk-must-not-appear"));
    assert!(!diagnostic.contains("crev-"));
}

// exact handle의 owner가 현재 effective user와 다르면 다른 metadata가 모두 안전해도
// WrongOwner로 거절해야 다른 local 계정의 credential을 읽지 않는지 검증합니다.
#[test]
fn rejects_metadata_owned_by_another_effective_user() {
    let mut metadata = storage::secure_snapshot();
    storage::change_owner(&mut metadata);
    let path = Path::new("credentials.yaml");

    assert!(matches!(
        storage::validate_snapshot(path, &metadata),
        Err(LocalCredentialStoreError::WrongOwner(rejected)) if rejected == path
    ));
}

// 같은 handle에서 read 전후 snapshot의 identity·권한·크기·시각 중 하나라도 달라지면
// captured byte를 사용하지 않고 Changed로 거절하는지 길이 변경 사례로 검증합니다.
#[test]
fn rejects_metadata_that_changes_while_the_handle_is_read() {
    let before = storage::secure_snapshot();
    let mut after = before.clone();
    storage::change_length(&mut after);
    let path = Path::new("credentials.yaml");

    assert!(matches!(
        storage::validate_stable_snapshots(path, &before, &after),
        Err(LocalCredentialStoreError::Changed(rejected)) if rejected == path
    ));
}

// planned revision이 이미 보이더라도 전달된 candidate가 실제 pair 값과 다르면 exact retry가
// 아니므로 conflict로 거절하고 처음 commit한 secret을 그대로 유지하는지 검증합니다.
#[test]
fn planned_revision_requires_the_exact_candidate_for_idempotent_set() {
    let (_directory, repository) = repository("planned-candidate");
    let (provider, account) = pair("openrouter", "default");
    let mutation = repository.prepare_set(&provider, &account).unwrap();
    repository
        .commit(&mutation, Some(&secret("sk-exact")))
        .unwrap();

    let error = repository
        .commit(&mutation, Some(&secret("sk-different")))
        .unwrap_err();

    assert!(matches!(error, LocalCredentialStoreError::Conflict(_)));
    assert_eq!(
        repository
            .capture()
            .unwrap()
            .resolve(&provider, &account)
            .unwrap()
            .expose_secret(),
        "sk-exact"
    );
}

// 기존 snapshot은 한도 안이지만 새 pair까지 encode하면 한도를 넘는 경우 commit은
// PreparedTooLarge로 실패하고 기존 파일 byte와 모든 credential을 전혀 바꾸지 않습니다.
#[test]
fn oversized_prepared_snapshot_preserves_the_exact_old_file() {
    let (_directory, repository) = repository("prepared-too-large");
    fs::create_dir_all(repository.path().parent().unwrap()).unwrap();
    let large_secret = "x".repeat(15_900);
    let mut contents = String::from("providers:\n  openrouter:\n");
    for index in 0..4 {
        contents.push_str(&format!(
            "    account-{index}:\n      api_key: {large_secret}\n"
        ));
    }
    assert!(contents.len() < storage::MAX_CREDENTIAL_FILE_BYTES as usize);
    fs::write(repository.path(), &contents).unwrap();
    fs::set_permissions(repository.path(), fs::Permissions::from_mode(0o600)).unwrap();
    let before = fs::read(repository.path()).unwrap();
    let (provider, account) = pair("openrouter", "overflow");
    let mutation = repository.prepare_set(&provider, &account).unwrap();

    let error = repository
        .commit(&mutation, Some(&secret(&large_secret)))
        .unwrap_err();

    assert!(matches!(error, LocalCredentialStoreError::PreparedTooLarge));
    assert_eq!(fs::read(repository.path()).unwrap(), before);
}
