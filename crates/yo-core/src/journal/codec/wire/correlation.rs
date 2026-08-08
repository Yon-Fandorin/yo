use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use super::JournalCodecError;
use crate::{
    ContinuationStrategy, JournalSequence, ReplayExecutor, TurnId,
    journal::codec::{
        BindingCloseReason, CacheState, DetailAvailability, ExchangeDirection, ExchangeKind,
        OperationId, TransitionMode, VersionedIdentity,
    },
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireVersionedIdentity {
    pub(super) schema: String,
    pub(super) value: String,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireExchangeKind {
    Request,
    Response,
    Notification,
    ServerRequest,
    Retry,
    TerminalOutcome,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireExchangeDirection {
    YoToBackend,
    BackendToYo,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireDetailAvailability {
    Persisted,
    Volatile,
    Missing,
    Unsupported,
    Unpersisted,
    Redacted,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireTransitionMode {
    Initial,
    ExactReplay,
    LossyHandoff,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireCacheState {
    NotApplicable,
    Lost,
    Unknown,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireBindingCloseReason {
    Replaced,
    Revoked,
    Exhausted,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireResumableStatus {
    Completed,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireBindingTransition {
    pub(super) mode: WireTransitionMode,
    pub(super) cache: WireCacheState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) source_anchor_sequence: Option<u64>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum WireContinuationStrategy {
    ExactReplay { executor: WireReplayExecutor },
    BackendManagedState,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireReplayExecutor {
    LocalClient,
    ManagedServer,
}

impl From<ContinuationStrategy> for WireContinuationStrategy {
    fn from(value: ContinuationStrategy) -> Self {
        match value {
            ContinuationStrategy::ExactReplay { executor } => Self::ExactReplay {
                executor: executor.into(),
            },
            ContinuationStrategy::BackendManagedState => Self::BackendManagedState,
        }
    }
}

impl From<WireContinuationStrategy> for ContinuationStrategy {
    fn from(value: WireContinuationStrategy) -> Self {
        match value {
            WireContinuationStrategy::ExactReplay { executor } => Self::ExactReplay {
                executor: executor.into(),
            },
            WireContinuationStrategy::BackendManagedState => Self::BackendManagedState,
        }
    }
}

impl From<ReplayExecutor> for WireReplayExecutor {
    fn from(value: ReplayExecutor) -> Self {
        match value {
            ReplayExecutor::LocalClient => Self::LocalClient,
            ReplayExecutor::ManagedServer => Self::ManagedServer,
        }
    }
}

impl From<WireReplayExecutor> for ReplayExecutor {
    fn from(value: WireReplayExecutor) -> Self {
        match value {
            WireReplayExecutor::LocalClient => Self::LocalClient,
            WireReplayExecutor::ManagedServer => Self::ManagedServer,
        }
    }
}

pub(super) fn encode_identity(
    identity: &VersionedIdentity,
) -> Result<WireVersionedIdentity, JournalCodecError> {
    validate_ascii(identity.schema(), "identity schema")?;
    validate_value(identity.value(), "identity value")?;
    Ok(WireVersionedIdentity {
        schema: identity.schema().to_owned(),
        value: identity.value().to_owned(),
    })
}

pub(super) fn decode_identity(
    identity: WireVersionedIdentity,
) -> Result<VersionedIdentity, JournalCodecError> {
    validate_ascii(&identity.schema, "identity schema")?;
    validate_value(&identity.value, "identity value")?;
    Ok(VersionedIdentity::new(identity.schema, identity.value))
}

pub(super) fn validate_ascii(value: &str, name: &str) -> Result<(), JournalCodecError> {
    if value.is_empty() || value.len() > 128 || !value.is_ascii() {
        return Err(JournalCodecError::new(format!(
            "{name} must be non-empty ASCII of at most 128 bytes"
        )));
    }
    Ok(())
}

pub(super) fn validate_value(value: &str, name: &str) -> Result<(), JournalCodecError> {
    if value.is_empty() || value.len() > 4096 {
        return Err(JournalCodecError::new(format!(
            "{name} must be non-empty UTF-8 of at most 4096 bytes"
        )));
    }
    Ok(())
}

pub(super) fn operation_id(value: String) -> Result<OperationId, JournalCodecError> {
    let parsed = value.parse::<uuid::Uuid>().map_err(|_| {
        JournalCodecError::new("operation_id must be a canonical lowercase hyphenated UUIDv4")
    })?;
    if parsed.to_string() != value {
        return Err(JournalCodecError::new(
            "operation_id must be a canonical lowercase hyphenated UUIDv4",
        ));
    }
    OperationId::from_uuid(parsed)
        .ok_or_else(|| JournalCodecError::new("operation_id must be a canonical UUIDv4"))
}

pub(super) fn positive(value: u64, name: &str) -> Result<u64, JournalCodecError> {
    (value > 0)
        .then_some(value)
        .ok_or_else(|| JournalCodecError::new(format!("{name} must be positive")))
}

pub(super) fn sequence(value: u64, name: &str) -> Result<JournalSequence, JournalCodecError> {
    Ok(JournalSequence::new(positive(value, name)?))
}

pub(super) fn turn_id(value: u64) -> Result<TurnId, JournalCodecError> {
    NonZeroU64::new(value)
        .map(TurnId::new)
        .ok_or_else(|| JournalCodecError::new("turn_id must be positive"))
}

impl From<ExchangeKind> for WireExchangeKind {
    fn from(value: ExchangeKind) -> Self {
        match value {
            ExchangeKind::Request => Self::Request,
            ExchangeKind::Response => Self::Response,
            ExchangeKind::Notification => Self::Notification,
            ExchangeKind::ServerRequest => Self::ServerRequest,
            ExchangeKind::Retry => Self::Retry,
            ExchangeKind::TerminalOutcome => Self::TerminalOutcome,
        }
    }
}

impl From<WireExchangeKind> for ExchangeKind {
    fn from(value: WireExchangeKind) -> Self {
        match value {
            WireExchangeKind::Request => Self::Request,
            WireExchangeKind::Response => Self::Response,
            WireExchangeKind::Notification => Self::Notification,
            WireExchangeKind::ServerRequest => Self::ServerRequest,
            WireExchangeKind::Retry => Self::Retry,
            WireExchangeKind::TerminalOutcome => Self::TerminalOutcome,
        }
    }
}

impl From<ExchangeDirection> for WireExchangeDirection {
    fn from(value: ExchangeDirection) -> Self {
        match value {
            ExchangeDirection::YoToBackend => Self::YoToBackend,
            ExchangeDirection::BackendToYo => Self::BackendToYo,
        }
    }
}

impl From<WireExchangeDirection> for ExchangeDirection {
    fn from(value: WireExchangeDirection) -> Self {
        match value {
            WireExchangeDirection::YoToBackend => Self::YoToBackend,
            WireExchangeDirection::BackendToYo => Self::BackendToYo,
        }
    }
}

impl From<DetailAvailability> for WireDetailAvailability {
    fn from(value: DetailAvailability) -> Self {
        match value {
            DetailAvailability::Persisted => Self::Persisted,
            DetailAvailability::Volatile => Self::Volatile,
            DetailAvailability::Missing => Self::Missing,
            DetailAvailability::Unsupported => Self::Unsupported,
            DetailAvailability::Unpersisted => Self::Unpersisted,
            DetailAvailability::Redacted => Self::Redacted,
        }
    }
}

impl From<WireDetailAvailability> for DetailAvailability {
    fn from(value: WireDetailAvailability) -> Self {
        match value {
            WireDetailAvailability::Persisted => Self::Persisted,
            WireDetailAvailability::Volatile => Self::Volatile,
            WireDetailAvailability::Missing => Self::Missing,
            WireDetailAvailability::Unsupported => Self::Unsupported,
            WireDetailAvailability::Unpersisted => Self::Unpersisted,
            WireDetailAvailability::Redacted => Self::Redacted,
        }
    }
}

impl From<TransitionMode> for WireTransitionMode {
    fn from(value: TransitionMode) -> Self {
        match value {
            TransitionMode::Initial => Self::Initial,
            TransitionMode::ExactReplay => Self::ExactReplay,
            TransitionMode::LossyHandoff => Self::LossyHandoff,
        }
    }
}

impl From<WireTransitionMode> for TransitionMode {
    fn from(value: WireTransitionMode) -> Self {
        match value {
            WireTransitionMode::Initial => Self::Initial,
            WireTransitionMode::ExactReplay => Self::ExactReplay,
            WireTransitionMode::LossyHandoff => Self::LossyHandoff,
        }
    }
}

impl From<CacheState> for WireCacheState {
    fn from(value: CacheState) -> Self {
        match value {
            CacheState::NotApplicable => Self::NotApplicable,
            CacheState::Lost => Self::Lost,
            CacheState::Unknown => Self::Unknown,
        }
    }
}

impl From<WireCacheState> for CacheState {
    fn from(value: WireCacheState) -> Self {
        match value {
            WireCacheState::NotApplicable => Self::NotApplicable,
            WireCacheState::Lost => Self::Lost,
            WireCacheState::Unknown => Self::Unknown,
        }
    }
}

impl From<BindingCloseReason> for WireBindingCloseReason {
    fn from(value: BindingCloseReason) -> Self {
        match value {
            BindingCloseReason::Replaced => Self::Replaced,
            BindingCloseReason::Revoked => Self::Revoked,
            BindingCloseReason::Exhausted => Self::Exhausted,
        }
    }
}

impl From<WireBindingCloseReason> for BindingCloseReason {
    fn from(value: WireBindingCloseReason) -> Self {
        match value {
            WireBindingCloseReason::Replaced => Self::Replaced,
            WireBindingCloseReason::Revoked => Self::Revoked,
            WireBindingCloseReason::Exhausted => Self::Exhausted,
        }
    }
}
