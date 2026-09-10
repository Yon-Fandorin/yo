use std::{collections::HashSet, fmt, sync::Arc};

use super::{
    RepositoryError, SessionWriterRepository, StoredDiscoveryValidation, StoredSessionReader,
    StoredSessionSnapshot,
    history::{
        InheritedSessionHistory, normalize_recovered, project_inherited, validate_discovery,
    },
    journal::{recover_entries, recover_repository},
};
use crate::{
    AgentCommand, BackendBindingEvidence, BackendIdentity, BackendResumeSource,
    BackendResumeTarget, ContinuationStrategy, InputImageHistory, JournalDurability,
    JournalSequence, ModelReplay, SessionDescriptor, SessionId, SubmissionId,
    journal::{
        JournalEntry,
        codec::{ForkSource, HistoricalForkKind, JournalRecord, RecoveredJournal},
    },
};

/// Physical capture budgets and an independent presentation bound for historical forks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionForkLimits {
    physical_bytes: u64,
    physical_records: usize,
    returned_boundaries: usize,
}

impl SessionForkLimits {
    /// Accepts positive bounds up to 256 MiB, 65536 envelopes, and 256 returned points.
    pub fn try_new(
        physical_bytes: u64,
        physical_records: usize,
        returned_boundaries: usize,
    ) -> Result<Self, RepositoryError> {
        if !(1..=256 * 1024 * 1024).contains(&physical_bytes)
            || !(1..=65_536).contains(&physical_records)
            || !(1..=256).contains(&returned_boundaries)
        {
            return Err(RepositoryError::Unavailable {
                message: "historical fork limits require 1–268435456 bytes, 1–65536 physical records, and 1–256 returned boundaries".to_owned(),
            });
        }
        Ok(Self {
            physical_bytes,
            physical_records,
            returned_boundaries,
        })
    }

    /// Maximum captured physical bytes, including uncommitted trailing bytes.
    pub const fn physical_bytes(self) -> u64 {
        self.physical_bytes
    }
    /// Maximum complete physical envelopes, including superseded snapshots.
    pub const fn physical_records(self) -> usize {
        self.physical_records
    }
    /// Maximum newest verified boundary rows; never a validation cutoff.
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

/// Reconstruction represented by a historical boundary row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredSessionForkSourceKind {
    /// Completed outcome plus its exact Continuation Anchor and replay chain.
    Anchor,
    /// Quiescent checkpoint reconstruction in its successor context epoch.
    Checkpoint,
    /// Complete initial child bootstrap, possibly under a later exact binding owner.
    InitialFork,
}

/// Display metadata derived from the same frozen capture as its opaque selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSessionForkBoundary {
    source_kind: StoredSessionForkSourceKind,
    journal_cutoff: JournalSequence,
    logical_cutoff: JournalSequence,
    binding_epoch: u64,
    context_epoch: u64,
    model_label: String,
    input_excerpt: Option<String>,
    record_count: usize,
}

impl StoredSessionForkBoundary {
    /// Kind of the exact reconstruction represented by this row.
    pub const fn source_kind(&self) -> StoredSessionForkSourceKind {
        self.source_kind
    }
    /// Recovery includes this record cutoff, including the Anchor or complete bootstrap.
    pub const fn journal_cutoff(&self) -> JournalSequence {
        self.journal_cutoff
    }
    /// Last inherited logical event; an Anchor's outcome precedes its own record cutoff.
    pub const fn logical_cutoff(&self) -> JournalSequence {
        self.logical_cutoff
    }
    /// Effective parent binding epoch at the selected recovery cutoff.
    pub const fn binding_epoch(&self) -> u64 {
        self.binding_epoch
    }
    /// Effective parent context epoch at the selected recovery cutoff.
    pub const fn context_epoch(&self) -> u64 {
        self.context_epoch
    }
    /// Bounded recorded model identity for display; never model-resolution authority.
    pub fn model_label(&self) -> &str {
        &self.model_label
    }
    /// At most 120 visible input scalars (480 UTF-8 bytes), with controls replaced by spaces.
    /// Expanded model text, instructions, reference contents and inherited input are never
    /// included.
    pub fn input_excerpt(&self) -> Option<&str> {
        self.input_excerpt.as_deref()
    }
}

#[derive(Debug)]
struct ForkCapture {
    recovered: RecoveredJournal,
    durability: JournalDurability,
}

/// A complete validated physical capture with a separately capped newest-first catalog.
#[derive(Clone, Debug)]
pub struct StoredSessionForkCatalog {
    capture: Arc<ForkCapture>,
    boundaries: Vec<StoredSessionForkBoundary>,
    truncated: bool,
}

impl StoredSessionForkCatalog {
    /// Verified rows in newest-first order, independently bounded after full capture validation.
    pub fn boundaries(&self) -> &[StoredSessionForkBoundary] {
        &self.boundaries
    }
    /// True only when full validation succeeded and older display rows were omitted.
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
    /// Binds one row to this exact capture; callers cannot construct raw-sequence selections.
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

/// An immutable selection bound to the full capture and historical effective ownership.
#[derive(Clone, Debug)]
pub struct StoredSessionForkSelection {
    capture: Arc<ForkCapture>,
    boundary: StoredSessionForkBoundary,
}

impl StoredSessionForkSelection {
    /// Display metadata for this immutable selection; never a substitute for its capture.
    pub fn boundary(&self) -> &StoredSessionForkBoundary {
        &self.boundary
    }

    pub(crate) fn prepare_source(
        &self,
        session_id: SessionId,
        durability: JournalDurability,
    ) -> Result<StoredSessionContinuation, StoredSessionContinuationError> {
        if self.capture.durability != durability
            || self
                .capture
                .recovered
                .descriptor()
                .map(SessionDescriptor::session_id)
                != Some(session_id)
        {
            return Err(StoredSessionContinuationError::new(
                "historical fork selection is stale or belongs to another Session",
            ));
        }
        let prefix = self
            .capture
            .recovered
            .historical_fork_prefix(self.boundary.record_count, self.boundary.journal_cutoff)
            .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
        if prefix.binding_epoch() != Some(self.boundary.binding_epoch)
            || prefix.context_epoch() != Some(self.boundary.context_epoch)
        {
            return Err(StoredSessionContinuationError::new(
                "historical fork ownership differs from its captured boundary",
            ));
        }
        build_continuation(prefix, session_id)
    }
}

pub(crate) fn read_fork_catalog(
    reader: &(impl StoredSessionReader + ?Sized),
    session_id: SessionId,
    limits: SessionForkLimits,
) -> Result<StoredSessionForkCatalog, StoredSessionContinuationError> {
    let entries = match reader
        .read_session_bounded(session_id, limits)
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?
    {
        StoredSessionSnapshot::Present(entries) if !entries.is_empty() => entries,
        _ => {
            return Err(StoredSessionContinuationError::new(
                "historical fork capture has no complete Session envelopes",
            ));
        },
    };
    let recovered = recover_entries(session_id, &entries)
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
    let descriptor = recovered.descriptor().ok_or_else(|| {
        StoredSessionContinuationError::new("historical fork capture has no descriptor")
    })?;
    if descriptor.session_id() != session_id
        || validate_discovery(&entries, descriptor, &recovered)
            != StoredDiscoveryValidation::Consistent
    {
        return Err(StoredSessionContinuationError::new(
            "historical fork capture has inconsistent physical discovery metadata",
        ));
    }
    // Validate the latest executable source too: an older valid prefix cannot bypass uncertainty.
    recovered
        .validate_fork_capture()
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
    let (points, truncated) = recovered
        .historical_fork_boundaries(limits.returned_boundaries())
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
    let boundaries = points
        .into_iter()
        .map(|point| StoredSessionForkBoundary {
            source_kind: match point.source {
                HistoricalForkKind::Anchor => StoredSessionForkSourceKind::Anchor,
                HistoricalForkKind::Checkpoint => StoredSessionForkSourceKind::Checkpoint,
                HistoricalForkKind::InitialFork => StoredSessionForkSourceKind::InitialFork,
            },
            journal_cutoff: point.cutoff,
            logical_cutoff: point.logical_cutoff,
            binding_epoch: point.binding_epoch,
            context_epoch: point.context_epoch,
            model_label: point.model_label,
            input_excerpt: point.input_excerpt,
            record_count: point.record_count,
        })
        .collect();
    let durability = JournalDurability::Durable {
        journal_sequence: recovered.journal_cutoff(),
        repository_sequence: entries.last().expect("nonempty capture").sequence(),
    };
    Ok(StoredSessionForkCatalog {
        capture: Arc::new(ForkCapture {
            recovered,
            durability,
        }),
        boundaries,
        truncated,
    })
}

/// Fully validated durable state required to reopen one executable Session.
#[derive(Clone, Debug)]
pub struct StoredSessionContinuation {
    recovered: RecoveredJournal,
    target: BackendResumeTarget,
    next_turn_id: u64,
    transcript_records: Vec<crate::TranscriptRecord>,
    inherited_history: Option<Arc<InheritedSessionHistory>>,
}

impl StoredSessionContinuation {
    /// Prepares an independent exact-replay child without storage or backend side effects.
    ///
    /// The candidate binding must satisfy the captured source identity and replay profile.
    /// The returned plan becomes durable and executable only after successful Session startup
    /// and publication; preparing it does not select the child or mutate its parent.
    pub fn prepare_exact_fork(
        &self,
        child: SessionDescriptor,
        candidate_binding: BackendBindingEvidence,
    ) -> Result<Self, StoredSessionContinuationError> {
        use crate::{
            AgentEvent, ContextPolicyChanged,
            journal::codec::{
                BackendBindingOpened, BindingTransition, JournalCommit, ReplaySequence,
                SequencedJournalRecord, VersionedIdentity, recover,
            },
        };

        let child_id = child.session_id();
        let seed = self
            .recovered
            .capture_fork(child_id)
            .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
        let identity =
            |value: &BackendIdentity| VersionedIdentity::new(value.schema(), value.value());
        let mut records = vec![
            JournalRecord::EventCommitted(AgentEvent::SessionCreated {
                session_id: child_id,
            }),
            JournalRecord::InitialForkSeed(Box::new(seed)),
            JournalRecord::BackendBindingOpened(BackendBindingOpened::new(
                1,
                candidate_binding.backend_kind(),
                candidate_binding.backend_version(),
                identity(candidate_binding.binding_identity()),
                identity(candidate_binding.model_identity()),
                identity(candidate_binding.session_locator()),
                BindingTransition::initial_fork(JournalSequence::new(2)),
                candidate_binding.continuation_strategy(),
            )),
        ];
        if let Some(policy) = self.recovered.context_policy() {
            let policy = ContextPolicyChanged::try_new(
                1,
                policy.enabled(),
                policy.strategy(),
                policy.warning_percent(),
                policy.trigger_percent(),
                policy.retained_raw_percent(),
                policy.retained_raw_max_tokens(),
            )
            .map_err(StoredSessionContinuationError::new)?;
            records.push(JournalRecord::ContextPolicyChanged(policy));
        }
        let cutoff =
            JournalSequence::new(u64::try_from(records.len()).expect("bounded fork bootstrap"));
        let mut entries = vec![SequencedJournalRecord::storage(
            ReplaySequence::new(1),
            JournalRecord::SessionDescriptor(child),
        )];
        entries.extend(records.into_iter().enumerate().map(|(index, record)| {
            let sequence = u64::try_from(index).expect("bounded fork bootstrap") + 1;
            SequencedJournalRecord::with_journal_sequence(
                ReplaySequence::new(sequence + 1),
                JournalSequence::new(sequence),
                record,
            )
        }));
        let snapshot = JournalCommit::snapshot_through(cutoff, entries);
        let recovered = recover(&[snapshot])
            .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
        build_continuation(recovered, child_id)
    }

    #[must_use]
    pub const fn descriptor(&self) -> &SessionDescriptor {
        self.recovered
            .descriptor()
            .expect("a continuation always has a descriptor")
    }

    #[must_use]
    pub const fn target(&self) -> &BackendResumeTarget {
        &self.target
    }

    /// Source-qualified archive, excluded from child execution and usage.
    #[must_use]
    pub fn inherited_history(&self) -> Option<&InheritedSessionHistory> {
        self.inherited_history.as_deref()
    }

    pub(crate) fn semantic_entries(&self) -> Vec<JournalEntry> {
        self.recovered.semantic_entries()
    }

    pub(crate) fn snapshot(&self) -> crate::journal::codec::JournalCommit {
        self.recovered.complete_snapshot()
    }

    pub(crate) fn journal_cutoff(&self) -> Option<JournalSequence> {
        self.recovered.journal_cutoff()
    }

    pub(crate) fn submission_ids(&self) -> HashSet<SubmissionId> {
        self.recovered.submission_ids().iter().copied().collect()
    }

    pub(crate) const fn next_turn_id(&self) -> u64 {
        self.next_turn_id
    }

    /// Read-only frontend history recovered and validated from this Session's journal.
    /// Hosts may preserve admitted attachment evidence from these records without
    /// reopening source files or treating replay-only images as editable input.
    pub fn transcript_records(&self) -> &[crate::TranscriptRecord] {
        &self.transcript_records
    }
}

/// Why a durable Session cannot be admitted for executable native resume.
#[derive(Debug)]
pub struct StoredSessionContinuationError {
    detail: String,
}

impl StoredSessionContinuationError {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for StoredSessionContinuationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for StoredSessionContinuationError {}

/// Acquires one Session writer lease, then revalidates its executable continuation.
pub fn recover_stored_session_continuation(
    repository: &mut (impl SessionWriterRepository + ?Sized),
    session_id: SessionId,
) -> Result<StoredSessionContinuation, StoredSessionContinuationError> {
    repository
        .acquire_session_writer(session_id)
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
    let recovered = recover_repository(repository, session_id)
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
    build_continuation(recovered, session_id)
}

/// Validates executable continuation without acquiring a writer lease or mutating storage.
pub fn read_stored_session_continuation(
    reader: &(impl StoredSessionReader + ?Sized),
    session_id: SessionId,
) -> Result<StoredSessionContinuation, StoredSessionContinuationError> {
    let entries = match reader
        .read_session(session_id)
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?
    {
        StoredSessionSnapshot::Missing => {
            return Err(StoredSessionContinuationError::new(format!(
                "stored Session {session_id} was not found"
            )));
        },
        StoredSessionSnapshot::Present(entries) if entries.is_empty() => {
            return Err(StoredSessionContinuationError::new(format!(
                "stored Session {session_id} has no complete envelope"
            )));
        },
        StoredSessionSnapshot::Present(entries) => entries,
    };
    let recovered = recover_entries(session_id, &entries)
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
    build_continuation(recovered, session_id)
}

fn recovered_input_image_history(recovered: &RecoveredJournal) -> InputImageHistory {
    let mut history = InputImageHistory::TextOnly;
    for record in recovered.records() {
        let JournalRecord::CommandCommitted(committed) = record.record() else {
            continue;
        };
        if matches!(
            committed.command(),
            AgentCommand::StartTurn { input, .. } | AgentCommand::SteerTurn { input, .. }
                if !input.images().is_empty()
        ) {
            history = InputImageHistory::ContainsImages;
            break;
        }
    }
    if history == InputImageHistory::ContainsImages {
        return history;
    }
    let Some(seed_sequence) = recovered.initial_fork_seed() else {
        return history;
    };
    let Some(seed) = recovered
        .records()
        .iter()
        .find_map(|record| {
            (record.journal_sequence() == Some(seed_sequence)).then(|| match record.record() {
                JournalRecord::InitialForkSeed(seed) => Some(seed.as_ref()),
                _ => None,
            })
        })
        .flatten()
    else {
        return InputImageHistory::Unknown;
    };
    for entry in seed.history() {
        if let JournalRecord::CommandCommitted(committed) = entry.record().record()
            && matches!(
                committed.command(),
                AgentCommand::StartTurn { input, .. } | AgentCommand::SteerTurn { input, .. }
                    if !input.images().is_empty()
            )
        {
            return InputImageHistory::ContainsImages;
        }
    }
    if matches!(seed.source(), ForkSource::Empty) {
        InputImageHistory::TextOnly
    } else {
        InputImageHistory::Unknown
    }
}

pub(crate) fn build_continuation(
    recovered: RecoveredJournal,
    session_id: SessionId,
) -> Result<StoredSessionContinuation, StoredSessionContinuationError> {
    let descriptor = recovered.descriptor().ok_or_else(|| {
        StoredSessionContinuationError::new(format!(
            "stored Session {session_id} has no durable descriptor"
        ))
    })?;
    if descriptor.session_id() != session_id {
        return Err(StoredSessionContinuationError::new(
            "stored Session descriptor identity does not match its repository key",
        ));
    }
    let epoch = recovered.binding_epoch().ok_or_else(|| {
        StoredSessionContinuationError::new("Continuation Anchor has no open backend binding")
    })?;
    let binding = recovered
        .records()
        .iter()
        .find_map(|record| match record.record() {
            JournalRecord::BackendBindingOpened(candidate) if candidate.epoch() == epoch => {
                Some(candidate.clone())
            },
            _ => None,
        })
        .ok_or_else(|| {
            StoredSessionContinuationError::new(
                "the open backend binding is absent from the recovered semantic Journal",
            )
        })?;
    let transition_source = binding
        .transition()
        .source_anchor_sequence()
        .map(BackendResumeSource::ContinuationAnchor)
        .or_else(|| {
            binding
                .transition()
                .source_checkpoint_sequence()
                .map(BackendResumeSource::ContextCheckpoint)
        });
    let open_epoch_has_accepted_request = recovered.records().iter().any(|record| {
        matches!(
            record.record(),
            JournalRecord::BackendRequestAccepted(request) if request.epoch() == epoch
        )
    });
    let resume_source = recovered
        .continuation_anchor()
        .map(BackendResumeSource::ContinuationAnchor)
        .or_else(|| {
            recovered
                .context_checkpoint()
                .map(BackendResumeSource::ContextCheckpoint)
        })
        .or_else(|| recovered.initial_fork_seed().map(BackendResumeSource::InitialFork))
        .or_else(|| {
            (!open_epoch_has_accepted_request)
                .then_some(transition_source)
                .flatten()
        })
        .ok_or_else(|| {
            StoredSessionContinuationError::new(format!(
                "stored Session {session_id} has no newest durable Continuation Anchor, context checkpoint, or initial fork seed"
            ))
        })?;
    let source_sequence = resume_source.sequence();
    let source_epoch = recovered
        .records()
        .iter()
        .find_map(|record| {
            (record.journal_sequence() == Some(source_sequence)).then_some(record.record())
        })
        .and_then(|record| match (resume_source, record) {
            (
                BackendResumeSource::ContinuationAnchor(_),
                JournalRecord::ContinuationAnchor(anchor),
            ) => Some(anchor.epoch()),
            (
                BackendResumeSource::ContextCheckpoint(_),
                JournalRecord::ContextCheckpoint(checkpoint),
            ) => Some(checkpoint.epoch()),
            (BackendResumeSource::InitialFork(sequence), JournalRecord::InitialForkSeed(_))
                if recovered.initial_fork_seed() == Some(sequence) =>
            {
                Some(epoch)
            },
            _ => None,
        })
        .ok_or_else(|| {
            StoredSessionContinuationError::new(format!(
                "continuation source {} is absent from the recovered semantic Journal",
                source_sequence.get()
            ))
        })?;
    let resumes_from_replacement_source = transition_source == Some(resume_source)
        && matches!(
            binding.transition().mode(),
            crate::journal::codec::TransitionMode::ExactReplay
                | crate::journal::codec::TransitionMode::BackendNativeModelRebind
        );
    if source_epoch != epoch && !resumes_from_replacement_source {
        return Err(StoredSessionContinuationError::new(format!(
            "continuation source {} belongs to epoch {source_epoch}, not open epoch {epoch}",
            source_sequence.get()
        )));
    }

    let mut max_turn = 0_u64;
    for record in recovered.records() {
        if let JournalRecord::CommandCommitted(committed) = record.record() {
            match committed.command() {
                AgentCommand::StartTurn { turn, .. }
                | AgentCommand::SteerTurn { turn, .. }
                | AgentCommand::InterruptTurn { turn } => {
                    max_turn = max_turn.max(turn.turn_id().get().get());
                },
                AgentCommand::RespondToActivity { request, .. } => {
                    max_turn = max_turn.max(request.activity().turn().turn_id().get().get());
                },
                AgentCommand::CreateSession { .. } | AgentCommand::CompactContext { .. } => {},
            }
        }
    }
    let evidence = BackendBindingEvidence::new(
        binding.backend_kind(),
        binding.backend_version(),
        identity(binding.binding_identity()),
        identity(binding.model_identity()),
        identity(binding.session_locator()),
        binding.continuation_strategy(),
    );
    let model_replay = match binding.continuation_strategy() {
        ContinuationStrategy::ExactReplay { .. } => recovered.model_replay().clone(),
        ContinuationStrategy::BackendManagedState => ModelReplay::default(),
    };
    let next_turn_id = max_turn.checked_add(1).ok_or_else(|| {
        StoredSessionContinuationError::new("stored Turn identity space is exhausted")
    })?;
    let transcript_records = normalize_recovered(&recovered).map_err(|detail| {
        StoredSessionContinuationError::new(format!(
            "stored Session {session_id} transcript cannot be restored: {detail}"
        ))
    })?;
    let inherited_history =
        project_inherited(&recovered).map_err(StoredSessionContinuationError::new)?;
    let target = match resume_source {
        BackendResumeSource::ContinuationAnchor(sequence) => {
            BackendResumeTarget::new(session_id, epoch, evidence, sequence)
        },
        BackendResumeSource::ContextCheckpoint(sequence) => {
            BackendResumeTarget::from_checkpoint(session_id, epoch, evidence, sequence)
        },
        BackendResumeSource::InitialFork(sequence) => {
            BackendResumeTarget::from_initial_fork(session_id, epoch, evidence, sequence)
        },
    }
    .with_model_replay(model_replay)
    .with_input_image_history(recovered_input_image_history(&recovered))
    .with_context_state(
        recovered.context_policy().cloned(),
        recovered.context_epoch(),
        recovered.model_replay_groups(),
    )
    .with_replay_contract_rebind_required(recovered.replay_contract_rebind_required())
    .with_binding_has_accepted_request(open_epoch_has_accepted_request);
    Ok(StoredSessionContinuation {
        recovered,
        target,
        next_turn_id,
        transcript_records,
        inherited_history,
    })
}

fn identity(value: &crate::journal::codec::VersionedIdentity) -> BackendIdentity {
    BackendIdentity::new(value.schema(), value.value())
}

#[cfg(test)]
mod tests;
