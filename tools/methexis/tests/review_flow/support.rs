//! Shared isolated-repository fixture and structured CLI assertions.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

pub(super) const KNOWLEDGE_ID: &str = "tui.relocated";

pub(super) struct TempRepository {
    pub(super) path: PathBuf,
}

impl TempRepository {
    pub(super) fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "methexis-review-flow-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temporary repository");
        copy_directory(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/relocation-a/methexis"),
            &path.join("methexis"),
        );
        Self { path }
    }

    pub(super) fn request(&self, name: &str, value: &Value) -> PathBuf {
        let directory = self.path.join(".local-exclude/methexis/requests");
        fs::create_dir_all(&directory).expect("create request directory");
        let path = directory.join(name);
        let mut bytes = serde_json::to_vec(value).expect("serialize request");
        bytes.push(b'\n');
        fs::write(&path, bytes).expect("write request");
        path
    }

    pub(super) fn check(&self) -> Value {
        success_json(run(&self.path, &["check"]))
    }

    pub(super) fn revision(&self) -> String {
        self.check()["units"][0]["revision"]
            .as_str()
            .expect("unit revision")
            .to_owned()
    }
}

impl Drop for TempRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(super) fn projection_request(revision: &str, korean_markdown: &str) -> Value {
    json!({
        "schema": "methexis.review-projection-request/v1alpha1",
        "knowledge_id": KNOWLEDGE_ID,
        "expected_revision": revision,
        "korean_markdown": korean_markdown,
    })
}

pub(super) fn approval_request(
    revision: &str,
    projection_hash: &str,
    reviewer: &str,
    reviewed_at: &str,
) -> Value {
    json!({
        "schema": "methexis.approval-request/v1alpha1",
        "knowledge_id": KNOWLEDGE_ID,
        "expected_revision": revision,
        "projection_hash": projection_hash,
        "reviewer": reviewer,
        "reviewed_at": reviewed_at,
    })
}

pub(super) fn has_diagnostic(result: &Value, code: &str) -> bool {
    result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == code)
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

pub(super) fn run(repository: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_methexis"))
        .current_dir(repository)
        .args(args)
        .output()
        .expect("run methexis")
}

pub(super) fn success_json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).expect("success JSON")
}

pub(super) fn failure_json(output: Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(output.stdout.is_empty());
    serde_json::from_slice(&output.stderr).expect("failure JSON")
}

fn copy_directory(source: &Path, target: &Path) {
    fs::create_dir(target).expect("create fixture directory");
    for entry in fs::read_dir(source).expect("read fixture directory") {
        let entry = entry.expect("fixture entry");
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type().expect("fixture type").is_dir() {
            copy_directory(&source_path, &target_path);
        } else {
            fs::copy(source_path, target_path).expect("copy fixture file");
        }
    }
}
