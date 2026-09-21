mod failure;
mod model;
mod prepare;
mod skills;

#[cfg(test)]
pub(super) use failure::require_exact_print_resume_binding;
pub(super) use model::{PreparedAgent, StartupFrontend, StartupOutcome, StartupSnapshots};
#[cfg(test)]
pub(super) use prepare::{
    fork_descriptor, require_exact_fork_selection, require_supported_fork_binding,
    restored_notification_cutoff, resume_prompt_history,
};
pub(super) use prepare::{prepare_agent, prepare_fork_agent, prepare_new_agent};
#[cfg(test)]
pub(super) use skills::{PreparedLocalSkills, prepare_local_skills};

use crate::{application::codex_diagnostics::CodexWarningCollector, state::config::Config};

impl<'a> StartupSnapshots<'a> {
    pub(super) fn new(
        config: &'a Config,
        credentials: &'a mut Option<yo_core::CredentialSnapshot>,
        stored_preference: Option<&'a yo_core::StartupTarget>,
        codex_warnings: &'a CodexWarningCollector,
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
