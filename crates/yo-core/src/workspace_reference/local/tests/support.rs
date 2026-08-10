use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::WorkspaceHostId;

static TEMP_FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);
const MAX_FIXTURE_ATTEMPTS: usize = 128;

pub(super) struct TempFixture {
    root: PathBuf,
}

impl TempFixture {
    pub(super) fn new(label: &str) -> Self {
        for _ in 0..MAX_FIXTURE_ATTEMPTS {
            let counter = TEMP_FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!("yo-{label}-{}-{counter}", std::process::id());
            let mut last_error = None;
            let mut collision = false;
            for base in [PathBuf::from("/dev/shm"), std::env::temp_dir()] {
                let root = base.join(&name);
                match fs::create_dir(&root) {
                    Ok(()) => return Self { root },
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        collision = true;
                    },
                    Err(error) => last_error = Some(error),
                }
            }
            if let Some(error) = last_error {
                panic!("creating test fixture {name} failed: {error}");
            }
            if collision {
                continue;
            }
        }
        panic!("creating test fixture {label} failed after {MAX_FIXTURE_ATTEMPTS} collisions");
    }

    pub(super) fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.root) {
            if std::thread::panicking() {
                let mut stderr = std::io::stderr();
                let _ = writeln!(
                    stderr,
                    "cleaning test fixture {} failed during unwinding: {error}",
                    self.root.display()
                );
            } else {
                panic!(
                    "cleaning test fixture {} failed: {error}",
                    self.root.display()
                );
            }
        }
    }
}

pub(super) fn host_id() -> WorkspaceHostId {
    "10000000-0000-4000-8000-000000000001".parse().unwrap()
}
