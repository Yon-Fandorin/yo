use std::{
    fmt,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url, header, redirect};
use tokio::sync::mpsc as async_mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    ConnectorError, ConnectorFailureKind, ResponsesConnectorLimits, ResponsesEvent, ResponsesPoll,
    ResponsesRequest, sse::ResponsesSseDecoder,
};
use crate::{ApiCredential, ApiProtocol, ConnectorId, EffectiveModelBinding};

const EVENT_QUEUE_CAPACITY: usize = 256;

#[derive(Clone, Default)]
pub struct ResponsesCancellation(CancellationToken);

impl ResponsesCancellation {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.cancel();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

impl fmt::Debug for ResponsesCancellation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResponsesCancellation")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[derive(Clone)]
pub struct OpenAiResponsesConnector {
    client: Client,
    request_url: Url,
    model: String,
    credential: ApiCredential,
    limits: ResponsesConnectorLimits,
}

impl OpenAiResponsesConnector {
    pub fn new(
        binding: &EffectiveModelBinding,
        credential: ApiCredential,
        limits: ResponsesConnectorLimits,
    ) -> Result<Self, ConnectorError> {
        limits.validate()?;
        if binding.connector_id().as_str() != ConnectorId::OPENAI_RESPONSES
            || binding.api_protocol() != ApiProtocol::OpenAiResponses
        {
            return Err(configuration_failure(
                "binding does not select the openai-responses connector and protocol",
            ));
        }
        let request_url = binding
            .endpoint()
            .append_path_segment("responses")
            .map_err(|_| configuration_failure("cannot append responses to the base endpoint"))?;
        let origin = Origin::from_url(&request_url)?;
        let max_redirects = limits.max_redirects;
        let redirect_origin = origin.clone();
        let redirect_policy = redirect::Policy::custom(move |attempt| {
            match validate_redirect(
                &redirect_origin,
                attempt.url(),
                attempt.previous().len(),
                max_redirects,
            ) {
                Ok(()) => attempt.follow(),
                Err(message) => attempt.error(message),
            }
        });
        let client = Client::builder()
            .connect_timeout(limits.connect_timeout)
            .redirect(redirect_policy)
            .retry(reqwest::retry::never())
            .build()
            .map_err(|_| configuration_failure("cannot initialize the Responses HTTP client"))?;
        Ok(Self {
            client,
            request_url,
            model: binding.model_id().as_str().to_owned(),
            credential,
            limits,
        })
    }

    #[must_use]
    pub fn request_url(&self) -> &str {
        self.request_url.as_str()
    }

    pub fn start(
        &self,
        request: ResponsesRequest,
        cancellation: ResponsesCancellation,
    ) -> Result<ResponsesStream, ConnectorError> {
        if cancellation.is_cancelled() {
            return Err(cancelled_failure());
        }
        let (acceptance_sender, acceptance_receiver) = mpsc::sync_channel(1);
        let (event_sender, event_receiver) = async_mpsc::channel(EVENT_QUEUE_CAPACITY);
        let (outcome_sender, outcome_receiver) = mpsc::sync_channel(1);
        let client = self.client.clone();
        let request_url = self.request_url.clone();
        let model = self.model.clone();
        let credential = self.credential.clone();
        let limits = self.limits.clone();
        let worker_cancellation = cancellation.clone();
        let worker = thread::Builder::new()
            .name("yo-openai-responses".to_owned())
            .spawn(move || {
                run_worker(
                    client,
                    request_url,
                    model,
                    credential,
                    limits,
                    request,
                    worker_cancellation,
                    acceptance_sender,
                    event_sender,
                    outcome_sender,
                );
            })
            .map_err(|_| {
                ConnectorError::new(
                    ConnectorFailureKind::Transport,
                    "cannot start the Responses request worker",
                )
            })?;

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
                Err(ConnectorError::new(
                    ConnectorFailureKind::Transport,
                    "Responses request worker stopped before HTTP acceptance",
                ))
            },
        }
    }
}

fn validate_redirect(
    origin: &Origin,
    target: &Url,
    previous_count: usize,
    max_redirects: usize,
) -> Result<(), &'static str> {
    if previous_count >= max_redirects {
        Err("Responses redirect limit exceeded")
    } else if !origin.matches(target) {
        Err("Responses redirect changed origin")
    } else if !target.username().is_empty()
        || target.password().is_some()
        || target.query().is_some()
        || target.fragment().is_some()
    {
        Err("Responses redirect target is not normalized")
    } else {
        Ok(())
    }
}

impl fmt::Debug for OpenAiResponsesConnector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiResponsesConnector")
            .field("request_url", &self.request_url)
            .field("model", &self.model)
            .field("credential", &self.credential)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

pub struct ResponsesStream {
    receiver: async_mpsc::Receiver<ResponsesEvent>,
    outcome: mpsc::Receiver<Result<(), ConnectorError>>,
    cancellation: ResponsesCancellation,
    worker: Option<thread::JoinHandle<()>>,
    closed: bool,
}

impl ResponsesStream {
    pub fn poll(&mut self) -> Result<ResponsesPoll, ConnectorError> {
        if self.closed {
            return Ok(ResponsesPoll::Closed);
        }
        match self.receiver.try_recv() {
            Ok(event) => return Ok(ResponsesPoll::Event(event)),
            Err(async_mpsc::error::TryRecvError::Empty) => return Ok(ResponsesPoll::Pending),
            Err(async_mpsc::error::TryRecvError::Disconnected) => {},
        }
        match self.outcome.try_recv() {
            Ok(Ok(())) => {
                self.closed = true;
                Ok(ResponsesPoll::Closed)
            },
            Ok(Err(error)) => {
                self.closed = true;
                Err(error)
            },
            Err(mpsc::TryRecvError::Empty) => Ok(ResponsesPoll::Pending),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.closed = true;
                Err(transport_failure(
                    "Responses request worker stopped without a terminal outcome",
                ))
            },
        }
    }

    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub fn shutdown(&mut self) -> Result<(), ConnectorError> {
        self.cancellation.cancel();
        self.closed = true;
        self.join_worker()
    }

    fn join_worker(&mut self) -> Result<(), ConnectorError> {
        self.worker.take().map_or(Ok(()), |worker| {
            worker.join().map_err(|_| {
                ConnectorError::new(
                    ConnectorFailureKind::Cleanup,
                    "Responses request worker panicked during cleanup",
                )
            })
        })
    }
}

impl Drop for ResponsesStream {
    fn drop(&mut self) {
        self.cancellation.cancel();
        let _ = self.join_worker();
    }
}

impl fmt::Debug for ResponsesStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResponsesStream")
            .field("cancelled", &self.cancellation.is_cancelled())
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct Origin {
    scheme: String,
    host: String,
    port: u16,
}

impl Origin {
    fn from_url(url: &Url) -> Result<Self, ConnectorError> {
        let host = url
            .host_str()
            .ok_or_else(|| configuration_failure("Responses request URL has no host"))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| configuration_failure("Responses request URL has no effective port"))?;
        Ok(Self {
            scheme: url.scheme().to_owned(),
            host: host.to_owned(),
            port,
        })
    }

    fn matches(&self, url: &Url) -> bool {
        url.scheme() == self.scheme
            && url.host_str() == Some(self.host.as_str())
            && url.port_or_known_default() == Some(self.port)
    }
}

#[allow(clippy::too_many_arguments)]
fn run_worker(
    client: Client,
    request_url: Url,
    model: String,
    credential: ApiCredential,
    limits: ResponsesConnectorLimits,
    request: ResponsesRequest,
    cancellation: ResponsesCancellation,
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
                "cannot initialize the Responses request runtime",
            )));
            return;
        },
    };
    let mut acceptance_sender = Some(acceptance_sender);
    let result = runtime.block_on(execute_request(
        &client,
        request_url,
        &model,
        &credential,
        &limits,
        &request,
        &cancellation,
        &mut acceptance_sender,
        &event_sender,
    ));
    match (acceptance_sender.take(), result) {
        (Some(sender), Err(error)) => {
            let _ = sender.send(Err(error));
        },
        (Some(sender), Ok(())) => {
            let _ = sender.send(Err(transport_failure(
                "Responses request completed before HTTP acceptance",
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
    model: &str,
    credential: &ApiCredential,
    limits: &ResponsesConnectorLimits,
    request: &ResponsesRequest,
    cancellation: &ResponsesCancellation,
    acceptance_sender: &mut Option<mpsc::SyncSender<Result<(), ConnectorError>>>,
    event_sender: &async_mpsc::Sender<ResponsesEvent>,
) -> Result<(), ConnectorError> {
    let started = Instant::now();
    let response = cancellable_timeout(
        cancellation,
        effective_timeout(
            started,
            limits.total_request_timeout,
            limits.response_header_timeout,
            "Responses response-header deadline expired",
        )?,
        client
            .post(request_url)
            .bearer_auth(credential.expose_secret())
            .header(header::ACCEPT, "text/event-stream")
            .json(&request.wire_body(model))
            .send(),
    )
    .await?
    .map_err(map_reqwest_error)?;

    if !response.status().is_success() {
        let status = response.status();
        consume_error_body(response, limits, cancellation, started).await?;
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
            "Responses HTTP success did not return text/event-stream",
        ));
    }
    acceptance_sender
        .take()
        .ok_or_else(|| transport_failure("Responses acceptance channel was already consumed"))?
        .send(Ok(()))
        .map_err(|_| transport_failure("Responses caller stopped before HTTP acceptance"))?;

    let mut decoder = ResponsesSseDecoder::new(limits.clone());
    let mut chunks = response.bytes_stream();
    loop {
        let timeout = effective_timeout(
            started,
            limits.total_request_timeout,
            limits.stream_idle_timeout,
            "Responses stream idle deadline expired",
        )?;
        let next = cancellable_timeout(cancellation, timeout, chunks.next()).await?;
        match next {
            Some(Ok(bytes)) => {
                for event in decoder.push(&bytes)? {
                    send_event(event_sender, cancellation, started, limits, event).await?;
                }
            },
            Some(Err(_)) => return Err(transport_failure("Responses stream read failed")),
            None => {
                for event in decoder.finish()? {
                    send_event(event_sender, cancellation, started, limits, event).await?;
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
    started: Instant,
) -> Result<(), ConnectorError> {
    let mut total = 0_usize;
    let mut chunks = response.bytes_stream();
    loop {
        let timeout = effective_timeout(
            started,
            limits.total_request_timeout,
            limits.stream_idle_timeout,
            "Responses error-body idle deadline expired",
        )?;
        let next = cancellable_timeout(cancellation, timeout, chunks.next()).await?;
        match next {
            Some(Ok(bytes)) => {
                total = total
                    .checked_add(bytes.len())
                    .ok_or_else(|| limit_failure("Responses error body size overflowed"))?;
                if total > limits.max_error_body_bytes {
                    return Err(limit_failure("Responses error body limit exceeded"));
                }
            },
            Some(Err(_)) => return Err(transport_failure("Responses error body read failed")),
            None => return Ok(()),
        }
    }
}

async fn send_event(
    sender: &async_mpsc::Sender<ResponsesEvent>,
    cancellation: &ResponsesCancellation,
    started: Instant,
    limits: &ResponsesConnectorLimits,
    event: ResponsesEvent,
) -> Result<(), ConnectorError> {
    let timeout = effective_timeout(
        started,
        limits.total_request_timeout,
        limits.total_request_timeout,
        "Responses event delivery deadline expired",
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

async fn cancellable_timeout<F, T>(
    cancellation: &ResponsesCancellation,
    timeout: EffectiveTimeout,
    future: F,
) -> Result<T, ConnectorError>
where
    F: Future<Output = T>,
{
    tokio::select! {
        biased;
        () = cancellation.0.cancelled() => Err(cancelled_failure()),
        result = tokio::time::timeout(timeout.duration, future) => {
            result.map_err(|_| ConnectorError::new(ConnectorFailureKind::Timeout, timeout.message))
        },
    }
}

#[derive(Clone, Copy)]
struct EffectiveTimeout {
    duration: Duration,
    message: &'static str,
}

fn effective_timeout(
    started: Instant,
    total: Duration,
    phase: Duration,
    phase_message: &'static str,
) -> Result<EffectiveTimeout, ConnectorError> {
    let remaining = total.checked_sub(started.elapsed()).ok_or_else(|| {
        ConnectorError::new(
            ConnectorFailureKind::Timeout,
            "Responses total request deadline expired",
        )
    })?;
    if remaining.is_zero() {
        return Err(ConnectorError::new(
            ConnectorFailureKind::Timeout,
            "Responses total request deadline expired",
        ));
    }
    if remaining <= phase {
        Ok(EffectiveTimeout {
            duration: remaining,
            message: "Responses total request deadline expired",
        })
    } else {
        Ok(EffectiveTimeout {
            duration: phase,
            message: phase_message,
        })
    }
}

fn map_reqwest_error(error: reqwest::Error) -> ConnectorError {
    if error.is_timeout() {
        ConnectorError::new(
            ConnectorFailureKind::Timeout,
            "Responses HTTP transport deadline expired",
        )
    } else if error.is_redirect() {
        transport_failure("Responses HTTP redirect was rejected")
    } else {
        transport_failure("Responses HTTP request failed")
    }
}

fn configuration_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Configuration, message)
}

fn transport_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Transport, message)
}

fn protocol_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Protocol, message)
}

fn limit_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Limit, message)
}

fn cancelled_failure() -> ConnectorError {
    ConnectorError::new(
        ConnectorFailureKind::Cancelled,
        "Responses request was cancelled",
    )
}

fn http_status_failure(status: StatusCode) -> ConnectorError {
    ConnectorError::new(
        ConnectorFailureKind::HttpStatus,
        format!("Responses HTTP request returned status {}", status.as_u16()),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;

    fn dummy_event(id: &str) -> ResponsesEvent {
        ResponsesEvent::ResponseCreated {
            response_id: id.to_owned(),
        }
    }

    // redirect는 scheme·host·effective port가 모두 같은 origin이고 profile 횟수 안일 때만
    // 허용하여 다른 origin으로 bearer credential이나 request body가 전달되지 않습니다.
    #[test]
    fn permits_only_bounded_same_origin_redirects() {
        let original = Url::parse("https://example.com/v1/responses").unwrap();
        let origin = Origin::from_url(&original).unwrap();

        assert!(
            validate_redirect(
                &origin,
                &Url::parse("https://example.com:443/next").unwrap(),
                1,
                3,
            )
            .is_ok()
        );
        assert!(
            validate_redirect(
                &origin,
                &Url::parse("https://other.example/next").unwrap(),
                1,
                3,
            )
            .is_err()
        );
        assert!(validate_redirect(&origin, &original, 3, 3).is_err());
    }

    // 같은 origin이어도 user information·query·fragment가 붙은 redirect는 normalized
    // HTTPS target이 아니므로 bearer request를 따라가지 않는지 각각 검증합니다.
    #[test]
    fn rejects_non_normalized_same_origin_redirect_targets() {
        let original = Url::parse("https://example.com/v1/responses").unwrap();
        let origin = Origin::from_url(&original).unwrap();

        for target in [
            "https://user@example.com/next",
            "https://example.com/next?trace=1",
            "https://example.com/next#fragment",
        ] {
            assert!(validate_redirect(&origin, &Url::parse(target).unwrap(), 0, 3).is_err());
        }
    }

    // bounded event queue가 가득 찬 동안에도 전달 대기는 total request deadline을
    // 벗어나지 않고 typed Timeout으로 끝나는지 검증합니다.
    #[test]
    fn event_backpressure_obeys_the_total_request_deadline() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, _receiver) = async_mpsc::channel(1);
        sender.try_send(dummy_event("first")).unwrap();
        let limits = ResponsesConnectorLimits {
            total_request_timeout: Duration::from_millis(25),
            ..ResponsesConnectorLimits::default()
        };
        let cancellation = ResponsesCancellation::new();
        let started = Instant::now();

        let error = runtime
            .block_on(send_event(
                &sender,
                &cancellation,
                started,
                &limits,
                dummy_event("second"),
            ))
            .unwrap_err();

        assert_eq!(error.kind(), ConnectorFailureKind::Timeout);
        assert!(started.elapsed() >= Duration::from_millis(10));
    }

    // worker outcome이 먼저 보이더라도 event sender가 닫힐 때까지 queue를 계속 drain하여
    // outcome과 동시에 enqueue된 마지막 terminal event를 잃지 않는지 검증합니다.
    #[test]
    fn drains_every_event_before_observing_the_worker_outcome() {
        let (event_sender, event_receiver) = async_mpsc::channel(1);
        let (outcome_sender, outcome_receiver) = mpsc::sync_channel(1);
        outcome_sender.send(Ok(())).unwrap();
        let cancellation = ResponsesCancellation::new();
        let mut stream = ResponsesStream {
            receiver: event_receiver,
            outcome: outcome_receiver,
            cancellation,
            worker: None,
            closed: false,
        };

        assert_eq!(stream.poll().unwrap(), ResponsesPoll::Pending);
        event_sender.try_send(dummy_event("terminal")).unwrap();
        drop(event_sender);
        assert!(matches!(
            stream.poll().unwrap(),
            ResponsesPoll::Event(ResponsesEvent::ResponseCreated { response_id })
                if response_id == "terminal"
        ));
        assert_eq!(stream.poll().unwrap(), ResponsesPoll::Closed);
    }

    // stream Drop은 가득 찬 event queue에서 대기 중인 worker를 취소한 뒤 join하여
    // detached thread나 포화 queue shutdown deadlock을 남기지 않는지 검증합니다.
    #[test]
    fn drop_cancels_and_joins_a_worker_blocked_by_event_backpressure() {
        let (event_sender, event_receiver) = async_mpsc::channel(1);
        event_sender.try_send(dummy_event("first")).unwrap();
        let (outcome_sender, outcome_receiver) = mpsc::sync_channel(1);
        let cancellation = ResponsesCancellation::new();
        let worker_cancellation = cancellation.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = Arc::clone(&finished);
        let worker = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let result = runtime.block_on(send_event(
                &event_sender,
                &worker_cancellation,
                Instant::now(),
                &ResponsesConnectorLimits::default(),
                dummy_event("second"),
            ));
            let _ = outcome_sender.send(result);
            worker_finished.store(true, Ordering::Release);
        });
        let stream = ResponsesStream {
            receiver: event_receiver,
            outcome: outcome_receiver,
            cancellation,
            worker: Some(worker),
            closed: false,
        };

        drop(stream);

        assert!(finished.load(Ordering::Acquire));
    }
}
