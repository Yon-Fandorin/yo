//! Git-backed isolated repository fixture and structured CLI assertions.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

pub(super) const KNOWLEDGE_ID: &str = "tui.relocated";

pub(super) struct GitRepository {
    pub(super) path: PathBuf,
}

impl GitRepository {
    pub(super) fn foundation() -> Self {
        Self::from_fixture("relocation-a")
    }

    pub(super) fn code_foundation() -> Self {
        Self::from_fixture("code-active")
    }

    pub(super) fn from_fixture(fixture: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "methexis-checkpoint-flow-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        copy_directory(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(fixture)
                .join("methexis"),
            &path.join("methexis"),
        );
        let repository = Self { path };
        repository.git(&[
            "init",
            "--initial-branch=develop",
            "--object-format=sha1",
            "--template=",
        ]);
        repository.git(&["config", "user.email", "fixture@example.invalid"]);
        repository.git(&["config", "user.name", "Methexis Fixture"]);
        repository.git(&["add", "methexis"]);
        repository.git(&["commit", "-m", "fixture foundation"]);
        repository.git(&["switch", "-c", "feature"]);
        repository
    }

    pub(super) fn approved() -> Self {
        let repository = Self::foundation();
        repository.integrate_approval();
        repository
    }

    pub(super) fn code_approved() -> Self {
        let repository = Self::code_foundation();
        repository.integrate_approval();
        repository
    }

    fn integrate_approval(&self) {
        self.approve_units(&[KNOWLEDGE_ID]);
        self.git(&["add", "methexis"]);
        self.git(&["commit", "-m", "fixture approval"]);
        self.git(&["branch", "-f", "develop", "HEAD"]);
    }

    pub(super) fn integrate_active_checkpoint(&self) {
        self.integrate_active_checkpoint_roots(&[KNOWLEDGE_ID]);
    }

    pub(super) fn integrate_active_checkpoint_roots(&self, roots: &[&str]) {
        let create_request = self.request(
            "checkpoint.json",
            &json!({
                "schema": "methexis.checkpoint-request/v1alpha1",
                "roots": roots
            }),
        );
        let created =
            success_json(self.run(&["create-checkpoint", create_request.to_str().unwrap()]));
        let activation_request = self.request(
            "activation.json",
            &json!({
                "schema": "methexis.activation-request/v1alpha1",
                "checkpoint_id": created["checkpoint_id"],
                "checkpoint_hash": created["hash"]
            }),
        );
        success_json(self.run(&["propose-activation", activation_request.to_str().unwrap()]));
        self.git(&[
            "add",
            "methexis/checkpoints",
            "methexis/active-checkpoint.yaml",
        ]);
        self.git(&["commit", "-m", "activate fixture checkpoint"]);
        self.git(&["branch", "-f", "develop", "HEAD"]);
    }

    pub(super) fn approve_units(&self, ids: &[&str]) {
        for id in ids {
            self.approve_unit(id);
        }
    }

    fn approve_unit(&self, id: &str) {
        let revision = self.revision_for(id);
        let projection_request = self.request(
            &format!("projection-{id}.json"),
            &json!({
                "schema": "methexis.review-projection-request/v1alpha1",
                "knowledge_id": id,
                "expected_revision": revision,
                "korean_markdown": "물리적 위치는 의미적 정체성이 아닙니다."
            }),
        );
        let projection =
            success_json(self.run(&["project-review", projection_request.to_str().unwrap()]));
        let approval_request = self.request(
            &format!("approval-{id}.json"),
            &json!({
                "schema": "methexis.approval-request/v1alpha1",
                "knowledge_id": id,
                "expected_revision": revision,
                "projection_hash": projection["hash"],
                "reviewer": "tui-architecture",
                "reviewed_at": "2026-07-24T12:00:00Z"
            }),
        );
        success_json(self.run(&["approve", approval_request.to_str().unwrap()]));
    }

    pub(super) fn request(&self, name: &str, value: &Value) -> PathBuf {
        let directory = self.path.join(".local-exclude/methexis/requests");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(name);
        let mut bytes = serde_json::to_vec(value).unwrap();
        bytes.push(b'\n');
        fs::write(&path, bytes).unwrap();
        path
    }

    pub(super) fn revision_for(&self, id: &str) -> String {
        success_json(self.run(&["check"]))["units"]
            .as_array()
            .unwrap()
            .iter()
            .find(|unit| unit["id"] == id)
            .unwrap()["revision"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    pub(super) fn run(&self, args: &[&str]) -> Output {
        self.run_with_env(args, &[])
    }

    pub(super) fn run_with_env(&self, args: &[&str], environment: &[(&str, &Path)]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_methexis"));
        command.current_dir(&self.path).args(args);
        for (key, value) in environment {
            command.env(key, value);
        }
        command.output().unwrap()
    }

    pub(super) fn git(&self, args: &[&str]) -> Output {
        let output = Command::new("/usr/bin/git")
            .env_clear()
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_GRAFT_FILE", "/dev/null")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("LC_ALL", "C")
            .arg("--no-replace-objects")
            .current_dir(&self.path)
            .env("GIT_AUTHOR_DATE", "2026-07-24T12:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-07-24T12:00:00Z")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

impl Drop for GitRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(super) fn checkpoint_request() -> Value {
    json!({
        "schema": "methexis.checkpoint-request/v1alpha1",
        "roots": [KNOWLEDGE_ID]
    })
}

pub(super) fn success_json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

pub(super) fn failure_json(output: Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(output.stdout.is_empty());
    serde_json::from_slice(&output.stderr).unwrap()
}

fn copy_directory(source: &Path, target: &Path) {
    fs::create_dir(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_directory(&source_path, &target_path);
        } else {
            fs::copy(source_path, target_path).unwrap();
        }
    }
}
