use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::{session, submission, turn};
use crate::{
    AgentCommand, AgentRuntime, BackendCapabilities, BackendScriptStep, ImageInputCapability,
    InputAdmissionHost, InputImage, InputImageSnapshot, ResolvedSkill, RuntimeError,
    ScriptedBackend, SubmissionRejection, SubmissionRejectionKind, UserInput,
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
