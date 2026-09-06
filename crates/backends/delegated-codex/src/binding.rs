//! Durable Codex binding admission and native-model rebind decoding.

use serde_json::Value;
use yo_core::{
    AccountId, BackendBindingEvidence, BackendFailure, BackendFailureKind, BackendIdentity, HostId,
    ModelId, derive_host_account_id,
};

use crate::{
    BACKEND_KIND, LEGACY_READ_ONLY_BINDING_SCHEMA, LEGACY_STANDARD_BINDING_SCHEMA,
    MODEL_IDENTITY_SCHEMA, READ_ONLY_BINDING_SCHEMA, STANDARD_BINDING_SCHEMA, protocol,
};

/// Exact account and model recovered from a rebind-capable Codex binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexNativeModelBinding {
    account: AccountId,
    model: ModelId,
}

impl CodexNativeModelBinding {
    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    pub const fn model(&self) -> &ModelId {
        &self.model
    }
}

/// Decodes only the account-bearing binding schema that can authorize native model rebind.
pub fn native_model_binding(
    binding: &BackendBindingEvidence,
) -> Result<Option<CodexNativeModelBinding>, BackendFailure> {
    native_model_binding_from_parts(
        binding.backend_kind(),
        binding.binding_identity(),
        binding.model_identity(),
    )
}

/// Decodes the same account-bearing Codex binding from one live Request-trace fact.
pub fn native_model_binding_from_trace(
    record: &yo_core::RequestTraceRecord,
) -> Result<Option<CodexNativeModelBinding>, BackendFailure> {
    let yo_core::RequestTraceRecord::BindingOpened {
        backend_kind,
        binding_identity,
        model_identity,
        ..
    } = record
    else {
        return Ok(None);
    };
    native_model_binding_from_parts(backend_kind, binding_identity, model_identity)
}

fn native_model_binding_from_parts(
    backend_kind: &str,
    binding_identity: &BackendIdentity,
    model_identity: &BackendIdentity,
) -> Result<Option<CodexNativeModelBinding>, BackendFailure> {
    if backend_kind != BACKEND_KIND {
        return Ok(None);
    }
    let Some(account) = binding_account(binding_identity)? else {
        return Ok(None);
    };
    let (model, _) = model_and_provider(model_identity)?;
    Ok(Some(CodexNativeModelBinding { account, model }))
}

pub(crate) fn decode_optional_account(
    host: &HostId,
    result: &Value,
) -> Result<Option<AccountId>, BackendFailure> {
    let Some((_, evidence)) = protocol::decode_optional_account_identity(result) else {
        return Ok(None);
    };
    let evidence_refs = evidence
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    derive_host_account_id(host, &evidence_refs)
        .map(Some)
        .map_err(|error| protocol::protocol_failure(error.to_string()))
}

pub(crate) fn binding_account(
    identity: &BackendIdentity,
) -> Result<Option<AccountId>, BackendFailure> {
    match identity.schema() {
        LEGACY_STANDARD_BINDING_SCHEMA | LEGACY_READ_ONLY_BINDING_SCHEMA => Ok(None),
        STANDARD_BINDING_SCHEMA | READ_ONLY_BINDING_SCHEMA => {
            let value: Value = serde_json::from_str(identity.value())
                .map_err(|_| protocol::protocol_failure("Codex binding identity is malformed"))?;
            let account = value
                .get("accountId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    protocol::protocol_failure("Codex binding identity has no accountId")
                })?;
            AccountId::new(account)
                .map(Some)
                .map_err(|error| protocol::protocol_failure(error.to_string()))
        },
        _ => Err(BackendFailure::new(
            BackendFailureKind::Unsupported,
            format!("unsupported Codex binding identity `{}`", identity.schema()),
        )),
    }
}

pub(crate) fn model_and_provider(
    identity: &BackendIdentity,
) -> Result<(ModelId, String), BackendFailure> {
    if identity.schema() != MODEL_IDENTITY_SCHEMA {
        return Err(BackendFailure::new(
            BackendFailureKind::Unsupported,
            format!("unsupported Codex model identity `{}`", identity.schema()),
        ));
    }
    let value: Value = serde_json::from_str(identity.value())
        .map_err(|_| protocol::protocol_failure("Codex model identity is malformed"))?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol::protocol_failure("Codex model identity has no exact model"))?;
    let provider = value
        .get("provider")
        .and_then(Value::as_str)
        .filter(|provider| !provider.is_empty())
        .ok_or_else(|| protocol::protocol_failure("Codex model identity has no exact provider"))?;
    let model =
        ModelId::new(model).map_err(|error| protocol::protocol_failure(error.to_string()))?;
    Ok((model, provider.to_owned()))
}
