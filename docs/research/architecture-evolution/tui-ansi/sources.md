# ANSI 화면의 코드 출처와 적용 판단

> Research only. source/snapshot inspection이며 실제 upstream UI 실행이나 최신 릴리즈 확인이 아니다.

Pi v1.0.2 `cd32f7725fdbddbaecdff5b1e68491563394e0ca`, Codex rust-v0.160.0
`a956835d020762cb2b570053af06f643a11c0ecc`, Yo `880467b3186ac7ace0111acd37cb1aa334c9dc4c`를 기준으로 한다.
비교한 Codex pin과 Yo delegated adapter의 지원 버전을 혼동하지 않는다.

각 장면의 P/C 참조는 **화면 구성에 참고한 패턴**이다. 해당 참조가 장면의 모든 기능을
제공한다는 뜻은 아니다. native Plan·Goal·memory·MCP·storage/rescue·권한 재사용의 실제
의미는 [Yo 설계와 계약 대조](../critical-review.md)를 따른다.

Yo 현재 코드와 제안의 차이는 [64개 요소 검수](../tui-review.md),
[입력](../tui-review-input.md), [콘텐츠](../tui-review-content.md),
[화면 체계](../tui-review-shell.md), [추가 콘텐츠](../tui-review-gap.md)에서 추적한다.

<a id="p01"></a>

## P01 · Pi

[packages/coding-agent/src/modes/interactive/components/custom-editor.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/components/custom-editor.ts) — `CustomEditor.renderTopBorder / handleInput`.

Pi는 작업 상태를 입력창 가까이 배치한다. 좁은 폭에서 spinner로 축약될 수 있다. Yo 제안에서는 취소 의미를 보존한다.

<a id="p02"></a>

## P02 · Pi

[packages/coding-agent/src/modes/interactive/components/footer.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/components/footer.ts) — `FooterComponent.render`.

작업 폴더·모델·사용량의 위계. Yo에서는 상태를 장식보다 우선.

<a id="p03"></a>

## P03 · Pi

[packages/coding-agent/src/modes/interactive/components/status-indicator.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/components/status-indicator.ts) — `RetryStatusIndicator / CompactionStatusIndicator`.

실행·재시도·정리 상태와 실제 시간/횟수 구분.

<a id="p04"></a>

## P04 · Pi

[packages/coding-agent/src/modes/interactive/interactive-mode.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/interactive-mode.ts) — `updatePendingMessagesDisplay / restoreQueuedMessagesToEditor / handleExportCommand`.

steer/follow-up 분리. Pi의 큐 텍스트 합치기는 Yo에 도입하지 않음. export는 Yo 신규 제안.

<a id="p05"></a>

## P05 · Pi

[packages/coding-agent/src/modes/interactive/components/model-selector.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/components/model-selector.ts) — `filterModels / updateList`.

검색·현재·기본·갱신을 구별. Yo 예약 상태와 disabled 이유 추가.

<a id="p06"></a>

## P06 · Pi

[packages/coding-agent/src/modes/interactive/components/settings-selector.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/components/settings-selector.ts) — `SettingsSelectorComponent`.

설명과 현재 값이 있는 설정 목록. Yo 통합 설정 화면은 제안.

<a id="p07"></a>

## P07 · Pi

[packages/coding-agent/src/modes/interactive/components/session-selector.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/components/session-selector.ts) — `SessionSelectorHeader.render / SessionList.render`.

사람이 읽는 이름·내용·scope·검색·실패 표시.

<a id="p08"></a>

## P08 · Pi

[packages/coding-agent/examples/extensions/permission-gate.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/examples/extensions/permission-gate.ts) — `tool_call handler`.

예제 extension의 확인 패턴만 참고. native 권한 보장/정책 동등성의 근거가 아님.

<a id="p09"></a>

## P09 · Pi

[packages/coding-agent/src/modes/interactive/components/extension-selector.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/components/extension-selector.ts) — `handleInput / dispose`.

선택지·설명·상황별 취소. Yo 요청 ID와 만료 처리는 별도 유지.

<a id="p10"></a>

## P10 · Pi

[packages/coding-agent/src/modes/interactive/components/tool-execution.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/components/tool-execution.ts) — `createResultFallback / updateDisplay`.

도구 상태·보이는 출력·생략량. 보관되지 않은 출력 복원은 주장하지 않음.

<a id="p11"></a>

## P11 · Pi

[packages/coding-agent/src/modes/interactive/components/assistant-message.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/components/assistant-message.ts) — `AssistantMessageComponent.updateContent`.

본문 위계·공개 reasoning 구별·중단 후 남은 내용.

<a id="p12"></a>

## P12 · Pi

[packages/coding-agent/src/modes/interactive/components/compaction-summary-message.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/components/compaction-summary-message.ts) — `updateDisplay / setExpanded`.

정리 진행과 완료 summary 기록을 구별.

<a id="c01"></a>

## C01 · Codex

[codex-rs/tui/src/bottom_pane/chat_composer.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs) — `handle_key_event_without_popup / handle_submission_with_time`.

입력·첨부·수락 상태. Yo의 입력 수락 경계는 유지.

<a id="c02"></a>

## C02 · Codex

[codex-rs/tui/src/bottom_pane/footer.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/footer.rs) — `FooterProps / single_line_footer_layout`.

action 상태에서 도출하는 footer, 측정과 표시의 일치.

<a id="c03"></a>

## C03 · Codex

[codex-rs/tui/src/status_indicator_widget.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/status_indicator_widget.rs) — `StatusIndicatorRender::lines`.

대기열이 있어도 실행 중 중단 안내 유지.

<a id="c04"></a>

## C04 · Codex

[codex-rs/tui/src/bottom_pane/pending_input_preview.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/pending_input_preview.rs) — `as_renderable / PREVIEW_LINE_LIMIT`.

steer/실패/다음 턴을 분리. 키는 caller override 가능; snapshot 키를 보편값으로 복사하지 않음.

<a id="c05"></a>

## C05 · Codex

[codex-rs/tui/src/bottom_pane/approval_overlay.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/approval_overlay.rs) — `build_options / cancel_current_request / approval_footer_hint`.

범위·거절·전체 내용 공개. 작은 화면에서는 숨은 실행을 수락시키지 않음.

<a id="c06"></a>

## C06 · Codex

[codex-rs/tui/src/bottom_pane/request_user_input/mod.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/request_user_input/mod.rs) — `new_with_keymap / advance_queue_or_complete_at`.

질문 전용 composer와 질문별 답변 상태. Yo의 일반 초안 분리 제안 근거.

<a id="c07"></a>

## C07 · Codex

[codex-rs/tui/src/resume_picker.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/resume_picker.rs) — `render_dense_session_lines / render_footer_lines / render_empty_state_line`.

이름·preview·검색·취소와 실패 복귀.

<a id="c08"></a>

## C08 · Codex

[codex-rs/tui/src/chatwidget/settings_popups.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/chatwidget/settings_popups.rs) — `open_theme_picker / open_experimental_popup`.

설정 선택과 시작 전/로딩/실패 상태 구분.

<a id="c09"></a>

## C09 · Codex

[codex-rs/tui/src/diff_render.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/diff_render.rs) — `render_changes_block / render_wrapped_diff_line`.

부호·파일 summary·좁은 폭. Proposed/Recorded/Current 의미는 Yo 제안이며 Codex 동등 의미 주장이 아님.

<a id="c10"></a>

## C10 · Codex

[codex-rs/tui/src/exec_cell/render.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/exec_cell/render.rs) — `output_lines / truncate_lines_middle`.

실행/실패·생략 바이트·보관된 출력 범위.

<a id="c11"></a>

## C11 · Codex

[codex-rs/tui/src/app/transcript_export.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/app/transcript_export.rs) — `load_export_transcript / write_transcript`.

내보내기 범위·fallback·쓰기 실패. Yo의 미저장 suffix 구조는 별도 설계 필요.

<a id="c12"></a>

## C12 · Codex

[codex-rs/tui/src/app/reconnect.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/app/reconnect.rs) — `begin_reconnect / finish_reconnect`.

stale 표시·제출 제한·초안 보관·오래된 요청 응답 폐기.

## 실제 읽은 snapshot과 제한

- Codex `status_indicator_widget`의 `renders_with_queued_messages`는 Working/interrupt와 큐를 함께 표시한다.
- Codex `approval_overlay/clipping_tests.rs`의 complete-command snapshot은 작은 화면에서 상세 경로를 드러낸다.
- Codex `request_user_input`의 multi-question/tight-height/unanswered snapshot은 질문별 입력과 전체 제출을 구분한다.
- Codex resume picker의 narrow/footer/expanded snapshot과 narrow-primary-action 테스트를 읽었다.
- Codex diff gallery, truncated-live-output, export completion, reconnect failure snapshot을 참고했다.
- Pi footer-width의 CJK, model-selector filter-selection, interactive status, session selector 검색/삭제 테스트를 읽었다.

이 upstream 테스트는 이번 작업에서 실행하지 않았다. snapshot 키는 설정/호출자에 따라 다를 수 있다.
Yo의 Ctrl+G는 `crates/yo-tui/src/runner/state/external_editor.rs::handle_external_editor_key`
에서 실제 binding을 확인했다. 질문/승인/다른 overlay 활성 상태에서는 일반 외부 편집기로 동작하지 않는다.
Pi의 permission-gate는 예제 extension이다. Codex의 export도 ephemeral/legacy에서는 보이는 기록의
fallback이 있으므로 완전 복구 보장을 그대로 가져올 수 없다.
