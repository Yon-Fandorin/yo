use super::{ConnectorError, ConnectorFailureKind};

pub(super) struct SseFrame {
    pub(super) declared_event: Option<String>,
    pub(super) data: Option<String>,
}

pub(super) struct SseFrameBatch {
    pub(super) frames: Vec<SseFrame>,
    pub(super) failure: Option<ConnectorError>,
}

pub(super) struct SseFramer {
    buffer: Vec<u8>,
    event_count: usize,
    maximum_event_bytes: usize,
    maximum_events: usize,
    protocol: &'static str,
}

impl SseFramer {
    pub(super) fn new(
        maximum_event_bytes: usize,
        maximum_events: usize,
        protocol: &'static str,
    ) -> Self {
        Self {
            buffer: Vec::new(),
            event_count: 0,
            maximum_event_bytes,
            maximum_events,
            protocol,
        }
    }

    pub(super) fn push(&mut self, bytes: &[u8]) -> SseFrameBatch {
        let mut frames = Vec::new();
        for byte in bytes {
            self.buffer.push(*byte);
            if let Some(separator_len) = trailing_separator_len(&self.buffer) {
                let frame_len = self.buffer.len() - separator_len;
                let frame = self.buffer[..frame_len].to_vec();
                self.buffer.clear();
                if !frame.is_empty() {
                    match self.decode(&frame) {
                        Ok(frame) => frames.push(frame),
                        Err(failure) => {
                            return SseFrameBatch {
                                frames,
                                failure: Some(failure),
                            };
                        },
                    }
                }
            } else if self.buffer.len() > self.maximum_event_bytes {
                return SseFrameBatch {
                    frames,
                    failure: Some(self.limit_failure("SSE event byte limit exceeded")),
                };
            }
        }
        SseFrameBatch {
            frames,
            failure: None,
        }
    }

    pub(super) fn finish(&mut self) -> Result<Option<SseFrame>, ConnectorError> {
        if self.buffer.is_empty() {
            return Ok(None);
        }
        let frame = std::mem::take(&mut self.buffer);
        self.decode(&frame).map(Some)
    }

    fn decode(&mut self, bytes: &[u8]) -> Result<SseFrame, ConnectorError> {
        if bytes.len() > self.maximum_event_bytes {
            return Err(self.limit_failure("SSE event byte limit exceeded"));
        }
        self.event_count = self
            .event_count
            .checked_add(1)
            .ok_or_else(|| self.limit_failure("SSE event count overflowed"))?;
        if self.event_count > self.maximum_events {
            return Err(self.limit_failure("SSE event count limit exceeded"));
        }
        let text = std::str::from_utf8(bytes)
            .map_err(|_| self.protocol_failure("SSE event is not valid UTF-8"))?;
        let mut declared_event = None;
        let mut data_lines = Vec::new();
        for raw_line in text.split('\n') {
            let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => declared_event = Some(value.to_owned()),
                "data" => data_lines.push(value),
                "id" | "retry" => {},
                _ => {},
            }
        }
        Ok(SseFrame {
            declared_event,
            data: (!data_lines.is_empty()).then(|| data_lines.join("\n")),
        })
    }

    fn protocol_failure(&self, message: &str) -> ConnectorError {
        ConnectorError::new(
            ConnectorFailureKind::Protocol,
            format!("{} {message}", self.protocol),
        )
    }

    fn limit_failure(&self, message: &str) -> ConnectorError {
        ConnectorError::new(
            ConnectorFailureKind::Limit,
            format!("{} {message}", self.protocol),
        )
    }
}

fn trailing_separator_len(bytes: &[u8]) -> Option<usize> {
    if bytes.ends_with(b"\r\n\r\n") {
        Some(4)
    } else if bytes.ends_with(b"\n\n") || bytes.ends_with(b"\r\r") {
        Some(2)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 한 network chunk의 뒤쪽 frame이 UTF-8 경계에서 실패해도 앞에서 완성된 frame은
    // dialect decoder가 먼저 전달할 수 있도록 failure와 함께 보존한다.
    #[test]
    fn preserves_completed_frames_before_a_later_same_chunk_failure() {
        let mut framer = SseFramer::new(1_024, 8, "test");

        let batch = framer.push(b"data: visible\n\ndata: \xff\n\n");

        assert_eq!(batch.frames.len(), 1);
        assert_eq!(batch.frames[0].data.as_deref(), Some("visible"));
        assert!(matches!(
            batch.failure,
            Some(ref failure) if failure.kind() == ConnectorFailureKind::Protocol
        ));
    }
}
