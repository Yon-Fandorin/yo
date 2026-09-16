use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use super::{
    EditRequest, MAX_FILE_BYTES, Scratch, Success, Terminal, UnwindCleanup, WriteRequest,
    catch_failure, execute_edit_after_capture, execute_write, execute_write_after_mode,
    lock_mutation, publish_in_parent,
};
use crate::execution::tools::{filesystem::path::AdmittedPath, tests::TestDirectory};

// waiting mutation은 같은 host lock을 우회하지 않으며 cancellation이 이미 보이면
// filesystem phase에 들어가지 않고 Interrupted 경로를 선택할 수 있습니다.
#[test]
fn mutation_lock_wait_observes_cancellation_without_interleaving() {
    let lock = Arc::new(Mutex::new(()));
    let _held = lock.lock().unwrap();
    assert!(
        lock_mutation(&lock, &AtomicBool::new(true))
            .unwrap()
            .is_none()
    );
}

// rename failure와 prepublication cancellation은 owned scratch를 제거하고 target을
// publish하지 않으며 각각 stable failure와 Interrupted를 구분합니다.
#[test]
fn publication_failure_and_cancellation_cleanup_owned_scratch() {
    let directory = TestDirectory::new();
    fs::create_dir(directory.0.join("target")).unwrap();
    let parent = fs::File::open(&directory.0).unwrap();
    let path = AdmittedPath::new("target".to_owned(), vec![OsString::from("target")]);
    let result = publish_in_parent(
        parent,
        OsString::from("target"),
        None,
        &path,
        b"content",
        0o600,
        &AtomicBool::new(false),
        Success::Write(7),
        UnwindCleanup::default(),
    );
    assert_eq!(
        result.output(),
        r#"{"path":"target","status":"error","error":"publication_failed"}"#
    );

    let parent = fs::File::open(&directory.0).unwrap();
    let cancelled_path =
        AdmittedPath::new("cancelled".to_owned(), vec![OsString::from("cancelled")]);
    let cancelled = publish_in_parent(
        parent,
        OsString::from("cancelled"),
        None,
        &cancelled_path,
        b"content",
        0o600,
        &AtomicBool::new(true),
        Success::Write(7),
        UnwindCleanup::default(),
    );
    assert_eq!(
        cancelled.outcome(),
        yo_core::ToolExecutionOutcome::Interrupted
    );
    assert!(!directory.0.join("cancelled").exists());
    assert!(fs::read_dir(&directory.0).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".yo-write-")
    }));
}

// 외부 same-UID publisher가 scratch 이름을 바꿔치기하면 cleanup은 foreign entry를
// unlink하지 않고 원래 scratch_changed 결과를 유지합니다.
#[test]
fn cleanup_never_unlinks_a_foreign_scratch_replacement() {
    let directory = TestDirectory::new();
    let parent = fs::File::open(&directory.0).unwrap();
    let scratch = Scratch::create(parent, UnwindCleanup::default()).unwrap();
    let scratch_path = directory.0.join(&scratch.name);
    fs::rename(&scratch_path, directory.0.join("moved-owned")).unwrap();
    fs::write(&scratch_path, "foreign").unwrap();

    let result = scratch.finish("target", Terminal::Failed("scratch_changed"));

    assert_eq!(
        result.output(),
        r#"{"path":"target","status":"error","error":"scratch_changed"}"#
    );
    assert_eq!(fs::read_to_string(scratch_path).unwrap(), "foreign");
}

// 첫 capture 뒤 source가 한 byte 커지면 size bound를 먼저 반환하지 않고 fixed phase
// order에 따라 metadata/observed-length 변화가 changed_during_read를 선택합니다.
#[test]
fn edit_growth_precedes_the_stable_size_limit() {
    let directory = TestDirectory::new();
    let source = directory.0.join("growing.txt");
    fs::write(&source, vec![b'x'; MAX_FILE_BYTES]).unwrap();
    let workspace = fs::File::open(&directory.0).unwrap();
    let request = EditRequest {
        path: AdmittedPath::new(
            "growing.txt".to_owned(),
            vec![OsString::from("growing.txt")],
        ),
        edits: vec![super::super::mutation_plan::ExactEdit::new(
            "x".into(),
            "y".into(),
        )],
    };

    let result = execute_edit_after_capture(
        workspace,
        None,
        Arc::new(Mutex::new(())),
        request,
        &AtomicBool::new(false),
        UnwindCleanup::default(),
        || {
            OpenOptions::new()
                .append(true)
                .open(&source)
                .unwrap()
                .write_all(b"y")
                .unwrap();
        },
    );

    assert_eq!(
        result.output(),
        r#"{"path":"growing.txt","status":"error","error":"changed_during_read"}"#
    );
}

// final mode 적용 뒤 panic이 발생해도 Scratch Drop이 exact owned pathname을 한 번
// 정리하고 operation_failed를 보존하며, poisoned lock은 다음 mutation에 재사용됩니다.
#[test]
fn panic_after_final_mode_cleans_scratch_and_does_not_disable_mutation() {
    let directory = TestDirectory::new();
    let lock = Arc::new(Mutex::new(()));
    let cleanup = UnwindCleanup::default();
    let result = catch_failure("panic.txt", &cleanup, || {
        execute_write_after_mode(
            fs::File::open(&directory.0).unwrap(),
            None,
            Arc::clone(&lock),
            WriteRequest {
                path: AdmittedPath::new("panic.txt".to_owned(), vec![OsString::from("panic.txt")]),
                content: "complete content".to_owned(),
            },
            0o600,
            &AtomicBool::new(false),
            cleanup.clone(),
            |_| panic!("injected after-mode cut"),
        )
    });
    assert_eq!(
        result.output(),
        r#"{"path":"panic.txt","status":"error","error":"operation_failed"}"#
    );
    assert!(!directory.0.join("panic.txt").exists());
    assert!(fs::read_dir(&directory.0).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".yo-write-")
    }));

    let next = execute_write(
        fs::File::open(&directory.0).unwrap(),
        None,
        lock,
        WriteRequest {
            path: AdmittedPath::new("next.txt".to_owned(), vec![OsString::from("next.txt")]),
            content: "next".to_owned(),
        },
        0o600,
        &AtomicBool::new(false),
        UnwindCleanup::default(),
    );
    assert_eq!(next.outcome(), yo_core::ToolExecutionOutcome::Completed);
    assert_eq!(
        fs::read_to_string(directory.0.join("next.txt")).unwrap(),
        "next"
    );
}

// panic cleanup 자체가 실패했다고 주입하면 primary operation_failed보다
// cleanup_failed가 우선하며, 성공 cleanup 경로와 결과 선택을 구분합니다.
#[test]
fn panic_cleanup_failure_overrides_the_internal_failure() {
    let directory = TestDirectory::new();
    let cleanup = UnwindCleanup::default();
    cleanup.force_failure();
    let result = catch_failure("panic.txt", &cleanup, || {
        execute_write_after_mode(
            fs::File::open(&directory.0).unwrap(),
            None,
            Arc::new(Mutex::new(())),
            WriteRequest {
                path: AdmittedPath::new("panic.txt".to_owned(), vec![OsString::from("panic.txt")]),
                content: "complete content".to_owned(),
            },
            0o600,
            &AtomicBool::new(false),
            cleanup.clone(),
            |_| panic!("injected after-mode cut"),
        )
    });
    assert_eq!(
        result.output(),
        r#"{"path":"panic.txt","status":"error","error":"cleanup_failed"}"#
    );
}
