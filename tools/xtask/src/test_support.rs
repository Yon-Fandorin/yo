use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
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
        let status = Command::new("git")
            .current_dir(&self.path)
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_WORK_TREE")
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
