use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use nix::{
    dir::Dir,
    errno::Errno,
    sys::stat::{SFlag, fstatat},
};

use crate::local_tools::execution::{completed, failed, interrupted};

const MAX_LIST_ENTRIES: usize = 100_000;
const LIST_TRUNCATION_MARKER: &str = "\n[yo: tool output truncated]";

pub(super) fn list_files(
    mut directory: Dir,
    relative_directory: PathBuf,
    limit: usize,
    cancelled: &AtomicBool,
) -> yo_core::ToolExecutionResult {
    let retained = retain_list_names(
        directory.iter().map(|entry| {
            entry.map(|entry| OsString::from(OsStr::from_bytes(entry.file_name().to_bytes())))
        }),
        MAX_LIST_ENTRIES,
        cancelled,
    );
    let RetainedListNames { names, truncated } = match retained {
        Ok(retained) => retained,
        Err(ListObservationError::Interrupted) => return interrupted(),
        Err(ListObservationError::Failed) => return failed("list_files failed"),
    };
    render_list_names(
        names,
        &relative_directory,
        limit,
        truncated,
        cancelled,
        |name| classify_list_entry(&directory, name),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ListObservationError {
    Interrupted,
    Failed,
}

#[derive(Debug, Eq, PartialEq)]
struct RetainedListNames {
    names: Vec<OsString>,
    truncated: bool,
}

fn retain_list_names(
    mut entries: impl Iterator<Item = Result<OsString, Errno>>,
    maximum: usize,
    cancelled: &AtomicBool,
) -> Result<RetainedListNames, ListObservationError> {
    let mut names = Vec::with_capacity(maximum.min(4_096));
    let truncated = loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(ListObservationError::Interrupted);
        }
        let entry = entries.next();
        if cancelled.load(Ordering::Acquire) {
            return Err(ListObservationError::Interrupted);
        }
        let Some(entry) = entry else {
            break false;
        };
        let name = entry.map_err(|_| ListObservationError::Failed)?;
        if name.as_bytes() == b"." || name.as_bytes() == b".." {
            continue;
        }
        if names.len() == maximum {
            break true;
        }
        names.push(name);
    };
    names.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    Ok(RetainedListNames { names, truncated })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ListedEntryKind {
    Directory,
    Regular,
    Excluded,
}

fn classify_list_entry(directory: &Dir, name: &OsStr) -> Result<ListedEntryKind, Errno> {
    let metadata = fstatat(directory, name, nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW)?;
    let file_type = SFlag::from_bits_truncate(metadata.st_mode) & SFlag::S_IFMT;
    Ok(if file_type == SFlag::S_IFDIR {
        ListedEntryKind::Directory
    } else if file_type == SFlag::S_IFREG {
        ListedEntryKind::Regular
    } else {
        ListedEntryKind::Excluded
    })
}

fn render_list_names(
    names: Vec<OsString>,
    relative_directory: &Path,
    limit: usize,
    mut truncated: bool,
    cancelled: &AtomicBool,
    mut classify: impl FnMut(&OsStr) -> Result<ListedEntryKind, Errno>,
) -> yo_core::ToolExecutionResult {
    let reserved_limit = limit.saturating_sub(LIST_TRUNCATION_MARKER.len());
    let mut complete_output = String::new();
    let mut reserved_output = String::new();
    let mut reserved_open = limit > LIST_TRUNCATION_MARKER.len();

    for name in names {
        if cancelled.load(Ordering::Acquire) {
            return interrupted();
        }
        if name.as_bytes() == b".git" {
            continue;
        }
        let Some(name) = name
            .to_str()
            .filter(|name| !name.chars().any(char::is_control))
        else {
            truncated = true;
            continue;
        };
        let kind = match classify(OsStr::new(name)) {
            Ok(kind) => kind,
            Err(Errno::ENOENT) => continue,
            Err(_) => return failed("list_files failed"),
        };
        if cancelled.load(Ordering::Acquire) {
            return interrupted();
        }
        let directory = matches!(kind, ListedEntryKind::Directory);
        if matches!(kind, ListedEntryKind::Excluded) {
            continue;
        }
        let relative = relative_directory.join(name);
        let token = relative
            .to_str()
            .expect("an admitted path joined with exact UTF-8 remains UTF-8");
        let token_len = token.len().saturating_add(usize::from(directory));
        if token_len > 1_024 || token.chars().any(char::is_control) {
            truncated = true;
            continue;
        }
        let line = if directory {
            format!("{token}/\n")
        } else {
            format!("{token}\n")
        };
        if reserved_open {
            if reserved_output.len().saturating_add(line.len()) <= reserved_limit {
                reserved_output.push_str(&line);
            } else {
                reserved_open = false;
            }
        }
        if complete_output.len().saturating_add(line.len()) > limit {
            if cancelled.load(Ordering::Acquire) {
                return interrupted();
            }
            return completed(reserved_output, true);
        }
        complete_output.push_str(&line);
    }
    if cancelled.load(Ordering::Acquire) {
        return interrupted();
    }
    if truncated {
        completed(reserved_output, true)
    } else {
        completed(complete_output, false)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        ffi::{OsStr, OsString},
        os::unix::ffi::OsStringExt,
        path::Path,
        sync::atomic::{AtomicBool, Ordering},
    };

    use nix::errno::Errno;

    use super::{
        LIST_TRUNCATION_MARKER, ListObservationError, ListedEntryKind, MAX_LIST_ENTRIES,
        render_list_names, retain_list_names,
    };

    // raw entry 한도는 정렬 전에 iteration 순서로 자르고 100001번째만 probe하므로,
    // 전역 정렬로 더 작은 뒤쪽 이름을 선택하거나 probe 뒤를 읽는 구현을 막습니다.
    #[test]
    fn raw_entry_budget_uses_one_probe_before_unsigned_sorting() {
        let cancelled = AtomicBool::new(false);
        let pulled = Cell::new(0_usize);
        let entries = [b".".as_slice(), b"z", b"a", b"b", b"unread"]
            .into_iter()
            .map(|name| {
                pulled.set(pulled.get() + 1);
                Ok(OsString::from_vec(name.to_vec()))
            });
        let retained = retain_list_names(entries, 2, &cancelled).unwrap();
        assert_eq!(retained.names, [OsString::from("a"), OsString::from("z")]);
        assert!(retained.truncated);
        assert_eq!(pulled.get(), 4);

        let exact = retain_list_names(
            (0..MAX_LIST_ENTRIES).map(|index| Ok(OsString::from(index.to_string()))),
            MAX_LIST_ENTRIES,
            &cancelled,
        )
        .unwrap();
        assert_eq!(exact.names.len(), MAX_LIST_ENTRIES);
        assert!(!exact.truncated);

        let probed = Cell::new(0_usize);
        let over = retain_list_names(
            (0..MAX_LIST_ENTRIES + 2).map(|index| {
                probed.set(probed.get() + 1);
                Ok(OsString::from(index.to_string()))
            }),
            MAX_LIST_ENTRIES,
            &cancelled,
        )
        .unwrap();
        assert_eq!(over.names.len(), MAX_LIST_ENTRIES);
        assert!(over.truncated);
        assert_eq!(probed.get(), MAX_LIST_ENTRIES + 1);
    }

    // `.`과 `..`만 budget 밖이며 `.git`은 retained slot을 소비한 뒤 분류 없이 빠져,
    // `.git`을 공짜 이름으로 취급해 다음 entry까지 노출하는 변형을 구분합니다.
    #[test]
    fn dot_git_consumes_the_raw_budget_without_classification() {
        let retained = retain_list_names(
            [".", "..", ".git", "visible"]
                .into_iter()
                .map(|name| Ok(OsString::from(name))),
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(retained.names, [OsString::from(".git")]);
        assert!(retained.truncated);

        let calls = Cell::new(0_usize);
        let result = render_list_names(
            retained.names,
            Path::new(""),
            4096,
            retained.truncated,
            &AtomicBool::new(false),
            |_| {
                calls.set(calls.get() + 1);
                Ok(ListedEntryKind::Regular)
            },
        );
        assert_eq!(calls.get(), 0);
        assert!(result.output().is_empty());
        assert!(result.truncated());
    }

    // UTF-8로 표현할 수 없거나 control scalar가 든 raw name은 fstatat 전에 빠지고,
    // 정상 이름만 한 번 분류되어 lossy/escape 경로가 model output에 생기지 않습니다.
    #[test]
    fn unrepresentable_names_are_truncated_before_classification() {
        let classified = Cell::new(0_usize);
        let result = render_list_names(
            vec![
                OsString::from_vec(vec![0xff]),
                OsString::from("control\nname"),
                OsString::from("valid"),
            ],
            Path::new("selected"),
            4096,
            false,
            &AtomicBool::new(false),
            |name| {
                assert_eq!(name, OsStr::new("valid"));
                classified.set(classified.get() + 1);
                Ok(ListedEntryKind::Regular)
            },
        );
        assert_eq!(classified.get(), 1);
        assert_eq!(result.output(), "selected/valid\n");
        assert!(result.truncated());
    }

    // directory의 `/`까지 포함한 model-visible token은 1024 bytes를 허용하고 1025
    // bytes부터 생략+truncated로 바뀌어, LF만 제외한다는 경계를 고정합니다.
    #[test]
    fn rendered_directory_token_enforces_the_complete_byte_limit() {
        let at_limit = render_list_names(
            vec![OsString::from("b")],
            Path::new(&"a".repeat(1_021)),
            4096,
            false,
            &AtomicBool::new(false),
            |_| Ok(ListedEntryKind::Directory),
        );
        assert_eq!(at_limit.output().len(), 1_025);
        assert!(!at_limit.truncated());

        let over = render_list_names(
            vec![OsString::from("b")],
            Path::new(&"a".repeat(1_022)),
            4096,
            false,
            &AtomicBool::new(false),
            |_| Ok(ListedEntryKind::Directory),
        );
        assert!(over.output().is_empty());
        assert!(over.truncated());

        let regular = render_list_names(
            vec![OsString::from("é")],
            Path::new(&"a".repeat(1_021)),
            4096,
            false,
            &AtomicBool::new(false),
            |_| Ok(ListedEntryKind::Regular),
        );
        assert_eq!(regular.output().len(), 1_025);
        assert!(!regular.truncated());

        let regular_over = render_list_names(
            vec![OsString::from("é")],
            Path::new(&"a".repeat(1_022)),
            4096,
            false,
            &AtomicBool::new(false),
            |_| Ok(ListedEntryKind::Regular),
        );
        assert!(regular_over.output().is_empty());
        assert!(regular_over.truncated());
    }

    // ENOENT는 사라진 child 하나만 건너뛰지만, 이미 만든 줄 뒤의 EIO도 전체 결과를
    // exact Failed로 바꿔 partial output과 truncated 상태가 새지 않게 합니다.
    #[test]
    fn metadata_failure_discards_partial_output_but_enoent_skips_one_child() {
        let names = vec![OsString::from("a"), OsString::from("b")];
        let skipped = render_list_names(
            names.clone(),
            Path::new(""),
            4096,
            false,
            &AtomicBool::new(false),
            |name| {
                if name == "a" {
                    Err(Errno::ENOENT)
                } else {
                    Ok(ListedEntryKind::Regular)
                }
            },
        );
        assert_eq!(skipped.output(), "b\n");
        assert!(!skipped.truncated());

        let failed = render_list_names(
            names,
            Path::new(""),
            4096,
            false,
            &AtomicBool::new(false),
            |name| {
                if name == "b" {
                    Err(Errno::EIO)
                } else {
                    Ok(ListedEntryKind::Regular)
                }
            },
        );
        assert_eq!(failed.outcome(), yo_core::ToolExecutionOutcome::Failed);
        assert_eq!(failed.output(), "list_files failed");
        assert!(!failed.truncated());

        assert_eq!(
            retain_list_names(
                [Ok(OsString::from("a")), Err(Errno::EIO)].into_iter(),
                10,
                &AtomicBool::new(false),
            ),
            Err(ListObservationError::Failed)
        );
    }

    // 마지막 fstatat 동안 취소가 도착해도 publication 전 check가 이를 관찰하여,
    // 직전에 만든 정상 줄까지 버리고 exact Interrupted만 반환합니다.
    #[test]
    fn cancellation_after_the_last_classification_discards_output() {
        let cancelled = AtomicBool::new(false);
        let result = render_list_names(
            vec![OsString::from("first"), OsString::from("last")],
            Path::new(""),
            4096,
            false,
            &cancelled,
            |name| {
                if name == "last" {
                    cancelled.store(true, Ordering::Release);
                }
                Ok(ListedEntryKind::Regular)
            },
        );
        assert_eq!(result.outcome(), yo_core::ToolExecutionOutcome::Interrupted);
        assert_eq!(result.output(), "interrupted");
        assert!(!result.truncated());
    }

    // 불완전 결과는 common marker 전체를 먼저 예약하고 완전한 LF 줄만 넘기며,
    // tiny bound는 빈 worker prefix로 남겨 상위 bounded_output이 marker prefix만 만듭니다.
    #[test]
    fn incomplete_listing_reserves_the_exact_marker_without_cutting_lines() {
        assert_eq!(LIST_TRUNCATION_MARKER, "\n[yo: tool output truncated]");
        for limit in [0, 1, 27, 28, 29] {
            let result = render_list_names(
                vec![OsString::from("a")],
                Path::new(""),
                limit,
                true,
                &AtomicBool::new(false),
                |_| Ok(ListedEntryKind::Regular),
            );
            assert!(result.output().is_empty(), "limit {limit}");
            assert!(result.truncated());
        }
        for (limit, expected) in [
            (LIST_TRUNCATION_MARKER.len() + 1, ""),
            (LIST_TRUNCATION_MARKER.len() + 2, "a\n"),
            (LIST_TRUNCATION_MARKER.len() + 3, "a\n"),
        ] {
            let result = render_list_names(
                vec![OsString::from("a")],
                Path::new(""),
                limit,
                true,
                &AtomicBool::new(false),
                |_| Ok(ListedEntryKind::Regular),
            );
            assert_eq!(result.output(), expected, "limit {limit}");
            assert!(result.truncated());
        }

        let long = "x".repeat(20);
        let result = render_list_names(
            vec![
                OsString::from(format!("a{long}")),
                OsString::from(format!("b{long}")),
                OsString::from(format!("c{long}")),
            ],
            Path::new(""),
            LIST_TRUNCATION_MARKER.len() + 22,
            false,
            &AtomicBool::new(false),
            |_| Ok(ListedEntryKind::Regular),
        );
        assert_eq!(result.output(), format!("a{long}\n"));
        assert!(result.truncated());

        let exact = render_list_names(
            vec![OsString::from("a"), OsString::from("b")],
            Path::new(""),
            4,
            false,
            &AtomicBool::new(false),
            |_| Ok(ListedEntryKind::Regular),
        );
        assert_eq!(exact.output(), "a\nb\n");
        assert!(!exact.truncated());
    }
}
