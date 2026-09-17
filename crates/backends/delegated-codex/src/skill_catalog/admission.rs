//! Codex 작업공간과 명시적 skill 선택을 제출 시점에 검증합니다.

use yo_core::{
    InputAdmissionHost, InputReference, LocalWorkspaceInputAdmission, ResolvedSkill,
    SkillAvailability, SkillReference, SubmissionRejection, SubmissionRejectionKind, UserInput,
    WorkspaceHostId,
};

use super::{
    catalog::{SkillMetadata, candidate_from_wire, load_skill_metadata},
    resolution::resolve_selected_skill,
};
use crate::{CodexBackendConfig, CodexWarningObserver};

/// Codex의 authoritative skill catalog을 사용하는 실행 호스트 입력 admission입니다.
/// Runtime은 provider-neutral `InputAdmissionHost` port를 통해 이 타입을 사용합니다.
pub struct CodexSkillInputAdmission {
    config: CodexBackendConfig,
    workspace_host_id: WorkspaceHostId,
    workspace: LocalWorkspaceInputAdmission,
    warning_observer: Option<CodexWarningObserver>,
}

impl CodexSkillInputAdmission {
    /// 검색과 같은 작업공간 설정으로 admission을 고정합니다.
    pub fn new(
        config: CodexBackendConfig,
        workspace_host_id: WorkspaceHostId,
        warning_observer: Option<CodexWarningObserver>,
    ) -> Result<Self, String> {
        let workspace =
            LocalWorkspaceInputAdmission::new(config.working_directory(), workspace_host_id)?;
        Ok(Self {
            config,
            workspace_host_id,
            workspace,
            warning_observer,
        })
    }

    fn validate_workspace(&self, input: &UserInput) -> Result<(), SubmissionRejection> {
        if input.references().len() > 128 {
            return Err(SubmissionRejection::new(
                SubmissionRejectionKind::OverBudget,
                "at most 128 input references are supported",
            ));
        }
        let workspace_input = UserInput::with_references(
            input.as_str(),
            input
                .references()
                .iter()
                .filter(|reference| reference.workspace_reference().is_some())
                .cloned()
                .collect(),
        )
        .map_err(|error| {
            SubmissionRejection::new(SubmissionRejectionKind::InvalidReference, error.to_string())
        })?;
        self.workspace.validate(&workspace_input)
    }

    fn selected(&self, input: &UserInput) -> Result<Option<SkillReference>, SubmissionRejection> {
        self.validate_workspace(input)?;
        let Some(selected) = input
            .references()
            .iter()
            .find_map(InputReference::skill_reference)
        else {
            return Ok(None);
        };
        let environment = format!("local-host:{}", self.workspace_host_id);
        if selected.execution_environment_identity() != environment {
            return Err(SubmissionRejection::new(
                SubmissionRejectionKind::EnvironmentUnavailable,
                "skill belongs to another execution host",
            ));
        }
        let catalog =
            load_skill_metadata(&self.config, self.warning_observer.clone()).map_err(|error| {
                SubmissionRejection::new(SubmissionRejectionKind::EnvironmentUnavailable, error)
            })?;
        validate_selected_skill(&environment, selected, catalog.skills)?;
        Ok(Some(selected.clone()))
    }
}

impl InputAdmissionHost for CodexSkillInputAdmission {
    fn validate(&self, input: &UserInput) -> Result<(), SubmissionRejection> {
        self.selected(input).map(|_| ())
    }

    fn prepare(&self, input: &UserInput) -> Result<Option<ResolvedSkill>, SubmissionRejection> {
        let Some(selected) = self.selected(input)? else {
            return Ok(None);
        };
        resolve_selected_skill(selected).map(Some)
    }
}

pub(super) fn validate_selected_skill(
    environment: &str,
    selected: &SkillReference,
    skills: Vec<SkillMetadata>,
) -> Result<(), SubmissionRejection> {
    let mut matching = skills
        .into_iter()
        .filter(|skill| skill.path == selected.locator());
    let Some(skill) = matching.next() else {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::StaleReference,
            "selected skill was removed; select it again",
        ));
    };
    if matching.next().is_some() {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::InvalidReference,
            "skill catalog contains an ambiguous locator",
        ));
    }
    // Codex 명시적 선택은 enabled를 사용하며 allow_implicit_invocation과는 무관합니다.
    let candidate = candidate_from_wire(
        environment,
        skill,
        selected.catalog_generation(),
        Ok(selected.entry_revision().to_owned()),
    );
    if candidate.reference() != selected {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::StaleReference,
            "selected skill descriptor changed; select it again",
        ));
    }
    if let SkillAvailability::Disabled(reason) = candidate.availability() {
        return Err(SubmissionRejection::new(
            SubmissionRejectionKind::Unauthorized,
            reason,
        ));
    }
    Ok(())
}
