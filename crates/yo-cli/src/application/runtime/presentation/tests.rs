use std::env::vars_os;

use super::*;

fn fixture_git() -> Command {
    let mut command = Command::new("git");
    // A commit hook must not redirect fixture commands to its own repository.
    for (key, _) in vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
}

fn shared() -> Shared {
    Arc::new((Mutex::new(State::default()), Condvar::new()))
}

// 상태 프로세스의 출력은 첫 초과 바이트에서 거절하며 시간 제한 뒤 소유한 자식을 회수한다.
#[test]
fn output_and_execution_are_bounded() {
    let state = shared();
    let mut output = Command::new("printf");
    output.arg("x".repeat(1025));
    assert_eq!(
        run(output, &state, Duration::from_secs(2))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    let mut sleeping = Command::new("sleep");
    sleeping.arg("30");
    let started = Instant::now();
    assert_eq!(
        run(sleeping, &state, Duration::from_millis(30))
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

// 종료 신호는 상태 수집을 취소하며 worker의 대기 주기를 기다리지 않는다.
#[test]
fn shutdown_cancels_collection_and_idle_wait() {
    let state = shared();
    state.0.lock().unwrap().stop = true;
    let mut command = Command::new("sleep");
    command.arg("30");
    assert!(run(command, &state, Duration::from_secs(2)).is_err());
    let started = Instant::now();
    drop(Presentation::start(std::env::temp_dir()));
    assert!(started.elapsed() < Duration::from_secs(2));
}

struct Source {
    polls: usize,
    readiness: usize,
}
impl AgentConnection for Source {
    type Error = io::Error;
    fn dispatch(&mut self, _: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }
    fn retry(&mut self, _: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }
    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        self.polls += 1;
        if self.polls == 1 {
            Err(io::ErrorKind::BrokenPipe.into())
        } else {
            Ok(AgentPoll::Closed)
        }
    }
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<()> {
        self.readiness += 1;
        Poll::Pending
    }
}

// 연속 호스트 갱신 사이에도 원래 연결의 오류·종료 순서를 보존하고 종료 후 상태를 내보내지 않는다.
#[test]
fn host_updates_do_not_starve_or_replace_source_outcomes() {
    let mut presentation = Presentation {
        shared: shared(),
        worker: None,
        host_turn: true,
        closed: false,
    };
    let mut source = Source {
        polls: 0,
        readiness: 0,
    };
    let status = status("Git · main");
    publish(&presentation.shared, status.clone());
    assert_eq!(
        presentation.connection(&mut source).poll().unwrap(),
        AgentPoll::StatusLine(status.clone())
    );
    publish(&presentation.shared, status.clone());
    assert_eq!(
        presentation
            .connection(&mut source)
            .poll()
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        presentation.connection(&mut source).poll().unwrap(),
        AgentPoll::StatusLine(status.clone())
    );
    publish(&presentation.shared, status);
    assert_eq!(
        presentation.connection(&mut source).poll().unwrap(),
        AgentPoll::Closed
    );
    assert_eq!(
        presentation.connection(&mut source).poll().unwrap(),
        AgentPoll::Closed
    );
    assert_eq!(source.polls, 2);
}

// 최신 상태 한 개만 보유하며 상태가 준비되어도 원래 연결의 깨우기 등록을 생략하지 않는다.
#[test]
fn coalesced_status_registers_both_readiness_sources() {
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        task::Wake,
    };
    struct Counter(AtomicUsize);
    impl Wake for Counter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let count = Arc::new(Counter(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&count));
    let mut context = Context::from_waker(&waker);
    let mut presentation = Presentation {
        shared: shared(),
        worker: None,
        host_turn: true,
        closed: false,
    };
    let mut source = Source {
        polls: 0,
        readiness: 0,
    };
    assert!(
        presentation
            .connection(&mut source)
            .poll_ready(&mut context)
            .is_pending()
    );
    publish(&presentation.shared, status("first"));
    publish(&presentation.shared, status("latest"));
    assert_eq!(count.0.load(Ordering::SeqCst), 1);
    assert!(
        presentation
            .connection(&mut source)
            .poll_ready(&mut context)
            .is_ready()
    );
    assert_eq!(source.readiness, 2);
    assert_eq!(presentation.take(), Some(status("latest")));
    assert!(presentation.take().is_none());
}

// 실제 Git 저장소의 unborn 브랜치 변경과 실패 경로를 캐시 추측 없이 그대로 반영한다.
#[test]
fn git_branch_follows_observed_repository_changes() {
    let root = std::env::temp_dir().join(format!("yo-host-status-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let state = shared();
    assert_eq!(branch(&root, &state), "Git status unavailable");
    assert!(
        fixture_git()
            .args(["init", "--quiet", "--initial-branch=initial"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(branch(&root, &state), "Git · initial");
    assert!(
        fixture_git()
            .current_dir(&root)
            .args(["symbolic-ref", "HEAD", "refs/heads/changed"])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(branch(&root, &state), "Git · changed");
    let mut exact = Command::new("printf");
    exact.arg("x".repeat(1024));
    assert_eq!(
        run(exact, &state, Duration::from_secs(2)).unwrap().1.len(),
        1024
    );
}

// 실제 호스트 연결은 새 파일 링크와 Git ignore를 반영하고 링크 오류 뒤에는 기존 매핑을 해제한다.
#[test]
fn execution_host_link_inventory_honors_git_ignore_and_new_files() {
    let root = std::env::temp_dir().join(format!("yo-host-links-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    assert!(
        fixture_git()
            .args(["init", "--quiet"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(root.join(".gitignore"), "ignored\n").unwrap();
    std::fs::write(root.join("ignored"), "hidden").unwrap();
    std::fs::write(root.join("visible.rs"), "visible").unwrap();
    let state = shared();
    let (exit, bytes) = run_with_limit(
        LocalWorkspaceReferenceProvider::output_link_command(&root),
        &state,
        Duration::from_secs(2),
        4 * 1024 * 1024,
    )
    .unwrap();
    assert!(exit.success());
    let paths =
        LocalWorkspaceReferenceProvider::output_links(&root, Some(&bytes), || false).unwrap();
    assert!(paths.contains_key("visible.rs"));
    assert!(!paths.contains_key("ignored"));
    assert!(resolver(&paths).is_some());
    std::fs::remove_file(root.join("visible.rs")).unwrap();
    let paths =
        LocalWorkspaceReferenceProvider::output_links(&root, Some(&bytes), || false).unwrap();
    assert!(!paths.contains_key("visible.rs"));
    assert!(resolver(&BTreeMap::new()).is_none());
    let mut presentation = Presentation {
        shared: state,
        worker: None,
        host_turn: true,
        closed: false,
    };
    presentation.shared.0.lock().unwrap().links = Some(resolver(&paths));
    let mut source = Source {
        polls: 0,
        readiness: 0,
    };
    assert!(matches!(
        presentation.connection(&mut source).poll().unwrap(),
        AgentPoll::Links(Some(_))
    ));
    presentation.shared.0.lock().unwrap().links = Some(None);
    assert!(presentation.connection(&mut source).poll().is_err());
    assert_eq!(
        presentation.connection(&mut source).poll().unwrap(),
        AgentPoll::Links(None)
    );
}

// 실제 background worker가 Git 작업공간의 파일을 게시하고 삭제 후 링크를 해제한 뒤 종료한다.
#[test]
fn worker_publishes_and_revokes_workspace_links() {
    let root = std::env::temp_dir().join(format!("yo-host-refresh-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    assert!(
        fixture_git()
            .args(["init", "--quiet"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(root.join("visible.rs"), "content").unwrap();
    let presentation = Presentation::start(root.clone());
    let wait_links = || {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(update) = presentation.shared.0.lock().unwrap().links.take() {
                return update;
            }
            assert!(
                Instant::now() < deadline,
                "host worker did not publish links"
            );
            thread::sleep(Duration::from_millis(10));
        }
    };
    assert!(wait_links().is_some());
    std::fs::remove_file(root.join("visible.rs")).unwrap();
    assert!(wait_links().is_none());
    drop(presentation);
}

// URL 인코딩은 한 번만 풀고 공백·한글·리터럴 퍼센트·플러스를 구분하며 검증된 항목만 선택한다.
#[test]
fn encoded_output_paths_select_only_validated_inventory_entries() {
    let names = ["notes today.md", "notes%20today.md", "한글.rs", "a+b.rs"];
    let links = names
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                Hyperlink::from_file_path(&Path::new("/tmp").join(name)).unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for (source, expected) in [
        ("notes%20today.md", "notes today.md"),
        ("notes%2520today.md", "notes%20today.md"),
        ("%ED%95%9C%EA%B8%80.rs", "한글.rs"),
        ("./%ed%95%9c%ea%b8%80.rs", "한글.rs"),
        ("a+b.rs", "a+b.rs"),
        ("a%2Bb.rs", "a+b.rs"),
    ] {
        assert_eq!(
            resolve_link(&links, source),
            links.get(expected).cloned(),
            "{source}"
        );
    }
    for source in [
        "missing.rs",
        "%",
        "%GG",
        "%FF",
        "%00",
        "..%2foutside",
        "file%3A%2F%2F%2Ftmp%2Fnotes%20today.md",
    ] {
        assert!(resolve_link(&links, source).is_none(), "{source}");
    }
}
