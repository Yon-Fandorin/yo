//! Bounded read-only ancestry derived from complete, validated inline fork provenance.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    InheritedHistorySource, RepositoryEntry, RepositoryError, StoredDiscoveryValidation,
    StoredSession, StoredSessionUnavailableReason, history::validate_discovery,
    journal::recover_entries,
};
use crate::{
    HostWorkspacePath, SessionId, WorkspaceHostId,
    journal::codec::{ForkSource, JournalRecord},
};

/// Independent physical work bounds for one tree query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionTreeLimits {
    directory_entries: usize,
    sessions: usize,
    physical_bytes: u64,
    physical_records: usize,
}

impl SessionTreeLimits {
    pub fn try_new(
        directory_entries: usize,
        sessions: usize,
        physical_bytes: u64,
        physical_records: usize,
    ) -> Result<Self, RepositoryError> {
        if !(1..=65_536).contains(&directory_entries)
            || !(1..=1024).contains(&sessions)
            || !(1..=256 * 1024 * 1024).contains(&physical_bytes)
            || !(1..=65_536).contains(&physical_records)
        {
            return Err(RepositoryError::Unavailable {
                message: "Session tree limits must be positive and within 65536 directory entries, 1024 Sessions, 256 MiB, and 65536 records".into(),
            });
        }
        Ok(Self {
            directory_entries,
            sessions,
            physical_bytes,
            physical_records,
        })
    }

    #[must_use]
    pub const fn directory_entries(self) -> usize {
        self.directory_entries
    }
    #[must_use]
    pub const fn sessions(self) -> usize {
        self.sessions
    }
    #[must_use]
    pub const fn physical_bytes(self) -> u64 {
        self.physical_bytes
    }
    #[must_use]
    pub const fn physical_records(self) -> usize {
        self.physical_records
    }
}

impl Default for SessionTreeLimits {
    fn default() -> Self {
        Self {
            directory_entries: 4096,
            sessions: 64,
            physical_bytes: 32 * 1024 * 1024,
            physical_records: 4096,
        }
    }
}

/// Ancestry evidence; absence of legacy provenance never establishes a root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionTreeAncestry {
    UnknownLegacy,
    Uninspected,
    Invalid {
        detail: String,
    },
    ValidatedFork {
        parent_session_id: SessionId,
        source: InheritedHistorySource,
    },
}

/// Why an ancestor or unreadable candidate cannot be shown as a validated local Session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionTreePlaceholder {
    MissingAncestor,
    OutsideWorkspace,
    Unavailable,
    Uninspected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionTreeNode {
    session_id: SessionId,
    metadata: Option<StoredSession>,
    depth: usize,
    ancestry: SessionTreeAncestry,
    placeholder: Option<SessionTreePlaceholder>,
}

impl SessionTreeNode {
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
    #[must_use]
    pub const fn metadata(&self) -> Option<&StoredSession> {
        self.metadata.as_ref()
    }
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }
    #[must_use]
    pub const fn ancestry(&self) -> &SessionTreeAncestry {
        &self.ancestry
    }
    #[must_use]
    pub const fn placeholder(&self) -> Option<SessionTreePlaceholder> {
        self.placeholder
    }
    #[must_use]
    pub const fn parent_session_id(&self) -> Option<SessionId> {
        match self.ancestry {
            SessionTreeAncestry::ValidatedFork {
                parent_session_id, ..
            } => Some(parent_session_id),
            _ => None,
        }
    }
    #[must_use]
    pub const fn source(&self) -> Option<InheritedHistorySource> {
        match self.ancestry {
            SessionTreeAncestry::ValidatedFork { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Deterministic parent-before-child forest; unknown nodes are not claimed to be roots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSessionTree {
    nodes: Vec<SessionTreeNode>,
    truncated: bool,
}

impl StoredSessionTree {
    #[must_use]
    pub fn nodes(&self) -> &[SessionTreeNode] {
        &self.nodes
    }
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

pub(super) struct TreeCandidate {
    node: SessionTreeNode,
    workspace: Option<(WorkspaceHostId, HostWorkspacePath)>,
}

impl TreeCandidate {
    pub(super) fn uninspected(session_id: SessionId) -> Self {
        Self {
            node: SessionTreeNode {
                session_id,
                metadata: None,
                depth: 0,
                ancestry: SessionTreeAncestry::Uninspected,
                placeholder: Some(SessionTreePlaceholder::Uninspected),
            },
            workspace: None,
        }
    }

    pub(super) fn unavailable(session: StoredSession, detail: String) -> Self {
        Self {
            node: SessionTreeNode {
                session_id: session.session_id(),
                metadata: Some(session),
                depth: 0,
                ancestry: SessionTreeAncestry::Invalid { detail },
                placeholder: Some(SessionTreePlaceholder::Unavailable),
            },
            workspace: None,
        }
    }

    pub(super) fn inspect(session: StoredSession, entries: &[RepositoryEntry]) -> Self {
        let session_id = session.session_id();
        let validated = recover_entries(session_id, entries)
            .map_err(|error| RepositoryError::Unavailable {
                message: error.to_string(),
            })
            .and_then(|recovered| {
                let descriptor =
                    recovered
                        .descriptor()
                        .ok_or_else(|| RepositoryError::Unavailable {
                            message: "Session tree candidate has no durable descriptor".into(),
                        })?;
                if validate_discovery(entries, descriptor, &recovered)
                    != StoredDiscoveryValidation::Consistent
                {
                    return Err(RepositoryError::Unavailable {
                        message:
                            "Session tree discovery metadata disagrees with semantic authority"
                                .into(),
                    });
                }
                let workspace = (
                    descriptor.workspace_host_id(),
                    descriptor.workspace_path().clone(),
                );
                let ancestry = recovered
                    .records()
                    .iter()
                    .find_map(|entry| match entry.record() {
                        JournalRecord::InitialForkSeed(seed) => {
                            let source = match seed.source() {
                                ForkSource::Empty => InheritedHistorySource::Empty,
                                ForkSource::Anchor(point) => InheritedHistorySource::Anchor {
                                    record_sequence: point.record_sequence(),
                                    journal_boundary: point.journal_boundary(),
                                },
                                ForkSource::Checkpoint(point) => {
                                    InheritedHistorySource::Checkpoint {
                                        record_sequence: point.record_sequence(),
                                        journal_boundary: point.journal_boundary(),
                                    }
                                },
                                ForkSource::InitialFork(point) => {
                                    InheritedHistorySource::InitialFork {
                                        record_sequence: point.record_sequence(),
                                        journal_boundary: point.journal_boundary(),
                                    }
                                },
                            };
                            Some(SessionTreeAncestry::ValidatedFork {
                                parent_session_id: seed.parent_session_id(),
                                source,
                            })
                        },
                        _ => None,
                    })
                    .unwrap_or(SessionTreeAncestry::UnknownLegacy);
                Ok((workspace, ancestry))
            });
        match validated {
            Ok((workspace, ancestry)) => Self {
                node: SessionTreeNode {
                    session_id,
                    metadata: Some(session),
                    depth: 0,
                    ancestry,
                    placeholder: None,
                },
                workspace: Some(workspace),
            },
            Err(error) => {
                let detail = error.to_string();
                Self::unavailable(
                    StoredSession::Unavailable {
                        session_id,
                        reason: StoredSessionUnavailableReason::Corrupt {
                            message: detail.clone(),
                        },
                    },
                    detail,
                )
            },
        }
    }
}

pub(super) fn assemble(
    candidates: Vec<TreeCandidate>,
    workspace_host: WorkspaceHostId,
    workspace: &HostWorkspacePath,
    truncated: bool,
    mut parent_presence: impl FnMut(SessionId) -> SessionTreePlaceholder,
) -> StoredSessionTree {
    let mut outside = BTreeMap::new();
    let mut nodes = BTreeMap::new();
    for candidate in candidates {
        if candidate
            .workspace
            .as_ref()
            .is_some_and(|(host, path)| *host != workspace_host || path != workspace)
        {
            outside.insert(candidate.node.session_id, candidate.node);
        } else {
            nodes.insert(candidate.node.session_id, candidate.node);
        }
    }
    let parents: BTreeSet<_> = nodes
        .values()
        .filter_map(SessionTreeNode::parent_session_id)
        .collect();
    for parent in parents {
        if nodes.contains_key(&parent) {
            continue;
        }
        let (metadata, placeholder) = match outside.remove(&parent) {
            Some(node) => (node.metadata, SessionTreePlaceholder::OutsideWorkspace),
            None => (None, parent_presence(parent)),
        };
        nodes.insert(
            parent,
            SessionTreeNode {
                session_id: parent,
                metadata,
                depth: 0,
                ancestry: SessionTreeAncestry::Uninspected,
                placeholder: Some(placeholder),
            },
        );
    }
    // Each walk is bounded by the collected graph; a cycle never becomes a false lineage.
    let mut cycles = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for start in nodes.keys() {
        if visited.contains(start) {
            continue;
        }
        let mut path = Vec::new();
        let mut positions = BTreeMap::new();
        let mut next = Some(*start);
        while let Some(id) = next {
            if let Some(index) = positions.get(&id).copied() {
                cycles.extend(path[index..].iter().copied());
                break;
            }
            if visited.contains(&id) {
                break;
            }
            positions.insert(id, path.len());
            path.push(id);
            next = nodes.get(&id).and_then(SessionTreeNode::parent_session_id);
        }
        visited.extend(path);
    }
    for id in cycles {
        let node = nodes
            .get_mut(&id)
            .expect("cycle belongs to collected graph");
        node.ancestry = SessionTreeAncestry::Invalid {
            detail: "validated Session fork provenance forms a cycle".into(),
        };
        node.placeholder = Some(SessionTreePlaceholder::Unavailable);
        node.metadata = Some(StoredSession::Unavailable {
            session_id: id,
            reason: StoredSessionUnavailableReason::Corrupt {
                message: "validated Session fork provenance forms a cycle".into(),
            },
        });
    }
    let mut children: BTreeMap<Option<SessionId>, Vec<SessionId>> = BTreeMap::new();
    for node in nodes.values() {
        children
            .entry(node.parent_session_id())
            .or_default()
            .push(node.session_id);
    }
    let mut stack = children
        .remove(&None)
        .unwrap_or_default()
        .into_iter()
        .rev()
        .map(|id| (id, 0))
        .collect::<Vec<_>>();
    let mut ordered = Vec::with_capacity(nodes.len());
    while let Some((id, depth)) = stack.pop() {
        if let Some(mut node) = nodes.remove(&id) {
            node.depth = depth;
            ordered.push(node);
            if let Some(descendants) = children.remove(&Some(id)) {
                stack.extend(
                    descendants
                        .into_iter()
                        .rev()
                        .map(|child| (child, depth + 1)),
                );
            }
        }
    }
    StoredSessionTree {
        nodes: ordered,
        truncated,
    }
}

#[cfg(test)]
mod tests;
