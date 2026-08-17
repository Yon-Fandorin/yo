use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;
use crate::{AccountId, ModelId, ModelSelection, ProviderId, VersionedProfileId};

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

fn stored_account() -> ConnectionAccount {
    ConnectionAccount::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        Some("QwenCloud".to_owned()),
        Some("Default".to_owned()),
    )
    .unwrap()
}

fn stored_binding(model: &str, effort: &str) -> StoredModelBinding {
    let durable = format!(
        r#"{{"provider":"qwencloud","account":"default","model":"{model}","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{"effort":"{effort}"}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1"}}"#
    );
    StoredModelBinding::new(
        crate::CompleteModelBinding::from_durable_json(&durable).unwrap(),
        Some(format!("Model {model}")),
    )
    .unwrap()
}

fn stored_kimi_binding() -> StoredModelBinding {
    StoredModelBinding::new(
        crate::CompleteModelBinding::from_durable_json(
            r#"{"provider":"kimi","account":"team","model":"kimi-k3","connector":"kimi-chat-completions","base_url":"https://api.moonshot.ai/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1048576,"max_output_tokens":131072,"reasoning_parameters":{"effort":"max"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","replay_profile":"kimi-private-local-plaintext/v1"}"#,
        )
        .unwrap(),
        Some("Kimi K3".to_owned()),
    )
    .unwrap()
}

fn stored_kimi_account() -> ConnectionAccount {
    ConnectionAccount::new(
        ProviderId::new("kimi").unwrap(),
        AccountId::new("team").unwrap(),
        Some("Kimi".to_owned()),
        Some("Team".to_owned()),
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
        "revision: rev-00000000000000000000000000000000\n",
    )
    .unwrap();
    fs::set_permissions(repository.path(), fs::Permissions::from_mode(0o640)).unwrap();

    assert!(matches!(
        repository.capture(),
        Err(ConnectionRepositoryError::InsecurePermissions(_))
    ));
}

// 닫힌 stored schema와 맞지 않는 opaque binding/account는 typed state로 오인하지 않고
// capture 단계에서 실패해 기존 상태를 바꾸지 않습니다.
#[test]
fn malformed_models_and_accounts_fail_closed() {
    let (_directory, repository) = repository("unsupported-stored-state");
    fs::create_dir_all(repository.path().parent().unwrap()).unwrap();
    fs::write(
        repository.path(),
        concat!(
            "",
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
        &error,
        ConnectionRepositoryError::InvalidContents(_)
    ));
    assert_eq!(fs::read(repository.path()).unwrap(), before);
}

// 별도 format classifier가 없으므로 top-level version은 일반 unknown field로 닫히고
// 원문은 자동 migration이나 삭제 없이 보존됩니다.
#[test]
fn unknown_top_level_version_is_invalid_connection_state() {
    let (_directory, repository) = repository("unknown-version");
    fs::create_dir_all(repository.path().parent().unwrap()).unwrap();
    let stale = "version: 1\nrevision: rev-00000000000000000000000000000000\n";
    fs::write(repository.path(), stale).unwrap();
    fs::set_permissions(repository.path(), fs::Permissions::from_mode(0o600)).unwrap();

    let error = repository.capture().unwrap_err();
    assert!(matches!(
        error,
        ConnectionRepositoryError::InvalidContents(_)
    ));
    assert_eq!(fs::read_to_string(repository.path()).unwrap(), stale);
}

// binding 내부의 version 오타도 원문을 보존한 일반 invalid-content 오류로 닫힙니다.
#[test]
fn nested_version_field_is_invalid_connection_state() {
    let (_directory, repository) = repository("nested-version");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let original = fs::read_to_string(repository.path()).unwrap();
    let malformed = original.replace("model: model-a", "model: model-a\n    version: 1");
    assert_ne!(malformed, original, "fixture must add the nested field");
    fs::write(repository.path(), &malformed).unwrap();

    let error = repository.capture().unwrap_err();
    assert!(matches!(
        &error,
        ConnectionRepositoryError::InvalidContents(_)
    ));
    assert_eq!(fs::read_to_string(repository.path()).unwrap(), malformed);
}

// 첫 stored upsert는 account와 complete binding을 readable typed YAML로 함께 게시하고,
// 첫 성공 preference를 같은 CAS에 포함하며 exact retry 뒤에도 typed 값이 보존됩니다.
#[test]
fn stored_upsert_round_trips_complete_state_and_first_preference() {
    let (_directory, repository) = repository("stored-upsert");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
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

    assert_eq!(captured.accounts(), &[stored_account()]);
    assert_eq!(captured.models(), &[stored_binding("model-a", "medium")]);
    assert_eq!(captured.preference(), Some(&model_target("model-a")));
    let encoded = fs::read_to_string(repository.path()).unwrap();
    assert!(!encoded.contains("version:"));
    assert!(encoded.contains("reasoning_parameters:"));
    assert!(encoded.contains("effort: medium"));
    assert!(!encoded.contains("verification_profile"));
    assert!(!encoded.contains("secret"));
}

// connections.yaml의 profile은 실행 동작만 저장하므로 폐기된 연결 검증 필드를
// unknown field로 닫고 원문을 자동 변환하지 않습니다.
#[test]
fn durable_profile_rejects_connection_verification_field() {
    let (_directory, repository) = repository("verification-field");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let original = fs::read_to_string(repository.path()).unwrap();
    let malformed = original.replace(
        "tool_capability_policy: local-tools/v1",
        "tool_capability_policy: local-tools/v1\n      verification_profile: semantic-terminal/v1",
    );
    assert_ne!(malformed, original, "fixture must add the retired field");
    fs::write(repository.path(), &malformed).unwrap();

    assert!(matches!(
        repository.capture(),
        Err(ConnectionRepositoryError::InvalidContents(_))
    ));
    assert_eq!(fs::read_to_string(repository.path()).unwrap(), malformed);
}

// durable structured profile 전체의 null은 recursive 내부 null과 구분되어 capture에서
// 실패하고 기존 connections.yaml bytes를 수정하지 않습니다.
#[test]
fn durable_profile_whole_field_null_is_rejected() {
    let (_directory, repository) = repository("whole-field-null");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let encoded = fs::read_to_string(repository.path()).unwrap();
    let malformed = encoded.replace(
        "reasoning_parameters:\n      effort: medium",
        "reasoning_parameters: null",
    );
    assert_ne!(
        malformed, encoded,
        "fixture must replace the structured field"
    );
    fs::write(repository.path(), &malformed).unwrap();

    assert!(matches!(
        repository.capture(),
        Err(ConnectionRepositoryError::InvalidContents(_))
    ));
    assert_eq!(fs::read_to_string(repository.path()).unwrap(), malformed);
}

// Optional durable fields distinguish omission from an explicitly authored null. Null is never
// accepted as an alias for "not present" at the connections.yaml boundary.
#[test]
fn durable_optional_whole_field_nulls_are_rejected() {
    let replacements = [
        (
            "provider_display_name: QwenCloud",
            "provider_display_name: null",
        ),
        (
            "account_display_name: Default",
            "account_display_name: null",
        ),
        (
            "model_display_name: Model model-a",
            "model_display_name: null",
        ),
        (
            "preference:\n  kind: model\n  provider: qwencloud\n  account: default\n  model: model-a",
            "preference: null",
        ),
    ];
    for (index, (needle, replacement)) in replacements.into_iter().enumerate() {
        let (_directory, repository) = repository(&format!("optional-null-{index}"));
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
        let encoded = fs::read_to_string(repository.path()).unwrap();
        let malformed = encoded.replace(needle, replacement);
        assert_ne!(malformed, encoded, "fixture must replace {needle}");
        fs::write(repository.path(), &malformed).unwrap();

        assert!(matches!(
            repository.capture(),
            Err(ConnectionRepositoryError::InvalidContents(_))
        ));
        assert_eq!(fs::read_to_string(repository.path()).unwrap(), malformed);
    }
}

// connections.yaml reader는 명시적 semantic-only도 호환 입력으로 읽지만 producer는 계속
// 생략하고, null·unknown·duplicate는 private profile이나 omission으로 축약하지 않습니다.
#[test]
fn stored_replay_profile_is_presence_aware_and_closed() {
    let (_directory, semantic_repository) = repository("stored-explicit-semantic-replay-profile");
    let mutation = semantic_repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_kimi_account(), stored_kimi_binding())
        .unwrap()
        .unwrap();
    semantic_repository.commit(&mutation).unwrap();
    let private = fs::read_to_string(semantic_repository.path()).unwrap();
    fs::write(
        semantic_repository.path(),
        private.replace(
            "replay_profile: kimi-private-local-plaintext/v1",
            "replay_profile: semantic-only/v1",
        ),
    )
    .unwrap();
    let explicit_semantic = semantic_repository.capture().unwrap();
    assert_eq!(
        explicit_semantic.models()[0]
            .complete()
            .profile()
            .replay_profile()
            .as_str(),
        "semantic-only/v1"
    );

    for replacement in [
        "replay_profile: null",
        "replay_profile: unknown/v1",
        concat!(
            "replay_profile: kimi-private-local-plaintext/v1\n",
            "      replay_profile: kimi-private-local-plaintext/v1"
        ),
    ] {
        let (_directory, repository) = repository("stored-replay-profile");
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(stored_kimi_account(), stored_kimi_binding())
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
        let original = fs::read_to_string(repository.path()).unwrap();
        let malformed = original.replace(
            "replay_profile: kimi-private-local-plaintext/v1",
            replacement,
        );
        fs::write(repository.path(), malformed).unwrap();

        assert!(
            matches!(
                repository.capture(),
                Err(ConnectionRepositoryError::InvalidContents(_))
            ),
            "{replacement}"
        );
    }
}

// connections.yaml은 Kimi private replay 동의를 readable profile field로 보존하지만
// 비밀이나 provider-private Session payload 자체는 섞지 않고 capture에서 같은 타입을 복원합니다.
#[test]
fn stored_kimi_binding_round_trips_the_private_replay_authorization_only() {
    let (_directory, repository) = repository("stored-kimi-private-replay");
    let account = ConnectionAccount::new(
        ProviderId::new("kimi").unwrap(),
        AccountId::new("team").unwrap(),
        Some("Kimi".to_owned()),
        Some("Team".to_owned()),
    )
    .unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(account, stored_kimi_binding())
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();

    let encoded = fs::read_to_string(repository.path()).unwrap();
    assert!(encoded.contains("replay_profile: kimi-private-local-plaintext/v1"));
    assert!(!encoded.contains("reasoning_content"));
    assert!(!encoded.contains("candidate"));
    assert_eq!(
        repository.capture().unwrap().models(),
        &[stored_kimi_binding()]
    );
}

// credential rotation은 public binding이 byte-for-byte 같아도 새 planned revision을 만들어
// journal recovery가 credential-first 절단점 뒤 exact public winner를 식별할 수 있습니다.
#[test]
fn stored_connect_forces_a_new_public_revision_for_equal_state() {
    let (_directory, repository) = repository("stored-connect-epoch");
    let first = repository
        .capture()
        .unwrap()
        .prepare_model_connect(stored_account(), stored_binding("model-a", "medium"))
        .unwrap();
    repository.commit(&first).unwrap();
    let captured = repository.capture().unwrap();

    let rotation = captured
        .prepare_model_connect(stored_account(), stored_binding("model-a", "medium"))
        .unwrap();

    assert_ne!(rotation.expected_revision(), rotation.planned_revision());
    assert_eq!(rotation.expected_revision(), captured.revision());
    repository.commit(&rotation).unwrap();
    assert_eq!(repository.capture().unwrap().models(), captured.models());
}

// Catalog seed도 model definition과 같은 repository revision에 저장되며, producer는 account
// 표시 이름을 catalog row에 중복 쓰지 않고 durable account 한 곳에서만 복원합니다.
#[test]
fn catalog_only_group_round_trips_as_one_stored_definition() {
    let (_directory, repository) = repository("catalog-group");
    let account = stored_account();
    let seed = ConnectionCatalogSeed::built_in(
        VersionedProfileId::new("qwencloud-token-plan-team-intl/v1").unwrap(),
        account.provider_id().clone(),
        account.account_id().clone(),
        account.provider_display_name().map(str::to_owned),
        account.account_display_name().map(str::to_owned),
    )
    .unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_group_replace(account, Vec::new(), Some(seed.clone()))
        .unwrap();
    repository.commit(&mutation).unwrap();

    let snapshot = repository.capture().unwrap();
    assert_eq!(snapshot.catalog_seeds(), &[seed]);
    assert!(snapshot.models().is_empty());
    assert!(snapshot.preference().is_none());
    assert_eq!(
        snapshot
            .qwencloud_catalog_seed(
                &ProviderId::new("qwencloud").unwrap(),
                &AccountId::new("default").unwrap(),
            )
            .unwrap()
            .unwrap()
            .models()
            .len(),
        18
    );
    let raw = std::fs::read_to_string(repository.path()).unwrap();
    assert!(raw.contains("kind: built_in"));
    assert_eq!(raw.matches("provider_display_name:").count(), 1);
    assert_eq!(raw.matches("account_display_name:").count(), 1);
}

// Group replacement은 같은 pair의 목록을 merge하지 않고 통째로 바꾸며, 제거된 exact
// ModelTarget preference도 같은 public CAS에서 함께 지웁니다.
#[test]
fn group_replace_removes_omitted_models_and_their_preference() {
    let (_directory, repository) = repository("whole-group-replace");
    let first = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&first).unwrap();
    let replacement = repository
        .capture()
        .unwrap()
        .prepare_group_replace(
            stored_account(),
            vec![stored_binding("model-b", "medium")],
            None,
        )
        .unwrap();
    repository.commit(&replacement).unwrap();

    let snapshot = repository.capture().unwrap();
    assert!(snapshot.preference().is_none());
    assert_eq!(snapshot.models().len(), 1);
    assert_eq!(snapshot.models()[0].selection().model().as_str(), "model-b");
}

// 같은 account에 model을 추가하거나 기존 model profile을 교체할 때 unrelated binding과
// 이미 정해진 preference는 그대로 남고 exact coordinate 하나만 바뀝니다.
#[test]
fn stored_upsert_preserves_unrelated_binding_and_existing_preference() {
    let (_directory, repository) = repository("stored-replace");
    for binding in [
        stored_binding("model-a", "medium"),
        stored_binding("model-b", "medium"),
    ] {
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(stored_account(), binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
    }
    let replacement = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-b", "high"))
        .unwrap()
        .unwrap();
    repository.commit(&replacement).unwrap();

    let captured = repository.capture().unwrap();
    assert_eq!(
        captured.models(),
        &[
            stored_binding("model-a", "medium"),
            stored_binding("model-b", "high"),
        ]
    );
    assert_eq!(captured.preference(), Some(&model_target("model-a")));
}

// stored remove는 선택한 binding과 exact matching preference를 함께 지우되 같은 account의
// 다른 model이 있으면 account를 보존하고, 마지막 binding을 지운 뒤에만 account를 제거합니다.
#[test]
fn stored_remove_preserves_shared_account_then_clears_last_account_and_preference() {
    let (_directory, repository) = repository("stored-remove");
    for binding in [
        stored_binding("model-a", "medium"),
        stored_binding("model-b", "medium"),
    ] {
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(stored_account(), binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
    }

    let remove_a = repository
        .capture()
        .unwrap()
        .prepare_model_remove(match model_target("model-a") {
            StartupTarget::Model(ref selection) => selection,
            StartupTarget::HostCodex => unreachable!(),
        })
        .unwrap();
    repository.commit(&remove_a).unwrap();
    let after_a = repository.capture().unwrap();
    assert!(after_a.preference().is_none());
    assert_eq!(after_a.accounts(), &[stored_account()]);
    assert_eq!(after_a.models(), &[stored_binding("model-b", "medium")]);

    let remove_b = after_a
        .prepare_model_remove(match model_target("model-b") {
            StartupTarget::Model(ref selection) => selection,
            StartupTarget::HostCodex => unreachable!(),
        })
        .unwrap();
    repository.commit(&remove_b).unwrap();
    let empty = repository.capture().unwrap();
    assert!(empty.accounts().is_empty());
    assert!(empty.models().is_empty());
}

// preference-only CAS는 typed stored arrays를 다시 encode할 때도 의미를 바꾸거나 버리지
// 않아 default 변경이 connection catalog를 손상하지 않습니다.
#[test]
fn preference_mutation_preserves_stored_state() {
    let (_directory, repository) = repository("preference-preserves-stored");
    let stored = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&stored).unwrap();
    let before = repository.capture().unwrap();
    let preference = before
        .prepare_preference(Some(StartupTarget::HostCodex))
        .unwrap()
        .unwrap();
    repository.commit(&preference).unwrap();

    let after = repository.capture().unwrap();
    assert_eq!(after.accounts(), before.accounts());
    assert_eq!(after.models(), before.models());
    assert_eq!(after.preference(), Some(&StartupTarget::HostCodex));
}

// 새 API는 reserved host Provider를 만들 수 없지만 이미 존재하는 durable host coordinate는
// decoder가 읽어 보존해 이전 공개 상태를 새 binary가 임의로 소실하지 않습니다.
#[test]
fn new_host_stored_state_is_forbidden_while_existing_durable_state_remains_readable() {
    assert!(
        ConnectionAccount::new(
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
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let encoded = fs::read_to_string(repository.path())
        .unwrap()
        .replace("qwencloud", "host");
    fs::write(repository.path(), encoded).unwrap();

    let captured = repository.capture().unwrap();
    assert_eq!(captured.accounts()[0].provider_id().as_str(), "host");
    assert_eq!(
        captured.models()[0]
            .complete()
            .binding()
            .provider_id()
            .as_str(),
        "host"
    );
}

// stored YAML도 64-bit 경계의 exact integer/float/String variant를 보존하고, non-finite
// overflow는 거절하며 사용자가 명시적으로 quote한 같은 spelling만 String으로 보존합니다.
#[test]
fn stored_profile_numbers_pin_range_fallback_and_reject_nonfinite_values() {
    let (_directory, repository) = repository("stored-profile-number");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let original = fs::read_to_string(repository.path()).unwrap();

    for (authored, expected) in [
        ("18446744073709551615", serde_json::json!(u64::MAX)),
        (
            "18446744073709551616",
            serde_json::json!(18_446_744_073_709_551_616.0_f64),
        ),
        (
            "340282366920938463463374607431768211456",
            serde_json::json!(340_282_366_920_938_463_463_374_607_431_768_211_456.0_f64),
        ),
        ("-9223372036854775808", serde_json::json!(i64::MIN)),
        (
            "-9223372036854775809",
            serde_json::json!(-9_223_372_036_854_775_809.0_f64),
        ),
        ("0xffffffffffffffff", serde_json::json!(u64::MAX)),
        (
            "0x10000000000000000",
            serde_json::json!("0x10000000000000000"),
        ),
    ] {
        let replacement = original.replace("effort: medium", &format!("value: {authored}"));
        fs::write(repository.path(), replacement).unwrap();
        let captured = repository
            .capture()
            .unwrap_or_else(|error| panic!("{authored}: {error}"));
        assert_eq!(
            captured.models()[0]
                .complete()
                .profile()
                .reasoning_parameters()
                .to_json_value()["value"],
            expected,
            "{authored}"
        );
    }

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
        captured.models()[0]
            .complete()
            .profile()
            .reasoning_parameters()
            .to_json_value(),
        serde_json::json!({"value": "1e400"})
    );
}

// last_failure는 complete binding과 분리된 warning-only 상태로 round-trip하고 성공 관찰은
// 값이 있을 때만 새 revision으로 제거하며, 이미 비어 있으면 불필요한 write를 준비하지 않습니다.
#[test]
fn model_observation_round_trips_and_success_clears_only_present_state() {
    let (_directory, repository) = repository("model-observation-round-trip");
    let binding = stored_binding("model-a", "medium");
    let complete = binding.complete().clone();
    let selection = binding.selection();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), binding)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();

    let failure =
        ModelLastFailure::new(ModelRequestFailureKind::RateLimited, "2026-08-17T09:10:11Z")
            .unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_observation(&selection, &complete, Some(failure.clone()))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();

    let captured = repository.capture().unwrap();
    assert_eq!(captured.models()[0].last_failure(), Some(&failure));
    let encoded = fs::read_to_string(repository.path()).unwrap();
    assert!(encoded.contains("kind: rate_limited"));
    assert!(encoded.contains("observed_at: 2026-08-17T09:10:11Z"));

    let mutation = captured
        .prepare_model_observation(&selection, &complete, None)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let cleared = repository.capture().unwrap();
    assert!(cleared.models()[0].last_failure().is_none());
    assert!(
        cleared
            .prepare_model_observation(&selection, &complete, None)
            .unwrap()
            .is_none()
    );
    assert!(
        !fs::read_to_string(repository.path())
            .unwrap()
            .contains("last_failure")
    );
}

// grouped replacement은 같은 좌표와 exact complete binding에만 기존 warning을 보존하고,
// profile이 달라 새 binding epoch가 열리면 오래된 관찰을 새 정의로 넘기지 않습니다.
#[test]
fn grouped_replacement_retains_failure_only_for_an_exact_complete_binding() {
    let (_directory, repository) = repository("group-observation-retention");
    let initial = stored_binding("model-a", "medium");
    let complete = initial.complete().clone();
    let selection = initial.selection();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), initial)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let failure = ModelLastFailure::new(
        ModelRequestFailureKind::Authentication,
        "2026-08-17T09:10:11Z",
    )
    .unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_observation(&selection, &complete, Some(failure.clone()))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();

    let mutation = repository
        .capture()
        .unwrap()
        .prepare_group_replace(
            stored_account(),
            vec![stored_binding("model-a", "medium")],
            None,
        )
        .unwrap();
    repository.commit(&mutation).unwrap();
    assert_eq!(
        repository.capture().unwrap().models()[0].last_failure(),
        Some(&failure)
    );

    let mutation = repository
        .capture()
        .unwrap()
        .prepare_group_replace(
            stored_account(),
            vec![stored_binding("model-a", "high")],
            None,
        )
        .unwrap();
    repository.commit(&mutation).unwrap();
    assert!(
        repository.capture().unwrap().models()[0]
            .last_failure()
            .is_none()
    );
}

// durable observation은 closed kind와 canonical UTC whole-second timestamp만 받아들여 null,
// fractional/offset/noncanonical timestamp, unknown kind가 parser-dependent 상태가 되지 않습니다.
#[test]
fn durable_last_failure_shape_is_closed_and_canonical() {
    assert!(
        ModelLastFailure::new(ModelRequestFailureKind::Protocol, "2026-08-17T09:10:11Z").is_ok()
    );
    for invalid in [
        "2026-08-17T09:10:11.1Z",
        "2026-08-17T18:10:11+09:00",
        "2026-08-17 09:10:11Z",
        "not-a-timestamp",
    ] {
        assert!(
            ModelLastFailure::new(ModelRequestFailureKind::Protocol, invalid).is_err(),
            "{invalid}"
        );
    }

    let (_directory, repository) = repository("closed-observation-wire");
    let binding = stored_binding("model-a", "medium");
    let complete = binding.complete().clone();
    let selection = binding.selection();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), binding)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let failure =
        ModelLastFailure::new(ModelRequestFailureKind::Protocol, "2026-08-17T09:10:11Z").unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_observation(&selection, &complete, Some(failure))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let valid = fs::read_to_string(repository.path()).unwrap();

    for (needle, replacement) in [
        ("last_failure:\n", "last_failure: null\n"),
        ("kind: protocol", "kind: future_failure"),
        (
            "observed_at: 2026-08-17T09:10:11Z",
            "observed_at: 2026-08-17T09:10:11.1Z",
        ),
    ] {
        let malformed = valid.replace(needle, replacement);
        assert_ne!(malformed, valid, "fixture must replace {needle:?}");
        fs::write(repository.path(), &malformed).unwrap();
        assert!(matches!(
            repository.capture(),
            Err(ConnectionRepositoryError::InvalidContents(_))
        ));
    }
}
