use yo_core::{
    SessionId,
    session_repository::{
        ContinuationEligibility, InheritedHistorySource, InheritedSessionHistory,
        SessionTreeAncestry, SessionTreePlaceholder, StoredSessionForkCatalog,
        StoredSessionForkSourceKind, StoredSessionTree,
    },
};

use super::{
    super::{ForkPickerToken, archival::ArchivedProjectionError},
    TuiSession,
    metadata::ResumeSessionEntry,
};
use crate::overlay::{PanelSnapshot, SelectionEntry};

impl TuiSession {
    /// 부모 실행 상태를 복원하지 않고 자식이 소유한 상속 프레젠테이션을 설치합니다.
    /// 자식 Session 레코드를 관찰하기 전 생성 과정에서 한 번 호출합니다.
    /// 원본 프레젠테이션이 끝나지 않았거나 이미 설치했다면 오류를 반환합니다.
    pub fn with_inherited_history(
        mut self,
        history: &InheritedSessionHistory,
    ) -> Result<Self, ArchivedProjectionError> {
        self.state
            .observe_inherited_history(history)
            .map_err(|error| ArchivedProjectionError {
                detail: format!("projecting inherited history failed: {error:?}"),
            })?;
        Ok(self)
    }
    /// 호스트가 발견한 저장 세션 선택기를 표시합니다. 선택 항목은 재개 전에 호스트가 다시
    /// 검증합니다.
    pub fn show_resume_picker(
        &mut self,
        sessions: Vec<ResumeSessionEntry>,
        has_more: bool,
    ) -> Result<(), String> {
        let mut entries = sessions
            .into_iter()
            .map(|session| {
                let id = session.session_id.to_string();
                if session.eligibility == ContinuationEligibility::Unavailable {
                    SelectionEntry::status(
                        id.clone(),
                        format!("Updated {} · {id} · unavailable", session.updated_label),
                    )
                } else {
                    SelectionEntry::enabled_with_context(
                        id.clone(),
                        format!("Updated {}", session.updated_label),
                        Some(id),
                        Some(
                            if session.eligibility == ContinuationEligibility::Eligible {
                                "Saved session · revalidated before resuming"
                            } else {
                                "Eligibility unknown · checked before resuming"
                            }
                            .to_owned(),
                        ),
                    )
                }
            })
            .collect::<Vec<_>>();
        if has_more {
            entries.push(SelectionEntry::status(
                "more",
                "More sessions: use yo session, then /resume UUID",
            ));
        }
        let panel = PanelSnapshot::new("Resume saved session", entries)
            .map_err(|error| format!("saved-session picker: {error:?}"))?;
        self.state
            .show_resume_picker(panel)
            .map_err(|error| format!("saved-session picker: {error:?}"))
    }

    /// 현재 대화를 버리지 않고 재개 준비 실패를 보고합니다.
    pub fn report_resume_failure(&mut self, detail: impl Into<String>) {
        self.state.report_resume_failure(detail.into());
    }

    /// 저장 세션을 선택한 뒤 정리 작업의 실패를 보고합니다.
    pub fn report_resume_cleanup_failure(&mut self, detail: impl Into<String>) {
        self.state.report_resume_cleanup_failure(detail.into());
    }

    pub(in crate::runner) fn take_resume_session_request(&mut self) -> Option<Option<SessionId>> {
        self.state.take_resume_session_request()
    }

    pub(in crate::runner) fn take_session_tree_request(&mut self) -> bool {
        self.state.take_session_tree_request()
    }

    /// 선택한 Session을 바꾸지 않고 읽기 전용 트리 조회 실패를 보고합니다.
    pub fn report_session_tree_failure(&mut self, detail: impl Into<String>) {
        self.state.report_session_tree_failure(detail.into());
    }

    /// 상위 노드가 먼저 오는 코어의 검증된 포리스트를 조상 관계를 추론하지 않고 표시합니다.
    pub fn show_session_tree(
        &mut self,
        tree: &StoredSessionTree,
        current: SessionId,
    ) -> Result<(), String> {
        let mut entries = Vec::new();
        for node in tree.nodes() {
            let id = node.session_id().to_string();
            let depth = node.depth();
            let branch = if depth == 0 {
                String::new()
            } else {
                format!("{}+- ", "  ".repeat(depth.min(8) - 1))
            };
            let mut relation = match node.ancestry() {
                SessionTreeAncestry::UnknownLegacy => {
                    "Ancestry unknown · no fork provenance".to_owned()
                },
                SessionTreeAncestry::Uninspected => "Ancestry not inspected".to_owned(),
                SessionTreeAncestry::Invalid { .. } => {
                    "Ancestry unavailable · validation failed".to_owned()
                },
                SessionTreeAncestry::ValidatedFork {
                    parent_session_id,
                    source,
                } => {
                    let source = match source {
                        InheritedHistorySource::Empty => "explicit empty source".to_owned(),
                        InheritedHistorySource::Anchor {
                            record_sequence, ..
                        } => format!("anchor {}", record_sequence.get()),
                        InheritedHistorySource::Checkpoint {
                            record_sequence, ..
                        } => format!("checkpoint {}", record_sequence.get()),
                        InheritedHistorySource::InitialFork {
                            record_sequence, ..
                        } => format!("initial fork {}", record_sequence.get()),
                    };
                    format!("Parent {parent_session_id} · {source}")
                },
            };
            if let Some(parent) = node.parent_session_id()
                && let Some(parent_node) = tree
                    .nodes()
                    .iter()
                    .find(|entry| entry.session_id() == parent)
                && let Some(placeholder) = parent_node.placeholder()
            {
                relation.push_str(match placeholder {
                    SessionTreePlaceholder::MissingAncestor => " · ancestor missing",
                    SessionTreePlaceholder::OutsideWorkspace => " · ancestor outside workspace",
                    SessionTreePlaceholder::Unavailable => " · ancestor unavailable",
                    SessionTreePlaceholder::Uninspected => " · ancestor not inspected",
                });
            }
            let status = match node.placeholder() {
                Some(SessionTreePlaceholder::MissingAncestor) => {
                    Some("Ancestor unavailable · missing")
                },
                Some(SessionTreePlaceholder::OutsideWorkspace) => {
                    Some("Ancestor outside this workspace")
                },
                Some(SessionTreePlaceholder::Unavailable) => {
                    Some("Session unavailable · workspace unverified")
                },
                Some(SessionTreePlaceholder::Uninspected) => {
                    Some("Not inspected · workspace unverified")
                },
                None if node.session_id() == current => Some("Current session"),
                None if node.metadata().is_none_or(|metadata| {
                    metadata.continuation_eligibility() == ContinuationEligibility::Unavailable
                }) =>
                {
                    Some("Continuation unavailable")
                },
                None => None,
            };
            let label = format!("{branch}{id} · depth {depth}");
            if let Some(status) = status {
                entries.push(SelectionEntry::status(
                    id,
                    format!("{label} · {status} · {relation}"),
                ));
            } else {
                entries.push(SelectionEntry::enabled_with_context(
                    id.clone(),
                    label,
                    Some(id),
                    Some(relation),
                ));
            }
        }
        if tree.truncated() {
            entries.push(SelectionEntry::status(
                "tree-limit",
                "Query limit reached · uninspected ancestry remains unknown",
            ));
        }
        if entries.is_empty() {
            entries.push(SelectionEntry::status(
                "tree-empty",
                "No sessions in this workspace",
            ));
        }
        let panel = PanelSnapshot::new("Session tree · read-only", entries)
            .map_err(|error| format!("session tree panel: {error:?}"))?
            .with_wrapped_entries();
        self.state
            .show_resume_picker(panel)
            .map_err(|error| format!("session tree panel: {error:?}"))
    }

    pub(in crate::runner) fn take_fork_session_request(&mut self) -> bool {
        self.state.take_fork_session_request()
    }

    pub(in crate::runner) fn take_fork_picker_request(&mut self) -> bool {
        self.state.take_fork_picker_request()
    }

    pub(in crate::runner) fn take_fork_boundary_request(
        &mut self,
    ) -> Option<(ForkPickerToken, usize)> {
        self.state.take_fork_boundary_request()
    }

    /// 호스트가 고정한 최신순 카탈로그를 표시하고 선택기 식별자를 반환합니다.
    /// 호스트는 카탈로그를 보관하며 선택한 행을 해석하기 전에 이 토큰을 검증합니다.
    pub fn show_fork_picker(
        &mut self,
        catalog: &StoredSessionForkCatalog,
    ) -> Result<ForkPickerToken, String> {
        let result = (|| {
            let mut entries = catalog
            .boundaries()
            .iter()
            .enumerate()
            .map(|(index, boundary)| {
                let source = match boundary.source_kind() {
                    StoredSessionForkSourceKind::Anchor => "Completed turn",
                    StoredSessionForkSourceKind::Checkpoint => "Compacted context",
                    StoredSessionForkSourceKind::InitialFork => "Inherited starting point",
                };
                let label = boundary.input_excerpt().map_or_else(
                    || format!("{}. {source}", index + 1),
                    |excerpt| format!("{}. {source} · {excerpt}", index + 1),
                );
                SelectionEntry::enabled_with_context(
                    index.to_string(),
                    label,
                    Some(boundary.model_label().to_owned()),
                    Some("Continue from this point; later messages stay in the parent conversation.".to_owned()),
                )
            })
            .collect::<Vec<_>>();
            if entries.is_empty() {
                entries.push(SelectionEntry::status(
                    "empty",
                    "No historical fork boundaries are available",
                ));
            }
            if catalog.truncated() {
                entries.push(SelectionEntry::status(
                    "truncated",
                    "Older boundaries were omitted from this bounded list",
                ));
            }
            let panel = PanelSnapshot::new("Fork from an earlier point", entries)
                .map_err(|error| format!("fork picker: {error:?}"))?
                .with_wrapped_entries();
            self.state
                .show_fork_picker(panel, catalog.boundaries().len())
        })();
        if result.is_err() {
            self.state.cancel_fork_picker();
        }
        result
    }

    /// 준비된 자식을 선택한 뒤 원본 Session을 식별합니다.
    pub fn report_fork_started(&mut self, parent: SessionId) {
        self.state.report_fork_started(parent);
    }

    /// 선택한 부모를 유지하면서 fork 준비 실패를 보고합니다.
    pub fn report_fork_failure(&mut self, detail: impl Into<String>) {
        self.state.report_fork_failure(detail.into());
    }

    /// 자식을 선택한 뒤 이전 backend 정리 작업의 실패를 보고합니다.
    pub fn report_fork_cleanup_failure(&mut self, detail: impl Into<String>) {
        self.state.report_fork_cleanup_failure(detail.into());
    }

    pub(in crate::runner) fn take_new_session_request(&mut self) -> bool {
        self.state.take_new_session_request()
    }

    /// 후속 세션을 선택한 뒤 이전 세션을 해제하지 못한 실패를 보고합니다.
    pub fn report_new_session_cleanup_failure(&mut self, detail: impl Into<String>) {
        self.state.report_new_session_cleanup_failure(detail.into());
    }

    /// 현재 대화를 유지하면서 새 세션 준비 실패를 보고합니다.
    pub fn report_new_session_failure(&mut self, detail: impl Into<String>) {
        self.state.report_new_session_failure(detail.into());
    }

    pub(in crate::runner) fn take_model_selection(&mut self) -> Option<yo_core::ModelPickerTarget> {
        self.state.take_model_selection()
    }

    /// 실행 중인 Session을 버리지 않고 호스트 측 모델 준비 실패를 보고합니다.
    pub fn report_model_switch_failure(&mut self, detail: impl Into<String>) {
        self.state.report_model_switch_failure(detail.into());
    }

    /// 코어 Session의 바인딩이 영구적으로 바뀐 뒤 프런트엔드 투영을 커밋합니다.
    pub fn commit_model_switch(
        &mut self,
        controller: yo_core::ModelSelectionController,
        backend_label: impl Into<String>,
        cleanup_warning: Option<String>,
    ) {
        self.state
            .commit_model_switch(controller, backend_label.into(), cleanup_warning);
    }
}
