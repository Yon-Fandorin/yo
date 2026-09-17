use std::sync::Arc;

use super::recovery::{StoredSessionContinuation, StoredSessionContinuationError};
use crate::{
    JournalDurability, JournalSequence, SessionDescriptor, SessionId,
    journal::codec::RecoveredJournal, session_repository::RepositoryError,
};

/// 과거 포크를 위한 물리 캡처 예산과 독립적인 표시 상한.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionForkLimits {
    physical_bytes: u64,
    physical_records: usize,
    returned_boundaries: usize,
}

impl SessionForkLimits {
    /// 256MiB, 65536개 엔벌로프, 반환 지점 256개 이하의 양수 상한을 허용합니다.
    pub fn try_new(
        physical_bytes: u64,
        physical_records: usize,
        returned_boundaries: usize,
    ) -> Result<Self, RepositoryError> {
        super::validation::validate_limits(physical_bytes, physical_records, returned_boundaries)?;
        Ok(Self {
            physical_bytes,
            physical_records,
            returned_boundaries,
        })
    }

    /// 커밋되지 않은 후행 바이트를 포함한 최대 캡처 물리 바이트입니다.
    pub const fn physical_bytes(self) -> u64 {
        self.physical_bytes
    }
    /// 대체된 스냅샷을 포함한 완전한 물리 엔벌로프의 최대 개수입니다.
    pub const fn physical_records(self) -> usize {
        self.physical_records
    }
    /// 가장 최신의 검증된 경계 행 최대 개수이며 검증 컷오프가 아닙니다.
    pub const fn returned_boundaries(self) -> usize {
        self.returned_boundaries
    }
}

impl Default for SessionForkLimits {
    fn default() -> Self {
        Self {
            physical_bytes: 32 * 1024 * 1024,
            physical_records: 4096,
            returned_boundaries: 128,
        }
    }
}

/// 과거 경계 행이 나타내는 재구성입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredSessionForkSourceKind {
    /// 완료된 결과와 정확한 연속성 앵커 및 재생 체인입니다.
    Anchor,
    /// 후속 컨텍스트 에포크에서 정지된 체크포인트를 재구성합니다.
    Checkpoint,
    /// 이후의 정확한 바인딩 소유자 아래에 있을 수 있는 완전한 초기 자식 부트스트랩입니다.
    InitialFork,
}

/// 불투명한 선택과 동일한 고정 캡처에서 파생된 표시 메타데이터입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSessionForkBoundary {
    pub(super) source_kind: StoredSessionForkSourceKind,
    pub(super) journal_cutoff: JournalSequence,
    pub(super) logical_cutoff: JournalSequence,
    pub(super) binding_epoch: u64,
    pub(super) context_epoch: u64,
    pub(super) model_label: String,
    pub(super) input_excerpt: Option<String>,
    pub(super) record_count: usize,
}

impl StoredSessionForkBoundary {
    /// 이 행이 나타내는 정확한 재구성의 종류입니다.
    pub const fn source_kind(&self) -> StoredSessionForkSourceKind {
        self.source_kind
    }
    /// 앵커 또는 완전한 부트스트랩을 포함하는 복구 레코드 컷오프입니다.
    pub const fn journal_cutoff(&self) -> JournalSequence {
        self.journal_cutoff
    }
    /// 마지막으로 상속된 논리 이벤트이며 앵커의 결과는 자체 레코드 컷오프보다 앞섭니다.
    pub const fn logical_cutoff(&self) -> JournalSequence {
        self.logical_cutoff
    }
    /// 선택한 복구 컷오프에서 유효한 부모 바인딩 에포크입니다.
    pub const fn binding_epoch(&self) -> u64 {
        self.binding_epoch
    }
    /// 선택한 복구 컷오프에서 유효한 부모 컨텍스트 에포크입니다.
    pub const fn context_epoch(&self) -> u64 {
        self.context_epoch
    }
    /// 표시를 위한 제한된 기록 모델 식별자이며 모델 확인의 권한이 아닙니다.
    pub fn model_label(&self) -> &str {
        &self.model_label
    }
    /// 제어 문자를 공백으로 바꾼 최대 120개의 표시 입력 스칼라(UTF-8 480바이트)입니다.
    /// 확장 모델 텍스트, 지침, 참조 내용, 상속 입력은 포함하지 않습니다.
    pub fn input_excerpt(&self) -> Option<&str> {
        self.input_excerpt.as_deref()
    }
}

#[derive(Debug)]
pub(super) struct ForkCapture {
    pub(super) recovered: RecoveredJournal,
    pub(super) durability: JournalDurability,
}

/// 별도의 상한을 적용한 최신 우선 카탈로그를 포함하는 완전한 검증 물리 캡처입니다.
#[derive(Clone, Debug)]
pub struct StoredSessionForkCatalog {
    pub(super) capture: Arc<ForkCapture>,
    pub(super) boundaries: Vec<StoredSessionForkBoundary>,
    pub(super) truncated: bool,
}

impl StoredSessionForkCatalog {
    /// 전체 캡처 검증 후 독립적으로 상한을 적용한 최신 우선 검증 행입니다.
    pub fn boundaries(&self) -> &[StoredSessionForkBoundary] {
        &self.boundaries
    }
    /// 전체 검증에 성공하고 오래된 표시 행을 생략한 경우에만 참입니다.
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
    /// 한 행을 이 정확한 캡처에 결합하며 호출자는 원시 시퀀스 선택을 만들 수 없습니다.
    pub fn selection(
        &self,
        index: usize,
    ) -> Result<StoredSessionForkSelection, StoredSessionContinuationError> {
        let boundary = self.boundaries.get(index).cloned().ok_or_else(|| {
            StoredSessionContinuationError::new(
                "historical fork selection is outside its captured catalog",
            )
        })?;
        Ok(StoredSessionForkSelection {
            capture: Arc::clone(&self.capture),
            boundary,
        })
    }

    pub(crate) fn durability(&self) -> JournalDurability {
        self.capture.durability
    }
}

/// 전체 캡처와 과거의 유효 소유권에 결합된 불변 선택입니다.
#[derive(Clone, Debug)]
pub struct StoredSessionForkSelection {
    pub(super) capture: Arc<ForkCapture>,
    boundary: StoredSessionForkBoundary,
}

impl StoredSessionForkSelection {
    /// 이 불변 선택의 표시 메타데이터이며 캡처를 대신하지 않습니다.
    pub fn boundary(&self) -> &StoredSessionForkBoundary {
        &self.boundary
    }

    pub(crate) fn prepare_source(
        &self,
        session_id: SessionId,
        durability: JournalDurability,
    ) -> Result<StoredSessionContinuation, StoredSessionContinuationError> {
        let capture_session_matches = self
            .capture
            .recovered
            .descriptor()
            .map(SessionDescriptor::session_id)
            == Some(session_id);
        if !super::validation::selection_identity_matches(
            self.capture.durability == durability,
            capture_session_matches,
        ) {
            return Err(StoredSessionContinuationError::new(
                "historical fork selection is stale or belongs to another Session",
            ));
        }
        let prefix = self
            .capture
            .recovered
            .historical_fork_prefix(self.boundary.record_count, self.boundary.journal_cutoff)
            .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
        if !super::validation::boundary_epochs_match(
            prefix.binding_epoch(),
            prefix.context_epoch(),
            self.boundary.binding_epoch,
            self.boundary.context_epoch,
        ) {
            return Err(StoredSessionContinuationError::new(
                "historical fork ownership differs from its captured boundary",
            ));
        }
        super::recovery::build_continuation(prefix, session_id)
    }
}
