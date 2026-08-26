use std::path::Path;

use yo_core::{
    AgentCommand, TranscriptRecord,
    session_repository::{
        LocalSessionReader, StoredRequestTraceRecord, StoredSessionReader, read_stored_session,
    },
};

use crate::{
    review_egress::AuthorizedDelivery,
    review_session::{managed_binding_matches, provider_request_identity},
};

#[derive(Clone, Default)]
pub(super) struct SessionObservation {
    pub(super) session_id: Option<String>,
    pub(super) provider_request_count: usize,
    pub(super) provider_request_id: Option<String>,
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
    let mut outcome_identities = Vec::new();
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
                request_identity, ..
            } => request_identities.push(request_identity.value().to_owned()),
            StoredRequestTraceRecord::ResumableOutcome {
                outcome_identity, ..
            } => outcome_identities.push(
                outcome_identity
                    .as_ref()
                    .map(|identity| identity.value().to_owned()),
            ),
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
    observation.provider_request_id = Some(
        provider_request_identity(&request_identities, &outcome_identities)
            .map_err(|error| (observation.clone(), error))?,
    );
    Ok(observation)
}
