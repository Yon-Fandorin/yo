use std::num::NonZeroU64;

use super::{
    ActivityId, ActivityRef, HostWorkspacePath, JournalRecord, JournalSequence,
    SequencedJournalRecord, SessionDescriptor, SubmissionId, TurnId, TurnRef,
};

pub(super) fn activity() -> ActivityRef {
    let session_id = crate::fixture_session(1);
    let turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(2).unwrap()));
    ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(3).unwrap()))
}

pub(super) fn submission(value: u8) -> SubmissionId {
    SubmissionId::from_uuid(uuid::Builder::from_random_bytes([value; 16]).into_uuid())
        .expect("the test submission fixture is a UUIDv4")
}

pub(super) fn sequenced(
    first: u64,
    records: impl IntoIterator<Item = JournalRecord>,
) -> Vec<SequencedJournalRecord> {
    records
        .into_iter()
        .enumerate()
        .map(|(offset, record)| {
            SequencedJournalRecord::new(
                JournalSequence::new(first + u64::try_from(offset).unwrap()),
                record,
            )
        })
        .collect()
}

pub(super) fn descriptor_with_path(path: Vec<u8>) -> SessionDescriptor {
    SessionDescriptor::for_session(
        activity().session_id(),
        "10000000-0000-4000-8000-000000000001"
            .parse()
            .expect("the test Host fixture is a UUIDv4"),
        HostWorkspacePath::from_unix_bytes(path)
            .expect("the test workspace path is absolute and NUL-free"),
    )
}
