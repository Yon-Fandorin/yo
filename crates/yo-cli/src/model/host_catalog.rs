use std::path::Path;

use yo_core::{AccountId, HostId, HostModelCatalog, ModelId, ModelSelectionController};

use super::DelegatedExecutionProfile;

const CATALOG_UNAVAILABLE: &str = "model catalog unavailable";
const SEMANTIC_HANDOFF_UNAVAILABLE: &str = "semantic handoff is not implemented";
const NATIVE_REBIND_UNAVAILABLE: &str =
    "this host does not advertise state-preserving model switching";
const ACCOUNT_MISMATCH_UNAVAILABLE: &str =
    "the authenticated host account differs from this Session";

#[derive(Clone, Debug)]
pub(crate) struct HostCatalogObservation {
    host: HostId,
    catalog: Result<HostModelCatalog, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveHostModel {
    host: HostId,
    account: AccountId,
    model: ModelId,
    native_model_rebind: bool,
}

impl ActiveHostModel {
    pub(crate) const fn new(
        host: HostId,
        account: AccountId,
        model: ModelId,
        native_model_rebind: bool,
    ) -> Self {
        Self {
            host,
            account,
            model,
            native_model_rebind,
        }
    }

    pub(crate) const fn host(&self) -> &HostId {
        &self.host
    }

    pub(crate) const fn account(&self) -> &AccountId {
        &self.account
    }

    pub(crate) const fn model(&self) -> &ModelId {
        &self.model
    }

    pub(crate) const fn supports_native_model_rebind(&self) -> bool {
        self.native_model_rebind
    }

    pub(crate) fn set_model(&mut self, model: ModelId) {
        self.model = model;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HostInventoryRequest {
    host: HostId,
    execution: DelegatedExecutionProfile,
    outer_sandboxed_review: bool,
}

impl HostCatalogObservation {
    fn new(host: HostId, catalog: Result<HostModelCatalog, String>) -> Self {
        Self { host, catalog }
    }

    pub(crate) const fn host(&self) -> &HostId {
        &self.host
    }

    pub(crate) fn catalog(&self) -> Result<&HostModelCatalog, &str> {
        self.catalog.as_ref().map_err(String::as_str)
    }
}

/// Reads every built-in delegated host inventory concurrently. The active host keeps its exact
/// execution profile; inactive hosts use their ordinary session-free inventory profile.
pub(crate) fn read_builtin_host_catalogs(
    workspace: &Path,
    active: Option<(&HostId, DelegatedExecutionProfile)>,
) -> Vec<HostCatalogObservation> {
    let requests = inventory_requests(
        active,
        std::env::var_os(yo_backend_delegated_grok::OUTER_SANDBOX_REVIEW_ENV).is_some(),
    );
    let codex = requests[0].clone();
    let grok = requests[1].clone();
    let codex_workspace = workspace.to_path_buf();
    let grok_workspace = workspace.to_path_buf();

    std::thread::scope(|scope| {
        let codex_reader = scope.spawn(move || {
            let config = yo_backend_delegated_codex::CodexBackendConfig::new(codex_workspace)
                .with_read_only_review(codex.execution.is_read_only_review());
            yo_backend_delegated_codex::read_model_catalog(config)
                .map_err(|error| error.to_string())
        });
        let grok_reader = scope.spawn(move || {
            let config = yo_backend_delegated_grok::GrokBackendConfig::new(grok_workspace)
                .with_read_only_review(grok.execution.is_read_only_review())
                .with_outer_sandboxed_review(grok.outer_sandboxed_review);
            yo_backend_delegated_grok::read_model_catalog(config).map_err(|error| error.to_string())
        });

        vec![
            HostCatalogObservation::new(
                HostId::codex(),
                codex_reader
                    .join()
                    .unwrap_or_else(|_| Err("Codex model inventory reader panicked".to_owned())),
            ),
            HostCatalogObservation::new(
                HostId::grok(),
                grok_reader
                    .join()
                    .unwrap_or_else(|_| Err("Grok model inventory reader panicked".to_owned())),
            ),
        ]
    })
}

/// Resolves the live host state used by the picker. Codex current state comes only from its
/// durable binding; a pre-start inventory default is never treated as the running thread model.
pub(crate) fn resolve_active_host_model(
    active_host: Option<&HostId>,
    confirmed_codex: Option<(&AccountId, &ModelId)>,
    supports_native_model_rebind: bool,
    is_resume: bool,
    observations: &[HostCatalogObservation],
) -> Option<ActiveHostModel> {
    let host = active_host?;
    if host.as_str() == HostId::CODEX {
        let (account, model) = confirmed_codex?;
        return Some(ActiveHostModel::new(
            host.clone(),
            account.clone(),
            model.clone(),
            supports_native_model_rebind,
        ));
    }
    let catalog = observations
        .iter()
        .find(|observation| observation.host() == host)?
        .catalog()
        .ok()?;
    Some(ActiveHostModel::new(
        host.clone(),
        catalog.account().clone(),
        catalog.current_model()?.clone(),
        supports_native_model_rebind && !is_resume,
    ))
}

/// Projects every observed host independently so one missing binary or account cannot suppress a
/// sibling host section. Only the live host is allowed to mark an advertised model as current.
pub(crate) fn project_host_catalogs(
    mut controller: ModelSelectionController,
    active: Option<&ActiveHostModel>,
    observations: &[HostCatalogObservation],
) -> ModelSelectionController {
    for observation in observations {
        let is_active_host = active.is_some_and(|active| active.host == observation.host);
        controller = match &observation.catalog {
            Ok(catalog) => {
                let same_account = active.is_some_and(|active| {
                    active.host == observation.host && active.account == *catalog.account()
                });
                let active_model =
                    same_account.then(|| active.expect("same account is active").model());
                let unavailable = if same_account
                    && active.is_some_and(ActiveHostModel::supports_native_model_rebind)
                {
                    None
                } else if same_account {
                    Some(NATIVE_REBIND_UNAVAILABLE)
                } else if is_active_host {
                    Some(ACCOUNT_MISMATCH_UNAVAILABLE)
                } else {
                    Some(SEMANTIC_HANDOFF_UNAVAILABLE)
                };
                controller.with_host_catalog_state(
                    catalog.clone(),
                    is_active_host,
                    active_model,
                    unavailable,
                )
            },
            Err(_) => {
                let account =
                    yo_core::derive_host_account_id(&observation.host, &[("local", "local")])
                        .expect("the fixed local host-account evidence is valid");
                controller
                    .with_host_status(
                        &observation.host,
                        host_label(&observation.host),
                        &account,
                        "local",
                        CATALOG_UNAVAILABLE,
                    )
                    .expect("the fixed host status projection is valid")
            },
        };
    }
    controller
}

fn inventory_requests(
    active: Option<(&HostId, DelegatedExecutionProfile)>,
    outer_sandbox_available: bool,
) -> [HostInventoryRequest; 2] {
    let profile_for = |host: &HostId| {
        active
            .filter(|(active, _)| *active == host)
            .map_or(DelegatedExecutionProfile::Standard, |(_, profile)| profile)
    };
    let codex = HostId::codex();
    let grok = HostId::grok();
    let grok_execution = profile_for(&grok);
    [
        HostInventoryRequest {
            execution: profile_for(&codex),
            host: codex,
            outer_sandboxed_review: false,
        },
        HostInventoryRequest {
            host: grok,
            execution: grok_execution,
            outer_sandboxed_review: outer_sandbox_available && grok_execution.is_read_only_review(),
        },
    ]
}

fn host_label(host: &HostId) -> &str {
    match host.as_str() {
        HostId::CODEX => "Codex",
        HostId::GROK => "Grok",
        _ => host.as_str(),
    }
}

#[cfg(test)]
mod tests;
