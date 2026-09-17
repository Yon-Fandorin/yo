//! physical discovery summary를 semantic Journal 권위와 검증하는 값과 함수입니다.

use std::fmt;

use crate::{
    JournalSequence, SessionDescriptor,
    journal::codec::RecoveredJournal,
    session_repository::{RepositoryEntry, RepositorySequence},
};

/// 한 stored Session history의 physical discovery summary 검증 결과입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredDiscoveryValidation {
    Consistent,
    Mismatch(StoredDiscoveryMismatch),
}

/// semantic Journal 권위와 처음 달라지는 physical discovery summary입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredDiscoveryMismatch {
    pub(super) repository_sequence: RepositorySequence,
    pub(super) kind: StoredDiscoveryMismatchKind,
}

impl StoredDiscoveryMismatch {
    #[must_use]
    pub const fn new(
        repository_sequence: RepositorySequence,
        kind: StoredDiscoveryMismatchKind,
    ) -> Self {
        Self {
            repository_sequence,
            kind,
        }
    }

    #[must_use]
    pub const fn repository_sequence(self) -> RepositorySequence {
        self.repository_sequence
    }

    #[must_use]
    pub const fn kind(self) -> StoredDiscoveryMismatchKind {
        self.kind
    }
}

impl fmt::Display for StoredDiscoveryMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let repository_sequence = self.repository_sequence.get();
        match self.kind {
            StoredDiscoveryMismatchKind::Missing => write!(
                formatter,
                "metadata is missing at repository sequence {repository_sequence}"
            ),
            StoredDiscoveryMismatchKind::Descriptor => write!(
                formatter,
                "descriptor disagrees with its semantic Journal at repository sequence {repository_sequence}"
            ),
            StoredDiscoveryMismatchKind::BindingEpoch { claimed } => write!(
                formatter,
                "binding epoch {claimed} at repository sequence {repository_sequence} has no semantic Journal binding evidence"
            ),
            StoredDiscoveryMismatchKind::ContinuationAnchor { referenced } => write!(
                formatter,
                "Continuation Anchor Journal sequence {} at repository sequence {repository_sequence} has no semantic Journal anchor evidence",
                referenced.get()
            ),
            StoredDiscoveryMismatchKind::InitialForkSeed { referenced } => write!(
                formatter,
                "Initial fork seed Journal sequence {} at repository sequence {repository_sequence} has no semantic Journal seed evidence",
                referenced.get()
            ),
            StoredDiscoveryMismatchKind::MissingInitialForkSeed { expected } => write!(
                formatter,
                "Initial fork seed Journal sequence {} is missing at repository sequence {repository_sequence}",
                expected.get()
            ),
            StoredDiscoveryMismatchKind::BindingEpochDisagreement { expected, claimed } => write!(
                formatter,
                "binding epoch {claimed} disagrees with semantic Journal epoch {expected} at repository sequence {repository_sequence}"
            ),
            StoredDiscoveryMismatchKind::ContinuationAnchorDisagreement { expected, claimed } => {
                write!(
                    formatter,
                    "Continuation Anchor Journal sequence {} disagrees with semantic Journal sequence {} at repository sequence {repository_sequence}",
                    claimed.get(),
                    expected.get()
                )
            },
            StoredDiscoveryMismatchKind::MissingBindingEpoch { expected } => write!(
                formatter,
                "binding epoch {expected} is missing at repository sequence {repository_sequence}"
            ),
            StoredDiscoveryMismatchKind::MissingContinuationAnchor { expected } => write!(
                formatter,
                "Continuation Anchor Journal sequence {} is missing at repository sequence {repository_sequence}",
                expected.get()
            ),
        }
    }
}

/// physical discovery summary를 semantic Journal prefix에서 만들 수 없는 이유입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredDiscoveryMismatchKind {
    Missing,
    Descriptor,
    BindingEpoch {
        claimed: u64,
    },
    ContinuationAnchor {
        referenced: JournalSequence,
    },
    /// physical hint에 검증된 initial child seed가 없습니다.
    InitialForkSeed {
        referenced: JournalSequence,
    },
    /// 검증된 실행 가능한 initial seed가 physical discovery에서 누락되었습니다.
    MissingInitialForkSeed {
        expected: JournalSequence,
    },
    BindingEpochDisagreement {
        expected: u64,
        claimed: u64,
    },
    ContinuationAnchorDisagreement {
        expected: JournalSequence,
        claimed: JournalSequence,
    },
    MissingBindingEpoch {
        expected: u64,
    },
    MissingContinuationAnchor {
        expected: JournalSequence,
    },
}

pub(super) fn validate_discovery(
    entries: &[RepositoryEntry],
    descriptor: &SessionDescriptor,
    recovered: &RecoveredJournal,
) -> StoredDiscoveryValidation {
    for (entry, expected) in entries.iter().zip(recovered.discovery_states()) {
        let repository_sequence = entry.sequence();
        let Some(discovery) = entry.record().discovery() else {
            return StoredDiscoveryValidation::Mismatch(StoredDiscoveryMismatch::new(
                repository_sequence,
                StoredDiscoveryMismatchKind::Missing,
            ));
        };
        let kind = if discovery.descriptor() != descriptor {
            Some(StoredDiscoveryMismatchKind::Descriptor)
        } else if expected.initial_fork_seed() != discovery.initial_fork_seed() {
            match discovery.initial_fork_seed() {
                Some(referenced) => {
                    Some(StoredDiscoveryMismatchKind::InitialForkSeed { referenced })
                },
                None => Some(StoredDiscoveryMismatchKind::MissingInitialForkSeed {
                    expected: expected
                        .initial_fork_seed()
                        .expect("different hints have an expected seed"),
                }),
            }
        } else {
            discovery_coordinates_mismatch(
                expected.binding_epoch(),
                discovery.binding_epoch(),
                expected.continuation_anchor(),
                discovery.continuation_anchor(),
            )
        };
        if let Some(kind) = kind {
            return StoredDiscoveryValidation::Mismatch(StoredDiscoveryMismatch::new(
                repository_sequence,
                kind,
            ));
        }
    }
    StoredDiscoveryValidation::Consistent
}

pub(super) fn discovery_coordinates_mismatch(
    expected_epoch: Option<u64>,
    claimed_epoch: Option<u64>,
    expected_anchor: Option<JournalSequence>,
    claimed_anchor: Option<JournalSequence>,
) -> Option<StoredDiscoveryMismatchKind> {
    if expected_epoch != claimed_epoch {
        return Some(match (expected_epoch, claimed_epoch) {
            (None, Some(claimed)) => StoredDiscoveryMismatchKind::BindingEpoch { claimed },
            (Some(expected), None) => StoredDiscoveryMismatchKind::MissingBindingEpoch { expected },
            (Some(expected), Some(claimed)) => {
                StoredDiscoveryMismatchKind::BindingEpochDisagreement { expected, claimed }
            },
            (None, None) => unreachable!("equal optional epochs were handled above"),
        });
    }
    if expected_anchor != claimed_anchor {
        return Some(match (expected_anchor, claimed_anchor) {
            (None, Some(referenced)) => {
                StoredDiscoveryMismatchKind::ContinuationAnchor { referenced }
            },
            (Some(expected), None) => {
                StoredDiscoveryMismatchKind::MissingContinuationAnchor { expected }
            },
            (Some(expected), Some(claimed)) => {
                StoredDiscoveryMismatchKind::ContinuationAnchorDisagreement { expected, claimed }
            },
            (None, None) => unreachable!("equal optional Anchors were handled above"),
        });
    }
    None
}
