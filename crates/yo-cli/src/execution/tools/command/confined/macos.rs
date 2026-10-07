//! 이름으로 고정한 Seatbelt 범위와 호출별 물리 디렉터리의 수명만 소유한다.

#[cfg(not(target_os = "macos"))]
use std::env;
use std::{
    collections::BTreeSet,
    fs, io,
    os::unix::fs::{DirBuilderExt, MetadataExt, symlink},
    path::{Path, PathBuf},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};
#[cfg(target_os = "macos")]
use std::{process::Command, ptr, sync::atomic::AtomicBool, time::Instant};

use serde_json::{Value, json};
use yo_core::{ToolExecutionError, ToolPlanUnavailable};

use super::{HiddenPath, PRIVATE_HOME, PinnedRoot, error, utf8};

// 실제 deny-default 정책 qualification을 마친 release만 여기에 추가한다.
// 이전 lifecycle-only 진단은 어떤 production release도 승인하지 않는다.
const QUALIFIED_RELEASES: &[(&str, &str, &str, &str)] = &[];
const FILE_KIND_MASK: u32 = 0o170000;

pub(super) fn available() -> bool {
    #[cfg(target_os = "macos")]
    {
        if QUALIFIED_RELEASES.is_empty() {
            return false;
        }
        let metadata = [
            "kern.osproductversion",
            "kern.osrelease",
            "kern.osversion",
            "hw.machine",
        ]
        .map(system_value);
        QUALIFIED_RELEASES
            .iter()
            .any(|&(product, kernel, build, architecture)| {
                metadata
                    == [
                        Some(product.to_owned()),
                        Some(kernel.to_owned()),
                        Some(build.to_owned()),
                        Some(architecture.to_owned()),
                    ]
            })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = QUALIFIED_RELEASES;
        false
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn system_value(name: &str) -> Option<String> {
    use std::ffi::CString;
    let name = CString::new(name).ok()?;
    let mut bytes = [0u8; 256];
    let mut length = bytes.len();
    // SAFETY: 이름은 NUL 종료되고 출력 버퍼 크기는 syscall에 정확히 전달한다.
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            &mut length,
            ptr::null_mut(),
            0,
        )
    };
    if status != 0 || length == 0 || length > bytes.len() || bytes[length - 1] != 0 {
        return None;
    }
    String::from_utf8(bytes[..length - 1].to_vec()).ok()
}

#[derive(Clone)]
struct PathIdentity {
    path: PathBuf,
    expected: Option<(u64, u64, u32)>,
}

impl PathIdentity {
    fn capture(path: &Path) -> Result<Self, ToolPlanUnavailable> {
        let expected = match fs::symlink_metadata(path) {
            Ok(metadata) => Some((
                metadata.dev(),
                metadata.ino(),
                metadata.mode() & FILE_KIND_MASK,
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(_) => return Err(ToolPlanUnavailable::UnresolvedScope),
        };
        Ok(Self {
            path: path.to_owned(),
            expected,
        })
    }

    fn unchanged(&self) -> bool {
        Self::capture(&self.path).is_ok_and(|current| current.expected == self.expected)
    }
}

struct PrivateDirectory {
    path: PathBuf,
    identity: PathIdentity,
    cleanup_on_drop: bool,
}

impl PrivateDirectory {
    fn new() -> Result<Self, ToolPlanUnavailable> {
        #[cfg(target_os = "macos")]
        let base = PathBuf::from("/private/tmp");
        #[cfg(not(target_os = "macos"))]
        let base = env::temp_dir();
        let base = base
            .canonicalize()
            .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
        let id = yo_core::SessionId::new().map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
        let path = base.join(format!("yo-command-{id}"));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
        let identity = PathIdentity::capture(&path)?;
        Ok(Self {
            path,
            identity,
            cleanup_on_drop: true,
        })
    }
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        // 다른 프로세스가 이름을 교체했다면 그 디렉터리를 재귀 삭제하지 않는다.
        if self.cleanup_on_drop && self.identity.unchanged() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[derive(Default)]
pub(super) struct CleanupHooks {
    #[cfg(test)]
    pub(super) entered: Option<mpsc::SyncSender<()>>,
    #[cfg(test)]
    pub(super) resume: Option<mpsc::Receiver<()>>,
    #[cfg(test)]
    pub(super) completed: Option<mpsc::SyncSender<()>>,
}

type CleanupWorker = (mpsc::Receiver<Result<(), ()>>, JoinHandle<()>);

pub(super) struct Plan {
    directory: PrivateDirectory,
    home: PathBuf,
    temporary: PathBuf,
    identities: Vec<PathIdentity>,
    policy: String,
    parameters: Vec<(String, String)>,
}

impl Plan {
    pub(super) fn cleanup(self, grace: Duration) -> Result<(), ()> {
        let (receiver, _worker) = self.start_cleanup(CleanupHooks::default())?;
        receiver.recv_timeout(grace).map_err(|_| ())?
    }

    fn start_cleanup(mut self, hooks: CleanupHooks) -> Result<CleanupWorker, ()> {
        // spawn 실패로 closure가 drop되어도 재귀 삭제를 호출자에서 다시 시작하지 않는다.
        self.directory.cleanup_on_drop = false;
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("yo-command-private-cleanup".to_owned())
            .spawn(move || {
                #[cfg(not(test))]
                let _ = hooks;
                #[cfg(test)]
                {
                    if let Some(entered) = hooks.entered {
                        let _ = entered.send(());
                    }
                    if let Some(resume) = hooks.resume {
                        let _ = resume.recv();
                    }
                }
                let result = if self.directory.identity.unchanged() {
                    fs::remove_dir_all(&self.directory.path).map_err(|_| ())
                } else {
                    Err(())
                };
                let _ = sender.send(result);
                #[cfg(test)]
                if let Some(completed) = hooks.completed {
                    let _ = completed.send(());
                }
            })
            .map_err(|_| ())?;
        Ok((receiver, worker))
    }

    #[cfg(test)]
    pub(super) fn test_cleanup(
        self,
        grace: Duration,
        hooks: CleanupHooks,
    ) -> Result<(Result<(), ()>, JoinHandle<()>), ()> {
        let (receiver, worker) = self.start_cleanup(hooks)?;
        Ok((
            receiver
                .recv_timeout(grace)
                .map_err(|_| ())
                .and_then(|result| result),
            worker,
        ))
    }

    pub(super) fn new() -> Result<Self, ToolPlanUnavailable> {
        let directory = PrivateDirectory::new()?;
        let home = directory.path.join("home");
        let temporary = directory.path.join("tmp");
        for path in [&home, &temporary, &home.join(".cargo")] {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(path)
                .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
        }
        let identities = [&directory.path, &home, &temporary, &home.join(".cargo")]
            .into_iter()
            .map(|path| PathIdentity::capture(path))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            directory,
            home,
            temporary,
            identities,
            policy: String::new(),
            parameters: Vec::new(),
        })
    }

    pub(super) fn home(&self) -> &Path {
        &self.home
    }
    pub(super) fn temporary(&self) -> &Path {
        &self.temporary
    }

    pub(super) fn destination(
        &mut self,
        source: &Path,
        destination: &Path,
    ) -> Result<PathBuf, ToolPlanUnavailable> {
        let Ok(relative) = destination.strip_prefix(PRIVATE_HOME) else {
            return Ok(destination.to_owned());
        };
        // 가상 mount 대신 두 Cargo source cache만 호출별 HOME에 연결한다.
        if relative != Path::new(".cargo/registry") && relative != Path::new(".cargo/git") {
            return Err(ToolPlanUnavailable::UnsupportedProfile);
        }
        let source = source
            .canonicalize()
            .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
        let destination = self.home.join(relative);
        symlink(source, &destination).map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
        self.identities.push(PathIdentity::capture(&destination)?);
        Ok(destination)
    }

    pub(super) fn freeze(
        &mut self,
        roots: &[PinnedRoot],
        hidden: &[HiddenPath],
        secret_roots: &[PathBuf],
        secret_endpoints: &[PathBuf],
    ) -> Result<(), ToolPlanUnavailable> {
        let mut exclusions = secret_roots
            .iter()
            .chain(secret_endpoints)
            .cloned()
            .collect::<Vec<_>>();
        exclusions.sort();
        exclusions.dedup();
        for path in roots
            .iter()
            .map(|root| &root.source)
            .chain(exclusions.iter())
        {
            for ancestor in path.ancestors() {
                self.identities.push(PathIdentity::capture(ancestor)?);
            }
        }
        self.identities.sort_by(|a, b| a.path.cmp(&b.path));
        self.identities.dedup_by(|a, b| a.path == b.path);
        self.policy = "(version 1)\n(deny default)\n(allow syscall-unix)\n(allow process-fork)\n(allow process-exec)\n(allow signal (target same-sandbox))\n(allow process-info* (target same-sandbox))\n".to_owned();
        // CPU/OS 조회만 허용하며 credential/session Mach 서비스는 하나도 grant하지 않는다.
        self.policy.push_str("(allow sysctl-read (sysctl-name \"hw.ncpu\") (sysctl-name \"hw.activecpu\") (sysctl-name \"hw.logicalcpu\") (sysctl-name \"hw.logicalcpu_max\") (sysctl-name \"hw.physicalcpu\") (sysctl-name \"hw.memsize\") (sysctl-name \"hw.pagesize\") (sysctl-name \"hw.machine\") (sysctl-name \"kern.osrelease\") (sysctl-name \"kern.osversion\") (sysctl-name \"kern.osproductversion\"))\n");
        let exclusion_parameters = exclusions
            .iter()
            .map(|path| self.parameter(path))
            .collect::<Result<Vec<_>, _>>()?;
        let exclusion_filters = exclusion_parameters.iter().map(|name| format!(" (require-not (literal (param \"{name}\"))) (require-not (subpath (param \"{name}\")))")).collect::<String>();
        for root in roots {
            let metadata = root
                .file
                .metadata()
                .map_err(|_| ToolPlanUnavailable::UnresolvedScope)?;
            let name = self.parameter(&root.source)?;
            let predicate = if metadata.is_dir() {
                format!("(require-any (literal (param \"{name}\")) (subpath (param \"{name}\")))")
            } else {
                format!("(literal (param \"{name}\"))")
            };
            self.policy.push_str(&format!("(allow file-read* file-map-executable (require-all {predicate}{exclusion_filters}))\n"));
            if root.writable {
                let mut readonly_filters = String::new();
                for readonly in roots.iter().filter(|candidate| {
                    !candidate.writable && candidate.source.starts_with(&root.source)
                }) {
                    let readonly = self.parameter(&readonly.source)?;
                    readonly_filters.push_str(&format!(" (require-not (literal (param \"{readonly}\"))) (require-not (subpath (param \"{readonly}\")))"));
                }
                self.policy.push_str(&format!("(allow file-write* (require-all {predicate}{exclusion_filters}{readonly_filters}))\n"));
            }
        }
        let private = self.directory.path.clone();
        let name = self.parameter(&private)?;
        self.policy.push_str(&format!("(allow file-read* file-write* file-map-executable (require-all (require-any (literal (param \"{name}\")) (subpath (param \"{name}\"))){exclusion_filters}))\n"));
        self.policy.push_str(&format!(
            "(deny file-write-unlink (literal (param \"{name}\")))\n"
        ));
        // 열기 위한 조상 metadata만 제공한다. 조상 디렉터리 전체 읽기는 grant하지 않는다.
        let ancestors = roots
            .iter()
            .flat_map(|root| root.source.ancestors().skip(1))
            .chain(private.ancestors().skip(1))
            .map(Path::to_owned)
            .collect::<BTreeSet<_>>();
        for path in ancestors {
            let name = self.parameter(&path)?;
            self.policy.push_str(&format!("(allow file-read-metadata (require-all (literal (param \"{name}\")){exclusion_filters}))\n"));
        }
        for hidden in hidden {
            for ancestor in hidden.source.ancestors().skip(1) {
                if roots
                    .iter()
                    .any(|root| root.writable && ancestor.starts_with(&root.source))
                {
                    let name = self.parameter(ancestor)?;
                    self.policy.push_str(&format!(
                        "(deny file-write-unlink (literal (param \"{name}\")))\n"
                    ));
                }
            }
        }
        self.policy.push_str("(allow file-read* (literal \"/dev/null\") (literal \"/dev/urandom\") (literal \"/dev/random\"))\n(allow file-write-data (require-all (literal \"/dev/null\") (vnode-type CHARACTER-DEVICE)))\n");
        Ok(())
    }

    fn parameter(&mut self, path: &Path) -> Result<String, ToolPlanUnavailable> {
        let name = format!("P{}", self.parameters.len());
        self.parameters.push((name.clone(), utf8(path)?.to_owned()));
        Ok(name)
    }

    pub(super) fn manifest(&self) -> Result<Value, ToolPlanUnavailable> {
        Ok(json!({
            "private_root":utf8(&self.directory.path)?, "home":utf8(&self.home)?, "tmp":utf8(&self.temporary)?,
            "identities":self.identities.iter().map(|identity| (&identity.path, identity.expected)).collect::<Vec<_>>(),
            "policy":self.policy, "parameters":self.parameters, "mach_lookups":[], "host_unix_connect":false,
        }))
    }

    pub(super) fn verify(&self, roots: &[PinnedRoot]) -> Result<(), ToolExecutionError> {
        if self.policy.is_empty() || self.identities.iter().any(|identity| !identity.unchanged()) {
            return Err(error());
        }
        for root in roots {
            let pinned = root.file.metadata().map_err(|_| error())?;
            let named = fs::symlink_metadata(&root.source).map_err(|_| error())?;
            if (pinned.dev(), pinned.ino(), pinned.mode() & FILE_KIND_MASK)
                != (named.dev(), named.ino(), named.mode() & FILE_KIND_MASK)
            {
                return Err(error());
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub(super) fn launch(
    plan: &super::ConfinedPlan,
    cancelled: &AtomicBool,
    deadline: Option<Instant>,
) -> Result<(Command, Vec<u8>), ToolExecutionError> {
    use std::{
        os::unix::process::CommandExt,
        process::{Command, Stdio},
        sync::atomic::Ordering,
        time::Instant,
    };
    let macos = plan.macos.as_ref().ok_or_else(error)?;
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command.arg("-p").arg(&macos.policy);
    for (name, value) in &macos.parameters {
        command.arg(format!("-D{name}={value}"));
    }
    command.arg("--");
    let stdin = match &plan.payload {
        super::Payload::Shell(text) => {
            command.args(["/bin/sh", "-c", text]);
            Vec::new()
        },
        super::Payload::Configured(prepared, arguments) => {
            let verified =
                prepared.verify_for_launch(&mut || cancelled.load(Ordering::Acquire), deadline)?;
            command.arg(verified.executable).args(verified.args);
            let mut bytes = arguments.clone();
            bytes.push(b'\n');
            bytes
        },
    };
    command
        .current_dir(&plan.workspace)
        .env_clear()
        .envs(plan.environment.iter().map(|(name, value)| (name, value)))
        .process_group(0)
        .stdin(if stdin.is_empty() {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_descriptors(&mut command);
    for hidden in &plan.hidden {
        hidden.verify()?;
    }
    macos.verify(&plan.roots)?;
    if cancelled.load(Ordering::Acquire)
        || deadline.is_some_and(|deadline| Instant::now() >= deadline)
    {
        return Err(error());
    }
    Ok((command, stdin))
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn configure_descriptors(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // fork 전 한 번 할당한다. child 단일 스레드의 proc_pidinfo는 __proc_info syscall만 호출한다.
    let mut entries = vec![
        libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0
        };
        65_536
    ];
    unsafe {
        // SAFETY: fork 뒤에는 미리 할당한 버퍼와 syscall만 사용한다. 닫지 않고
        // CLOEXEC를 설정하므로 Rust exec 오류 pipe도 오류 보고까지 살아 있다.
        command.pre_exec(move || {
            let capacity = entries.len() * size_of::<libc::proc_fdinfo>();
            let bound =
                libc::proc_pidinfo(libc::getpid(), libc::PROC_PIDLISTFDS, 0, ptr::null_mut(), 0);
            if bound <= 0
                || bound as usize > capacity
                || bound as usize % size_of::<libc::proc_fdinfo>() != 0
            {
                return Err(io::Error::from_raw_os_error(libc::EOVERFLOW));
            }
            let size = libc::proc_pidinfo(
                libc::getpid(),
                libc::PROC_PIDLISTFDS,
                0,
                entries.as_mut_ptr().cast(),
                capacity as i32,
            );
            if size <= 0 || size > bound || size as usize % size_of::<libc::proc_fdinfo>() != 0 {
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            for entry in &entries[..size as usize / size_of::<libc::proc_fdinfo>()] {
                let fd = entry.proc_fd;
                if fd > 2 {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) < 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
            }
            Ok(())
        });
    }
}

#[cfg(all(target_os = "macos", test))]
pub(super) fn test_exec_failure(path: &Path) -> io::Error {
    let mut command = Command::new(path);
    configure_descriptors(&mut command);
    command.spawn().unwrap_err()
}
