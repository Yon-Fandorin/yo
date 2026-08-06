use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

pub(crate) fn unique_path(label: &str) -> PathBuf {
    let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "yo-xtask-{label}-{}-{sequence}",
        std::process::id()
    ))
}

pub(crate) struct TestRepository {
    pub(crate) path: PathBuf,
}

impl TestRepository {
    pub(crate) fn new(label: &str) -> Self {
        let path = unique_path(label);
        std::fs::create_dir_all(&path).unwrap();
        let repository = Self { path };
        repository.git(["init", "--quiet", "-b", "develop"]);
        repository.git(["config", "user.name", "xtask Test"]);
        repository.git(["config", "user.email", "xtask@example.invalid"]);
        let hooks = repository.path.join(".git/disabled-hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        repository.git(["config", "core.hooksPath", hooks.to_str().unwrap()]);
        repository
    }

    pub(crate) fn write(&self, relative: impl AsRef<Path>, content: &str) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
        path
    }

    pub(crate) fn git<I, S>(&self, arguments: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let status = crate::git::command_in(&self.path, false)
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
