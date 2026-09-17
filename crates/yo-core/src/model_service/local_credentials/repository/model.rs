use std::fmt::{self, Debug, Formatter, Result as FmtResult};

use super::{
    super::super::LocalCredentialStoreError,
    mutation::{
        CredentialMutationAction, PreparedAccountSessionMutation, PreparedCredentialMutation,
    },
};
use crate::model_service::{AccountId, ApiCredential, CredentialStore, ProviderId};

/// 하나의 전체 credential snapshot을 비교 교환하는 private opaque receipt입니다.
///
/// 진단 projection에서는 내부 token을 의도적으로 숨깁니다. token은 credential snapshot과
/// 권한이 제한된 connection-operation journal에만 저장할 수 있습니다.
#[derive(Clone, Eq, PartialEq)]
pub struct CredentialRevision(CredentialRevisionKind);

#[derive(Clone, Eq, PartialEq)]
enum CredentialRevisionKind {
    Absent,
    Managed(String),
    Derived(String),
}

impl CredentialRevision {
    #[must_use]
    pub const fn is_absent(&self) -> bool {
        matches!(self.0, CredentialRevisionKind::Absent)
    }

    pub(in crate::model_service::local_credentials) fn absent() -> Self {
        Self(CredentialRevisionKind::Absent)
    }

    pub(in crate::model_service::local_credentials) fn managed(token: String) -> Self {
        Self(CredentialRevisionKind::Managed(token))
    }

    pub(in crate::model_service::local_credentials) fn derived(token: String) -> Self {
        Self(CredentialRevisionKind::Derived(token))
    }

    pub(in crate::model_service::local_credentials) fn managed_token(&self) -> Option<&str> {
        match &self.0 {
            CredentialRevisionKind::Managed(token) => Some(token),
            CredentialRevisionKind::Absent | CredentialRevisionKind::Derived(_) => None,
        }
    }

    pub(crate) fn operation_journal_token(&self) -> &str {
        match &self.0 {
            CredentialRevisionKind::Absent => "absent",
            CredentialRevisionKind::Managed(token) | CredentialRevisionKind::Derived(token) => {
                token
            },
        }
    }

    pub(crate) fn from_operation_journal(value: &str) -> Option<Self> {
        if value == "absent" {
            return Some(Self::absent());
        }
        parse_managed_revision_token(value).or_else(|| parse_derived_revision_token(value))
    }
}

impl Debug for CredentialRevision {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        if self.is_absent() {
            formatter.write_str("CredentialRevision(Absent)")
        } else {
            formatter.write_str("CredentialRevision([PRIVATE])")
        }
    }
}

/// 정확한 Provider-and-Account 해석으로만 secret 값에 접근할 수 있는 불변 credential
/// snapshot입니다. `ApiCredential`의 redaction 동작을 유지합니다.
#[derive(Clone)]
pub struct CredentialSnapshot {
    revision: CredentialRevision,
    credentials: CredentialStore,
    account_sessions: CredentialStore,
}

impl CredentialSnapshot {
    pub(in crate::model_service::local_credentials) fn from_stored(
        revision: CredentialRevision,
        credentials: CredentialStore,
        account_sessions: CredentialStore,
    ) -> Self {
        Self {
            revision,
            credentials,
            account_sessions,
        }
    }

    #[must_use]
    pub const fn revision(&self) -> &CredentialRevision {
        &self.revision
    }

    #[must_use]
    pub fn resolve(&self, provider: &ProviderId, account: &AccountId) -> Option<&ApiCredential> {
        self.credentials.resolve(provider, account)
    }

    /// 정확한 Provider-and-Account에 대한 optional account-observation session을 해석합니다.
    ///
    /// Model dispatch는 계속 [`Self::resolve`]를 사용해야 합니다. 이 secret은 고정된
    /// remote origin을 소유한 account-capacity 경계를 위해 예약되어 있습니다.
    #[must_use]
    pub fn resolve_account_session(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Option<&ApiCredential> {
        self.account_sessions.resolve(provider, account)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.credentials.is_empty()
    }

    #[must_use]
    pub const fn credentials(&self) -> &CredentialStore {
        &self.credentials
    }

    /// 현재 관찰한 credential revision을 기준으로 정확한 account-session mutation을 준비합니다.
    ///
    /// snapshot에서 준비하면 호출자가 실제로 확인한 상태에 이후 secret capture와 remote 작업을
    /// 결합할 수 있어 commit 시점에 조용히 다시 계획하지 않습니다.
    pub fn prepare_set_account_session(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<PreparedAccountSessionMutation, LocalCredentialStoreError> {
        if self.resolve(provider, account).is_none() {
            return Err(LocalCredentialStoreError::InvalidMutation);
        }
        let action = if self.resolve_account_session(provider, account).is_some() {
            CredentialMutationAction::Replace
        } else {
            CredentialMutationAction::Add
        };
        Ok(PreparedAccountSessionMutation::new(
            self.revision.clone(),
            new_revision()?,
            provider.clone(),
            account.clone(),
            action,
        ))
    }

    pub(crate) fn matches_expected(&self, mutation: &PreparedCredentialMutation) -> bool {
        &self.revision == mutation.expected_revision()
            && mutation.action().matches_presence(
                self.resolve(mutation.provider(), mutation.account())
                    .is_some(),
            )
    }

    pub(crate) fn matches_planned(&self, mutation: &PreparedCredentialMutation) -> bool {
        if &self.revision != mutation.planned_revision() {
            return false;
        }
        let present = self
            .resolve(mutation.provider(), mutation.account())
            .is_some();
        match mutation.action() {
            CredentialMutationAction::Add | CredentialMutationAction::Replace => present,
            CredentialMutationAction::Remove => !present,
        }
    }
}

impl Debug for CredentialSnapshot {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter
            .debug_struct("CredentialSnapshot")
            .field("revision", &self.revision)
            .field("credential_count", &self.credentials.len())
            .field("account_session_count", &self.account_sessions.len())
            .finish()
    }
}

pub(in crate::model_service::local_credentials) fn new_revision()
-> Result<CredentialRevision, LocalCredentialStoreError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| LocalCredentialStoreError::Randomness(error.to_string()))?;
    let mut token = String::with_capacity(37);
    token.push_str("crev-");
    for byte in bytes {
        use fmt::Write as _;
        write!(token, "{byte:02x}").expect("formatting into a String cannot fail");
    }
    Ok(CredentialRevision::managed(token))
}

pub(in crate::model_service::local_credentials) fn parse_managed_revision_token(
    revision: &str,
) -> Option<CredentialRevision> {
    let valid = revision.len() == 37
        && revision.starts_with("crev-")
        && revision[5..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    valid.then(|| CredentialRevision::managed(revision.to_owned()))
}

pub(in crate::model_service::local_credentials) fn parse_derived_revision_token(
    revision: &str,
) -> Option<CredentialRevision> {
    let valid = revision.len() == 120
        && revision.starts_with("derived-")
        && revision[8..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    valid.then(|| CredentialRevision::derived(revision.to_owned()))
}
