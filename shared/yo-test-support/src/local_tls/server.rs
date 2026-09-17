use std::{
    env,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use super::{
    child::{
        ChildGuard, LocalTlsChildSpec, TempDirectory, create_private_file, spawn_local_tls_child,
        wait_for_local_tls_ready,
    },
    modes::LocalServerMode,
};

/// canonical Python listener를 구동하고 request marker를 보관하는 bounded HTTPS fixture입니다.
pub struct LocalTlsServer {
    _child: ChildGuard,
    _root: TempDirectory,
    requests: PathBuf,
    accepted: PathBuf,
    sent: PathBuf,
    closed: PathBuf,
    endpoint: String,
}

impl LocalTlsServer {
    /// 지정한 mode로 loopback HTTPS listener를 시작합니다.
    pub fn start(mode: LocalServerMode) -> Self {
        let certificate = env::var_os("YO_MODEL_CONNECTOR_TEST_CERT")
            .expect("the local TLS child must provide its test certificate path");
        let key = env::var_os("YO_MODEL_CONNECTOR_TEST_KEY")
            .expect("the local TLS child must provide its test key path");
        let mode = mode.into_child_config();
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
        fs::write(&payload, &mode.payload).unwrap();

        let child = spawn_local_tls_child(LocalTlsChildSpec {
            script: include_str!("../local_tls_server.py"),
            certificate: certificate.as_os_str(),
            key: key.as_os_str(),
            ready: &ready,
            requests: &requests,
            accepted: &accepted,
            sent: &sent,
            mode: mode.wire_mode,
            closed: &closed,
            payload: &payload,
            content_type: &mode.content_type,
            status: mode.status,
            location: &mode.location,
            max_connections: mode.max_connections,
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

    /// listener endpoint의 normalized base URL을 반환합니다.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// listener가 response 경계까지 진행할 때까지 기다립니다.
    pub fn wait_for_response_sent(&self) {
        self.wait_for_marker(
            &self.sent,
            "local TLS listener did not report its response boundary",
        );
    }

    /// listener가 connector peer의 close를 관찰할 때까지 기다립니다.
    pub fn wait_for_peer_closed(&self) {
        self.wait_for_marker(
            &self.closed,
            "local TLS listener did not observe the connector closing its peer",
        );
    }

    /// listener가 accept한 TCP connection 수를 반환합니다.
    pub fn accepted_connections(&self) -> usize {
        self.marker_count(&self.accepted)
    }

    /// listener가 기록한 request marker JSON을 읽습니다.
    pub fn requests(&self) -> Vec<serde_json::Value> {
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
