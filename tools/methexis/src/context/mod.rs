//! Deterministic, token-bounded ContextBuild facade.

mod candidate;
mod hash;
mod operations;
mod payload;
mod selection;
mod storage;
mod wire;

use std::path::Path;

pub(crate) use wire::{ResolveFailure, ResolveSuccess};

pub(crate) struct ContextService<'a> {
    repository_root: &'a Path,
}

impl<'a> ContextService<'a> {
    pub(crate) const fn new(repository_root: &'a Path) -> Self {
        Self { repository_root }
    }

    pub(crate) fn resolve(&self, request_path: &Path) -> Result<ResolveSuccess, ResolveFailure> {
        operations::resolve(self.repository_root, request_path)
    }
}
