mod failure;
mod model;
mod prepare;
mod skills;

pub(super) use failure::{
    complete_with_read_only_resume, handle_launch_failure, require_exact_print_resume_binding,
};
pub(super) use model::{PreparedAgent, StartupFrontend, StartupOutcome, StartupSnapshots};
pub(super) use prepare::{
    fork_descriptor, prepare_agent, prepare_fork_agent, prepare_new_agent,
    require_exact_fork_selection, require_supported_fork_binding,
};
pub(super) use skills::{PreparedLocalSkills, prepare_local_skills};

impl<'a> StartupSnapshots<'a> {
    pub(super) fn new(
        config: &'a crate::state::config::Config,
        credentials: &'a mut Option<yo_core::CredentialSnapshot>,
        stored_preference: Option<&'a yo_core::StartupTarget>,
        codex_warnings: &'a crate::application::codex_diagnostics::CodexWarningCollector,
    ) -> Self {
        Self {
            config,
            credentials,
            stored_preference,
            codex_warnings,
        }
    }
}

#[cfg(test)]
mod tests;
