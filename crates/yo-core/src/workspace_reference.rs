//! Frontend-neutral identities and search messages for execution-workspace references.

use std::fmt;

use unicode_normalization::UnicodeNormalization;

mod local;

pub use local::LocalWorkspaceReferenceProvider;

/// The filesystem kind observed by the execution environment during discovery.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WorkspaceReferenceKind {
    File,
    Directory,
}

/// One typed reference into an execution environment's workspace root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceReference {
    identity: String,
    execution_environment_identity: String,
    workspace_identity: String,
    root_identity: String,
    relative_path: String,
    kind: WorkspaceReferenceKind,
}

/// A path that cannot identify one canonical entry below a workspace root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceReferencePathError;

/// One ranked row returned to a prompt-assist controller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceReferenceCandidate {
    reference: WorkspaceReference,
    label: String,
    detail: String,
}

/// A revision-bound, replaceable workspace search request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceReferenceSearchRequest {
    request_id: u64,
    editor_revision: u64,
    cursor: usize,
    replacement_start: usize,
    replacement_end: usize,
    expected_trigger: String,
    query: String,
}

/// Completeness reported by the authoritative execution environment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceReferenceSearchStatus {
    Complete,
    Incomplete(String),
    Failed(String),
}

/// One ordered provider update. V1 providers may publish one final update only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceReferenceSearchUpdate {
    request_id: u64,
    editor_revision: u64,
    sequence: u64,
    final_update: bool,
    status: WorkspaceReferenceSearchStatus,
    candidates: Vec<WorkspaceReferenceCandidate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceReferenceProviderPoll {
    Pending,
    Update(WorkspaceReferenceSearchUpdate),
}

pub trait WorkspaceReferenceProvider: Send {
    fn search(&mut self, request: WorkspaceReferenceSearchRequest) -> Result<(), String>;
    fn poll(&mut self) -> Result<WorkspaceReferenceProviderPoll, String>;
}

impl WorkspaceReference {
    /// Builds a reference from a canonical, root-relative `/`-separated path.
    pub fn new(
        identity: impl Into<String>,
        execution_environment_identity: impl Into<String>,
        workspace_identity: impl Into<String>,
        root_identity: impl Into<String>,
        relative_path: impl Into<String>,
        kind: WorkspaceReferenceKind,
    ) -> Result<Self, WorkspaceReferencePathError> {
        let relative_path = relative_path.into();
        if relative_path.is_empty()
            || relative_path.starts_with('/')
            || relative_path.ends_with('/')
            || relative_path
                .split('/')
                .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return Err(WorkspaceReferencePathError);
        }
        Ok(Self {
            identity: identity.into(),
            execution_environment_identity: execution_environment_identity.into(),
            workspace_identity: workspace_identity.into(),
            root_identity: root_identity.into(),
            relative_path,
            kind,
        })
    }

    pub(crate) fn from_validated_persisted_v1(
        identity: String,
        execution_environment_identity: String,
        workspace_identity: String,
        root_identity: String,
        relative_path: String,
        kind: WorkspaceReferenceKind,
    ) -> Self {
        Self {
            identity,
            execution_environment_identity,
            workspace_identity,
            root_identity,
            relative_path,
            kind,
        }
    }

    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }
    #[must_use]
    pub fn root_identity(&self) -> &str {
        &self.root_identity
    }
    #[must_use]
    pub fn execution_environment_identity(&self) -> &str {
        &self.execution_environment_identity
    }
    #[must_use]
    pub fn workspace_identity(&self) -> &str {
        &self.workspace_identity
    }
    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }
    #[must_use]
    pub const fn kind(&self) -> WorkspaceReferenceKind {
        self.kind
    }
}

impl fmt::Display for WorkspaceReferencePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("workspace reference path must be canonical and root-relative")
    }
}

impl std::error::Error for WorkspaceReferencePathError {}

impl WorkspaceReferenceCandidate {
    #[must_use]
    pub fn new(reference: WorkspaceReference) -> Self {
        let path = reference.relative_path();
        let (parent, label) = path.rsplit_once('/').unwrap_or((".", path));
        let label = match reference.kind() {
            WorkspaceReferenceKind::Directory => format!("{label}/"),
            WorkspaceReferenceKind::File => label.to_owned(),
        };
        let detail = if parent == "." {
            String::new()
        } else {
            format!("{parent}/")
        };
        Self {
            reference,
            label,
            detail,
        }
    }

    #[must_use]
    pub fn reference(&self) -> &WorkspaceReference {
        &self.reference
    }
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl WorkspaceReferenceSearchRequest {
    #[must_use]
    pub fn new(
        request_id: u64,
        editor_revision: u64,
        cursor: usize,
        replacement: std::ops::Range<usize>,
        expected_trigger: impl Into<String>,
        query: impl Into<String>,
    ) -> Self {
        Self {
            request_id,
            editor_revision,
            cursor,
            replacement_start: replacement.start,
            replacement_end: replacement.end,
            expected_trigger: expected_trigger.into(),
            query: query.into(),
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
    pub fn query(&self) -> &str {
        &self.query
    }
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }
    #[must_use]
    pub const fn replacement(&self) -> std::ops::Range<usize> {
        self.replacement_start..self.replacement_end
    }
    #[must_use]
    pub fn expected_trigger(&self) -> &str {
        &self.expected_trigger
    }
}

impl WorkspaceReferenceSearchUpdate {
    #[must_use]
    pub fn final_result(
        request: &WorkspaceReferenceSearchRequest,
        status: WorkspaceReferenceSearchStatus,
        candidates: Vec<WorkspaceReferenceCandidate>,
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
    pub fn status(&self) -> &WorkspaceReferenceSearchStatus {
        &self.status
    }
    #[must_use]
    pub fn candidates(&self) -> &[WorkspaceReferenceCandidate] {
        &self.candidates
    }
}

/// Returns a deterministic relevance key after Unicode normalization.
#[must_use]
pub fn normalized_search_key(value: &str) -> String {
    value.nfkc().flat_map(char::to_lowercase).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        WorkspaceReference, WorkspaceReferenceCandidate, WorkspaceReferenceKind,
        normalized_search_key,
    };

    // 조합 방식이 다른 동등한 Unicode 경로도 같은 검색 키가 되어 provider별 순서가 흔들리지 않는다.
    #[test]
    fn normalization_makes_equivalent_unicode_paths_share_a_search_key() {
        assert_eq!(
            normalized_search_key("Cafe\u{301}"),
            normalized_search_key("CAFÉ")
        );
    }

    // 디렉터리 후보 label은 파일과 즉시 구분되도록 basename 뒤에 `/`를 붙인다.
    #[test]
    fn directory_candidate_label_has_a_trailing_separator() {
        let candidate = WorkspaceReferenceCandidate::new(
            WorkspaceReference::new(
                "id",
                "environment",
                "workspace",
                "root",
                "src/components",
                WorkspaceReferenceKind::Directory,
            )
            .unwrap(),
        );
        assert_eq!(candidate.label(), "components/");
        assert_eq!(candidate.detail(), "src/");
    }

    // 참조 identity에는 표시용 `/`를 섞지 않고 한 가지 canonical 경로만 허용한다.
    #[test]
    fn reference_rejects_non_canonical_relative_paths() {
        for path in [
            "",
            "/src",
            "src/",
            "src//main.rs",
            "./src",
            "src/../main.rs",
        ] {
            assert!(
                WorkspaceReference::new(
                    "id",
                    "environment",
                    "workspace",
                    "root",
                    path,
                    WorkspaceReferenceKind::File,
                )
                .is_err()
            );
        }
    }
}
