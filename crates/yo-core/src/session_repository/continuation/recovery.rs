use std::{collections::HashSet, error::Error, fmt, sync::Arc};

use crate::{
    AgentCommand, BackendBindingEvidence, BackendIdentity, BackendResumeSource,
    BackendResumeTarget, ContinuationStrategy, InputImageHistory, JournalSequence, ModelReplay,
    SessionDescriptor, SessionId, SubmissionId,
    journal::{
        JournalEntry,
        codec::{
            ForkSource, JournalCommit, JournalRecord, RecoveredJournal, TransitionMode,
            VersionedIdentity,
        },
    },
    session_repository::{
        SessionWriterRepository, StoredSessionReader, StoredSessionSnapshot,
        history::{InheritedSessionHistory, normalize_recovered, project_inherited},
        journal::{recover_entries, recover_repository},
    },
};

/// 실행 가능한 세션을 다시 열기 위해 필요한 완전히 검증된 내구 상태입니다.
#[derive(Clone, Debug)]
pub struct StoredSessionContinuation {
    recovered: RecoveredJournal,
    target: BackendResumeTarget,
    next_turn_id: u64,
    transcript_records: Vec<crate::TranscriptRecord>,
    inherited_history: Option<Arc<InheritedSessionHistory>>,
}

impl StoredSessionContinuation {
    /// 저장소나 백엔드 부작용 없이 독립적인 정확한 재생 자식을 준비합니다.
    ///
    /// 후보 바인딩은 캡처된 소스 식별자와 재생 프로필을 충족해야 합니다.
    /// 반환된 계획은 세션 시작과 게시에 성공한 뒤에만 내구화되고 실행 가능해집니다.
    /// 준비 과정에서 자식을 선택하거나 부모를 변경하지 않습니다.
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

    /// 소스가 구분된 보관 기록이며 자식 실행과 사용량에서 제외됩니다.
    #[must_use]
    pub fn inherited_history(&self) -> Option<&InheritedSessionHistory> {
        self.inherited_history.as_deref()
    }

    pub(crate) fn semantic_entries(&self) -> Vec<JournalEntry> {
        self.recovered.semantic_entries()
    }

    pub(crate) fn snapshot(&self) -> JournalCommit {
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

    /// 이 세션 저널에서 복구하고 검증한 읽기 전용 프런트엔드 기록입니다.
    /// 호스트는 소스 파일을 다시 열거나 재생 전용 이미지를 편집 가능한 입력으로 취급하지 않고
    /// 이 기록의 승인된 첨부 증거를 보존할 수 있습니다.
    pub fn transcript_records(&self) -> &[crate::TranscriptRecord] {
        &self.transcript_records
    }
}

/// 내구 세션을 실행 가능한 네이티브 재개로 승인할 수 없는 이유입니다.
#[derive(Debug)]
pub struct StoredSessionContinuationError {
    detail: String,
}

impl StoredSessionContinuationError {
    pub(super) fn new(detail: impl Into<String>) -> Self {
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

impl Error for StoredSessionContinuationError {}

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

/// 쓰기 임대를 획득하거나 저장소를 변경하지 않고 실행 가능한 연속성을 검증합니다.
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
            TransitionMode::ExactReplay | TransitionMode::BackendNativeModelRebind
        );
    if !super::validation::source_epoch_matches_open_epoch(
        source_epoch,
        epoch,
        resumes_from_replacement_source,
    ) {
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

fn identity(value: &VersionedIdentity) -> BackendIdentity {
    BackendIdentity::new(value.schema(), value.value())
}
