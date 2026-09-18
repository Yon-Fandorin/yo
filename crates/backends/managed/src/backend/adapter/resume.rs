//! 재개와 바인딩 교체 승인.

use yo_core::{
    BackendBindingEvidence, BackendFailure, BackendFailureKind, BackendResumeTarget,
    ModelReplayItem,
};

use super::super::{
    NativeModelBackend, failure,
    identity::{same_execution_manifest, semantically_equal_native_binding_identity},
};

pub(super) fn resume_session(
    backend: &mut NativeModelBackend,
    target: &BackendResumeTarget,
) -> Result<BackendBindingEvidence, BackendFailure> {
    if backend.closed || backend.session.is_some() || backend.turn.is_some() {
        return Err(failure(
            BackendFailureKind::Session,
            "native backend is not available for resume",
        ));
    }
    let expected = backend.binding_evidence(target.session_id());
    let Some(contract) = target.model_replay().contract() else {
        return Err(failure(
            BackendFailureKind::Session,
            "durable native replay has no contract",
        ));
    };
    if !same_native_resume_identity(&expected, target.binding())
        || (contract != &backend.contract && contract != &backend.legacy_contract)
    {
        return Err(failure(
            BackendFailureKind::Session,
            "durable native model binding or replay contract does not match current configuration",
        ));
    }
    backend.secret_interaction_enabled = contract == &backend.contract;
    backend.contract = contract.clone();
    backend.session = Some(target.session_id());
    backend.replay = target.model_replay().clone();
    backend.restore_context_state(target)?;
    Ok(target.binding().clone())
}

pub(super) fn resume_session_replacing_binding(
    backend: &mut NativeModelBackend,
    target: &BackendResumeTarget,
) -> Result<BackendBindingEvidence, BackendFailure> {
    if backend.closed || backend.session.is_some() || backend.turn.is_some() {
        return Err(failure(
            BackendFailureKind::Session,
            "native backend is not available for binding replacement",
        ));
    }
    let Some(contract) = target.model_replay().contract() else {
        return Err(failure(
            BackendFailureKind::Session,
            "durable exact replay has no contract",
        ));
    };
    if contract != &backend.contract && contract != &backend.legacy_contract {
        return Err(failure(
            BackendFailureKind::Session,
            "durable exact replay contract does not match the replacement binding",
        ));
    }
    backend.secret_interaction_enabled = contract != &backend.legacy_contract;
    backend.contract = contract.clone();
    if backend.image_accounting.is_none()
        && target
            .model_replay()
            .items()
            .iter()
            .any(|item| matches!(item, ModelReplayItem::MultimodalUser { .. }))
    {
        return Err(failure(
            BackendFailureKind::Unsupported,
            "The replacement binding cannot replay retained image input",
        ));
    }
    if !same_execution_manifest(
        &backend.binding_identity,
        target.binding().binding_identity(),
    ) {
        return Err(failure(
            BackendFailureKind::Session,
            "durable command execution manifest does not match the replacement binding",
        ));
    }
    backend.session = Some(target.session_id());
    backend.replay = target.model_replay().clone();
    backend.restore_context_state(target)?;
    Ok(backend.binding_evidence(target.session_id()))
}

fn same_native_resume_identity(
    current: &BackendBindingEvidence,
    durable: &BackendBindingEvidence,
) -> bool {
    current.backend_kind() == durable.backend_kind()
        && current.model_identity() == durable.model_identity()
        && current.session_locator() == durable.session_locator()
        && current.continuation_strategy() == durable.continuation_strategy()
        && semantically_equal_native_binding_identity(
            current.binding_identity(),
            durable.binding_identity(),
        )
}
