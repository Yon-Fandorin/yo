use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::{session, submission, turn};
use crate::{
    AgentCommand, AgentRuntime, BackendCapabilities, BackendFailure, BackendFailureKind,
    BackendScriptStep, ContinuationStrategy, ImageInputCapability, InputAdmissionHost, InputImage,
    InputImageHistory, InputImageSnapshot, ResolvedSkill, RuntimeError, ScriptedBackend,
    SubmissionRejection, SubmissionRejectionKind, UserInput,
};

fn input() -> UserInput {
    let snapshot: InputImageSnapshot = serde_json::from_str(r#"{"profile":"yo.input-image-rgba8-triangle/v1","mime_type":"image/png","width":1,"height":1,"byte_length":70,"sha256":"sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}"#).unwrap();
    UserInput::new("[image]")
        .with_images(vec![InputImage::new(0..7, 70, snapshot).unwrap()])
        .unwrap()
}

struct UnpreparedHost(Arc<AtomicUsize>);
impl InputAdmissionHost for UnpreparedHost {
    fn validate(&self, _: &UserInput) -> Result<(), SubmissionRejection> {
        Ok(())
    }
    fn prepare(&self, _: &UserInput) -> Result<Option<ResolvedSkill>, SubmissionRejection> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(None)
    }
}

// 능력 미확인·미지원·용량 초과·host 증거 부재는 backend와 스킬 본문 준비 전에 구별해 거절한다.
#[test]
fn image_rejections_preserve_backend_and_preparation_boundaries() {
    let supported = ImageInputCapability::Supported {
        maximum_occurrences: 16,
        maximum_image_bytes: 9 * 1024 * 1024,
        maximum_input_bytes: 9 * 1024 * 1024,
    };
    for (capability, expected) in [
        (
            ImageInputCapability::Unknown,
            SubmissionRejectionKind::ImageCapabilityUnknown,
        ),
        (
            ImageInputCapability::Unsupported,
            SubmissionRejectionKind::ImageUnsupported,
        ),
        (
            ImageInputCapability::Supported {
                maximum_occurrences: 16,
                maximum_image_bytes: 69,
                maximum_input_bytes: 70,
            },
            SubmissionRejectionKind::OverBudget,
        ),
        (supported, SubmissionRejectionKind::EnvironmentUnavailable),
    ] {
        let session_id = session(91);
        let create = AgentCommand::CreateSession { session_id };
        let text = AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::new("retry"),
        };
        let backend = ScriptedBackend::new([
            BackendScriptStep::AcceptCommand(create.clone()),
            BackendScriptStep::AcceptCommand(text.clone()),
            BackendScriptStep::Shutdown(Ok(())),
        ])
        .with_capabilities(BackendCapabilities::none().with_image_input(capability));
        let mut runtime = AgentRuntime::new(backend);
        let prepares = Arc::new(AtomicUsize::new(0));
        runtime
            .configure_input_admission(Box::new(UnpreparedHost(Arc::clone(&prepares))))
            .unwrap();
        runtime.execute_command(create).unwrap();
        let error = runtime
            .execute_submission(
                AgentCommand::StartTurn {
                    turn: turn(session_id, 1),
                    input: input(),
                },
                submission(91),
            )
            .unwrap_err();
        assert!(
            matches!(error, RuntimeError::InputRejected(ref rejection) if rejection.kind() == expected),
            "{error}"
        );
        assert_eq!(prepares.load(Ordering::Relaxed), 0);
        runtime.execute_submission(text, submission(92)).unwrap();
        runtime.shutdown().unwrap();
    }
}

// BackendManagedState에서 상속 archive가 Unknown이면 image가 없는 다음 text도 capability를
// 다시 확인하고, 같은 submission identity로 보수적 거절 뒤 재시도할 수 있어야 합니다.
#[test]
fn unknown_image_history_guards_later_text_without_consuming_submission() {
    let session_id = session(92);
    let create = AgentCommand::CreateSession { session_id };
    let text = AgentCommand::StartTurn {
        turn: turn(session_id, 1),
        input: UserInput::new("retry after inherited history"),
    };
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(create.clone()),
        BackendScriptStep::AcceptCommand(text.clone()),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut runtime = AgentRuntime::new(backend);
    runtime.execute_command(create).unwrap();
    runtime.continuation_strategy = Some(ContinuationStrategy::BackendManagedState);
    runtime.input_image_history = InputImageHistory::Unknown;
    let submission = submission(93);

    let error = runtime
        .execute_submission(text.clone(), submission)
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::InputRejected(ref rejection)
            if rejection.kind() == SubmissionRejectionKind::ImageCapabilityUnknown
    ));
    runtime.input_image_history = InputImageHistory::TextOnly;
    runtime.execute_submission(text, submission).unwrap();
    runtime.shutdown().unwrap();
}

// adapter의 outbound image budget rejection은 core InputRejected로 매핑되지만 submission ID를
// 소비하지 않아 같은 immutable 입력을 보정하거나 재시도할 수 있어야 합니다.
#[test]
fn input_over_budget_failure_preserves_submission_identity_for_retry() {
    let session_id = session(93);
    let create = AgentCommand::CreateSession { session_id };
    let text = AgentCommand::StartTurn {
        turn: turn(session_id, 1),
        input: UserInput::new("retry over budget"),
    };
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(create.clone()),
        BackendScriptStep::RejectCommand {
            command: text.clone(),
            failure: BackendFailure::new(
                BackendFailureKind::InputOverBudget,
                "encoded request exceeds the Codex JSONL boundary",
            ),
        },
        BackendScriptStep::AcceptCommand(text.clone()),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut runtime = AgentRuntime::new(backend);
    runtime.execute_command(create).unwrap();
    let submission = submission(94);

    let error = runtime
        .execute_submission(text.clone(), submission)
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::InputRejected(ref rejection)
            if rejection.kind() == SubmissionRejectionKind::OverBudget
    ));
    runtime.execute_submission(text, submission).unwrap();
    runtime.shutdown().unwrap();
}
