use std::{error::Error, fmt};

/// 세션 비밀값을 노출하지 않고 QwenCloud 계정 용량 실패를 분류합니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QwenCloudCapacityFailureKind {
    Configuration,
    Transport,
    HttpStatus,
    MediaType,
    Limit,
    Protocol,
    Timeout,
    ExpiredSession,
}

/// QwenCloud 계정 용량 프로토콜이 반환하는 provider 소유의 안전한 오류입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QwenCloudCapacityError {
    kind: QwenCloudCapacityFailureKind,
    message: String,
}

impl QwenCloudCapacityError {
    fn new(kind: QwenCloudCapacityFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> QwenCloudCapacityFailureKind {
        self.kind
    }

    #[must_use]
    pub const fn is_expired_session(&self) -> bool {
        matches!(self.kind, QwenCloudCapacityFailureKind::ExpiredSession)
    }
}

impl fmt::Display for QwenCloudCapacityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for QwenCloudCapacityError {}

pub(super) type QwenCloudResult<T> = Result<T, QwenCloudCapacityError>;

pub(super) fn expired_session_error() -> QwenCloudCapacityError {
    failure(
        QwenCloudCapacityFailureKind::ExpiredSession,
        "QwenCloud console session expired; enter a new browser Cookie. The stored model connection and API key are unchanged",
    )
}

pub(super) fn failure(
    kind: QwenCloudCapacityFailureKind,
    message: impl Into<String>,
) -> QwenCloudCapacityError {
    QwenCloudCapacityError::new(kind, message)
}
