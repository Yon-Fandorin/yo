use serde::Deserialize;
use yo_core::{
    AccountId, ApiDialect, BackendResumeTarget, CompleteModelBinding, ConnectorId, HostId, ModelId,
    ModelSelection, ModelSelectionController, NormalizedEndpoint, ProviderId, StartupPolicy,
    StartupSelectionSources, StartupTarget, resolve_startup_target,
};

use super::{DelegatedExecutionProfile, StartupBackend};
use crate::{AppError, state::config::Config};

const SESSION_TOOL_EXPOSURE_PROFILE: &str = "yo.session-tool-exposure/v1alpha1";

#[derive(Clone, Debug, Eq, PartialEq)]
enum DurableBackendKind {
    Host(HostId),
    Native,
}

pub(super) fn replacement(
    selection: &ModelSelection,
    registry_revision: crate::execution::tools::LocalToolRegistryRevision,
) -> StartupBackend {
    StartupBackend::Native {
        provider: selection.provider().clone(),
        account: selection.account().clone(),
        model: selection.model().clone(),
        replace_binding: true,
        registry_revision,
    }
}

pub(super) fn resolve(
    config: &Config,
    stored_preference: Option<StartupTarget>,
    override_model: Option<&str>,
    no_tools: bool,
    read_only_review: bool,
    resume: Option<&BackendResumeTarget>,
) -> Result<StartupBackend, AppError> {
    if let Some(target) = resume {
        return resolve_resume(config, override_model, target);
    }
    resolve_new_session_with_tool_restriction(
        config.model_catalog(),
        stored_preference,
        None,
        override_model,
        no_tools,
        read_only_review,
    )
}

#[cfg(test)]
fn resolve_new_session(
    catalog: &yo_core::ModelCatalog,
    stored_preference: Option<StartupTarget>,
    operator: Option<StartupTarget>,
    reference: Option<&str>,
) -> Result<StartupBackend, AppError> {
    resolve_new_session_with_tool_restriction(
        catalog,
        stored_preference,
        operator,
        reference,
        false,
        false,
    )
}

fn resolve_new_session_with_tool_restriction(
    catalog: &yo_core::ModelCatalog,
    stored_preference: Option<StartupTarget>,
    operator: Option<StartupTarget>,
    reference: Option<&str>,
    no_tools: bool,
    read_only_review: bool,
) -> Result<StartupBackend, AppError> {
    let target = resolve_startup_target(
        catalog,
        &StartupPolicy::initial(),
        StartupSelectionSources {
            invocation: reference,
            stored_preference,
            operator_target: operator,
        },
    )
    .map_err(|error| AppError::single("resolving startup target", error))?;
    let Some(target) = target else {
        return Err(
            AppError::message("no startup target is selected").with_help([
                "yo connect",
                "yo --model host:codex",
                "yo --model host:grok",
            ]),
        );
    };
    match target {
        StartupTarget::Host(host) if no_tools => Err(AppError::many([format!(
            "--no-tools ({SESSION_TOOL_EXPOSURE_PROFILE}) is supported only for native model Sessions; {} owns its tool surface",
            host.reference()
        )])),
        StartupTarget::Host(host) => resolve_host(
            host,
            if read_only_review {
                DelegatedExecutionProfile::ReadOnlyReview
            } else {
                DelegatedExecutionProfile::Standard
            },
        ),
        StartupTarget::Model(_) if read_only_review => Err(AppError::many([
            "--sandbox read-only (yo.delegated-review-execution/v1alpha1) is supported only for delegated host Sessions"
                .to_owned(),
        ])),
        StartupTarget::Model(selection) => {
            let entry = catalog
                .resolve_model(selection.provider(), selection.account(), selection.model())
                .map_err(|error| AppError::single("resolving the initial tool policy", error))?;
            let registry_revision = if no_tools
                || entry.explicit_profile().is_some_and(|profile| {
                    profile.tool_capability_policy().as_str() == "no-tools/v1"
                }) {
                crate::execution::tools::LocalToolRegistryRevision::NoTools
            } else {
                crate::execution::tools::LocalToolRegistryRevision::BasicFiles
            };
            Ok(native_selection(selection, false, registry_revision))
        },
    }
}

fn resolve_resume(
    config: &Config,
    override_model: Option<&str>,
    target: &BackendResumeTarget,
) -> Result<StartupBackend, AppError> {
    match classify_durable_backend(target.binding().backend_kind())? {
        DurableBackendKind::Host(host) => {
            return resolve_host_resume(host, override_model, target.binding());
        },
        DurableBackendKind::Native => {},
    }
    let binding_identity = target.binding().binding_identity();
    let durable_binding =
        parse_durable_binding(binding_identity.schema(), binding_identity.value())?;
    let registry_revision =
        crate::execution::tools::revision_for_replay_contract(target.model_replay().contract())
            .map_err(|error| AppError::single("selecting the saved local tool registry", error))?;
    resolve_native_resume(
        config.model_catalog(),
        durable_binding,
        override_model,
        registry_revision,
    )
}

fn classify_durable_backend(kind: &str) -> Result<DurableBackendKind, AppError> {
    if let Some(host) = crate::execution::host::from_backend_kind(kind) {
        return Ok(DurableBackendKind::Host(host));
    }
    if kind == "yo-managed-model" {
        return Ok(DurableBackendKind::Native);
    }
    Err(AppError::many([format!(
        "unsupported durable backend kind {kind:?}; the saved Session can only be opened read-only"
    )]))
}

fn resolve_host(
    host: HostId,
    execution: DelegatedExecutionProfile,
) -> Result<StartupBackend, AppError> {
    crate::execution::host::require_supported(&host)?;
    Ok(match execution {
        DelegatedExecutionProfile::Standard => StartupBackend::Host(host),
        DelegatedExecutionProfile::ReadOnlyReview => StartupBackend::ReadOnlyHost(host),
    })
}

fn resolve_host_resume(
    host: HostId,
    override_model: Option<&str>,
    binding: &yo_core::BackendBindingEvidence,
) -> Result<StartupBackend, AppError> {
    let execution = durable_host_execution(&host, binding.binding_identity().schema())?;
    match override_model {
        None => resolve_host(host, execution),
        Some(reference) if reference == host.reference() => resolve_host(host, execution),
        Some(_) => Err(AppError::many([format!(
            "a different target cannot replace a {} Session; cross-backend handoff is not supported",
            host.reference()
        )])),
    }
}

fn durable_host_execution(
    host: &HostId,
    binding_schema: &str,
) -> Result<DelegatedExecutionProfile, AppError> {
    match (host.as_str(), binding_schema) {
        (HostId::CODEX, "codex.app-server/thread-binding/v1")
        | (HostId::CODEX, "codex.app-server/thread-binding/v2")
        | (HostId::GROK, "grok.acp/session-binding/v1") => Ok(DelegatedExecutionProfile::Standard),
        (HostId::CODEX, "codex.app-server/thread-binding/v1alpha1")
        | (HostId::CODEX, "codex.app-server/thread-binding/v1alpha2")
        | (HostId::GROK, "grok.acp/session-binding/v1alpha1") => {
            Ok(DelegatedExecutionProfile::ReadOnlyReview)
        },
        _ => Err(AppError::many([format!(
            "{} Session has unsupported delegated execution binding `{binding_schema}`; it cannot be resumed without risking a permission downgrade",
            host.reference()
        )])),
    }
}

fn resolve_native_resume(
    catalog: &yo_core::ModelCatalog,
    durable_binding: DurableNativeBinding,
    reference: Option<&str>,
    registry_revision: crate::execution::tools::LocalToolRegistryRevision,
) -> Result<StartupBackend, AppError> {
    let binding = durable_binding.binding();
    let durable_selection = ModelSelection::new(
        binding.provider_id().clone(),
        binding.account_id().clone(),
        binding.model_id().clone(),
    );
    let selection = match reference {
        Some(reference) => {
            match ModelSelectionController::new(catalog.clone(), Some(durable_selection.clone()))
                .resolve_target_reference(reference)
                .map_err(|error| AppError::single("resolving resumed model", error))?
            {
                StartupTarget::Host(_) => {
                    return Err(AppError::many([
                        "an agent host cannot replace a native model Session; cross-backend handoff is not supported"
                            .to_owned(),
                    ]));
                },
                StartupTarget::Model(selection) => selection,
            }
        },
        None => durable_selection,
    };
    let entry = catalog
        .resolve_model(selection.provider(), selection.account(), selection.model())
        .map_err(|error| AppError::single("resolving resumed model", error))?;
    let replace_binding = !durable_binding.matches(entry);
    if reference.is_some() || replace_binding {
        entry
            .require_enabled()
            .map_err(|error| AppError::single("resolving resumed model", error))?;
    }
    Ok(native_selection(
        selection,
        replace_binding,
        registry_revision,
    ))
}

fn native_selection(
    selection: ModelSelection,
    replace_binding: bool,
    registry_revision: crate::execution::tools::LocalToolRegistryRevision,
) -> StartupBackend {
    StartupBackend::Native {
        provider: selection.provider().clone(),
        account: selection.account().clone(),
        model: selection.model().clone(),
        replace_binding,
        registry_revision,
    }
}

fn parse_durable_binding(schema: &str, value: &str) -> Result<DurableNativeBinding, AppError> {
    match schema {
        "yo.model-binding/v1" => {
            let durable: DurableBinding = serde_json::from_str(value).map_err(|_| {
                AppError::many(["the durable native binding identity is malformed".to_owned()])
            })?;
            parse_legacy_binding(durable).map(DurableNativeBinding::Legacy)
        },
        "yo.complete-model-binding/v1" => CompleteModelBinding::from_durable_json(value)
            .map(DurableNativeBinding::Complete)
            .map_err(|error| AppError::single("validating durable complete binding", error)),
        _ => Err(AppError::many([
            "the durable native binding has an unsupported identity schema".to_owned(),
        ])),
    }
}

fn parse_legacy_binding(
    durable: DurableBinding,
) -> Result<yo_core::EffectiveModelBinding, AppError> {
    parse_binding_coordinates(
        durable.provider,
        durable.account,
        durable.model,
        durable.connector,
        durable.api_dialect,
        durable.base_url,
    )
}

fn parse_binding_coordinates(
    provider: String,
    account: String,
    model: String,
    connector: String,
    api_dialect: String,
    base_url: String,
) -> Result<yo_core::EffectiveModelBinding, AppError> {
    let durable_provider = ProviderId::new(provider)
        .map_err(|error| AppError::single("validating durable Provider", error))?;
    let durable_account = AccountId::new(account)
        .map_err(|error| AppError::single("validating durable Account", error))?;
    let durable_model =
        ModelId::new(model).map_err(|error| AppError::single("validating durable Model", error))?;
    let durable_connector = ConnectorId::new(connector)
        .map_err(|error| AppError::single("validating durable connector", error))?;
    let durable_dialect = api_dialect
        .parse::<ApiDialect>()
        .map_err(|error| AppError::single("validating durable API dialect", error))?;
    let durable_endpoint = NormalizedEndpoint::parse(&base_url)
        .map_err(|error| AppError::single("validating durable endpoint", error))?;
    yo_core::EffectiveModelBinding::from_durable(
        durable_provider,
        durable_account,
        durable_model,
        durable_connector,
        durable_dialect,
        durable_endpoint,
    )
    .map_err(|error| AppError::single("validating durable model binding", error))
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DurableNativeBinding {
    Legacy(yo_core::EffectiveModelBinding),
    Complete(CompleteModelBinding),
}

impl DurableNativeBinding {
    fn binding(&self) -> &yo_core::EffectiveModelBinding {
        match self {
            Self::Legacy(binding) => binding,
            Self::Complete(binding) => binding.binding(),
        }
    }

    fn matches(&self, entry: &yo_core::ModelCatalogEntry) -> bool {
        match self {
            Self::Legacy(binding) => {
                entry.explicit_profile().is_none() && entry.binding() == binding
            },
            Self::Complete(binding) => entry.complete_binding() == Some(binding),
        }
    }
}

#[derive(Deserialize)]
struct DurableBinding {
    provider: String,
    account: String,
    model: String,
    connector: String,
    api_dialect: String,
    base_url: String,
}

#[cfg(test)]
mod tests;
