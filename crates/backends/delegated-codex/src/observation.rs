//! One-shot account capacity and model inventory reads without an Agent Session.

use std::collections::HashSet;

use serde_json::{Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    AccountCapacitySnapshot, AccountId, BackendFailure, HostId, ModelId, derive_host_account_id,
};

use crate::{
    CodexBackendConfig, CodexCompatibilityWarning, CodexWarningObserver, client::AppServerClient,
    config::validate_config, protocol, transport::StdioPeer,
};

/// A one-shot Codex read and the compatibility warning observed during its handshake.
#[derive(Debug)]
pub struct CodexRead<T> {
    value: T,
    compatibility_warning: Option<CodexCompatibilityWarning>,
}

impl<T> CodexRead<T> {
    fn new(value: T, compatibility_warning: Option<CodexCompatibilityWarning>) -> Self {
        Self {
            value,
            compatibility_warning,
        }
    }

    /// Borrows the one-shot value without discarding its warning metadata.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Splits the read value from its optional compatibility warning.
    pub fn into_parts(self) -> (T, Option<CodexCompatibilityWarning>) {
        (self.value, self.compatibility_warning)
    }
}

/// Reads the current Codex account capacity and account identity without creating an Agent
/// Session.
pub fn read_account_capacity(
    config: CodexBackendConfig,
) -> Result<CodexRead<AccountCapacitySnapshot>, BackendFailure> {
    validate_config(&config)?;
    let peer = StdioPeer::spawn(&config)?;
    let mut client = AppServerClient::new(peer, config.request_timeout());
    let observation = observe_account_capacity(&mut client);
    let cleanup = client.shutdown();
    match (observation, cleanup) {
        (Ok(snapshot), Ok(())) => Ok(snapshot),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(observation), Ok(())) => Err(observation),
        (Err(observation), Err(cleanup)) => Err(BackendFailure::new(
            observation.kind(),
            format!(
                "{}; cleanup also failed: {}",
                observation.message(),
                cleanup
            ),
        )),
    }
}

/// Reads the authenticated Codex account and complete visible app-server model inventory.
pub fn read_model_catalog(
    config: CodexBackendConfig,
) -> Result<yo_core::HostModelCatalog, BackendFailure> {
    read_model_catalog_with_warning_observer(config, None)
}

/// Reads the authenticated Codex model inventory and forwards compatibility observations.
pub fn read_model_catalog_with_warning_observer(
    config: CodexBackendConfig,
    warning_observer: Option<CodexWarningObserver>,
) -> Result<yo_core::HostModelCatalog, BackendFailure> {
    validate_config(&config)?;
    let peer = StdioPeer::spawn(&config)?;
    let mut client = AppServerClient::new(peer, config.request_timeout())
        .with_warning_observer(warning_observer);
    let observation = observe_model_catalog(&mut client);
    let cleanup = client.shutdown();
    match (observation, cleanup) {
        (Ok(catalog), Ok(())) => Ok(catalog),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(observation), Ok(())) => Err(observation),
        (Err(observation), Err(cleanup)) => Err(BackendFailure::new(
            observation.kind(),
            format!("{}; cleanup also failed: {cleanup}", observation.message()),
        )),
    }
}

fn observe_account_capacity<P: JsonMessagePeer>(
    client: &mut AppServerClient<P>,
) -> Result<CodexRead<AccountCapacitySnapshot>, BackendFailure> {
    let initialize = client.initialize()?;
    let account_result = client
        .call("account/read", json!({ "refreshToken": false }))?
        .result;
    let (account_label, evidence) = protocol::decode_account_capacity_identity(&account_result)?;
    let evidence_refs = evidence
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    if !matches!(
        evidence.first().map(|(key, _)| key.as_str()),
        Some("account_id" | "email")
    ) {
        return Err(protocol::protocol_failure(
            "Codex account/read response has no stable email account identity",
        ));
    }
    let account = derive_host_account_id(&HostId::codex(), &evidence_refs)
        .map_err(|error| protocol::protocol_failure(error.to_string()))?;
    let result = client.call("account/rateLimits/read", Value::Null)?.result;
    let snapshot = protocol::decode_account_capacity(result, account)?;
    Ok(CodexRead::new(
        snapshot.with_account_label(account_label),
        initialize.compatibility_warning,
    ))
}

fn observe_model_catalog<P: JsonMessagePeer>(
    client: &mut AppServerClient<P>,
) -> Result<yo_core::HostModelCatalog, BackendFailure> {
    const PAGE_LIMIT: u64 = 100;
    const MAX_MODELS: usize = 4096;

    client.initialize()?;
    let account_result = client
        .call("account/read", json!({ "refreshToken": false }))?
        .result;
    let host = HostId::codex();
    let (account_label, account) = decode_account(&host, &account_result)?;

    let mut cursor = None::<String>;
    let mut seen_cursors = HashSet::new();
    let mut seen_models = HashSet::new();
    let mut models = Vec::new();
    let mut ids = Vec::new();
    let mut current = None;
    loop {
        let mut params = json!({ "limit": PAGE_LIMIT, "includeHidden": false });
        if let Some(cursor) = cursor.as_ref() {
            params["cursor"] = Value::String(cursor.clone());
        }
        let page = protocol::decode_model_list(client.call("model/list", params)?.result)?;
        for (id, label, is_default) in page.models {
            if models.len() == MAX_MODELS || !seen_models.insert(id.clone()) {
                return Err(protocol::protocol_failure(
                    "Codex model/list exceeded the model bound or repeated a model id",
                ));
            }
            let id =
                ModelId::new(id).map_err(|error| protocol::protocol_failure(error.to_string()))?;
            if is_default && current.replace(id.clone()).is_some() {
                return Err(protocol::protocol_failure(
                    "Codex model/list advertised more than one default model",
                ));
            }
            ids.push(id.clone());
            models.push(
                yo_core::HostCatalogModel::selectable(id, label)
                    .map_err(|error| protocol::protocol_failure(error.to_string()))?,
            );
        }
        let Some(next) = page.next_cursor else {
            break;
        };
        if !seen_cursors.insert(next.clone()) {
            return Err(protocol::protocol_failure(
                "Codex model/list repeated a pagination cursor",
            ));
        }
        cursor = Some(next);
    }
    let revision = yo_core::derive_host_catalog_revision(&host, &account, current.as_ref(), &ids);
    yo_core::HostModelCatalog::new(
        host,
        "Codex",
        account,
        account_label,
        revision,
        current,
        models,
    )
    .map_err(|error| protocol::protocol_failure(error.to_string()))
}

fn decode_account(host: &HostId, result: &Value) -> Result<(String, AccountId), BackendFailure> {
    let (account_label, evidence) = protocol::decode_account_identity(result)?;
    let evidence_refs = evidence
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let account = derive_host_account_id(host, &evidence_refs)
        .map_err(|error| protocol::protocol_failure(error.to_string()))?;
    Ok((account_label, account))
}

#[cfg(test)]
mod tests;
