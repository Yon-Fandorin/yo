//! Grok 로컬 MCP와 Yo 숨김 입력 사이의 위임형 비밀 상호작용입니다.

use std::{
    collections::HashSet,
    io::{self, ErrorKind, Read, Write},
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
    BackendFailureKind, SecretInput,
};

use super::state::Backend;
use crate::{protocol, transport::JsonPeer};

const PROBE_TOOL_NAME: &str = "yo_secret_entry_probe";
pub(super) const DELIVERY_TOOL_NAME: &str = "yo_request_secret_input";
const DELIVERY_SERVER_NAME: &str = "yo_secret_input";
const DELIVERY_QUALIFIED_TOOL_NAME: &str = "yo_secret_input__yo_request_secret_input";
const MAX_REQUEST_BYTES: usize = 32 * 1024;
const MAX_CONNECTIONS: usize = 16;
const PROBE_RESULT: &str = "Sample secret entry verified locally and discarded by Yo. No value was sent to Grok or the model.";
pub(super) const PROTECTED_RESULT: &str = "Protected secret tool result omitted by Yo.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SecretToolMode {
    Probe,
    Deliver,
}

pub(super) fn is_generic_use_tool_update(update: &Value) -> bool {
    update.get("name").and_then(Value::as_str) == Some("use_tool")
}

pub(super) fn has_mcp_target_identity(update: &Value) -> bool {
    update
        .get("rawInput")
        .and_then(Value::as_object)
        .and_then(|input| input.get("tool_name"))
        .and_then(Value::as_str)
        .is_some()
        || update
            .get("rawOutput")
            .and_then(Value::as_object)
            .is_some_and(|output| {
                output.get("type").and_then(Value::as_str) == Some("MCP")
                    && output.get("server_name").and_then(Value::as_str).is_some()
                    && output.get("tool_name").and_then(Value::as_str).is_some()
            })
}

pub(super) fn is_delivery_tool_update(update: &Value) -> bool {
    update.get("name").and_then(Value::as_str) == Some(DELIVERY_QUALIFIED_TOOL_NAME)
        || update
            .get("rawInput")
            .and_then(Value::as_object)
            .and_then(|input| input.get("tool_name"))
            .and_then(Value::as_str)
            == Some(DELIVERY_QUALIFIED_TOOL_NAME)
        || update
            .get("rawOutput")
            .and_then(Value::as_object)
            .is_some_and(|output| {
                output.get("type").and_then(Value::as_str) == Some("MCP")
                    && output.get("server_name").and_then(Value::as_str)
                        == Some(DELIVERY_SERVER_NAME)
                    && output.get("tool_name").and_then(Value::as_str) == Some(DELIVERY_TOOL_NAME)
            })
}

enum BridgeResult {
    ProbeCompleted,
    Secret(SecretInput),
}

struct SecretRequest {
    title: String,
    question: String,
    purpose: String,
}

pub(super) struct ProbeInvocation {
    response: Sender<BridgeResult>,
    completion: Receiver<bool>,
    request: Option<SecretRequest>,
    permit: ProbePermit,
}

pub(super) struct PendingProbe {
    pub(super) request: ActivityRequestRef,
    activity: ActivityRef,
    response: Sender<BridgeResult>,
    completion: Receiver<bool>,
    secret_request: Option<SecretRequest>,
    permit: ProbePermit,
}

#[derive(Default)]
struct GateState {
    generation: u64,
    active: bool,
    in_flight: bool,
    call_id: Option<Value>,
    cancelled: Option<Arc<AtomicBool>>,
    seen_call_ids: HashSet<String>,
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
    mode: SecretToolMode,
    incoming: Receiver<ProbeInvocation>,
    gate: Arc<Mutex<GateState>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl SecretProbeBridge {
    pub(super) fn start(mode: SecretToolMode) -> Result<Self, BackendFailure> {
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
        let worker_mode = mode;
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
                match accept_with_ingress(&listener, &worker_gate) {
                    Ok((stream, accepted_ingress)) => {
                        if workers.len() >= MAX_CONNECTIONS {
                            continue;
                        }
                        let sender = sender.clone();
                        let path = path.clone();
                        let gate = Arc::clone(&worker_gate);
                        let stop = Arc::clone(&worker_stop);
                        workers.push(thread::spawn(move || {
                            serve_connection(
                                stream,
                                &path,
                                &sender,
                                &gate,
                                accepted_ingress,
                                &stop,
                                worker_mode,
                            )
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
            mode,
            incoming,
            gate,
            stop,
            worker: Some(worker),
        })
    }

    pub(super) fn server_spec(&self) -> Value {
        let name = match self.mode {
            SecretToolMode::Probe => PROBE_TOOL_NAME,
            SecretToolMode::Deliver => DELIVERY_SERVER_NAME,
        };
        json!({ "type": "http", "name": name, "url": self.url, "headers": [] })
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
        state.seen_call_ids.clear();
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
        state.seen_call_ids.clear();
        drop(state);
        while let Ok(invocation) = self.incoming.try_recv() {
            invocation.permit.cancelled.store(true, Ordering::Relaxed);
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
    accepted_ingress: Ingress,
    stop: &AtomicBool,
    mode: SecretToolMode,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let result = read_request(&stream, path, stop);
    let mut completion = None;
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
                    "serverInfo": { "name": server_name(mode), "version": "1" }
                }),
                "tools/list" => json!({ "tools": [tool_spec(mode)] }),
                "tools/call" => {
                    let request = call_request(mode, &message);
                    let result = if let (Some(id), Some(request)) = (id.as_ref(), request) {
                        let permit = admit(gate, id, accepted_ingress);
                        if let Some(permit) = permit {
                            let cancelled = Arc::clone(&permit.cancelled);
                            let (response, receiver) = mpsc::channel();
                            let (completion_sender, completion_receiver) = mpsc::channel();
                            let invocation = ProbeInvocation {
                                response,
                                completion: completion_receiver,
                                request,
                                permit,
                            };
                            if sender.send(invocation).is_ok() {
                                let result =
                                    wait_for_result(&stream, receiver, gate, &cancelled, stop);
                                if result.is_some() {
                                    completion = Some(completion_sender);
                                }
                                result
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    match result {
                        Some(BridgeResult::ProbeCompleted) => {
                            json!({ "content": [{ "type": "text", "text": PROBE_RESULT }] })
                        },
                        Some(BridgeResult::Secret(secret)) => {
                            let body = json!({
                                "content": [{ "type": "text", "text": secret.expose() }]
                            });
                            drop(secret);
                            body
                        },
                        None => json!({
                            "isError": true,
                            "content": [{
                                "type": "text",
                                "text": "Yo delegated secret interaction was unavailable or cancelled."
                            }]
                        }),
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
    let written = stream.write_all(header.as_bytes()).is_ok()
        && stream.write_all(payload.as_bytes()).is_ok()
        && stream.flush().is_ok();
    if let Some(completion) = completion {
        let _ = completion.send(written);
    }
}

fn server_name(mode: SecretToolMode) -> &'static str {
    match mode {
        SecretToolMode::Probe => PROBE_TOOL_NAME,
        SecretToolMode::Deliver => DELIVERY_SERVER_NAME,
    }
}

fn tool_spec(mode: SecretToolMode) -> Value {
    match mode {
        SecretToolMode::Probe => json!({
            "name": PROBE_TOOL_NAME,
            "description": "Diagnostic only: ask for a MADE-UP sample in Yo's hidden editor. Yo discards it and returns only a fixed status. Never request a real password, token, or credential; the model cannot use the entered value.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        SecretToolMode::Deliver => json!({
            "name": DELIVERY_TOOL_NAME,
            "description": "Ask the user for one secret value needed for the current task. Yo sends the answer once to Grok and its selected model. They may retain or reuse it. Arguments must contain only the public request.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short public title for the secret request." },
                    "question": { "type": "string", "description": "Public question shown before secret entry." },
                    "purpose": { "type": "string", "description": "Public reason the current task needs the secret." }
                },
                "required": ["title", "question", "purpose"],
                "additionalProperties": false
            }
        }),
    }
}

fn call_request(mode: SecretToolMode, message: &Value) -> Option<Option<SecretRequest>> {
    let params = message.get("params")?;
    let expected_name = match mode {
        SecretToolMode::Probe => PROBE_TOOL_NAME,
        SecretToolMode::Deliver => DELIVERY_TOOL_NAME,
    };
    if params.get("name")?.as_str()? != expected_name {
        return None;
    }
    let arguments = match params.get("arguments") {
        None => serde_json::Map::new(),
        Some(arguments) => arguments.as_object()?.clone(),
    };
    match mode {
        SecretToolMode::Probe => arguments.is_empty().then_some(None),
        SecretToolMode::Deliver => parse_secret_request(&arguments).map(Some),
    }
}

fn parse_secret_request(arguments: &serde_json::Map<String, Value>) -> Option<SecretRequest> {
    if arguments.len() != 3
        || !arguments
            .keys()
            .all(|key| matches!(key.as_str(), "title" | "question" | "purpose"))
    {
        return None;
    }
    Some(SecretRequest {
        title: public_argument(arguments, "title", 80, false)?.to_owned(),
        question: public_argument(arguments, "question", 4096, true)?.to_owned(),
        purpose: public_argument(arguments, "purpose", 4096, true)?.to_owned(),
    })
}

fn public_argument<'a>(
    arguments: &'a serde_json::Map<String, Value>,
    name: &str,
    max_bytes: usize,
    multiline: bool,
) -> Option<&'a str> {
    let value = arguments.get(name)?.as_str()?;
    let invalid_control = value.chars().any(|character| {
        character.is_control() && !(multiline && matches!(character, '\t' | '\n' | '\r'))
    });
    (!value.is_empty() && value.len() <= max_bytes && !invalid_control).then_some(value)
}

fn accept_with_ingress(
    listener: &TcpListener,
    gate: &Mutex<GateState>,
) -> io::Result<(TcpStream, Ingress)> {
    // accept와 세대 관찰을 같은 gate 임계 구역에 두어 Turn 전환이 둘 사이에 끼지 못합니다.
    let state = gate.lock().expect("probe gate");
    let (stream, _) = listener.accept()?;
    let accepted_ingress = Ingress {
        generation: state.generation,
        active: state.active,
    };
    drop(state);
    Ok((stream, accepted_ingress))
}

fn admit(gate: &Arc<Mutex<GateState>>, call_id: &Value, ingress: Ingress) -> Option<ProbePermit> {
    let call_key = serde_json::to_string(call_id).ok()?;
    let mut state = gate.lock().expect("probe gate");
    if !ingress.active
        || !state.active
        || state.generation != ingress.generation
        || state.in_flight
        || state.seen_call_ids.contains(&call_key)
    {
        return None;
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    state.in_flight = true;
    state.call_id = Some(call_id.clone());
    state.cancelled = Some(Arc::clone(&cancelled));
    state.seen_call_ids.insert(call_key);
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
    receiver: &Receiver<BridgeResult>,
) -> Option<BridgeResult> {
    if cancel_current(gate, cancelled) {
        None
    } else {
        // 제출이 gate를 먼저 소비했다면 뒤늦게 소켓이 닫혀도 그 결과를 한 번 씁니다.
        receiver.recv_timeout(Duration::from_millis(100)).ok()
    }
}

fn wait_for_result(
    stream: &TcpStream,
    receiver: Receiver<BridgeResult>,
    gate: &Mutex<GateState>,
    cancelled: &Arc<AtomicBool>,
    stop: &AtomicBool,
) -> Option<BridgeResult> {
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
            Ok(result) => break Some(result),
            Err(TryRecvError::Disconnected) => break None,
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
            // 한 번 쓰는 HTTP MCP 연결에서 추가 바이트는 취소로 처리해 긴 대기를 막습니다.
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
    pub(super) fn prepare_secret_tool(&mut self) -> Result<(), BackendFailure> {
        if self.secret_probe.is_none()
            && self.mcp_http
            && let Some(mode) = self.secret_tool_mode
        {
            self.secret_probe = Some(SecretProbeBridge::start(mode)?);
        }
        Ok(())
    }

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
            return Ok(None);
        }
        let turn = match self.prompt.as_ref() {
            Some(prompt) if !prompt.interrupt_requested && self.pending_probe.is_none() => {
                prompt.turn
            },
            _ => {
                return Ok(None);
            },
        };
        self.ensure_activity_capacity()?;
        let activity = self.next_activity(turn)?;
        let request_id = self.next_request()?;
        let request = ActivityRequestRef::new(activity, request_id);
        let plain_text = invocation.request.as_ref().map_or_else(
            || "Sample input test\n\nEnter a made-up sample value only. Never enter a real password, token, or credential. Yo discards this value and sends only a fixed completion status to Grok.\nEsc interrupts the turn.".to_owned(),
            |request| format!(
                "{}\n\n{}\n\nPurpose: {}\n\nSubmitting sends this value once to Grok and its selected model. They may retain or reuse it. Yo does not save this input for reuse.\nEnter submits once; Esc interrupts the turn.",
                request.title, request.question, request.purpose
            ),
        );
        let snapshot = ActivityQuestion {
            plain_text,
            choices: Vec::new(),
            allow_notes: false,
            is_secret: true,
            storage_offer: None,
            previous_question: false,
            draft: None,
            draft_choice: None,
        }
        .to_snapshot()
        .ok_or_else(|| {
            protocol::protocol_failure("Grok secret-entry probe prompt exceeded its display limit")
        })?;
        self.pending_probe = Some(PendingProbe {
            request,
            activity,
            response: invocation.response,
            completion: invocation.completion,
            secret_request: invocation.request,
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
        let ActivityResponse::SecretInput(input) = response else {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Grok delegated secret interaction requires hidden input",
            ));
        };
        let probe = self.pending_probe.take().expect("matched pending probe");
        let delivery = probe.secret_request.is_some();
        let result = if delivery {
            BridgeResult::Secret(input)
        } else {
            drop(input);
            BridgeResult::ProbeCompleted
        };
        let mut state = probe.permit.gate.lock().expect("probe gate");
        let cancelled = !state.active
            || state.generation != probe.permit.generation
            || probe.permit.cancelled.load(Ordering::Relaxed);
        if cancelled {
            drop(state);
            self.pending_events
                .push_back(BackendEvent::ActivityFinished {
                    activity: probe.activity,
                    outcome: ActivityOutcome::Interrupted,
                });
            return Ok(BackendCommandEvidence::None);
        }
        // 응답 채널로 넘기는 순간 이 요청을 소비합니다. 이후 취소나 실패는 같은 값을
        // 다시 보내지 못하며, writer 완료를 기다리는 동안 다른 호출도 받지 않습니다.
        state.call_id = None;
        state.cancelled = None;
        let handed_off = probe.response.send(result).is_ok();
        drop(state);
        let written = handed_off
            && probe
                .completion
                .recv_timeout(Duration::from_secs(6))
                .unwrap_or(false);
        let mut state = probe.permit.gate.lock().expect("probe gate");
        if state.generation == probe.permit.generation {
            state.in_flight = false;
        }
        drop(state);
        self.pending_events
            .push_back(BackendEvent::ActivityFinished {
                activity: probe.activity,
                outcome: if written {
                    ActivityOutcome::Completed
                } else {
                    ActivityOutcome::Interrupted
                },
            });
        if !handed_off {
            return Err(protocol::protocol_failure(
                "Grok delegated secret result receiver closed before handoff",
            ));
        }
        if !written {
            return Err(BackendFailure::new(
                BackendFailureKind::ProcessExit,
                "delegated secret delivery failed with an unknown outcome",
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
                update: ActivityUpdate::TextSnapshot(if delivery {
                    "Secret submitted once to Grok. Yo did not save it for reuse.".into()
                } else {
                    "Sample secret entry verified locally and discarded.".into()
                }),
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
            self.pending_events
                .push_back(BackendEvent::ActivityFinished {
                    activity: probe.activity,
                    outcome: ActivityOutcome::Interrupted,
                });
        }
    }

    fn cancel_pending_secret_probe(&mut self) {
        if let Some(probe) = self.pending_probe.take() {
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
    use yo_backend::transport::JsonMessagePeer;
    use yo_core::{
        ActivityId, AgentCommand, BackendStopHandle, RequestId, SessionId, TurnId, TurnRef,
    };

    use super::*;
    use crate::{client::AcpClient, transport::PeerPoll};

    struct NoopPeer;

    impl JsonMessagePeer for NoopPeer {
        fn stop_handle(&self) -> BackendStopHandle {
            BackendStopHandle::no_op()
        }

        fn send(&mut self, _: &Value) -> Result<(), BackendFailure> {
            Ok(())
        }

        fn receive(&mut self, _: Duration) -> Result<PeerPoll, BackendFailure> {
            Ok(PeerPoll::Pending)
        }

        fn try_receive(&mut self) -> Result<PeerPoll, BackendFailure> {
            Ok(PeerPoll::Pending)
        }

        fn shutdown(&mut self) -> Result<(), BackendFailure> {
            Ok(())
        }
    }

    // 공개 요청 인자는 정확한 필드와 UTF-8 byte 경계를 지키며 허용된 줄바꿈만 받습니다.
    #[test]
    fn delivery_request_validates_public_argument_shape_and_bounds() {
        let valid = serde_json::Map::from_iter([
            ("title".to_owned(), json!("한".repeat(26))),
            ("question".to_owned(), json!("첫 줄\n둘째 줄\t설명")),
            ("purpose".to_owned(), json!("현재 요청 인증")),
        ]);
        assert!(parse_secret_request(&valid).is_some());

        for invalid in [
            serde_json::Map::from_iter([
                ("title".to_owned(), json!("x".repeat(81))),
                ("question".to_owned(), json!("질문")),
                ("purpose".to_owned(), json!("목적")),
            ]),
            serde_json::Map::from_iter([
                ("title".to_owned(), json!("제목\n노출")),
                ("question".to_owned(), json!("질문")),
                ("purpose".to_owned(), json!("목적")),
            ]),
            serde_json::Map::from_iter([
                ("title".to_owned(), json!("제목")),
                ("question".to_owned(), json!("질문\u{0}")),
                ("purpose".to_owned(), json!("목적")),
            ]),
            serde_json::Map::from_iter([
                ("title".to_owned(), json!("제목")),
                ("question".to_owned(), json!("질문")),
                ("purpose".to_owned(), json!("목적")),
                ("secret".to_owned(), json!("공개 인자에 둘 수 없음")),
            ]),
        ] {
            assert!(parse_secret_request(&invalid).is_none());
        }
    }

    // HTTP writer가 결과 수신 뒤 완료 확인을 주지 못하면 실제 전달 여부가 불명확하므로
    // 성공 영수증 없이 요청을 소비하고 같은 값을 다시 보낼 수 없게 합니다.
    #[test]
    fn delivery_writer_failure_is_unknown_and_non_retryable() {
        use std::num::NonZeroU64;

        let session = SessionId::from_uuid(uuid::Uuid::from_u128(
            0x0189_0f00_0000_7000_8000_0000_0000_0001,
        ))
        .unwrap();
        let turn = TurnRef::new(session, TurnId::new(NonZeroU64::new(1).unwrap()));
        let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
        let request =
            ActivityRequestRef::new(activity, RequestId::new(NonZeroU64::new(1).unwrap()));
        let cancelled = Arc::new(AtomicBool::new(false));
        let gate = Arc::new(Mutex::new(GateState {
            active: true,
            in_flight: true,
            call_id: Some(json!(41)),
            cancelled: Some(Arc::clone(&cancelled)),
            ..GateState::default()
        }));
        let permit = ProbePermit {
            gate: Arc::clone(&gate),
            generation: 0,
            cancelled,
        };
        let (response, delivered) = mpsc::channel();
        let (completion, completion_receiver) = mpsc::channel::<bool>();
        drop(completion);
        let mut backend = Backend::new_uninitialized(
            AcpClient::new(NoopPeer, Duration::from_secs(1)),
            "/workspace".into(),
            false,
        );
        backend.pending_probe = Some(PendingProbe {
            request,
            activity,
            response,
            completion: completion_receiver,
            secret_request: Some(SecretRequest {
                title: "Token".into(),
                question: "Enter token".into(),
                purpose: "Authenticate".into(),
            }),
            permit,
        });

        let failure = backend
            .respond_to_secret_probe(ActivityResponse::SecretInput(
                SecretInput::new("single-use-secret").unwrap(),
            ))
            .unwrap_err();
        assert_eq!(
            failure.message(),
            "delegated secret delivery failed with an unknown outcome"
        );
        assert!(!failure.message().contains("single-use-secret"));
        let BridgeResult::Secret(secret) = delivered.recv().unwrap() else {
            panic!("delivery result")
        };
        assert_eq!(secret.expose(), "single-use-secret");
        assert!(backend.pending_probe.is_none());
        assert!(backend.pending_events.iter().all(|event| {
            !format!("{event:?}").contains("single-use-secret")
                && !matches!(event, BackendEvent::ActivityUpdated { .. })
        }));
        let state = gate.lock().unwrap();
        assert!(!state.in_flight);
        assert!(state.call_id.is_none());
        drop(state);

        let retry = backend
            .execute_command(AgentCommand::RespondToActivity {
                request,
                response: ActivityResponse::SecretInput(SecretInput::new("retry-secret").unwrap()),
            })
            .unwrap_err();
        assert!(retry.message().contains("no matching Grok request"));
    }

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
        let permit = admit(
            &gate,
            &json!(1),
            Ingress {
                generation: 0,
                active: true,
            },
        )
        .unwrap();
        let cancelled = Arc::clone(&permit.cancelled);
        let (sender, receiver) = mpsc::channel();
        {
            let mut state = gate.lock().unwrap();
            state.in_flight = false;
            state.call_id = None;
            state.cancelled = None;
            sender.send(BridgeResult::ProbeCompleted).unwrap();
        }
        assert!(
            wait_for_result(&server, receiver, &gate, &cancelled, &AtomicBool::new(true),)
                .is_some()
        );
    }

    // Turn A에서 accept한 연결은 worker가 Turn B에서 실행되어도 B의 입력창을 열 수 없다.
    #[test]
    fn accepted_connection_cannot_admit_after_a_new_generation() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let gate = Arc::new(Mutex::new(GateState {
            active: true,
            generation: 7,
            ..GateState::default()
        }));
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let (server, accepted_ingress) = accept_with_ingress(&listener, &gate).unwrap();
        {
            let mut state = gate.lock().unwrap();
            state.generation = 8;
        }
        let (sender, receiver) = mpsc::channel();
        let worker_gate = Arc::clone(&gate);
        let worker = thread::spawn(move || {
            serve_connection(
                server,
                "/mcp/test",
                &sender,
                &worker_gate,
                accepted_ingress,
                &AtomicBool::new(false),
                SecretToolMode::Probe,
            )
        });
        let body = json!({
            "jsonrpc":"2.0", "id":1, "method":"tools/call",
            "params":{"name":PROBE_TOOL_NAME}
        })
        .to_string();
        write!(
            client,
            "POST /mcp/test HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        worker.join().unwrap();
        assert!(response.contains("isError"));
        assert!(receiver.try_recv().is_err());
    }
}
