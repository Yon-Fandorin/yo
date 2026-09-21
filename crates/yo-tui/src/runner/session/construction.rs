use super::{
    super::{
        FrameRateLimit, SkillReferenceConnection, WorkspaceReferenceConnection, state::TuiState,
    },
    TuiSession,
    generation::PublicationRecoveryEvidence,
    metadata::TuiSessionInfo,
};
use crate::{
    AssistantRenderer, DocumentRenderer, LinkResolver, OutputPreferences, PromptTemplates, Theme,
    ThemeOverrides, ToolRenderer,
    appearance::{
        AppearanceCandidate, AppearanceState, ColorCapability, GlyphProfile, MotionPreference,
    },
};

impl TuiSession {
    /// 명시된 Markdown 링크를 호스트가 해석하도록 설정합니다. None이면 웹 전용 링크로 복원합니다.
    /// 콜백을 만들기 전에 파일 소유권을 확인하며, 터미널 재진입 때도 이를 유지합니다.
    #[must_use]
    pub fn with_link_resolver(mut self, resolver: Option<LinkResolver>) -> Self {
        self.appearance
            .select_link_resolver(resolver)
            .expect("startup appearance revision must be available");
        self
    }
    /// 어시스턴트 답변의 Markdown 표시를 설정합니다. None이면 원래 답변 표시로 복원합니다.
    /// 터미널 결과 텍스트와 소스 내보내기는 바뀌지 않으며, 테마 변경 후에도 핸들을 유지합니다.
    #[must_use]
    pub fn with_assistant_renderer(mut self, renderer: Option<AssistantRenderer>) -> Self {
        self.appearance
            .select_assistant_renderer(renderer)
            .expect("startup appearance revision must be available");
        self
    }

    /// 명시적인 문서 본문 Markdown 렌더러를 설정합니다. None이면 원래 본문 표시로 복원합니다.
    /// 테마 변경과 터미널 재진입 후에도 이 세션 전용 콜백을 유지합니다.
    #[must_use]
    pub fn with_document_renderer(mut self, renderer: Option<DocumentRenderer>) -> Self {
        self.appearance
            .select_document_renderer(renderer)
            .expect("startup appearance revision must be available");
        self
    }

    /// 도구 본문 Markdown 렌더러를 설정합니다. `None`이면 도구 출력을 그대로 표시합니다.
    /// 테마 변경과 터미널 재진입 후에도 이 세션 전용 콜백을 유지합니다.
    #[must_use]
    pub fn with_tool_renderer(mut self, renderer: Option<ToolRenderer>) -> Self {
        self.appearance
            .select_tool_renderer(renderer)
            .expect("startup appearance revision must be available");
        self
    }

    /// 선택한 테마 위에 의미 색상을 적용하면서 레이아웃과 터미널 기능 폴백을 유지합니다.
    #[must_use]
    pub fn with_theme_overrides(mut self, overrides: ThemeOverrides) -> Self {
        self.appearance
            .select_theme_overrides(overrides)
            .expect("theme overrides are valid and startup revision must be available");
        self
    }

    /// `/prompt`를 통한 리터럴 삽입에 사용할 검증된 사용자 프롬프트를 설정합니다.
    #[must_use]
    pub fn with_prompt_templates(mut self, templates: PromptTemplates) -> Self {
        self.state.set_prompt_templates(templates);
        self
    }

    /// 오프라인 미리 보기를 포함해 이 세션의 출력 레이아웃 설정을 적용합니다.
    /// 테마 변경과 터미널 재진입 후에도 이 설정을 유지합니다.
    #[must_use]
    pub fn with_output_preferences(mut self, preferences: OutputPreferences) -> Self {
        self.appearance
            .select_output_preferences(preferences)
            .expect("output preferences are valid and startup revision must be available");
        self
    }

    /// 터미널 bell을 사용할지 선택합니다. 기본값은 꺼져 있습니다.
    #[must_use]
    pub fn with_notifications(mut self, enabled: bool) -> Self {
        self.state.set_notifications_enabled(enabled);
        self
    }

    /// 복원된 Session 이력의 마지막 Turn은 실행 중 입력과 교차 도착해도 알리지 않습니다.
    #[must_use]
    pub fn with_notification_history_cutoff(mut self, turn: Option<yo_core::TurnRef>) -> Self {
        self.state.set_notification_history_cutoff(turn);
        self
    }
    /// 명시된 호스트 외관 정보로 비어 있는 Rich 글리프 TUI 세션을 만듭니다.
    #[must_use]
    pub fn new(color_capability: ColorCapability, motion_preference: MotionPreference) -> Self {
        Self::with_glyph_profile(GlyphProfile::Rich, color_capability, motion_preference)
    }

    /// 명시된 프로필과 호스트 외관 정보로 비어 있는 TUI 세션을 만듭니다.
    #[must_use]
    pub fn with_glyph_profile(
        profile: GlyphProfile,
        color_capability: ColorCapability,
        motion_preference: MotionPreference,
    ) -> Self {
        Self::with_session_info(
            profile,
            TuiSessionInfo::default(),
            color_capability,
            motion_preference,
        )
    }

    /// 호스트 레이블, 글리프, 색상, 모션 설정을 명시해 세션을 만듭니다.
    #[must_use]
    pub fn with_session_info(
        profile: GlyphProfile,
        info: TuiSessionInfo,
        color_capability: ColorCapability,
        motion_preference: MotionPreference,
    ) -> Self {
        Self {
            state: TuiState::with_session_info(info),
            appearance: AppearanceState::new(
                AppearanceCandidate::for_profile_with_host_preferences(
                    profile,
                    color_capability,
                    motion_preference,
                ),
            )
            .expect("built-in appearance profiles must always be valid"),
            pending_dispatch: None,
            pending_control: None,
            frame_rate_limit: FrameRateLimit::default(),
            workspace_references: None,
            skill_references: None,
            image_preparation: None,
            publication_recovery_evidence: PublicationRecoveryEvidence::default(),
        }
    }

    /// 글리프, 호스트 기능, 모션 설정을 유지하면서 내장 팔레트를 선택합니다.
    /// 선택 결과는 이 세션에 속하며 터미널 일시 중지와 재개 후에도 유지됩니다.
    #[must_use]
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.appearance
            .select_theme(theme)
            .expect("built-in themes must be valid and startup revision must be available");
        self
    }

    /// 라이브 프레임 요청을 합칠 때 사용할 최대 표시 빈도를 선택합니다.
    #[must_use]
    pub fn with_frame_rate_limit(mut self, limit: FrameRateLimit) -> Self {
        self.frame_rate_limit = limit;
        self
    }

    /// 실행 환경의 비차단 workspace provider를 설정합니다.
    #[must_use]
    pub fn with_workspace_references(
        mut self,
        connection: impl WorkspaceReferenceConnection + 'static,
    ) -> Self {
        self.workspace_references = Some(Box::new(connection));
        self.state.enable_workspace_references();
        self
    }

    /// 실행 환경의 비차단 skill catalog provider를 설정합니다.
    #[must_use]
    pub fn with_skill_references(
        mut self,
        connection: impl SkillReferenceConnection + 'static,
    ) -> Self {
        self.skill_references = Some(Box::new(connection));
        self.state.enable_skill_references();
        self
    }

    /// 검증된 프런트엔드 비종속 모델 선택 컨트롤러를 설정합니다.
    #[must_use]
    pub fn with_model_selection(mut self, controller: yo_core::ModelSelectionController) -> Self {
        self.state.enable_model_selection(controller);
        self
    }
}
