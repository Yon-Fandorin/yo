//! 한 호출에 고정한 workspace-v2 실행 범위와 CLI 소유 플랫폼 경계.

#[cfg(target_os = "linux")]
mod linux;
mod risk;

use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Read},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{self, Path, PathBuf},
    process::{Command, Stdio},
    str,
    sync::atomic::AtomicBool,
    thread,
    time::{Duration, Instant},
};

use serde_json::json;
use sha2::{Digest, Sha256};
use yo_core::{
    ToolExecutionError, ToolExecutionPlan, ToolExecutionPreparation, ToolExecutionRequest,
    ToolPlanUnavailable,
};

use super::PreparedCommand;

const PROFILE: &str = "yo.command-execution/workspace-confined-v2";
const PRIVATE_HOME: &str = "/yo-private/home";
const PRIVATE_TMP: &str = "/yo-private/tmp";

/// 조립 경계가 실제 저장소 위치와 현재 호스트 에이전트 endpoint를 제공한다.
pub(crate) struct CommandConfinement {
    workspace: PathBuf,
    secret_roots: Vec<PathBuf>,
    secret_endpoints: Vec<PathBuf>,
    support_roots: Vec<(PathBuf, PathBuf)>,
    git_identity: Vec<(String, String)>,
}

struct PinnedRoot {
    source: PathBuf,
    destination: PathBuf,
    file: File,
    writable: bool,
}

type AncestorIdentity = (PathBuf, Option<(u64, u64)>);

struct HiddenPath {
    path: PathBuf,
    directory: bool,
    source: PathBuf,
    ancestors: Vec<AncestorIdentity>,
}

enum Payload {
    Shell(String),
    Configured(Box<PreparedCommand>, Vec<u8>),
}

pub(in crate::execution::tools) struct ConfinedPlan {
    workspace: PathBuf,
    roots: Vec<PinnedRoot>,
    hidden: Vec<HiddenPath>,
    environment: Vec<(String, String)>,
    payload: Payload,
    request_digest: [u8; 32],
    turn: yo_core::TurnRef,
    call_id: String,
    definition: yo_core::ToolDefinition,
    maximum_output_bytes: usize,
    maximum_retained_output_bytes: Option<usize>,
    absolute_execution_timeout: Option<Duration>,
}

impl CommandConfinement {
    pub(crate) fn from_environment(
        workspace: &Path,
        mut protected_roots: Vec<PathBuf>,
    ) -> Result<Self, ToolExecutionError> {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute())
            .ok_or_else(error)?;
        for name in [
            ".ssh",
            ".aws",
            ".gnupg",
            ".codex",
            ".config",
            ".azure",
            ".kube",
            ".local/state/yo",
        ] {
            protected_roots.push(home.join(name));
        }
        for key in [
            "XDG_RUNTIME_DIR",
            "XDG_CONFIG_HOME",
            "GNUPGHOME",
            "DOCKER_CONFIG",
            "AZURE_CONFIG_DIR",
        ] {
            if let Some(path) = env::var_os(key) {
                protected_roots.push(PathBuf::from(path));
            }
        }
        let mut endpoints = vec![
            home.join(".netrc"),
            home.join(".npmrc"),
            home.join(".pypirc"),
            home.join(".cargo/credentials"),
            home.join(".cargo/credentials.toml"),
        ];
        for key in [
            "SSH_AUTH_SOCK",
            "AWS_SHARED_CREDENTIALS_FILE",
            "AWS_CONFIG_FILE",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "NPM_CONFIG_USERCONFIG",
        ] {
            if let Some(path) = env::var_os(key).filter(|path| !path.is_empty()) {
                endpoints.push(PathBuf::from(path));
            }
        }
        if let Some(paths) = env::var_os("KUBECONFIG") {
            endpoints.extend(env::split_paths(&paths));
        }
        if let Ok(info) = env::var("GPG_AGENT_INFO")
            && let Some(path) = info.split(':').next().filter(|path| !path.is_empty())
        {
            endpoints.push(PathBuf::from(path));
        }
        if let Ok(address) = env::var("DBUS_SESSION_BUS_ADDRESS") {
            for entry in address.split(';') {
                if let Some(value) = entry.strip_prefix("unix:path=") {
                    let path = value.split(',').next().ok_or_else(error)?;
                    if path.contains('%') {
                        return Err(error());
                    }
                    endpoints.push(PathBuf::from(path));
                } else if !entry.starts_with("unix:abstract=") {
                    // 호스트 TCP 세션 버스 등은 private-net으로 도달할 수 없다.
                }
            }
        }
        let cargo = env::var_os("CARGO_HOME").map_or_else(|| home.join(".cargo"), PathBuf::from);
        endpoints.extend([cargo.join("credentials"), cargo.join("credentials.toml")]);
        let rustup = env::var_os("RUSTUP_HOME").map_or_else(|| home.join(".rustup"), PathBuf::from);
        let mut support = Vec::new();
        for path in [cargo.join("bin"), rustup] {
            if path.exists() {
                support.push((path.clone(), path));
            }
        }
        for name in ["registry", "git"] {
            let path = cargo.join(name);
            if path.exists() {
                support.push((path, Path::new(PRIVATE_HOME).join(".cargo").join(name)));
            }
        }
        let mut policy = Self::new(workspace, protected_roots, endpoints, support)?;
        policy.git_identity = read_git_identity(
            workspace,
            &home,
            env::var_os("XDG_CONFIG_HOME").as_deref().map(Path::new),
        );
        Ok(policy)
    }

    pub(crate) fn new(
        workspace: &Path,
        secret_roots: Vec<PathBuf>,
        secret_endpoints: Vec<PathBuf>,
        support_roots: Vec<(PathBuf, PathBuf)>,
    ) -> Result<Self, ToolExecutionError> {
        Ok(Self {
            workspace: workspace.canonicalize().map_err(|_| error())?,
            secret_roots: freeze_exclusions(secret_roots).map_err(|_| error())?,
            secret_endpoints: freeze_exclusions(secret_endpoints).map_err(|_| error())?,
            support_roots,
            git_identity: Vec::new(),
        })
    }

    // 정확한 보호 파일을 숨기되 그 파일의 workspace 부모는 쓰기 가능하게 유지한다.
    pub(crate) fn with_protected_file(mut self, path: &Path) -> Result<Self, ToolExecutionError> {
        let absolute = path::absolute(path).map_err(|_| error())?;
        self.secret_endpoints
            .extend(freeze_exclusions(vec![absolute]).map_err(|_| error())?);
        self.secret_endpoints.sort();
        self.secret_endpoints.dedup();
        Ok(self)
    }

    pub(crate) fn prepare(
        &self,
        request: &ToolExecutionRequest,
        configured: Option<PreparedCommand>,
    ) -> ToolExecutionPreparation {
        match self.plan(request, configured) {
            Ok((plan, approval, scope, identity)) => {
                let plan = ToolExecutionPlan::new(identity, scope, plan);
                if approval {
                    ToolExecutionPreparation::ApprovalRequired(plan)
                } else {
                    ToolExecutionPreparation::Automatic(plan)
                }
            },
            Err(reason) => ToolExecutionPreparation::Unavailable(reason),
        }
    }

    fn plan(
        &self,
        request: &ToolExecutionRequest,
        configured: Option<PreparedCommand>,
    ) -> Result<(ConfinedPlan, bool, String, [u8; 32]), ToolPlanUnavailable> {
        if !platform_available() {
            return Err(ToolPlanUnavailable::UnqualifiedPlatform);
        }
        let (payload, assessment, configured) = if let Some(command) = configured {
            if command.definition() != request.call.definition() {
                return Err(ToolPlanUnavailable::UnsupportedProfile);
            }
            (
                Payload::Configured(
                    Box::new(command),
                    request.call.normalized_arguments().to_vec(),
                ),
                risk::Assessment::default(),
                true,
            )
        } else {
            let text = request
                .call
                .arguments()
                .get("command")
                .and_then(|value| value.as_str())
                .filter(|text| !text.is_empty() && !text.contains('\0'))
                .ok_or(ToolPlanUnavailable::UnresolvedScope)?;
            (Payload::Shell(text.to_owned()), risk::assess(text), false)
        };
        // 공유 host-net은 추상 host IPC의 배제를 별도로 입증하기 전에는 사용할 수 없다.
        if assessment.network {
            return Err(ToolPlanUnavailable::UnqualifiedPlatform);
        }
        let mut roots = vec![pin(&self.workspace, &self.workspace, true)?];
        for support in ["/usr", "/bin", "/sbin", "/lib", "/lib64"] {
            let path = Path::new(support);
            if path.exists() {
                roots.push(pin(path, path, false)?);
            }
        }
        for (source, destination) in &self.support_roots {
            roots.push(pin(source, destination, false)?);
        }
        if let Payload::Configured(command, _) = &payload {
            for (source, destination) in command.read_only_artifacts() {
                let destination_exposed = roots
                    .iter()
                    .any(|root| destination.starts_with(&root.destination));
                let source_exposed = roots.iter().any(|root| {
                    source
                        .strip_prefix(&root.destination)
                        .ok()
                        .is_some_and(|relative| {
                            root.source
                                .join(relative)
                                .canonicalize()
                                .is_ok_and(|resolved| resolved == source)
                        })
                });
                if !destination_exposed {
                    roots.push(pin(&source, &destination, false)?);
                } else if !source_exposed {
                    roots.push(pin(&source, &source, false)?);
                }
            }
        }
        let git_roots = git_metadata_roots(&self.workspace)?;
        let mut requested = Vec::new();
        for path in git_roots {
            if !path.starts_with(&self.workspace) {
                let writable = assessment.git_write;
                if writable {
                    requested.push(path.clone());
                }
                roots.push(pin(&path, &path, writable)?);
            }
        }
        for directory in &assessment.git_directories {
            let path = resolve(&self.workspace.join(directory))?;
            if !roots.iter().any(|root| path.starts_with(&root.source)) {
                return Err(ToolPlanUnavailable::UnresolvedScope);
            }
        }
        for target in &assessment.writes {
            if target == "/dev/null" {
                continue;
            }
            let path = resolve(&self.workspace.join(target))?;
            if !path.starts_with(&self.workspace)
                && !path.starts_with(PRIVATE_TMP)
                && !path.starts_with(PRIVATE_HOME)
            {
                // 없는 파일의 부모를 암묵적으로 grant하지 않는다.
                if !path.exists() || !path.is_file() {
                    return Err(ToolPlanUnavailable::UnresolvedScope);
                }
                requested.push(path.clone());
                roots.push(pin(&path, &path, true)?);
            }
        }
        // 직접 여는 파일 grant는 이미 아는 credential inode도 거부한다. 전체 alias 탐색은 아니다.
        let protected_files = self
            .secret_endpoints
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .filter(|meta| meta.is_file())
            .map(|meta| (meta.dev(), meta.ino()))
            .collect::<Vec<_>>();
        for root in &roots {
            let meta = root
                .file
                .metadata()
                .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
            if meta.is_file() && protected_files.contains(&(meta.dev(), meta.ino())) {
                return Err(ToolPlanUnavailable::ProtectedScope);
            }
        }
        let mut hidden = Vec::new();
        let mut ancestor_roots = Vec::new();
        for (path, directory) in self
            .secret_roots
            .iter()
            .map(|path| (path, true))
            .chain(self.secret_endpoints.iter().map(|path| (path, false)))
        {
            for root in &roots {
                if root.source.starts_with(path) {
                    return Err(ToolPlanUnavailable::ProtectedScope);
                }
                if path.starts_with(&root.source) {
                    let relative = path
                        .strip_prefix(&root.source)
                        .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
                    let ancestors = capture_ancestors(path, &root.source)?;
                    for (ancestor, expected) in &ancestors {
                        let expected = expected.ok_or(ToolPlanUnavailable::UnresolvedScope)?;
                        let relative = ancestor
                            .strip_prefix(&root.source)
                            .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
                        let pinned =
                            pin(ancestor, &root.destination.join(relative), root.writable)?;
                        let meta = pinned
                            .file
                            .metadata()
                            .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
                        if !meta.is_dir() || (meta.dev(), meta.ino()) != expected {
                            return Err(ToolPlanUnavailable::UnresolvedScope);
                        }
                        ancestor_roots.push(pinned);
                    }
                    hidden.push(HiddenPath {
                        path: root.destination.join(relative),
                        directory,
                        source: path.clone(),
                        ancestors,
                    });
                }
            }
        }
        for ancestor in ancestor_roots {
            if !roots
                .iter()
                .any(|root| root.destination == ancestor.destination)
            {
                roots.push(ancestor);
            }
        }
        roots.sort_by(|a, b| {
            a.destination
                .components()
                .count()
                .cmp(&b.destination.components().count())
                .then_with(|| a.destination.cmp(&b.destination))
        });
        hidden.sort_by(|a, b| a.path.cmp(&b.path));
        hidden.dedup_by(|a, b| a.path == b.path);
        let mut reduced: Vec<HiddenPath> = Vec::new();
        for candidate in hidden {
            if !reduced
                .iter()
                .any(|parent| parent.directory && candidate.path.starts_with(&parent.path))
            {
                reduced.push(candidate);
            }
        }
        let hidden = reduced;
        let mut environment = vec![
            ("PATH".to_owned(), "/usr/local/bin:/usr/bin:/bin".to_owned()),
            ("HOME".to_owned(), PRIVATE_HOME.to_owned()),
            ("TMPDIR".to_owned(), PRIVATE_TMP.to_owned()),
            ("LANG".to_owned(), "C.UTF-8".to_owned()),
            ("CARGO_HOME".to_owned(), format!("{PRIVATE_HOME}/.cargo")),
            ("RUSTUP_AUTO_INSTALL".to_owned(), "0".to_owned()),
        ];
        environment.extend(self.git_identity.clone());
        for (source, destination) in &self.support_roots {
            if source == destination && source.file_name().is_some_and(|name| name == "bin") {
                environment[0].1 = format!("{}:{}", utf8(source)?, environment[0].1);
            }
            if source == destination && source.join("toolchains").is_dir() {
                environment.push(("RUSTUP_HOME".to_owned(), utf8(source)?.to_owned()));
            }
        }
        let root_manifest = roots
            .iter()
            .map(|root| {
                let meta = root
                    .file
                    .metadata()
                    .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
                Ok(
                    json!({"source": utf8(&root.source)?, "destination": utf8(&root.destination)?,
                "device":meta.dev(), "inode":meta.ino(), "writable":root.writable}),
                )
            })
            .collect::<Result<Vec<_>, ToolPlanUnavailable>>()?;
        let hidden_manifest = hidden
            .iter()
            .map(|hidden| Ok(json!({"path":utf8(&hidden.path)?,"directory":hidden.directory,"ancestors":hidden.ancestors.iter().map(|(path,identity)| (path.to_string_lossy(),identity)).collect::<Vec<_>>()})))
            .collect::<Result<Vec<_>, ToolPlanUnavailable>>()?;
        let request_digest = Sha256::digest(request.call.normalized_arguments()).into();
        let identity = Sha256::digest(serde_json::to_vec(&json!({
            "profile":PROFILE,"classifier":risk::REVISION,"platform":"linux-bwrap-private-net-v2",
            "workspace":utf8(&self.workspace)?,"cwd":utf8(&self.workspace)?,
            "arguments":request.call.normalized_arguments(),"tool":request.call.definition().id().as_str(),
            "configured_launch":match &payload { Payload::Configured(command, _) => command.launch_manifest(), Payload::Shell(_) => serde_json::Value::Null },
            "roots":root_manifest,"hidden":hidden_manifest,"environment":environment,
            "descriptors":[0,1,2],"network":false,"private_unix_ipc":true,
            "process_scope":"private-pid-namespace-and-process-group",
            "maximum_output_bytes":request.maximum_output_bytes,
            "maximum_retained_output_bytes":request.maximum_retained_output_bytes,
            "absolute_execution_timeout":request.absolute_execution_timeout,
        })).map_err(|_| ToolPlanUnavailable::UnresolvedScope)?).into();
        let scope = format!(
            "Workspace: {}\nNetwork: disabled{}{}",
            json!(utf8(&self.workspace)?),
            if assessment.approval {
                "\nRecognized destructive command"
            } else {
                ""
            },
            requested
                .iter()
                .map(|path| format!(
                    "\nAdditional writable path: {}",
                    json!(path.to_string_lossy())
                ))
                .collect::<String>()
        );
        Ok((
            ConfinedPlan {
                workspace: self.workspace.clone(),
                roots,
                hidden,
                environment,
                payload,
                request_digest,
                turn: request.turn,
                call_id: request.call.call_id().to_owned(),
                definition: request.call.definition().clone(),
                maximum_output_bytes: request.maximum_output_bytes,
                maximum_retained_output_bytes: request.maximum_retained_output_bytes,
                absolute_execution_timeout: request.absolute_execution_timeout,
            },
            configured || assessment.approval || !requested.is_empty(),
            scope,
            identity,
        ))
    }
}

impl ConfinedPlan {
    pub(super) fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub(super) fn matches(&self, request: &ToolExecutionRequest) -> bool {
        self.maximum_output_bytes == request.maximum_output_bytes
            && self.maximum_retained_output_bytes == request.maximum_retained_output_bytes
            && self.absolute_execution_timeout == request.absolute_execution_timeout
            && self.turn == request.turn
            && self.call_id == request.call.call_id()
            && self.definition == *request.call.definition()
            && self.request_digest == Sha256::digest(request.call.normalized_arguments()).as_slice()
    }

    pub(super) fn launch(
        &self,
        cancelled: &AtomicBool,
        deadline: Option<Instant>,
    ) -> Result<(Command, Vec<u8>), ToolExecutionError> {
        #[cfg(target_os = "linux")]
        {
            linux::launch(self, cancelled, deadline)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (cancelled, deadline);
            Err(error())
        }
    }
}

fn platform_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::available()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn utf8(path: &Path) -> Result<&str, ToolPlanUnavailable> {
    path.to_str().ok_or(ToolPlanUnavailable::UnresolvedScope)
}

fn pin(
    source: &Path,
    destination: &Path,
    writable: bool,
) -> Result<PinnedRoot, ToolPlanUnavailable> {
    let source = source
        .canonicalize()
        .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&source)
        .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
    Ok(PinnedRoot {
        source,
        destination: destination.to_owned(),
        file,
        writable,
    })
}

fn resolve(path: &Path) -> Result<PathBuf, ToolPlanUnavailable> {
    if !path.is_absolute() {
        return Err(ToolPlanUnavailable::UnresolvedScope);
    }
    let mut existing = path;
    let mut missing = Vec::new();
    while !existing.exists() {
        let name = existing
            .file_name()
            .ok_or(ToolPlanUnavailable::UnresolvedScope)?;
        missing.push(name.to_owned());
        existing = existing
            .parent()
            .ok_or(ToolPlanUnavailable::UnresolvedScope)?;
    }
    let mut result = existing
        .canonicalize()
        .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
    for component in missing.into_iter().rev() {
        result.push(component);
    }
    Ok(result)
}

// Git의 경로 지시자는 FIFO나 장치가 아닌 고정 크기 일반 파일에서만 읽는다.
fn read_git_pointer(path: &Path) -> Result<Vec<u8>, ToolPlanUnavailable> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
    if !file
        .metadata()
        .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?
        .is_file()
    {
        return Err(ToolPlanUnavailable::UnresolvedScope);
    }
    let mut bytes = Vec::new();
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
    if bytes.len() > 4096 {
        return Err(ToolPlanUnavailable::UnresolvedScope);
    }
    Ok(bytes)
}

fn git_metadata_roots(workspace: &Path) -> Result<Vec<PathBuf>, ToolPlanUnavailable> {
    let entry = workspace.join(".git");
    if !entry.exists() {
        return Ok(Vec::new());
    }
    let directory = if entry.is_dir() {
        resolve(&entry)?
    } else {
        let bytes = read_git_pointer(&entry)?;
        let text = str::from_utf8(&bytes).map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
        let pointer = text
            .trim()
            .strip_prefix("gitdir: ")
            .filter(|path| !path.is_empty())
            .ok_or(ToolPlanUnavailable::UnresolvedScope)?;
        resolve(&workspace.join(pointer))?
    };
    if !directory.is_dir() {
        return Err(ToolPlanUnavailable::UnresolvedScope);
    }
    let common_file = directory.join("commondir");
    let mut roots = vec![directory.clone()];
    if common_file.exists() {
        let bytes = read_git_pointer(&common_file)?;
        let text = str::from_utf8(&bytes).map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
        if text.trim().is_empty() {
            return Err(ToolPlanUnavailable::UnresolvedScope);
        }
        let common = resolve(&directory.join(text.trim()))?;
        if !common.is_dir() {
            return Err(ToolPlanUnavailable::UnresolvedScope);
        }
        if common != directory {
            roots.push(common);
        }
    }
    Ok(roots)
}

fn error() -> ToolExecutionError {
    ToolExecutionError::new("workspace command confinement is unavailable")
}

fn freeze_exclusions(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, ToolPlanUnavailable> {
    let mut frozen = Vec::new();
    for path in paths {
        if !path.is_absolute() {
            return Err(ToolPlanUnavailable::UnresolvedScope);
        }
        frozen.push(path.clone());
        frozen.push(resolve(&path)?);
    }
    frozen.sort();
    frozen.dedup();
    Ok(frozen)
}

fn capture_ancestors(
    path: &Path,
    admitted: &Path,
) -> Result<Vec<AncestorIdentity>, ToolPlanUnavailable> {
    let mut ancestors = Vec::new();
    let mut current = path;
    while current.starts_with(admitted) && current != admitted {
        match fs::symlink_metadata(current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(ToolPlanUnavailable::UnresolvedScope);
            },
            Ok(meta) if current != path => {
                ancestors.push((current.to_owned(), Some((meta.dev(), meta.ino()))))
            },
            Ok(_) => {},
            Err(error) if error.kind() == ErrorKind::NotFound && current != path => {
                ancestors.push((current.to_owned(), None))
            },
            Err(error) if error.kind() == ErrorKind::NotFound => {},
            Err(_) => return Err(ToolPlanUnavailable::UnresolvedScope),
        }
        current = current
            .parent()
            .ok_or(ToolPlanUnavailable::UnresolvedScope)?;
    }
    Ok(ancestors)
}

impl HiddenPath {
    fn verify(&self) -> Result<(), ToolExecutionError> {
        if fs::symlink_metadata(&self.source)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(error());
        }
        for (path, expected) in &self.ancestors {
            match fs::symlink_metadata(path) {
                Ok(meta)
                    if meta.is_dir()
                        && !meta.file_type().is_symlink()
                        && expected.is_none_or(|expected| expected == (meta.dev(), meta.ino())) => {
                },
                Err(error) if error.kind() == ErrorKind::NotFound && expected.is_none() => {},
                _ => return Err(error()),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

// 원시 Git 설정이나 credential helper를 전달하지 않고 공개 작성자 이름/주소만 고정한다.
fn read_git_identity(workspace: &Path, home: &Path, xdg: Option<&Path>) -> Vec<(String, String)> {
    let mut identity = Vec::new();
    for (key, suffix) in [("user.name", "NAME"), ("user.email", "EMAIL")] {
        let mut query = Command::new("/usr/bin/git");
        query
            .current_dir(workspace)
            .env_clear()
            .env("HOME", home)
            .args(["config", "--null", "--get", key])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(xdg) = xdg {
            query.env("XDG_CONFIG_HOME", xdg);
        }
        let Ok(mut child) = query.spawn() else {
            continue;
        };
        let start = Instant::now();
        let passed = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) if start.elapsed() < Duration::from_secs(2) => {
                    thread::sleep(Duration::from_millis(10))
                },
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                },
            }
        };
        if !passed {
            continue;
        }
        let mut bytes = Vec::new();
        let Some(stdout) = child.stdout.take() else {
            continue;
        };
        if stdout.take(1026).read_to_end(&mut bytes).is_err() || bytes.len() > 1025 {
            continue;
        }
        if bytes.pop() != Some(0) {
            continue;
        }
        let Ok(value) = String::from_utf8(bytes) else {
            continue;
        };
        if value.is_empty() || value.chars().any(char::is_control) {
            continue;
        }
        for role in ["AUTHOR", "COMMITTER"] {
            identity.push((format!("GIT_{role}_{suffix}"), value.clone()));
        }
    }
    identity
}
