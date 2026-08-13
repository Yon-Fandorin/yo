use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;
use crate::{AccountId, ModelId, ModelSelection, ProviderId};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "yo-connections-{}-{name}-{nonce}",
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

fn repository(name: &str) -> (TestDirectory, LocalConnectionRepository) {
    let directory = TestDirectory::new(name);
    let repository = LocalConnectionRepository::new(directory.0.join("nested/connections.yaml"));
    (directory, repository)
}

fn model_target(model: &str) -> StartupTarget {
    StartupTarget::Model(ModelSelection::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        ModelId::new(model).unwrap(),
    ))
}

fn managed_account() -> ManagedConnectionAccount {
    ManagedConnectionAccount::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        Some("QwenCloud".to_owned()),
        Some("Default".to_owned()),
    )
    .unwrap()
}

fn managed_binding(model: &str, effort: &str) -> ManagedConnectionBinding {
    let durable = format!(
        r#"{{"provider":"qwencloud","account":"default","model":"{model}","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{"effort":"{effort}"}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}}"#
    );
    ManagedConnectionBinding::new(
        crate::CompleteModelBinding::from_durable_json(&durable).unwrap(),
        Some(format!("Model {model}")),
    )
    .unwrap()
}

// 파일이 없는 첫 capture는 부모 경로도 만들지 않은 채 absent revision과 unset preference를
// 반환해야 읽기와 준비만 수행한 중단이 사용자 상태를 바꾸지 않음을 검증합니다.
#[test]
fn missing_repository_is_non_creating_canonical_empty_state() {
    let (_directory, repository) = repository("missing");

    let snapshot = repository.capture().unwrap();

    assert_eq!(snapshot.revision(), &ConnectionRevision::Absent);
    assert!(snapshot.preference().is_none());
    assert!(!repository.path().parent().unwrap().exists());
}

// absent 상태에서 준비한 HostTarget CAS는 정확히 한 번 0600 파일로 게시되고 재실행한
// 동일 mutation은 caller가 첫 성공을 못 본 경우에도 AlreadyCommitted로 판정합니다.
#[test]
fn first_publication_is_secure_and_exact_retry_is_idempotent() {
    let (_directory, repository) = repository("first-publication");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_preference(Some(StartupTarget::HostCodex))
        .unwrap()
        .unwrap();

    assert_eq!(
        repository.commit(&mutation).unwrap(),
        ConnectionCommit::Committed
    );
    assert_eq!(
        repository.commit(&mutation).unwrap(),
        ConnectionCommit::AlreadyCommitted
    );

    let metadata = fs::metadata(repository.path()).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    let current = repository.capture().unwrap();
    assert_eq!(current.revision(), mutation.planned_revision());
    assert_eq!(current.preference(), Some(&StartupTarget::HostCodex));
}

// 같은 expected revision에서 준비한 두 기본값 중 먼저 commit한 승자 뒤에는 loser가
// revision conflict로 실패하고 승자의 preference bytes가 덮어써지지 않음을 검증합니다.
#[test]
fn stale_cas_preserves_the_concurrent_winner() {
    let (_directory, repository) = repository("winner");
    let captured = repository.capture().unwrap();
    let winner = captured
        .prepare_preference(Some(model_target("winner")))
        .unwrap()
        .unwrap();
    let loser = captured
        .prepare_preference(Some(model_target("loser")))
        .unwrap()
        .unwrap();

    repository.commit(&winner).unwrap();
    let error = repository.commit(&loser).unwrap_err();

    assert!(matches!(error, ConnectionRepositoryError::Conflict { .. }));
    assert_eq!(
        repository.capture().unwrap().preference(),
        Some(&model_target("winner"))
    );
}

// 기존 preference를 clear하도록 준비만 하고 CAS 전에 호출이 사라지면 expected 파일은
// 그대로여야 하며, 새 invocation이 같은 clear를 새 revision으로 정상 완료할 수 있습니다.
#[test]
fn interruption_before_cas_leaves_old_state_for_a_fresh_retry() {
    let (_directory, repository) = repository("pre-cas");
    let set = repository
        .capture()
        .unwrap()
        .prepare_preference(Some(StartupTarget::HostCodex))
        .unwrap()
        .unwrap();
    repository.commit(&set).unwrap();
    let before = repository.capture().unwrap();
    let abandoned = before.prepare_preference(None).unwrap().unwrap();
    drop(abandoned);

    assert_eq!(repository.capture().unwrap().revision(), before.revision());
    let retry = repository
        .capture()
        .unwrap()
        .prepare_preference(None)
        .unwrap()
        .unwrap();
    repository.commit(&retry).unwrap();
    assert!(repository.capture().unwrap().preference().is_none());
}

// operation guard가 살아 있는 동안 두 번째 process-equivalent handle은 즉시 Busy를 받고,
// guard 해제 뒤에는 같은 persistent lock file에서 다시 획득되어 장기 hang 없이 직렬화됩니다.
#[test]
fn operation_lock_is_nonblocking_and_reacquirable() {
    let (_directory, repository) = repository("operation-lock");
    let guard = repository.acquire_operation().unwrap();

    assert!(matches!(
        repository.acquire_operation(),
        Err(ConnectionRepositoryError::OperationBusy(_))
    ));
    drop(guard);
    repository.acquire_operation().unwrap();
}

// operation lock 아래에서 pending journal이 보이면 preference-only command가 새 CAS를
// 계획하지 않고 fail-closed해야 아직 구현되지 않은 recoverable operation을 덮지 않습니다.
#[test]
fn pending_operation_blocks_a_new_preference_mutation() {
    let (_directory, repository) = repository("pending-operation");
    let _guard = repository.acquire_operation().unwrap();
    let journal = repository
        .path()
        .parent()
        .unwrap()
        .join(PENDING_OPERATION_FILE);
    fs::write(&journal, "pending\n").unwrap();

    assert!(matches!(
        repository.recover_pending_operation(),
        Err(ConnectionRepositoryError::PendingOperation(found)) if found == journal
    ));
    assert!(!repository.path().exists());
}

// 사용자가 아닌 group에 읽기 권한이 열린 connections.yaml은 내용이 올바르더라도 capture가
// 거절되어 이후 mutation이 안전하지 않은 공개 저장소를 정상 상태로 취급하지 않습니다.
#[test]
fn insecure_existing_snapshot_is_rejected() {
    let (_directory, repository) = repository("permissions");
    fs::create_dir_all(repository.path().parent().unwrap()).unwrap();
    fs::write(
        repository.path(),
        "version: 1\nrevision: rev-00000000000000000000000000000000\n",
    )
    .unwrap();
    fs::set_permissions(repository.path(), fs::Permissions::from_mode(0o640)).unwrap();

    assert!(matches!(
        repository.capture(),
        Err(ConnectionRepositoryError::InsecurePermissions(_))
    ));
}

// 닫힌 managed schema와 맞지 않는 opaque binding/account는 typed state로 오인하지 않고
// capture 단계에서 실패해 기존 상태를 바꾸지 않습니다.
#[test]
fn malformed_managed_bindings_and_accounts_fail_closed() {
    let (_directory, repository) = repository("unsupported-managed-state");
    fs::create_dir_all(repository.path().parent().unwrap()).unwrap();
    fs::write(
        repository.path(),
        concat!(
            "version: 1\n",
            "revision: rev-00000000000000000000000000000000\n",
            "bindings:\n  - binding: retained\n",
            "accounts:\n  - account: retained\n",
        ),
    )
    .unwrap();
    fs::set_permissions(repository.path(), fs::Permissions::from_mode(0o600)).unwrap();

    let before = fs::read(repository.path()).unwrap();
    let error = repository.capture().unwrap_err();

    assert!(matches!(
        error,
        ConnectionRepositoryError::InvalidContents(_)
    ));
    assert_eq!(fs::read(repository.path()).unwrap(), before);
}

// 첫 managed upsert는 account와 complete binding을 readable typed YAML로 함께 게시하고,
// 첫 성공 preference를 같은 CAS에 포함하며 exact retry 뒤에도 typed 값이 보존됩니다.
#[test]
fn managed_upsert_round_trips_complete_state_and_first_preference() {
    let (_directory, repository) = repository("managed-upsert");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_managed_upsert(managed_account(), managed_binding("model-a", "medium"))
        .unwrap()
        .unwrap();

    assert_eq!(
        repository.commit(&mutation).unwrap(),
        ConnectionCommit::Committed
    );
    assert_eq!(
        repository.commit(&mutation).unwrap(),
        ConnectionCommit::AlreadyCommitted
    );
    let captured = repository.capture().unwrap();

    assert_eq!(captured.managed_accounts(), &[managed_account()]);
    assert_eq!(
        captured.managed_bindings(),
        &[managed_binding("model-a", "medium")]
    );
    assert_eq!(captured.preference(), Some(&model_target("model-a")));
    let encoded = fs::read_to_string(repository.path()).unwrap();
    assert!(encoded.contains("reasoning_parameters:"));
    assert!(encoded.contains("effort: medium"));
    assert!(!encoded.contains("secret"));
}

// 같은 account에 model을 추가하거나 기존 model profile을 교체할 때 unrelated binding과
// 이미 정해진 preference는 그대로 남고 exact coordinate 하나만 바뀝니다.
#[test]
fn managed_upsert_preserves_unrelated_binding_and_existing_preference() {
    let (_directory, repository) = repository("managed-replace");
    for binding in [
        managed_binding("model-a", "medium"),
        managed_binding("model-b", "medium"),
    ] {
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_managed_upsert(managed_account(), binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
    }
    let replacement = repository
        .capture()
        .unwrap()
        .prepare_managed_upsert(managed_account(), managed_binding("model-b", "high"))
        .unwrap()
        .unwrap();
    repository.commit(&replacement).unwrap();

    let captured = repository.capture().unwrap();
    assert_eq!(
        captured.managed_bindings(),
        &[
            managed_binding("model-a", "medium"),
            managed_binding("model-b", "high"),
        ]
    );
    assert_eq!(captured.preference(), Some(&model_target("model-a")));
}

// managed remove는 선택한 binding만 지우고 같은 account의 다른 model이 있으면 account를
// 보존하며, 마지막 model과 exact matching preference가 사라질 때 둘을 함께 clear합니다.
#[test]
fn managed_remove_preserves_shared_account_then_clears_last_account_and_preference() {
    let (_directory, repository) = repository("managed-remove");
    for binding in [
        managed_binding("model-a", "medium"),
        managed_binding("model-b", "medium"),
    ] {
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_managed_upsert(managed_account(), binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
    }

    let remove_a = repository
        .capture()
        .unwrap()
        .prepare_managed_remove(match model_target("model-a") {
            StartupTarget::Model(ref selection) => selection,
            StartupTarget::HostCodex => unreachable!(),
        })
        .unwrap();
    repository.commit(&remove_a).unwrap();
    let after_a = repository.capture().unwrap();
    assert!(after_a.preference().is_none());
    assert_eq!(after_a.managed_accounts(), &[managed_account()]);
    assert_eq!(
        after_a.managed_bindings(),
        &[managed_binding("model-b", "medium")]
    );

    let remove_b = after_a
        .prepare_managed_remove(match model_target("model-b") {
            StartupTarget::Model(ref selection) => selection,
            StartupTarget::HostCodex => unreachable!(),
        })
        .unwrap();
    repository.commit(&remove_b).unwrap();
    let empty = repository.capture().unwrap();
    assert!(empty.managed_accounts().is_empty());
    assert!(empty.managed_bindings().is_empty());
}

// preference-only CAS는 typed managed arrays를 다시 encode할 때도 의미를 바꾸거나 버리지
// 않아 default 변경이 connection catalog를 손상하지 않습니다.
#[test]
fn preference_mutation_preserves_managed_state() {
    let (_directory, repository) = repository("preference-preserves-managed");
    let managed = repository
        .capture()
        .unwrap()
        .prepare_managed_upsert(managed_account(), managed_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&managed).unwrap();
    let before = repository.capture().unwrap();
    let preference = before
        .prepare_preference(Some(StartupTarget::HostCodex))
        .unwrap()
        .unwrap();
    repository.commit(&preference).unwrap();

    let after = repository.capture().unwrap();
    assert_eq!(after.managed_accounts(), before.managed_accounts());
    assert_eq!(after.managed_bindings(), before.managed_bindings());
    assert_eq!(after.preference(), Some(&StartupTarget::HostCodex));
}

// 새 API는 reserved host Provider를 만들 수 없지만 이미 존재하는 durable host coordinate는
// decoder가 읽어 보존해 이전 공개 상태를 새 binary가 임의로 소실하지 않습니다.
#[test]
fn new_host_managed_state_is_forbidden_while_existing_durable_state_remains_readable() {
    assert!(
        ManagedConnectionAccount::new(
            ProviderId::new("host").unwrap(),
            AccountId::new("default").unwrap(),
            None,
            None,
        )
        .is_err()
    );

    let (_directory, repository) = repository("legacy-host");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_managed_upsert(managed_account(), managed_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let encoded = fs::read_to_string(repository.path())
        .unwrap()
        .replace("qwencloud", "host");
    fs::write(repository.path(), encoded).unwrap();

    let captured = repository.capture().unwrap();
    assert_eq!(
        captured.managed_accounts()[0].provider_id().as_str(),
        "host"
    );
    assert_eq!(
        captured.managed_bindings()[0]
            .complete()
            .binding()
            .provider_id()
            .as_str(),
        "host"
    );
}

// managed YAML도 authored config와 같은 scalar-style 검사를 거쳐 plain overflow가 String으로
// 재분류되는 우회를 막고, 사용자가 명시적으로 quote한 같은 spelling만 String으로 보존합니다.
#[test]
fn managed_profile_numbers_reject_plain_overflow_and_preserve_quoted_string() {
    let (_directory, repository) = repository("managed-profile-number");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_managed_upsert(managed_account(), managed_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let original = fs::read_to_string(repository.path()).unwrap();

    let overflow = original.replace("effort: medium", "value: 1e400");
    fs::write(repository.path(), &overflow).unwrap();
    assert!(matches!(
        repository.capture(),
        Err(ConnectionRepositoryError::InvalidContents(_))
    ));

    let quoted = original.replace("effort: medium", "value: '1e400'");
    fs::write(repository.path(), quoted).unwrap();
    let captured = repository.capture().unwrap();
    assert_eq!(
        captured.managed_bindings()[0]
            .complete()
            .profile()
            .reasoning_parameters()
            .to_json_value(),
        serde_json::json!({"value": "1e400"})
    );
}
