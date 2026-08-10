use serde::Deserialize;
use yo_core::{
    AccountId, ApiDialect, BackendResumeTarget, ConnectorId, ModelId, ModelSelection,
    ModelSelectionController, NormalizedEndpoint, ProviderId, StartupPolicy,
    StartupSelectionSources, StartupTarget, resolve_startup_target,
};

use super::StartupBackend;
use crate::{AppError, config::Config};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DurableBackendKind {
    Codex,
    Native,
}

pub(super) fn replacement(selection: &yo_core::ModelSelection) -> StartupBackend {
    StartupBackend::Native {
        provider: selection.provider().clone(),
        account: selection.account().clone(),
        model: selection.model().clone(),
        replace_binding: true,
    }
}

pub(super) fn resolve(
    config: &Config,
    override_model: Option<&str>,
    resume: Option<&BackendResumeTarget>,
) -> Result<StartupBackend, AppError> {
    if let Some(target) = resume {
        return resolve_resume(config, override_model, target);
    }
    let startup = config.startup_target().cloned();
    resolve_new_session(config.model_catalog(), startup, override_model)
}

pub(super) fn resolve_new_session(
    catalog: &yo_core::ModelCatalog,
    operator: Option<StartupTarget>,
    reference: Option<&str>,
) -> Result<StartupBackend, AppError> {
    let target = resolve_startup_target(
        catalog,
        &StartupPolicy::initial(),
        StartupSelectionSources {
            invocation: reference,
            stored_preference: None,
            operator_target: operator,
        },
    )
    .map_err(|error| AppError::single("resolving startup target", error))?;
    let Some(target) = target else {
        return Err(AppError::message("no startup target is selected")
            .with_help(["yo connect", "yo --model host:codex"]));
    };
    match target {
        StartupTarget::HostCodex => Ok(StartupBackend::Codex),
        StartupTarget::Model(selection) => Ok(native_selection(selection, false)),
    }
}

fn resolve_resume(
    config: &Config,
    override_model: Option<&str>,
    target: &BackendResumeTarget,
) -> Result<StartupBackend, AppError> {
    match classify_durable_backend(target.binding().backend_kind())? {
        DurableBackendKind::Codex => return resolve_codex_resume(override_model),
        DurableBackendKind::Native => {},
    }
    let binding_identity = target.binding().binding_identity();
    if binding_identity.schema() != "yo.model-binding/v1" {
        return Err(AppError::many([
            "the durable native binding has an unsupported identity schema".to_owned(),
        ]));
    }
    let durable_binding = parse_durable_binding(binding_identity.value())?;
    resolve_native_resume(config.model_catalog(), durable_binding, override_model)
}

pub(super) fn classify_durable_backend(kind: &str) -> Result<DurableBackendKind, AppError> {
    match kind {
        "codex-app-server" => Ok(DurableBackendKind::Codex),
        "yo-managed-model" => Ok(DurableBackendKind::Native),
        other => Err(AppError::many([format!(
            "unsupported durable backend kind {other:?}; the saved Session can only be opened read-only"
        )])),
    }
}

pub(super) fn resolve_codex_resume(
    override_model: Option<&str>,
) -> Result<StartupBackend, AppError> {
    match override_model {
        None | Some(StartupTarget::HOST_CODEX_REFERENCE) => Ok(StartupBackend::Codex),
        Some(_) => Err(AppError::many([
            "a different model target cannot replace a Codex Session; cross-backend handoff is not supported"
                .to_owned(),
        ])),
    }
}

pub(super) fn resolve_native_resume(
    catalog: &yo_core::ModelCatalog,
    durable_binding: yo_core::EffectiveModelBinding,
    reference: Option<&str>,
) -> Result<StartupBackend, AppError> {
    let durable_selection = ModelSelection::new(
        durable_binding.provider_id().clone(),
        durable_binding.account_id().clone(),
        durable_binding.model_id().clone(),
    );
    let selection = match reference {
        Some(reference) => {
            match ModelSelectionController::new(catalog.clone(), Some(durable_selection.clone()))
                .resolve_target_reference(reference)
                .map_err(|error| AppError::single("resolving resumed model", error))?
            {
                StartupTarget::HostCodex => {
                    return Err(AppError::many([
                    "Local Codex cannot replace a Yo-managed Session; cross-backend handoff is not supported"
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
    let replace_binding = entry.binding() != &durable_binding;
    Ok(native_selection(selection, replace_binding))
}

fn native_selection(selection: ModelSelection, replace_binding: bool) -> StartupBackend {
    StartupBackend::Native {
        provider: selection.provider().clone(),
        account: selection.account().clone(),
        model: selection.model().clone(),
        replace_binding,
    }
}

pub(super) fn parse_durable_binding(
    value: &str,
) -> Result<yo_core::EffectiveModelBinding, AppError> {
    let durable: DurableBinding = serde_json::from_str(value).map_err(|_| {
        AppError::many(["the durable native binding identity is malformed".to_owned()])
    })?;
    let durable_provider = ProviderId::new(durable.provider)
        .map_err(|error| AppError::single("validating durable Provider", error))?;
    let durable_account = AccountId::new(durable.account)
        .map_err(|error| AppError::single("validating durable Account", error))?;
    let durable_model = ModelId::new(durable.model)
        .map_err(|error| AppError::single("validating durable Model", error))?;
    let durable_connector = ConnectorId::new(durable.connector)
        .map_err(|error| AppError::single("validating durable connector", error))?;
    let durable_dialect = durable
        .api_dialect
        .parse::<ApiDialect>()
        .map_err(|error| AppError::single("validating durable API dialect", error))?;
    let durable_endpoint = NormalizedEndpoint::parse(&durable.base_url)
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

#[derive(Deserialize)]
struct DurableBinding {
    provider: String,
    account: String,
    model: String,
    connector: String,
    api_dialect: String,
    base_url: String,
}
