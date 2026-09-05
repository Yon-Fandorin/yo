use std::{fmt, os::unix::ffi::OsStrExt};

use super::super::{
    codex_diagnostics::CodexWarningCollector, output::write_session_command_output,
};
use crate::{agent, command, config, diagnostic::AppError, live, local_tools, model, storage};

#[derive(Clone, Copy)]
pub(super) enum StartupFrontend {
    Terminal,
    Print,
}

pub(super) struct StartupSnapshots<'a> {
    pub(super) config: &'a config::Config,
    pub(super) credentials: &'a mut Option<yo_core::CredentialSnapshot>,
    pub(super) stored_preference: Option<&'a yo_core::StartupTarget>,
    pub(super) codex_warnings: &'a CodexWarningCollector,
}

pub(super) struct PreparedAgent {
    pub(super) agent: agent::TuiAgentConnection,
    pub(super) workspace: std::path::PathBuf,
    pub(super) workspace_references: Option<yo_core::LocalWorkspaceReferenceProvider>,
    pub(super) skill_references: Option<yo_backend_delegated_codex::CodexSkillReferenceProvider>,
    pub(super) selection: model::StartupBackend,
    pub(super) local_tool_registry: Option<local_tools::LocalToolRegistryRevision>,
    pub(super) active_host: Option<yo_core::HostId>,
    pub(super) active_host_execution: Option<model::DelegatedExecutionProfile>,
    pub(super) active_host_model: Option<model::ActiveHostModel>,
    pub(super) host_catalogs: Vec<model::HostCatalogObservation>,
}

pub(super) enum StartupOutcome {
    Ready(Box<PreparedAgent>),
    Complete,
}

pub(super) fn prepare_agent(
    termination: &mut impl yo_tui::TerminationSource,
    cwd: &std::path::Path,
    options: &command::LiveOptions,
    launch_failure_selection: live::LiveSelection,
    read_only_storage: Option<&storage::LocalReadStorage>,
    snapshots: &mut StartupSnapshots<'_>,
    frontend: StartupFrontend,
) -> Result<StartupOutcome, AppError> {
    let config = snapshots.config;
    let credentials = &mut *snapshots.credentials;
    let stored_preference = snapshots.stored_preference;
    let codex_warnings = snapshots.codex_warnings;
    let codex_warning_observer = codex_warnings.observer();

    let storage = match storage::open_default() {
        Ok(storage) => storage,
        Err(error) => {
            return handle_launch_failure(
                launch_failure_selection,
                options.glyph_profile,
                read_only_storage,
                live::ResumeFailureStage::WritableStorage,
                error,
            );
        },
    };
    let (mut repository, workspace_host_id) = storage.into_parts();
    let launch = match options.selection {
        command::LiveSelection::New => {
            let workspace_path = yo_core::HostWorkspacePath::normalize_local(cwd)
                .map_err(|error| AppError::single("normalizing the workspace path", error))?;
            Launch::New(
                yo_core::SessionDescriptor::new(workspace_host_id, workspace_path)
                    .map_err(|error| AppError::single("generating a Session descriptor", error))?,
            )
        },
        command::LiveSelection::Resume(session_id) => {
            let continuation =
                match yo_core::session_repository::recover_stored_session_continuation(
                    &mut repository,
                    session_id,
                ) {
                    Ok(continuation) => continuation,
                    Err(error) => {
                        drop(repository);
                        return handle_launch_failure(
                            launch_failure_selection,
                            options.glyph_profile,
                            read_only_storage,
                            live::ResumeFailureStage::Revalidation,
                            error,
                        );
                    },
                };
            if continuation.descriptor().workspace_host_id() != workspace_host_id {
                drop(repository);
                return handle_launch_failure(
                    launch_failure_selection,
                    options.glyph_profile,
                    read_only_storage,
                    live::ResumeFailureStage::Revalidation,
                    "the Session belongs to another workspace host",
                );
            }
            Launch::Resume(Box::new(continuation))
        },
        command::LiveSelection::Continue => {
            unreachable!("--continue is resolved before the live generation")
        },
    };
    let session_cwd = match &launch {
        Launch::New(_) => cwd.to_owned(),
        Launch::Resume(continuation) => std::path::PathBuf::from(std::ffi::OsStr::from_bytes(
            continuation.descriptor().workspace_path().as_unix_bytes(),
        )),
    };
    if !session_cwd.is_dir() {
        if matches!(&launch, Launch::Resume(_)) {
            drop(repository);
            return handle_launch_failure(
                launch_failure_selection,
                options.glyph_profile,
                read_only_storage,
                live::ResumeFailureStage::RecordedWorkspace,
                session_cwd.display(),
            );
        }
        return Err(AppError::many([format!(
            "workspace is unavailable at {}",
            session_cwd.display()
        )]));
    }
    let workspace_references = if matches!(frontend, StartupFrontend::Terminal) {
        match yo_core::LocalWorkspaceReferenceProvider::start(&session_cwd, workspace_host_id) {
            Ok(provider) => Some(provider),
            Err(error) => {
                if launch.resume_id().is_some() {
                    drop(repository);
                    return handle_launch_failure(
                        launch_failure_selection,
                        options.glyph_profile,
                        read_only_storage,
                        live::ResumeFailureStage::WorkspaceReferences,
                        error,
                    );
                }
                return Err(AppError::single(
                    "starting workspace reference discovery",
                    error,
                ));
            },
        }
    } else {
        None
    };
    let selection = match model::resolve(
        config,
        stored_preference.cloned(),
        options.model.as_deref(),
        options.no_tools,
        options.sandbox.is_some(),
        match &launch {
            Launch::New(_) => None,
            Launch::Resume(continuation) => Some(continuation.target()),
        },
    ) {
        Ok(selection) => selection,
        Err(error) if launch.resume_id().is_some() => {
            drop(repository);
            return handle_launch_failure(
                launch_failure_selection,
                options.glyph_profile,
                read_only_storage,
                live::ResumeFailureStage::BackendSpawn,
                error,
            );
        },
        Err(error) => return Err(error),
    };
    require_exact_print_resume_binding(
        frontend,
        launch.resume_id().is_some(),
        selection.replaces_binding(),
    )?;
    let active_host = selection.delegated_host().map(|(host, _)| host.clone());
    let active_host_execution = selection.delegated_host().map(|(_, execution)| execution);
    let resumed_codex_binding = match (&launch, active_host.as_ref()) {
        (Launch::Resume(continuation), Some(host)) if host.as_str() == yo_core::HostId::CODEX => {
            yo_backend_delegated_codex::native_model_binding(continuation.target().binding())
                .ok()
                .flatten()
        },
        _ => None,
    };
    let is_resume = launch.resume_id().is_some();
    let host_catalogs = if matches!(frontend, StartupFrontend::Terminal) {
        model::read_builtin_host_catalogs_with_codex_warning_observer(
            &session_cwd,
            selection.delegated_host(),
            Some(codex_warning_observer.clone()),
        )
    } else {
        Vec::new()
    };
    let (backend, skill_references): (
        Box<dyn yo_core::AgentBackend + Send>,
        Option<yo_backend_delegated_codex::CodexSkillReferenceProvider>,
    ) = match selection.delegated_host() {
        Some((host, execution)) if host.as_str() == yo_core::HostId::CODEX => {
            let codex_config = yo_backend_delegated_codex::CodexBackendConfig::new(&session_cwd)
                .with_read_only_review(execution.is_read_only_review());
            let skills = if matches!(frontend, StartupFrontend::Terminal) {
                match yo_backend_delegated_codex::CodexSkillReferenceProvider::start_with_warning_observer(
                codex_config.clone(),
                workspace_host_id,
                Some(codex_warning_observer.clone()),
            ) {
                Ok(skills) => Some(skills),
                Err(error) if launch.resume_id().is_some() => {
                    drop(repository);
                    return handle_launch_failure(
                        launch_failure_selection,
                        options.glyph_profile,
                        read_only_storage,
                        live::ResumeFailureStage::SkillReferences,
                        error,
                    );
                },
                Err(error) => {
                    return Err(AppError::single("starting Codex skill discovery", error));
                },
            }
            } else {
                None
            };
            let backend =
                match yo_backend_delegated_codex::CodexBackend::spawn_with_warning_observer(
                    codex_config,
                    Some(codex_warning_observer.clone()),
                ) {
                    Ok(backend) => backend,
                    Err(error) if launch.resume_id().is_some() => {
                        drop(repository);
                        return handle_launch_failure(
                            launch_failure_selection,
                            options.glyph_profile,
                            read_only_storage,
                            live::ResumeFailureStage::BackendSpawn,
                            error,
                        );
                    },
                    Err(error) => return Err(AppError::single("starting Codex", error)),
                };
            (Box::new(backend), skills)
        },
        Some((host, execution)) if host.as_str() == yo_core::HostId::GROK => {
            let outer_sandboxed_review =
                std::env::var_os(yo_backend_delegated_grok::OUTER_SANDBOX_REVIEW_ENV).is_some();
            let grok_config = yo_backend_delegated_grok::GrokBackendConfig::new(&session_cwd)
                .with_read_only_review(execution.is_read_only_review())
                .with_outer_sandboxed_review(outer_sandboxed_review);
            let backend = match yo_backend_delegated_grok::GrokBackend::spawn(grok_config) {
                Ok(backend) => backend,
                Err(error) if launch.resume_id().is_some() => {
                    drop(repository);
                    return handle_launch_failure(
                        launch_failure_selection,
                        options.glyph_profile,
                        read_only_storage,
                        live::ResumeFailureStage::BackendSpawn,
                        error,
                    );
                },
                Err(error) => return Err(AppError::single("starting Grok", error)),
            };
            (Box::new(backend), None)
        },
        Some((host, _)) => {
            return Err(AppError::message(format!(
                "unsupported agent host {:?}",
                host.as_str()
            )));
        },
        None => {
            let selected_credentials =
                match model::credentials_for_startup(config, credentials, &selection) {
                    Ok(Some(credentials)) => credentials,
                    Ok(None) => unreachable!("native selection requires credentials"),
                    Err(error) if launch.resume_id().is_some() => {
                        drop(repository);
                        return handle_launch_failure(
                            launch_failure_selection,
                            options.glyph_profile,
                            read_only_storage,
                            live::ResumeFailureStage::BackendSpawn,
                            error,
                        );
                    },
                    Err(error) => return Err(error),
                };
            let backend =
                match model::start_native(config, selected_credentials, &selection, &session_cwd) {
                    Ok(backend) => backend,
                    Err(error) if launch.resume_id().is_some() => {
                        drop(repository);
                        return handle_launch_failure(
                            launch_failure_selection,
                            options.glyph_profile,
                            read_only_storage,
                            live::ResumeFailureStage::BackendSpawn,
                            error,
                        );
                    },
                    Err(error) => return Err(error),
                };
            (backend, None)
        },
    };
    let supports_native_model_rebind = backend.capabilities().supports_native_model_rebind();
    let agent = match launch {
        Launch::New(descriptor) => agent::TuiAgentConnection::start_persistent(
            backend,
            descriptor,
            repository,
            termination,
        ),
        Launch::Resume(continuation) => {
            let replace_binding = selection.replaces_binding();
            match agent::TuiAgentConnection::start_resumed(
                backend,
                *continuation,
                repository,
                replace_binding,
                termination,
            ) {
                Ok(agent) => Ok(agent),
                Err(error) => {
                    return handle_launch_failure(
                        launch_failure_selection,
                        options.glyph_profile,
                        read_only_storage,
                        live::ResumeFailureStage::NativeResume,
                        error,
                    );
                },
            }
        },
    }
    .map_err(|error| AppError::single("creating the agent Session", error))?;
    let Some(agent) = agent else {
        return Ok(StartupOutcome::Complete);
    };
    let started_codex_binding = if !is_resume
        && active_host
            .as_ref()
            .is_some_and(|host| host.as_str() == yo_core::HostId::CODEX)
    {
        agent.initial_binding_record().and_then(|record| {
            yo_backend_delegated_codex::native_model_binding_from_trace(&record)
                .ok()
                .flatten()
        })
    } else {
        None
    };
    let confirmed_codex_binding = resumed_codex_binding.or(started_codex_binding);
    let active_host_model = model::resolve_active_host_model(
        active_host.as_ref(),
        confirmed_codex_binding
            .as_ref()
            .map(|binding| (binding.account(), binding.model())),
        supports_native_model_rebind,
        is_resume,
        &host_catalogs,
    );

    let local_tool_registry = selection.registry_revision();
    Ok(StartupOutcome::Ready(Box::new(PreparedAgent {
        agent,
        workspace: session_cwd,
        workspace_references,
        skill_references,
        selection,
        local_tool_registry,
        active_host,
        active_host_execution,
        active_host_model,
        host_catalogs,
    })))
}

fn require_exact_print_resume_binding(
    frontend: StartupFrontend,
    is_resume: bool,
    replaces_binding: bool,
) -> Result<(), AppError> {
    if matches!(frontend, StartupFrontend::Print) && is_resume && replaces_binding {
        return Err(AppError::message(
            "print resume requires the saved backend binding to remain executable without replacement",
        ));
    }
    Ok(())
}

fn complete_with_read_only_resume(
    storage: &storage::LocalReadStorage,
    session_id: yo_core::SessionId,
    glyph_profile: yo_tui::GlyphProfile,
    reason: &str,
) -> Result<StartupOutcome, AppError> {
    let reader = storage
        .reader()
        .ok_or_else(|| AppError::message("captured read-only storage has no Session reader"))?;
    let output = command::read_only_resume_from(reader, session_id, glyph_profile, reason)?;
    write_session_command_output(output)?;
    Ok(StartupOutcome::Complete)
}

fn handle_launch_failure(
    selection: live::LiveSelection,
    glyph_profile: yo_tui::GlyphProfile,
    storage: Option<&storage::LocalReadStorage>,
    stage: live::ResumeFailureStage,
    detail: impl fmt::Display,
) -> Result<StartupOutcome, AppError> {
    match live::classify_launch_failure(selection, stage, detail) {
        live::ResumeFailureDisposition::Abort(reason) => Err(AppError::many([reason])),
        live::ResumeFailureDisposition::ReadOnly { session_id, reason } => {
            let storage = storage.ok_or_else(|| {
                AppError::message("read-only resume fallback lost its captured local storage")
            })?;
            complete_with_read_only_resume(storage, session_id, glyph_profile, &reason)
        },
    }
}

enum Launch {
    New(yo_core::SessionDescriptor),
    Resume(Box<yo_core::session_repository::StoredSessionContinuation>),
}

impl Launch {
    fn resume_id(&self) -> Option<yo_core::SessionId> {
        match self {
            Self::New(_) => None,
            Self::Resume(continuation) => Some(continuation.descriptor().session_id()),
        }
    }
}

#[cfg(test)]
mod tests;
