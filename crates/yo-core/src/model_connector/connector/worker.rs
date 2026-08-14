use std::{sync::mpsc, thread, time::Instant};

use futures_util::StreamExt;
use reqwest::{Client, Url, header};
use serde_json::Value;
use tokio::sync::mpsc as async_mpsc;

use super::{
    super::{ConnectorError, ConnectorFailureKind, ResponsesConnectorLimits, ResponsesEvent},
    EVENT_QUEUE_CAPACITY, ResponsesCancellation, ResponsesStream, SseDecoder,
    failure::{
        cancellable_timeout, cancelled_failure, http_status_failure, limit_failure,
        map_reqwest_error, phase_timeout, protocol_failure, record_body_progress,
        transport_failure,
    },
    transport::{Origin, validate_redirect},
};
use crate::ApiCredential;

#[allow(clippy::too_many_arguments)]
pub(super) fn start_stream(
    client: Client,
    request_url: Url,
    credential: ApiCredential,
    limits: ResponsesConnectorLimits,
    body: Value,
    decoder: Box<dyn SseDecoder>,
    cancellation: ResponsesCancellation,
    worker_name: &'static str,
) -> Result<ResponsesStream, ConnectorError> {
    if cancellation.is_cancelled() {
        return Err(cancelled_failure());
    }
    let request_started = Instant::now();
    let (acceptance_sender, acceptance_receiver) = mpsc::sync_channel(1);
    let (event_sender, event_receiver) = async_mpsc::channel(EVENT_QUEUE_CAPACITY);
    let (outcome_sender, outcome_receiver) = mpsc::sync_channel(1);
    let worker_cancellation = cancellation.clone();
    let worker = thread::Builder::new()
        .name(worker_name.to_owned())
        .spawn(move || {
            run_worker(
                client,
                request_url,
                credential,
                limits,
                body,
                decoder,
                worker_cancellation,
                request_started,
                acceptance_sender,
                event_sender,
                outcome_sender,
            );
        })
        .map_err(|_| transport_failure("cannot start the model-connector request worker"))?;

    match acceptance_receiver.recv() {
        Ok(Ok(())) => Ok(ResponsesStream {
            receiver: event_receiver,
            outcome: outcome_receiver,
            cancellation,
            worker: Some(worker),
            closed: false,
        }),
        Ok(Err(error)) => {
            let _ = worker.join();
            Err(error)
        },
        Err(_) => {
            let _ = worker.join();
            Err(transport_failure(
                "model-connector request worker stopped before HTTP acceptance",
            ))
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn run_worker(
    client: Client,
    request_url: Url,
    credential: ApiCredential,
    limits: ResponsesConnectorLimits,
    body: Value,
    decoder: Box<dyn SseDecoder>,
    cancellation: ResponsesCancellation,
    request_started: Instant,
    acceptance_sender: mpsc::SyncSender<Result<(), ConnectorError>>,
    event_sender: async_mpsc::Sender<ResponsesEvent>,
    outcome_sender: mpsc::SyncSender<Result<(), ConnectorError>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            let _ = acceptance_sender.send(Err(ConnectorError::new(
                ConnectorFailureKind::Transport,
                "cannot initialize the model-connector request runtime",
            )));
            return;
        },
    };
    let mut acceptance_sender = Some(acceptance_sender);
    let result = runtime.block_on(execute_request(
        &client,
        request_url,
        &credential,
        &limits,
        &body,
        decoder,
        &cancellation,
        request_started,
        &mut acceptance_sender,
        &event_sender,
    ));
    match (acceptance_sender.take(), result) {
        (Some(sender), Err(error)) => {
            let _ = sender.send(Err(error));
        },
        (Some(sender), Ok(())) => {
            let _ = sender.send(Err(transport_failure(
                "model-connector request completed before HTTP acceptance",
            )));
        },
        (None, result) => {
            let _ = outcome_sender.send(result);
        },
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_request(
    client: &Client,
    request_url: Url,
    credential: &ApiCredential,
    limits: &ResponsesConnectorLimits,
    body: &Value,
    mut decoder: Box<dyn SseDecoder>,
    cancellation: &ResponsesCancellation,
    request_started: Instant,
    acceptance_sender: &mut Option<mpsc::SyncSender<Result<(), ConnectorError>>>,
    event_sender: &async_mpsc::Sender<ResponsesEvent>,
) -> Result<(), ConnectorError> {
    let origin = Origin::from_url(&request_url)?;
    let mut attempt_url = request_url;
    let mut followed_redirects = 0_usize;
    let response = loop {
        let response_header_started = Instant::now();
        let response = cancellable_timeout(
            cancellation,
            phase_timeout(
                request_started,
                limits.absolute_request_timeout,
                response_header_started,
                limits.response_header_timeout,
                "model-connector response-header deadline expired",
            )?,
            client
                .post(attempt_url.clone())
                .bearer_auth(credential.expose_secret())
                .header(header::ACCEPT, "text/event-stream")
                .json(body)
                .send(),
        )
        .await?
        .map_err(map_reqwest_error)?;
        if !matches!(
            response.status(),
            reqwest::StatusCode::TEMPORARY_REDIRECT | reqwest::StatusCode::PERMANENT_REDIRECT
        ) {
            break response;
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| transport_failure("model-connector redirect has no valid location"))?;
        let target = attempt_url
            .join(location)
            .map_err(|_| transport_failure("model-connector redirect location is invalid"))?;
        validate_redirect(&origin, &target, followed_redirects, limits.max_redirects)
            .map_err(transport_failure)?;
        followed_redirects += 1;
        attempt_url = target;
    };

    if !response.status().is_success() {
        let status = response.status();
        consume_error_body(response, limits, cancellation, request_started).await?;
        return Err(http_status_failure(status));
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !content_type
        .split(';')
        .next()
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("text/event-stream"))
    {
        return Err(protocol_failure(
            "model-connector HTTP success did not return text/event-stream",
        ));
    }
    acceptance_sender
        .take()
        .ok_or_else(|| {
            transport_failure("model-connector acceptance channel was already consumed")
        })?
        .send(Ok(()))
        .map_err(|_| transport_failure("model-connector caller stopped before HTTP acceptance"))?;

    let mut chunks = response.bytes_stream();
    let mut stream_progress = Instant::now();
    loop {
        let timeout = phase_timeout(
            request_started,
            limits.absolute_request_timeout,
            stream_progress,
            limits.stream_idle_timeout,
            "model-connector stream idle deadline expired",
        )?;
        let next = cancellable_timeout(cancellation, timeout, chunks.next()).await?;
        match next {
            Some(Ok(bytes)) => {
                record_body_progress(&mut stream_progress, &bytes, Instant::now());
                let batch = decoder.push(&bytes);
                for event in batch.events {
                    send_event(event_sender, cancellation, request_started, limits, event).await?;
                }
                if let Some(failure) = batch.failure {
                    return Err(failure);
                }
            },
            Some(Err(_)) => return Err(transport_failure("model-connector stream read failed")),
            None => {
                for event in decoder.finish()? {
                    send_event(event_sender, cancellation, request_started, limits, event).await?;
                }
                return Ok(());
            },
        }
    }
}

async fn consume_error_body(
    response: reqwest::Response,
    limits: &ResponsesConnectorLimits,
    cancellation: &ResponsesCancellation,
    request_started: Instant,
) -> Result<(), ConnectorError> {
    let mut total = 0_usize;
    let mut chunks = response.bytes_stream();
    let mut error_body_progress = Instant::now();
    loop {
        let timeout = phase_timeout(
            request_started,
            limits.absolute_request_timeout,
            error_body_progress,
            limits.error_body_idle_timeout,
            "model-connector error-body idle deadline expired",
        )?;
        let next = cancellable_timeout(cancellation, timeout, chunks.next()).await?;
        match next {
            Some(Ok(bytes)) => {
                record_body_progress(&mut error_body_progress, &bytes, Instant::now());
                total = total
                    .checked_add(bytes.len())
                    .ok_or_else(|| limit_failure("model-connector error body size overflowed"))?;
                if total > limits.max_error_body_bytes {
                    return Err(limit_failure("model-connector error body limit exceeded"));
                }
            },
            Some(Err(_)) => {
                return Err(transport_failure("model-connector error body read failed"));
            },
            None => return Ok(()),
        }
    }
}

pub(super) async fn send_event(
    sender: &async_mpsc::Sender<ResponsesEvent>,
    cancellation: &ResponsesCancellation,
    request_started: Instant,
    limits: &ResponsesConnectorLimits,
    event: ResponsesEvent,
) -> Result<(), ConnectorError> {
    let timeout = phase_timeout(
        request_started,
        limits.absolute_request_timeout,
        Instant::now(),
        limits.event_delivery_timeout,
        "model-connector event delivery deadline expired",
    )?;
    tokio::select! {
        biased;
        () = cancellation.0.cancelled() => Err(cancelled_failure()),
        result = tokio::time::timeout(timeout.duration, sender.send(event)) => {
            result
                .map_err(|_| ConnectorError::new(ConnectorFailureKind::Timeout, timeout.message))?
                .map_err(|_| cancelled_failure())
        },
    }
}
