use crate::{JournalSequence, SessionDescriptor};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RepositorySequence(u64);

impl RepositorySequence {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableRecordKind {
    Incremental,
    Snapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableRecord {
    kind: DurableRecordKind,
    payload: String,
    journal_cutoff: Option<JournalSequence>,
    discovery: Option<RecordDiscovery>,
}

impl DurableRecord {
    pub fn incremental(payload: impl Into<String>) -> Self {
        Self {
            kind: DurableRecordKind::Incremental,
            payload: payload.into(),
            journal_cutoff: None,
            discovery: None,
        }
    }

    pub fn snapshot(payload: impl Into<String>) -> Self {
        Self {
            kind: DurableRecordKind::Snapshot,
            payload: payload.into(),
            journal_cutoff: None,
            discovery: None,
        }
    }

    pub const fn kind(&self) -> DurableRecordKind {
        self.kind
    }

    pub fn payload(&self) -> &str {
        &self.payload
    }

    pub(crate) const fn with_journal_cutoff(
        mut self,
        journal_cutoff: Option<JournalSequence>,
    ) -> Self {
        self.journal_cutoff = journal_cutoff;
        self
    }

    pub(crate) const fn journal_cutoff(&self) -> Option<JournalSequence> {
        self.journal_cutoff
    }

    pub fn with_discovery(mut self, discovery: RecordDiscovery) -> Self {
        self.discovery = Some(discovery);
        self
    }

    pub const fn discovery(&self) -> Option<&RecordDiscovery> {
        self.discovery.as_ref()
    }
}

/// Semantic inputs from which the physical writer builds one discovery summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordDiscovery {
    descriptor: SessionDescriptor,
    binding_epoch: Option<u64>,
    continuation_anchor: Option<JournalSequence>,
}

impl RecordDiscovery {
    pub const fn new(descriptor: SessionDescriptor) -> Self {
        Self {
            descriptor,
            binding_epoch: None,
            continuation_anchor: None,
        }
    }

    pub const fn descriptor(&self) -> &SessionDescriptor {
        &self.descriptor
    }

    pub const fn binding_epoch(&self) -> Option<u64> {
        self.binding_epoch
    }

    pub const fn continuation_anchor(&self) -> Option<JournalSequence> {
        self.continuation_anchor
    }

    pub const fn with_binding_epoch(mut self, binding_epoch: u64) -> Self {
        self.binding_epoch = Some(binding_epoch);
        self
    }

    pub const fn with_continuation_anchor(mut self, continuation_anchor: JournalSequence) -> Self {
        self.continuation_anchor = Some(continuation_anchor);
        self
    }
}

/// Storage-neutral metadata obtained from one validated physical envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionDiscovery {
    descriptor: SessionDescriptor,
    updated_unix_millis: u64,
    binding_epoch: Option<u64>,
    continuation_anchor: Option<JournalSequence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSessionSummary {
    repository_sequence: RepositorySequence,
    record_version: SessionRecordVersion,
    discovery: SessionDiscovery,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoredSession {
    Available(StoredSessionSummary),
    Unavailable {
        session_id: crate::SessionId,
        reason: StoredSessionUnavailableReason,
    },
}

/// Typed reason why bounded discovery could not produce a trusted summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoredSessionUnavailableReason {
    NoCompleteEnvelope,
    Quarantined { message: String },
    UnsupportedSchema { schema: String },
    Corrupt { message: String },
    Unreadable { message: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuationEligibility {
    Eligible,
    Unavailable,
    Unknown,
}

impl StoredSession {
    pub const fn session_id(&self) -> crate::SessionId {
        match self {
            Self::Available(summary) => summary.discovery().descriptor().session_id(),
            Self::Unavailable { session_id, .. } => *session_id,
        }
    }

    pub const fn summary(&self) -> Option<&StoredSessionSummary> {
        match self {
            Self::Available(summary) => Some(summary),
            Self::Unavailable { .. } => None,
        }
    }

    pub const fn unavailable_reason(&self) -> Option<&StoredSessionUnavailableReason> {
        match self {
            Self::Available(_) => None,
            Self::Unavailable { reason, .. } => Some(reason),
        }
    }

    pub const fn continuation_eligibility(&self) -> ContinuationEligibility {
        match self {
            Self::Available(summary) => {
                if summary.discovery().continuation_anchor().is_some() {
                    ContinuationEligibility::Eligible
                } else {
                    // The physical v1 discovery shape intentionally remains unchanged and cannot
                    // distinguish an incomplete legacy history from a checkpoint-only replay
                    // root. Full semantic recovery owns that decision.
                    ContinuationEligibility::Unknown
                }
            },
            Self::Unavailable {
                reason: StoredSessionUnavailableReason::UnsupportedSchema { .. },
                ..
            } => ContinuationEligibility::Unknown,
            Self::Unavailable { .. } => ContinuationEligibility::Unavailable,
        }
    }
}

impl std::fmt::Display for StoredSessionUnavailableReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCompleteEnvelope => {
                formatter.write_str("Session log has no complete durable envelope")
            },
            Self::Quarantined { message }
            | Self::Corrupt { message }
            | Self::Unreadable { message } => formatter.write_str(message),
            Self::UnsupportedSchema { schema } => {
                write!(formatter, "unsupported Session record schema {schema:?}")
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionRecordVersion {
    V1,
}

impl StoredSessionSummary {
    pub(crate) const fn new(
        repository_sequence: RepositorySequence,
        record_version: SessionRecordVersion,
        discovery: SessionDiscovery,
    ) -> Self {
        Self {
            repository_sequence,
            record_version,
            discovery,
        }
    }

    pub const fn repository_sequence(&self) -> RepositorySequence {
        self.repository_sequence
    }

    pub const fn discovery(&self) -> &SessionDiscovery {
        &self.discovery
    }

    pub const fn record_version(&self) -> SessionRecordVersion {
        self.record_version
    }
}

impl SessionDiscovery {
    pub(crate) const fn new(
        descriptor: SessionDescriptor,
        updated_unix_millis: u64,
        binding_epoch: Option<u64>,
        continuation_anchor: Option<JournalSequence>,
    ) -> Self {
        Self {
            descriptor,
            updated_unix_millis,
            binding_epoch,
            continuation_anchor,
        }
    }

    pub const fn descriptor(&self) -> &SessionDescriptor {
        &self.descriptor
    }

    pub const fn updated_unix_millis(&self) -> u64 {
        self.updated_unix_millis
    }

    pub const fn binding_epoch(&self) -> Option<u64> {
        self.binding_epoch
    }

    pub const fn continuation_anchor(&self) -> Option<JournalSequence> {
        self.continuation_anchor
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryEntry {
    sequence: RepositorySequence,
    record: DurableRecord,
}

impl RepositoryEntry {
    pub const fn new(sequence: RepositorySequence, record: DurableRecord) -> Self {
        Self { sequence, record }
    }

    pub const fn sequence(&self) -> RepositorySequence {
        self.sequence
    }

    pub const fn record(&self) -> &DurableRecord {
        &self.record
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppendReceipt {
    sequence: RepositorySequence,
}

impl AppendReceipt {
    pub const fn new(sequence: RepositorySequence) -> Self {
        Self { sequence }
    }

    pub const fn sequence(self) -> RepositorySequence {
        self.sequence
    }
}
