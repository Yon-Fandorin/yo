use std::path::Path;

use yo_core::{HostId, HostModelCatalog, ModelSelectionController};

use super::DelegatedExecutionProfile;

const CATALOG_UNAVAILABLE: &str = "model catalog unavailable";

#[derive(Clone, Debug)]
pub(crate) struct HostCatalogObservation {
    host: HostId,
    catalog: Result<HostModelCatalog, String>,
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

/// Projects every observed host independently so one missing binary or account cannot suppress a
/// sibling host section. Only the live host is allowed to mark an advertised model as current.
pub(crate) fn project_host_catalogs(
    mut controller: ModelSelectionController,
    active_host: Option<&HostId>,
    observations: &[HostCatalogObservation],
) -> ModelSelectionController {
    for observation in observations {
        let active = active_host == Some(&observation.host);
        controller = match &observation.catalog {
            Ok(catalog) => controller.with_host_catalog(catalog.clone(), active),
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
