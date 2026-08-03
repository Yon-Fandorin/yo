//! Frontend-neutral identities and search messages for explicit skill references.

use std::ops::Range;

mod search;

pub(crate) use search::search_candidates;

/// Provenance reported by the execution environment that owns the skill catalog.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SkillReferenceScope {
    Workspace,
    User,
    System,
    Admin,
}

/// Whether the exact catalog entry may currently be selected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkillAvailability {
    Enabled,
    Disabled(String),
}

/// One revision-bound skill descriptor from an execution environment's catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillReference {
    identity: String,
    execution_environment_identity: String,
    locator: String,
    name: String,
    scope: SkillReferenceScope,
    catalog_generation: u64,
    entry_revision: String,
}

/// One display candidate while its typed identity remains separate from the projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillReferenceCandidate {
    reference: SkillReference,
    display_name: String,
    description: String,
    availability: SkillAvailability,
}

/// A replaceable `$` search tied to one editor revision and trigger span.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillReferenceSearchRequest {
    request_id: u64,
    editor_revision: u64,
    cursor: usize,
    replacement_start: usize,
    replacement_end: usize,
    expected_trigger: String,
    query: String,
    refresh_catalog: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkillReferenceSearchStatus {
    Complete,
    Incomplete(String),
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillReferenceSearchUpdate {
    request_id: u64,
    editor_revision: u64,
    sequence: u64,
    final_update: bool,
    status: SkillReferenceSearchStatus,
    candidates: Vec<SkillReferenceCandidate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkillReferenceProviderPoll {
    Pending,
    Update(SkillReferenceSearchUpdate),
}

pub trait SkillReferenceProvider: Send {
    fn search(&mut self, request: SkillReferenceSearchRequest) -> Result<(), String>;
    fn poll(&mut self) -> Result<SkillReferenceProviderPoll, String>;
}

impl SkillReference {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        identity: impl Into<String>,
        execution_environment_identity: impl Into<String>,
        locator: impl Into<String>,
        name: impl Into<String>,
        scope: SkillReferenceScope,
        catalog_generation: u64,
        entry_revision: impl Into<String>,
    ) -> Self {
        Self {
            identity: identity.into(),
            execution_environment_identity: execution_environment_identity.into(),
            locator: locator.into(),
            name: name.into(),
            scope,
            catalog_generation,
            entry_revision: entry_revision.into(),
        }
    }

    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }
    #[must_use]
    pub fn execution_environment_identity(&self) -> &str {
        &self.execution_environment_identity
    }
    #[must_use]
    pub fn locator(&self) -> &str {
        &self.locator
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub const fn scope(&self) -> SkillReferenceScope {
        self.scope
    }
    #[must_use]
    pub const fn catalog_generation(&self) -> u64 {
        self.catalog_generation
    }
    #[must_use]
    pub fn entry_revision(&self) -> &str {
        &self.entry_revision
    }
}

impl SkillReferenceCandidate {
    #[must_use]
    pub fn new(
        reference: SkillReference,
        display_name: impl Into<String>,
        description: impl Into<String>,
        availability: SkillAvailability,
    ) -> Self {
        Self {
            reference,
            display_name: display_name.into(),
            description: description.into(),
            availability,
        }
    }

    #[must_use]
    pub fn reference(&self) -> &SkillReference {
        &self.reference
    }
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
    #[must_use]
    pub fn availability(&self) -> &SkillAvailability {
        &self.availability
    }
}

impl SkillReferenceSearchRequest {
    #[must_use]
    pub fn new(
        request_id: u64,
        editor_revision: u64,
        cursor: usize,
        replacement: Range<usize>,
        expected_trigger: impl Into<String>,
        query: impl Into<String>,
        refresh_catalog: bool,
    ) -> Self {
        Self {
            request_id,
            editor_revision,
            cursor,
            replacement_start: replacement.start,
            replacement_end: replacement.end,
            expected_trigger: expected_trigger.into(),
            query: query.into(),
            refresh_catalog,
        }
    }

    #[must_use]
    pub const fn request_id(&self) -> u64 {
        self.request_id
    }
    #[must_use]
    pub const fn editor_revision(&self) -> u64 {
        self.editor_revision
    }
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }
    #[must_use]
    pub const fn replacement(&self) -> Range<usize> {
        self.replacement_start..self.replacement_end
    }
    #[must_use]
    pub fn expected_trigger(&self) -> &str {
        &self.expected_trigger
    }
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }
    #[must_use]
    pub const fn refresh_catalog(&self) -> bool {
        self.refresh_catalog
    }
}

impl SkillReferenceSearchUpdate {
    #[must_use]
    pub fn final_result(
        request: &SkillReferenceSearchRequest,
        status: SkillReferenceSearchStatus,
        candidates: Vec<SkillReferenceCandidate>,
    ) -> Self {
        Self {
            request_id: request.request_id,
            editor_revision: request.editor_revision,
            sequence: 0,
            final_update: true,
            status,
            candidates,
        }
    }

    #[must_use]
    pub const fn request_id(&self) -> u64 {
        self.request_id
    }
    #[must_use]
    pub const fn editor_revision(&self) -> u64 {
        self.editor_revision
    }
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
    #[must_use]
    pub const fn is_final(&self) -> bool {
        self.final_update
    }
    #[must_use]
    pub fn status(&self) -> &SkillReferenceSearchStatus {
        &self.status
    }
    #[must_use]
    pub fn candidates(&self) -> &[SkillReferenceCandidate] {
        &self.candidates
    }
}

#[cfg(test)]
mod tests;
