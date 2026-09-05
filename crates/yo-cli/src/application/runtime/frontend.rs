use super::{
    super::{
        codex_diagnostics::{CodexWarningCollector, publish_pending_codex_diagnostics},
        output::write_session_output,
    },
    LiveSession, PreparedAgent, SessionStep,
};
use crate::{command, config, diagnostic::AppError, model};

pub(super) fn build_live_session(
    prepared: PreparedAgent,
    config: &config::Config,
    options: &command::LiveOptions,
) -> LiveSession {
    let PreparedAgent {
        agent,
        workspace,
        workspace_references,
        skill_references,
        selection,
        local_tool_registry,
        active_host,
        active_host_execution,
        active_host_model,
        host_catalogs,
    } = prepared;
    let mut tui = yo_tui::TuiSession::with_session_info(
        options.glyph_profile,
        yo_tui::TuiSessionInfo::new(selection.label(), compact_workspace_label(&workspace)),
        terminal_color_capability(),
        yo_tui::MotionPreference::Standard,
    )
    .with_frame_rate_limit(config.frame_rate_limit())
    .with_workspace_references(
        workspace_references.expect("the terminal frontend started workspace references"),
    );
    if let Some(skill_references) = skill_references {
        tui = tui.with_skill_references(skill_references);
    }
    let model_controller = model::project_host_catalogs(
        yo_core::ModelSelectionController::new(
            config.model_catalog().clone(),
            selection.model_selection(),
        ),
        active_host_model.as_ref(),
        &host_catalogs,
    );
    if !model_controller.sections().is_empty() {
        tui = tui.with_model_selection(model_controller);
    }
    LiveSession {
        agent,
        tui,
        workspace,
        local_tool_registry,
        active_host,
        active_host_execution,
        active_host_model,
        host_catalogs,
    }
}

pub(super) fn run_terminal_generation(
    termination: &mut impl yo_tui::TerminationSource,
    live: &mut Option<LiveSession>,
    config: &config::Config,
    credentials: &mut Option<yo_core::CredentialSnapshot>,
    codex_warnings: &CodexWarningCollector,
    options: command::LiveOptions,
) -> Result<SessionStep, AppError> {
    let codex_warning_observer = codex_warnings.observer();

    publish_pending_codex_diagnostics(codex_warnings)?;
    let session = live
        .as_mut()
        .expect("live session is initialized before terminal acquisition");
    let terminal = yo_tui::run_session_with_mode(
        termination,
        &mut session.agent,
        &mut session.tui,
        options.mode,
    );

    let mut errors = Vec::<AppError>::new();
    match terminal {
        Ok(yo_tui::TerminalOutcome::SuspendRequested) => return Ok(SessionStep::Suspend),
        Ok(yo_tui::TerminalOutcome::ModelSelectionRequested(
            yo_core::ModelPickerTarget::Managed(selection),
        )) => {
            if let Some(host) = session.active_host.as_ref() {
                session.tui.report_model_switch_failure(format!(
                    "switching from host:{} to managed model {} requires the semantic-handoff transition",
                    host.as_str(),
                    selection.model()
                ));
                return Ok(SessionStep::Continue);
            }
            let replacement = model::replacement(
                &selection,
                session
                    .local_tool_registry
                    .expect("only a live native Session exposes model selection"),
            );
            match model::start_native(
                config,
                credentials
                    .as_ref()
                    .expect("a live native Session retained its credential snapshot"),
                &replacement,
                &session.workspace,
            ) {
                Ok(backend) => match session.agent.replace_backend(backend, termination) {
                    Ok(outcome) => {
                        let cleanup_warning = outcome.cleanup_failure().map(ToString::to_string);
                        let label = selection.model().to_string();
                        let model_controller = model::project_host_catalogs(
                            yo_core::ModelSelectionController::new(
                                config.model_catalog().clone(),
                                Some(selection),
                            ),
                            None,
                            &session.host_catalogs,
                        );
                        session
                            .tui
                            .commit_model_switch(model_controller, label, cleanup_warning);
                        return Ok(SessionStep::Continue);
                    },
                    Err(error) => {
                        session.tui.report_model_switch_failure(error.to_string());
                        return Ok(SessionStep::Continue);
                    },
                },
                Err(error) => {
                    session.tui.report_model_switch_failure(error.to_string());
                    return Ok(SessionStep::Continue);
                },
            }
        },
        Ok(yo_tui::TerminalOutcome::ModelSelectionRequested(yo_core::ModelPickerTarget::Host(
            selection,
        ))) => {
            let Some(active) = session.active_host_model.as_ref() else {
                session.tui.report_model_switch_failure(format!(
                    "switching to host:{} model {} requires the semantic-handoff transition",
                    selection.host().as_str(),
                    selection.model()
                ));
                return Ok(SessionStep::Continue);
            };
            if active.host() != selection.host() || active.account() != selection.account() {
                session.tui.report_model_switch_failure(
                    "the selected host or account differs from the active Session".to_owned(),
                );
                return Ok(SessionStep::Continue);
            }
            if active.model() == selection.model() {
                return Ok(SessionStep::Continue);
            }
            if !active.supports_native_model_rebind() {
                session.tui.report_model_switch_failure(
                    "the active host does not advertise state-preserving model switching"
                        .to_owned(),
                );
                return Ok(SessionStep::Continue);
            }

            let refreshed = model::read_builtin_host_catalogs_with_codex_warning_observer(
                &session.workspace,
                session
                    .active_host
                    .as_ref()
                    .zip(session.active_host_execution),
                Some(codex_warning_observer.clone()),
            );
            let admission = refreshed
                .iter()
                .find(|observation| observation.host() == selection.host())
                .ok_or_else(|| "the selected host inventory is absent".to_owned())
                .and_then(|observation| observation.catalog().map_err(str::to_owned))
                .and_then(|catalog| {
                    if catalog.account() != selection.account() {
                        return Err(
                            "the authenticated host account changed before selection".to_owned()
                        );
                    }
                    let model = catalog
                        .models()
                        .iter()
                        .find(|model| model.id() == selection.model())
                        .ok_or_else(|| {
                            "the selected model is absent from the refreshed host inventory"
                                .to_owned()
                        })?;
                    if !model.is_selectable() {
                        return Err(model
                            .unavailable_reason()
                            .unwrap_or("the selected model is unavailable")
                            .to_owned());
                    }
                    Ok(())
                });
            session.host_catalogs = refreshed;
            if let Err(error) = admission {
                session.tui.report_model_switch_failure(error);
                return Ok(SessionStep::Continue);
            }
            if selection.host().as_str() != yo_core::HostId::CODEX {
                session.tui.report_model_switch_failure(
                    "this host has no native model-rebind adapter".to_owned(),
                );
                return Ok(SessionStep::Continue);
            }
            let candidate_config =
                yo_backend_delegated_codex::CodexBackendConfig::new(&session.workspace)
                    .with_read_only_review(
                        session
                            .active_host_execution
                            .is_some_and(model::DelegatedExecutionProfile::is_read_only_review),
                    )
                    .with_model_rebind_target(
                        selection.account().clone(),
                        selection.model().clone(),
                    );
            let backend =
                match yo_backend_delegated_codex::CodexBackend::spawn_with_warning_observer(
                    candidate_config,
                    Some(codex_warning_observer.clone()),
                ) {
                    Ok(backend) => Box::new(backend) as Box<dyn yo_core::AgentBackend + Send>,
                    Err(error) => {
                        session.tui.report_model_switch_failure(error.to_string());
                        return Ok(SessionStep::Continue);
                    },
                };
            match session.agent.replace_backend(backend, termination) {
                Ok(outcome) => {
                    session
                        .active_host_model
                        .as_mut()
                        .expect("admitted host rebind retained active host state")
                        .set_model(selection.model().clone());
                    let controller = model::project_host_catalogs(
                        yo_core::ModelSelectionController::new(
                            config.model_catalog().clone(),
                            None,
                        ),
                        session.active_host_model.as_ref(),
                        &session.host_catalogs,
                    );
                    session.tui.commit_model_switch(
                        controller,
                        selection.model().to_string(),
                        outcome.cleanup_failure().map(ToString::to_string),
                    );
                    return Ok(SessionStep::Continue);
                },
                Err(error) => {
                    session.tui.report_model_switch_failure(error.to_string());
                    return Ok(SessionStep::Continue);
                },
            }
        },
        Ok(yo_tui::TerminalOutcome::Exited(outcome)) => {
            if let Some(output) = outcome.output()
                && let Err(error) = write_session_output(output)
            {
                codex_warnings.discard_pending();
                errors.push(AppError::message(format!(
                    "writing session output: {error}"
                )));
            }
        },
        Ok(_) => errors.push(AppError::message(
            "terminal session: unsupported terminal outcome",
        )),
        Err(error) => errors.push(AppError::message(format!("terminal session: {error}"))),
    }
    if let Err(error) = super::shutdown_live_session(live) {
        errors.push(error);
    }
    if errors.is_empty() {
        Ok(SessionStep::Complete)
    } else {
        Err(AppError::combine(errors))
    }
}

fn terminal_color_capability() -> yo_tui::ColorCapability {
    classify_terminal_color_capability(
        std::env::var_os("COLORTERM")
            .as_deref()
            .and_then(std::ffi::OsStr::to_str),
        std::env::var_os("TERM")
            .as_deref()
            .and_then(std::ffi::OsStr::to_str),
        std::env::var_os("NO_COLOR").is_some(),
    )
}

fn classify_terminal_color_capability(
    color_term: Option<&str>,
    term: Option<&str>,
    no_color: bool,
) -> yo_tui::ColorCapability {
    if no_color {
        return yo_tui::ColorCapability::Unknown;
    }
    if color_term.is_some_and(|value| {
        value.eq_ignore_ascii_case("truecolor") || value.eq_ignore_ascii_case("24bit")
    }) {
        return yo_tui::ColorCapability::TrueColor;
    }
    if term.is_some_and(|value| value.to_ascii_lowercase().contains("256color")) {
        return yo_tui::ColorCapability::Limited;
    }
    yo_tui::ColorCapability::Unknown
}

fn compact_workspace_label(cwd: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    compact_workspace_label_with_home(cwd, home.as_deref())
}

fn compact_workspace_label_with_home(
    cwd: &std::path::Path,
    home: Option<&std::path::Path>,
) -> String {
    let Some(home) = home else {
        return cwd.to_string_lossy().into_owned();
    };
    let Ok(relative) = cwd.strip_prefix(home) else {
        return cwd.to_string_lossy().into_owned();
    };
    if relative.as_os_str().is_empty() {
        "~".to_owned()
    } else {
        format!("~/{}", relative.to_string_lossy())
    }
}

#[cfg(test)]
mod tests;
