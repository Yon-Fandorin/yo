//! Closed, prospective refresh of tracked ContextBuild manifest goldens.

use std::{io, path::Path};

use serde::Serialize;

use super::{
    operations::{self, CompiledBuild},
    registry::{self, ContextManifestRegistration},
};
use crate::{
    checkpoint,
    publication::{self, CapturedFile, PublicationError, TargetLock},
};

mod inputs;
mod transaction;

const OPERATION: &str = "refresh_context_manifests";

struct Prepared {
    registration: &'static ContextManifestRegistration,
    _request_lock: TargetLock,
    request: CapturedFile,
    _context_lock: TargetLock,
    context: CapturedFile,
    manifest_lock: TargetLock,
    manifest: CapturedFile,
    compiled: CompiledBuild,
}

#[derive(Debug, Serialize)]
pub(crate) struct RefreshSuccess {
    schema: &'static str,
    pub(crate) ok: bool,
    operation: &'static str,
    status: &'static str,
    authority: &'static str,
    trusted_commit: String,
    checkpoint_id: String,
    checkpoint_hash: String,
    manifests: Vec<ManifestResult>,
    affected_ids: Vec<String>,
    next_actions: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct ManifestResult {
    path: &'static str,
    status: &'static str,
    build_id: String,
    hash: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct RefreshFailure {
    schema: &'static str,
    pub(crate) ok: bool,
    operation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    trusted_commit: Option<String>,
    error: Box<RefreshError>,
}

#[derive(Debug, Serialize)]
struct RefreshError {
    code: String,
    message: String,
    affected_ids: Vec<String>,
    affected_paths: Vec<String>,
    next_actions: Vec<String>,
}

pub(super) fn run(
    repository_root: &Path,
    activation_request: &Path,
) -> Result<RefreshSuccess, RefreshFailure> {
    let _transaction_guard = publication::lock_repository_exclusive(repository_root)
        .map_err(|error| publication_failure(error, transaction::JOURNAL_PATH))?;
    transaction::recover_batch(repository_root)?;
    let prospective = checkpoint::prepare_context_refresh(repository_root, activation_request)
        .map_err(checkpoint_failure)?;
    let mut prepared = Vec::with_capacity(registry::REGISTRATIONS.len());
    for registration in registry::REGISTRATIONS {
        let (request_lock, request) =
            inputs::capture_registered(repository_root, registration.request)?;
        let (context_lock, context) =
            inputs::capture_registered(repository_root, registration.context)?;
        let (manifest_lock, manifest) =
            inputs::capture_registered(repository_root, registration.manifest)?;
        let compiled = operations::compile_captured(
            repository_root,
            request.bytes(),
            &repository_root.join(registration.request),
            &prospective.authority,
        )
        .map_err(resolve_failure)?;
        if compiled.artifacts.context != context.bytes() {
            return Err(failure(
                Some(prospective.authority.trusted_commit.clone()),
                "context_payload_changed",
                "prospective authority changes a registered context.md payload",
                compiled.artifacts.included_ids.clone(),
                vec![registration.context.to_owned()],
                "review the payload change in a separate Slice instead of refreshing its manifest",
            ));
        }
        prepared.push(Prepared {
            registration,
            _request_lock: request_lock,
            request,
            _context_lock: context_lock,
            context,
            manifest_lock,
            manifest,
            compiled,
        });
    }

    for item in &prepared {
        let current = item
            .manifest_lock
            .capture(inputs::MAX_REGISTERED_BYTES)
            .map_err(|error| io_failure(error, item.registration.manifest))?;
        if current.bytes() != item.manifest.bytes() {
            return Err(failure(
                Some(prospective.authority.trusted_commit.clone()),
                "manifest_changed_during_refresh",
                "registered manifest changed after it was captured",
                Vec::new(),
                vec![item.registration.manifest.to_owned()],
                "retry after concurrent edits stop",
            ));
        }
        if let Err(error) = inputs::revalidate_registered_inputs(&item.request, &item.context) {
            return Err(failure(
                Some(prospective.authority.trusted_commit.clone()),
                "registered_input_changed_during_refresh",
                format!("registered request or context golden changed during refresh: {error}"),
                Vec::new(),
                vec![
                    item.registration.request.to_owned(),
                    item.registration.context.to_owned(),
                ],
                "retry after concurrent edits stop",
            ));
        }
    }
    let statuses = transaction::run_guarded_publication(
        || {
            prospective
                .final_revalidate(repository_root)
                .map_err(checkpoint_failure)
        },
        || {
            for item in &prepared {
                item.compiled
                    .final_revalidate(repository_root, &prospective.authority.trusted_commit)
                    .map_err(resolve_failure)?;
            }
            Ok(())
        },
        || transaction::publish_batch(repository_root, &prepared),
    )?;
    let mut results = Vec::with_capacity(prepared.len());
    for (item, status) in prepared.iter().zip(statuses) {
        results.push(ManifestResult {
            path: item.registration.manifest,
            status,
            build_id: item.compiled.artifacts.build_id.clone(),
            hash: item.compiled.artifacts.manifest_hash.clone(),
        });
    }
    let status = if results.iter().all(|item| item.status == "unchanged") {
        "unchanged"
    } else {
        "written"
    };
    let affected_ids = prospective.authority.active.iter().cloned().collect();
    Ok(RefreshSuccess {
        schema: "methexis.context-manifest-refresh/v1alpha1",
        ok: true,
        operation: OPERATION,
        status,
        authority: "prospective",
        trusted_commit: prospective.authority.trusted_commit.clone(),
        checkpoint_id: prospective.authority.checkpoint_id.clone(),
        checkpoint_hash: prospective.authority.checkpoint_hash.clone(),
        manifests: results,
        affected_ids,
        next_actions: vec![
            "stage the Checkpoint, active record, and every refreshed manifest, then run `methexis check --staged-activation`",
        ],
    })
}

pub(crate) fn transaction_reader_guard(
    repository_root: &Path,
) -> Result<publication::RepositoryGuard, String> {
    transaction::transaction_reader_guard(repository_root)
}

fn checkpoint_failure(error: checkpoint::OperationFailure) -> RefreshFailure {
    let (commit, code, message, ids) = error.parts();
    failure(
        commit,
        code,
        message,
        ids,
        Vec::new(),
        "repair the activation proposal and retry",
    )
}

fn resolve_failure(error: super::ResolveFailure) -> RefreshFailure {
    let (commit, code, message, ids, paths) = error.parts();
    failure(
        commit,
        code,
        message,
        ids,
        paths,
        "repair the registered Context Resolution request",
    )
}

fn publication_failure(error: PublicationError, path: &str) -> RefreshFailure {
    let (code, message) = match error {
        PublicationError::OutsideRepository => (
            "registered_path_invalid",
            "registered path escapes the repository".to_owned(),
        ),
        PublicationError::Symlink(path) => (
            "registered_path_symlink",
            format!("registered path uses symlink `{}`", path.display()),
        ),
        PublicationError::NotDirectory(path) => (
            "registered_path_not_directory",
            format!("registered parent is not a directory `{}`", path.display()),
        ),
        PublicationError::Locked(error) => ("manifest_refresh_locked", error.to_string()),
        PublicationError::Io(error) | PublicationError::DurabilityUnknown(error) => {
            ("manifest_publication_failed", error.to_string())
        },
    };
    failure(
        None,
        code,
        message,
        Vec::new(),
        vec![path.to_owned()],
        "repair the registered path or retry after the active writer finishes",
    )
}

fn io_failure(error: io::Error, path: &str) -> RefreshFailure {
    failure(
        None,
        "registered_input_unreadable",
        error.to_string(),
        Vec::new(),
        vec![path.to_owned()],
        "repair the registered context contract",
    )
}

fn failure(
    commit: Option<String>,
    code: impl Into<String>,
    message: impl Into<String>,
    affected_ids: Vec<String>,
    affected_paths: Vec<String>,
    next_action: impl Into<String>,
) -> RefreshFailure {
    RefreshFailure {
        schema: "methexis.context-manifest-refresh-failure/v1alpha1",
        ok: false,
        operation: OPERATION,
        trusted_commit: commit,
        error: Box::new(RefreshError {
            code: code.into(),
            message: message.into(),
            affected_ids,
            affected_paths,
            next_actions: vec![next_action.into()],
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    // filesystem 환경변수 hook 없이 publisher seam이 두 번째 write 실패 뒤 첫 항목을 되돌린다.
    #[test]
    fn publication_seam_rolls_back_a_late_failure() {
        let state = RefCell::new(vec![0, 0]);
        let failure = super::transaction::publish_sequence(
            &[0_usize, 1],
            |_| true,
            |index| {
                if *index == 1 {
                    return Err("late write");
                }
                state.borrow_mut()[*index] = 1;
                Ok(())
            },
            |index| {
                state.borrow_mut()[*index] = 0;
                Ok(())
            },
            |_| false,
        )
        .unwrap_err();

        assert_eq!(failure.write, "late write");
        assert!(failure.rollback.is_none());
        assert_eq!(*state.borrow(), vec![0, 0]);
    }

    // namespace commit 뒤 durability 오류는 현재 항목까지 rollback 대상에 포함한다.
    #[test]
    fn publication_seam_rolls_back_the_current_ambiguous_write() {
        let state = RefCell::new(vec![0, 0]);
        let failure = super::transaction::publish_sequence(
            &[0_usize, 1],
            |_| true,
            |index| {
                state.borrow_mut()[*index] = 1;
                if *index == 1 {
                    Err("durability unknown")
                } else {
                    Ok(())
                }
            },
            |index| {
                state.borrow_mut()[*index] = 0;
                Ok(())
            },
            |error| *error == "durability unknown",
        )
        .unwrap_err();

        assert_eq!(failure.write, "durability unknown");
        assert_eq!(*state.borrow(), vec![0, 0]);
    }

    // 여러 manifest를 순서대로 쓴 뒤 실패하면 이미 쓴 항목을 역순으로 되돌린다.
    #[test]
    fn publication_seam_rolls_back_in_reverse_order() {
        let state = RefCell::new(vec![0, 0, 0]);
        let write_order = RefCell::new(Vec::new());
        let rollback_order = RefCell::new(Vec::new());
        let failure = super::transaction::publish_sequence(
            &[0_usize, 1, 2],
            |_| true,
            |index| {
                write_order.borrow_mut().push(*index);
                if *index == 2 {
                    return Err("late write");
                }
                state.borrow_mut()[*index] = 1;
                Ok(())
            },
            |index| {
                rollback_order.borrow_mut().push(*index);
                state.borrow_mut()[*index] = 0;
                Ok(())
            },
            |_| false,
        )
        .unwrap_err();

        assert_eq!(failure.write, "late write");
        assert!(failure.rollback.is_none());
        assert_eq!(*write_order.borrow(), vec![0, 1, 2]);
        assert_eq!(*rollback_order.borrow(), vec![1, 0]);
        assert_eq!(*state.borrow(), vec![0, 0, 0]);
    }

    // rollback 자체가 실패하면 이미 namespace에 반영된 상태를 숨기지 않고 recovery를 요구한다.
    #[test]
    fn publication_seam_preserves_a_rollback_failure() {
        let state = RefCell::new(vec![0, 0]);
        let failure = super::transaction::publish_sequence(
            &[0_usize, 1],
            |_| true,
            |index| {
                if *index == 1 {
                    return Err("late write");
                }
                state.borrow_mut()[*index] = 1;
                Ok(())
            },
            |index| {
                if *index == 0 {
                    return Err("rollback failed");
                }
                state.borrow_mut()[*index] = 0;
                Ok(())
            },
            |_| false,
        )
        .unwrap_err();

        assert_eq!(failure.write, "late write");
        assert_eq!(failure.rollback, Some("rollback failed"));
        assert_eq!(*state.borrow(), vec![1, 0]);
    }

    // publication은 prospective guard, compiled-input guard, 실제 write 순서를 고정한다.
    #[test]
    fn guarded_publication_runs_guards_before_publication() {
        let order = RefCell::new(Vec::new());
        let result = super::transaction::run_guarded_publication(
            || {
                order.borrow_mut().push("prospective");
                Ok::<(), &'static str>(())
            },
            || {
                order.borrow_mut().push("compiled");
                Ok(())
            },
            || {
                order.borrow_mut().push("publish");
                Ok::<_, &'static str>("published")
            },
        )
        .unwrap();

        assert_eq!(result, "published");
        assert_eq!(*order.borrow(), vec!["prospective", "compiled", "publish"]);
    }

    // compiled-input guard가 실패하면 publication callback은 실행되지 않는다.
    #[test]
    fn guarded_publication_stops_before_publication_on_compiled_guard_failure() {
        let order = RefCell::new(Vec::new());
        let failure = super::transaction::run_guarded_publication(
            || {
                order.borrow_mut().push("prospective");
                Ok::<(), &'static str>(())
            },
            || {
                order.borrow_mut().push("compiled");
                Err::<(), _>("compiled changed")
            },
            || {
                order.borrow_mut().push("publish");
                Ok::<_, &'static str>(())
            },
        )
        .unwrap_err();

        assert_eq!(failure, "compiled changed");
        assert_eq!(*order.borrow(), vec!["prospective", "compiled"]);
    }

    // refresh 전용 capture seam은 동일 bytes request의 inode 교체도 최종 검증에서 거부한다.
    #[test]
    fn registered_request_identity_change_is_rejected_without_environment_hooks() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "methexis-refresh-capture-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let request_path = root.join("request.json");
        let context_path = root.join("context.md");
        fs::write(&request_path, b"{}\n").unwrap();
        fs::write(&context_path, b"context\n").unwrap();
        let request = super::publication::capture_file(&root, &request_path, 32).unwrap();
        let context = super::publication::capture_file(&root, &context_path, 32).unwrap();
        fs::write(root.join("replacement"), b"{}\n").unwrap();
        fs::rename(root.join("replacement"), &request_path).unwrap();

        assert!(super::inputs::revalidate_registered_inputs(&request, &context).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    // context.md가 같은 경로에서 다른 bytes로 바뀌면 capture identity 검증이 즉시 거부한다.
    #[test]
    fn registered_context_bytes_change_is_rejected_without_environment_hooks() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "methexis-refresh-context-bytes-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let request_path = root.join("request.json");
        let context_path = root.join("context.md");
        fs::write(&request_path, b"{}\n").unwrap();
        fs::write(&context_path, b"context\n").unwrap();
        let request = super::publication::capture_file(&root, &request_path, 32).unwrap();
        let context = super::publication::capture_file(&root, &context_path, 32).unwrap();
        fs::write(&context_path, b"changed context\n").unwrap();

        assert!(super::inputs::revalidate_registered_inputs(&request, &context).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    // manifest의 현재 bytes가 capture 이후 달라지면 publication 직전 guard가 거부한다.
    #[test]
    fn captured_manifest_bytes_change_is_rejected_before_publication() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "methexis-refresh-manifest-bytes-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let manifest_path = root.join("manifest.json");
        fs::write(&manifest_path, b"old manifest\n").unwrap();
        let lock = super::publication::lock_target(&root, &manifest_path).unwrap();
        let capture = lock.capture(64).unwrap();
        fs::write(&manifest_path, b"new manifest\n").unwrap();

        assert!(capture.revalidate().is_err());
        fs::remove_dir_all(root).unwrap();
    }

    // journal schema, 등록 순서, entry별 크기 bound는 모두 한 번에 검증한다.
    #[test]
    fn journal_validation_rejects_schema_paths_and_entry_sizes() {
        let registrations = super::registry::REGISTRATIONS;
        let mut journal = super::transaction::BatchJournal {
            schema: "methexis.context-manifest-refresh-transaction/v1alpha1".to_owned(),
            state: super::transaction::BatchState::Prepared,
            entries: registrations
                .iter()
                .map(|registration| super::transaction::BatchEntry {
                    path: registration.manifest.to_owned(),
                    old: b"old\n".to_vec(),
                    new: b"new\n".to_vec(),
                })
                .collect(),
        };

        assert!(super::transaction::validate_journal(&journal).is_ok());
        journal.schema = "wrong-schema".to_owned();
        assert_eq!(
            serde_json::to_value(super::transaction::validate_journal(&journal).unwrap_err())
                .unwrap()["error"]["code"],
            "batch_recovery_conflict"
        );

        journal.schema = "methexis.context-manifest-refresh-transaction/v1alpha1".to_owned();
        journal.entries.swap(0, 1);
        assert_eq!(
            serde_json::to_value(super::transaction::validate_journal(&journal).unwrap_err())
                .unwrap()["error"]["affected_paths"],
            serde_json::json!([super::transaction::JOURNAL_PATH])
        );

        journal.entries.swap(0, 1);
        journal.entries[0].new = vec![b'x'; super::inputs::MAX_REGISTERED_BYTES + 1];
        assert_eq!(
            serde_json::to_value(super::transaction::validate_journal(&journal).unwrap_err())
                .unwrap()["error"]["code"],
            "batch_recovery_conflict"
        );
    }

    // 두 registered manifest가 함께 직렬화될 때 journal 자체도 전체 크기 bound를 지킨다.
    #[test]
    fn journal_serialization_rejects_an_oversized_batch() {
        let journal = super::transaction::BatchJournal {
            schema: "methexis.context-manifest-refresh-transaction/v1alpha1".to_owned(),
            state: super::transaction::BatchState::Prepared,
            entries: super::registry::REGISTRATIONS
                .iter()
                .map(|registration| super::transaction::BatchEntry {
                    path: registration.manifest.to_owned(),
                    old: vec![b'o'; super::inputs::MAX_REGISTERED_BYTES],
                    new: vec![b'n'; super::inputs::MAX_REGISTERED_BYTES],
                })
                .collect(),
        };

        let failure = super::transaction::journal_bytes(&journal).unwrap_err();
        let failure = serde_json::to_value(failure).unwrap();
        assert_eq!(failure["error"]["code"], "manifest_publication_failed");
        assert_eq!(
            failure["error"]["affected_paths"],
            serde_json::json!([super::transaction::JOURNAL_PATH])
        );
    }

    // 실제 prospective owner guard는 Source identity 변경을 고유 오류로 반환하고 publication을
    // 막는다.
    #[test]
    fn actual_source_guard_stops_refresh_before_publication() {
        let (repository, _request, prospective) = prospective_fixture();
        let source = repository
            .path
            .join("methexis/sources/decision/tui.fixture.yaml");
        let replacement = source.with_extension("replacement");
        std::fs::write(&replacement, std::fs::read(&source).unwrap()).unwrap();
        std::fs::rename(replacement, source).unwrap();

        assert_actual_guard_failure(
            &repository,
            &prospective,
            "source_changed_during_validation",
        );
    }

    // 실제 activation request capture identity 변경은 proposal 고유 오류로 publication을 막는다.
    #[test]
    fn actual_proposal_guard_stops_refresh_before_publication() {
        let (repository, request, prospective) = prospective_fixture();
        let replacement = request.with_extension("replacement");
        std::fs::write(&replacement, std::fs::read(&request).unwrap()).unwrap();
        std::fs::rename(replacement, request).unwrap();

        assert_actual_guard_failure(
            &repository,
            &prospective,
            "activation_proposal_changed_during_refresh",
        );
    }

    // 실제 develop ref advance는 authority owner 오류로 publication을 막고 pinned snapshot을 바꾸지
    // 않는다.
    #[test]
    fn actual_ref_guard_stops_refresh_before_publication() {
        let (repository, _request, prospective) = prospective_fixture();
        std::fs::write(repository.path.join("advance.txt"), b"advance\n").unwrap();
        repository.git(&["add", "advance.txt"]);
        repository.git(&["commit", "-m", "advance trusted ref"]);
        repository.git(&["branch", "-f", "develop", "HEAD"]);

        assert_actual_guard_failure(
            &repository,
            &prospective,
            "authority_changed_during_validation",
        );
    }

    fn prospective_fixture() -> (
        crate::checkpoint::TestRepository,
        std::path::PathBuf,
        crate::checkpoint::ProspectiveContext,
    ) {
        use serde_json::json;

        let repository = crate::checkpoint::TestRepository::new();
        repository.approve(&["tui.context.base", "tui.context.large"]);
        let checkpoints = crate::checkpoint::CheckpointService::new(&repository.path);
        let create = repository.request(
            "checkpoint.json",
            &json!({
                "schema": "methexis.checkpoint-request/v1alpha1",
                "roots": ["tui.context.large"]
            }),
        );
        let created = checkpoints.create(&create).unwrap();
        let created = serde_json::to_value(created).unwrap();
        let request = repository.request(
            "activation.json",
            &json!({
                "schema": "methexis.activation-request/v1alpha1",
                "checkpoint_id": created["checkpoint_id"],
                "checkpoint_hash": created["hash"]
            }),
        );
        checkpoints.propose_activation(&request).unwrap();
        let prospective =
            crate::checkpoint::prepare_context_refresh(&repository.path, &request).unwrap();
        (repository, request, prospective)
    }

    fn assert_actual_guard_failure(
        repository: &crate::checkpoint::TestRepository,
        prospective: &crate::checkpoint::ProspectiveContext,
        expected: &str,
    ) {
        let publication_ran = RefCell::new(false);
        let failure = super::transaction::run_guarded_publication(
            || prospective.final_revalidate(&repository.path),
            || Ok(()),
            || {
                *publication_ran.borrow_mut() = true;
                Ok::<(), crate::checkpoint::OperationFailure>(())
            },
        )
        .unwrap_err();
        assert_eq!(failure.parts().1, expected);
        assert!(!*publication_ran.borrow());
    }

    // PREPARED recovery는 혼합 상태를 old bytes로, COMMITTED recovery는 new bytes로 수렴시킨다.
    #[test]
    fn recovery_converges_prepared_and_committed_batches() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let prepared_root = std::env::temp_dir().join(format!(
            "methexis-refresh-prepared-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&prepared_root).unwrap();
        let prepared_entries = super::registry::manifest_paths()
            .map(|path| super::transaction::BatchEntry {
                path: path.to_owned(),
                old: format!("old:{path}\n").into_bytes(),
                new: format!("new:{path}\n").into_bytes(),
            })
            .collect::<Vec<_>>();
        for entry in &prepared_entries {
            let target = prepared_root.join(&entry.path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(&target, &entry.old).unwrap();
        }
        let prepared_journal = super::transaction::BatchJournal {
            schema: "methexis.context-manifest-refresh-transaction/v1alpha1".to_owned(),
            state: super::transaction::BatchState::Prepared,
            entries: prepared_entries,
        };
        let journal_path = prepared_root.join(super::transaction::JOURNAL_PATH);
        fs::create_dir_all(journal_path.parent().unwrap()).unwrap();
        fs::write(
            &journal_path,
            super::transaction::journal_bytes(&prepared_journal).unwrap(),
        )
        .unwrap();
        let first = &prepared_journal.entries[0];
        fs::write(prepared_root.join(&first.path), &first.new).unwrap();

        super::transaction::recover_batch(&prepared_root).unwrap();

        for entry in &prepared_journal.entries {
            assert_eq!(
                fs::read(prepared_root.join(&entry.path)).unwrap(),
                entry.old
            );
        }
        assert!(!journal_path.exists());
        fs::remove_dir_all(&prepared_root).unwrap();

        let committed_root = std::env::temp_dir().join(format!(
            "methexis-refresh-committed-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&committed_root).unwrap();
        let committed_entries = super::registry::manifest_paths()
            .map(|path| super::transaction::BatchEntry {
                path: path.to_owned(),
                old: format!("old:{path}\n").into_bytes(),
                new: format!("new:{path}\n").into_bytes(),
            })
            .collect::<Vec<_>>();
        for entry in &committed_entries {
            let target = committed_root.join(&entry.path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(&target, &entry.old).unwrap();
        }
        let committed_journal = super::transaction::BatchJournal {
            schema: "methexis.context-manifest-refresh-transaction/v1alpha1".to_owned(),
            state: super::transaction::BatchState::Committed,
            entries: committed_entries,
        };
        let journal_path = committed_root.join(super::transaction::JOURNAL_PATH);
        fs::create_dir_all(journal_path.parent().unwrap()).unwrap();
        fs::write(
            &journal_path,
            super::transaction::journal_bytes(&committed_journal).unwrap(),
        )
        .unwrap();
        let first = &committed_journal.entries[0];
        fs::write(committed_root.join(&first.path), &first.new).unwrap();

        super::transaction::recover_batch(&committed_root).unwrap();

        for entry in &committed_journal.entries {
            assert_eq!(
                fs::read(committed_root.join(&entry.path)).unwrap(),
                entry.new
            );
        }
        assert!(!journal_path.exists());
        fs::remove_dir_all(&committed_root).unwrap();
    }

    // journal과 old/new 어느 쪽에도 없는 manifest는 보존하고 recovery를 중단한다.
    #[test]
    fn recovery_retains_conflicting_manifest_and_journal() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "methexis-refresh-conflict-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let entries = super::registry::manifest_paths()
            .map(|path| super::transaction::BatchEntry {
                path: path.to_owned(),
                old: format!("old:{path}\n").into_bytes(),
                new: format!("new:{path}\n").into_bytes(),
            })
            .collect::<Vec<_>>();
        for entry in &entries {
            let target = root.join(&entry.path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(&target, &entry.old).unwrap();
        }
        let conflict_path = root.join(&entries[0].path);
        fs::write(&conflict_path, b"unrecognized\n").unwrap();
        let journal = super::transaction::BatchJournal {
            schema: "methexis.context-manifest-refresh-transaction/v1alpha1".to_owned(),
            state: super::transaction::BatchState::Prepared,
            entries,
        };
        let journal_path = root.join(super::transaction::JOURNAL_PATH);
        fs::create_dir_all(journal_path.parent().unwrap()).unwrap();
        fs::write(
            &journal_path,
            super::transaction::journal_bytes(&journal).unwrap(),
        )
        .unwrap();

        let failure = super::transaction::recover_batch(&root).unwrap_err();
        let failure = serde_json::to_value(failure).unwrap();
        assert_eq!(failure["error"]["code"], "batch_recovery_conflict");
        assert_eq!(
            failure["error"]["affected_paths"],
            serde_json::json!([journal.entries[0].path])
        );
        assert_eq!(fs::read(&conflict_path).unwrap(), b"unrecognized\n");
        assert!(journal_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[ignore]
    // 기본 parent test가 실행해 PREPARED journal과 첫 manifest 교체 직후 강제 종료한다.
    #[test]
    fn child_is_killed_during_a_prepared_transaction() {
        use std::{fs, path::PathBuf, thread, time::Duration};

        let root = PathBuf::from(std::env::var_os("METHEXIS_CRASH_TEST_ROOT").unwrap());
        let ready = PathBuf::from(std::env::var_os("METHEXIS_CRASH_TEST_READY").unwrap());
        let paths = super::registry::manifest_paths().collect::<Vec<_>>();
        let journal = super::transaction::BatchJournal {
            schema: "methexis.context-manifest-refresh-transaction/v1alpha1".to_owned(),
            state: super::transaction::BatchState::Prepared,
            entries: paths
                .iter()
                .map(|path| super::transaction::BatchEntry {
                    path: (*path).to_owned(),
                    old: fs::read(root.join(path)).unwrap(),
                    new: format!("new:{path}\n").into_bytes(),
                })
                .collect(),
        };
        let journal_lock =
            super::publication::lock_target(&root, &root.join(super::transaction::JOURNAL_PATH))
                .unwrap();
        journal_lock
            .atomic_write(&super::transaction::journal_bytes(&journal).unwrap())
            .unwrap();
        let first = &journal.entries[0];
        let manifest_lock =
            super::publication::lock_target(&root, &root.join(&first.path)).unwrap();
        manifest_lock.atomic_write(&first.new).unwrap();
        fs::write(ready, b"prepared\n").unwrap();
        loop {
            thread::park_timeout(Duration::from_secs(30));
        }
    }

    #[cfg(unix)]
    // 실제 child SIGKILL 뒤 kernel lock을 회수하고 PREPARED mixed batch를 old/old로 복구한다.
    #[test]
    fn killed_prepared_transaction_recovers_to_old_batch() {
        use std::{
            fs,
            process::Command,
            thread,
            time::{Duration, SystemTime, UNIX_EPOCH},
        };

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "methexis-crash-recovery-{}-{unique}",
            std::process::id()
        ));
        for path in super::registry::manifest_paths() {
            let target = root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, format!("old:{path}\n")).unwrap();
        }
        let ready = root.join("ready");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "context::refresh::tests::child_is_killed_during_a_prepared_transaction",
                "--ignored",
                "--nocapture",
            ])
            .env("METHEXIS_CRASH_TEST_ROOT", &root)
            .env("METHEXIS_CRASH_TEST_READY", &ready)
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if ready.exists() {
                break;
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!("crash helper exited early: {status}");
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(ready.exists(), "crash helper did not become ready");
        child.kill().unwrap();
        child.wait().unwrap();

        super::transaction::recover_batch(&root).unwrap();

        for path in super::registry::manifest_paths() {
            assert_eq!(
                fs::read(root.join(path)).unwrap(),
                format!("old:{path}\n").as_bytes()
            );
        }
        assert!(!root.join(super::transaction::JOURNAL_PATH).exists());
        fs::remove_dir_all(root).unwrap();
    }

    // read-only transaction guard는 repository directory 자체를 shared-lock하며 lock 파일을 만들지
    // 않는다.
    #[test]
    fn reader_guard_does_not_create_workspace_bytes() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "methexis-reader-guard-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();

        let guard = super::transaction_reader_guard(&root).unwrap();

        assert!(!root.join(".local-exclude").exists());
        drop(guard);
        fs::remove_dir(root).unwrap();
    }
}
