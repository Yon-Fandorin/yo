//! Closed, prospective refresh of tracked ContextBuild manifest goldens.

use std::{io, path::Path};

use serde::{Deserialize, Serialize};

use super::{
    operations::{self, CompiledBuild},
    registry::{self, ContextManifestRegistration},
};
use crate::{
    checkpoint,
    publication::{self, CapturedFile, PublicationError, TargetLock},
};

const OPERATION: &str = "refresh_context_manifests";
const MAX_REGISTERED_BYTES: usize = 256 * 1024;
const MAX_JOURNAL_BYTES: usize = 1024 * 1024;
const JOURNAL_PATH: &str =
    "tools/methexis/examples/context-contract/.manifest-refresh-transaction.json";

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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BatchJournal {
    schema: String,
    state: BatchState,
    entries: Vec<BatchEntry>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum BatchState {
    Prepared,
    Committed,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BatchEntry {
    path: String,
    old: Vec<u8>,
    new: Vec<u8>,
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
        .map_err(|error| publication_failure(error, JOURNAL_PATH))?;
    recover_batch(repository_root)?;
    let prospective = checkpoint::prepare_context_refresh(repository_root, activation_request)
        .map_err(checkpoint_failure)?;
    let mut prepared = Vec::with_capacity(registry::REGISTRATIONS.len());
    for registration in registry::REGISTRATIONS {
        let (request_lock, request) = capture_registered(repository_root, registration.request)?;
        let (context_lock, context) = capture_registered(repository_root, registration.context)?;
        let (manifest_lock, manifest) = capture_registered(repository_root, registration.manifest)?;
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
            .capture(MAX_REGISTERED_BYTES)
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
        if let Err(error) = revalidate_registered_inputs(&item.request, &item.context) {
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
    let statuses = run_guarded_publication(
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
        || publish_batch(repository_root, &prepared),
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

fn publish_batch(
    repository_root: &Path,
    prepared: &[Prepared],
) -> Result<Vec<&'static str>, RefreshFailure> {
    let journal_lock =
        publication::lock_target(repository_root, &repository_root.join(JOURNAL_PATH))
            .map_err(|error| publication_failure(error, JOURNAL_PATH))?;
    match journal_lock.capture(MAX_JOURNAL_BYTES) {
        Ok(_) => {
            return Err(failure(
                None,
                "batch_recovery_required",
                "a manifest refresh transaction appeared during preparation",
                Vec::new(),
                vec![JOURNAL_PATH.to_owned()],
                "retry so Methexis can recover the transaction",
            ));
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {},
        Err(error) => return Err(io_failure(error, JOURNAL_PATH)),
    }
    let entries = prepared
        .iter()
        .map(|item| BatchEntry {
            path: item.registration.manifest.to_owned(),
            old: item.manifest.bytes().to_vec(),
            new: item.compiled.artifacts.manifest.clone(),
        })
        .collect::<Vec<_>>();
    let mut journal = BatchJournal {
        schema: "methexis.context-manifest-refresh-transaction/v1alpha1".to_owned(),
        state: BatchState::Prepared,
        entries,
    };
    let bytes = journal_bytes(&journal)?;
    journal_lock
        .atomic_write(&bytes)
        .map_err(|error| publication_failure(error, JOURNAL_PATH))?;

    if let Err(error) = publish_sequence(
        prepared,
        |item| item.manifest.bytes() != item.compiled.artifacts.manifest,
        |item| {
            item.manifest_lock
                .atomic_write(&item.compiled.artifacts.manifest)
        },
        |item| item.manifest_lock.atomic_write(item.manifest.bytes()),
        PublicationError::namespace_may_be_committed,
    ) {
        if error.rollback.is_some() {
            return Err(failure(
                None,
                "batch_recovery_required",
                format!(
                    "late publication failed and rollback did not complete: {:?}",
                    error.write
                ),
                Vec::new(),
                registry::manifest_paths().map(str::to_owned).collect(),
                "rerun refresh to recover the durable transaction",
            ));
        }
        journal_lock
            .remove()
            .map_err(|remove| publication_failure(remove, JOURNAL_PATH))?;
        return Err(publication_failure(
            error.write,
            prepared[error.index].registration.manifest,
        ));
    }
    journal.state = BatchState::Committed;
    journal_lock
        .atomic_write(&journal_bytes(&journal)?)
        .map_err(|error| publication_failure(error, JOURNAL_PATH))?;
    journal_lock
        .remove()
        .map_err(|error| publication_failure(error, JOURNAL_PATH))?;
    Ok(prepared
        .iter()
        .map(|item| {
            if item.manifest.bytes() == item.compiled.artifacts.manifest {
                "unchanged"
            } else {
                "written"
            }
        })
        .collect())
}

fn revalidate_registered_inputs(request: &CapturedFile, context: &CapturedFile) -> io::Result<()> {
    request.revalidate().and_then(|()| context.revalidate())
}

fn run_guarded_publication<E, T>(
    prospective: impl FnOnce() -> Result<(), E>,
    compiled: impl FnOnce() -> Result<(), E>,
    publish: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    prospective()?;
    compiled()?;
    publish()
}

struct SequenceFailure<E> {
    index: usize,
    write: E,
    rollback: Option<E>,
}

fn publish_sequence<T, E>(
    items: &[T],
    mut changed: impl FnMut(&T) -> bool,
    mut write: impl FnMut(&T) -> Result<(), E>,
    mut rollback: impl FnMut(&T) -> Result<(), E>,
    mut committed_on_error: impl FnMut(&E) -> bool,
) -> Result<(), SequenceFailure<E>> {
    let mut written = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if !changed(item) {
            continue;
        }
        if let Err(error) = write(item) {
            if committed_on_error(&error) {
                written.push(item);
            }
            let mut rollback_error = None;
            for previous in written.into_iter().rev() {
                if let Err(error) = rollback(previous) {
                    rollback_error = Some(error);
                    break;
                }
            }
            return Err(SequenceFailure {
                index,
                write: error,
                rollback: rollback_error,
            });
        }
        written.push(item);
    }
    Ok(())
}

fn recover_batch(repository_root: &Path) -> Result<(), RefreshFailure> {
    let journal_path = repository_root.join(JOURNAL_PATH);
    let journal_lock = publication::lock_target(repository_root, &journal_path)
        .map_err(|error| publication_failure(error, JOURNAL_PATH))?;
    let capture = match journal_lock.capture(MAX_JOURNAL_BYTES) {
        Ok(capture) => capture,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_failure(error, JOURNAL_PATH)),
    };
    let journal: BatchJournal = serde_json::from_slice(capture.bytes()).map_err(|error| {
        failure(
            None,
            "batch_recovery_conflict",
            error.to_string(),
            Vec::new(),
            vec![JOURNAL_PATH.to_owned()],
            "inspect the malformed transaction without overwriting tracked manifests",
        )
    })?;
    validate_journal(&journal)?;
    let desired_old = matches!(journal.state, BatchState::Prepared);
    let mut locks = Vec::new();
    for entry in &journal.entries {
        let lock = publication::lock_target(repository_root, &repository_root.join(&entry.path))
            .map_err(|error| publication_failure(error, &entry.path))?;
        let current = lock
            .capture(MAX_REGISTERED_BYTES)
            .map_err(|error| io_failure(error, &entry.path))?;
        if current.bytes() != entry.old && current.bytes() != entry.new {
            return Err(failure(
                None,
                "batch_recovery_conflict",
                "tracked manifest matches neither the journal old nor new bytes",
                Vec::new(),
                vec![entry.path.clone()],
                "inspect the ambiguous manifest and transaction before retrying",
            ));
        }
        locks.push((lock, current));
    }
    for ((lock, current), entry) in locks.iter().zip(&journal.entries) {
        let desired = if desired_old { &entry.old } else { &entry.new };
        if current.bytes() != desired {
            lock.atomic_write(desired)
                .map_err(|error| publication_failure(error, &entry.path))?;
        }
    }
    journal_lock
        .remove()
        .map_err(|error| publication_failure(error, JOURNAL_PATH))
}

pub(crate) fn transaction_reader_guard(
    repository_root: &Path,
) -> Result<publication::RepositoryGuard, String> {
    let guard = publication::lock_repository_shared(repository_root)
        .map_err(|error| format!("cannot lock manifest refresh transaction: {error:?}"))?;
    match publication::capture_file(
        repository_root,
        &repository_root.join(JOURNAL_PATH),
        MAX_JOURNAL_BYTES,
    ) {
        Ok(_) => Err("a manifest refresh transaction is pending recovery".to_owned()),
        Err(PublicationError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(guard),
        Err(error) => Err(format!(
            "cannot safely inspect manifest refresh transaction: {error:?}"
        )),
    }
}

fn validate_journal(journal: &BatchJournal) -> Result<(), RefreshFailure> {
    let expected = registry::manifest_paths().collect::<Vec<_>>();
    let actual = journal
        .entries
        .iter()
        .map(|item| item.path.as_str())
        .collect::<Vec<_>>();
    if journal.schema != "methexis.context-manifest-refresh-transaction/v1alpha1"
        || actual != expected
        || journal.entries.iter().any(|item| {
            item.old.len() > MAX_REGISTERED_BYTES || item.new.len() > MAX_REGISTERED_BYTES
        })
    {
        return Err(failure(
            None,
            "batch_recovery_conflict",
            "manifest refresh transaction schema, paths, or sizes are invalid",
            Vec::new(),
            vec![JOURNAL_PATH.to_owned()],
            "inspect the transaction without overwriting tracked manifests",
        ));
    }
    Ok(())
}

fn journal_bytes(journal: &BatchJournal) -> Result<Vec<u8>, RefreshFailure> {
    let mut bytes = serde_json::to_vec(journal).map_err(|error| {
        failure(
            None,
            "manifest_publication_failed",
            error.to_string(),
            Vec::new(),
            vec![JOURNAL_PATH.to_owned()],
            "report the transaction serialization failure",
        )
    })?;
    bytes.push(b'\n');
    if bytes.len() > MAX_JOURNAL_BYTES {
        return Err(failure(
            None,
            "manifest_publication_failed",
            "manifest refresh transaction exceeds its size limit",
            Vec::new(),
            vec![JOURNAL_PATH.to_owned()],
            "reduce the registered manifest set",
        ));
    }
    Ok(bytes)
}

fn capture_registered(
    repository_root: &Path,
    relative: &str,
) -> Result<(TargetLock, CapturedFile), RefreshFailure> {
    let lock = publication::lock_target(repository_root, &repository_root.join(relative))
        .map_err(|error| publication_failure(error, relative))?;
    let capture = lock
        .capture(MAX_REGISTERED_BYTES)
        .map_err(|error| io_failure(error, relative))?;
    Ok((lock, capture))
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
        let failure = super::publish_sequence(
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
        let failure = super::publish_sequence(
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

        assert!(super::revalidate_registered_inputs(&request, &context).is_err());
        fs::remove_dir_all(root).unwrap();
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
        let failure = super::run_guarded_publication(
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

    #[cfg(unix)]
    #[ignore]
    // 기본 parent test가 실행해 PREPARED journal과 첫 manifest 교체 직후 강제 종료한다.
    #[test]
    fn child_is_killed_during_a_prepared_transaction() {
        use std::{fs, path::PathBuf, thread, time::Duration};

        let root = PathBuf::from(std::env::var_os("METHEXIS_CRASH_TEST_ROOT").unwrap());
        let ready = PathBuf::from(std::env::var_os("METHEXIS_CRASH_TEST_READY").unwrap());
        let paths = super::registry::manifest_paths().collect::<Vec<_>>();
        let journal = super::BatchJournal {
            schema: "methexis.context-manifest-refresh-transaction/v1alpha1".to_owned(),
            state: super::BatchState::Prepared,
            entries: paths
                .iter()
                .map(|path| super::BatchEntry {
                    path: (*path).to_owned(),
                    old: fs::read(root.join(path)).unwrap(),
                    new: format!("new:{path}\n").into_bytes(),
                })
                .collect(),
        };
        let journal_lock =
            super::publication::lock_target(&root, &root.join(super::JOURNAL_PATH)).unwrap();
        journal_lock
            .atomic_write(&super::journal_bytes(&journal).unwrap())
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

        super::recover_batch(&root).unwrap();

        for path in super::registry::manifest_paths() {
            assert_eq!(
                fs::read(root.join(path)).unwrap(),
                format!("old:{path}\n").as_bytes()
            );
        }
        assert!(!root.join(super::JOURNAL_PATH).exists());
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
