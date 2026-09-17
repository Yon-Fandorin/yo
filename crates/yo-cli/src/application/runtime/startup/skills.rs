use std::path::Path;

use yo_core::{
    InputAdmissionHost, LocalSkillInputAdmission, LocalSkillReferenceProvider, LocalSkillRoot,
    LocalWorkspaceInputAdmission, SkillReferenceProvider, WorkspaceHostId,
};

use super::model::StartupFrontend;
use crate::state::config;

pub(in crate::application::runtime) struct PreparedLocalSkills {
    pub(in crate::application::runtime) admission: Box<dyn InputAdmissionHost>,
    pub(in crate::application::runtime) references: Option<Box<dyn SkillReferenceProvider>>,
}

// 명시한 skill root는 선택한 Session workspace를 기준으로 해석합니다.
// Skill instruction은 backend의 tool이나 sandbox capability를 바꾸지 않습니다.
pub(in crate::application::runtime) fn prepare_local_skills(
    config: &config::Config,
    workspace: &Path,
    host: WorkspaceHostId,
    frontend: StartupFrontend,
) -> Result<PreparedLocalSkills, String> {
    if config.skill_roots().is_empty() {
        return Ok(PreparedLocalSkills {
            admission: Box::new(LocalWorkspaceInputAdmission::new(workspace, host)?),
            references: None,
        });
    }
    let roots = config
        .skill_roots()
        .iter()
        .map(|root| {
            let path = if root.path.is_absolute() {
                root.path.clone()
            } else {
                workspace.join(&root.path)
            };
            LocalSkillRoot::new(path, root.scope)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let admission = LocalSkillInputAdmission::new(workspace, roots.clone(), host)?;
    let references = if matches!(frontend, StartupFrontend::Terminal) {
        Some(Box::new(LocalSkillReferenceProvider::start(roots, host)?)
            as Box<dyn SkillReferenceProvider>)
    } else {
        None
    };
    Ok(PreparedLocalSkills {
        admission: Box::new(admission),
        references,
    })
}
