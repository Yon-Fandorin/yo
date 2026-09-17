use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

use super::child::{TempDirectory, create_private_file, set_private_permissions};

// Apple은 새로 발급하는 TLS 서버 인증서의 유효기간을 825일로 제한하므로, fixture
// 인증서는 그보다 짧게 유지하고 root가 server leaf보다 먼저 만료되지 않게 합니다.
const LOCAL_TLS_ROOT_VALIDITY_DAYS: &str = "398";
const LOCAL_TLS_SERVER_VALIDITY_DAYS: &str = "397";
#[cfg(test)]
const FIXTURE_CERTIFICATE_MAX_VALIDITY_SECONDS: &str = "71280000";

pub(super) struct LocalTlsMaterial {
    pub(super) root: TempDirectory,
    pub(super) root_certificate: PathBuf,
    pub(super) root_key: PathBuf,
    pub(super) certificate: PathBuf,
    pub(super) key: PathBuf,
    pub(super) csr: PathBuf,
    pub(super) extensions: PathBuf,
}

impl LocalTlsMaterial {
    pub(super) fn generate() -> Self {
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
