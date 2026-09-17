use super::{
    LocalDirectoryIdentity,
    model::{ConnectionOperationExecutionError, ConnectionOperationExecutionOutcome},
    recovery::{RecoveryStep, execute_recovery},
    repositories::LocalConnectionOperationRepositories,
};
use crate::model_service::{
    ConnectionCommit, ConnectionSnapshot, CredentialSnapshot, LocalConnectionOperationGuard,
    PreparedConnectionMutation,
};

/// 후속 planning을 위해 guard를 유지하는 하나의 local operation lane입니다.
pub struct LocalConnectionOperationSession<'a> {
    pub(in super::super) repositories: &'a LocalConnectionOperationRepositories,
    pub(in super::super) guard: LocalConnectionOperationGuard,
    pub(in super::super) directory_identity: LocalDirectoryIdentity,
}

impl LocalConnectionOperationSession<'_> {
    pub fn recover_pending_operation(
        &mut self,
    ) -> Result<ConnectionOperationExecutionOutcome, ConnectionOperationExecutionError> {
        self.recover_pending_operation_with(|_| Ok(()))
    }

    /// 같은 serialized operation lane을 유지하면서 public state를 capture합니다.
    pub fn capture_connections(
        &self,
    ) -> Result<ConnectionSnapshot, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        self.repositories
            .connections
            .capture()
            .map_err(ConnectionOperationExecutionError::PublicCapture)
    }

    /// 같은 serialized operation lane을 유지하면서 private credential state를 capture합니다.
    pub fn capture_credentials(
        &self,
    ) -> Result<CredentialSnapshot, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        self.repositories
            .credentials
            .capture()
            .map_err(ConnectionOperationExecutionError::CredentialCapture)
    }

    /// 유지 중인 lane 아래에서 preference-only 또는 stored public mutation을 게시합니다.
    pub fn commit_connection_mutation(
        &mut self,
        mutation: &PreparedConnectionMutation,
    ) -> Result<ConnectionCommit, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        self.repositories
            .connections
            .commit(mutation)
            .map_err(ConnectionOperationExecutionError::PublicCommit)
    }

    fn recover_pending_operation_with(
        &mut self,
        mut observe: impl FnMut(RecoveryStep) -> Result<(), ConnectionOperationExecutionError>,
    ) -> Result<ConnectionOperationExecutionOutcome, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        self.repositories
            .journal
            .cleanup_pending_residues(&mut self.guard)
            .map_err(ConnectionOperationExecutionError::JournalCapture)?;
        self.directory_identity.revalidate()?;
        let Some(entry) = self
            .repositories
            .journal
            .capture()
            .map_err(ConnectionOperationExecutionError::JournalCapture)?
        else {
            return Ok(ConnectionOperationExecutionOutcome::NoPendingOperation);
        };
        execute_recovery(
            self.repositories,
            &self.directory_identity,
            &mut self.guard,
            entry,
            &mut observe,
        )
    }

    #[cfg(test)]
    pub(in super::super) fn recover_pending_operation_until(
        &mut self,
        stop: RecoveryStep,
    ) -> Result<ConnectionOperationExecutionOutcome, ConnectionOperationExecutionError> {
        self.recover_pending_operation_with(|step| {
            if step == stop {
                Err(ConnectionOperationExecutionError::InjectedInterruption)
            } else {
                Ok(())
            }
        })
    }
}
