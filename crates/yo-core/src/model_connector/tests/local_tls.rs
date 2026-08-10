use std::{
    env,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

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
        let child = Command::new("python3")
            .arg("-c")
            .arg(include_str!("local_tls_server.py"))
            .arg(&certificate)
            .arg(&key)
            .arg(&ready)
            .arg(&requests)
            .arg(&accepted)
            .arg(&sent)
            .arg(mode)
            .arg(&closed)
            .arg(&payload)
            .arg(content_type)
            .arg(status.to_string())
            .arg(location)
            .arg(max_connections.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("python3 is required for the local TLS listener fixture");
        let mut child = ChildGuard::new(child);
        let deadline = Instant::now() + Duration::from_secs(2);
        let port = loop {
            child.assert_running("local TLS listener exited before publishing its port");
            if let Ok(value) = fs::read_to_string(&ready)
                && let Ok(port) = value.trim().parse::<u16>()
            {
                break port;
            }
            assert!(
                Instant::now() < deadline,
                "local TLS listener published an invalid port"
            );
            thread::sleep(Duration::from_millis(1));
        };
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

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn assert_running(&mut self, message: &str) {
        assert!(
            self.0.as_mut().unwrap().try_wait().unwrap().is_none(),
            "{message}"
        );
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
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
