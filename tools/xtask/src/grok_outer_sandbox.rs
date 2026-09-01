use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) const OUTER_SANDBOX_REVIEW_PROFILE: &str = "yo-bwrap-read-only/v1alpha1";
const OUTER_SANDBOX_REVIEW_ENV: &str = "YO_GROK_REVIEW_ISOLATION";
const OUTER_SANDBOX_SENTINEL: &str = "/run/yo-grok-review-sandbox";

pub(crate) const NATIVE_SANDBOX_REVIEW_PROFILE: &str = "grok-native-read-only/v1alpha1";

const SYSTEM_RUNTIME_SOCKETS: &[&str] =
    &["/var/lib/libvirt/libvirt-sock", "/var/lib/lxd/unix.socket"];

/// Builds the only Yo-owned Grok fallback: a read-only root with narrowly writable
/// host state, temporary directories, and the exact Session repository.
pub(crate) fn command(
    program: &Path,
    arguments: &[String],
    working_directory: &Path,
    session_repository: &Path,
) -> Result<Command, String> {
    require_delivery_arguments(arguments)?;
    command_unchecked(program, arguments, working_directory, session_repository)
}

pub(crate) fn probe_command(program: &Path, working_directory: &Path) -> Result<Command, String> {
    let state = grok_state_directory()?;
    let arguments = [
        "--sandbox",
        "off",
        "--permission-mode",
        "dontAsk",
        "--tools",
        "",
        "--no-subagents",
        "--disable-web-search",
        "agent",
        "stdio",
    ]
    .map(str::to_owned);
    command_unchecked(program, &arguments, working_directory, &state)
}

fn command_unchecked(
    program: &Path,
    arguments: &[String],
    working_directory: &Path,
    session_repository: &Path,
) -> Result<Command, String> {
    require_absolute_existing_file(program, "outer-sandbox program")?;
    require_absolute_existing_directory(working_directory, "outer-sandbox working directory")?;
    let state = grok_state_directory()?;
    require_absolute_existing_directory(&state, "Grok state directory")?;

    if !session_repository.is_absolute() {
        return Err("outer-sandbox Session repository must be absolute".to_owned());
    }
    fs::create_dir_all(session_repository).map_err(|error| {
        format!(
            "cannot prepare outer-sandbox Session repository {}: {error}",
            session_repository.display()
        )
    })?;
    require_absolute_existing_directory(session_repository, "outer-sandbox Session repository")?;

    let bwrap = resolve_executable("bwrap")?;
    let state = fs::canonicalize(&state).map_err(|error| {
        format!(
            "cannot resolve outer-sandbox writable path {}: {error}",
            state.display()
        )
    })?;
    let session_repository = fs::canonicalize(session_repository).map_err(|error| {
        format!(
            "cannot resolve outer-sandbox writable path {}: {error}",
            session_repository.display()
        )
    })?;
    build_command(
        &bwrap,
        program,
        arguments,
        working_directory,
        &state,
        &session_repository,
        &runtime_sockets()?,
    )
}

fn require_delivery_arguments(arguments: &[String]) -> Result<(), String> {
    let fresh = arguments == ["-p", "--model", "host:grok", "--sandbox", "read-only"];
    let resume = matches!(arguments, [print, resume, session]
        if print == "-p"
            && resume == "--resume"
            && !session.is_empty()
            && session.len() <= 256
            && session.bytes().all(|byte| byte.is_ascii_graphic()));
    if fresh || resume {
        Ok(())
    } else {
        Err(
            "Grok outer sandbox accepts only the exact immutable review fresh or resume argv"
                .to_owned(),
        )
    }
}

fn build_command(
    bwrap: &Path,
    program: &Path,
    arguments: &[String],
    working_directory: &Path,
    state: &Path,
    session_repository: &Path,
    runtime_sockets: &[PathBuf],
) -> Result<Command, String> {
    let mut command = Command::new(bwrap);
    command.args([
        "--die-with-parent",
        "--new-session",
        "--unshare-user",
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--cap-drop",
        "ALL",
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
        "--tmpfs",
        "/var/tmp",
        "--tmpfs",
        "/run",
        "--ro-bind",
        "/usr",
        OUTER_SANDBOX_SENTINEL,
    ]);

    let mut writable = BTreeSet::new();
    for path in [state, session_repository] {
        if working_directory.starts_with(path) {
            return Err(format!(
                "outer-sandbox working directory must not be inside writable path {}",
                path.display()
            ));
        }
        if writable.insert(path) {
            command.arg("--bind").arg(path).arg(path);
        }
    }

    for socket in runtime_sockets {
        command.arg("--ro-bind").arg("/dev/null").arg(socket);
    }

    command
        .arg("--chdir")
        .arg(working_directory)
        .arg("--setenv")
        .arg(OUTER_SANDBOX_REVIEW_ENV)
        .arg(OUTER_SANDBOX_REVIEW_PROFILE)
        .arg("--")
        .arg(program)
        .args(arguments);
    Ok(command)
}

fn grok_state_directory() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("GROK_HOME") {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            return Ok(path);
        }
        return Err("GROK_HOME must be absolute for outer-sandbox review".to_owned());
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| "HOME is unavailable for outer-sandbox review".to_owned())?;
    let home = PathBuf::from(home);
    if !home.is_absolute() {
        return Err("HOME must be absolute for outer-sandbox review".to_owned());
    }
    Ok(home.join(".grok"))
}

fn runtime_sockets() -> Result<Vec<PathBuf>, String> {
    let mut candidates = SYSTEM_RUNTIME_SOCKETS
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if home.is_absolute() {
            candidates.extend([
                home.join(".docker/desktop/docker.sock"),
                home.join(".docker/run/docker.sock"),
            ]);
        }
    }

    let mut sockets = BTreeSet::new();
    for candidate in candidates {
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "outer-sandbox runtime endpoint must not be a symlink: {}",
                    candidate.display()
                ));
            },
            Ok(metadata) if metadata.file_type().is_dir() => {
                return Err(format!(
                    "outer-sandbox runtime endpoint unexpectedly names a directory: {}",
                    candidate.display()
                ));
            },
            Ok(_) => {
                sockets.insert(candidate);
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => {
                return Err(format!(
                    "cannot inspect outer-sandbox runtime endpoint {}: {error}",
                    candidate.display()
                ));
            },
        }
    }
    Ok(sockets.into_iter().collect())
}

fn resolve_executable(name: &str) -> Result<PathBuf, String> {
    let path = std::env::var_os("PATH")
        .ok_or_else(|| "PATH is unavailable for outer-sandbox review".to_owned())?;
    for directory in std::env::split_paths(&path) {
        if !directory.is_absolute() {
            continue;
        }
        let candidate = directory.join(name);
        if fs::metadata(&candidate).is_ok_and(|metadata| metadata.is_file()) {
            return fs::canonicalize(&candidate).map_err(|error| {
                format!(
                    "cannot resolve outer-sandbox executable {}: {error}",
                    candidate.display()
                )
            });
        }
    }
    Err(format!(
        "outer-sandbox executable `{name}` was not found on absolute PATH entries"
    ))
}

fn require_absolute_existing_file(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be absolute"));
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            Ok(())
        },
        Ok(_) => Err(format!("{label} must be a regular non-symlink file")),
        Err(error) => Err(format!(
            "cannot inspect {label} {}: {error}",
            path.display()
        )),
    }
}

fn require_absolute_existing_directory(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be absolute"));
    }
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(format!("{label} must be a directory")),
        Err(error) => Err(format!(
            "cannot inspect {label} {}: {error}",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsStr, path::Path};

    // builder는 network를 유지하되 root를 read-only로 만들고, private runtime/tmp와
    // 두 writable path, exact marker, no-tools child argv를 하나의 argv로 고정합니다.
    #[test]
    fn command_shape_keeps_the_outer_boundary_narrow() {
        let arguments = ["-p", "--model", "host:grok", "--sandbox", "read-only"].map(str::to_owned);
        let command = super::build_command(
            Path::new("/usr/bin/bwrap"),
            Path::new("/repo/target/debug/yo"),
            &arguments,
            Path::new("/repo/integration"),
            Path::new("/home/reviewer/.grok"),
            Path::new("/repo/sessions"),
            &["/home/reviewer/.docker/run/docker.sock".into()],
        )
        .unwrap();
        let argv = command.get_args().collect::<Vec<_>>();

        assert!(argv.windows(3).any(|window| window == [
            OsStr::new("--ro-bind"),
            OsStr::new("/"),
            OsStr::new("/")
        ]));
        assert!(
            argv.windows(2)
                .any(|window| window == [OsStr::new("--tmpfs"), OsStr::new("/run")])
        );
        assert!(
            argv.windows(2)
                .any(|window| window == [OsStr::new("--dev"), OsStr::new("/dev")])
        );
        assert!(!argv.iter().any(|argument| *argument == "--dev-bind"));
        assert!(argv.windows(3).any(|window| window
            == [
                OsStr::new("--setenv"),
                OsStr::new(super::OUTER_SANDBOX_REVIEW_ENV),
                OsStr::new(super::OUTER_SANDBOX_REVIEW_PROFILE)
            ]));
        assert!(argv.windows(3).any(|window| window
            == [
                OsStr::new("--ro-bind"),
                OsStr::new("/dev/null"),
                OsStr::new("/home/reviewer/.docker/run/docker.sock")
            ]));
        assert_eq!(
            &argv[argv.len() - arguments.len()..],
            arguments.iter().map(OsStr::new).collect::<Vec<_>>()
        );
    }

    // workspace가 writable state 또는 Session subtree 안으로 들어가면 read-only claim이
    // 성립하지 않으므로 bwrap 실행 전에 거부합니다.
    #[test]
    fn command_rejects_workspace_inside_a_writable_bind() {
        let error = super::build_command(
            Path::new("/usr/bin/bwrap"),
            Path::new("/repo/target/debug/yo"),
            &[],
            Path::new("/repo/sessions/workspace"),
            Path::new("/home/reviewer/.grok"),
            Path::new("/repo/sessions"),
            &[],
        )
        .unwrap_err();
        assert!(error.contains("must not be inside writable path"));
    }

    // outer fallback은 일반 host 실행 wrapper가 아니며 review runner가 만드는 두 exact
    // print-mode argv 외에는 filesystem 검사 전부터 닫힙니다.
    #[test]
    fn delivery_arguments_reject_general_host_invocation() {
        super::require_delivery_arguments(
            &["-p", "--model", "host:grok", "--sandbox", "read-only"].map(str::to_owned),
        )
        .unwrap();
        super::require_delivery_arguments(
            &["-p", "--resume", "01890f00-0000-7000-8000-000000000001"].map(str::to_owned),
        )
        .unwrap();
        assert!(
            super::require_delivery_arguments(&["--model", "host:grok"].map(str::to_owned))
                .is_err()
        );
    }
}
