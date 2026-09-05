use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use yo_core::{
    SessionId, ToolApprovalRequirement, ToolExecution, ToolExecutionHost, ToolExecutionOutcome,
    ToolExecutionPoll, ToolExecutionRequest, ToolExecutionResult, ToolRegistry,
    ToolSemanticAdmission, TurnId, TurnRef,
};

use super::{
    admission::LocalSemanticAdmission,
    filesystem::LocalToolHost,
    registry::{LocalToolRegistryRevision, registry},
};

pub(super) struct TestDirectory(pub(super) PathBuf);

impl TestDirectory {
    pub(super) fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("yo-local-tools-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn request(
    registry: &ToolRegistry,
    name: &str,
    arguments: &str,
) -> ToolExecutionRequest {
    ToolExecutionRequest {
        turn: TurnRef::new(
            SessionId::new().unwrap(),
            TurnId::new(std::num::NonZeroU64::new(1).unwrap()),
        ),
        call: registry
            .freeze()
            .validate_call("call-1", name, arguments, 4096)
            .unwrap(),
        maximum_output_bytes: 4096,
        absolute_execution_timeout: None,
    }
}

pub(super) fn finish(execution: &mut dyn ToolExecution) -> ToolExecutionResult {
    for _ in 0..1_000 {
        if execution.poll().unwrap() == ToolExecutionPoll::Ready {
            let result = execution.take_result().unwrap();
            execution.shutdown().unwrap();
            return result;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("local tool did not finish")
}

// process effect는 명시적 승인 대상으로 등록되고 완료와 취소가 bounded result로 닫힌다.
#[test]
fn command_execution_is_approval_bound_and_cancellable() {
    let directory = TestDirectory::new();
    let credential = directory.0.join("credentials.yaml");
    let registry = registry(LocalToolRegistryRevision::BasicFiles).unwrap();
    let frozen = registry.freeze();
    let run = frozen
        .definitions()
        .iter()
        .find(|definition| definition.wire_name() == "run_command")
        .unwrap();
    assert_eq!(run.approval(), ToolApprovalRequirement::Required);
    let mut host = LocalToolHost::new(&directory.0, &credential).unwrap();
    let mut execution = host
        .start(request(
            &registry,
            "run_command",
            r#"{"command":"printf done"}"#,
        ))
        .unwrap();
    let result = finish(execution.as_mut());
    assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
    assert!(result.output().contains("done"));

    let started = Instant::now();
    let mut background = host
        .start(request(
            &registry,
            "run_command",
            r#"{"command":"sleep 5 &"}"#,
        ))
        .unwrap();
    assert_eq!(
        finish(background.as_mut()).outcome(),
        ToolExecutionOutcome::Completed
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a background descendant retained the output pipes"
    );

    let mut cancelled = host
        .start(request(
            &registry,
            "run_command",
            r#"{"command":"sleep 5 & wait"}"#,
        ))
        .unwrap();
    cancelled.cancel();
    assert_eq!(
        finish(cancelled.as_mut()).outcome(),
        ToolExecutionOutcome::Interrupted
    );
}

// 선택한 API key가 tool output에 섞이면 replay나 transcript에 들어가기 전에 거부한다.
#[test]
fn semantic_admission_rejects_selected_credential_material() {
    let admission = LocalSemanticAdmission::new(
        yo_core::CredentialStore::new([
            (
                (
                    yo_core::ProviderId::new("openrouter").unwrap(),
                    yo_core::AccountId::new("default").unwrap(),
                ),
                yo_core::ApiCredential::new("sk-sensitive").unwrap(),
            ),
            (
                (
                    yo_core::ProviderId::new("qwencloud").unwrap(),
                    yo_core::AccountId::new("default").unwrap(),
                ),
                yo_core::ApiCredential::new("sk-other-account").unwrap(),
            ),
        ])
        .unwrap(),
    );
    let definition = registry(LocalToolRegistryRevision::BasicFiles)
        .unwrap()
        .freeze()
        .definitions()[0]
        .clone();

    assert!(admission.admit_output(&definition, "safe").is_ok());
    assert!(
        admission
            .admit_output(&definition, "prefix sk-sensitive suffix")
            .is_err()
    );
    assert!(
        admission
            .admit_output(&definition, "prefix sk-other-account suffix")
            .is_err()
    );
}

// schema subset에 표현하지 않은 batch/mutation bound도 같은 semantic-admission gate에서
// execution attempt 전에 거절되고, 안전한 완전 JSON은 byte-for-byte 보존됩니다.
#[test]
fn semantic_admission_enforces_concrete_file_tool_bounds() {
    let admission = LocalSemanticAdmission::new(yo_core::CredentialStore::default());
    let registry = registry(LocalToolRegistryRevision::BasicFiles)
        .unwrap()
        .freeze();
    let read = registry
        .definitions()
        .iter()
        .find(|definition| definition.wire_name() == "read_files")
        .unwrap();
    let valid = r#"{"files":[{"path":"src/lib.rs","limit":400}]}"#;

    assert_eq!(admission.admit_arguments(read, valid).unwrap(), valid);
    assert!(
        admission
            .admit_arguments(read, r#"{"files":[{"path":"src/lib.rs","limit":401}]}"#)
            .is_err()
    );
}

// batch reader는 요청 순서와 duplicate window를 유지하고, 한 파일의 부재가 성공한
// sibling을 지우지 않으며 다음 unread line을 compact JSON으로 알려 줍니다.
#[test]
fn batch_read_returns_ordered_windows_and_per_file_errors() {
    let directory = TestDirectory::new();
    fs::write(directory.0.join("notes.txt"), "one\ntwo\nthree\n").unwrap();
    let credential = directory.0.join("credentials.yaml");
    let registry = registry(LocalToolRegistryRevision::BasicFiles).unwrap();
    let mut host = LocalToolHost::new(&directory.0, &credential).unwrap();
    let mut execution = host
        .start(request(
            &registry,
            "read_files",
            r#"{"files":[{"path":"notes.txt","offset":2,"limit":1},{"path":"missing.txt"},{"path":"notes.txt","offset":3}]}"#,
        ))
        .unwrap();

    assert_eq!(
        finish(execution.as_mut()).output(),
        r#"{"results":[{"path":"notes.txt","status":"ok","start":2,"end":2,"total":3,"next_offset":3,"content":"two\n"},{"path":"missing.txt","status":"error","error":"unavailable"},{"path":"notes.txt","status":"ok","start":3,"end":3,"total":3,"content":"three\n"}]}"#
    );
}

// numeric/item/path semantic bounds는 어떤 파일도 열기 전에 complete call을 거절하고
// schema에 없는 host bound가 실행 중 조용히 clamp되지 않도록 합니다.
#[test]
fn batch_read_rejects_out_of_range_windows_before_execution() {
    let directory = TestDirectory::new();
    fs::write(directory.0.join("notes.txt"), "one\n").unwrap();
    let registry = registry(LocalToolRegistryRevision::BasicFiles).unwrap();
    let mut host = LocalToolHost::new(&directory.0, &directory.0.join("credentials.yaml")).unwrap();

    assert!(
        host.start(request(
            &registry,
            "read_files",
            r#"{"files":[{"path":"notes.txt","limit":401}]}"#,
        ))
        .is_err()
    );
    assert!(
        host.start(request(
            &registry,
            "read_files",
            r#"{"files":[{"path":"./"}]}"#,
        ))
        .is_err()
    );
}

// exact edits는 원본에서 위치를 모두 결정한 뒤 한 번만 publish하고, write_file은 새
// 파일 생성과 기존 permission 보존을 같은 compact success protocol로 닫습니다.
#[test]
fn mutation_tools_publish_complete_files_and_preserve_replacement_mode() {
    use std::os::unix::fs::PermissionsExt;

    let directory = TestDirectory::new();
    let credential = directory.0.join("credentials.yaml");
    let edited = directory.0.join("edited.txt");
    let written = directory.0.join("written.txt");
    fs::write(&edited, "alpha beta gamma").unwrap();
    fs::write(&written, "old").unwrap();
    fs::set_permissions(&written, fs::Permissions::from_mode(0o640)).unwrap();
    let registry = registry(LocalToolRegistryRevision::BasicFiles).unwrap();
    let mut host = LocalToolHost::new(&directory.0, &credential).unwrap();

    let mut edit = host
        .start(request(
            &registry,
            "edit_file",
            r#"{"path":"edited.txt","edits":[{"oldText":"alpha","newText":"A"},{"oldText":"gamma","newText":"G"}]}"#,
        ))
        .unwrap();
    assert_eq!(
        finish(edit.as_mut()).output(),
        r#"{"path":"edited.txt","status":"ok","replacements":2}"#
    );
    assert_eq!(fs::read_to_string(&edited).unwrap(), "A beta G");

    let mut write = host
        .start(request(
            &registry,
            "write_file",
            r#"{"path":"written.txt","content":"complete"}"#,
        ))
        .unwrap();
    assert_eq!(
        finish(write.as_mut()).output(),
        r#"{"path":"written.txt","status":"ok","bytes":8}"#
    );
    assert_eq!(fs::read_to_string(&written).unwrap(), "complete");
    assert_eq!(
        fs::metadata(&written).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(fs::read_dir(&directory.0).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".yo-write-")
    }));
}

// mutation failure는 target bytes를 보존하고 exact condition class를 반환하며, credential
// inode는 hard link로 다른 이름을 얻어도 write target이 될 수 없습니다.
#[test]
fn mutation_failures_preserve_targets_and_credential_identity() {
    let directory = TestDirectory::new();
    let target = directory.0.join("target.txt");
    let credential = directory.0.join("credentials.yaml");
    fs::write(&target, "aaa").unwrap();
    fs::write(&credential, "secret").unwrap();
    fs::hard_link(&credential, directory.0.join("alias.txt")).unwrap();
    let registry = registry(LocalToolRegistryRevision::BasicFiles).unwrap();
    let mut host = LocalToolHost::new(&directory.0, &credential).unwrap();

    let mut ambiguous = host
        .start(request(
            &registry,
            "edit_file",
            r#"{"path":"target.txt","edits":[{"oldText":"aa","newText":"x"}]}"#,
        ))
        .unwrap();
    assert_eq!(
        finish(ambiguous.as_mut()).output(),
        r#"{"path":"target.txt","status":"error","error":"match_ambiguous"}"#
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "aaa");

    let mut denied = host
        .start(request(
            &registry,
            "write_file",
            r#"{"path":"alias.txt","content":"replace"}"#,
        ))
        .unwrap();
    assert_eq!(
        finish(denied.as_mut()).output(),
        r#"{"path":"alias.txt","status":"error","error":"unavailable"}"#
    );
    assert_eq!(fs::read_to_string(&credential).unwrap(), "secret");
}
