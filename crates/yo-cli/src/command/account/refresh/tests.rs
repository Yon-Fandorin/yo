use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountId, ApiCredential,
    LocalConnectionOperationRepositories, LocalCredentialRepository, ProviderId,
};

use super::{qwencloud_account_session_prompt, refresh_qwencloud_capacity_with};
use crate::{AppError, command::account::qwencloud};

// QwenCloud login prompt는 누락과 만료를 같은 구조 안에서 구분하고 API key가 아니라
// browser session만 저장·교체한다는 행동을 짧게 보여 줍니다.
#[test]
fn qwencloud_session_prompt_distinguishes_first_save_from_replacement() {
    let provider = ProviderId::new("qwencloud").unwrap();
    let account = AccountId::new("default").unwrap();

    let missing = qwencloud_account_session_prompt(&provider, &account, false)
        .render(std::num::NonZeroU16::new(80).unwrap());
    let expired = qwencloud_account_session_prompt(&provider, &account, true)
        .render(std::num::NonZeroU16::new(80).unwrap());

    assert!(missing.contains("+ Browser session\n  Not saved · save for future"));
    assert!(expired.contains("~ Browser session\n  Expired · replace the saved session"));
    assert!(missing.contains("stored locally"));
    assert!(missing.ends_with("Cookie (hidden): "));
    assert!(!missing.contains("credentials.yaml"));
}

struct AccountCredentialFixture {
    root: PathBuf,
    credentials: LocalCredentialRepository,
    provider: ProviderId,
    account: AccountId,
}

impl AccountCredentialFixture {
    fn new(name: &str) -> Self {
        let fixture = Self::without_model(name);
        let add = fixture
            .credentials
            .prepare_set(&fixture.provider, &fixture.account)
            .unwrap();
        fixture
            .credentials
            .commit(&add, Some(&ApiCredential::new("sk-sp-model").unwrap()))
            .unwrap();
        fixture
    }

    fn without_model(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yo-account-session-{}-{name}-{nonce}",
            std::process::id()
        ));
        let credentials = LocalCredentialRepository::new(root.join("credentials.yaml"));
        let provider = ProviderId::new("qwencloud").unwrap();
        let account = AccountId::new("default").unwrap();
        Self {
            root,
            credentials,
            provider,
            account,
        }
    }

    fn persist_session(&self, value: &str) {
        let mutation = self
            .credentials
            .prepare_set_account_session(&self.provider, &self.account)
            .unwrap();
        self.credentials
            .commit_account_session(&mutation, &ApiCredential::new(value).unwrap())
            .unwrap();
    }

    fn repositories(&self) -> LocalConnectionOperationRepositories {
        LocalConnectionOperationRepositories::in_directory(&self.root).unwrap()
    }
}

impl Drop for AccountCredentialFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn qwen_snapshot(provider: &ProviderId, account: &AccountId) -> AccountCapacitySnapshot {
    AccountCapacitySnapshot::new(
        provider.clone(),
        account.clone(),
        vec![AccountCapacityBucket::new(
            Some("qwencloud".to_owned()),
            None,
            Some("moderato".to_owned()),
            None,
            None,
            None,
            None,
        )],
    )
}

// record에 게시하고 그 값으로 한 번만 refresh하며 model API key는 보존합니다.
#[test]
fn missing_qwencloud_session_is_captured_persisted_and_refreshed_once() {
    let fixture = AccountCredentialFixture::new("missing");
    let mut captures = Vec::new();
    let mut refreshes = 0;
    let repositories = fixture.repositories();
    let mut operation = repositories.acquire().unwrap();
    operation.recover_pending_operation().unwrap();
    let credentials = operation.capture_credentials().unwrap();

    let snapshot = refresh_qwencloud_capacity_with(
        &mut operation,
        &fixture.credentials,
        credentials,
        &fixture.provider,
        &fixture.account,
        |expired| {
            captures.push(expired);
            ApiCredential::new("cna=x; login_qwencloud_ticket=fresh")
                .map_err(|error| AppError::single("fixture session", error))
        },
        |session| {
            refreshes += 1;
            assert_eq!(
                session.expose_secret(),
                "cna=x; login_qwencloud_ticket=fresh"
            );
            Ok(qwen_snapshot(&fixture.provider, &fixture.account))
        },
    )
    .unwrap();

    assert_eq!(snapshot.provider(), &fixture.provider);
    assert_eq!(captures, [false]);
    assert_eq!(refreshes, 1);
    let stored = fixture.credentials.capture().unwrap();
    assert_eq!(
        stored
            .resolve(&fixture.provider, &fixture.account)
            .unwrap()
            .expose_secret(),
        "sk-sp-model"
    );
    assert_eq!(
        stored
            .resolve_account_session(&fixture.provider, &fixture.account)
            .unwrap()
            .expose_secret(),
        "cna=x; login_qwencloud_ticket=fresh"
    );
}

// refresh 결과를 반환하며 같은 호출에서 세 번째 request나 prompt를 만들지 않습니다.
#[test]
fn expired_qwencloud_session_is_replaced_and_retried_exactly_once() {
    let fixture = AccountCredentialFixture::new("expired");
    fixture.persist_session("cna=x; login_qwencloud_ticket=expired");
    let mut captures = Vec::new();
    let mut refreshes = 0;
    let repositories = fixture.repositories();
    let mut operation = repositories.acquire().unwrap();
    operation.recover_pending_operation().unwrap();
    let credentials = operation.capture_credentials().unwrap();

    let snapshot = refresh_qwencloud_capacity_with(
        &mut operation,
        &fixture.credentials,
        credentials,
        &fixture.provider,
        &fixture.account,
        |expired| {
            captures.push(expired);
            ApiCredential::new("cna=x; login_qwencloud_ticket=replacement")
                .map_err(|error| AppError::single("fixture replacement", error))
        },
        |session| {
            refreshes += 1;
            if session.expose_secret().ends_with("=expired") {
                Err(qwencloud::expired_session_error())
            } else {
                Ok(qwen_snapshot(&fixture.provider, &fixture.account))
            }
        },
    )
    .unwrap();

    assert_eq!(snapshot.account(), &fixture.account);
    assert_eq!(captures, [true]);
    assert_eq!(refreshes, 2);
    assert_eq!(
        fixture
            .credentials
            .capture()
            .unwrap()
            .resolve_account_session(&fixture.provider, &fixture.account)
            .unwrap()
            .expose_secret(),
        "cna=x; login_qwencloud_ticket=replacement"
    );
}

// prompt 한 번과 전체 remote attempt 두 번의 경계를 넘지 않습니다.
#[test]
fn replacement_rejection_does_not_start_a_third_refresh() {
    let fixture = AccountCredentialFixture::new("replacement-rejected");
    fixture.persist_session("cna=x; login_qwencloud_ticket=expired");
    let mut captures = 0;
    let mut refreshes = 0;
    let repositories = fixture.repositories();
    let mut operation = repositories.acquire().unwrap();
    operation.recover_pending_operation().unwrap();
    let credentials = operation.capture_credentials().unwrap();

    let error = refresh_qwencloud_capacity_with(
        &mut operation,
        &fixture.credentials,
        credentials,
        &fixture.provider,
        &fixture.account,
        |_| {
            captures += 1;
            ApiCredential::new("cna=x; login_qwencloud_ticket=also-expired")
                .map_err(|source| AppError::single("fixture replacement", source))
        },
        |_| {
            refreshes += 1;
            Err::<AccountCapacitySnapshot, _>(qwencloud::expired_session_error())
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("console session expired"));
    assert_eq!(captures, 1);
    assert_eq!(refreshes, 2);
}

// Provider refresh를 시작하지 않아, 저장할 수 없는 secret을 먼저 요구하지 않습니다.
#[test]
fn missing_model_credential_fails_before_cookie_capture() {
    let fixture = AccountCredentialFixture::without_model("missing-model-credential");
    let repositories = fixture.repositories();
    let mut operation = repositories.acquire().unwrap();
    operation.recover_pending_operation().unwrap();
    let credentials = operation.capture_credentials().unwrap();
    let mut captures = 0;
    let mut refreshes = 0;

    let error = refresh_qwencloud_capacity_with(
        &mut operation,
        &fixture.credentials,
        credentials,
        &fixture.provider,
        &fixture.account,
        |_| {
            captures += 1;
            ApiCredential::new("cna=x; login_qwencloud_ticket=unused")
                .map_err(|source| AppError::single("fixture session", source))
        },
        |_| {
            refreshes += 1;
            Ok(qwen_snapshot(&fixture.provider, &fixture.account))
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("has no API credential"));
    assert_eq!(captures, 0);
    assert_eq!(refreshes, 0);
}

// revision CAS가 충돌하고, 새 API key를 보존한 채 account session을 게시하지 않습니다.
#[test]
fn concurrent_credential_change_conflicts_instead_of_replanning_after_capture() {
    let fixture = AccountCredentialFixture::new("capture-conflict");
    let repositories = fixture.repositories();
    let mut operation = repositories.acquire().unwrap();
    operation.recover_pending_operation().unwrap();
    let credentials = operation.capture_credentials().unwrap();

    let error = refresh_qwencloud_capacity_with(
        &mut operation,
        &fixture.credentials,
        credentials,
        &fixture.provider,
        &fixture.account,
        |_| {
            let replacement = fixture
                .credentials
                .prepare_set(&fixture.provider, &fixture.account)
                .unwrap();
            fixture
                .credentials
                .commit(
                    &replacement,
                    Some(&ApiCredential::new("sk-concurrent").unwrap()),
                )
                .unwrap();
            ApiCredential::new("cna=x; login_qwencloud_ticket=stale")
                .map_err(|source| AppError::single("fixture session", source))
        },
        |_| -> Result<AccountCapacitySnapshot, _> {
            panic!("a conflicted account session must not reach the Provider")
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("conflict"));
    let current = fixture.credentials.capture().unwrap();
    assert_eq!(
        current
            .resolve(&fixture.provider, &fixture.account)
            .unwrap()
            .expose_secret(),
        "sk-concurrent"
    );
    assert!(
        current
            .resolve_account_session(&fixture.provider, &fixture.account)
            .is_none()
    );
}

// 덮지 못하고 exact revision conflict로 끝나며 최신 session bytes를 그대로 보존합니다.
#[test]
fn concurrent_session_replacement_is_not_overwritten_after_expiry() {
    let fixture = AccountCredentialFixture::new("replacement-conflict");
    fixture.persist_session("cna=x; login_qwencloud_ticket=expired");
    let repositories = fixture.repositories();
    let mut operation = repositories.acquire().unwrap();
    operation.recover_pending_operation().unwrap();
    let credentials = operation.capture_credentials().unwrap();

    let error = refresh_qwencloud_capacity_with(
        &mut operation,
        &fixture.credentials,
        credentials,
        &fixture.provider,
        &fixture.account,
        |_| {
            let replacement = fixture
                .credentials
                .prepare_set_account_session(&fixture.provider, &fixture.account)
                .unwrap();
            fixture
                .credentials
                .commit_account_session(
                    &replacement,
                    &ApiCredential::new("cna=x; login_qwencloud_ticket=concurrent-replacement")
                        .unwrap(),
                )
                .unwrap();
            ApiCredential::new("cna=x; login_qwencloud_ticket=stale-replacement")
                .map_err(|source| AppError::single("fixture replacement", source))
        },
        |_| Err::<AccountCapacitySnapshot, _>(qwencloud::expired_session_error()),
    )
    .unwrap_err();

    assert!(error.to_string().contains("conflict"));
    assert_eq!(
        fixture
            .credentials
            .capture()
            .unwrap()
            .resolve_account_session(&fixture.provider, &fixture.account)
            .unwrap()
            .expose_secret(),
        "cna=x; login_qwencloud_ticket=concurrent-replacement"
    );
}
