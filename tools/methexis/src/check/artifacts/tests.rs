use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::checkpoint::ActiveCheckpoint;

struct TemporaryRepository(PathBuf);

impl TemporaryRepository {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "methexis-artifact-check-{}-{unique}",
            std::process::id()
        ));
        for relative in crate::context::registry::manifest_paths() {
            fs::create_dir_all(
                root.join(relative)
                    .parent()
                    .expect("tracked artifact has a parent"),
            )
            .expect("create tracked artifact directory");
        }
        Self(root)
    }

    fn root(&self) -> &Path {
        &self.0
    }

    fn write_manifests(&self, id: &str, hash: &str, commit: &str) {
        let manifest = serde_json::json!({
            "plan": {
                "checkpoint": {
                    "id": id,
                    "hash": hash,
                    "authority_basis_commit": commit,
                }
            }
        });
        for relative in crate::context::registry::manifest_paths() {
            fs::write(
                self.0.join(relative),
                serde_json::to_vec(&manifest).expect("encode manifest"),
            )
            .expect("write tracked artifact");
        }
    }
}

impl Drop for TemporaryRepository {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove temporary repository");
    }
}

fn active_checkpoint() -> ActiveCheckpoint {
    ActiveCheckpoint {
        id: "sha256:active-id".to_owned(),
        hash: "sha256:active-hash".to_owned(),
        active_record_hash: "sha256:active-record".to_owned(),
        authority_basis_commit: "active-commit".to_owned(),
    }
}

// 두 추적 manifest가 현재 활성 Checkpoint provenance와 정확히 같으면 통과한다.
#[test]
fn matching_tracked_artifacts_pass() {
    let repository = TemporaryRepository::new();
    let active = active_checkpoint();
    repository.write_manifests(&active.id, &active.hash, &active.authority_basis_commit);

    assert!(super::validate(repository.root(), &active).is_empty());
}

// 활성화가 바뀐 뒤 예전 provenance가 남은 manifest를 stale로 찾아 과거 누락을 회귀 방지한다.
#[test]
fn stale_checkpoint_provenance_is_reported_for_each_manifest() {
    let repository = TemporaryRepository::new();
    repository.write_manifests("sha256:old-id", "sha256:old-hash", "old-commit");

    let diagnostics = super::validate(repository.root(), &active_checkpoint());

    assert_eq!(diagnostics.len(), 2);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "stale_tracked_artifact")
    );
}

#[cfg(unix)]
// 등록된 manifest가 symlink면 외부 파일을 따라 읽지 않고 안전한 읽기 실패로 거부한다.
#[test]
fn symlinked_tracked_artifact_is_rejected() {
    use std::os::unix::fs::symlink;

    let repository = TemporaryRepository::new();
    let active = active_checkpoint();
    repository.write_manifests(&active.id, &active.hash, &active.authority_basis_commit);
    let target = repository.root().join("outside.json");
    fs::write(&target, b"{}").expect("write symlink target");
    let artifact = repository
        .root()
        .join(crate::context::registry::REGISTRATIONS[0].manifest);
    fs::remove_file(&artifact).expect("remove regular artifact");
    symlink(&target, &artifact).expect("create artifact symlink");

    assert!(super::is_registered(repository.root()));
    let diagnostics = super::validate(repository.root(), &active);

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "tracked_artifact_unreadable");
}

// 비정상적으로 큰 tracked manifest는 제한된 양만 읽고 구조화된 실패로 보고한다.
#[test]
fn oversized_tracked_artifact_is_bounded() {
    let repository = TemporaryRepository::new();
    let active = active_checkpoint();
    repository.write_manifests(&active.id, &active.hash, &active.authority_basis_commit);
    fs::write(
        repository
            .root()
            .join(crate::context::registry::REGISTRATIONS[0].manifest),
        vec![b' '; super::MAX_ARTIFACT_BYTES + 1],
    )
    .expect("write oversized artifact");

    let diagnostics = super::validate(repository.root(), &active);

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "tracked_artifact_unreadable");
}

// closed set 중 하나라도 등록되면 나머지 manifest 누락도 통과시키지 않고 함께 보고한다.
#[test]
fn partial_registration_requires_every_tracked_artifact() {
    let repository = TemporaryRepository::new();
    let active = active_checkpoint();
    repository.write_manifests(&active.id, &active.hash, &active.authority_basis_commit);
    fs::remove_file(
        repository
            .root()
            .join(crate::context::registry::REGISTRATIONS[1].manifest),
    )
    .expect("remove one registered artifact");

    assert!(super::is_registered(repository.root()));
    let diagnostics = super::validate(repository.root(), &active);

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "tracked_artifact_unreadable");
}
