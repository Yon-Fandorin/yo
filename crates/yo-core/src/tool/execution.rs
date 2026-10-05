use std::{any::Any, time::Duration};

use sha2::{Digest, Sha256};

use super::{
    errors::ToolExecutionError,
    registry::ValidatedToolCall,
    schema::{ToolApprovalRequirement, ToolId},
};
use crate::TurnRef;

#[derive(Clone, Debug)]
pub struct ToolExecutionRequest {
    pub turn: TurnRef,
    pub call: ValidatedToolCall,
    pub maximum_output_bytes: usize,
    /// Separate, optional text retention budget; never increases model replay output.
    pub maximum_retained_output_bytes: Option<usize>,
    pub absolute_execution_timeout: Option<Duration>,
}

/// 실행 호스트가 고정한 단일 호출 계획. 복제 없이 승인에서 실행으로 소유권을 넘긴다.
pub struct ToolExecutionPlan {
    identity: [u8; 32],
    approval_scope: String,
    payload: Box<dyn Any + Send>,
}

impl ToolExecutionPlan {
    /// 전체 실행 계획의 digest와 사용자에게 공개 가능한 범위를 함께 고정한다.
    pub fn new(
        identity: [u8; 32],
        approval_scope: impl Into<String>,
        payload: impl Any + Send,
    ) -> Self {
        Self {
            identity,
            approval_scope: approval_scope.into(),
            payload: Box::new(payload),
        }
    }

    /// 이 호스트가 기존 v1 호출을 준비할 때 쓰는 고정된 호환 계획이다.
    pub fn legacy(request: &ToolExecutionRequest, host: &str) -> Self {
        Self::new(legacy_plan_identity(request, host), "", ())
    }

    pub fn matches_legacy(&self, request: &ToolExecutionRequest, host: &str) -> bool {
        request.call.definition().approval() != ToolApprovalRequirement::Planned
            && self.identity == legacy_plan_identity(request, host)
    }

    pub const fn identity(&self) -> &[u8; 32] {
        &self.identity
    }

    pub fn approval_scope(&self) -> &str {
        &self.approval_scope
    }

    /// 계획을 만든 구체 호스트만 실행 자료를 한 번 회수할 수 있다.
    pub fn into_payload<T: Any + Send>(self) -> Result<T, ToolExecutionError> {
        self.payload
            .downcast::<T>()
            .map(|value| *value)
            .map_err(|_| ToolExecutionError::new("execution plan belongs to another host"))
    }
}

/// 실행 전에 확인한 계획의 승인 판정. 사용할 수 없는 계획은 spawn하지 않는다.
pub enum ToolExecutionPreparation {
    Automatic(ToolExecutionPlan),
    ApprovalRequired(ToolExecutionPlan),
    Unavailable(ToolPlanUnavailable),
}

/// 구체 OS 오류나 비밀 경로를 공개하지 않는 안정된 준비 실패 코드.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolPlanUnavailable {
    UnsupportedProfile,
    UnqualifiedPlatform,
    UnresolvedScope,
    ProtectedScope,
}

impl ToolPlanUnavailable {
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnsupportedProfile => "command_profile_unsupported",
            Self::UnqualifiedPlatform => "command_platform_unqualified",
            Self::UnresolvedScope => "command_scope_unresolved",
            Self::ProtectedScope => "command_scope_protected",
        }
    }
}

fn legacy_plan_identity(request: &ToolExecutionRequest, host: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"yo.tool-execution/legacy-plan-v1\0");
    for bytes in [
        host.as_bytes(),
        request.call.definition().id().as_str().as_bytes(),
        request.call.normalized_arguments(),
    ] {
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    hash.finalize().into()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolExecutionOutcome {
    Completed,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolExecutionResult {
    outcome: ToolExecutionOutcome,
    output: String,
    truncated: bool,
    retained_output: Option<(String, bool)>,
}

impl ToolExecutionResult {
    pub fn new(outcome: ToolExecutionOutcome, output: impl Into<String>, truncated: bool) -> Self {
        Self {
            outcome,
            output: output.into(),
            truncated,
            retained_output: None,
        }
    }

    pub const fn outcome(&self) -> ToolExecutionOutcome {
        self.outcome
    }

    pub fn output(&self) -> &str {
        &self.output
    }

    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    /// Host-produced retained text, still awaiting semantic admission and storage.
    /// `truncated` describes this text independently of the model-facing result.
    pub fn with_retained_output(mut self, output: impl Into<String>, truncated: bool) -> Self {
        self.retained_output = Some((output.into(), truncated));
        self
    }

    /// Optional retained text and its omission observation.
    pub fn retained_output(&self) -> Option<(&str, bool)> {
        self.retained_output
            .as_ref()
            .map(|(text, truncated)| (text.as_str(), *truncated))
    }
}

/// Latest bounded, nonterminal output snapshot. It is never a replay result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolExecutionProgress {
    /// Host-produced snapshot, awaiting semantic admission.
    pub output: String,
    /// Whether the host omitted any output bytes.
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolExecutionPoll {
    Pending,
    Ready,
}

pub trait ToolExecution: Send {
    /// Takes the newest coalesced progress snapshot, if this host supports streaming.
    fn take_progress(&mut self) -> Option<ToolExecutionProgress> {
        None
    }

    fn poll(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError>;
    fn take_result(&mut self) -> Option<ToolExecutionResult>;
    fn cancel(&self);
    fn shutdown(&mut self) -> Result<(), ToolExecutionError>;
}

pub trait ToolExecutionHost: Send {
    fn identity(&self) -> &str;
    fn is_available(&self, tool: &ToolId) -> bool;
    /// v1 호스트의 기존 승인 정책은 보존하되 Planned를 무제한 실행으로 대체하지 않는다.
    fn prepare(&mut self, request: &ToolExecutionRequest) -> ToolExecutionPreparation {
        ToolExecutionPreparation::legacy(request, self.identity())
    }

    /// 승인된 계획을 소비한다. 새 프로파일은 이 경계에서 고정한 실행 자료를 사용한다.
    fn start_prepared(
        &mut self,
        request: ToolExecutionRequest,
        plan: ToolExecutionPlan,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        if !plan.matches_legacy(&request, self.identity()) {
            return Err(ToolExecutionError::new(
                "execution plan does not match the request",
            ));
        }
        plan.into_payload::<()>()?;
        self.start(request)
    }
    fn start(
        &mut self,
        request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError>;
    fn shutdown(&mut self) -> Result<(), ToolExecutionError>;
}

impl ToolExecutionPreparation {
    /// 기존 호스트 경로에만 적용되는 v1 승인 의미를 보존한다.
    pub fn legacy(request: &ToolExecutionRequest, host: &str) -> Self {
        let plan = ToolExecutionPlan::legacy(request, host);
        match request.call.definition().approval() {
            ToolApprovalRequirement::Automatic => ToolExecutionPreparation::Automatic(plan),
            ToolApprovalRequirement::Required => ToolExecutionPreparation::ApprovalRequired(plan),
            ToolApprovalRequirement::Planned => {
                ToolExecutionPreparation::Unavailable(ToolPlanUnavailable::UnsupportedProfile)
            },
        }
    }
}
