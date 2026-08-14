use std::{error::Error, fmt, time::Duration};

#[derive(Clone, Debug)]
pub struct ResponsesConnectorLimits {
    pub connect_timeout: Duration,
    pub response_header_timeout: Duration,
    pub stream_idle_timeout: Duration,
    pub error_body_idle_timeout: Duration,
    pub event_delivery_timeout: Duration,
    pub absolute_request_timeout: Option<Duration>,
    pub max_redirects: usize,
    pub max_error_body_bytes: usize,
    pub max_sse_event_bytes: usize,
    pub max_sse_events: usize,
    pub max_output_items: usize,
    pub max_response_text_bytes: usize,
    pub max_refusal_bytes: usize,
    pub max_reasoning_bytes: usize,
    pub max_function_argument_bytes: usize,
}

impl Default for ResponsesConnectorLimits {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(30),
            response_header_timeout: Duration::from_secs(5 * 60),
            stream_idle_timeout: Duration::from_secs(5 * 60),
            error_body_idle_timeout: Duration::from_secs(30),
            event_delivery_timeout: Duration::from_secs(5 * 60),
            absolute_request_timeout: None,
            max_redirects: 3,
            max_error_body_bytes: 64 * 1024,
            max_sse_event_bytes: 1024 * 1024,
            max_sse_events: 100_000,
            max_output_items: 1_024,
            max_response_text_bytes: 16 * 1024 * 1024,
            max_refusal_bytes: 16 * 1024 * 1024,
            max_reasoning_bytes: 16 * 1024 * 1024,
            max_function_argument_bytes: 4 * 1024 * 1024,
        }
    }
}

impl ResponsesConnectorLimits {
    pub(super) fn validate(&self) -> Result<(), ConnectorError> {
        let durations = [
            self.connect_timeout,
            self.response_header_timeout,
            self.stream_idle_timeout,
            self.error_body_idle_timeout,
            self.event_delivery_timeout,
        ];
        if durations.into_iter().any(|duration| duration.is_zero())
            || self
                .absolute_request_timeout
                .is_some_and(|duration| duration.is_zero())
        {
            return Err(ConnectorError::new(
                ConnectorFailureKind::Configuration,
                "Responses connector configured deadlines must be non-zero",
            ));
        }
        let bounds = [
            self.max_redirects,
            self.max_error_body_bytes,
            self.max_sse_event_bytes,
            self.max_sse_events,
            self.max_output_items,
            self.max_response_text_bytes,
            self.max_refusal_bytes,
            self.max_reasoning_bytes,
            self.max_function_argument_bytes,
        ];
        if bounds.into_iter().any(|bound| bound == 0) {
            return Err(ConnectorError::new(
                ConnectorFailureKind::Configuration,
                "Responses connector bounds must be non-zero",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectorFailureKind {
    Configuration,
    Transport,
    HttpStatus,
    Protocol,
    Limit,
    Timeout,
    Cancelled,
    Cleanup,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectorError {
    kind: ConnectorFailureKind,
    message: String,
}

impl ConnectorError {
    pub(super) fn new(kind: ConnectorFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> ConnectorFailureKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ConnectorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.message)
    }
}

impl Error for ConnectorError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResponsesUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResponseTerminal {
    Completed,
    Incomplete { reason: Option<String> },
    Failed { code: Option<String> },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ReasoningChannel {
    Text,
    Summary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResponsesEvent {
    ResponseCreated {
        response_id: String,
    },
    TextDelta {
        output_index: usize,
        item_id: String,
        content_index: usize,
        delta: String,
    },
    RefusalDelta {
        output_index: usize,
        item_id: String,
        content_index: usize,
        delta: String,
    },
    MessageDone {
        output_index: usize,
        item_id: String,
    },
    ReasoningDelta {
        output_index: usize,
        item_id: String,
        channel: ReasoningChannel,
        part_index: usize,
        delta: String,
    },
    FunctionCallStarted {
        output_index: usize,
        item_id: String,
        call_id: String,
        name: String,
    },
    FunctionArgumentsDelta {
        output_index: usize,
        item_id: String,
        delta: String,
    },
    FunctionCallDone {
        output_index: usize,
        item_id: String,
        call_id: String,
        name: String,
        arguments: String,
    },
    Terminal {
        response_id: String,
        status: ResponseTerminal,
        usage: ResponsesUsage,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResponsesPoll {
    Pending,
    Event(ResponsesEvent),
    Closed,
}
