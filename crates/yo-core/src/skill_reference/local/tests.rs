use std::{fs, os::unix::fs::symlink, sync::mpsc};

use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("yo-local-skills-{}", uuid::Uuid::now_v7()));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
    fn root(&self) -> LocalSkillRoot {
        LocalSkillRoot::new(self.0.clone(), SkillReferenceScope::Workspace).unwrap()
    }
    fn skill(&self, name: &str, text: &str) -> PathBuf {
        let directory = self.0.join(name);
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("SKILL.md");
        fs::write(&path, text).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn catalog(fixture: &Fixture, host: WorkspaceHostId) -> Catalog {
    Catalog::new(vec![fixture.root()], host).unwrap()
}
fn rows(catalog: &Catalog) -> Vec<SkillReferenceCandidate> {
    catalog.discover(1, &AtomicBool::new(false)).unwrap().0
}
fn input(reference: SkillReference) -> UserInput {
    let text = format!("${}", reference.name());
    UserInput::with_references(&text, vec![InputReference::skill(0..text.len(), reference)])
        .unwrap()
}

// 같은 이름의 서로 다른 출처는 별도 identity를 유지하고 본문·frontmatter는 변형하지 않는다.
#[test]
fn exact_snapshots_preserve_sources_policy_and_original_bytes() {
    let one = Fixture::new();
    let two = Fixture::new();
    let host = WorkspaceHostId::new().unwrap();
    let text = "---\nname: review\ndescription: Inspect changes\nmetadata: {other: retained}\n---\n한글 instructions\n";
    let path = one.skill("first", text);
    two.skill("second", text);
    let roots = vec![
        one.root(),
        LocalSkillRoot::new(two.0.clone(), SkillReferenceScope::User).unwrap(),
    ];
    let catalog = Catalog::new(roots.clone(), host).unwrap();
    let candidates = rows(&catalog);
    assert_eq!(candidates.len(), 2);
    assert_eq!(
        candidates[0].reference().name(),
        candidates[1].reference().name()
    );
    assert_ne!(
        candidates[0].reference().identity(),
        candidates[1].reference().identity()
    );
    let selected = candidates
        .iter()
        .find(|row| row.reference().locator() == path.to_str().unwrap())
        .unwrap()
        .reference()
        .clone();
    let admission = LocalSkillInputAdmission::new(&one.0, roots, host).unwrap();
    let draft = input(selected.clone());
    admission.validate(&draft).unwrap();
    let snapshot = admission.prepare(&draft).unwrap().unwrap();
    assert_eq!(snapshot.instructions(), text);
    assert_eq!(snapshot.reference(), &selected);
    fs::write(path, "changed").unwrap();
    assert_eq!(
        admission.prepare(&draft).unwrap_err().kind(),
        SubmissionRejectionKind::StaleReference
    );
    assert_eq!(snapshot.instructions(), text);
}

// 비활성·잘못된 metadata·비정규 파일은 숨기지 않고 선택 불가 행으로 보존한다.
#[test]
fn unusable_entries_remain_visible_and_cannot_be_prepared() {
    let fixture = Fixture::new();
    let host = WorkspaceHostId::new().unwrap();
    fixture.skill("disabled", "---\nenabled: false\n---\nbody");
    fixture.skill("private", "---\nuser-invocable: false\n---\nbody");
    fixture.skill("invalid", "---\nenabled: wrong\n---\nbody");
    fixture.skill("unclosed", "---\nname: missing terminator");
    fixture.skill("utf8", "ok");
    fs::write(fixture.0.join("utf8/SKILL.md"), [255]).unwrap();
    fs::create_dir_all(fixture.0.join("directory/SKILL.md")).unwrap();
    let candidates = rows(&catalog(&fixture, host));
    assert_eq!(candidates.len(), 6);
    assert!(candidates.iter().all(|row| matches!(row.availability(), SkillAvailability::Disabled(reason) if !reason.is_empty())));
    let admission = LocalSkillInputAdmission::new(&fixture.0, vec![fixture.root()], host).unwrap();
    for candidate in candidates {
        assert!(
            admission
                .prepare(&input(candidate.reference().clone()))
                .is_err()
        );
    }
}

// root·하위 디렉터리·SKILL 파일의 symlink는 따라가지 않고 기존 root inode 교체도 거절한다.
#[test]
fn symlink_paths_and_replaced_roots_cannot_redirect_skill_loading() {
    let fixture = Fixture::new();
    let outside = Fixture::new();
    let host = WorkspaceHostId::new().unwrap();
    let external = outside.skill("external", "outside");
    symlink(external.parent().unwrap(), fixture.0.join("directory-link")).unwrap();
    fs::create_dir(fixture.0.join("file-link")).unwrap();
    symlink(external, fixture.0.join("file-link/SKILL.md")).unwrap();
    let root_alias = fixture.0.join("root-link");
    symlink(&outside.0, &root_alias).unwrap();
    let aliased = Catalog::new(
        vec![LocalSkillRoot::new(root_alias, SkillReferenceScope::User).unwrap()],
        host,
    )
    .unwrap();
    assert!(matches!(
        aliased.discover(1, &AtomicBool::new(false)).unwrap().1,
        SkillReferenceSearchStatus::Incomplete(_)
    ));
    let catalog = catalog(&fixture, host);
    assert!(
        rows(&catalog)
            .iter()
            .all(|row| matches!(row.availability(), SkillAvailability::Disabled(_)))
    );
    let retired = fixture.0.with_extension("retired");
    fs::rename(&fixture.0, &retired).unwrap();
    fs::create_dir(&fixture.0).unwrap();
    assert!(catalog.roots[0].open().is_err());
    fs::remove_dir_all(retired).unwrap();
}

// 본문 제한값을 허용하고 첫 초과를 비활성화하며 검색 항목 수 초과는 incomplete로 보고한다.
#[test]
fn file_and_inventory_limits_do_not_silently_truncate() {
    let fixture = Fixture::new();
    let host = WorkspaceHostId::new().unwrap();
    let path = fixture.skill("bounded", &"x".repeat(ResolvedSkill::MAX_INSTRUCTION_BYTES));
    let catalog = catalog(&fixture, host);
    assert_eq!(
        rows(&catalog)[0].availability(),
        &SkillAvailability::Enabled
    );
    fs::write(&path, "x".repeat(ResolvedSkill::MAX_INSTRUCTION_BYTES + 1)).unwrap();
    assert!(
        matches!(rows(&catalog)[0].availability(), SkillAvailability::Disabled(reason) if reason.contains("limit"))
    );
    let entries = Fixture::new();
    for index in 0..MAX_ENTRIES {
        fs::write(entries.0.join(format!("file{index}")), "").unwrap();
    }
    let catalog = Catalog::new(vec![entries.root()], host).unwrap();
    assert_eq!(
        catalog.discover(1, &AtomicBool::new(false)).unwrap().1,
        SkillReferenceSearchStatus::Complete
    );
    fs::write(entries.0.join("first-excess"), "").unwrap();
    assert!(matches!(
        catalog.discover(1, &AtomicBool::new(false)).unwrap().1,
        SkillReferenceSearchStatus::Incomplete(_)
    ));
}

// 사라진 configured root는 호스트 구성을 막지 않고 새 선택만 실패하며 생성 후에는 검색된다.
#[test]
fn unavailable_roots_do_not_block_binding_or_frozen_snapshot_replay() {
    let workspace = Fixture::new();
    let missing = workspace.0.join("later");
    let root = LocalSkillRoot::new(missing.clone(), SkillReferenceScope::Workspace).unwrap();
    let host = WorkspaceHostId::new().unwrap();
    LocalSkillInputAdmission::new(&workspace.0, vec![root.clone()], host).unwrap();
    let catalog = Catalog::new(vec![root], host).unwrap();
    assert!(matches!(
        catalog.discover(1, &AtomicBool::new(false)).unwrap().1,
        SkillReferenceSearchStatus::Incomplete(_)
    ));
    fs::create_dir_all(missing.join("skill")).unwrap();
    fs::write(missing.join("skill/SKILL.md"), "instructions").unwrap();
    assert_eq!(rows(&catalog).len(), 1);
}

// 실제 worker는 결과를 게시하고 종료 요청이 대기 중인 condvar를 깨워 자원을 회수한다.
#[test]
fn worker_publishes_results_and_stops_without_a_pending_request() {
    let fixture = Fixture::new();
    fixture.skill("review", "instructions");
    let mut provider =
        LocalSkillReferenceProvider::start(vec![fixture.root()], WorkspaceHostId::new().unwrap())
            .unwrap();
    provider
        .search(SkillReferenceSearchRequest::new(
            1,
            1,
            1,
            0..1,
            "$",
            "",
            true,
        ))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let SkillReferenceProviderPoll::Update(update) = provider.poll().unwrap() {
            assert_eq!(update.request_id(), 1);
            assert_eq!(update.candidates().len(), 1);
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let (send, receive) = mpsc::channel();
    let worker = thread::spawn(move || {
        drop(provider);
        send.send(()).unwrap();
    });
    receive.recv_timeout(Duration::from_secs(5)).unwrap();
    worker.join().unwrap();
}

// 루트 16개와 합계 16MiB를 허용하고 첫 초과는 각각 구성 오류와 incomplete로 보고한다.
#[test]
fn root_and_aggregate_byte_limits_reject_the_first_excess() {
    let fixture = Fixture::new();
    let host = WorkspaceHostId::new().unwrap();
    let mut roots = Vec::new();
    for index in 0..=MAX_ROOTS {
        let path = fixture.0.join(format!("root{index}"));
        fs::create_dir(&path).unwrap();
        roots.push(LocalSkillRoot::new(path, SkillReferenceScope::Workspace).unwrap());
    }
    assert!(Catalog::new(roots[..MAX_ROOTS].to_vec(), host).is_ok());
    assert!(Catalog::new(roots, host).is_err());
    let bodies = Fixture::new();
    for index in 0..64 {
        bodies.skill(
            &format!("skill{index:03}"),
            &"x".repeat(ResolvedSkill::MAX_INSTRUCTION_BYTES),
        );
    }
    let catalog = catalog(&bodies, host);
    let (entries, status) = catalog.discover(1, &AtomicBool::new(false)).unwrap();
    assert_eq!(entries.len(), 64);
    assert_eq!(status, SkillReferenceSearchStatus::Complete);
    assert!(
        entries
            .iter()
            .all(|entry| entry.availability() == &SkillAvailability::Enabled)
    );
    bodies.skill("first-excess", "x");
    assert!(matches!(
        catalog.discover(2, &AtomicBool::new(false)).unwrap().1,
        SkillReferenceSearchStatus::Incomplete(_)
    ));
}

fn wait_bounded(mut child: std::process::Child) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            outcome => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("local skill test child did not finish: {outcome:?}");
            },
        }
    }
}

// 작성자가 없는 FIFO도 비활성 행으로 처리하며 회귀 시 자식 프로세스를 종료·회수한다.
#[test]
fn fifo_skill_without_writer_is_rejected_without_blocking() {
    use std::process::Command;
    const CHILD_ROOT: &str = "YO_LOCAL_SKILL_FIFO_TEST_ROOT";
    if let Some(path) = std::env::var_os(CHILD_ROOT) {
        let catalog = Catalog::new(
            vec![LocalSkillRoot::new(path.into(), SkillReferenceScope::Workspace).unwrap()],
            WorkspaceHostId::new().unwrap(),
        )
        .unwrap();
        let candidates = rows(&catalog);
        assert_eq!(candidates.len(), 1);
        assert!(
            matches!(candidates[0].availability(), SkillAvailability::Disabled(reason) if reason.contains("not a regular file"))
        );
        return;
    }
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("fifo")).unwrap();
    assert!(
        wait_bounded(
            Command::new("mkfifo")
                .arg(fixture.0.join("fifo/SKILL.md"))
                .spawn()
                .unwrap()
        )
        .success()
    );
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "skill_reference::local::tests::fifo_skill_without_writer_is_rejected_without_blocking",
        ])
        .env(CHILD_ROOT, &fixture.0)
        .spawn()
        .unwrap();
    assert!(wait_bounded(child).success());
}
