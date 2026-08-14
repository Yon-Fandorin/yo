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

use super::{admission::LocalSemanticAdmission, filesystem::LocalToolHost, registry::registry};

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
    let registry = registry().unwrap();
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
    let definition = registry().unwrap().freeze().definitions()[0].clone();

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
