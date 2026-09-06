#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

struct Repository(PathBuf);

impl Repository {
    fn command(&self, program: &str, args: &[&str]) -> ExitStatus {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(&self.0)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("command timed out: {program} {args:?}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn git(&self, args: &[&str]) {
        assert!(self.command("git", args).success(), "git {args:?}");
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// 실제 Git commit-msg 경계에서 일반 -m/-F commit은 통과하고 거짓 review 주장은
// HEAD 변경 전에 거부되는지 확인한다. 명시적 formal preflight는 계속 엄격하다.
#[test]
fn ordinary_git_messages_work_but_false_review_claims_do_not_commit() {
    let path = std::env::temp_dir().join(format!(
        "yo-change-commit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&path).unwrap();
    let repository = Repository(path);
    repository.git(&["init", "--quiet", "-b", "develop"]);
    repository.git(&["config", "user.name", "Workflow test"]);
    repository.git(&["config", "user.email", "workflow@example.invalid"]);
    repository.git(&["config", "commit.gpgsign", "false"]);
    repository.git(&["config", "core.hooksPath", ".git/hooks"]);

    let binary = env!("CARGO_BIN_EXE_xtask");
    let hook = repository.0.join(".git/hooks/commit-msg");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\nexec '{}' check change-preflight \"$1\"\n",
            binary.replace('\'', "'\\''")
        ),
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    fs::create_dir_all(repository.0.join("tools/example")).unwrap();
    fs::write(repository.0.join("tools/example/check.rs"), "first\n").unwrap();
    repository.git(&["add", "tools/example/check.rs"]);
    repository.git(&["commit", "--quiet", "-m", "fix: first ordinary change"]);

    fs::write(repository.0.join("tools/example/check.rs"), "second\n").unwrap();
    fs::write(
        repository.0.join(".git/message"),
        "fix: second ordinary change\n",
    )
    .unwrap();
    repository.git(&["add", "tools/example/check.rs"]);
    repository.git(&["commit", "--quiet", "-F", ".git/message"]);
    repository.git(&["branch", "before-rejected-claim"]);

    fs::write(repository.0.join("tools/example/check.rs"), "third\n").unwrap();
    repository.git(&["add", "tools/example/check.rs"]);
    assert!(
        !repository
            .command(
                "git",
                &[
                    "commit",
                    "--quiet",
                    "-m",
                    "fix: false claim\n\nSlice-Review: none - bypass review\n",
                ]
            )
            .success()
    );
    repository.git(&[
        "merge-base",
        "--is-ancestor",
        "HEAD",
        "before-rejected-claim",
    ]);
    repository.git(&[
        "merge-base",
        "--is-ancestor",
        "before-rejected-claim",
        "HEAD",
    ]);
    assert!(
        !repository
            .command(binary, &["check", "commit-preflight", ".git/message"])
            .success()
    );
}
