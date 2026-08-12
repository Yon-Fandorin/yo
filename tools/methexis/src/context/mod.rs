//! Deterministic, token-bounded ContextBuild facade.

mod candidate;
mod hash;
mod operations;
mod payload;
mod refresh;
pub(crate) mod registry;
mod selection;
mod storage;
mod verify;
mod wire;

use std::path::Path;

pub(crate) use refresh::{RefreshFailure, RefreshSuccess};
pub(crate) use wire::{ResolveFailure, ResolveSuccess, VerifySuccess};

pub(crate) struct ContextService<'a> {
    repository_root: &'a Path,
}

pub(crate) fn manifest_refresh_reader_guard(
    repository_root: &Path,
) -> Result<crate::publication::RepositoryGuard, String> {
    refresh::transaction_reader_guard(repository_root)
}

impl<'a> ContextService<'a> {
    pub(crate) const fn new(repository_root: &'a Path) -> Self {
        Self { repository_root }
    }

    pub(crate) fn resolve(&self, request_path: &Path) -> Result<ResolveSuccess, ResolveFailure> {
        operations::resolve(self.repository_root, request_path)
    }

    pub(crate) fn verify(
        &self,
        request_path: &Path,
        expected_build_id: &str,
    ) -> Result<VerifySuccess, ResolveFailure> {
        verify::run(self.repository_root, request_path, expected_build_id)
    }

    pub(crate) fn refresh_manifests(
        &self,
        request_path: &Path,
    ) -> Result<RefreshSuccess, RefreshFailure> {
        refresh::run(self.repository_root, request_path)
    }
}
