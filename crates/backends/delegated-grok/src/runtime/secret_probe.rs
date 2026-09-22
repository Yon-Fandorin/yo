//! Opt-in, mock-only hidden input diagnostic exposed to Grok through local MCP.

use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
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
    permit: ProbePermit,
}

pub(super) struct PendingProbe {
    pub(super) request: ActivityRequestRef,
    activity: ActivityRef,
    response: Sender<bool>,
    permit: ProbePermit,
}

#[derive(Default)]
struct GateState {
    generation: u64,
    active: bool,
    in_flight: bool,
    call_id: Option<Value>,
    cancelled: Option<Arc<AtomicBool>>,
}

#[derive(Clone, Copy)]
struct Ingress {
    generation: u64,
    active: bool,
}

struct ProbePermit {
    gate: Arc<Mutex<GateState>>,
    generation: u64,
    cancelled: Arc<AtomicBool>,
}

impl Drop for ProbePermit {
    fn drop(&mut self) {
        let mut state = self.gate.lock().expect("probe gate");
        if state.generation == self.generation
            && state
                .cancelled
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &self.cancelled))
        {
            state.in_flight = false;
            state.call_id = None;
            state.cancelled = None;
        }
    }
}

pub(super) struct SecretProbeBridge {
    url: String,
    incoming: Receiver<ProbeInvocation>,
    gate: Arc<Mutex<GateState>>,
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
        let gate = Arc::new(Mutex::new(GateState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_gate = Arc::clone(&gate);
        let worker = thread::spawn(move || {
            let mut workers: Vec<JoinHandle<()>> = Vec::new();
            while !worker_stop.load(Ordering::Relaxed) {
                let mut index = 0;
                while index < workers.len() {
                    if workers[index].is_finished() {
                        let _ = workers.swap_remove(index).join();
                    } else {
                        index += 1;
                    }
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        if workers.len() >= MAX_CONNECTIONS {
                            continue;
                        }
                        let sender = sender.clone();
                        let path = path.clone();
                        let gate = Arc::clone(&worker_gate);
                        let stop = Arc::clone(&worker_stop);
                        workers.push(thread::spawn(move || {
                            serve_connection(stream, &path, &sender, &gate, &stop)
                        }));
                    },
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    },
                    Err(_) => break,
                }
            }
            for worker in workers {
                let _ = worker.join();
            }
        });
        Ok(Self {
            url,
            incoming,
            gate,
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

    pub(super) fn activate(&self) {
        let mut state = self.gate.lock().expect("probe gate");
        state.generation = state.generation.wrapping_add(1);
        state.active = true;
        state.in_flight = false;
        state.call_id = None;
        state.cancelled = None;
    }

    pub(super) fn deactivate_and_drain(&self) {
        let mut state = self.gate.lock().expect("probe gate");
        if let Some(cancelled) = state.cancelled.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
        state.generation = state.generation.wrapping_add(1);
        state.active = false;
        state.in_flight = false;
        state.call_id = None;
        drop(state);
        while let Ok(invocation) = self.incoming.try_recv() {
            invocation.permit.cancelled.store(true, Ordering::Relaxed);
            let _ = invocation.response.send(false);
        }
    }

    fn accepts(&self, invocation: &ProbeInvocation) -> bool {
        let state = self.gate.lock().expect("probe gate");
        state.active
            && state.generation == invocation.permit.generation
            && !invocation.permit.cancelled.load(Ordering::Relaxed)
    }
}

impl Drop for SecretProbeBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.deactivate_and_drain();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn serve_connection(
    mut stream: TcpStream,
    path: &str,
    sender: &Sender<ProbeInvocation>,
    gate: &Arc<Mutex<GateState>>,
    stop: &AtomicBool,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    // 요청 본문을 기다리는 동안 다음 Turn이 시작되어도, 이 연결은 처음 관찰한 Turn에만
    // 귀속됩니다.
    let ingress = ingress(gate);
    let result = read_request(&stream, path, stop);
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
                            .is_none_or(|arguments| {
                                arguments.as_object().is_some_and(serde_json::Map::is_empty)
                            });
                    let completed = if valid {
                        let permit = admit(gate, id.as_ref().expect("validated call id"), ingress);
                        if let Some(permit) = permit {
                            let cancelled = Arc::clone(&permit.cancelled);
                            let (response, receiver) = mpsc::channel();
                            sender.send(ProbeInvocation { response, permit }).is_ok()
                                && wait_for_result(&stream, receiver, gate, &cancelled, stop)
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if completed {
                        json!({ "content": [{ "type": "text", "text": RESULT }] })
                    } else {
                        json!({ "isError": true, "content": [{ "type": "text", "text": "Yo secret-entry probe was unavailable or cancelled." }] })
                    }
                },
                "notifications/cancelled" => {
                    if let Some(request_id) = message
                        .get("params")
                        .and_then(|params| params.get("requestId"))
                    {
                        cancel_call(gate, request_id);
                    }
                    json!({})
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

fn ingress(gate: &Mutex<GateState>) -> Ingress {
    let state = gate.lock().expect("probe gate");
    Ingress {
        generation: state.generation,
        active: state.active,
    }
}

fn admit(gate: &Arc<Mutex<GateState>>, call_id: &Value, ingress: Ingress) -> Option<ProbePermit> {
    let mut state = gate.lock().expect("probe gate");
    if !ingress.active || !state.active || state.generation != ingress.generation || state.in_flight
    {
        return None;
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    state.in_flight = true;
    state.call_id = Some(call_id.clone());
    state.cancelled = Some(Arc::clone(&cancelled));
    Some(ProbePermit {
        gate: Arc::clone(gate),
        generation: ingress.generation,
        cancelled,
    })
}

fn cancel_call(gate: &Mutex<GateState>, call_id: &Value) {
    let state = gate.lock().expect("probe gate");
    if state.call_id.as_ref() == Some(call_id)
        && let Some(cancelled) = &state.cancelled
    {
        cancelled.store(true, Ordering::Relaxed);
    }
}

fn cancel_current(gate: &Mutex<GateState>, cancelled: &Arc<AtomicBool>) -> bool {
    let state = gate.lock().expect("probe gate");
    if state
        .cancelled
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, cancelled))
    {
        cancelled.store(true, Ordering::Relaxed);
        true
    } else {
        false
    }
}

fn cancellation_result(
    gate: &Mutex<GateState>,
    cancelled: &Arc<AtomicBool>,
    receiver: &Receiver<bool>,
) -> bool {
    if cancel_current(gate, cancelled) {
        false
    } else {
        // A completed call clears the gate only while delivering its result.
        // If completion won, keep that result even if the socket closed after it.
        receiver
            .recv_timeout(Duration::from_millis(100))
            .unwrap_or(false)
    }
}

fn wait_for_result(
    stream: &TcpStream,
    receiver: Receiver<bool>,
    gate: &Mutex<GateState>,
    cancelled: &Arc<AtomicBool>,
    stop: &AtomicBool,
) -> bool {
    let _ = stream.set_nonblocking(true);
    let deadline = Instant::now() + Duration::from_secs(600);
    let completed = loop {
        if stop.load(Ordering::Relaxed)
            || cancelled.load(Ordering::Relaxed)
            || Instant::now() >= deadline
        {
            break cancellation_result(gate, cancelled, &receiver);
        }
        match receiver.try_recv() {
            Ok(result) => break result,
            Err(TryRecvError::Disconnected) => break false,
            Err(TryRecvError::Empty) => {},
        }
        match stream.peek(&mut [0]) {
            Ok(0) => {
                break cancellation_result(gate, cancelled, &receiver);
            },
            Err(error) if error.kind() == ErrorKind::WouldBlock => {},
            Err(_) => {
                break cancellation_result(gate, cancelled, &receiver);
            },
            // HTTP MCP does not permit another request on this one-shot connection. Leaving
            // the byte unread would make this loop spin until its long response deadline.
            Ok(_) => break cancellation_result(gate, cancelled, &receiver),
        }
        thread::sleep(Duration::from_millis(50));
    };
    let _ = stream.set_nonblocking(false);
    completed
}

fn read_request(stream: &TcpStream, path: &str, stop: &AtomicBool) -> Result<Option<Value>, ()> {
    let mut reader = stream;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut line = String::new();
    read_bounded_line(&mut reader, &mut line, 8192, stop, deadline)?;
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
        read_bounded_line(&mut reader, &mut line, 8192 - header_bytes, stop, deadline)?;
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
    let mut filled = 0;
    while filled < length {
        if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
            return Err(());
        }
        match reader.read(&mut body[filled..]) {
            Ok(0) => return Err(()),
            Ok(read) => filled += read,
            Err(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {},
            Err(_) => return Err(()),
        }
    }
    if extra_request_bytes(stream)? {
        return Err(());
    }
    serde_json::from_slice(&body).map(Some).map_err(|_| ())
}

fn extra_request_bytes(stream: &TcpStream) -> Result<bool, ()> {
    stream.set_nonblocking(true).map_err(|_| ())?;
    let observed = match stream.peek(&mut [0]) {
        Ok(0) => Ok(false),
        Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(false),
        Ok(_) => Ok(true),
        Err(_) => Err(()),
    };
    stream.set_nonblocking(false).map_err(|_| ())?;
    observed
}

fn read_bounded_line(
    reader: &mut impl Read,
    line: &mut String,
    limit: usize,
    stop: &AtomicBool,
    deadline: Instant,
) -> Result<(), ()> {
    for _ in 0..limit {
        if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
            return Err(());
        }
        let mut byte = [0];
        match reader.read_exact(&mut byte) {
            Ok(()) => {},
            Err(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                continue;
            },
            Err(_) => return Err(()),
        }
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
        if self
            .pending_probe
            .as_ref()
            .is_some_and(|probe| probe.permit.cancelled.load(Ordering::Relaxed))
        {
            self.cancel_pending_secret_probe();
            return Ok(self.pending_events.pop_front());
        }
        let Some(invocation) = self
            .secret_probe
            .as_ref()
            .map(SecretProbeBridge::try_receive)
            .transpose()?
            .flatten()
        else {
            return Ok(None);
        };
        if !self
            .secret_probe
            .as_ref()
            .is_some_and(|bridge| bridge.accepts(&invocation))
        {
            let _ = invocation.response.send(false);
            return Ok(None);
        }
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
            permit: invocation.permit,
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
        let mut state = probe.permit.gate.lock().expect("probe gate");
        let cancelled = !state.active
            || state.generation != probe.permit.generation
            || probe.permit.cancelled.load(Ordering::Relaxed);
        if cancelled {
            drop(state);
            let _ = probe.response.send(false);
            self.pending_events
                .push_back(BackendEvent::ActivityFinished {
                    activity: probe.activity,
                    outcome: ActivityOutcome::Interrupted,
                });
            return Ok(BackendCommandEvidence::None);
        }
        state.in_flight = false;
        state.call_id = None;
        state.cancelled = None;
        // Hold the gate through delivery so cancellation cannot win between
        // the status check and the fixed MCP result handoff.
        let completed = probe.response.send(true).is_ok();
        drop(state);
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
        if let Some(bridge) = &self.secret_probe {
            bridge.deactivate_and_drain();
        }
        if let Some(probe) = self.pending_probe.take() {
            let _ = probe.response.send(false);
            self.pending_events
                .push_back(BackendEvent::ActivityFinished {
                    activity: probe.activity,
                    outcome: ActivityOutcome::Interrupted,
                });
        }
    }

    fn cancel_pending_secret_probe(&mut self) {
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

#[cfg(test)]
mod tests {
    use super::*;

    // 완료 상태가 먼저 전달됐다면 뒤늦은 연결 종료는 그 결과를 취소로 바꾸지 않는다.
    #[test]
    fn completed_result_survives_late_shutdown() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        drop(client);
        let gate = Arc::new(Mutex::new(GateState {
            active: true,
            ..GateState::default()
        }));
        let permit = admit(&gate, &json!(1), ingress(&gate)).unwrap();
        let cancelled = Arc::clone(&permit.cancelled);
        let (sender, receiver) = mpsc::channel();
        {
            let mut state = gate.lock().unwrap();
            state.in_flight = false;
            state.call_id = None;
            state.cancelled = None;
            sender.send(true).unwrap();
        }
        assert!(wait_for_result(
            &server,
            receiver,
            &gate,
            &cancelled,
            &AtomicBool::new(true),
        ));
    }

    // Turn A에서 시작한 요청은 Turn B가 활성화된 뒤 본문을 끝내도 B의 입력창을 열 수 없다.
    #[test]
    fn stale_ingress_cannot_admit_after_a_new_generation() {
        let gate = Arc::new(Mutex::new(GateState {
            active: true,
            generation: 7,
            ..GateState::default()
        }));
        let observed = ingress(&gate);
        {
            let mut state = gate.lock().unwrap();
            state.generation = 8;
        }
        assert!(admit(&gate, &json!(1), observed).is_none());
    }
}
