use std::path::Path;

use yo_core::{
    AgentCommand, SessionId, TranscriptRecord, TurnId,
    session_repository::{
        LocalSessionReader, StoredRequestTraceRecord, StoredSessionReader, read_stored_session,
        read_stored_session_continuation,
    },
};

use crate::{
    review_egress::AuthorizedDelivery,
    review_protocol::digest,
    review_session::{managed_binding_matches, provider_request_identity},
};

#[derive(Clone, Default)]
pub(super) struct SessionObservation {
    pub(super) session_id: Option<String>,
    pub(super) provider_request_count: usize,
    pub(super) provider_request_id: Option<String>,
    pub(super) turn_id: Option<TurnId>,
    pub(super) failure: Option<String>,
}

#[derive(Clone, Default)]
pub(super) struct ContinuationObservation {
    pub(super) provider_request_count: usize,
    pub(super) provider_request_id: Option<String>,
    pub(super) turn_id: Option<TurnId>,
    pub(super) continuation_anchor_sequence: Option<u64>,
    pub(super) failure: Option<String>,
}

pub(super) fn observe_session(
    root: &Path,
    packet: &[u8],
    delivery: &AuthorizedDelivery,
) -> SessionObservation {
    match observe_session_inner(root, packet, delivery) {
        Ok(observation) => observation,
        Err((observation, error)) => SessionObservation {
            failure: Some(error),
            ..observation
        },
    }
}

pub(super) fn observe_continuation(
    root: &Path,
    packet: &[u8],
    delivery: &AuthorizedDelivery,
    prior_anchor_sequence: u64,
    binding_epoch: u64,
) -> ContinuationObservation {
    match observe_continuation_inner(root, packet, delivery, prior_anchor_sequence, binding_epoch) {
        Ok(observation) => observation,
        Err((observation, error)) => ContinuationObservation {
            failure: Some(error),
            ..observation
        },
    }
}

fn observe_continuation_inner(
    root: &Path,
    packet: &[u8],
    delivery: &AuthorizedDelivery,
    prior_anchor_sequence: u64,
    binding_epoch: u64,
) -> Result<ContinuationObservation, (ContinuationObservation, String)> {
    let mut observation = ContinuationObservation::default();
    let session_id = delivery
        .session_id
        .as_deref()
        .ok_or_else(|| {
            (
                observation.clone(),
                "finding-resolution delivery has no reviewer Session".to_owned(),
            )
        })?
        .parse::<SessionId>()
        .map_err(|error| {
            (
                observation.clone(),
                format!("invalid reviewer Session identity: {error}"),
            )
        })?;
    let reader = LocalSessionReader::open(root).map_err(|error| {
        (
            observation.clone(),
            format!("cannot open continued durable Session repository: {error}"),
        )
    })?;
    let history = read_stored_session(&reader, session_id).map_err(|error| {
        (
            observation.clone(),
            format!("cannot recover continued durable Session: {error}"),
        )
    })?;
    let prior_packet_hash = delivery
        .prior_packet_hash
        .as_deref()
        .expect("preflight requires a prior packet hash");
    let starts = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::CommandCommitted(AgentCommand::StartTurn { input, .. }) => {
                Some(digest(input.as_str().as_bytes()))
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    if starts != [prior_packet_hash.to_owned(), digest(packet)] {
        return Err((
            observation,
            "continued durable Session does not contain exactly the prior and finding-resolution StartTurns"
                .to_owned(),
        ));
    }

    let mut binding_matches = Vec::new();
    let mut request_identities = Vec::new();
    let mut request_turns = Vec::new();
    let mut outcome_identities = Vec::new();
    let mut outcome_turns = Vec::new();
    let mut continuation_anchor_count = 0;
    for entry in history.request_trace() {
        match entry.record() {
            StoredRequestTraceRecord::BindingOpened {
                binding_identity, ..
            } => binding_matches.push(
                managed_binding_matches(
                    binding_identity.value(),
                    &delivery.provider,
                    &delivery.account,
                    &delivery.model,
                )
                .map_err(|error| (observation.clone(), error))?,
            ),
            StoredRequestTraceRecord::RequestAccepted {
                turn_id,
                request_identity,
                ..
            } => {
                request_turns.push(*turn_id);
                request_identities.push(request_identity.value().to_owned());
            },
            StoredRequestTraceRecord::ResumableOutcome {
                turn_id,
                outcome_identity,
                ..
            } => {
                outcome_turns.push(*turn_id);
                outcome_identities.push(
                    outcome_identity
                        .as_ref()
                        .map(|identity| identity.value().to_owned()),
                );
            },
            StoredRequestTraceRecord::ContinuationAnchor { .. } => {
                continuation_anchor_count += 1;
            },
            _ => {},
        }
    }
    observation.provider_request_count = request_identities.len().saturating_sub(1);
    if binding_matches != [true] {
        return Err((
            observation,
            "continued durable Session changed its authorized Provider/Account/Model binding"
                .to_owned(),
        ));
    }
    require_continuation_counts(
        request_identities.len(),
        outcome_identities.len(),
        continuation_anchor_count,
    )
    .map_err(|error| (observation.clone(), error))?;
    require_matching_turns(&request_turns, &outcome_turns)
        .map_err(|error| (observation.clone(), error))?;
    let prior_identity =
        provider_request_identity(&request_identities[..1], &outcome_identities[..1])
            .map_err(|error| (observation.clone(), error))?;
    if delivery.prior_provider_request_id.as_deref() != Some(prior_identity.as_str()) {
        return Err((
            observation,
            "continued durable Session changed the prior Provider request identity".to_owned(),
        ));
    }
    observation.provider_request_id = Some(
        provider_request_identity(&request_identities[1..], &outcome_identities[1..])
            .map_err(|error| (observation.clone(), error))?,
    );
    observation.turn_id = request_turns.get(1).copied();

    let continuation = read_stored_session_continuation(&reader, session_id).map_err(|error| {
        (
            observation.clone(),
            format!("continued Session has no new durable Continuation Anchor: {error}"),
        )
    })?;
    let anchor = continuation
        .target()
        .source_anchor_sequence()
        .ok_or_else(|| {
            (
                observation.clone(),
                "continued review Session has a checkpoint source, not a new Continuation Anchor"
                    .to_owned(),
            )
        })?
        .get();
    if continuation.target().epoch() != binding_epoch || anchor <= prior_anchor_sequence {
        return Err((
            observation,
            "continued Session did not advance one same-binding Continuation Anchor".to_owned(),
        ));
    }
    observation.continuation_anchor_sequence = Some(anchor);
    Ok(observation)
}

fn require_continuation_counts(
    request_count: usize,
    outcome_count: usize,
    anchor_count: usize,
) -> Result<(), String> {
    if request_count != 2 || outcome_count != 2 || anchor_count != 2 {
        return Err(format!(
            "continued durable Session has {request_count} accepted requests, {outcome_count} resumable outcomes, and {anchor_count} Continuation Anchors instead of two each"
        ));
    }
    Ok(())
}

fn observe_session_inner(
    root: &Path,
    packet: &[u8],
    delivery: &AuthorizedDelivery,
) -> Result<SessionObservation, (SessionObservation, String)> {
    let mut observation = SessionObservation::default();
    let reader = LocalSessionReader::open(root).map_err(|error| {
        (
            SessionObservation::default(),
            format!("cannot open isolated durable Session repository: {error}"),
        )
    })?;
    let sessions = reader.discover().map_err(|error| {
        (
            SessionObservation::default(),
            format!("cannot discover isolated durable Session: {error}"),
        )
    })?;
    if sessions.len() != 1 {
        return Err((
            observation.clone(),
            format!(
                "isolated delivery repository contains {} Sessions instead of exactly one",
                sessions.len()
            ),
        ));
    }
    let session = &sessions[0];
    observation.session_id = Some(session.session_id().to_string());
    if let Some(reason) = session.unavailable_reason() {
        return Err((
            observation.clone(),
            format!("isolated durable Session is unavailable: {reason}"),
        ));
    }
    let history = read_stored_session(&reader, session.session_id()).map_err(|error| {
        (
            observation.clone(),
            format!("cannot recover isolated durable Session: {error}"),
        )
    })?;
    let packet = std::str::from_utf8(packet).map_err(|error| {
        (
            observation.clone(),
            format!("verified review packet is not UTF-8: {error}"),
        )
    })?;
    let starts = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::CommandCommitted(AgentCommand::StartTurn { input, .. }) => {
                Some(input.as_str())
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    if starts != [packet] {
        return Err((
            observation,
            "durable Session does not contain exactly one byte-identical packet StartTurn"
                .to_owned(),
        ));
    }

    let mut binding_matches = Vec::new();
    let mut request_identities = Vec::new();
    let mut request_turns = Vec::new();
    let mut outcome_identities = Vec::new();
    let mut outcome_turns = Vec::new();
    for entry in history.request_trace() {
        match entry.record() {
            StoredRequestTraceRecord::BindingOpened {
                binding_identity, ..
            } => {
                binding_matches.push(
                    managed_binding_matches(
                        binding_identity.value(),
                        &delivery.provider,
                        &delivery.account,
                        &delivery.model,
                    )
                    .map_err(|error| (observation.clone(), error))?,
                );
            },
            StoredRequestTraceRecord::RequestAccepted {
                turn_id,
                request_identity,
                ..
            } => {
                request_turns.push(*turn_id);
                request_identities.push(request_identity.value().to_owned());
            },
            StoredRequestTraceRecord::ResumableOutcome {
                turn_id,
                outcome_identity,
                ..
            } => {
                outcome_turns.push(*turn_id);
                outcome_identities.push(
                    outcome_identity
                        .as_ref()
                        .map(|identity| identity.value().to_owned()),
                );
            },
            _ => {},
        }
    }
    observation.provider_request_count = request_identities.len();
    if binding_matches != [true] {
        let binding_count = binding_matches.len();
        return Err((
            observation,
            format!(
                "durable Session contains {} managed bindings instead of exactly one authorized Provider/Account/Model binding",
                binding_count
            ),
        ));
    }
    require_matching_turns(&request_turns, &outcome_turns)
        .map_err(|error| (observation.clone(), error))?;
    observation.provider_request_id = Some(
        provider_request_identity(&request_identities, &outcome_identities)
            .map_err(|error| (observation.clone(), error))?,
    );
    observation.turn_id = request_turns.first().copied();
    Ok(observation)
}

fn require_matching_turns(requests: &[TurnId], outcomes: &[TurnId]) -> Result<(), String> {
    if requests == outcomes {
        Ok(())
    } else {
        Err(
            "durable request and outcome records disagree on their exact turn identities"
                .to_owned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use yo_core::TurnId;

    use super::{require_continuation_counts, require_matching_turns};

    // finding-resolution 완료 관측은 original과 continuation 각각 하나의 accepted request,
    // resumable outcome, Anchor를 가져야 하며 어느 축의 누락·중복도 exact-once 성공으로
    // 축약하지 않습니다.
    #[test]
    fn continuation_shape_requires_two_records_on_every_request_axis() {
        assert!(require_continuation_counts(2, 2, 2).is_ok());
        for counts in [
            (1, 2, 2),
            (3, 2, 2),
            (2, 1, 2),
            (2, 3, 2),
            (2, 2, 1),
            (2, 2, 3),
        ] {
            assert!(require_continuation_counts(counts.0, counts.1, counts.2).is_err());
        }
    }

    // accepted request와 resumable outcome이 같은 개수더라도 turn이 다르면 usage를
    // 다른 요청에 귀속할 수 있으므로 delivery 관측을 성공시키지 않습니다.
    #[test]
    fn request_and_outcome_turns_must_match_exactly() {
        let first = TurnId::new(NonZeroU64::new(1).unwrap());
        let second = TurnId::new(NonZeroU64::new(2).unwrap());
        assert!(require_matching_turns(&[first], &[first]).is_ok());
        assert!(require_matching_turns(&[first], &[second]).is_err());
    }
}
