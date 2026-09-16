use std::path::Path;

use super::{
    super::SkillAvailability, catalog::Catalog, reject, roots::LocalSkillRoot,
    snapshot::read_snapshot,
};
use crate::{
    InputAdmissionHost, InputReference, LocalWorkspaceInputAdmission, ResolvedSkill,
    SubmissionRejection, SubmissionRejectionKind, UserInput, WorkspaceHostId,
};

/// Local workspace and explicit-skill admission; no model/provider-specific interpretation.
pub struct LocalSkillInputAdmission {
    workspace: LocalWorkspaceInputAdmission,
    catalog: Catalog,
}

impl LocalSkillInputAdmission {
    /// Binds only explicitly supplied source roots to one local execution host.
    pub fn new(
        workspace: &Path,
        roots: Vec<LocalSkillRoot>,
        host: WorkspaceHostId,
    ) -> Result<Self, String> {
        Ok(Self {
            workspace: LocalWorkspaceInputAdmission::new(workspace, host)?,
            catalog: Catalog::new(roots, host)?,
        })
    }
}

impl InputAdmissionHost for LocalSkillInputAdmission {
    fn validate(&self, input: &UserInput) -> Result<(), SubmissionRejection> {
        if input.references().len() > 128 {
            return Err(reject(
                SubmissionRejectionKind::OverBudget,
                "at most 128 input references are supported",
            ));
        }
        let workspace = UserInput::with_references(
            input.as_str(),
            input
                .references()
                .iter()
                .filter(|reference| reference.workspace_reference().is_some())
                .cloned()
                .collect(),
        )
        .map_err(|error| reject(SubmissionRejectionKind::InvalidReference, error.to_string()))?;
        self.workspace.validate(&workspace)?;
        if let Some(selected) = input
            .references()
            .iter()
            .find_map(InputReference::skill_reference)
        {
            self.catalog.select(selected)?;
        }
        Ok(())
    }

    fn prepare(&self, input: &UserInput) -> Result<Option<ResolvedSkill>, SubmissionRejection> {
        self.validate(input)?;
        let Some(selected) = input
            .references()
            .iter()
            .find_map(InputReference::skill_reference)
        else {
            return Ok(None);
        };
        let (root, child) = self.catalog.select(selected)?;
        let snapshot = read_snapshot(root, &child, ResolvedSkill::MAX_INSTRUCTION_BYTES)?;
        let expected = self.catalog.reference(
            root,
            &child,
            &snapshot.name,
            selected.catalog_generation(),
            &snapshot.revision,
        );
        if &expected != selected {
            return Err(reject(
                SubmissionRejectionKind::StaleReference,
                "selected skill changed; select it again",
            ));
        }
        if let SkillAvailability::Disabled(reason) = snapshot.availability {
            return Err(reject(SubmissionRejectionKind::Unauthorized, reason));
        }
        ResolvedSkill::new(selected.clone(), snapshot.text)
            .map(Some)
            .map_err(|error| {
                reject(
                    SubmissionRejectionKind::RequiredAssetUnavailable,
                    error.to_string(),
                )
            })
    }
}
