use std::path::{Path, PathBuf};

use super::{
    ConnectionOperationError, ConnectionOperationJournalEntry, ConnectionOperationPhase, storage,
};
use crate::model_service::LocalConnectionOperationGuard;

pub trait ConnectionOperationJournalRepository {
    type OperationGuard;

    fn capture(&self) -> Result<Option<ConnectionOperationJournalEntry>, ConnectionOperationError>;
    fn publish_intent(
        &self,
        guard: &mut Self::OperationGuard,
        entry: &ConnectionOperationJournalEntry,
    ) -> Result<(), ConnectionOperationError>;
    fn advance(
        &self,
        guard: &mut Self::OperationGuard,
        current: &ConnectionOperationJournalEntry,
        next: ConnectionOperationPhase,
    ) -> Result<ConnectionOperationJournalEntry, ConnectionOperationError>;
    fn abandon_intent(
        &self,
        guard: &mut Self::OperationGuard,
        current: &ConnectionOperationJournalEntry,
    ) -> Result<(), ConnectionOperationError>;
    fn clear_complete(
        &self,
        guard: &mut Self::OperationGuard,
        current: &ConnectionOperationJournalEntry,
    ) -> Result<(), ConnectionOperationError>;
}

#[derive(Clone, Debug)]
pub struct LocalConnectionOperationJournal {
    path: PathBuf,
}

impl LocalConnectionOperationJournal {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn capture(
        &self,
    ) -> Result<Option<ConnectionOperationJournalEntry>, ConnectionOperationError> {
        storage::capture(&self.path)
    }

    pub fn publish_intent(
        &self,
        guard: &mut LocalConnectionOperationGuard,
        entry: &ConnectionOperationJournalEntry,
    ) -> Result<(), ConnectionOperationError> {
        self.require_guard(guard)?;
        storage::publish_intent(&self.path, entry)
    }

    pub fn advance(
        &self,
        guard: &mut LocalConnectionOperationGuard,
        current: &ConnectionOperationJournalEntry,
        next: ConnectionOperationPhase,
    ) -> Result<ConnectionOperationJournalEntry, ConnectionOperationError> {
        self.require_guard(guard)?;
        storage::advance(&self.path, current, next)
    }

    pub fn clear_complete(
        &self,
        guard: &mut LocalConnectionOperationGuard,
        current: &ConnectionOperationJournalEntry,
    ) -> Result<(), ConnectionOperationError> {
        self.require_guard(guard)?;
        storage::clear_complete(&self.path, current)
    }

    pub fn abandon_intent(
        &self,
        guard: &mut LocalConnectionOperationGuard,
        current: &ConnectionOperationJournalEntry,
    ) -> Result<(), ConnectionOperationError> {
        self.require_guard(guard)?;
        storage::abandon_intent(&self.path, current)
    }

    fn require_guard(
        &self,
        guard: &LocalConnectionOperationGuard,
    ) -> Result<(), ConnectionOperationError> {
        if guard.authorizes(&self.path) {
            Ok(())
        } else {
            Err(ConnectionOperationError::OperationGuardMismatch(
                self.path.clone(),
            ))
        }
    }
}

impl ConnectionOperationJournalRepository for LocalConnectionOperationJournal {
    type OperationGuard = LocalConnectionOperationGuard;

    fn capture(&self) -> Result<Option<ConnectionOperationJournalEntry>, ConnectionOperationError> {
        Self::capture(self)
    }

    fn publish_intent(
        &self,
        guard: &mut Self::OperationGuard,
        entry: &ConnectionOperationJournalEntry,
    ) -> Result<(), ConnectionOperationError> {
        Self::publish_intent(self, guard, entry)
    }

    fn advance(
        &self,
        guard: &mut Self::OperationGuard,
        current: &ConnectionOperationJournalEntry,
        next: ConnectionOperationPhase,
    ) -> Result<ConnectionOperationJournalEntry, ConnectionOperationError> {
        Self::advance(self, guard, current, next)
    }

    fn clear_complete(
        &self,
        guard: &mut Self::OperationGuard,
        current: &ConnectionOperationJournalEntry,
    ) -> Result<(), ConnectionOperationError> {
        Self::clear_complete(self, guard, current)
    }

    fn abandon_intent(
        &self,
        guard: &mut Self::OperationGuard,
        current: &ConnectionOperationJournalEntry,
    ) -> Result<(), ConnectionOperationError> {
        Self::abandon_intent(self, guard, current)
    }
}
