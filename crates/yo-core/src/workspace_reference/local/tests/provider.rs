use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, TryRecvError},
    },
    task::{Context, Poll, Wake, Waker},
    thread,
    time::Duration,
};

use super::{
    super::{LocalWorkspaceReferenceProvider, worker},
    support::{TempFixture, host_id},
};
use crate::{
    WorkspaceReferenceKind, WorkspaceReferenceProvider, WorkspaceReferenceProviderPoll,
    WorkspaceReferenceSearchRequest, WorkspaceReferenceSearchStatus,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

struct ChannelWake {
    sender: mpsc::Sender<()>,
    wakes: AtomicUsize,
}

impl Wake for ChannelWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
        let _ = self.sender.send(());
    }
}

fn request(request_id: u64, query: impl Into<String>) -> WorkspaceReferenceSearchRequest {
    WorkspaceReferenceSearchRequest::new(
        request_id,
        request_id + 100,
        request_id as usize,
        0..0,
        "@",
        query,
    )
}

fn wait_for_provider_update(
    provider: &mut LocalWorkspaceReferenceProvider,
    request: WorkspaceReferenceSearchRequest,
) -> (crate::WorkspaceReferenceSearchUpdate, usize) {
    let (wake_sender, wake_receiver) = mpsc::channel();
    let wake = Arc::new(ChannelWake {
        sender: wake_sender,
        wakes: AtomicUsize::new(0),
    });
    let waker = Waker::from(Arc::clone(&wake));
    let mut context = Context::from_waker(&waker);
    assert_eq!(provider.poll_ready(&mut context), Poll::Pending);
    provider.search(request).unwrap();
    wake_receiver
        .recv_timeout(TEST_TIMEOUT)
        .expect("provider update readiness wake");
    assert_eq!(provider.poll_ready(&mut context), Poll::Ready(()));
    let update = match provider.poll().unwrap() {
        WorkspaceReferenceProviderPoll::Update(update) => update,
        WorkspaceReferenceProviderPoll::Pending => {
            panic!("provider wake arrived without a queued update")
        },
    };
    (update, wake.wakes.load(Ordering::SeqCst))
}

// alias root과 canonical root에 같은 host와 요청을 사용해, provider가 같은 typed reference와
// provenance를 반환하고 final update readiness wake를 전달하는지 확인한다.
#[test]
fn public_provider_alias_and_canonical_roots_share_reference_identity() {
    let fixture = TempFixture::new("provider-root");
    fs::create_dir(fixture.path().join("nested")).unwrap();
    fs::write(fixture.path().join("entry.txt"), "entry\n").unwrap();
    let requested_root = fixture.path().join("nested").join("..");
    let canonical_root = fs::canonicalize(fixture.path()).unwrap();
    let host_id = host_id();
    let mut alias_provider =
        LocalWorkspaceReferenceProvider::start(&requested_root, host_id).unwrap();
    let mut canonical_provider =
        LocalWorkspaceReferenceProvider::start(&canonical_root, host_id).unwrap();

    let (alias_update, alias_wake_count) =
        wait_for_provider_update(&mut alias_provider, request(7, "entry"));
    let (canonical_update, canonical_wake_count) =
        wait_for_provider_update(&mut canonical_provider, request(7, "entry"));
    assert!(alias_wake_count >= 1);
    assert!(canonical_wake_count >= 1);
    for update in [&alias_update, &canonical_update] {
        assert_eq!(update.request_id(), 7);
        assert_eq!(update.editor_revision(), 107);
        assert_eq!(update.sequence(), 0);
        assert!(update.is_final());
        assert_eq!(update.status(), &WorkspaceReferenceSearchStatus::Complete);
    }

    let alias_reference = alias_update
        .candidates()
        .iter()
        .find(|candidate| candidate.reference().relative_path() == "entry.txt")
        .expect("alias entry candidate")
        .reference();
    let canonical_reference = canonical_update
        .candidates()
        .iter()
        .find(|candidate| candidate.reference().relative_path() == "entry.txt")
        .expect("canonical entry candidate")
        .reference();
    assert_eq!(alias_reference, canonical_reference);
    assert_eq!(alias_reference.relative_path(), "entry.txt");
    assert_eq!(alias_reference.kind(), WorkspaceReferenceKind::File);
    assert_eq!(alias_reference.identity(), canonical_reference.identity());
    assert_eq!(
        alias_reference.root_identity(),
        canonical_reference.root_identity()
    );
    assert_eq!(
        alias_reference.execution_environment_identity(),
        canonical_reference.execution_environment_identity()
    );
    assert_eq!(
        alias_reference.workspace_identity(),
        canonical_reference.workspace_identity()
    );
    for provenance in [
        alias_reference.identity(),
        alias_reference.root_identity(),
        alias_reference.execution_environment_identity(),
        alias_reference.workspace_identity(),
    ] {
        assert!(!provenance.is_empty());
    }
}

// 두 요청을 worker 큐에 미리 넣고 실행해, 최신 요청 하나만 final update로 처리되는지 확인한다.
#[test]
fn worker_coalesces_queued_requests_to_the_newest_request() {
    let fixture = TempFixture::new("worker-coalescing");
    let (request_sender, request_receiver) = mpsc::channel();
    let (update_sender, update_receiver) = mpsc::channel();
    let readiness = Arc::new(crate::readiness::Readiness::new());
    request_sender.send(request(1, "old")).unwrap();
    request_sender.send(request(2, "new")).unwrap();

    let worker_thread = thread::spawn({
        let root = fixture.path().to_path_buf();
        let readiness = Arc::clone(&readiness);
        move || worker(root, host_id(), request_receiver, update_sender, &readiness)
    });
    drop(request_sender);

    let update = update_receiver
        .recv_timeout(TEST_TIMEOUT)
        .expect("coalesced worker update");
    assert_eq!(update.request_id(), 2);
    assert_eq!(update.editor_revision(), 102);
    assert!(update.is_final());
    assert_eq!(update.status(), &WorkspaceReferenceSearchStatus::Complete);
    assert_eq!(update.candidates().len(), 0);
    worker_thread.join().unwrap();
    assert_eq!(update_receiver.try_recv(), Err(TryRecvError::Disconnected));
}

// request sender를 닫거나 update receiver를 버리고, direct worker가 제한된 시간 안에 종료 이벤트를
// 보내는지 확인한다.
#[test]
fn worker_exits_when_request_sender_or_update_receiver_closes() {
    let request_sender_case = TempFixture::new("worker-request-close");
    let (request_sender, request_receiver) = mpsc::channel();
    let (update_sender, _update_receiver) = mpsc::channel();
    let readiness = Arc::new(crate::readiness::Readiness::new());
    let (finished_sender, finished_receiver) = mpsc::channel();
    let worker_thread = thread::spawn({
        let root = request_sender_case.path().to_path_buf();
        let readiness = Arc::clone(&readiness);
        move || {
            worker(root, host_id(), request_receiver, update_sender, &readiness);
            finished_sender.send(()).unwrap();
        }
    });
    drop(request_sender);
    finished_receiver
        .recv_timeout(TEST_TIMEOUT)
        .expect("worker exits after request sender closes");
    worker_thread.join().unwrap();

    let update_receiver_case = TempFixture::new("worker-update-close");
    let (request_sender, request_receiver) = mpsc::channel();
    let (update_sender, update_receiver) = mpsc::channel();
    drop(update_receiver);
    let readiness = Arc::new(crate::readiness::Readiness::new());
    let (finished_sender, finished_receiver) = mpsc::channel();
    let worker_thread = thread::spawn({
        let root = update_receiver_case.path().to_path_buf();
        let readiness = Arc::clone(&readiness);
        move || {
            worker(root, host_id(), request_receiver, update_sender, &readiness);
            finished_sender.send(()).unwrap();
        }
    });
    request_sender.send(request(3, "closed-output")).unwrap();
    finished_receiver
        .recv_timeout(TEST_TIMEOUT)
        .expect("worker exits after update receiver closes");
    worker_thread.join().unwrap();
    drop(request_sender);
}

// 디렉터리가 아닌 루트의 검색은 후보 없이 종료되고 오류에 해당 루트 경로가 포함된다.
#[test]
fn public_provider_reports_non_directory_root_as_final_failed_update() {
    let fixture = TempFixture::new("provider-file-root");
    let file_root = fixture.path().join("root-file");
    fs::write(&file_root, "not a directory\n").unwrap();
    let mut provider = LocalWorkspaceReferenceProvider::start(&file_root, host_id()).unwrap();
    let (update, wake_count) = wait_for_provider_update(&mut provider, request(8, ""));

    assert!(wake_count >= 1);
    assert_eq!(update.request_id(), 8);
    assert!(update.is_final());
    assert!(update.candidates().is_empty());
    match update.status() {
        WorkspaceReferenceSearchStatus::Failed(error) => {
            assert!(!error.trim().is_empty());
            assert!(error.contains(&file_root.display().to_string()));
        },
        status => panic!("expected final failed update, got {status:?}"),
    }
}

// 출력 링크는 일반 디렉터리·명시적 목록을 검증하고 삭제·심볼릭 링크·경로 이탈·취소를 배제한다.
#[test]
fn output_links_revalidate_files_and_containment() {
    use std::os::unix::fs::symlink;
    let fixture = TempFixture::new("output-links");
    let root = fixture.path().canonicalize().unwrap();
    fs::write(root.join("한글 file.rs"), "body").unwrap();
    fs::create_dir(root.join("directory")).unwrap();
    let paths = b"directory\0../outside\0/absolute\0missing\0";
    assert!(
        LocalWorkspaceReferenceProvider::output_links(&root, Some(paths), || false)
            .unwrap()
            .is_empty()
    );
    let links = LocalWorkspaceReferenceProvider::output_links(&root, None, || false).unwrap();
    assert_eq!(links.get("한글 file.rs"), Some(&root.join("한글 file.rs")));
    fs::remove_file(root.join("한글 file.rs")).unwrap();
    symlink("/etc/passwd", root.join("한글 file.rs")).unwrap();
    assert!(
        LocalWorkspaceReferenceProvider::output_links(
            &root,
            Some("한글 file.rs\0".as_bytes()),
            || false
        )
        .unwrap()
        .is_empty()
    );
    assert!(LocalWorkspaceReferenceProvider::output_links(&root, None, || true).is_err());
    fs::create_dir(root.join(".git")).unwrap();
    assert!(
        LocalWorkspaceReferenceProvider::output_links(
            &root,
            Some(&b"missing\0".repeat(8192)),
            || false
        )
        .is_ok()
    );
    assert!(LocalWorkspaceReferenceProvider::output_links(&root, None, || false).is_err());
    assert!(
        LocalWorkspaceReferenceProvider::output_links(&root, Some(&b"x\0".repeat(8193)), || false)
            .is_err()
    );
}

// 대기 중인 디렉터리가 외부 심볼릭 링크로 교체되면 외부 항목을 열거하기 전에 실패한다.
#[test]
fn output_links_reject_queued_directory_replacement() {
    use std::{cell::Cell, os::unix::fs::symlink};

    // Exercise both the final queued component and an ancestor of a queued child.
    for replace_at in [4, 6] {
        let fixture = TempFixture::new("output-directory-replacement");
        let outside = TempFixture::new("output-directory-outside");
        let root = fixture.path().canonicalize().unwrap();
        fs::create_dir(root.join("queued")).unwrap();
        fs::create_dir(outside.path().join("nested")).unwrap();
        if replace_at == 6 {
            fs::create_dir(root.join("queued/nested")).unwrap();
        }
        fs::write(outside.path().join("outside.txt"), "outside").unwrap();
        let checks = Cell::new(0);
        let result = LocalWorkspaceReferenceProvider::output_links(&root, None, || {
            let check = checks.get() + 1;
            checks.set(check);
            // Initial check, then one dequeue and one entry check per directory.
            if check == replace_at {
                fs::rename(root.join("queued"), root.join("retired")).unwrap();
                symlink(outside.path(), root.join("queued")).unwrap();
            }
            false
        });
        assert!(result.is_err(), "replacement must fail before enumeration");
        assert_eq!(
            checks.get(),
            replace_at,
            "no outside entry may be enumerated"
        );
        assert_eq!(
            fs::read(outside.path().join("outside.txt")).unwrap(),
            b"outside"
        );
    }
}

fn selected_input(root: &std::path::Path, path: &str) -> crate::UserInput {
    use crate::{InputReference, UserInput, workspace_reference_projection};
    let mut provider = LocalWorkspaceReferenceProvider::start(root, host_id()).unwrap();
    let (update, _) = wait_for_provider_update(&mut provider, request(1, path));
    let reference = update
        .candidates()
        .iter()
        .find(|candidate| candidate.reference().relative_path() == path)
        .unwrap()
        .reference()
        .clone();
    let projection = workspace_reference_projection(&reference);
    UserInput::with_references(
        projection.clone(),
        vec![InputReference::workspace(0..projection.len(), reference)],
    )
    .unwrap()
}

// 선택 뒤 내용이나 ignore 설정만 바뀌면 참조는 유효하다. 삭제·종류 변경·symlink 치환은
// 제출 시점의 typed 거절로 드러나며 파일 내용을 읽거나 다른 경로로 바꾸지 않는다.
#[test]
fn local_admission_revalidates_selected_paths_without_reading_contents() {
    use crate::{InputAdmissionHost, LocalWorkspaceInputAdmission, SubmissionRejectionKind};
    let fixture = TempFixture::new("reference-admission");
    let file = fixture.path().join("readme.md");
    fs::write(&file, "old").unwrap();
    let input = selected_input(fixture.path(), "readme.md");
    let host = LocalWorkspaceInputAdmission::new(fixture.path(), host_id()).unwrap();
    host.validate(&input).unwrap();
    fs::write(&file, "new contents").unwrap();
    fs::write(fixture.path().join(".gitignore"), "readme.md\n").unwrap();
    host.validate(&input).unwrap();
    fs::remove_file(&file).unwrap();
    assert_eq!(
        host.validate(&input).unwrap_err().kind(),
        SubmissionRejectionKind::StaleReference
    );
    fs::create_dir(&file).unwrap();
    assert_eq!(
        host.validate(&input).unwrap_err().kind(),
        SubmissionRejectionKind::StaleReference
    );
    fs::remove_dir(&file).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", &file).unwrap();
    assert_eq!(
        host.validate(&input).unwrap_err().kind(),
        SubmissionRejectionKind::StaleReference
    );
}

// 같은 문자열의 root 경로가 다른 directory로 바뀌거나 중간 directory가 symlink가 되면
// 원래 선택 신원은 새 파일로 대체되지 않는다.
#[test]
fn local_admission_rejects_root_and_ancestor_replacement() {
    use crate::{InputAdmissionHost, LocalWorkspaceInputAdmission, SubmissionRejectionKind};
    let fixture = TempFixture::new("reference-root");
    let root = fixture.path().join("workspace");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "original").unwrap();
    let input = selected_input(&root, "src/lib.rs");
    let host = LocalWorkspaceInputAdmission::new(&root, host_id()).unwrap();
    let old = fixture.path().join("original");
    fs::rename(&root, &old).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "replacement").unwrap();
    assert_eq!(
        host.validate(&input).unwrap_err().kind(),
        SubmissionRejectionKind::StaleReference
    );
    fs::remove_dir_all(&root).unwrap();
    fs::rename(&old, &root).unwrap();
    host.validate(&input).unwrap();
    fs::rename(root.join("src"), root.join("original-src")).unwrap();
    std::os::unix::fs::symlink("original-src", root.join("src")).unwrap();
    assert_eq!(
        host.validate(&input).unwrap_err().kind(),
        SubmissionRejectionKind::StaleReference
    );
}

// 다른 host 신원은 거절하고 최대 128개까지 검증하되 129번째 참조부터 전체 요청을 거절한다.
#[test]
fn local_admission_checks_environment_and_first_excess_reference() {
    use crate::{
        InputAdmissionHost, InputReference, LocalWorkspaceInputAdmission, SubmissionRejectionKind,
        UserInput, WorkspaceHostId,
    };
    let fixture = TempFixture::new("reference-bounds");
    fs::write(fixture.path().join("a"), "x").unwrap();
    let input = selected_input(fixture.path(), "a");
    let other_host =
        LocalWorkspaceInputAdmission::new(fixture.path(), WorkspaceHostId::new().unwrap()).unwrap();
    assert_eq!(
        other_host.validate(&input).unwrap_err().kind(),
        SubmissionRejectionKind::EnvironmentUnavailable
    );
    let host = LocalWorkspaceInputAdmission::new(fixture.path(), host_id()).unwrap();
    let reference = input.references()[0].workspace_reference().unwrap();
    for count in [128, 129] {
        let text = "@a ".repeat(count);
        let references = (0..count)
            .map(|index| InputReference::workspace(index * 3..index * 3 + 2, reference.clone()))
            .collect();
        let input = UserInput::with_references(text, references).unwrap();
        if count == 128 {
            host.validate(&input).unwrap();
        } else {
            assert_eq!(
                host.validate(&input).unwrap_err().kind(),
                SubmissionRejectionKind::OverBudget
            );
        }
    }
}

// 접근 권한이 사라진 위치가 leaf·상위 디렉터리·루트 중 어디든 Unauthorized로 분류한다.
// 실행 계정이 권한을 우회하는 환경은 이 파일 권한 시나리오를 실행하지 않는다.
#[test]
fn local_admission_preserves_permission_denials_at_every_path_level() {
    use std::os::unix::fs::PermissionsExt;

    use crate::{InputAdmissionHost, LocalWorkspaceInputAdmission, SubmissionRejectionKind};
    let fixture = TempFixture::new("reference-permissions");
    let root = fixture.path().join("workspace");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a"), "contents").unwrap();
    let input = selected_input(&root, "src/a");
    let host = LocalWorkspaceInputAdmission::new(&root, host_id()).unwrap();
    for path in [root.join("src/a"), root.join("src"), root.clone()] {
        let original = fs::metadata(&path).unwrap().permissions();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let result = host.validate(&input);
        let permission_enforced = fs::File::open(&path).is_err();
        fs::set_permissions(&path, original).unwrap();
        if permission_enforced {
            assert_eq!(
                result.unwrap_err().kind(),
                SubmissionRejectionKind::Unauthorized
            );
        }
        host.validate(&input).unwrap();
    }
}
