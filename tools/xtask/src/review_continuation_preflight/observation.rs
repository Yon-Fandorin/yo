use yo_core::{
    AgentCommand, TranscriptRecord, session_repository as core_session_repository,
    session_repository::StoredRequestTraceRecord,
};

use super::Observation;
use crate::{
    review_egress::AuthorizedDelivery, review_protocol::digest,
    review_session::managed_binding_matches,
};

pub(super) fn observe_history(
    history: &core_session_repository::StoredSessionHistory,
    delivery: &AuthorizedDelivery,
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
                binding_identity, ..
            } => binding_matches.push(managed_binding_matches(
                binding_identity.value(),
                &delivery.provider,
                &delivery.account,
                &delivery.model,
            )?),
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
