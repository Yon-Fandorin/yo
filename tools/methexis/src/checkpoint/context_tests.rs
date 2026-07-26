use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;

use super::{
    CheckpointService, final_revalidate_context_authority, hash_bytes, resolve_context_authority,
};
use crate::{check::load_foundation, review::ReviewService};

#[test]
fn final_guard_rejects_a_concurrent_trusted_ref_advance_without_switching_snapshot() {
    let repository = Repository::new();
    repository.approve(&["tui.context.base", "tui.context.large"]);
    let checkpoint = CheckpointService::new(&repository.path);
    let create = repository.request(
        "checkpoint.json",
        &json!({
            "schema": "methexis.checkpoint-request/v1alpha1",
            "roots": ["tui.context.large"]
        }),
    );
    let created = checkpoint.create(&create).unwrap();
    let activate = repository.request(
        "activation.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": created.checkpoint_id,
            "checkpoint_hash": created.hash
        }),
    );
    checkpoint.propose_activation(&activate).unwrap();
    repository.commit_authority("activate checkpoint");

    let authority = resolve_context_authority(&repository.path).unwrap();
    let original = authority.trusted_commit.clone();
    fs::write(repository.path.join("unrelated.txt"), "concurrent\n").unwrap();
    repository.git(&["add", "unrelated.txt"]);
    repository.git(&["commit", "-m", "concurrent authority advance"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);

    let failure = final_revalidate_context_authority(&repository.path, &authority).unwrap_err();

    assert!(failure.retryable);
    assert_eq!(failure.trusted_commit.as_deref(), Some(original.as_str()));
    assert_eq!(
        failure.diagnostics[0].code,
        "authority_changed_during_resolution"
    );

    repository.git(&["branch", "-D", "develop"]);
    let missing = final_revalidate_context_authority(&repository.path, &authority).unwrap_err();
    assert!(missing.retryable);
    assert_eq!(
        missing.diagnostics[0].code,
        "authority_changed_during_resolution"
    );
}

struct Repository {
    path: PathBuf,
}

impl Repository {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "methexis-context-authority-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        copy_directory(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/context-active/methexis"),
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
        repository.git(&["commit", "-m", "foundation"]);
        repository.git(&["switch", "-c", "feature"]);
        repository
    }

    fn approve(&self, ids: &[&str]) {
        let foundation = load_foundation(&self.path).unwrap();
        let reviews = ReviewService::new(&self.path);
        for id in ids {
            let revision = &foundation
                .units
                .iter()
                .find(|unit| unit.metadata.id == *id)
                .unwrap()
                .revision;
            let projection = self.request(
                &format!("projection-{id}.json"),
                &json!({
                    "schema": "methexis.review-projection-request/v1alpha1",
                    "knowledge_id": id,
                    "expected_revision": revision,
                    "korean_markdown": "검토된 컨텍스트 지식입니다."
                }),
            );
            reviews.generate_projection(&projection).unwrap();
            let projection_path = self
                .path
                .join("methexis/review-projections")
                .join(format!("{id}.md"));
            let approval = self.request(
                &format!("approval-{id}.json"),
                &json!({
                    "schema": "methexis.approval-request/v1alpha1",
                    "knowledge_id": id,
                    "expected_revision": revision,
                    "projection_hash": hash_bytes(&fs::read(projection_path).unwrap()),
                    "reviewer": "tui-architecture",
                    "reviewed_at": "2026-07-24T12:00:00Z"
                }),
            );
            reviews.record_approval(&approval).unwrap();
        }
        self.commit_authority("approve context");
    }

    fn commit_authority(&self, message: &str) {
        self.git(&["add", "methexis"]);
        self.git(&["commit", "-m", message]);
        self.git(&["branch", "-f", "develop", "HEAD"]);
    }

    fn request(&self, name: &str, value: &serde_json::Value) -> PathBuf {
        let directory = self.path.join(".local-exclude/methexis/requests");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(name);
        let mut bytes = serde_json::to_vec(value).unwrap();
        bytes.push(b'\n');
        fs::write(&path, bytes).unwrap();
        path
    }

    fn git(&self, args: &[&str]) {
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
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_directory(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
