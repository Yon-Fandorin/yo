use std::collections::VecDeque;

use crate::{BackendFailure, BackendFailureKind};

const DEFAULT_PENDING_MESSAGE_LIMIT: usize = 1024;
const MAX_JSON_RPC_REQUEST_ID: u64 = i64::MAX as u64;

/// Bounded request identity and deferred-message state shared by JSON-RPC-like adapters.
pub struct JsonRpcMailbox<M> {
    protocol_name: &'static str,
    next_request_id: u64,
    pending: VecDeque<M>,
    pending_message_limit: usize,
}

impl<M> JsonRpcMailbox<M> {
    pub fn new(protocol_name: &'static str) -> Self {
        Self {
            protocol_name,
            next_request_id: 1,
            pending: VecDeque::new(),
            pending_message_limit: DEFAULT_PENDING_MESSAGE_LIMIT,
        }
    }

    pub fn next_request_id(&mut self) -> Result<u64, BackendFailure> {
        let id = self.next_request_id;
        if id > MAX_JSON_RPC_REQUEST_ID {
            return Err(BackendFailure::new(
                BackendFailureKind::Protocol,
                format!("{} request id space was exhausted", self.protocol_name),
            ));
        }
        self.next_request_id = self.next_request_id.checked_add(1).ok_or_else(|| {
            BackendFailure::new(
                BackendFailureKind::Protocol,
                format!("{} request id space was exhausted", self.protocol_name),
            )
        })?;
        Ok(id)
    }

    pub fn push(&mut self, message: M) -> Result<(), BackendFailure> {
        if self.pending.len() == self.pending_message_limit {
            return Err(BackendFailure::new(
                BackendFailureKind::Unavailable,
                format!(
                    "{} event backlog filled while awaiting a response",
                    self.protocol_name
                ),
            ));
        }
        self.pending.push_back(message);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<M> {
        self.pending.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // request ID는 1부터 단조 증가해 서로 다른 in-flight correlation 좌표를 제공합니다.
    #[test]
    fn request_ids_are_monotonic() {
        let mut mailbox = JsonRpcMailbox::<()>::new("fixture");

        assert_eq!(mailbox.next_request_id().unwrap(), 1);
        assert_eq!(mailbox.next_request_id().unwrap(), 2);
    }

    // JSON-RPC와 ACP numeric request ID는 signed int64이므로 양수 경곗값은 한 번 발급할 수
    // 있지만 schema 밖의 u64 값을 만들기 전에 allocator가 실패하는지 확인한다.
    #[test]
    fn request_ids_stop_at_the_signed_integer_boundary() {
        let mut mailbox = JsonRpcMailbox::<()> {
            protocol_name: "fixture",
            next_request_id: MAX_JSON_RPC_REQUEST_ID,
            pending: VecDeque::new(),
            pending_message_limit: DEFAULT_PENDING_MESSAGE_LIMIT,
        };

        assert_eq!(mailbox.next_request_id().unwrap(), MAX_JSON_RPC_REQUEST_ID);
        let failure = mailbox.next_request_id().unwrap_err();
        assert_eq!(failure.kind(), BackendFailureKind::Protocol);
        assert!(failure.message().contains("request id space was exhausted"));
    }

    // deferred message는 입력 순서를 보존해야 protocol event 재생 순서가 바뀌지 않습니다.
    #[test]
    fn deferred_messages_preserve_fifo_order() {
        let mut mailbox = JsonRpcMailbox::new("fixture");
        mailbox.push(1).unwrap();
        mailbox.push(2).unwrap();

        assert_eq!(mailbox.pop(), Some(1));
        assert_eq!(mailbox.pop(), Some(2));
    }
}
