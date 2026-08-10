use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    sync::{Arc, Barrier},
    thread,
};

use super::{
    super::LocalWorkspaceHostIdentity,
    support::{TestDirectory, create_user_only_directory},
};

// 첫 open은 user-only 상태 디렉터리와 완결된 ID 파일을 만들고, 재실행은 새 ID를
// 만들지 않고 같은 값을 읽어 Session의 Host 소속이 안정적으로 유지되는지 검증합니다.
#[test]
fn creates_once_and_reopens_the_same_user_only_identity() {
    let directory = TestDirectory::new("stable");

    let first = LocalWorkspaceHostIdentity::open(directory.path()).unwrap();
    let second = LocalWorkspaceHostIdentity::open(directory.path()).unwrap();

    assert_eq!(second, first);
    assert_eq!(
        fs::read_to_string(directory.path().join("host-id")).unwrap(),
        format!("yo.workspace-host-id/v1 {}\n", first.id())
    );
    assert_eq!(
        fs::metadata(directory.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(directory.path().join("host-id"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

// 읽기 전용 Session 명령이 처음 실행된 머신에서는 Host 상태 디렉터리나 ID 파일을
// 만들지 않고 "기존 identity 없음"만 관찰해야 새 Session을 암묵적으로 시작하지 않는다.
#[test]
fn read_only_open_reports_a_missing_identity_without_creating_state() {
    let directory = TestDirectory::new("read-only-missing");
    let root = directory.path().join("host");

    let identity = LocalWorkspaceHostIdentity::open_existing(&root).unwrap();

    assert_eq!(identity, None);
    assert!(!root.exists());
}

// writer가 이미 만든 identity는 read-only open에서도 같은 UUID로 읽되 파일 내용과
// 수정 시각을 바꾸지 않아 목록 조회가 저장소 mutation을 일으키지 않는다.
#[test]
fn read_only_open_reuses_an_existing_identity_without_rewriting_it() {
    let directory = TestDirectory::new("read-only-existing");
    let created = LocalWorkspaceHostIdentity::open(directory.path()).unwrap();
    let path = directory.path().join("host-id");
    let before = fs::metadata(&path).unwrap().modified().unwrap();

    let observed = LocalWorkspaceHostIdentity::open_existing(directory.path()).unwrap();

    assert_eq!(observed, Some(created));
    assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), before);
}

// 여러 실행 흐름의 동시 opener가 모두 서로 다른 임시 후보를 만들더라도
// 완결된 final 파일 하나만 채택하고 모든 호출자가 같은 Host ID를 관찰하는지 검증합니다.
#[test]
fn concurrent_first_openers_converge_on_one_complete_identity() {
    let directory = TestDirectory::new("concurrent");
    let path = Arc::new(directory.path().to_owned());
    let barrier = Arc::new(Barrier::new(8));
    let workers = (0..8)
        .map(|_| {
            let path = Arc::clone(&path);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                LocalWorkspaceHostIdentity::open(path.as_path())
                    .unwrap()
                    .id()
            })
        })
        .collect::<Vec<_>>();
    let ids = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();

    assert!(ids.iter().all(|id| *id == ids[0]));
    assert_eq!(
        fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != "host-id")
            .count(),
        0
    );
}

// 0000, 0500, 0300처럼 서로 다른 owner-only 중간 mode를 만드는 제한적 umask에서도
// 동시 opener가 chmod 완료를 기다려 같은 ID와 정확한 0700/0600 경계로 수렴합니다.
#[test]
fn nested_creation_establishes_exact_modes_under_a_restrictive_umask() {
    const CHILD_PATH: &str = "YO_HOST_UMASK_TEST_PATH";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        let path = Arc::new(PathBuf::from(path));
        let barrier = Arc::new(Barrier::new(8));
        let workers = (0..8)
            .map(|_| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    LocalWorkspaceHostIdentity::open(path.as_path())
                        .unwrap()
                        .id()
                })
            })
            .collect::<Vec<_>>();
        let ids = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert!(ids.iter().all(|id| *id == ids[0]));
        return;
    }

    let directory = TestDirectory::new("restrictive-umask");
    create_user_only_directory(directory.path());
    for mask in ["0777", "0277", "0477"] {
        let root = directory.path().join(mask).join("one/two/host");
        let status = Command::new("sh")
            .arg("-c")
            .arg("umask \"$1\"; shift; exec \"$@\"")
            .arg("sh")
            .arg(mask)
            .arg(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("host::tests::persistence::nested_creation_establishes_exact_modes_under_a_restrictive_umask")
            .arg("--nocapture")
            .env(CHILD_PATH, &root)
            .status()
            .unwrap();

        assert!(status.success(), "the {mask} umask child must converge");
        for path in [
            directory.path().join(mask),
            directory.path().join(mask).join("one"),
            directory.path().join(mask).join("one/two"),
            root.clone(),
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        assert_eq!(
            fs::metadata(root.join("host-id"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
