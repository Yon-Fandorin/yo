use std::{
    ffi::OsStr,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

pub(crate) fn output_in(
    directory: &Path,
    arguments: &[&str],
    inherit_repository_environment: bool,
) -> Result<String, String> {
    let output = output_bytes_in(directory, arguments, inherit_repository_environment)?;
    String::from_utf8(output).map_err(|error| {
        format!(
            "git {} returned non-UTF-8 output: {error}",
            arguments.join(" ")
        )
    })
}

pub(crate) fn output_bytes_in(
    directory: &Path,
    arguments: &[&str],
    inherit_repository_environment: bool,
) -> Result<Vec<u8>, String> {
    let result = command_in(directory, inherit_repository_environment)
        .args(arguments)
        .output()
        .map_err(|error| format!("cannot run git {}: {error}", arguments.join(" ")))?;
    if result.status.success() {
        Ok(result.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        Err(format!(
            "git {} failed with {}{}",
            arguments.join(" "),
            result.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ))
    }
}

pub(crate) fn output_bytes_in_with_index(
    directory: &Path,
    arguments: &[&str],
    index_file: Option<&OsStr>,
) -> Result<Vec<u8>, String> {
    let mut command = command_in(directory, false);
    if let Some(index_file) = index_file {
        command.env("GIT_INDEX_FILE", index_file);
    }
    let result = command
        .args(arguments)
        .output()
        .map_err(|error| format!("cannot run git {}: {error}", arguments.join(" ")))?;
    if result.status.success() {
        Ok(result.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        Err(format!(
            "git {} failed with {}{}",
            arguments.join(" "),
            result.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ))
    }
}

pub(crate) fn optional_output_in(
    directory: &Path,
    arguments: &[&str],
    inherit_repository_environment: bool,
) -> Result<Option<String>, String> {
    let result = command_in(directory, inherit_repository_environment)
        .args(arguments)
        .output()
        .map_err(|error| format!("cannot run git {}: {error}", arguments.join(" ")))?;
    if result.status.success() {
        String::from_utf8(result.stdout).map(Some).map_err(|error| {
            format!(
                "git {} returned non-UTF-8 output: {error}",
                arguments.join(" ")
            )
        })
    } else {
        Ok(None)
    }
}

pub(crate) fn succeeds_in(
    directory: &Path,
    arguments: &[&str],
    inherit_repository_environment: bool,
) -> Result<bool, String> {
    command_in(directory, inherit_repository_environment)
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .map_err(|error| format!("cannot run git {}: {error}", arguments.join(" ")))
}

pub(crate) fn command_in(directory: &Path, inherit_repository_environment: bool) -> Command {
    let mut command = Command::new("git");
    command.current_dir(directory);
    if !inherit_repository_environment {
        clear_repository_environment(&mut command);
        #[cfg(test)]
        isolate_test_environment(&mut command);
    }
    command
}

#[cfg(test)]
pub(crate) fn test_command_in(directory: &Path) -> Command {
    command_in(directory, false)
}

pub(crate) fn trusted_output_in(directory: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = trusted_output_bytes_in(directory, arguments)?;
    String::from_utf8(output).map_err(|error| {
        format!(
            "trusted Git {} returned non-UTF-8 output: {error}",
            arguments.join(" ")
        )
    })
}

pub(crate) fn trusted_output_bytes_in(
    directory: &Path,
    arguments: &[&str],
) -> Result<Vec<u8>, String> {
    let result = trusted_command_in(directory)
        .args(arguments)
        .output()
        .map_err(|error| format!("cannot run trusted Git {}: {error}", arguments.join(" ")))?;
    if result.status.success() {
        Ok(result.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        Err(format!(
            "trusted Git {} failed with {}{}",
            arguments.join(" "),
            result.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ))
    }
}

pub(crate) fn trusted_succeeds_in(directory: &Path, arguments: &[&str]) -> Result<bool, String> {
    trusted_command_in(directory)
        .args(arguments)
        .output()
        .map(|output| output.status.success())
        .map_err(|error| format!("cannot run trusted Git {}: {error}", arguments.join(" ")))
}

pub(crate) fn trusted_command_in(directory: &Path) -> Command {
    let mut command = Command::new("/usr/bin/git");
    command
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_GRAFT_FILE", "/dev/null")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("LC_ALL", "C")
        .args(["-c", "advice.graftFileDeprecated=false"])
        .arg("--no-replace-objects")
        .current_dir(directory);
    command
}

fn clear_repository_environment(command: &mut Command) {
    for name in [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_COUNT",
        "GIT_OBJECT_DIRECTORY",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_IMPLICIT_WORK_TREE",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_NO_REPLACE_OBJECTS",
        "GIT_REPLACE_REF_BASE",
        "GIT_PREFIX",
        "GIT_SHALLOW_FILE",
        "GIT_COMMON_DIR",
    ] {
        command.env_remove(name);
    }
}

#[cfg(test)]
fn isolate_test_environment(command: &mut Command) {
    let inherited = [
        ("PATH", std::env::var_os("PATH")),
        ("TMPDIR", std::env::var_os("TMPDIR")),
        ("TMP", std::env::var_os("TMP")),
        ("TEMP", std::env::var_os("TEMP")),
    ];
    command.env_clear();
    for (name, value) in inherited {
        if let Some(value) = value {
            command.env(name, value);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("LANG", "C");
}

pub(crate) fn interpret_trailers(message: &str) -> Result<String, String> {
    let mut child = command_in(Path::new("."), false)
        .args(["interpret-trailers", "--parse"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start git interpret-trailers: {error}"))?;
    child
        .stdin
        .take()
        .expect("piped stdin is available")
        .write_all(message.as_bytes())
        .map_err(|error| {
            format!("cannot pass the commit message to git interpret-trailers: {error}")
        })?;
    let result = child
        .wait_with_output()
        .map_err(|error| format!("cannot wait for git interpret-trailers: {error}"))?;
    if !result.status.success() {
        return Err("git interpret-trailers could not parse the commit message".to_owned());
    }
    String::from_utf8(result.stdout)
        .map_err(|error| format!("git interpret-trailers returned non-UTF-8 output: {error}"))
}

pub(crate) fn read(path: &Path, label: &str) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {label} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, process::Command};

    use super::{isolate_test_environment, test_command_in};
    use crate::test_support::unique_path;

    // 바깥 저장소를 가리키는 Git 환경을 자식 명령마다 주입해도 공용 테스트 격리가
    // 이를 제거하여 바깥 config·index·HEAD/ref 바이트와 commit identity를 보존한다.
    #[test]
    fn isolated_test_commands_preserve_outer_repository_bytes_and_identity() {
        let root = unique_path("git-isolation");
        let outer = root.join("outer");
        let inner = root.join("inner");
        fs::create_dir_all(&outer).unwrap();
        fs::create_dir_all(&inner).unwrap();

        run(
            &mut test_command_in(&outer),
            ["init", "-q", "-b", "develop"],
        );
        run(
            &mut test_command_in(&outer),
            ["config", "--local", "user.name", "Outer Owner"],
        );
        run(
            &mut test_command_in(&outer),
            ["config", "--local", "user.email", "outer@example.invalid"],
        );
        fs::write(outer.join("tracked.txt"), b"outer\n").unwrap();
        run(&mut test_command_in(&outer), ["add", "tracked.txt"]);
        run(&mut test_command_in(&outer), ["commit", "-qm", "outer"]);

        let git_dir = outer.join(".git");
        let before = outer_repository_bytes(&git_dir);

        run(
            &mut poisoned_test_command(&inner, &outer),
            ["init", "-q", "-b", "develop"],
        );
        run(
            &mut poisoned_test_command(&inner, &outer),
            ["config", "--local", "user.name", "Inner Test"],
        );
        run(
            &mut poisoned_test_command(&inner, &outer),
            ["config", "--local", "user.email", "inner@example.invalid"],
        );
        fs::write(inner.join("tracked.txt"), b"inner\n").unwrap();
        run(
            &mut poisoned_test_command(&inner, &outer),
            ["add", "tracked.txt"],
        );
        run(
            &mut poisoned_test_command(&inner, &outer),
            ["commit", "-qm", "inner"],
        );

        let identity = poisoned_test_command(&inner, &outer)
            .args(["log", "-1", "--format=%an%x00%ae"])
            .output()
            .unwrap();
        assert!(identity.status.success());
        assert_eq!(identity.stdout, b"Inner Test\0inner@example.invalid\n");
        assert_eq!(outer_repository_bytes(&git_dir), before);

        fs::remove_dir_all(root).unwrap();
    }

    fn poisoned_test_command(directory: &Path, outer: &Path) -> Command {
        let git_dir = outer.join(".git");
        let mut command = Command::new("git");
        command
            .current_dir(directory)
            .env("GIT_DIR", &git_dir)
            .env("GIT_WORK_TREE", outer)
            .env("GIT_COMMON_DIR", &git_dir)
            .env("GIT_INDEX_FILE", git_dir.join("index"))
            .env("GIT_NAMESPACE", "outer-namespace")
            .env("GIT_AUTHOR_NAME", "Leaked Author")
            .env("GIT_AUTHOR_EMAIL", "leaked-author@example.invalid")
            .env("GIT_COMMITTER_NAME", "Leaked Committer")
            .env("GIT_COMMITTER_EMAIL", "leaked-committer@example.invalid");
        isolate_test_environment(&mut command);
        command
    }

    fn outer_repository_bytes(git_dir: &Path) -> [Vec<u8>; 4] {
        [
            fs::read(git_dir.join("config")).unwrap(),
            fs::read(git_dir.join("index")).unwrap(),
            fs::read(git_dir.join("HEAD")).unwrap(),
            fs::read(git_dir.join("refs/heads/develop")).unwrap(),
        ]
    }

    fn run<const N: usize>(command: &mut Command, arguments: [&str; N]) {
        let output = command.args(arguments).output().unwrap();
        assert!(
            output.status.success(),
            "Git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
