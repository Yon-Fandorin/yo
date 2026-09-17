/// 로컬 HTTPS listener가 반환하거나 멈추는 동작을 선택합니다.
pub enum LocalServerMode {
    /// 제한된 body와 content type으로 성공 응답을 보냅니다.
    Success {
        /// listener가 보낼 bounded response body입니다.
        body: Vec<u8>,
        /// listener가 보낼 Content-Type입니다.
        content_type: String,
    },
    /// 지정한 non-success status와 body를 반환합니다.
    Status {
        /// listener가 반환할 HTTP status입니다.
        status: u16,
        /// status와 함께 보낼 bounded body입니다.
        body: Vec<u8>,
    },
    /// 같은 origin의 지정한 위치로 redirect한 뒤 성공 응답을 보냅니다.
    Redirect {
        /// 첫 응답의 Location 값입니다.
        location: String,
        /// redirect 뒤 성공 응답으로 보낼 body입니다.
        final_body: Vec<u8>,
    },
    /// 각 redirect hop을 지정한 시간만큼 지연한 뒤 chain을 진행합니다.
    DelayedRedirectChain {
        /// redirect chain 마지막 성공 응답의 body입니다.
        final_body: Vec<u8>,
        /// 각 응답 전에 적용할 bounded 지연 시간입니다.
        response_delay_millis: u16,
    },
    /// 같은 origin redirect를 반복해 redirect 상한을 관찰합니다.
    RedirectLoop,
    /// 허용된 응답 body 상한보다 큰 Content-Length를 선언합니다.
    DeclaredOversize,
    /// Content-Length 없이 body를 보내 응답 body 상한을 관찰합니다.
    UnframedSuccess {
        /// Content-Length 없이 보낼 bounded response body입니다.
        body: Vec<u8>,
        /// unframed 성공 응답의 Content-Type입니다.
        content_type: String,
    },
    /// response header 뒤 body를 보내지 않고 연결을 유지합니다.
    HeadersThenStall {
        /// body 없이 보낼 response header의 Content-Type입니다.
        content_type: String,
    },
    /// 하나의 SSE body를 보낸 뒤 연결을 유지합니다.
    EventThenStall {
        /// stall 전에 보낼 bounded SSE body입니다.
        body: Vec<u8>,
    },
    /// non-success body 일부를 보낸 뒤 연결을 유지합니다.
    ErrorBodyThenStall {
        /// stall 전에 반환할 non-success status입니다.
        status: u16,
        /// stall 전에 보낼 bounded error body입니다.
        body: Vec<u8>,
    },
    /// 빈 SSE heartbeat를 여러 번 보낸 뒤 연결을 유지합니다.
    HeartbeatsThenStall,
    /// TLS 연결과 request 수신 뒤 response header를 보내지 않습니다.
    ResponseHeaderStall,
    /// TCP 연결 뒤 TLS handshake를 완료하지 않습니다.
    TlsHandshakeStall,
}

pub(super) struct LocalTlsModeConfig {
    pub(super) wire_mode: &'static str,
    pub(super) content_type: String,
    pub(super) status: u16,
    pub(super) location: String,
    pub(super) max_connections: usize,
    pub(super) payload: Vec<u8>,
}

impl LocalServerMode {
    pub(super) fn into_child_config(self) -> LocalTlsModeConfig {
        match self {
            Self::Success { body, content_type } => LocalTlsModeConfig {
                wire_mode: "success",
                content_type,
                status: 200,
                location: String::new(),
                max_connections: 1,
                payload: body,
            },
            Self::Status { status, body } => LocalTlsModeConfig {
                wire_mode: "status",
                content_type: "text/plain; charset=utf-8".to_owned(),
                status,
                location: String::new(),
                max_connections: 1,
                payload: body,
            },
            Self::Redirect {
                location,
                final_body,
            } => LocalTlsModeConfig {
                wire_mode: "redirect",
                content_type: "text/event-stream".to_owned(),
                status: 307,
                location,
                max_connections: 2,
                payload: final_body,
            },
            Self::DelayedRedirectChain {
                final_body,
                response_delay_millis,
            } => LocalTlsModeConfig {
                wire_mode: "delayed-redirect-chain",
                content_type: "text/event-stream".to_owned(),
                status: response_delay_millis,
                location: String::new(),
                max_connections: 3,
                payload: final_body,
            },
            Self::RedirectLoop => LocalTlsModeConfig {
                wire_mode: "redirect-loop",
                content_type: "application/json".to_owned(),
                status: 307,
                location: String::new(),
                max_connections: 4,
                payload: Vec::new(),
            },
            Self::DeclaredOversize => LocalTlsModeConfig {
                wire_mode: "declared-oversize",
                content_type: "application/json".to_owned(),
                status: 200,
                location: String::new(),
                max_connections: 1,
                payload: Vec::new(),
            },
            Self::UnframedSuccess { body, content_type } => LocalTlsModeConfig {
                wire_mode: "unframed-success",
                content_type,
                status: 200,
                location: String::new(),
                max_connections: 1,
                payload: body,
            },
            Self::HeadersThenStall { content_type } => LocalTlsModeConfig {
                wire_mode: "headers-stall",
                content_type,
                status: 200,
                location: String::new(),
                max_connections: 1,
                payload: Vec::new(),
            },
            Self::EventThenStall { body } => LocalTlsModeConfig {
                wire_mode: "event-stall",
                content_type: "text/event-stream".to_owned(),
                status: 200,
                location: String::new(),
                max_connections: 1,
                payload: body,
            },
            Self::ErrorBodyThenStall { status, body } => LocalTlsModeConfig {
                wire_mode: "error-body-stall",
                content_type: "text/plain; charset=utf-8".to_owned(),
                status,
                location: String::new(),
                max_connections: 1,
                payload: body,
            },
            Self::HeartbeatsThenStall => LocalTlsModeConfig {
                wire_mode: "heartbeat-stall",
                content_type: "text/event-stream".to_owned(),
                status: 200,
                location: String::new(),
                max_connections: 1,
                payload: Vec::new(),
            },
            Self::ResponseHeaderStall => LocalTlsModeConfig {
                wire_mode: "header-stall",
                content_type: String::new(),
                status: 0,
                location: String::new(),
                max_connections: 1,
                payload: Vec::new(),
            },
            Self::TlsHandshakeStall => LocalTlsModeConfig {
                wire_mode: "tls-stall",
                content_type: String::new(),
                status: 0,
                location: String::new(),
                max_connections: 1,
                payload: Vec::new(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 모든 public mode가 canonical Python child protocol의 유한한 인자 집합으로 변환되는지
    // 고정합니다.
    #[test]
    fn maps_the_complete_connector_mode_superset_to_bounded_child_protocol() {
        let cases = [
            (
                LocalServerMode::Success {
                    body: b"success".to_vec(),
                    content_type: "text/plain".to_owned(),
                },
                "success",
                200,
                1,
            ),
            (
                LocalServerMode::Status {
                    status: 500,
                    body: b"status".to_vec(),
                },
                "status",
                500,
                1,
            ),
            (
                LocalServerMode::Redirect {
                    location: "/v1/next".to_owned(),
                    final_body: b"redirect".to_vec(),
                },
                "redirect",
                307,
                2,
            ),
            (
                LocalServerMode::DelayedRedirectChain {
                    final_body: b"delayed".to_vec(),
                    response_delay_millis: 80,
                },
                "delayed-redirect-chain",
                80,
                3,
            ),
            (LocalServerMode::RedirectLoop, "redirect-loop", 307, 4),
            (
                LocalServerMode::DeclaredOversize,
                "declared-oversize",
                200,
                1,
            ),
            (
                LocalServerMode::UnframedSuccess {
                    body: b"unframed".to_vec(),
                    content_type: "application/json".to_owned(),
                },
                "unframed-success",
                200,
                1,
            ),
            (
                LocalServerMode::HeadersThenStall {
                    content_type: "text/event-stream".to_owned(),
                },
                "headers-stall",
                200,
                1,
            ),
            (
                LocalServerMode::EventThenStall {
                    body: b"event".to_vec(),
                },
                "event-stall",
                200,
                1,
            ),
            (
                LocalServerMode::ErrorBodyThenStall {
                    status: 503,
                    body: b"error".to_vec(),
                },
                "error-body-stall",
                503,
                1,
            ),
            (
                LocalServerMode::HeartbeatsThenStall,
                "heartbeat-stall",
                200,
                1,
            ),
            (LocalServerMode::ResponseHeaderStall, "header-stall", 0, 1),
            (LocalServerMode::TlsHandshakeStall, "tls-stall", 0, 1),
        ];

        for (mode, wire_mode, status, max_connections) in cases {
            let config = mode.into_child_config();
            assert_eq!(config.wire_mode, wire_mode);
            assert_eq!(config.status, status);
            assert_eq!(config.max_connections, max_connections);
        }
    }
}
