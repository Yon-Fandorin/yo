//! Opt-in, mock-only hidden input diagnostic exposed to Grok through local MCP.

use std::{
    io::{BufReader, ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde_json::{Value, json};
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityQuestion, ActivityRef, ActivityRequestRef,
    ActivityResponse, ActivityUpdate, BackendCommandEvidence, BackendEvent, BackendFailure,
    BackendFailureKind,
};

use super::state::Backend;
use crate::{protocol, transport::JsonPeer};

const TOOL_NAME: &str = "yo_secret_entry_probe";
const MAX_REQUEST_BYTES: usize = 32 * 1024;
const MAX_CONNECTIONS: usize = 16;
const RESULT: &str = "Sample secret entry verified locally and discarded by Yo. No value was sent to Grok or the model.";

pub(super) struct ProbeInvocation {
    response: Sender<bool>,
}

pub(super) struct PendingProbe {
    pub(super) request: ActivityRequestRef,
    activity: ActivityRef,
    response: Sender<bool>,
}

pub(super) struct SecretProbeBridge {
    url: String,
    incoming: Receiver<ProbeInvocation>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl SecretProbeBridge {
    pub(super) fn start() -> Result<Self, BackendFailure> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|_| {
            BackendFailure::new(
                BackendFailureKind::Initialization,
                "Grok secret-entry probe could not bind a local port",
            )
        })?;
        listener.set_nonblocking(true).map_err(|_| {
            BackendFailure::new(
                BackendFailureKind::Initialization,
                "Grok secret-entry probe could not configure its local port",
            )
        })?;
        let token = uuid::Uuid::new_v4().simple().to_string();
        let path = format!("/mcp/{token}");
        let url = format!(
            "http://127.0.0.1:{}{path}",
            listener.local_addr().expect("bound listener").port()
        );
        let (sender, incoming) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let active = Arc::new(AtomicUsize::new(0));
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if active.fetch_add(1, Ordering::Relaxed) >= MAX_CONNECTIONS {
                            active.fetch_sub(1, Ordering::Relaxed);
                            continue;
                        }
                        let sender = sender.clone();
                        let path = path.clone();
                        let active = Arc::clone(&active);
                        thread::spawn(move || {
                            serve_connection(stream, &path, &sender);
                            active.fetch_sub(1, Ordering::Relaxed);
                        });
                    },
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    },
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            url,
            incoming,
            stop,
            worker: Some(worker),
        })
    }

    pub(super) fn server_spec(&self) -> Value {
        json!({ "type": "http", "name": "yo_secret_entry_probe", "url": self.url, "headers": [] })
    }

    fn try_receive(&self) -> Result<Option<ProbeInvocation>, BackendFailure> {
        match self.incoming.try_recv() {
            Ok(invocation) => Ok(Some(invocation)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(protocol::protocol_failure(
                "Grok secret-entry probe listener stopped",
            )),
        }
    }
}

impl Drop for SecretProbeBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn serve_connection(mut stream: TcpStream, path: &str, sender: &Sender<ProbeInvocation>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let result = read_request(&stream, path);
    let (status, response) = match result {
        Ok(Some(message)) => {
            let id = message.get("id").cloned();
            let method = message
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let body = match method {
                "server/discover" => json!({}),
                "initialize" => json!({
                    "protocolVersion": "2025-03-26",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "yo-secret-entry-probe", "version": "1" }
                }),
                "tools/list" => json!({ "tools": [{
                    "name": TOOL_NAME,
                    "description": "Diagnostic only: ask for a MADE-UP sample in Yo's hidden editor. Yo discards it and returns only a fixed status. Never request a real password, token, or credential; the model cannot use the entered value.",
                    "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
                }] }),
                "tools/call" => {
                    let params = message.get("params");
                    let valid = id.is_some()
                        && params
                            .and_then(|params| params.get("name"))
                            .and_then(Value::as_str)
                            == Some(TOOL_NAME)
                        && params
                            .and_then(|params| params.get("arguments"))
                            .and_then(Value::as_object)
                            .is_some_and(serde_json::Map::is_empty);
                    let completed = if valid {
                        let (response, receiver) = mpsc::channel();
                        sender.send(ProbeInvocation { response }).is_ok()
                            && receiver
                                .recv_timeout(Duration::from_secs(600))
                                .unwrap_or(false)
                    } else {
                        false
                    };
                    if completed {
                        json!({ "content": [{ "type": "text", "text": RESULT }] })
                    } else {
                        json!({ "isError": true, "content": [{ "type": "text", "text": "Yo secret-entry probe was unavailable or cancelled." }] })
                    }
                },
                _ => json!({}),
            };
            if let Some(id) = id {
                (
                    200,
                    Some(json!({ "jsonrpc": "2.0", "id": id, "result": body })),
                )
            } else {
                (202, None)
            }
        },
        Ok(None) => (404, None),
        Err(()) => (400, None),
    };
    let payload = response.map(|value| value.to_string()).unwrap_or_default();
    let header = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(payload.as_bytes());
}

fn read_request(stream: &TcpStream, path: &str) -> Result<Option<Value>, ()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    read_bounded_line(&mut reader, &mut line, 8192)?;
    if !line.starts_with("POST ") || !line.contains(" HTTP/1.") {
        return Err(());
    }
    if line.split_whitespace().nth(1) != Some(path) {
        return Ok(None);
    }
    let mut length = None;
    let mut header_bytes = line.len();
    loop {
        line.clear();
        read_bounded_line(&mut reader, &mut line, 8192 - header_bytes)?;
        header_bytes += line.len();
        if header_bytes > 8192 {
            return Err(());
        }
        if line == "\r\n" {
            break;
        }
        if line.is_empty() {
            return Err(());
        }
        if let Some(value) = line
            .split_once(':')
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        {
            length = Some(value.1.trim().parse::<usize>().map_err(|_| ())?);
        }
    }
    let length = length
        .filter(|length| *length <= MAX_REQUEST_BYTES)
        .ok_or(())?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body).map_err(|_| ())?;
    serde_json::from_slice(&body).map(Some).map_err(|_| ())
}

fn read_bounded_line(reader: &mut impl Read, line: &mut String, limit: usize) -> Result<(), ()> {
    for _ in 0..limit {
        let mut byte = [0];
        reader.read_exact(&mut byte).map_err(|_| ())?;
        if !byte[0].is_ascii() {
            return Err(());
        }
        line.push(char::from(byte[0]));
        if byte[0] == b'\n' {
            return Ok(());
        }
    }
    Err(())
}

impl<P: JsonPeer> Backend<P> {
    pub(super) fn secret_probe_servers(&self) -> Vec<Value> {
        self.secret_probe
            .as_ref()
            .map(|bridge| vec![bridge.server_spec()])
            .unwrap_or_default()
    }

    pub(super) fn poll_secret_probe(&mut self) -> Result<Option<BackendEvent>, BackendFailure> {
        let Some(invocation) = self
            .secret_probe
            .as_ref()
            .map(SecretProbeBridge::try_receive)
            .transpose()?
            .flatten()
        else {
            return Ok(None);
        };
        let turn = match self.prompt.as_ref() {
            Some(prompt) if !prompt.interrupt_requested && self.pending_probe.is_none() => {
                prompt.turn
            },
            _ => {
                let _ = invocation.response.send(false);
                return Ok(None);
            },
        };
        self.ensure_activity_capacity()?;
        let activity = self.next_activity(turn)?;
        let request_id = self.next_request()?;
        let request = ActivityRequestRef::new(activity, request_id);
        let snapshot = ActivityQuestion {
            plain_text: "Sample input test\n\nEnter a made-up sample value only. Never enter a real password, token, or credential. Yo discards this value and sends only a fixed completion status to Grok.\nEsc interrupts the turn.".into(),
            choices: Vec::new(),
            allow_notes: false,
            is_secret: true,
            storage_offer: None,
            previous_question: false,
            draft: None,
            draft_choice: None,
        }.to_snapshot().ok_or_else(|| protocol::protocol_failure("Grok secret-entry probe prompt exceeded its display limit"))?;
        self.pending_probe = Some(PendingProbe {
            request,
            activity,
            response: invocation.response,
        });
        self.pending_events
            .push_back(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::UserInputRequest { request_id },
            });
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(snapshot),
            });
        Ok(self.pending_events.pop_front())
    }

    pub(super) fn respond_to_secret_probe(
        &mut self,
        response: ActivityResponse,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        let ActivityResponse::SecretInput(sample) = response else {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Grok secret-entry probe requires hidden sample input",
            ));
        };
        drop(sample);
        let probe = self.pending_probe.take().expect("matched pending probe");
        let completed = probe.response.send(true).is_ok();
        self.pending_events
            .push_back(BackendEvent::ActivityFinished {
                activity: probe.activity,
                outcome: if completed {
                    ActivityOutcome::Completed
                } else {
                    ActivityOutcome::Interrupted
                },
            });
        if !completed {
            return Err(protocol::protocol_failure(
                "Grok secret-entry probe result receiver closed",
            ));
        }
        let activity = self.next_activity(probe.request.activity().turn())?;
        self.pending_events
            .push_back(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::UserInputResponse {
                    request_id: probe.request.request_id(),
                },
            });
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(
                    "Sample secret entry verified locally and discarded.".into(),
                ),
            });
        self.pending_events
            .push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            });
        Ok(BackendCommandEvidence::None)
    }

    pub(super) fn cancel_secret_probe(&mut self) {
        if let Some(probe) = self.pending_probe.take() {
            let _ = probe.response.send(false);
            self.pending_events
                .push_back(BackendEvent::ActivityFinished {
                    activity: probe.activity,
                    outcome: ActivityOutcome::Interrupted,
                });
        }
    }
}
