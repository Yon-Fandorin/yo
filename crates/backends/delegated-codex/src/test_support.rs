//! Shared app-server wire fixtures for Session and one-shot observation tests.

use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

use serde_json::{Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::BackendFailure;

use crate::transport::PeerPoll;

#[derive(Clone)]
pub(super) struct Sent(pub(super) Rc<RefCell<Vec<Value>>>);

pub(super) struct FakePeer {
    incoming: VecDeque<Result<PeerPoll, BackendFailure>>,
    sent: Sent,
}

impl FakePeer {
    pub(super) fn new(incoming: impl IntoIterator<Item = Value>) -> (Self, Sent) {
        let sent = Sent(Rc::new(RefCell::new(Vec::new())));
        (
            Self {
                incoming: incoming
                    .into_iter()
                    .map(|value| Ok(PeerPoll::Message(value)))
                    .collect(),
                sent: sent.clone(),
            },
            sent,
        )
    }
}

impl JsonMessagePeer for FakePeer {
    fn stop_handle(&self) -> yo_core::BackendStopHandle {
        yo_core::BackendStopHandle::no_op()
    }

    fn send(&mut self, message: &Value) -> Result<(), BackendFailure> {
        self.sent.0.borrow_mut().push(message.clone());
        Ok(())
    }

    fn receive(&mut self, _timeout: Duration) -> Result<PeerPoll, BackendFailure> {
        self.incoming.pop_front().unwrap_or(Ok(PeerPoll::Closed))
    }

    fn try_receive(&mut self) -> Result<PeerPoll, BackendFailure> {
        self.incoming.pop_front().unwrap_or(Ok(PeerPoll::Pending))
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        Ok(())
    }
}

pub(super) fn initialize_response(id: u64, version: &str) -> Value {
    json!({
        "id": id,
        "result": {
            "userAgent": format!("codex_cli_rs/{version} (test)"),
            "platformFamily": "unix",
            "platformOs": "linux",
            "codexHome": "/tmp/codex-test"
        }
    })
}
