use std::{
    any::Any,
    env,
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::Read,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const CHILD_STDERR_LIMIT: usize = 4 * 1024;

pub(super) enum LocalServerMode {
    Success {
        body: Vec<u8>,
        content_type: String,
    },
    Status {
        status: u16,
        body: Vec<u8>,
    },
    Redirect {
        location: String,
        final_body: Vec<u8>,
    },
    HeadersThenStall {
        content_type: String,
    },
    EventThenStall {
        body: Vec<u8>,
    },
    ResponseHeaderStall,
    TlsHandshakeStall,
}

pub(super) struct LocalTlsServer {
    _child: ChildGuard,
    _root: TempDirectory,
    requests: PathBuf,
    accepted: PathBuf,
    sent: PathBuf,
    closed: PathBuf,
    endpoint: String,
}

impl LocalTlsServer {
    pub(super) fn start(mode: LocalServerMode) -> Self {
        let certificate = env::var_os("YO_MODEL_CONNECTOR_TEST_CERT")
            .expect("the local TLS child must provide its test certificate path");
        let key = env::var_os("YO_MODEL_CONNECTOR_TEST_KEY")
            .expect("the local TLS child must provide its test key path");
        let root = TempDirectory::new("yo-model-connector-server");
        let ready = root.path().join("ready");
        let requests = root.path().join("requests");
        let accepted = root.path().join("accepted");
        let sent = root.path().join("sent");
        let closed = root.path().join("closed");
        let payload = root.path().join("payload");
        for path in [&ready, &requests, &accepted, &sent, &closed, &payload] {
            create_private_file(path);
        }
        let (mode, content_type, status, location, max_connections, payload_bytes) = match mode {
            LocalServerMode::Success { body, content_type } => {
                ("success", content_type, 200, String::new(), 1, body)
            },
            LocalServerMode::Status { status, body } => (
                "status",
                "text/plain; charset=utf-8".to_owned(),
                status,
                String::new(),
                1,
                body,
            ),
            LocalServerMode::Redirect {
                location,
                final_body,
            } => (
                "redirect",
                "text/event-stream".to_owned(),
                307,
                location,
                2,
                final_body,
            ),
            LocalServerMode::HeadersThenStall { content_type } => (
                "headers-stall",
                content_type,
                200,
                String::new(),
                1,
                Vec::new(),
            ),
            LocalServerMode::EventThenStall { body } => (
                "event-stall",
                "text/event-stream".to_owned(),
                200,
                String::new(),
                1,
                body,
            ),
            LocalServerMode::ResponseHeaderStall => (
                "header-stall",
                String::new(),
                0,
                String::new(),
                1,
                Vec::new(),
            ),
            LocalServerMode::TlsHandshakeStall => {
                ("tls-stall", String::new(), 0, String::new(), 1, Vec::new())
            },
        };
        fs::write(&payload, payload_bytes).unwrap();
        let child = spawn_local_tls_child(LocalTlsChildSpec {
            script: include_str!("local_tls_server.py"),
            certificate: certificate.as_os_str(),
            key: key.as_os_str(),
            ready: &ready,
            requests: &requests,
            accepted: &accepted,
            sent: &sent,
            mode,
            closed: &closed,
            payload: &payload,
            content_type: &content_type,
            status,
            location: &location,
            max_connections,
        });
        let (child, port) = wait_for_local_tls_ready(child, &ready, Duration::from_secs(2));
        Self {
            _child: child,
            _root: root,
            requests,
            accepted,
            sent,
            closed,
            endpoint: format!("https://127.0.0.1:{port}/v1"),
        }
    }

    pub(super) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(super) fn wait_for_response_sent(&self) {
        self.wait_for_marker(
            &self.sent,
            "local TLS listener did not report its response boundary",
        );
    }

    pub(super) fn wait_for_peer_closed(&self) {
        self.wait_for_marker(
            &self.closed,
            "local TLS listener did not observe the connector closing its peer",
        );
    }

    pub(super) fn accepted_connections(&self) -> usize {
        self.marker_count(&self.accepted)
    }

    pub(super) fn requests(&self) -> Vec<serde_json::Value> {
        let mut source = String::new();
        File::open(&self.requests)
            .unwrap()
            .read_to_string(&mut source)
            .unwrap();
        source
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn marker_count(&self, marker: &Path) -> usize {
        let mut source = String::new();
        File::open(marker)
            .unwrap()
            .read_to_string(&mut source)
            .unwrap();
        source.lines().filter(|line| !line.is_empty()).count()
    }

    fn wait_for_marker(&self, marker: &Path, message: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.marker_count(marker) == 0 {
            assert!(Instant::now() < deadline, "{message}");
            thread::sleep(Duration::from_millis(1));
        }
    }
}

struct LocalTlsChildSpec<'a> {
    script: &'a str,
    certificate: &'a OsStr,
    key: &'a OsStr,
    ready: &'a Path,
    requests: &'a Path,
    accepted: &'a Path,
    sent: &'a Path,
    mode: &'a str,
    closed: &'a Path,
    payload: &'a Path,
    content_type: &'a str,
    status: u16,
    location: &'a str,
    max_connections: usize,
}

fn spawn_local_tls_child(spec: LocalTlsChildSpec<'_>) -> ChildGuard {
    let child = Command::new("python3")
        .arg("-c")
        .arg(spec.script)
        .arg(spec.certificate)
        .arg(spec.key)
        .arg(spec.ready)
        .arg(spec.requests)
        .arg(spec.accepted)
        .arg(spec.sent)
        .arg(spec.mode)
        .arg(spec.closed)
        .arg(spec.payload)
        .arg(spec.content_type)
        .arg(spec.status.to_string())
        .arg(spec.location)
        .arg(spec.max_connections.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python3 is required for the local TLS listener fixture");
    let mut sensitive_paths = vec![
        spec.certificate,
        spec.key,
        spec.ready.as_os_str(),
        spec.requests.as_os_str(),
        spec.accepted.as_os_str(),
        spec.sent.as_os_str(),
        spec.closed.as_os_str(),
        spec.payload.as_os_str(),
    ];
    if let Some(server_root) = spec.ready.parent() {
        sensitive_paths.push(server_root.as_os_str());
    }
    ChildGuard::new(
        child,
        child_sensitive_values(&sensitive_paths, spec.location, spec.key),
    )
}

fn wait_for_local_tls_ready(
    mut child: ChildGuard,
    ready: &Path,
    timeout: Duration,
) -> (ChildGuard, u16) {
    let deadline = Instant::now() + timeout;
    let port = loop {
        child.assert_running("local TLS listener exited before publishing its port");
        if let Ok(value) = fs::read_to_string(ready)
            && let Ok(port) = value.trim().parse::<u16>()
        {
            break port;
        }
        assert!(
            Instant::now() < deadline,
            "{}",
            child.readiness_failure("local TLS listener published an invalid port")
        );
        thread::sleep(Duration::from_millis(1));
    };
    (child, port)
}

struct ChildGuard {
    child: Option<Child>,
    stderr: Arc<Mutex<CapturedStderr>>,
    stderr_reader: Option<thread::JoinHandle<()>>,
    sensitive_values: Option<Vec<String>>,
}

struct CapturedStderr {
    bytes: Vec<u8>,
    truncated: bool,
}

impl ChildGuard {
    fn new(child: Child, sensitive_values: Option<Vec<String>>) -> Self {
        let mut child = child;
        let stderr = Arc::new(Mutex::new(CapturedStderr {
            bytes: Vec::with_capacity(CHILD_STDERR_LIMIT),
            truncated: false,
        }));
        let stderr_reader = child.stderr.take().map(|pipe| {
            let captured = Arc::clone(&stderr);
            thread::spawn(move || capture_stderr(pipe, captured))
        });
        Self {
            child: Some(child),
            stderr,
            stderr_reader,
            sensitive_values,
        }
    }

    fn assert_running(&mut self, message: &str) {
        let status = self
            .child
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap_or_else(|error| panic!("{message}: cannot inspect child process: {error}"));
        if let Some(status) = status {
            self.join_stderr_reader();
            panic!("{}", self.failure_message(message, status));
        }
    }

    fn readiness_failure(&mut self, message: &str) -> String {
        self.terminate();
        let status = self
            .child
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten());
        match status {
            Some(status) => self.failure_message(message, status),
            None => self.failure_message_without_status(message),
        }
    }

    fn terminate(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let running = child.try_wait().map_or(true, |status| status.is_none());
            if running {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        self.join_stderr_reader();
    }

    fn join_stderr_reader(&mut self) {
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }

    fn failure_message(&mut self, message: &str, status: std::process::ExitStatus) -> String {
        let mut diagnostic = format!(
            "{message} (child exited with {})",
            status
                .code()
                .map_or_else(|| "a signal".to_owned(), |code| code.to_string())
        );
        self.append_stderr(&mut diagnostic);
        diagnostic
    }

    fn failure_message_without_status(&mut self, message: &str) -> String {
        let mut diagnostic = message.to_owned();
        self.append_stderr(&mut diagnostic);
        diagnostic
    }

    fn append_stderr(&mut self, diagnostic: &mut String) {
        let Some(sensitive_values) = self.sensitive_values.as_deref() else {
            return;
        };
        let Ok(captured) = self.stderr.lock() else {
            return;
        };
        if captured.bytes.is_empty() {
            return;
        }
        diagnostic.push_str("; child stderr: ");
        diagnostic.push_str(&sanitize_stderr(&captured.bytes, sensitive_values));
        if captured.truncated {
            diagnostic.push_str(" [truncated]");
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn capture_stderr(mut stderr: impl Read, captured: Arc<Mutex<CapturedStderr>>) {
    let mut buffer = [0_u8; 512];
    loop {
        let read = match stderr.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        let Ok(mut captured) = captured.lock() else {
            return;
        };
        let remaining = CHILD_STDERR_LIMIT.saturating_sub(captured.bytes.len());
        let retained = read.min(remaining);
        captured.bytes.extend_from_slice(&buffer[..retained]);
        if retained < read {
            captured.truncated = true;
        }
    }
}

fn child_sensitive_values(paths: &[&OsStr], location: &str, key: &OsStr) -> Option<Vec<String>> {
    let mut values = paths
        .iter()
        .map(|path| safe_python_argument(path))
        .collect::<Option<Vec<_>>>()?;
    if !location.is_empty() {
        values.push(safe_python_text(location)?);
    }
    let key_contents = String::from_utf8(fs::read(key).ok()?).ok()?;
    values.push(key_contents);
    Some(values)
}

fn safe_python_argument(value: &OsStr) -> Option<String> {
    let value = value.to_str()?;
    safe_python_text(value)
}

fn safe_python_text(value: &str) -> Option<String> {
    if value.is_empty()
        || value.chars().any(|character| {
            !character.is_ascii_graphic() || matches!(character, '\\' | '\'' | '"')
        })
    {
        return None;
    }
    Some(value.to_owned())
}

fn sanitize_stderr(bytes: &[u8], sensitive_values: &[String]) -> String {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    for value in sensitive_values {
        if !value.is_empty() {
            text = text.replace(value, "[redacted]");
        }
    }
    text = redact_private_key_blocks(&text);
    text = redact_authorization_values(&text);
    truncate_diagnostic(text)
}

fn redact_private_key_blocks(text: &str) -> String {
    let mut redacted = String::with_capacity(text.len());
    let mut in_private_key = false;
    for line in text.split_inclusive('\n') {
        let marker = line.to_ascii_uppercase();
        if marker.contains("-----BEGIN") && marker.contains("PRIVATE KEY-----") {
            redacted.push_str("[redacted private key]\n");
            in_private_key = true;
        } else if in_private_key {
            if marker.contains("-----END") && marker.contains("PRIVATE KEY-----") {
                in_private_key = false;
            }
        } else {
            redacted.push_str(line);
        }
    }
    redacted
}

fn redact_authorization_values(text: &str) -> String {
    text.lines()
        .map(redact_authorization_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_authorization_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let Some(label) = lower.find("authorization") else {
        return redact_bearer_value(line);
    };
    let label_end = label + "authorization".len();
    let value_start = line[label_end..]
        .char_indices()
        .find(|(_, character)| !character.is_whitespace())
        .and_then(|(offset, character)| {
            matches!(character, ':' | '=').then_some(label_end + offset + 1)
        })
        .unwrap_or(label_end);
    let redacted = format!("{}[redacted]", &line[..value_start]);
    redact_bearer_value(&redacted)
}

fn redact_bearer_value(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let Some(scheme) = lower.find("bearer ") else {
        return line.to_owned();
    };
    let token_start = scheme + "bearer ".len();
    format!("{}[redacted]", &line[..token_start])
}

fn truncate_diagnostic(text: String) -> String {
    let mut end = text.len().min(CHILD_STDERR_LIMIT);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = text[..end].to_owned();
    if end < text.len() {
        truncated.push('…');
    }
    truncated
}

fn diagnostic_test_child(
    script: &str,
    location: &str,
) -> (ChildGuard, TempDirectory, PathBuf, PathBuf) {
    let root = TempDirectory::new("yo-model-connector-diagnostic");
    let certificate = root.path().join("certificate-private-path-sentinel");
    let key = root.path().join("key-private-path-sentinel");
    let ready = root.path().join("ready-private-path-sentinel");
    let requests = root.path().join("requests-private-path-sentinel");
    let accepted = root.path().join("accepted-private-path-sentinel");
    let sent = root.path().join("sent-private-path-sentinel");
    let closed = root.path().join("closed-private-path-sentinel");
    let payload = root.path().join("payload-private-path-sentinel");
    for path in [
        &certificate,
        &key,
        &ready,
        &requests,
        &accepted,
        &sent,
        &closed,
        &payload,
    ] {
        create_private_file(path);
    }
    fs::write(&key, b"private-key-bytes-must-not-escape").unwrap();
    let child = spawn_local_tls_child(LocalTlsChildSpec {
        script,
        certificate: certificate.as_os_str(),
        key: key.as_os_str(),
        ready: &ready,
        requests: &requests,
        accepted: &accepted,
        sent: &sent,
        mode: "success",
        closed: &closed,
        payload: &payload,
        content_type: "text/plain",
        status: 200,
        location,
        max_connections: 1,
    });
    (child, root, ready, sent)
}

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_owned();
    }
    "panic payload was not a string".to_owned()
}

fn wait_for_test_marker(path: &Path, expected: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if matches!(fs::read_to_string(path).as_deref(), Ok(value) if value == expected) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "test child did not publish its marker"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

// 실제 Python fixture의 stderr 파이프 배선에서 즉시 종료한 child의 assert_running 진단을
// 관찰하고, 모든 private argv path·location과 긴 stderr가 각각 비노출·bounded인지 고정합니다.
#[test]
fn reports_bounded_sanitized_stderr_for_an_early_child_exit() {
    let location = "location-private-path-sentinel";
    let (child, _root, ready, _started) = diagnostic_test_child(
        "import sys\nsys.stderr.write('listener early exit: ' + ' '.join(sys.argv[1:]) + '\\n' + 'x' * 1048576)\nsys.stderr.flush()\nsys.exit(23)",
        location,
    );
    let failure = catch_unwind(AssertUnwindSafe(|| {
        let _ = wait_for_local_tls_ready(child, &ready, Duration::from_secs(2));
    }))
    .expect_err("an early listener exit must fail readiness");
    let diagnostic = panic_message(failure);

    assert!(diagnostic.contains("local TLS listener exited before publishing its port"));
    assert!(diagnostic.contains("child exited with 23"));
    assert!(diagnostic.contains("listener early exit"));
    assert!(diagnostic.contains("[truncated]"));
    assert!(diagnostic.len() <= CHILD_STDERR_LIMIT + 128);
    assert!(!diagnostic.contains("private-path-sentinel"));
    assert!(!diagnostic.contains(location));
    assert!(!diagnostic.contains("private-key-bytes-must-not-escape"));
}

// 실제 Python fixture가 readiness marker를 쓰지 않고 살아 있는 동안 timeout 경로가 child를
// kill-wait-join한 뒤 stderr는 보존하고 private argv path는 숨기는지 고정합니다.
#[test]
fn reports_sanitized_stderr_for_a_readiness_timeout() {
    let location = "timeout-location-private-path-sentinel";
    let (child, _root, ready, started) = diagnostic_test_child(
        "import sys,time\nsys.stderr.write('listener readiness timeout: ' + ' '.join(sys.argv[1:]))\nsys.stderr.flush()\nwith open(sys.argv[6], 'w') as marker: marker.write('started\\n')\ntime.sleep(10)",
        location,
    );
    wait_for_test_marker(&started, "started\n", Duration::from_secs(2));
    let started_at = Instant::now();
    let failure = catch_unwind(AssertUnwindSafe(|| {
        let _ = wait_for_local_tls_ready(child, &ready, Duration::from_millis(100));
    }))
    .expect_err("a listener without a readiness marker must time out");
    let diagnostic = panic_message(failure);

    assert!(started_at.elapsed() < Duration::from_secs(2));
    assert!(diagnostic.contains("local TLS listener published an invalid port"));
    assert!(
        diagnostic.contains("child exited with a signal")
            || (diagnostic.contains("child exited with ")
                && !diagnostic.contains("child exited with 0"))
    );
    assert!(diagnostic.contains("listener readiness timeout"));
    assert!(!diagnostic.contains("private-path-sentinel"));
    assert!(!diagnostic.contains(location));
    assert!(!diagnostic.contains("private-key-bytes-must-not-escape"));
}

// known-sensitive 목록이 비어도 delimiter 없는 authorization 다중 token과 Bearer 및 PEM 본문을
// fail-closed redaction하는지 독립적으로 고정합니다.
#[test]
fn redacts_auth_and_private_key_content_without_known_sensitive_values() {
    let stderr = concat!(
        "authorization delimiter-free-token-a delimiter-free-token-b\n",
        "Authorization Bearer malformed-token-a malformed-token-b\n",
        "Authorization Basic basic-credential-material: rejected\n",
        "Authorization: Bearer delimited-token-a delimited-token-b\n",
        "Bearer standalone-token-a standalone-token-b\n",
        "-----BEGIN PRIVATE KEY-----\n",
        "private-key-bytes-must-not-escape\n",
        "-----END PRIVATE KEY-----\n",
    );
    let diagnostic = sanitize_stderr(stderr.as_bytes(), &[]);

    assert!(diagnostic.contains("authorization[redacted]"));
    assert!(!diagnostic.contains("delimiter-free-token-a"));
    assert!(!diagnostic.contains("delimiter-free-token-b"));
    assert!(!diagnostic.contains("malformed-token-a"));
    assert!(!diagnostic.contains("malformed-token-b"));
    assert!(!diagnostic.contains("basic-credential-material"));
    assert!(!diagnostic.contains("rejected"));
    assert!(!diagnostic.contains("delimited-token-a"));
    assert!(!diagnostic.contains("delimited-token-b"));
    assert!(!diagnostic.contains("standalone-token-a"));
    assert!(!diagnostic.contains("standalone-token-b"));
    assert!(!diagnostic.contains("private-key-bytes-must-not-escape"));
    assert!(diagnostic.contains("[redacted private key]"));
}

// Python이 escape하거나 표현할 수 없는 private argv path를 만나 ChildGuard가 stderr를 생략할
// 때 실제 early-exit failure를 관찰하여 non-UTF-8 path와 stderr sentinel이 진단에 섞이지 않게
// 합니다.
#[test]
fn omits_child_stderr_when_a_sensitive_path_is_not_safely_representable() {
    let invalid_path = {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;

            std::ffi::OsString::from_vec(vec![b'/', 0xff])
        }
        #[cfg(not(unix))]
        {
            std::ffi::OsString::from("path\nescaped")
        }
    };

    assert!(
        child_sensitive_values(&[invalid_path.as_os_str()], "", OsStr::new("unused")).is_none()
    );

    let child = Command::new("python3")
        .arg("-c")
        .arg("import sys; sys.stderr.write('unavailable-sensitive-stderr-sentinel'); sys.exit(23)")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python3 is required for the fail-closed stderr test");
    let mut child = ChildGuard::new(child, None);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if child.child.as_mut().unwrap().try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fail-closed test child did not exit"
        );
        thread::sleep(Duration::from_millis(1));
    }
    let failure = catch_unwind(AssertUnwindSafe(|| {
        child.assert_running("fail-closed child exited before readiness");
    }))
    .expect_err("the fail-closed child must report its early exit");
    let diagnostic = panic_message(failure);
    assert!(diagnostic.contains("fail-closed child exited before readiness"));
    assert!(!diagnostic.contains("unavailable-sensitive-stderr-sentinel"));
}

// NormalizedEndpoint는 HTTPS만 허용하고 production client에는 test root 주입구가 없으므로,
// child process에만 ephemeral root를 신뢰시키고 loopback TLS listener를 띄웁니다.
pub(super) fn run_in_tls_child(test_name: &str) -> bool {
    if env::var_os("YO_MODEL_CONNECTOR_TEST_CHILD").is_some() {
        let marker = env::var_os("YO_MODEL_CONNECTOR_TEST_MARKER")
            .expect("the local TLS child must provide its execution marker path");
        fs::write(marker, b"1\n").expect("the local TLS child must publish its execution marker");
        return false;
    }
    let root = TempDirectory::new("yo-model-connector-cert");
    let root_certificate = root.path().join("root.pem");
    let root_key = root.path().join("root-key.pem");
    let certificate = root.path().join("certificate.pem");
    let key = root.path().join("key.pem");
    let csr = root.path().join("certificate.csr");
    let extensions = root.path().join("extensions.cnf");
    let marker = root.path().join("executed");
    create_private_file(&marker);
    create_private_file(&extensions);
    fs::write(
        &extensions,
        "[v3_server]\nsubjectAltName=IP:127.0.0.1\nextendedKeyUsage=serverAuth\nbasicConstraints=critical,CA:FALSE\n",
    )
    .unwrap();
    openssl(&[
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        root_key.to_str().unwrap(),
        "-out",
        root_certificate.to_str().unwrap(),
        "-days",
        "3650",
        "-subj",
        "/CN=yo local test root",
        "-addext",
        "basicConstraints=critical,CA:TRUE",
        "-addext",
        "keyUsage=critical,keyCertSign,cRLSign",
    ]);
    openssl(&[
        "req",
        "-new",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        key.to_str().unwrap(),
        "-out",
        csr.to_str().unwrap(),
        "-subj",
        "/CN=127.0.0.1",
    ]);
    openssl(&[
        "x509",
        "-req",
        "-in",
        csr.to_str().unwrap(),
        "-CA",
        root_certificate.to_str().unwrap(),
        "-CAkey",
        root_key.to_str().unwrap(),
        "-CAcreateserial",
        "-out",
        certificate.to_str().unwrap(),
        "-days",
        "3650",
        "-extfile",
        extensions.to_str().unwrap(),
        "-extensions",
        "v3_server",
    ]);
    for path in [
        &root_certificate,
        &root_key,
        &certificate,
        &key,
        &csr,
        &extensions,
    ] {
        set_private_permissions(path);
    }
    let status = Command::new(env::current_exe().unwrap())
        .arg("--exact")
        .arg(test_name)
        .arg("--nocapture")
        .env("YO_MODEL_CONNECTOR_TEST_CHILD", "1")
        .env("YO_MODEL_CONNECTOR_TEST_CERT", &certificate)
        .env("YO_MODEL_CONNECTOR_TEST_KEY", &key)
        .env("YO_MODEL_CONNECTOR_TEST_MARKER", &marker)
        .env("SSL_CERT_FILE", &root_certificate)
        .env("SSL_CERT_DIR", "")
        .status()
        .expect("the local TLS characterization child must start");
    assert!(status.success(), "local TLS characterization child failed");
    assert_eq!(
        fs::read_to_string(marker).unwrap(),
        "1\n",
        "the exact child characterization test did not execute"
    );
    true
}

fn openssl(args: &[&str]) {
    let status = Command::new("openssl")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("openssl is required for the local TLS characterization fixture");
    assert!(
        status.success(),
        "openssl failed to create local TLS material"
    );
}

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(prefix: &str) -> Self {
        let path = unique_temp_dir(prefix);
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;

            builder.mode(0o700);
        }
        builder.create(&path).unwrap();
        set_directory_permissions(&path);
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

fn create_private_file(path: &Path) {
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600);
    }
    options.open(path).unwrap();
    set_private_permissions(path);
}

fn set_private_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions).unwrap();
    }
}

fn set_directory_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).unwrap();
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    env::temp_dir().join(format!("{prefix}-{}-{stamp}", std::process::id()))
}
