use std::result;

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use super::{Result, SecretStoreError};
use crate::{ModelId, ProviderId};

/// 실시간 인증 응답에서 얻은 계정 식별자입니다. 설정 슬롯과 호환되지 않습니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveAuthenticatedAccount(String);

impl LiveAuthenticatedAccount {
    /// 호출자는 인증된 실시간 프로토콜 증거에서만 이 값을 생성해야 합니다.
    pub fn from_live_observation(identity: String) -> Result<Self> {
        if identity.is_empty() || identity.len() > 4096 || identity.chars().any(char::is_control) {
            return Err(SecretStoreError);
        }
        Ok(Self(identity))
    }
}

/// 공급자·모델·인증된 계정의 정확한 비밀 재사용 경계입니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretDestination {
    provider: String,
    model: String,
    authenticated_account: String,
}

impl SecretDestination {
    /// 설정된 AccountId가 아닌 실시간 인증 증거로만 경계를 구성합니다.
    pub fn from_live_authentication(
        provider: &ProviderId,
        model: &ModelId,
        evidence: &LiveAuthenticatedAccount,
    ) -> Result<Self> {
        Ok(Self {
            provider: provider.as_str().to_owned(),
            model: model.as_str().to_owned(),
            authenticated_account: evidence.0.clone(),
        })
    }

    /// 공개 공급자 식별자입니다.
    pub fn provider(&self) -> &str {
        &self.provider
    }
    /// 공개 모델 식별자입니다.
    pub fn model(&self) -> &str {
        &self.model
    }
    /// 인증된 공개 계정 식별자입니다.
    pub fn authenticated_account(&self) -> &str {
        &self.authenticated_account
    }
}

/// 사용자만 선택할 수 있는 로컬 보존 정책입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetentionPolicy {
    /// 1–365일의 명시적 보존 기간입니다.
    ForDays(u16),
    /// 사용자가 확인한 정확한 만료 시각을 저장 시점에도 유지합니다.
    ForDaysAt {
        /// 사용자가 선택한 1–365일의 기간입니다.
        days: u16,
        /// 선택 화면에서 확인한 Unix 초 단위 만료 시각입니다.
        expires_at: u64,
    },
    /// 명시적 삭제 전까지 보존합니다.
    UntilDeleted,
}

/// 인증 후에만 외부로 반환하는 공개 항목 메타데이터입니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretMetadata {
    destination: SecretDestination,
    scope: String,
    title: String,
    retention: RetentionKind,
    #[serde(deserialize_with = "required_option")]
    expires_at: Option<u64>,
}

impl SecretMetadata {
    /// 공개 목적지와 범위에서만 유도되는 안정적인 로컬 삭제 선택자입니다.
    pub fn public_id(&self) -> Result<String> {
        destination_scope_identity(&self.destination, &self.scope)
    }
    pub(super) fn new(
        destination: &SecretDestination,
        scope: &str,
        title: &str,
        policy: RetentionPolicy,
        now: u64,
    ) -> Result<Self> {
        if !valid_scope(scope)
            || title.is_empty()
            || title.len() > 80
            || title.chars().any(char::is_control)
        {
            return Err(SecretStoreError);
        }
        let (retention, expires_at) = match policy {
            RetentionPolicy::ForDays(days @ 1..=365) => (
                RetentionKind::Timed,
                Some(
                    now.checked_add(u64::from(days) * 86400)
                        .ok_or(SecretStoreError)?,
                ),
            ),
            RetentionPolicy::ForDaysAt {
                days: days @ 1..=365,
                expires_at,
            } => {
                let latest = now
                    .checked_add(u64::from(days) * 86400)
                    .ok_or(SecretStoreError)?;
                if expires_at <= now || expires_at > latest {
                    return Err(SecretStoreError);
                }
                (RetentionKind::Timed, Some(expires_at))
            },
            RetentionPolicy::UntilDeleted => (RetentionKind::UntilDeleted, None),
            _ => return Err(SecretStoreError),
        };
        Ok(Self {
            destination: destination.clone(),
            scope: scope.to_owned(),
            title: title.to_owned(),
            retention,
            expires_at,
        })
    }
    pub(super) fn validate(&self) -> Result<()> {
        if !valid_scope(&self.scope)
            || self.title.is_empty()
            || self.title.len() > 80
            || self.title.chars().any(char::is_control)
            || self.destination.provider.is_empty()
            || self.destination.model.is_empty()
            || self.destination.authenticated_account.is_empty()
            || self.destination.authenticated_account.len() > 4096
            || self
                .destination
                .authenticated_account
                .chars()
                .any(char::is_control)
            || !matches!(
                (self.retention, self.expires_at),
                (RetentionKind::Timed, Some(_)) | (RetentionKind::UntilDeleted, None)
            )
        {
            return Err(SecretStoreError);
        }
        Ok(())
    }
    /// 정확한 재사용 목적지입니다.
    pub fn destination(&self) -> &SecretDestination {
        &self.destination
    }
    /// 모델이 제공한 공개 범위입니다.
    pub fn scope(&self) -> &str {
        &self.scope
    }
    /// 공개 표시 제목입니다.
    pub fn title(&self) -> &str {
        &self.title
    }
    /// Unix 초 단위 만료 시각이며 None은 명시적 삭제 정책입니다.
    pub const fn expires_at(&self) -> Option<u64> {
        self.expires_at
    }
}

pub(super) fn destination_scope_identity(
    destination: &SecretDestination,
    scope: &str,
) -> Result<String> {
    let bytes = serde_json::to_vec(&(destination, scope)).map_err(|_| SecretStoreError)?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RetentionKind {
    Timed,
    UntilDeleted,
}

pub(super) fn valid_scope(scope: &str) -> bool {
    let edge = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    !scope.is_empty()
        && scope.len() <= 128
        && edge(scope.as_bytes()[0])
        && edge(scope.as_bytes()[scope.len() - 1])
        && scope
            .bytes()
            .all(|b| edge(b) || matches!(b, b'.' | b'_' | b'-'))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Generation {
    pub format: String,
    pub state: GenerationState,
    pub generation: String,
    #[serde(deserialize_with = "required_option")]
    pub entry: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub predecessor_generation: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub predecessor_entry: Option<String>,
    pub destination: SecretDestination,
    pub scope: String,
    pub title: String,
    #[serde(deserialize_with = "required_option")]
    pub retention: Option<SecretMetadata>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GenerationState {
    Entry,
    Tombstone,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Barrier {
    pub format: String,
    pub operation: Operation,
    pub destination: SecretDestination,
    pub scope: String,
    #[serde(deserialize_with = "required_option")]
    pub prior: Option<Generation>,
    pub successor: Generation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    Create,
    Replace,
    Delete,
}

pub(super) fn new_id() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| SecretStoreError)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub(super) fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

fn required_option<'de, D, T>(deserializer: D) -> result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
