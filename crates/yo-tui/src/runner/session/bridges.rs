use yo_core::{ImagePreparationHost, interview};

use super::{super::PendingDispatch, TuiSession, metadata::TuiStatusLine};
#[cfg(test)]
use crate::appearance::{AppearancePin, GlyphProfile};
use crate::{
    appearance::{AppearanceCandidate, AppearanceCommitError, AppearanceRevision},
    overlay::{AcceptanceReceipt, OverlayInstanceToken, PanelSnapshot, SlotError},
};

impl TuiSession {
    /// 세션 컨트롤러가 쓰기를 소유하며, 호스트는 변경할 수 없는 Journal 근거만 해석합니다.
    #[must_use]
    pub fn with_interview_repository(
        mut self,
        repository: interview::InterviewRepository,
        host: Box<dyn super::super::InterviewHistoryHost>,
    ) -> Self {
        self.state.interview = Some(super::super::interview::InterviewController::new(
            repository, host,
        ));
        self
    }
    /// Supplies complete live destination evidence for opt-in secret recovery.
    #[must_use]
    pub fn with_secret_recovery_destination(
        mut self,
        destination: interview::SecretRecoveryDestination,
    ) -> Self {
        if let Some(controller) = &mut self.state.interview {
            controller.set_recovery_destination(destination);
        }
        self
    }

    /// Replaces the destination evidence after an admitted live model rebind.
    pub fn set_secret_recovery_destination(
        &mut self,
        destination: interview::SecretRecoveryDestination,
    ) {
        if let Some(controller) = &mut self.state.interview {
            controller.set_recovery_destination(destination);
        }
    }
    /// 편집 가능한 사본과 변경할 수 없는 미리 보기를 새로 독립 준비한 Session으로 전달합니다.
    pub fn transfer_interview_to(
        &mut self,
        candidate: &mut Self,
        intent: interview::NewConversation,
        reader: yo_core::TranscriptReader,
    ) {
        if let Some(mut controller) = self.state.interview.take() {
            controller.accepted_pending(intent, reader);
            candidate.state.interview = Some(controller);
        }
    }
    /// 인터뷰 처리 중 보류된 명령을 세션에 보존하여 후속 실행에서 재시도할 수 있게 합니다.
    pub fn retain_interview_backpressure(&mut self, pending: PendingDispatch) {
        self.pending_dispatch = Some(pending);
    }
    /// 인터뷰 처리 실패 내용을 대화 알림으로 남깁니다.
    pub fn report_interview_failure(&mut self, detail: impl Into<String>) {
        let _ = self.state.chat_notice(detail.into());
    }
    pub(in crate::runner) fn take_interview_conversation(
        &mut self,
    ) -> Option<interview::NewConversation> {
        self.state.interview_conversation.take()
    }
    pub(in crate::runner) fn flush_interview(&mut self) {
        if let Some(controller) = &mut self.state.interview
            && let Some(notice) = controller.flush()
        {
            self.report_interview_failure(notice);
        }
    }
    /// 실행 호스트의 단일 비동기 이미지 준비 통로를 연결합니다.
    pub fn with_image_preparation(mut self, host: Box<dyn ImagePreparationHost>) -> Self {
        self.image_preparation = Some(host);
        self.state.enable_image_preparation();
        self
    }

    /// 대화나 provider 상태를 바꾸지 않고 임시 호스트 상태를 교체합니다.
    /// 표시 값이 바뀌었는지 반환합니다. 라이브 호스트는 AgentPoll::StatusLine을 사용합니다.
    pub fn set_status_line(&mut self, status: TuiStatusLine) -> bool {
        self.state.set_status_line(status)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "prompt providers consume this reserved session seam"
        )
    )]
    pub(crate) fn open_prompt_overlay(
        &mut self,
        snapshot: PanelSnapshot,
    ) -> Result<OverlayInstanceToken, SlotError> {
        self.state.open_overlay(snapshot)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "prompt providers consume this reserved session seam"
        )
    )]
    pub(crate) fn refresh_prompt_overlay(
        &mut self,
        token: OverlayInstanceToken,
        snapshot: PanelSnapshot,
    ) -> Result<(), SlotError> {
        self.state.refresh_overlay(token, snapshot)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "prompt providers consume this reserved session seam"
        )
    )]
    pub(crate) fn close_prompt_overlay(
        &mut self,
        token: OverlayInstanceToken,
    ) -> Result<(), SlotError> {
        self.state.close_overlay(token)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "prompt providers consume this reserved session seam"
        )
    )]
    pub(crate) fn take_prompt_overlay_acceptance(&mut self) -> Option<AcceptanceReceipt> {
        self.state.take_overlay_acceptance()
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the first Slice reserves a crate-private runtime replacement seam"
        )
    )]
    pub(crate) fn commit_appearance(
        &mut self,
        candidate: AppearanceCandidate,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        self.appearance.commit(candidate)
    }

    #[cfg(test)]
    pub(crate) fn select_glyph_profile(
        &mut self,
        profile: GlyphProfile,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        self.commit_appearance(AppearanceCandidate::for_profile(profile))
    }

    #[cfg(test)]
    pub(in crate::runner) fn appearance_pin(&self) -> AppearancePin {
        self.appearance.pin()
    }
}
