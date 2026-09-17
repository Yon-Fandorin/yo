//! 저장된 Session history의 semantic projection 값입니다.

use std::sync::Arc;

use super::{
    discovery::StoredDiscoveryValidation,
    inherited::InheritedSessionHistory,
    request_trace::StoredRequestTraceEntry,
    session_usage::{SessionUsageError, SessionUsageProjection},
};
use crate::{
    JournalSequence, SessionDescriptor, TranscriptRecord,
    interview::{self, InterviewCatalog},
};

/// 저장된 Session 한 시점의 검증된 semantic projection입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSessionHistory {
    pub(super) descriptor: SessionDescriptor,
    pub(super) journal_cutoff: Option<JournalSequence>,
    pub(super) recovery: StoredSessionRecovery,
    pub(super) continuity: StoredSessionContinuity,
    pub(super) discovery_validation: StoredDiscoveryValidation,
    pub(super) records: Vec<TranscriptRecord>,
    pub(super) request_trace: Vec<StoredRequestTraceEntry>,
    pub(super) inherited_history: Option<Arc<InheritedSessionHistory>>,
    pub(super) accepted_initial: Option<(crate::SubmissionId, crate::TurnRef, JournalSequence)>,
}

impl StoredSessionHistory {
    #[must_use]
    pub fn accepted_initial_submission(
        &self,
        id: crate::SubmissionId,
    ) -> Option<(crate::TurnRef, JournalSequence)> {
        self.accepted_initial
            .filter(|(actual, _, _)| *actual == id)
            .map(|(_, turn, sequence)| (turn, sequence))
    }

    /// 상속 archival을 제외하고 durable question/answer provenance를 다시 검증합니다.
    #[must_use]
    pub fn interviews(&self) -> InterviewCatalog {
        InterviewCatalog::from_records(&self.records)
    }

    #[must_use]
    pub const fn descriptor(&self) -> &SessionDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn journal_cutoff(&self) -> Option<JournalSequence> {
        self.journal_cutoff
    }

    #[must_use]
    pub const fn recovery(&self) -> StoredSessionRecovery {
        self.recovery
    }

    /// physical history가 volatile suffix가 유실되지 않았음을 증명할 수 있는지 나타냅니다.
    #[must_use]
    pub const fn continuity(&self) -> StoredSessionContinuity {
        self.continuity
    }

    /// 모든 physical discovery summary를 semantic Journal 권위와 대조한 결과입니다.
    #[must_use]
    pub const fn discovery_validation(&self) -> StoredDiscoveryValidation {
        self.discovery_validation
    }

    #[must_use]
    pub const fn discovery_consistent(&self) -> bool {
        matches!(
            self.discovery_validation,
            StoredDiscoveryValidation::Consistent
        )
    }

    /// 복구된 frontend-independent record를 durable 순서로 반환합니다.
    #[must_use]
    pub fn records(&self) -> &[TranscriptRecord] {
        &self.records
    }

    /// 현재 Session record와 usage에서 제외된 source-qualified archival history입니다.
    #[must_use]
    pub fn inherited_history(&self) -> Option<&InheritedSessionHistory> {
        self.inherited_history.as_deref()
    }

    /// payload가 없는 모든 Request correlation 사실을 durable Journal 순서로 반환합니다.
    #[must_use]
    pub fn request_trace(&self) -> &[StoredRequestTraceEntry] {
        &self.request_trace
    }

    pub fn session_usage(&self) -> Result<SessionUsageProjection, SessionUsageError> {
        SessionUsageProjection::from_records(&self.records)
    }
}

/// archival recovery가 durable terminal이 없는 message를 닫아야 했는지 나타냅니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredSessionRecovery {
    NotRequired,
    Interrupted,
}

/// 저장된 physical history가 process-local semantic suffix에 대해 증명할 수 있는 내용입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredSessionContinuity {
    /// 현재 v1 형식으로는 중단된 writer가 volatile record를 잃었는지 증명할 수 없습니다.
    NotObservable,
}
