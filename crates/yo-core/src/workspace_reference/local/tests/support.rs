use std::{
    fs,
    io::{self, Write},
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
        let root = allocate_fixture_root(
            label,
            &[PathBuf::from("/dev/shm"), std::env::temp_dir()],
            |path: &Path| fs::create_dir(path),
        );
        Self { root }
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

fn allocate_fixture_root(
    label: &str,
    bases: &[PathBuf],
    mut create_dir: impl FnMut(&Path) -> io::Result<()>,
) -> PathBuf {
    for _ in 0..MAX_FIXTURE_ATTEMPTS {
        let counter = TEMP_FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("yo-{label}-{}-{counter}", std::process::id());
        let mut last_error = None;
        let mut collision = false;
        for base in bases {
            let root = base.join(&name);
            match create_dir(&root) {
                Ok(()) => return root,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    collision = true;
                },
                Err(error) => last_error = Some(error),
            }
        }
        if collision {
            continue;
        }
        if let Some(error) = last_error {
            panic!("creating test fixture {name} failed: {error}");
        }
    }
    panic!("creating test fixture {label} failed after {MAX_FIXTURE_ATTEMPTS} collisions");
}

// 선택 base 오류 뒤 fallback 충돌이 발생하면 오류를 확정하지 않고 다음 이름으로 재시도한다.
#[test]
fn fixture_allocator_retries_after_optional_base_error_and_fallback_collision() {
    use std::cell::Cell;

    let calls = Cell::new(0);
    let root = allocate_fixture_root(
        "mixed-outcomes",
        &[PathBuf::from("/optional"), PathBuf::from("/fallback")],
        |path| {
            let call = calls.get();
            calls.set(call + 1);
            match call {
                0 => Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("optional base unavailable: {}", path.display()),
                )),
                1 => Err(io::Error::from(io::ErrorKind::AlreadyExists)),
                2 => Ok(()),
                _ => unreachable!("the mixed allocator fixture should succeed on its retry"),
            }
        },
    );

    assert_eq!(calls.get(), 3);
    assert_eq!(root.parent(), Some(Path::new("/optional")));
}

// 모든 base가 일반 오류를 반환하면 충돌 재시도를 기다리지 않고 즉시 실패한다.
#[test]
fn fixture_allocator_fails_immediately_when_all_bases_error() {
    use std::cell::Cell;

    let calls = Cell::new(0);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        allocate_fixture_root(
            "all-errors",
            &[PathBuf::from("/optional"), PathBuf::from("/fallback")],
            |_| {
                calls.set(calls.get() + 1);
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "base unavailable",
                ))
            },
        )
    }));

    assert!(result.is_err());
    assert_eq!(calls.get(), 2);
}

pub(super) fn host_id() -> WorkspaceHostId {
    "10000000-0000-4000-8000-000000000001".parse().unwrap()
}
