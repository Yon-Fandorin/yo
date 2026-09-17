use yo_core::{
    AgentCommand, TranscriptRecord, session_repository as core_session_repository,
    session_repository::StoredRequestTraceRecord,
};

use super::Observation;
use crate::{
    review_egress::AuthorizedHostDelivery,
    review_protocol::digest,
    review_session::{
        delegated_backend_kind_matches, delegated_binding_matches, host_request_identity,
    },
};

pub(super) fn observe_history(
    history: &core_session_repository::StoredSessionHistory,
    delivery: &AuthorizedHostDelivery,
) -> Result<Observation, String> {
    let start_packet_hashes = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::CommandCommitted(AgentCommand::StartTurn { input, .. }) => {
                Some(digest(input.as_str().as_bytes()))
            },
            _ => None,
        })
        .collect();
    let mut binding_matches = Vec::new();
    let mut request_identities = Vec::new();
    let mut outcome_identities = Vec::new();
    let mut continuation_anchors = Vec::new();
    for entry in history.request_trace() {
        match entry.record() {
            StoredRequestTraceRecord::BindingOpened {
                backend_kind,
                binding_identity,
                ..
            } => binding_matches.push(
                delegated_backend_kind_matches(backend_kind, &delivery.host)
                    && delegated_binding_matches(
                        binding_identity.schema(),
                        binding_identity.value(),
                        &delivery.host,
                        &delivery.execution_profile,
                    )?,
            ),
            StoredRequestTraceRecord::RequestAccepted {
                request_identity, ..
            } => request_identities.push(request_identity.value().to_owned()),
            StoredRequestTraceRecord::ResumableOutcome {
                outcome_identity, ..
            } => outcome_identities.push(
                outcome_identity
                    .as_ref()
                    .map(|identity| identity.value().to_owned()),
            ),
            StoredRequestTraceRecord::ContinuationAnchor {
                epoch,
                accepted_request_sequence,
                resumable_outcome_sequence,
                journal_boundary,
            } => continuation_anchors.push((
                *epoch,
                accepted_request_sequence.get(),
                resumable_outcome_sequence.get(),
                journal_boundary.get(),
            )),
            _ => {},
        }
    }
    Ok(Observation {
        start_packet_hashes,
        binding_matches,
        request_identities,
        outcome_identities,
        continuation_anchors,
    })
}

pub(super) fn validate_observation(
    observation: &Observation,
    expected_packet_hash: &str,
    expected_host_request_id: &str,
) -> Result<(), String> {
    if observation.start_packet_hashes != [expected_packet_hash] {
        return Err(
            "delegated reviewer Session does not contain exactly one matching original StartTurn"
                .to_owned(),
        );
    }
    if observation.binding_matches != [true] {
        return Err(
            "delegated reviewer Session does not contain exactly one matching host review binding"
                .to_owned(),
        );
    }
    let observed = host_request_identity(
        &observation.request_identities,
        &observation.outcome_identities,
    )?;
    if observed != expected_host_request_id {
        return Err(
            "delegated reviewer Session changed the prior host request identity".to_owned(),
        );
    }
    if observation.continuation_anchors.len() != 1 {
        return Err(
            "delegated reviewer Session must contain exactly one Continuation Anchor".to_owned(),
        );
    }
    Ok(())
}
