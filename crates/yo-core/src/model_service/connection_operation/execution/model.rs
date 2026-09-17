use std::{error::Error, fmt, path::PathBuf};

use super::super::{
    ConnectionCredentialAction, ConnectionOperationError, ConnectionOperationJournalEntry,
    ConnectionOperationKind, ConnectionOperationPhase, ExternalConnectionError,
    ExternalDisconnectError,
};
use crate::model_service::{ConnectionRepositoryError, LocalCredentialStoreError};

/// 로컬 복구가 실패한 저장소 역할입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionOperationRepositoryKind {
    Public,
    Credential,
    Journal,
}

/// 하나의 보류 중인 작업을 확인하고 필요한 경우 완료한 결과입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionOperationExecutionOutcome {
    NoPendingOperation,
    Abandoned {
        kind: ConnectionOperationKind,
        action: ConnectionCredentialAction,
    },
    Completed {
        kind: ConnectionOperationKind,
        action: ConnectionCredentialAction,
        recovered_from: ConnectionOperationPhase,
    },
}

/// private credential revision을 진단 정보에 포함하지 않는 typed 복구 실패입니다.
#[derive(Debug)]
pub enum ConnectionOperationExecutionError {
    InvalidRepositoryLayout {
        repository: ConnectionOperationRepositoryKind,
        path: PathBuf,
    },
    OperationLock(ConnectionRepositoryError),
    JournalCapture(ConnectionOperationError),
    ExternalPreparation(ExternalConnectionError),
    ExternalDisconnectPreparation(ExternalDisconnectError),
    PublicCapture(ConnectionRepositoryError),
    PublicPreparation(ConnectionRepositoryError),
    PublicCommit(ConnectionRepositoryError),
    CredentialCapture(LocalCredentialStoreError),
    PublicRepository {
        kind: ConnectionOperationKind,
        action: ConnectionCredentialAction,
        phase: ConnectionOperationPhase,
        source: ConnectionRepositoryError,
    },
    CredentialRepository {
        kind: ConnectionOperationKind,
        action: ConnectionCredentialAction,
        phase: ConnectionOperationPhase,
        source: LocalCredentialStoreError,
    },
    Journal {
        kind: ConnectionOperationKind,
        action: ConnectionCredentialAction,
        phase: ConnectionOperationPhase,
        source: ConnectionOperationError,
    },
    #[cfg(test)]
    InjectedInterruption,
}

impl fmt::Display for ConnectionOperationExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRepositoryLayout { repository, path } => write!(
                formatter,
                "{repository:?} connection-operation repository has an incompatible local path: {}",
                path.display()
            ),
            Self::OperationLock(source) => {
                write!(
                    formatter,
                    "acquiring the connection-operation lock failed: {source}"
                )
            },
            Self::JournalCapture(source) => {
                write!(
                    formatter,
                    "capturing the pending connection operation failed: {source}"
                )
            },
            Self::ExternalPreparation(source) => write!(formatter, "{source}"),
            Self::ExternalDisconnectPreparation(source) => write!(formatter, "{source}"),
            Self::PublicCapture(source) => {
                write!(
                    formatter,
                    "capturing public connection state failed: {source}"
                )
            },
            Self::PublicPreparation(source) => {
                write!(
                    formatter,
                    "preparing public connection state failed: {source}"
                )
            },
            Self::PublicCommit(source) => {
                write!(
                    formatter,
                    "committing public connection state failed: {source}"
                )
            },
            Self::CredentialCapture(source) => {
                write!(
                    formatter,
                    "capturing private credential state failed: {source}"
                )
            },
            Self::PublicRepository {
                kind,
                action,
                phase,
                source,
            } => write!(
                formatter,
                "{kind:?} recovery at {phase:?} with {action:?} failed in the public repository: {source}"
            ),
            Self::CredentialRepository {
                kind,
                action,
                phase,
                source,
            } => write!(
                formatter,
                "{kind:?} recovery at {phase:?} with {action:?} failed in the credential repository: {source}"
            ),
            Self::Journal {
                kind,
                action,
                phase,
                source,
            } => write!(
                formatter,
                "{kind:?} recovery at {phase:?} with {action:?} failed in the operation journal: {source}"
            ),
            #[cfg(test)]
            Self::InjectedInterruption => {
                formatter.write_str("connection recovery interrupted at an injected test boundary")
            },
        }
    }
}

impl Error for ConnectionOperationExecutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidRepositoryLayout { .. } => None,
            Self::OperationLock(source) | Self::PublicRepository { source, .. } => Some(source),
            Self::JournalCapture(source) | Self::Journal { source, .. } => Some(source),
            Self::ExternalPreparation(source) => Some(source),
            Self::ExternalDisconnectPreparation(source) => Some(source),
            Self::PublicCapture(source)
            | Self::PublicPreparation(source)
            | Self::PublicCommit(source) => Some(source),
            Self::CredentialCapture(source) => Some(source),
            Self::CredentialRepository { source, .. } => Some(source),
            #[cfg(test)]
            Self::InjectedInterruption => None,
        }
    }
}

pub(in super::super) fn public_error(
    entry: &ConnectionOperationJournalEntry,
    source: ConnectionRepositoryError,
) -> ConnectionOperationExecutionError {
    ConnectionOperationExecutionError::PublicRepository {
        kind: entry.kind(),
        action: entry.credential_action(),
        phase: entry.phase(),
        source,
    }
}

pub(in super::super) fn credential_error(
    entry: &ConnectionOperationJournalEntry,
    source: LocalCredentialStoreError,
) -> ConnectionOperationExecutionError {
    ConnectionOperationExecutionError::CredentialRepository {
        kind: entry.kind(),
        action: entry.credential_action(),
        phase: entry.phase(),
        source,
    }
}

pub(in super::super) fn journal_error(
    entry: &ConnectionOperationJournalEntry,
    source: ConnectionOperationError,
) -> ConnectionOperationExecutionError {
    ConnectionOperationExecutionError::Journal {
        kind: entry.kind(),
        action: entry.credential_action(),
        phase: entry.phase(),
        source,
    }
}
