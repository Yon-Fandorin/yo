//! Local execution-workspace discovery kept outside the terminal UI thread.

use std::{
    collections::{BTreeSet, HashSet},
    ffi::OsStr,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    task::{Context, Poll},
    thread,
};

use super::{
    WorkspaceReference, WorkspaceReferenceCandidate, WorkspaceReferenceKind,
    WorkspaceReferenceProvider, WorkspaceReferenceProviderPoll, WorkspaceReferenceSearchRequest,
    WorkspaceReferenceSearchStatus, WorkspaceReferenceSearchUpdate, normalized_search_key,
};
use crate::WorkspaceHostId;

const RESULT_CAP: usize = 40;

pub struct LocalWorkspaceReferenceProvider {
    requests: Sender<WorkspaceReferenceSearchRequest>,
    updates: Receiver<WorkspaceReferenceSearchUpdate>,
    readiness: Arc<crate::readiness::Readiness>,
}

struct Inventory {
    entries: Vec<WorkspaceReferenceCandidate>,
    status: WorkspaceReferenceSearchStatus,
}

impl LocalWorkspaceReferenceProvider {
    pub fn start(root: &Path, workspace_host_id: WorkspaceHostId) -> Result<Self, std::io::Error> {
        let (request_tx, request_rx) = mpsc::channel();
        let (update_tx, update_rx) = mpsc::channel();
        let readiness = Arc::new(crate::readiness::Readiness::new());
        let worker_readiness = Arc::clone(&readiness);
        let root = std::fs::canonicalize(root)?;
        thread::Builder::new()
            .name("yo-workspace-search".to_owned())
            .spawn(move || {
                worker(
                    root,
                    workspace_host_id,
                    request_rx,
                    update_tx,
                    &worker_readiness,
                );
                worker_readiness.notify();
            })?;
        Ok(Self {
            requests: request_tx,
            updates: update_rx,
            readiness,
        })
    }
}

impl WorkspaceReferenceProvider for LocalWorkspaceReferenceProvider {
    fn search(&mut self, request: WorkspaceReferenceSearchRequest) -> Result<(), String> {
        self.requests
            .send(request)
            .map_err(|_| "workspace search worker closed".to_owned())
    }

    fn poll(&mut self) -> Result<WorkspaceReferenceProviderPoll, String> {
        match self.updates.try_recv() {
            Ok(update) => Ok(WorkspaceReferenceProviderPoll::Update(update)),
            Err(TryRecvError::Empty) => Ok(WorkspaceReferenceProviderPoll::Pending),
            Err(TryRecvError::Disconnected) => Err("workspace search worker closed".to_owned()),
        }
    }

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        self.readiness.poll(context)
    }
}

fn worker(
    root: PathBuf,
    workspace_host_id: WorkspaceHostId,
    requests: Receiver<WorkspaceReferenceSearchRequest>,
    updates: Sender<WorkspaceReferenceSearchUpdate>,
    readiness: &crate::readiness::Readiness,
) {
    let inventory = build_inventory(&root, workspace_host_id);
    while let Ok(mut request) = requests.recv() {
        while let Ok(newer) = requests.try_recv() {
            request = newer;
        }
        let update = match &inventory {
            Ok(inventory) => WorkspaceReferenceSearchUpdate::final_result(
                &request,
                inventory.status.clone(),
                search(&inventory.entries, request.query()),
            ),
            Err(error) => WorkspaceReferenceSearchUpdate::final_result(
                &request,
                WorkspaceReferenceSearchStatus::Failed(error.clone()),
                Vec::new(),
            ),
        };
        if updates.send(update).is_err() {
            break;
        }
        readiness.notify();
    }
}

fn build_inventory(root: &Path, workspace_host_id: WorkspaceHostId) -> Result<Inventory, String> {
    let honor_git_ignore = is_git_workspace(root)?;
    let (mut paths, mut incomplete) = discover_entries(root, honor_git_ignore)?;
    if honor_git_ignore {
        let (tracked, tracked_incomplete) = discover_tracked_entries(root)?;
        paths.extend(tracked);
        incomplete |= tracked_incomplete;
    }
    let root_identity = format!(
        "local-root:{}",
        hex_bytes(root.as_os_str().as_encoded_bytes())
    );
    let execution_environment_identity = format!("local-host:{workspace_host_id}");
    let workspace_identity = format!("{workspace_host_id}:{root_identity}");
    let entries = paths
        .into_iter()
        .map(|(path, kind)| {
            let kind_name = match kind {
                WorkspaceReferenceKind::File => "file",
                WorkspaceReferenceKind::Directory => "directory",
            };
            WorkspaceReference::new(
                format!("local:{kind_name}:{}", hex_bytes(path.as_bytes())),
                execution_environment_identity.clone(),
                workspace_identity.clone(),
                root_identity.clone(),
                path,
                kind,
            )
            .map(WorkspaceReferenceCandidate::new)
            .map_err(|error| format!("normalizing a discovered workspace path failed: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Inventory {
        entries,
        status: if incomplete {
            WorkspaceReferenceSearchStatus::Incomplete(
                "Some non-UTF-8 or unreadable workspace paths were skipped".to_owned(),
            )
        } else {
            WorkspaceReferenceSearchStatus::Complete
        },
    })
}

fn is_git_workspace(root: &Path) -> Result<bool, String> {
    if !has_git_marker(root)? {
        return Ok(false);
    }
    let output = git_command(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|error| format!("starting Git workspace detection failed: {error}"))?;
    classify_git_workspace(output.status.success(), &output.stdout, &output.stderr)
}

fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_IMPLICIT_WORK_TREE")
        .env_remove("GIT_GRAFT_FILE")
        .env_remove("GIT_NO_REPLACE_OBJECTS")
        .env_remove("GIT_REPLACE_REF_BASE")
        .env_remove("GIT_PREFIX")
        .env_remove("GIT_SHALLOW_FILE")
        .env_remove("GIT_CEILING_DIRECTORIES")
        .env_remove("GIT_DISCOVERY_ACROSS_FILESYSTEM");
    command
}

fn has_git_marker(root: &Path) -> Result<bool, String> {
    for ancestor in root.ancestors() {
        let marker = ancestor.join(".git");
        match std::fs::symlink_metadata(&marker) {
            Ok(_) => {
                let output = git_command(root)
                    .arg("rev-parse")
                    .arg("--resolve-git-dir")
                    .arg(&marker)
                    .output()
                    .map_err(|error| format!("validating a Git marker failed: {error}"))?;
                if !output.status.success() {
                    let diagnostic = String::from_utf8_lossy(&output.stderr);
                    let diagnostic = diagnostic.trim();
                    return Err(if diagnostic.is_empty() {
                        format!("the Git marker at {} is invalid", marker.display())
                    } else {
                        format!(
                            "validating the Git marker at {} failed: {diagnostic}",
                            marker.display()
                        )
                    });
                }
                return Ok(true);
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => {
                return Err(format!(
                    "checking the Git marker at {} failed: {error}",
                    marker.display()
                ));
            },
        }
    }
    Ok(false)
}

fn classify_git_workspace(success: bool, stdout: &[u8], stderr: &[u8]) -> Result<bool, String> {
    if success {
        return match stdout.trim_ascii() {
            b"true" => Ok(true),
            b"false" => Ok(false),
            other => Err(format!(
                "Git workspace detection returned an unexpected value: {}",
                String::from_utf8_lossy(other)
            )),
        };
    }
    let diagnostic = String::from_utf8_lossy(stderr);
    let diagnostic = diagnostic.trim();
    Err(if diagnostic.is_empty() {
        "Git workspace detection failed without a diagnostic".to_owned()
    } else {
        format!("Git workspace detection failed: {diagnostic}")
    })
}

fn discover_tracked_entries(
    root: &Path,
) -> Result<(BTreeSet<(String, WorkspaceReferenceKind)>, bool), String> {
    let output = git_output(root, ["ls-files", "-z", "--cached"])?;
    let mut visible = BTreeSet::new();
    let mut incomplete = false;
    for raw_path in output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let Ok(path) = std::str::from_utf8(raw_path) else {
            incomplete = true;
            continue;
        };
        let relative = Path::new(path);
        let mut current = PathBuf::new();
        let mut valid = true;
        let components = relative.components().collect::<Vec<_>>();
        for (index, component) in components.iter().enumerate() {
            current.push(component.as_os_str());
            let metadata = match std::fs::symlink_metadata(root.join(&current)) {
                Ok(metadata) => metadata,
                Err(_) => {
                    valid = false;
                    break;
                },
            };
            if metadata.file_type().is_symlink() {
                valid = false;
                break;
            }
            let normalized = current
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            if index + 1 == components.len() {
                if metadata.is_file() {
                    visible.insert((normalized, WorkspaceReferenceKind::File));
                }
            } else if metadata.is_dir() {
                visible.insert((normalized, WorkspaceReferenceKind::Directory));
            } else {
                valid = false;
                break;
            }
        }
        if !valid {
            continue;
        }
    }
    Ok((visible, incomplete))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn discover_entries(
    root: &Path,
    honor_git_ignore: bool,
) -> Result<(BTreeSet<(String, WorkspaceReferenceKind)>, bool), String> {
    let mut visible = BTreeSet::new();
    let mut skipped_non_utf8 = false;
    let mut frontier = vec![PathBuf::new()];
    while !frontier.is_empty() {
        let mut candidates = Vec::new();
        for relative in std::mem::take(&mut frontier) {
            let directory = root.join(&relative);
            let entries = std::fs::read_dir(&directory)
                .map_err(|error| format!("reading {} failed: {error}", directory.display()))?;
            for entry in entries {
                let entry =
                    entry.map_err(|error| format!("reading a workspace entry failed: {error}"))?;
                if entry.file_name() == OsStr::new(".git") {
                    continue;
                }
                let file_type = entry.file_type().map_err(|error| {
                    format!(
                        "reading the kind of {} failed: {error}",
                        entry.path().display()
                    )
                })?;
                if !file_type.is_symlink() && (file_type.is_dir() || file_type.is_file()) {
                    candidates.push((
                        relative.join(entry.file_name()),
                        if file_type.is_dir() {
                            WorkspaceReferenceKind::Directory
                        } else {
                            WorkspaceReferenceKind::File
                        },
                    ));
                }
            }
        }
        let raw_paths = candidates
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        let ignored = if honor_git_ignore {
            ignored_paths(root, &raw_paths)?
        } else {
            HashSet::new()
        };
        for (candidate, kind) in candidates {
            let Some(path) = candidate.to_str() else {
                skipped_non_utf8 = true;
                continue;
            };
            let normalized = path.replace(std::path::MAIN_SEPARATOR, "/");
            if !ignored.contains(&normalized) {
                visible.insert((normalized, kind));
                if kind == WorkspaceReferenceKind::Directory {
                    frontier.push(candidate);
                }
            }
        }
    }
    Ok((visible, skipped_non_utf8))
}

fn ignored_paths(root: &Path, paths: &[PathBuf]) -> Result<HashSet<String>, String> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }
    let mut child = git_command(root)
        .args(["check-ignore", "--stdin", "-z"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("starting git check-ignore failed: {error}"))?;
    {
        let stdin = child.stdin.as_mut().expect("piped Git stdin is available");
        for path in paths {
            stdin
                .write_all(path.as_os_str().as_encoded_bytes())
                .and_then(|()| stdin.write_all(&[0]))
                .map_err(|error| format!("sending paths to git check-ignore failed: {error}"))?;
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("waiting for git check-ignore failed: {error}"))?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|path| std::str::from_utf8(path).ok())
        .map(|path| path.replace(std::path::MAIN_SEPARATOR, "/"))
        .collect())
}

fn git_output<const N: usize>(root: &Path, args: [&str; N]) -> Result<Vec<u8>, String> {
    let output = git_command(root)
        .args(args)
        .output()
        .map_err(|error| format!("starting Git workspace discovery failed: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

fn search(
    entries: &[WorkspaceReferenceCandidate],
    query: &str,
) -> Vec<WorkspaceReferenceCandidate> {
    let query = normalized_search_key(query.trim_end_matches('/'));
    let mut ranked = entries
        .iter()
        .filter_map(|candidate| {
            let path = candidate.reference().relative_path();
            let label = path.rsplit('/').next().unwrap_or(path);
            rank(path, label, &query).map(|score| {
                (
                    score,
                    normalized_search_key(candidate.reference().relative_path()),
                    candidate,
                )
            })
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    ranked
        .into_iter()
        .take(RESULT_CAP)
        .map(|(_, _, candidate)| candidate.clone())
        .collect()
}

fn rank(path: &str, label: &str, query: &str) -> Option<(u8, usize, usize, usize)> {
    let depth = path.bytes().filter(|byte| *byte == b'/').count();
    if query.is_empty() {
        return Some((0, 0, depth, path.chars().count()));
    }
    let path = normalized_search_key(path);
    let label = normalized_search_key(label);
    let path_length = path.chars().count();
    if path == query || label == query {
        return Some((0, 0, depth, path_length));
    }
    if path.starts_with(query) || label.starts_with(query) {
        return Some((1, 0, depth, path_length));
    }
    if path.split('/').any(|segment| segment.starts_with(query)) {
        return Some((2, 0, depth, path_length));
    }
    if label.contains(query) {
        return Some((3, 0, depth, path_length));
    }
    if path.contains(query) {
        return Some((3, 0, depth, path_length));
    }
    subsequence_gaps(query, &path).map(|gaps| (4, gaps, depth, path_length))
}

fn subsequence_gaps(query: &str, candidate: &str) -> Option<usize> {
    let mut positions = candidate.chars().enumerate();
    let mut previous_index = None;
    let mut gaps = 0;
    for wanted in query.chars() {
        let (position, _) = positions.by_ref().find(|(_, found)| *found == wanted)?;
        if let Some(previous_index) = previous_index {
            gaps += position.saturating_sub(previous_index + 1);
        }
        previous_index = Some(position);
    }
    Some(gaps)
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsStr, fs, os::unix::fs::symlink, path::Path, process::Command, time::SystemTime,
    };

    use super::{
        build_inventory, classify_git_workspace, discover_entries, git_command, is_git_workspace,
        rank, search,
    };
    use crate::{WorkspaceReference, WorkspaceReferenceCandidate, WorkspaceReferenceKind};

    fn candidate(path: &str) -> WorkspaceReferenceCandidate {
        WorkspaceReferenceCandidate::new(
            WorkspaceReference::new(
                path,
                "environment",
                "workspace",
                "root",
                path,
                WorkspaceReferenceKind::File,
            )
            .unwrap(),
        )
    }

    // basename의 정확 일치와 접두 일치는 단순 경로 포함보다 먼저 나오며 동률은 경로순으로 고정된다.
    #[test]
    fn ranking_prioritizes_familiar_path_matches_deterministically() {
        let entries = vec![
            candidate("src/main.rs"),
            candidate("notes/main-guide.md"),
            candidate("examples/domain.rs"),
        ];
        let results = search(&entries, "main");
        assert_eq!(results[0].reference().relative_path(), "src/main.rs");
        assert_eq!(
            results[1].reference().relative_path(),
            "notes/main-guide.md"
        );
        assert!(
            rank("src/main.rs", "main.rs", "src/main.rs") < rank("src/main.rs", "main.rs", "main")
        );
        assert!(rank("src/main.rs", "main.rs", "missing").is_none());
    }

    // 디렉터리 표시용 `/`는 검색 점수에 섞지 않아 basename 정확 일치를 그대로 보존한다.
    #[test]
    fn directory_decoration_does_not_weaken_an_exact_basename_match() {
        assert_eq!(
            rank("src/components", "components", "components")
                .unwrap()
                .0,
            0
        );
    }

    // Git 미설치·권한·safe.directory 오류를 일반 폴더로 오인해 ignore 파일을 노출하지 않는다.
    #[test]
    fn git_detection_distinguishes_non_repositories_from_operational_failures() {
        assert!(classify_git_workspace(true, b"true\n", b"").unwrap());
        assert!(!classify_git_workspace(true, b"false\n", b"").unwrap());
        assert!(classify_git_workspace(false, b"", b"fatal: not a git repository\n").is_err());
        assert!(
            classify_git_workspace(false, b"", b"fatal: detected dubious ownership\n").is_err()
        );
        assert!(
            classify_git_workspace(false, b"", "치명적: 깃 저장소가 아닙니다\n".as_bytes())
                .is_err()
        );
    }

    // 깨진 `.git` 표식은 ignore 보호를 끈 일반 폴더로 강등하지 않고 명시적으로 실패한다.
    #[test]
    fn invalid_git_marker_fails_closed() {
        let suffix = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("yo-invalid-git-marker-{suffix}"));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(".git"), "gitdir: missing\n").unwrap();

        assert!(is_git_workspace(&root).is_err());

        fs::remove_dir_all(root).unwrap();
    }

    // provider 소유 Git 명령은 caller의 대체 index를 제거해 실제 workspace index만 읽는다.
    #[test]
    fn provider_git_commands_remove_an_inherited_alternate_index() {
        let command = git_command(Path::new("."));
        assert!(
            command
                .get_envs()
                .any(|(name, value)| { name == OsStr::new("GIT_INDEX_FILE") && value.is_none() })
        );
    }

    // 실제 Git 저장소에서 nested .gitignore와 repository exclude를 적용하면서
    // 숨김 파일과 파일을 가진 디렉터리는 후보로 남기고 Git 내부는 노출하지 않는다.
    #[test]
    fn inventory_uses_the_effective_repository_ignore_stack() {
        let suffix = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("yo-workspace-{suffix}"));
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::create_dir_all(root.join("ignored")).unwrap();
        assert!(
            Command::new("git")
                .arg("init")
                .arg("-q")
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        fs::write(root.join(".git/info/exclude"), "local-only.txt\n").unwrap();
        fs::write(root.join(".hidden"), "visible").unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("src/nested/.gitignore"), "skip.md\n").unwrap();
        fs::write(root.join("src/nested/keep.md"), "keep\n").unwrap();
        fs::write(root.join("src/nested/skip.md"), "skip\n").unwrap();
        fs::write(root.join("ignored/private.txt"), "private\n").unwrap();
        fs::write(root.join("ignored/tracked.txt"), "tracked\n").unwrap();
        fs::write(root.join("local-only.txt"), "local\n").unwrap();
        symlink("src", root.join("linked-src")).unwrap();
        symlink("/tmp", root.join("outside-link")).unwrap();
        assert!(
            Command::new("git")
                .args(["add", "-f", "ignored/tracked.txt"])
                .current_dir(&root)
                .status()
                .unwrap()
                .success()
        );

        let host_id = "10000000-0000-4000-8000-000000000001".parse().unwrap();
        let inventory = build_inventory(&root, host_id).unwrap();
        let paths = inventory
            .entries
            .iter()
            .map(|entry| entry.reference().relative_path())
            .collect::<Vec<_>>();
        assert!(paths.contains(&".hidden"));
        assert!(paths.contains(&"src"));
        assert!(paths.contains(&"src/nested/keep.md"));
        assert!(!paths.iter().any(|path| path.starts_with(".git/")));
        assert!(paths.contains(&"ignored"));
        assert!(paths.contains(&"ignored/tracked.txt"));
        assert!(!paths.contains(&"ignored/private.txt"));
        assert!(!paths.contains(&"src/nested/skip.md"));
        assert!(!paths.contains(&"local-only.txt"));
        assert!(!paths.contains(&"linked-src"));
        assert!(!paths.contains(&"outside-link"));

        fs::remove_dir_all(root).unwrap();
    }

    // Git 저장소가 아닌 일반 작업 디렉터리도 파일과 디렉터리를 같은 후보 계약으로 제공한다.
    #[test]
    fn discovery_supports_a_plain_non_git_workspace() {
        let suffix = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("yo-plain-workspace-{suffix}"));
        fs::create_dir_all(root.join("notes/drafts")).unwrap();
        fs::write(root.join("notes/drafts/plan.md"), "plan\n").unwrap();

        let (entries, incomplete) = discover_entries(&root, false).unwrap();
        assert!(!incomplete);
        let paths = entries
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"notes"));
        assert!(paths.contains(&"notes/drafts"));
        assert!(paths.contains(&"notes/drafts/plan.md"));

        fs::remove_dir_all(root).unwrap();
    }
}
