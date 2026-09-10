use std::{
    io::{Cursor, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::*;

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = fs::canonicalize(env::temp_dir()).unwrap().join(format!(
            "yo-clipboard-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(2)
}

fn command_error(result: Result<Vec<u8>, CommandError>) -> String {
    match result {
        Err(CommandError::Failed(error)) => error.to_string(),
        Err(CommandError::Missing) => panic!("synthetic process must exist"),
        Ok(_) => panic!("synthetic process must fail"),
    }
}

// 첫 초과 바이트에서 읽기를 중단하며 정확한 4 MiB 입력은 수용한다.
#[test]
fn compressed_source_limit_reads_only_the_first_excess_byte() {
    let mut exact = Cursor::new(vec![42; MAX_SOURCE_BYTES]);
    assert_eq!(
        read_bounded(&mut exact, deadline(), &mut || false)
            .unwrap()
            .len(),
        MAX_SOURCE_BYTES
    );
    let mut excess = Cursor::new(vec![42; MAX_SOURCE_BYTES + 128]);
    let error = read_bounded(&mut excess, deadline(), &mut || false).unwrap_err();
    assert!(error.to_string().contains("4 MiB"));
    assert_eq!(excess.position(), (MAX_SOURCE_BYTES + 1) as u64);
}

// 텍스트와 JPEG는 클립보드 PNG 계약으로 위장할 수 없다.
#[test]
fn clipboard_payload_must_be_png_before_normalization() {
    for bytes in [
        b"ordinary clipboard text".as_slice(),
        &[0xff, 0xd8, 0xff, 0xe0],
    ] {
        assert!(
            prepare_png(bytes, &mut || false)
                .unwrap_err()
                .to_string()
                .contains("PNG image")
        );
    }
}

// 합성 프로세스의 성공 출력을 한 번 정규화하며 원본 바이트 수를 유지한다.
#[test]
fn successful_command_prepares_immutable_pixels() {
    let directory = Directory::new();
    let path = directory.0.join("source.png");
    let png = super::super::encode(&image::RgbaImage::new(2, 3), 1024).unwrap();
    fs::write(&path, &png).unwrap();
    let result = read_command(Command::new("/bin/cat").arg(&path), deadline(), &mut || {
        false
    });
    let bytes = match result {
        Ok(bytes) => bytes,
        Err(_) => panic!("owned PNG must be read"),
    };
    fs::remove_file(path).unwrap();
    let prepared = prepare_png(&bytes, &mut || false).unwrap();
    assert_eq!(prepared.source_byte_length(), png.len() as u64);
    assert_eq!(
        (prepared.snapshot().width(), prepared.snapshot().height()),
        (2, 3)
    );
}

// 실행 파일 부재와 이미지가 없는 정상 종료와 읽기 실패를 구분한다.
#[test]
fn command_missing_empty_and_nonzero_are_distinct() {
    let directory = Directory::new();
    assert!(matches!(
        read_command(
            &mut Command::new(directory.0.join("absent")),
            deadline(),
            &mut || false
        ),
        Err(CommandError::Missing)
    ));
    let empty = command_error(read_command(
        &mut Command::new("/usr/bin/true"),
        deadline(),
        &mut || false,
    ));
    assert!(empty.contains("contains no PNG"));
    let failed = command_error(read_command(
        &mut Command::new("/usr/bin/false"),
        deadline(),
        &mut || false,
    ));
    assert!(failed.contains("could not provide"));
}

// stdout이 닫혀도 종료하지 않는 프로세스는 제한 시간에 종료하고 회수한다.
#[test]
fn deadline_kills_and_reaps_child_after_stdout_eof() {
    let directory = Directory::new();
    let pid_path = directory.0.join("pid");
    let mut command = Command::new("python3");
    command.args(["-c", "import os,sys,time; open(sys.argv[1], 'w').write(str(os.getpid())); os.close(1); time.sleep(30)"]).arg(&pid_path);
    let started = Instant::now();
    let error = command_error(read_command(
        &mut command,
        started + Duration::from_millis(500),
        &mut || false,
    ));
    assert!(error.contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(2));
    let pid: i32 = fs::read_to_string(pid_path).unwrap().parse().unwrap();
    assert_eq!(
        nix::sys::wait::waitpid(
            nix::unistd::Pid::from_raw(pid),
            Some(nix::sys::wait::WaitPidFlag::WNOHANG)
        ),
        Err(Errno::ECHILD)
    );
}

// 읽기 중 취소는 전체 제한 시간을 기다리지 않고 작업 프로세스를 정리한다.
#[test]
fn cancellation_interrupts_pending_process_read() {
    let started = Instant::now();
    let mut command = Command::new("/bin/sleep");
    command.arg("30");
    let error = command_error(read_command(&mut command, deadline(), &mut || {
        started.elapsed() >= Duration::from_millis(50)
    }));
    assert!(error.contains("cancelled"));
    assert!(started.elapsed() < Duration::from_secs(1));
}

// 명시한 개인 소켓에서 요청 본문 없이 받은 원본만 준비 결과에 반영한다.
#[test]
fn explicit_private_socket_returns_one_bounded_snapshot() {
    let directory = Directory::new();
    let path = directory.0.join("clipboard.sock");
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let png = super::super::encode(&image::RgbaImage::new(3, 2), 1024).unwrap();
    let expected = png.clone();
    let server = thread::spawn(move || {
        let end = deadline();
        let (mut client, _) = loop {
            match listener.accept() {
                Ok(client) => break client,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < end);
                    thread::park_timeout(POLL_INTERVAL);
                },
                Err(error) => panic!("owned socket accept: {error}"),
            }
        };
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut request = [0_u8; 1];
        assert_eq!(client.read(&mut request).unwrap(), 0);
        client.write_all(&png).unwrap();
    });
    let bytes = read_socket(&path, deadline(), &mut || false).unwrap();
    server.join().unwrap();
    assert_eq!(bytes, expected);
    let prepared = prepare_png(&bytes, &mut || false).unwrap();
    assert_eq!(prepared.source_byte_length(), expected.len() as u64);
    assert_eq!((prepared.source_width(), prepared.source_height()), (3, 2));
}

// 상대 경로·일반 파일·심볼릭 링크·공개 디렉터리는 접속 전에 거부한다.
#[test]
fn unsafe_socket_paths_are_rejected_before_connect() {
    assert!(validate_socket(Path::new("relative.sock")).is_err());
    let directory = Directory::new();
    let regular = directory.0.join("regular");
    fs::write(&regular, b"not a socket").unwrap();
    assert!(validate_socket(&regular).is_err());
    let socket = directory.0.join("clipboard.sock");
    let _listener = UnixListener::bind(&socket).unwrap();
    validate_socket(&socket).unwrap();
    let symlink = directory.0.join("linked.sock");
    std::os::unix::fs::symlink(&socket, &symlink).unwrap();
    assert!(validate_socket(&symlink).is_err());
    fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(validate_socket(&socket).is_err());
}

// 중간 경로의 링크는 최종 개인 디렉터리가 안전해도 허용하지 않는다.
#[test]
fn intermediate_symlink_ancestor_is_rejected() {
    let directory = Directory::new();
    let real = directory.0.join("real");
    let private = real.join("private");
    fs::create_dir_all(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    let socket = private.join("clipboard.sock");
    let _listener = UnixListener::bind(&socket).unwrap();
    validate_socket(&socket).unwrap();
    let linked = directory.0.join("linked");
    std::os::unix::fs::symlink(&real, &linked).unwrap();
    assert!(validate_socket(&linked.join("private/clipboard.sock")).is_err());
}

// 상위 디렉터리의 외부 쓰기 권한은 하위 개인 디렉터리 교체를 허용하므로 거부한다.
#[test]
fn writable_ancestor_is_rejected_even_with_private_immediate_parent() {
    let directory = Directory::new();
    let ancestor = directory.0.join("ancestor");
    let private = ancestor.join("private");
    fs::create_dir_all(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    let socket = private.join("clipboard.sock");
    let _listener = UnixListener::bind(&socket).unwrap();
    validate_socket(&socket).unwrap();
    for mode in [0o775, 0o777] {
        fs::set_permissions(&ancestor, fs::Permissions::from_mode(mode)).unwrap();
        assert!(validate_socket(&socket).is_err());
    }
}

// 상위 이동 요소를 정규화하지 않고 거부하여 검증한 경로의 의미를 고정한다.
#[test]
fn parent_directory_component_is_rejected() {
    let directory = Directory::new();
    let private = directory.0.join("private");
    fs::create_dir(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    let _listener = UnixListener::bind(private.join("clipboard.sock")).unwrap();
    assert!(validate_socket(&private.join("../private/clipboard.sock")).is_err());
}

// 응답하지 않는 소켓도 동일한 제한 시간과 취소를 적용한다.
#[test]
fn socket_reads_obey_deadline_and_cancellation() {
    let (mut reader, _writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    let started = Instant::now();
    let error = read_bounded(
        &mut reader,
        started + Duration::from_millis(30),
        &mut || false,
    )
    .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(1));
    let error = read_bounded(&mut reader, deadline(), &mut || true).unwrap_err();
    assert!(error.to_string().contains("cancelled"));
}
