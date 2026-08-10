//! Shared isolated-repository fixture and structured CLI assertions.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

pub(super) const KNOWLEDGE_ID: &str = "tui.grapheme-cells";
pub(super) const MULTI_SOURCE_ID: &str = "tui.multi-sourced";

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
            "methexis-author-flow-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temporary repository");
        copy_directory(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/author-revision/methexis"),
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
        success_json(run(&self.path, &["check", "--only", "records,relations"]))
    }

    /// The unit's current derived revision, read from the tracked Projection
    /// the fixture carries for it.
    pub(super) fn revision(&self) -> String {
        let projection = fs::read_to_string(
            self.path
                .join("methexis/review-projections/tui.grapheme-cells.md"),
        )
        .expect("read fixture projection");
        projection
            .lines()
            .find_map(|line| line.strip_prefix("revision: "))
            .expect("projection revision")
            .to_owned()
    }
}

impl Drop for TempRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(super) fn author_request(revision: &str) -> Value {
    json!({
        "schema": "methexis.author-revision-request/v1alpha1",
        "knowledge_id": KNOWLEDGE_ID,
        "expected_revision": revision,
        "source_content": "Cells are allocated per measured grapheme cluster.",
        "knowledge_body": "# Grapheme cell storage\n\n## Statement\n\nTerminal cells store exactly one measured grapheme cluster each.\n\n## Rationale\n\nSplitting clusters across cells corrupts cursor accounting.\n",
        "korean_markdown": "터미널 셀은 각각 측정된 하나의 자소 클러스터를 저장합니다.",
    })
}

pub(super) fn semantic_author_request(revision: &str) -> Value {
    json!({
        "schema": "methexis.author-revision-request/v1alpha2",
        "knowledge_id": KNOWLEDGE_ID,
        "expected_revision": revision,
        "source_content": "Cells are allocated per measured grapheme cluster.",
        "knowledge_body": "# Grapheme cell storage\n\n## Statement\n\nTerminal cells store exactly one measured grapheme cluster each.\n\n## Rationale\n\nSplitting clusters across cells corrupts cursor accounting.\n",
    })
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
