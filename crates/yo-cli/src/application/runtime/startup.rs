use std::{fmt, os::unix::ffi::OsStrExt, path::Path};

use yo_core::{
    ImagePreparationHost, InputAdmissionHost, LocalSkillInputAdmission,
    LocalSkillReferenceProvider, LocalSkillRoot, LocalWorkspaceInputAdmission,
    SkillReferenceProvider, WorkspaceHostId,
    session_repository::{InheritedSessionHistory, StoredSessionContinuation},
};

use super::{
    super::{
        codex_diagnostics::CodexWarningCollector,
        output::{write_cli_diagnostics, write_session_command_output},
    },
    session::termination_requested,
};
use crate::{
    application::{agent, live_selection as live},
    command,
    execution::{model, tools as local_tools},
    interaction::diagnostic::{AppError, CliDiagnostic},
    state::{config, storage},
};

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
    pub(super) is_resume: bool,
    pub(super) session_id: yo_core::SessionId,
    pub(super) inherited_history: Option<InheritedSessionHistory>,
    pub(super) agent: agent::TuiAgentConnection,
    pub(super) workspace: std::path::PathBuf,
    pub(super) workspace_references: Option<yo_core::LocalWorkspaceReferenceProvider>,
    pub(super) skill_references: Option<Box<dyn SkillReferenceProvider>>,
    pub(super) image_preparation: Box<dyn ImagePreparationHost>,
    pub(super) selection: model::StartupBackend,
    pub(super) local_tool_registry: Option<local_tools::LocalToolRegistryRevision>,
    pub(super) execution_manifest_digest: Option<String>,
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
    cwd: &Path,
    options: &command::LiveOptions,
    launch_failure_selection: live::LiveSelection,
    read_only_storage: Option<&storage::LocalReadStorage>,
    snapshots: &mut StartupSnapshots<'_>,
    frontend: StartupFrontend,
) -> Result<StartupOutcome, AppError> {
    prepare_agent_with_target(
        termination,
        cwd,
        options,
        (launch_failure_selection, read_only_storage),
        snapshots,
        frontend,
        LaunchRequest::Options(None),
    )
}

pub(super) fn prepare_new_agent(
    termination: &mut impl yo_tui::TerminationSource,
    cwd: &Path,
    options: &command::LiveOptions,
    snapshots: &mut StartupSnapshots<'_>,
    host_target: Option<&model::ActiveHostModel>,
) -> Result<StartupOutcome, AppError> {
    prepare_agent_with_target(
        termination,
        cwd,
        options,
        (live::LiveSelection::New, None),
        snapshots,
        StartupFrontend::Terminal,
        LaunchRequest::Options(host_target),
    )
}

pub(super) fn prepare_fork_agent(
    termination: &mut impl yo_tui::TerminationSource,
    cwd: &Path,
    options: &command::LiveOptions,
    snapshots: &mut StartupSnapshots<'_>,
    parent: StoredSessionContinuation,
) -> Result<StartupOutcome, AppError> {
    require_supported_fork_binding(parent.target().binding())?;
    if parent.target().source().is_none() || parent.target().model_replay().items().is_empty() {
        return Err(AppError::message(
            "fork requires a complete exact parent reconstruction",
        ));
    }
    prepare_agent_with_target(
        termination,
        cwd,
        options,
        (live::LiveSelection::New, None),
        snapshots,
        StartupFrontend::Terminal,
        LaunchRequest::Fork(Box::new(parent)),
    )
}

fn prepare_agent_with_target(
    termination: &mut impl yo_tui::TerminationSource,
    cwd: &Path,
    options: &command::LiveOptions,
    failure_context: (live::LiveSelection, Option<&storage::LocalReadStorage>),
    snapshots: &mut StartupSnapshots<'_>,
    frontend: StartupFrontend,
    request: LaunchRequest<'_>,
) -> Result<StartupOutcome, AppError> {
    let (launch_failure_selection, read_only_storage) = failure_context;
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
    let (launch, host_target) = match request {
        LaunchRequest::Fork(parent) => {
            let descriptor = fork_descriptor(cwd, workspace_host_id, parent.descriptor())?;
            (Launch::Fork { descriptor, parent }, None)
        },
        LaunchRequest::Options(host_target) => (
            match options.selection {
                command::LiveSelection::New => {
                    let workspace_path =
                        yo_core::HostWorkspacePath::normalize_local(cwd).map_err(|error| {
                            AppError::single("normalizing the workspace path", error)
                        })?;
                    Launch::New(
                        yo_core::SessionDescriptor::new(workspace_host_id, workspace_path)
                            .map_err(|error| {
                                AppError::single("generating a Session descriptor", error)
                            })?,
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
            },
            host_target,
        ),
    };
    let session_cwd = match &launch {
        Launch::New(_) | Launch::Fork { .. } => cwd.to_owned(),
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
    let mut input_admission: Box<dyn InputAdmissionHost> = Box::new(
        LocalWorkspaceInputAdmission::new(&session_cwd, workspace_host_id)
            .map_err(|error| AppError::single("binding workspace input admission", error))?,
    );

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
    let resolved_selection = match &launch {
        Launch::Fork { parent, .. } => {
            model::resolve(config, None, None, false, false, Some(parent.target()))
        },
        _ => model::resolve(
            config,
            stored_preference.cloned(),
            options.model.as_deref(),
            options.no_tools,
            options.sandbox.is_some(),
            match &launch {
                Launch::New(_) => None,
                Launch::Resume(continuation) => Some(continuation.target()),
                Launch::Fork { .. } => unreachable!("fork selection uses only its saved target"),
            },
        ),
    };
    let selection = match resolved_selection {
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
    if matches!(&launch, Launch::Fork { .. }) {
        require_exact_fork_selection(&selection)?;
    }
    require_exact_print_resume_binding(
        frontend,
        launch.resume_id().is_some(),
        selection.replaces_binding(),
    )?;
    if selection.delegated_host().is_some() && !config.command_tools().is_empty() {
        write_cli_diagnostics(&[CliDiagnostic::warning(
            "tools.commands is available to managed models only; the selected agent host manages its own tools",
        )])?;
    }
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
    let session_id = match &launch {
        Launch::New(descriptor) | Launch::Fork { descriptor, .. } => descriptor.session_id(),
        Launch::Resume(continuation) => continuation.descriptor().session_id(),
    };
    let host_catalogs = if matches!(frontend, StartupFrontend::Terminal) {
        model::read_builtin_host_catalogs_with_codex_warning_observer(
            &session_cwd,
            selection.delegated_host(),
            Some(codex_warning_observer.clone()),
        )
    } else {
        Vec::new()
    };
    let local_skill_references = if selection
        .delegated_host()
        .is_some_and(|(host, _)| host.as_str() == yo_core::HostId::CODEX)
    {
        None
    } else {
        match prepare_local_skills(config, &session_cwd, workspace_host_id, frontend) {
            Ok(PreparedLocalSkills {
                admission,
                references,
            }) => {
                input_admission = admission;
                references
            },
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
            Err(error) => return Err(AppError::single("binding local skills", error)),
        }
    };
    let mut prepared_fork = None;
    let mut execution_manifest_digest = None;
    let (backend, skill_references): (
        Box<dyn yo_core::AgentBackend + Send>,
        Option<Box<dyn SkillReferenceProvider>>,
    ) = match selection.delegated_host() {
        Some((host, execution)) if host.as_str() == yo_core::HostId::CODEX => {
            let codex_config = yo_backend_delegated_codex::CodexBackendConfig::new(&session_cwd)
                .with_read_only_review(execution.is_read_only_review());
            let codex_config = if let Some(target) = host_target {
                codex_config
                    .with_new_session_target(target.account().clone(), target.model().clone())
            } else {
                codex_config
            };
            input_admission = Box::new(
                yo_backend_delegated_codex::CodexSkillInputAdmission::new(
                    codex_config.clone(),
                    workspace_host_id,
                    Some(codex_warning_observer.clone()),
                )
                .map_err(|error| AppError::single("binding Codex skill input admission", error))?,
            );
            let skills = if matches!(frontend, StartupFrontend::Terminal) {
                match yo_backend_delegated_codex::CodexSkillReferenceProvider::start_with_warning_observer(
                codex_config.clone(),
                workspace_host_id,
                Some(codex_warning_observer.clone()),
            ) {
                Ok(skills) => Some(Box::new(skills) as Box<dyn SkillReferenceProvider>),
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
            (Box::new(backend), local_skill_references)
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
            let started = match &launch {
                Launch::Fork { descriptor, parent } => model::start_native_for_fork(
                    config,
                    selected_credentials,
                    &selection,
                    &session_cwd,
                    parent,
                    descriptor.clone(),
                    &mut || termination_requested(termination),
                )
                .map(|(backend, continuation, digest)| {
                    prepared_fork = Some(continuation);
                    (backend, digest)
                }),
                _ => model::start_native(
                    config,
                    selected_credentials,
                    &selection,
                    &session_cwd,
                    &mut || termination_requested(termination),
                ),
            };
            let backend = match started {
                Ok((backend, digest)) => {
                    execution_manifest_digest = digest;
                    backend
                },
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
            (backend, local_skill_references)
        },
    };
    let supports_native_model_rebind = backend.capabilities().supports_native_model_rebind();
    let inherited_history = if matches!(frontend, StartupFrontend::Terminal) {
        match &launch {
            Launch::New(_) => None,
            Launch::Resume(continuation) => continuation.inherited_history().cloned(),
            Launch::Fork { .. } => prepared_fork
                .as_ref()
                .and_then(StoredSessionContinuation::inherited_history)
                .cloned(),
        }
    } else {
        None
    };
    let restored_records = match &launch {
        Launch::New(_) => &[][..],
        Launch::Resume(continuation) => continuation.transcript_records(),
        Launch::Fork { .. } => prepared_fork
            .as_ref()
            .map_or(&[][..], StoredSessionContinuation::transcript_records),
    };
    let (input_admission, image_preparation) = crate::execution::image::bind(
        input_admission,
        &session_cwd,
        restored_records,
        inherited_history.as_ref(),
        config.clipboard_source(),
    );
    let agent = match launch {
        Launch::New(descriptor) => agent::TuiAgentConnection::start_persistent(
            backend,
            descriptor,
            repository,
            termination,
        ),
        Launch::Fork { .. } => agent::TuiAgentConnection::start_resumed(
            backend,
            prepared_fork.expect("managed fork preparation returned its child bootstrap"),
            repository,
            false,
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
    let Some(mut agent) = agent else {
        return Ok(StartupOutcome::Complete);
    };
    agent
        .configure_input_admission(input_admission)
        .map_err(|error| AppError::single("configuring input admission", error))?;
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
        is_resume,
        session_id,
        inherited_history,
        agent,
        workspace: session_cwd,
        workspace_references,
        skill_references,
        image_preparation,
        selection,
        local_tool_registry,
        execution_manifest_digest,
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
    Resume(Box<StoredSessionContinuation>),
    Fork {
        descriptor: yo_core::SessionDescriptor,
        parent: Box<StoredSessionContinuation>,
    },
}

enum LaunchRequest<'a> {
    Options(Option<&'a model::ActiveHostModel>),
    Fork(Box<StoredSessionContinuation>),
}

impl Launch {
    fn resume_id(&self) -> Option<yo_core::SessionId> {
        match self {
            Self::New(_) | Self::Fork { .. } => None,
            Self::Resume(continuation) => Some(continuation.descriptor().session_id()),
        }
    }
}

fn require_supported_fork_binding(
    binding: &yo_core::BackendBindingEvidence,
) -> Result<(), AppError> {
    if binding.backend_kind() != "yo-managed-model"
        || !matches!(
            binding.continuation_strategy(),
            yo_core::ContinuationStrategy::ExactReplay {
                executor: yo_core::ReplayExecutor::LocalClient,
                ..
            }
        )
    {
        return Err(AppError::message(
            "this backend does not support a verified exact Session fork",
        ));
    }
    Ok(())
}

fn require_exact_fork_selection(selection: &model::StartupBackend) -> Result<(), AppError> {
    if selection.delegated_host().is_some() || selection.replaces_binding() {
        return Err(AppError::message(
            "fork requires the parent's saved model and tool profile without replacement",
        ));
    }
    Ok(())
}

fn fork_descriptor(
    cwd: &Path,
    host: WorkspaceHostId,
    parent: &yo_core::SessionDescriptor,
) -> Result<yo_core::SessionDescriptor, AppError> {
    let path = yo_core::HostWorkspacePath::normalize_local(cwd)
        .map_err(|error| AppError::single("normalizing the fork workspace", error))?;
    if parent.workspace_host_id() != host || parent.workspace_path() != &path {
        return Err(AppError::message(
            "fork requires the parent's recorded workspace host and path",
        ));
    }
    yo_core::SessionDescriptor::new(host, path)
        .map_err(|error| AppError::single("generating the child Session descriptor", error))
}

struct PreparedLocalSkills {
    admission: Box<dyn InputAdmissionHost>,
    references: Option<Box<dyn SkillReferenceProvider>>,
}

// Explicit roots are resolved against the selected Session workspace, including resumed Sessions.
// Skill instructions do not change the backend's tool or sandbox capabilities.
fn prepare_local_skills(
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

#[cfg(test)]
mod tests;
