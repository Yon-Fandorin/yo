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

// file root에 한 요청을 보내, final Failed update가 non-directory root의 `.git` context를
// 포함하는지 확인한다.
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
            assert!(error.contains(&file_root.join(".git").display().to_string()));
        },
        status => panic!("expected final failed update, got {status:?}"),
    }
}
