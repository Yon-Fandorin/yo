use std::path::Path;

use yo_core::{
    AgentCommand, SessionId, TranscriptRecord, TurnId,
    session_repository::{
        LocalSessionReader, StoredRequestTraceRecord, StoredSessionReader, read_stored_session,
        read_stored_session_continuation,
    },
};

use crate::{
    review::{
        egress::AuthorizedHostDelivery,
        session::{
            delegated_backend_kind_matches, delegated_binding_matches, host_request_identity,
        },
    },
    review_protocol::digest,
};

#[derive(Clone, Default)]
pub(super) struct HostSessionObservation {
    pub(super) session_id: Option<String>,
    pub(super) host_request_count: usize,
    pub(super) host_request_id: Option<String>,
    pub(super) turn_id: Option<TurnId>,
    pub(super) failure: Option<String>,
}

#[derive(Clone, Default)]
pub(super) struct HostContinuationObservation {
    pub(super) host_request_count: usize,
    pub(super) host_request_id: Option<String>,
    pub(super) turn_id: Option<TurnId>,
    pub(super) continuation_anchor_sequence: Option<u64>,
    pub(super) failure: Option<String>,
}

struct HostTraceObservation {
    binding_matches: Vec<bool>,
    requests: Vec<String>,
    request_turns: Vec<TurnId>,
    outcomes: Vec<Option<String>>,
    outcome_turns: Vec<TurnId>,
    anchor_count: usize,
}

pub(super) fn observe_host_session(
    root: &Path,
    packet: &[u8],
    delivery: &AuthorizedHostDelivery,
) -> HostSessionObservation {
    match observe_host_session_inner(root, packet, delivery) {
        Ok(observation) => observation,
        Err((observation, error)) => HostSessionObservation {
            failure: Some(error),
            ..observation
        },
    }
}

pub(super) fn observe_host_continuation(
    root: &Path,
    packet: &[u8],
    delivery: &AuthorizedHostDelivery,
    prior_anchor_sequence: u64,
    binding_epoch: u64,
) -> HostContinuationObservation {
    match observe_host_continuation_inner(
        root,
        packet,
        delivery,
        prior_anchor_sequence,
        binding_epoch,
    ) {
        Ok(observation) => observation,
        Err((observation, error)) => HostContinuationObservation {
            failure: Some(error),
            ..observation
        },
    }
}

fn observe_host_session_inner(
    root: &Path,
    packet: &[u8],
    delivery: &AuthorizedHostDelivery,
) -> Result<HostSessionObservation, (HostSessionObservation, String)> {
    let mut observation = HostSessionObservation::default();
    let reader = LocalSessionReader::open(root).map_err(|error| {
        (
            observation.clone(),
            format!("cannot open isolated delegated Session repository: {error}"),
        )
    })?;
    let sessions = reader.discover().map_err(|error| {
        (
            observation.clone(),
            format!("cannot discover isolated delegated Session: {error}"),
        )
    })?;
    if sessions.len() != 1 {
        return Err((
            observation,
            format!(
                "isolated delegated repository contains {} Sessions instead of exactly one",
                sessions.len()
            ),
        ));
    }
    let session = &sessions[0];
    observation.session_id = Some(session.session_id().to_string());
    if let Some(reason) = session.unavailable_reason() {
        return Err((
            observation,
            format!("isolated delegated Session is unavailable: {reason}"),
        ));
    }
    let history = read_stored_session(&reader, session.session_id()).map_err(|error| {
        (
            observation.clone(),
            format!("cannot recover isolated delegated Session: {error}"),
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
            "delegated Session does not contain exactly one byte-identical packet StartTurn"
                .to_owned(),
        ));
    }

    let trace = observe_trace(&history, delivery).map_err(|error| (observation.clone(), error))?;
    observation.host_request_count = trace.requests.len();
    if trace.binding_matches != [true] {
        return Err((
            observation,
            "delegated Session does not contain exactly one matching host review binding"
                .to_owned(),
        ));
    }
    require_matching_turns(&trace).map_err(|error| (observation.clone(), error))?;
    observation.host_request_id = Some(
        host_request_identity(&trace.requests, &trace.outcomes)
            .map_err(|error| (observation.clone(), error))?,
    );
    observation.turn_id = trace.request_turns.first().copied();
    Ok(observation)
}

fn observe_host_continuation_inner(
    root: &Path,
    packet: &[u8],
    delivery: &AuthorizedHostDelivery,
    prior_anchor_sequence: u64,
    binding_epoch: u64,
) -> Result<HostContinuationObservation, (HostContinuationObservation, String)> {
    let mut observation = HostContinuationObservation::default();
    let session_id = delivery
        .session_id
        .as_deref()
        .ok_or_else(|| {
            (
                observation.clone(),
                "delegated finding-resolution has no reviewer Session".to_owned(),
            )
        })?
        .parse::<SessionId>()
        .map_err(|error| {
            (
                observation.clone(),
                format!("invalid delegated reviewer Session identity: {error}"),
            )
        })?;
    let reader = LocalSessionReader::open(root).map_err(|error| {
        (
            observation.clone(),
            format!("cannot open continued delegated Session repository: {error}"),
        )
    })?;
    let history = read_stored_session(&reader, session_id).map_err(|error| {
        (
            observation.clone(),
            format!("cannot recover continued delegated Session: {error}"),
        )
    })?;
    let prior_packet_hash = delivery
        .prior_packet_hash
        .as_deref()
        .expect("delegated preflight requires a prior packet hash");
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
            "continued delegated Session does not contain exactly the prior and finding-resolution StartTurns"
                .to_owned(),
        ));
    }
    let trace = observe_trace(&history, delivery).map_err(|error| (observation.clone(), error))?;
    observation.host_request_count = trace.requests.len().saturating_sub(1);
    if trace.binding_matches != [true] {
        return Err((
            observation,
            "continued delegated Session changed its authorized host review binding".to_owned(),
        ));
    }
    if trace.requests.len() != 2 || trace.outcomes.len() != 2 || trace.anchor_count != 2 {
        return Err((
            observation,
            format!(
                "continued delegated Session has {} accepted requests, {} resumable outcomes, and {} Continuation Anchors instead of two each",
                trace.requests.len(),
                trace.outcomes.len(),
                trace.anchor_count
            ),
        ));
    }
    require_matching_turns(&trace).map_err(|error| (observation.clone(), error))?;
    let prior = host_request_identity(&trace.requests[..1], &trace.outcomes[..1])
        .map_err(|error| (observation.clone(), error))?;
    if delivery.prior_host_request_id.as_deref() != Some(prior.as_str()) {
        return Err((
            observation,
            "continued delegated Session changed the prior host request identity".to_owned(),
        ));
    }
    observation.host_request_id = Some(
        host_request_identity(&trace.requests[1..], &trace.outcomes[1..])
            .map_err(|error| (observation.clone(), error))?,
    );
    observation.turn_id = trace.request_turns.get(1).copied();
    let continuation = read_stored_session_continuation(&reader, session_id).map_err(|error| {
        (
            observation.clone(),
            format!("continued delegated Session has no new Continuation Anchor: {error}"),
        )
    })?;
    let anchor = continuation
        .target()
        .source_anchor_sequence()
        .ok_or_else(|| {
            (
                observation.clone(),
                "continued delegated review Session has a checkpoint source, not a new Continuation Anchor"
                    .to_owned(),
            )
        })?
        .get();
    if continuation.target().epoch() != binding_epoch || anchor <= prior_anchor_sequence {
        return Err((
            observation,
            "continued delegated Session did not advance one same-binding Continuation Anchor"
                .to_owned(),
        ));
    }
    observation.continuation_anchor_sequence = Some(anchor);
    Ok(observation)
}

fn observe_trace(
    history: &yo_core::session_repository::StoredSessionHistory,
    delivery: &AuthorizedHostDelivery,
) -> Result<HostTraceObservation, String> {
    let mut bindings = Vec::new();
    let mut requests = Vec::new();
    let mut request_turns = Vec::new();
    let mut outcomes = Vec::new();
    let mut outcome_turns = Vec::new();
    let mut anchors = 0;
    for entry in history.request_trace() {
        match entry.record() {
            StoredRequestTraceRecord::BindingOpened {
                backend_kind,
                binding_identity,
                ..
            } => bindings.push(
                delegated_backend_kind_matches(backend_kind, &delivery.host)
                    && delegated_binding_matches(
                        binding_identity.schema(),
                        binding_identity.value(),
                        &delivery.host,
                        &delivery.execution_profile,
                    )?,
            ),
            StoredRequestTraceRecord::RequestAccepted {
                turn_id,
                request_identity,
                ..
            } => {
                request_turns.push(*turn_id);
                requests.push(request_identity.value().to_owned());
            },
            StoredRequestTraceRecord::ResumableOutcome {
                turn_id,
                outcome_identity,
                ..
            } => {
                outcome_turns.push(*turn_id);
                outcomes.push(
                    outcome_identity
                        .as_ref()
                        .map(|identity| identity.value().to_owned()),
                );
            },
            StoredRequestTraceRecord::ContinuationAnchor { .. } => anchors += 1,
            _ => {},
        }
    }
    Ok(HostTraceObservation {
        binding_matches: bindings,
        requests,
        request_turns,
        outcomes,
        outcome_turns,
        anchor_count: anchors,
    })
}

fn require_matching_turns(trace: &HostTraceObservation) -> Result<(), String> {
    if trace.request_turns == trace.outcome_turns {
        Ok(())
    } else {
        Err(
            "durable delegated request and outcome records disagree on their exact turn identities"
                .to_owned(),
        )
    }
}
