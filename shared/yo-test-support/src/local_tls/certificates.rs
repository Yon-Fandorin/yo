use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Stdio},
};

use super::child::{
    ChildGuard, TempDirectory, child_sensitive_values, create_private_file, set_private_permissions,
};

// Apple은 새로 발급하는 TLS 서버 인증서의 유효기간을 825일로 제한하므로, fixture
// 인증서는 그보다 짧게 유지하고 root가 server leaf보다 먼저 만료되지 않게 합니다.
const LOCAL_TLS_ROOT_VALIDITY_DAYS: &str = "398";
const LOCAL_TLS_SERVER_VALIDITY_DAYS: &str = "397";
#[cfg(test)]
const FIXTURE_CERTIFICATE_MAX_VALIDITY_SECONDS: &str = "71280000";

struct LocalTlsMaterial {
    root: TempDirectory,
    root_certificate: PathBuf,
    root_key: PathBuf,
    certificate: PathBuf,
    key: PathBuf,
    csr: PathBuf,
    extensions: PathBuf,
}

impl LocalTlsMaterial {
    fn generate() -> Self {
        let root = TempDirectory::new("yo-model-connector-cert");
        let root_certificate = root.path().join("root.pem");
        let root_key = root.path().join("root-key.pem");
        let certificate = root.path().join("certificate.pem");
        let key = root.path().join("key.pem");
        let csr = root.path().join("certificate.csr");
        let extensions = root.path().join("extensions.cnf");
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
            LOCAL_TLS_ROOT_VALIDITY_DAYS,
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
            LOCAL_TLS_SERVER_VALIDITY_DAYS,
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

        Self {
            root,
            root_certificate,
            root_key,
            certificate,
            key,
            csr,
            extensions,
        }
    }

    #[cfg(test)]
    fn assert_server_auth_and_conservative_validity(&self) {
        openssl(&[
            "verify",
            "-purpose",
            "sslserver",
            "-CAfile",
            self.root_certificate.to_str().unwrap(),
            self.certificate.to_str().unwrap(),
        ]);
        for (certificate, role) in [
            (&self.root_certificate, "root"),
            (&self.certificate, "server"),
        ] {
            openssl(&[
                "x509",
                "-in",
                certificate.to_str().unwrap(),
                "-noout",
                "-checkend",
                "0",
            ]);
            let exceeds_fixture_horizon = openssl_succeeds(&[
                "x509",
                "-in",
                certificate.to_str().unwrap(),
                "-noout",
                "-checkend",
                FIXTURE_CERTIFICATE_MAX_VALIDITY_SECONDS,
            ]);
            assert!(
                !exceeds_fixture_horizon,
                "generated local TLS {role} certificate remains valid beyond the fixture's conservative 825-day horizon"
            );
        }
    }
}

// 실제 child가 사용하는 인증서가 serverAuth 체인으로 검증되고 Apple의 825일 TLS
// 유효기간 상한 안에서 만료됨을 관찰해, root 주입만으로 fixture 회귀가 통과하지 못하게 합니다.
#[test]
fn generates_current_server_auth_material_that_expires_within_apple_limit() {
    LocalTlsMaterial::generate().assert_server_auth_and_conservative_validity();
}

/// OS별 platform verifier의 SSL_CERT_FILE 해석에 의존하지 않고, child process의 test-only
/// client에만 ephemeral root를 명시적으로 더해 HTTPS loopback listener를 띄웁니다.
pub fn run_in_tls_child(test_name: &str) -> bool {
    if env::var_os("YO_MODEL_CONNECTOR_TEST_CHILD").is_some() {
        let marker = env::var_os("YO_MODEL_CONNECTOR_TEST_MARKER")
            .expect("the local TLS child must provide its execution marker path");
        fs::write(marker, b"1\n").expect("the local TLS child must publish its execution marker");
        return false;
    }
    let material = LocalTlsMaterial::generate();
    let marker = material.root.path().join("executed");
    create_private_file(&marker);
    let child = Command::new(env::current_exe().unwrap())
        .arg("--exact")
        .arg(test_name)
        .arg("--nocapture")
        .env("YO_MODEL_CONNECTOR_TEST_CHILD", "1")
        .env("YO_MODEL_CONNECTOR_TEST_ROOT", &material.root_certificate)
        .env("YO_MODEL_CONNECTOR_TEST_CERT", &material.certificate)
        .env("YO_MODEL_CONNECTOR_TEST_KEY", &material.key)
        .env("YO_MODEL_CONNECTOR_TEST_MARKER", &marker)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the local TLS characterization child must start");
    let sensitive_values = child_sensitive_values(
        &[
            material.root.path().as_os_str(),
            material.root_certificate.as_os_str(),
            material.root_key.as_os_str(),
            material.certificate.as_os_str(),
            material.key.as_os_str(),
            material.csr.as_os_str(),
            material.extensions.as_os_str(),
            marker.as_os_str(),
        ],
        "",
        material.key.as_os_str(),
    );
    let mut child = ChildGuard::new(child, sensitive_values);
    child.assert_success("local TLS characterization child failed");
    assert_eq!(
        fs::read_to_string(marker).unwrap(),
        "1\n",
        "the exact child characterization test did not execute"
    );
    true
}

fn openssl(args: &[&str]) {
    assert!(
        openssl_succeeds(args),
        "openssl failed to create or validate local TLS material"
    );
}

fn openssl_succeeds(args: &[&str]) -> bool {
    let status = Command::new("openssl")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("openssl is required for the local TLS characterization fixture");
    status.success()
}
