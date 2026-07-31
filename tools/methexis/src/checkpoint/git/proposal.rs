//! Immutable, read-only capture of the caller-selected proposal index.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::Path,
};

use super::{git_output, git_output_with_index, safe_relative, valid_commit};
use crate::checkpoint::{MAX_RECORD_BYTES, OperationFailure};

#[derive(Eq, PartialEq)]
struct BlobIdentity {
    mode: Vec<u8>,
    oid: String,
}

type IndexEntries = BTreeMap<(Vec<u8>, u8), BlobIdentity>;
type TreeEntries = BTreeMap<Vec<u8>, BlobIdentity>;

pub(in crate::checkpoint) struct StagedEntry {
    pub(in crate::checkpoint) status: char,
    pub(in crate::checkpoint) path: Vec<u8>,
}

pub(in crate::checkpoint) struct ProposalIndex {
    file: Option<OsString>,
    head: String,
    identity: Vec<u8>,
    entries: IndexEntries,
}

pub(in crate::checkpoint) fn capture_index(
    repository_root: &Path,
    operation: &'static str,
) -> Result<ProposalIndex, OperationFailure> {
    capture_index_from(
        repository_root,
        std::env::var_os("GIT_INDEX_FILE"),
        operation,
    )
}

pub(in crate::checkpoint) fn capture_index_from(
    repository_root: &Path,
    file: Option<OsString>,
    operation: &'static str,
) -> Result<ProposalIndex, OperationFailure> {
    let identity = git_output_with_index(
        repository_root,
        &["ls-files", "--stage", "-z"],
        file.as_deref(),
        operation,
        None,
        "staged_candidate_unreadable",
    )?;
    let entries = parse_index_entries(&identity, operation)?;
    let head = git_output(
        repository_root,
        &["rev-parse", "--verify", "HEAD^{commit}"],
        operation,
        None,
        "staged_candidate_unreadable",
    )?;
    let head = String::from_utf8(head)
        .map_err(|error| {
            OperationFailure::new(
                operation,
                None,
                "invalid_git_output",
                error.to_string(),
                Vec::new(),
                "repair the Git index and retry",
            )
        })?
        .trim()
        .to_owned();
    if !valid_commit(&head) {
        return Err(OperationFailure::new(
            operation,
            None,
            "invalid_git_output",
            "proposal parent did not resolve to a hexadecimal commit ID",
            Vec::new(),
            "repair HEAD and retry",
        ));
    }
    Ok(ProposalIndex {
        file,
        head,
        identity,
        entries,
    })
}

pub(in crate::checkpoint) fn staged_entries(
    repository_root: &Path,
    index: &ProposalIndex,
    operation: &'static str,
) -> Result<Vec<StagedEntry>, OperationFailure> {
    let head_listing = git_output(
        repository_root,
        &["ls-tree", "-r", "-z", "--full-tree", &index.head],
        operation,
        None,
        "staged_candidate_unreadable",
    )?;
    let head_entries = parse_tree_entries(&head_listing, operation)?;
    let paths = index
        .entries
        .keys()
        .map(|(path, _)| path.clone())
        .chain(head_entries.keys().cloned())
        .collect::<BTreeSet<_>>();
    paths
        .into_iter()
        .filter_map(|path| {
            let proposal = index
                .entries
                .iter()
                .filter(|((entry_path, _), _)| entry_path.as_slice() == path.as_slice())
                .collect::<Vec<_>>();
            let status = match proposal.as_slice() {
                [] if head_entries.contains_key(&path) => Some('D'),
                [] => None,
                [((_, 0), candidate)] => match head_entries.get(&path) {
                    None => Some('A'),
                    Some(head) if head != *candidate => Some('M'),
                    Some(_) => None,
                },
                _ => Some('U'),
            }?;
            Some(Ok(StagedEntry { status, path }))
        })
        .collect()
}

pub(in crate::checkpoint) fn read_index_blob(
    repository_root: &Path,
    index: &ProposalIndex,
    path: &str,
    operation: &'static str,
) -> Result<Vec<u8>, OperationFailure> {
    if !safe_relative(Path::new(path)) {
        return Err(OperationFailure::new(
            operation,
            None,
            "invalid_git_path",
            "candidate path is not a safe repository-relative path",
            Vec::new(),
            "repair the Git index and retry",
        ));
    }
    let entries = index
        .entries
        .iter()
        .filter(|((entry_path, _), _)| entry_path == path.as_bytes())
        .collect::<Vec<_>>();
    let Some(entry) = entries
        .as_slice()
        .first()
        .copied()
        .filter(|_| entries.len() == 1)
    else {
        return Err(OperationFailure::new(
            operation,
            None,
            "staged_candidate_unreadable",
            "candidate path does not resolve to exactly one stage-zero index entry",
            vec![path.to_owned()],
            "stage one regular candidate file and retry",
        ));
    };
    let ((_, stage), blob) = entry;
    if *stage != 0 || blob.mode.as_slice() != b"100644" {
        return Err(OperationFailure::new(
            operation,
            None,
            "unsupported_staged_entry",
            "activation candidates must be regular non-executable stage-zero blobs",
            vec![path.to_owned()],
            "replace symlinks, executable files, and unresolved index entries",
        ));
    }
    let bytes = git_output(
        repository_root,
        &["cat-file", "blob", &blob.oid],
        operation,
        None,
        "staged_candidate_unreadable",
    )?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(OperationFailure::new(
            operation,
            None,
            "staged_record_too_large",
            "staged candidate exceeds the Pilot size limit",
            Vec::new(),
            "reduce or repair the staged record",
        ));
    }
    Ok(bytes)
}

pub(in crate::checkpoint) fn ensure_index_unchanged(
    repository_root: &Path,
    index: &ProposalIndex,
    operation: &'static str,
) -> Result<(), OperationFailure> {
    let current = git_output_with_index(
        repository_root,
        &["ls-files", "--stage", "-z"],
        index.file.as_deref(),
        operation,
        None,
        "staged_candidate_unreadable",
    )?;
    let head = git_output(
        repository_root,
        &["rev-parse", "--verify", "HEAD^{commit}"],
        operation,
        None,
        "staged_candidate_unreadable",
    )?;
    if current != index.identity || String::from_utf8_lossy(&head).trim() != index.head {
        return Err(OperationFailure::new(
            operation,
            None,
            "staged_candidate_changed_during_validation",
            "proposal index changed during prospective activation validation",
            Vec::new(),
            "retry after the staged candidate stops changing",
        ));
    }
    Ok(())
}

fn parse_index_entries(
    listing: &[u8],
    operation: &'static str,
) -> Result<IndexEntries, OperationFailure> {
    listing
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (header, path) = split_listing_entry(entry, operation)?;
            let fields = header.split_ascii_whitespace().collect::<Vec<_>>();
            if fields.len() != 3 || !valid_commit(fields[1]) {
                return Err(OperationFailure::new(
                    operation,
                    None,
                    "invalid_git_output",
                    "proposal index entry has an invalid mode, object ID, or stage",
                    Vec::new(),
                    "repair the Git index and retry",
                ));
            }
            let stage = fields[2].parse::<u8>().map_err(|error| {
                OperationFailure::new(
                    operation,
                    None,
                    "invalid_git_output",
                    error.to_string(),
                    Vec::new(),
                    "repair the Git index and retry",
                )
            })?;
            Ok((
                (path.to_vec(), stage),
                BlobIdentity {
                    mode: fields[0].as_bytes().to_vec(),
                    oid: fields[1].to_owned(),
                },
            ))
        })
        .collect()
}

fn parse_tree_entries(
    listing: &[u8],
    operation: &'static str,
) -> Result<TreeEntries, OperationFailure> {
    listing
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (header, path) = split_listing_entry(entry, operation)?;
            let fields = header.split_ascii_whitespace().collect::<Vec<_>>();
            if fields.len() != 3 || !valid_commit(fields[2]) {
                return Err(OperationFailure::new(
                    operation,
                    None,
                    "invalid_git_output",
                    "proposal parent tree entry has an invalid mode, type, or object ID",
                    Vec::new(),
                    "repair HEAD and retry",
                ));
            }
            Ok((
                path.to_vec(),
                BlobIdentity {
                    mode: fields[0].as_bytes().to_vec(),
                    oid: fields[2].to_owned(),
                },
            ))
        })
        .collect()
}

fn split_listing_entry<'a>(
    entry: &'a [u8],
    operation: &'static str,
) -> Result<(&'a str, &'a [u8]), OperationFailure> {
    let separator = entry
        .iter()
        .position(|byte| *byte == b'\t')
        .ok_or_else(|| {
            OperationFailure::new(
                operation,
                None,
                "invalid_git_output",
                "Git listing entry has no path separator",
                Vec::new(),
                "repair the Git repository and retry",
            )
        })?;
    let header = std::str::from_utf8(&entry[..separator]).map_err(|error| {
        OperationFailure::new(
            operation,
            None,
            "invalid_git_output",
            error.to_string(),
            Vec::new(),
            "repair the Git repository and retry",
        )
    })?;
    Ok((header, &entry[separator + 1..]))
}
