//! Authenticated Grok account-capacity and model-catalog observation.

use std::path::Path;

use serde_json::json;
use yo_core::{AccountCapacitySnapshot, BackendFailure, HostId, derive_host_account_id};

#[cfg(test)]
mod tests;

use crate::{
    admission::validate_config,
    billing_log,
    client::{AcpClient, combine_with_cleanup, initialize_and_authenticate},
    config::GrokBackendConfig,
    protocol,
    transport::{JsonPeer, StdioPeer},
};

/// Reads the current Grok account capacity and account identity without creating an Agent Session.
pub fn read_account_capacity(
    config: GrokBackendConfig,
) -> Result<AccountCapacitySnapshot, BackendFailure> {
    validate_config(&config)?;
    let peer = StdioPeer::spawn(&config)?;
    let mut client = AcpClient::new(peer, config.request_timeout());
    let observation = observe_account_capacity(&mut client, config.usage_log_path());
    let cleanup = client.shutdown();
    combine_with_cleanup(observation, cleanup)
}

/// Reads the exact authenticated Grok ACP model inventory without creating an Agent Session.
pub fn read_model_catalog(
    config: GrokBackendConfig,
) -> Result<yo_core::HostModelCatalog, BackendFailure> {
    validate_config(&config)?;
    let peer = StdioPeer::spawn(&config)?;
    let mut client = AcpClient::new(peer, config.request_timeout());
    let observation = observe_model_catalog(&mut client);
    let cleanup = client.shutdown();
    combine_with_cleanup(observation, cleanup)
}

fn observe_account_capacity<P: JsonPeer>(
    client: &mut AcpClient<P>,
    usage_log_path: Option<&Path>,
) -> Result<AccountCapacitySnapshot, BackendFailure> {
    let authenticated = initialize_and_authenticate(client)?;
    let (account_label, evidence) =
        protocol::decode_account_capacity_identity(&authenticated.authentication)?;
    let evidence_refs = evidence
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let account = derive_host_account_id(&HostId::grok(), &evidence_refs)
        .map_err(|error| protocol::protocol_failure(error.to_string()))?;
    // Validate the authenticated plan before making the optional billing read.
    protocol::decode_account_capacity(authenticated.authentication.clone(), None, account.clone())?;
    let usage = match client.call_optional("_x.ai/billing", json!({}))? {
        Some(billing) => billing_log::decode_billing_response(&billing.result)?,
        None => usage_log_path.and_then(|path| billing_log::read_latest_usage(path).ok().flatten()),
    };
    let snapshot = protocol::decode_account_capacity(authenticated.authentication, usage, account)?;
    Ok(snapshot.with_account_label(account_label))
}

fn observe_model_catalog<P: JsonPeer>(
    client: &mut AcpClient<P>,
) -> Result<yo_core::HostModelCatalog, BackendFailure> {
    let authenticated = initialize_and_authenticate(client)?;
    let (account_label, evidence) =
        protocol::decode_account_identity(&authenticated.authentication)?;
    let evidence_refs = evidence
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let host = HostId::grok();
    let account = derive_host_account_id(&host, &evidence_refs)
        .map_err(|error| protocol::protocol_failure(error.to_string()))?;
    let current = authenticated
        .initialized
        .current_model_id
        .map(yo_core::ModelId::new)
        .transpose()
        .map_err(|error| protocol::protocol_failure(error.to_string()))?;
    let mut ids = Vec::new();
    let models = authenticated
        .initialized
        .available_models
        .into_iter()
        .map(|(id, label)| {
            let id = yo_core::ModelId::new(id)
                .map_err(|error| protocol::protocol_failure(error.to_string()))?;
            ids.push(id.clone());
            yo_core::HostCatalogModel::selectable(id, label)
                .map_err(|error| protocol::protocol_failure(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let revision = yo_core::derive_host_catalog_revision(&host, &account, current.as_ref(), &ids);
    yo_core::HostModelCatalog::new(
        host,
        "Grok",
        account,
        account_label,
        revision,
        current,
        models,
    )
    .map_err(|error| protocol::protocol_failure(error.to_string()))
}
