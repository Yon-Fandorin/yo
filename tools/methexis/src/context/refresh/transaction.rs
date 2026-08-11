//! Bounded journal publication and recovery for registered ContextBuild manifests.

use std::{io, path::Path};

use serde::{Deserialize, Serialize};

use super::{super::registry, Prepared, RefreshFailure, failure, io_failure, publication_failure};
use crate::publication::{self, PublicationError};

pub(super) const MAX_JOURNAL_BYTES: usize = 1024 * 1024;
pub(super) const JOURNAL_PATH: &str =
    "tools/methexis/examples/context-contract/.manifest-refresh-transaction.json";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BatchJournal {
    pub(super) schema: String,
    pub(super) state: BatchState,
    pub(super) entries: Vec<BatchEntry>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BatchState {
    Prepared,
    Committed,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BatchEntry {
    pub(super) path: String,
    pub(super) old: Vec<u8>,
    pub(super) new: Vec<u8>,
}

pub(super) fn publish_batch(
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

pub(super) struct SequenceFailure<E> {
    pub(super) index: usize,
    pub(super) write: E,
    pub(super) rollback: Option<E>,
}

pub(super) fn publish_sequence<T, E>(
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

pub(super) fn recover_batch(repository_root: &Path) -> Result<(), RefreshFailure> {
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
            .capture(super::inputs::MAX_REGISTERED_BYTES)
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

pub(super) fn transaction_reader_guard(
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

pub(super) fn validate_journal(journal: &BatchJournal) -> Result<(), RefreshFailure> {
    let expected = registry::manifest_paths().collect::<Vec<_>>();
    let actual = journal
        .entries
        .iter()
        .map(|item| item.path.as_str())
        .collect::<Vec<_>>();
    if journal.schema != "methexis.context-manifest-refresh-transaction/v1alpha1"
        || actual != expected
        || journal.entries.iter().any(|item| {
            item.old.len() > super::inputs::MAX_REGISTERED_BYTES
                || item.new.len() > super::inputs::MAX_REGISTERED_BYTES
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

pub(super) fn journal_bytes(journal: &BatchJournal) -> Result<Vec<u8>, RefreshFailure> {
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

pub(super) fn run_guarded_publication<E, T>(
    prospective: impl FnOnce() -> Result<(), E>,
    compiled: impl FnOnce() -> Result<(), E>,
    publish: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    prospective()?;
    compiled()?;
    publish()
}
