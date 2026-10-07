# T41–T64 독립 read-only 감사

> Status: non-authoritative research audit

2026-10-05. 제품 코드 변경·빌드·테스트 실행·네트워크 호출 없이 현재 Yo 코드와 대표 테스트의 assertions를 정적으로 검토했다. Pi는 `/home/yon/projects/pi`의 지정 pin v1.0.2 / `cd32f7725fdbddbaecdff5b1e68491563394e0ca`, Codex는 `/tmp/yo-codex-rust-v0.160.0-source/openai-codex-79b1b66`의 지정 pin `a956835d020762cb2b570053af06f643a11c0ecc`를 비교 경로로 사용했다. pin 검증과 실행 검증은 통합 담당 범위다. 아래 `Y/`는 `crates/yo-tui/src/`, `P/`는 Pi 루트, `C/`는 Codex `codex-rs/tui/src/`를 뜻한다. 실제 SSH/tmux·screen reader·터미널 폰트·clipboard 도착·시각 대비·FPS/latency는 검증하지 않았다. 테스트 이름은 기존 검증의 범위를 설명하며 이번 실행 통과 주장이 아니다.

공통 authority: `tui.chrome.input-stack`, `tui.terminal.inline-viewport`, `tui.runtime.frame-scheduling`, `tui.appearance.session-publication`, `tui.terminal.job-control-suspend-resume`와 active checkpoint. 특히 whole-segment 상태 제거, immutable native scrollback, prepare/present/ack, session 단일 appearance, restore-before-stop을 보존해야 한다. 타 제품의 더 많은 위젯이 이 계약을 약화시킬 근거는 아니다.

## T41 · header와 실행 identity — P2

**Yo 현재.** Chat에는 항상 보이는 별도 top header 대신 prompt 아래 metrics가 있다. [Y/shell/chrome.rs::paint_metrics](../../../crates/yo-tui/src/shell/chrome.rs)는 backend 100, usage 50, workspace 30 우선순위로 좌/우 배치한다. 한 segment 자체가 폭보다 크면 먼저 통째로 제거한다. `crates/yo-cli/src/application/runtime/frontend.rs::build_live_session`은 실제 selection label과 compact workspace를 `TuiSessionInfo`로 준다. 없으면 표시를 생략하며 model/host를 추론하지 않는다. 비 Chat은 [Y/runner/view.rs::paint_header](../../../crates/yo-tui/src/runner/view.rs)가 view명·context·F1/F2/F3를 폭에 맞춘 여러 완전한 형식으로 전환한다. 대표 검증은 [Y/shell/chrome/tests.rs::metrics_drop_workspace_as_one_segment_when_width_is_insufficient](../../../crates/yo-tui/src/shell/chrome/tests.rs), [Y/runner/tests/views/frame.rs::header_forms_switch_at_measured_cell_width_boundaries](../../../crates/yo-tui/src/runner/tests/views/frame.rs)다.

**Pi/Codex.** [P/packages/coding-agent/src/modes/interactive/components/footer.ts::render](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/components/footer.ts)는 cwd/branch/session명 행과 usage/model/thinking 행, extension status 행을 둔다. 긴 identity를 줄여 보여 주지만 model이 통째로 사라질 수도 있다. `footer-width.test.ts`는 wide model/provider/name 폭을 검증한다. [C/bottom_pane/footer.rs::passive_footer_status_line](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/footer.rs)과 [C/status/card.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/status/card.rs)는 상시 요약과 상세 status를 구분한다. Codex footer snapshot들은 status와 context 동시 배치를 다룬다.

**문제·최소 개선.** 긴 backend label이 단독으로 안 들어가면 높은 priority도 소용없고 identity가 완전히 사라진다. authority의 whole-segment 제거를 유지하되 host가 제공하는 짧은 정확 label을 별도 후보로 정의하고 `/status`로 전체 식별자를 안내한다. 없는 Git/permission/비용을 장식용으로 만들지 않는다. **수락:** 같은 긴 model/한글 workspace에서 80→24→80 columns, 정확한 짧은 identity 또는 상세 진입 힌트를 보여 주고 원문 identity는 status에서 보존한다. 실제 host별 label 길이 분포와 대비는 미측정.

## T42 · 상태줄·진행·대기 — P2

**Yo 현재.** [Y/shell/chrome.rs::paint_transient](../../../crates/yo-tui/src/shell/chrome.rs)는 request를 Working보다 우선해 `Waiting for approval/your answer/question`을 표시한다. active→idle는 reserved work geometry를 유지한다. [Y/runner/state/presentation.rs::chrome_snapshot](../../../crates/yo-tui/src/runner/state/presentation.rs)은 active turn, 실제 queue, host status, usage와 durability를 읽는다. optional status는 `History not saved`가 host status보다 우선한다. live elapsed timer와 dispatch/acceptance/saving 단계별 진행률은 이 chrome에 없다. [Y/shell/chrome/tests.rs::idle_and_active_layouts_keep_the_same_prompt_origin](../../../crates/yo-tui/src/shell/chrome/tests.rs), [Y/runner/tests/views/navigation.rs::storage_failure_status_survives_new_chat_and_narrow_resize](../../../crates/yo-tui/src/runner/tests/views/navigation.rs), [Y/runner/tests/appearance/activity.rs::host_status_line_updates_without_changing_conversation_source](../../../crates/yo-tui/src/runner/tests/appearance/activity.rs)가 대표 근거다.

**Pi/Codex.** Pi interactive `setWorkingMessage/setWorkingVisible`, `components/status-indicator.ts`와 `test/status-indicator.test.ts`는 working/compaction/retry를 같은 높이로 투영하고 retry countdown을 dispose한다. [C/status_indicator_widget.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/status_indicator_widget.rs)는 header/detail와 elapsed/interrupt 우선순위, `timer.rs`는 pause/resume 시간을 가진다. `renders_remapped_interrupt_hint`, `renders_without_spinner_when_animations_disabled` 등이 있다.

**개선.** 우선 `/status`에 host-known pending/admission/wait 원인을 표현하고 Working과 request 전이를 더 명확히 한다. timer는 유용하지만 기존 chrome authority가 별도 후속 계약 대상으로 명시하므로 단순 장식 추가로 처리하지 않는다. **수락:** 전송 보류→admitted→approval 대기→완료에서 동일 prompt 위치와 사실에 맞는 status, 저장 gap 발생 시 host 메시지보다 warning 유지. latency·진행률은 backend 근거 없으면 표시하지 않는다.

## T43 · footer·키 힌트 — P1, 구체 계약 위반

**Yo 현재와 문제.** [Y/shell/chrome/help.rs::paint](../../../crates/yo-tui/src/shell/chrome/help.rs)는 active 기본 후보에 `Esc/^C interrupt`를 유지하지만 queue가 하나라도 있으면 `Queued n … M-r edit/pause M-q queue/resume`, `Queued n`, `Q:n` 후보를 맨 앞에 삽입한다. 모두 interrupt를 포함하지 않는다. [Y/shell.rs::render_with_measure_hook](../../../crates/yo-tui/src/shell.rs)는 footer의 실제 내용이 아니라 `layout.mode.size.height == 0`일 때만 work row에 interrupt 힌트를 넣는다. 따라서 active+queued+footer-visible 상태는 넓은 폭에서도 두 힌트가 모두 사라질 수 있다. input 동작이 없어지는 문제는 아니다. `tui.chrome.input-stack`은 active footer에서 interruption이 mode/newline/exit/장식보다 우선하고 completed input stack에 두 interrupt affordance가 있어야 한다. 기존 [Y/shell/chrome/tests.rs::queued_footer_preserves_count_pause_and_newline_binding](../../../crates/yo-tui/src/shell/chrome/tests.rs)는 idle snapshot을 사용하고 queue 문자열만 검사해 이 결합을 놓친다. request footer는 confirm/decline, answer/notes/secret별로 별도 의미를 가진다는 점은 보존한다.

**Pi/Codex.** Pi `components/keybinding-hints.ts`는 실제 keybinding에서 label을 생성한다. Codex `bottom_pane/footer.rs`도 queue 힌트를 우선하고 관련 snapshot이 많다. 그 우선순위를 Yo에 복사하면 위 Yo 계약과 충돌한다. Codex는 별도 status indicator의 interrupt 행도 검토해야 전체 affordance를 판정할 수 있다.

**최소 개선·수락.** queue 후보에도 interrupt pair를 먼저 남기거나, footer가 실제로 pair를 표시했는지를 work-row fallback에 전달한다. active queued/paused 각각 12/24/40/88 columns와 footer 없는 높이에서 전체 프레임의 Esc·^C를 검사; 둘이 물리적으로 안 들어가는 폭만 생략 허용. queue count와 pause 의미도 가능한 다음 우선순위로 보존한다.

## T44 · 레이아웃·공간 배분 — P2

**Yo 현재.** 실제 owner는 작은 `layout.rs` façade보다 [Y/shell/chrome.rs::layout](../../../crates/yo-tui/src/shell/chrome.rs), [Y/shell.rs::render_with_measure_hook](../../../crates/yo-tui/src/shell.rs)다. prompt 한 줄→transcript 최대 두 줄 floor→work→metrics→footer→추가 prompt→separator→optional host status 순으로 예산을 배분한다. prompt cap은 `min(desired, max(height/3,5))`; shell 9행부터 prompt frame을 쓴다. overlay는 prompt 위에 bottom-anchor하고 fitting 가능한 경우에만 ordinary work row를 가린다. request overlay는 본문 예산을 더 줄일 수 있다. [Y/shell/tests/resize.rs::long_draft_preserves_conversation_space_and_cursor](../../../crates/yo-tui/src/shell/tests/resize.rs)는 24행에 30행 draft를 넣어 prompt 8행/본문 12행 이상/실제 caret과 source 보존을 확인한다. 2행/8→9행 경계 테스트도 있다.

**Pi/Codex.** Pi `tui.ts::OverlayOptions`는 anchor/percent/minWidth/maxHeight/visibility와 normal/fullscreen을 지원하며 `overlay-options.test.ts`가 경계와 wide grapheme을 다룬다. Codex bottom pane과 theme picker는 남은 영역에 따라 side-by-side→stacked preview를 고른다. 어느 것도 작은 높이에 중요한 approve 본문이 읽힌다는 증거 자체는 아니다.

**개선·수락.** 기존 floor를 유지하며 24×6, 24×9에서 prompt+approval+storage warning+queue가 경쟁하는 completed-frame matrix를 추가한다. 잘린 detail에는 명시적인 detail 경로/계속 읽기 힌트를 둔다. control 자체를 읽지 못하는 panel은 acceptance를 막는 기존 visibility gate와 연결한다. 임의 최소 terminal 크기로 기능을 닫는 재설계보다 이 결합 검증이 우선. 모든 overlay별 최소 크기는 이번 범위에서 실측하지 않았다.

## T45 · focus·cursor·공개된 frame — 유지/P2 검증

**Yo 현재.** [Y/runner/state/presentation.rs::PreparedFrame](../../../crates/yo-tui/src/runner/state/presentation.rs)는 overlay token/presentation, 실제 painted 여부, secret request correlation, cursor, view state, appearance revision을 함께 보관한다. [Y/runner/state.rs::commit_frame](../../../crates/yo-tui/src/runner/state.rs)가 성공 프레임에서 editor layout width와 view state, presented secret/overlay를 commit한다. [Y/runner/tests/overlay.rs](../../../crates/yo-tui/src/runner/tests/overlay.rs)의 `visible_overlay_dismissal_precedes_active_turn_escape_interrupt`, `hidden_overlay_does_not_steal_active_turn_escape`, `switching_away_from_chat_closes_overlay_without_resurrection`은 보이는 overlay가 Esc를 소유하고 숨긴 것은 소유하지 않으며 Ctrl+C는 interrupt로 남음을 확인한다. resize와 suspend는 overlay보다 먼저 처리한다. 이 구조는 보이지 않은 선택을 Enter로 수락하지 않게 하는 핵심이다.

**Pi/Codex.** Pi `tui.ts`는 explicit focus target, capturing/non-capturing overlay, hidden overlay 복원, `CURSOR_MARKER`를 갖는다. `mouse-components.test.ts`는 submenu가 닫혀도 detached child 대신 delegating parent focus를 유지한다. Codex `transcript_view/input_tests.rs`는 link activation과 selection의 gesture 소유권, `bookmark_tests.rs`는 복귀 위치를 검증한다. Yo의 publication receipt와 동일한 보장은 이 비교만으로 확인하지 않았다.

**개선·수락.** 구조 교체보다 80→24→80 사이 hidden panel의 늦은 결과·Enter·Esc를 결합 검증한다. selection/caret는 마지막 committed frame과 일치하고 실패한 준비가 focus를 바꾸면 안 된다. 논리 focus와 OS terminal focus(T52)는 다른 개념이다; 이 항목의 강점을 focus-aware notification 지원으로 확대하지 않는다.

## T46 · resize·reflow·읽던 위치 — P1/P2

**Yo 현재.** follow-tail reflow와 draft/caret는 [Y/shell/tests/resize.rs::resize_reallocates_tracks_and_reflows_follow_tail](../../../crates/yo-tui/src/shell/tests/resize.rs) 등으로 검증된다. geometry epoch는 동일 크기로 돌아오는 resize도 old frame을 무효화하며 [Y/runner/tests/reentry/publication.rs::same_size_post_flush_resize_still_rejects_the_prepared_live_frame](../../../crates/yo-tui/src/runner/tests/reentry/publication.rs)가 이를 검증한다. 그러나 공통 Chat/Transcript viewport인 [Y/transcript/viewport.rs::TranscriptViewState](../../../crates/yo-tui/src/transcript/viewport.rs)는 `mode`와 `first_visible_row`만 저장한다. `VisibleRows::resolve`의 Detached는 새 content height에 같은 행 번호를 clamp한다. 폭 변경으로 앞부분 wrap 수가 바뀌면 같은 문장을 보존하는 의미 anchor가 아니다. Output/Changes는 별도 position 로직/테스트가 있으므로 이 결론을 그 view까지 확대하지 않는다.

**Pi/Codex.** Pi `tui-render.test.ts`의 height/width full-render 및 shrink 테스트는 화면 정합성 근거지만 semantic 읽기 위치 보장 자체는 아니다. Codex `transcript_view.rs::Anchor/Position::Reading/start/resolve`는 entry key+source offset+row bias로 복원한다. `transcript_view/bookmark_tests.rs::cancel_restores_both_positions_after_resize_and_pagination`가 비교상 더 직접적이다.

**최소 개선·수락.** detached viewport에 stable item id+source offset/row bias를 두고 reflow 후 재해석; 동일한 공통 owner를 사용한다. Chat 과거의 한글 긴 문장 중간에 멈추고 80→24→80 및 앞 item 접기/펴기 후 같은 문장 근처 유지, 새 output이 latest로 강제 복귀시키지 않음을 검사한다. 현재 source로는 drift가 가능한 구조라는 판단이며 terminal에서 재현 실행한 결함 보고는 아니다.

## T47 · inline/fullscreen·publication — 강점 유지/P2

**Yo 현재.** Inline은 main screen/native scrollback, Fullscreen은 alternate screen이다([Y/terminal/mode/screen.rs::enter_screen](../../../crates/yo-tui/src/terminal/mode/screen.rs)). [Y/runner/state/presentation.rs](../../../crates/yo-tui/src/runner/state/presentation.rs)는 eligible Chat FollowTail에서 final prefix와 unpublished live suffix를 분리해 compact height를 측정한다. detached 및 diagnostic view에서는 publication을 얼리고 full retained history를 탐색한다. prepare는 cursor를 전진시키지 않고 presented receipt에서 acknowledge한다. post-flush resize는 persistent-prefix 성공과 live ownership을 따로 판정한다. [Y/runner/tests/reentry/publication.rs::post_flush_resize_acknowledges_publication_but_rejects_stale_live_geometry](../../../crates/yo-tui/src/runner/tests/reentry/publication.rs)가 대표 검증이다. published native rows를 resize 때 재flow/재전송하지 않는 것은 결함이 아니라 계약이다.

**Pi/Codex.** Pi `tui.ts::TuiMode`는 regular/fullscreen이고 render baseline/reset 및 terminal history를 별도로 다룬다. Codex `transcript_mode.rs::resolve`는 Owned/Terminal을 launch policy로 확정하고 `tui.rs::enter_alt_screen/leave_alt_screen`, `tui/scrollback.rs`가 일시 overlay와 session-owned screen을 구분한다. 이 비교가 Yo의 exact partial-write reconciliation을 대체하지 않는다.

**개선·수락.** user help에서 native history와 application history의 차이, startup inline/fullscreen 선택과 view 전환의 차이를 명시한다. 런타임 mode-toggle은 현재 registry에 없으므로 있는 듯 안내하지 않는다. final A/streaming B 상태에서 resize·review·tail 복귀·suspend 후 A native publication 중복 없음, B 최신 live 유지, 정상 종료 suffix만 출력됨을 검증한다. 실제 tmux history 회수 및 terminal scrollback retention은 별도 환경 검증 필요.

## T48 · 스크롤·follow latest — P2

**Yo 현재.** [Y/transcript/viewport.rs::VisibleRows](../../../crates/yo-tui/src/transcript/viewport.rs)는 line/page, item start, Home/End와 Detached/FollowTail을 가진다. page는 `height-1`로 한 행 overlap, 실제 위로 이동하면 detach, down이 tail에 닿으면 follow한다. [Y/shell/chrome.rs::paint_history_position](../../../crates/yo-tui/src/shell/chrome.rs)은 `History first-end/total · End latest`→짧은 후보를 표시한다. active work와 history indicator는 transient가 한 행뿐이면 경쟁한다. [Y/runner/tests/views/navigation.rs::wheel_bursts_restore_view_and_preserve_draft](../../../crates/yo-tui/src/runner/tests/views/navigation.rs), [Y/shell/tests/resize.rs::detached_history_has_a_position_and_return_hint](../../../crates/yo-tui/src/shell/tests/resize.rs)가 draft/위치/End 복귀를 확인한다. 새 message 수 badge는 이 상태에 없다.

**Pi/Codex.** Pi `wheel-scroll.test.ts`는 속도/고정 line/terminal acceleration 정책을 분리한다. Codex `transcript_view.rs::scroll/jump_to_latest`와 `follow_control_tests.rs::no_op_scroll_keeps_following_and_real_scroll_pauses`, `control_tracks_pause_activity_hover_resize_and_click`는 pause와 최신 복귀를 명시적 control로 만든다.

**개선·수락.** Detached 상태와 latest 복귀 힌트가 active+작은 높이에서 사라지는 경우를 줄이고, 정말 필요하면 현재 보기 뒤 도착한 final item 수만 계산한 badge를 추가한다. backend progress를 추측하지 않는다. queue나 live token count와 혼동 없는 문구가 필요하다. **수락:** draft를 가진 채 wheel/PageUp으로 detach, streaming append 후 위치 유지, End 한 번에 최신, all views 왕복해 각 위치 복원. semantic resize anchor 문제는 T46에서 함께 해결한다.

## T49 · 항목 접기와 전체 접기 — P2

**Yo 현재.** [Y/runner/view.rs::handle_local](../../../crates/yo-tui/src/runner/view.rs)에서 Chat Ctrl+O는 global expanded_tools를 반전하고 item overrides를 모두 지운다. Alt+O는 committed focused activity만 바꾸며 pending scroll이 있으면 소비만 한다. Alt+Up/Down은 item start로 이동한다. `TuiDocument::with_expanded` 초기값은 사용자 override보다 우선하지 않는다. [Y/runner/tests/activity_projection.rs::long_tool_logs_expand_without_changing_output_or_failure](../../../crates/yo-tui/src/runner/tests/activity_projection.rs), [Y/runner/tests/appearance/document.rs::host_document_initial_expansion_is_optional_and_user_overridable](../../../crates/yo-tui/src/runner/tests/appearance/document.rs)가 source/outcome과 초기 펼침의 소유권을 검증한다. `/help`는 global reset의 의미를 명시하지만 기본 footer에는 이 기능이 드러나지 않는다.

**Pi/Codex.** Pi interactive mode의 `setToolsExpanded/toggleToolOutputExpansion` 및 `ExpandableComponent`는 header/help와 tool들의 expansion을 연동한다. Codex transcript view는 detailed presentation과 activity disclosure를 별도로 갖고 `transcript_view/activity_tests.rs`가 visible row/escape ownership을 검증한다. Yo의 모두 reset 의도를 upstream의 단순 expansion과 같다고 봐선 안 된다.

**개선·수락.** item 이동 시 focus 대상에 짧은 fold/unfold affordance, global 조작 후 `all expanded/collapsed; item overrides reset` 같은 한 번의 알림을 제공한다. **수락:** A만 펼침→B로 이동→Ctrl+O→A/B 및 host document가 global값으로 통일, source export 불변, 실패 결과와 retained truncation은 접혀도 식별. 좁은 화면에서 focus marker의 실제 인지성은 미검증.

## T50 · theme·semantic colors — P2

**Yo 현재.** [Y/appearance/palette.rs](../../../crates/yo-tui/src/appearance/palette.rs)는 Default/Light/Mono와 semantic ThemeRole, terminal default/RGB/fallback colors를 resolve한다. [Y/appearance.rs::AppearanceState](../../../crates/yo-tui/src/appearance.rs)는 검증된 전체 snapshot을 revision과 함께 publish한다. frame pin으로 measure/paint를 일치시키고 renderer/output preferences를 theme 전환에서도 유지한다. [Y/appearance/tests.rs::built_in_selection_focus_uses_resolved_theme_accent_without_bold](../../../crates/yo-tui/src/appearance/tests.rs)는 unknown color에서 selected와 label 모두 default style일 수 있음을 명시하므로 선택의 비색상 glyph를 함께 유지해야 한다. `theme_round_trip_preserves_host_preferences_and_pinned_frames`, [Y/runner/tests/appearance/preferences.rs::semantic_theme_overrides_reach_frames_and_survive_theme_switches](../../../crates/yo-tui/src/runner/tests/appearance/preferences.rs)가 범위다.

**Pi/Codex.** Pi ThemeSelector는 current preselection과 selection preview/cancel callback, resource themes를 제공한다(`theme-selector.ts`, `test/theme-picker.test.ts`). Codex `/theme`는 `theme_picker.rs`에서 live syntax preview, confirm persist, cancel restore, narrow stacked preview; 전체 UI palette와 syntax theme를 혼동하면 안 된다.

**개선·수락.** Yo 현재 command registry에는 theme picker가 없다. CLI theme 설정만으로 contrast를 고르기 어려우므로 개발용 offline preview에서 Default/Light/Mono 보기와 source-safe cancel/restore를 검증하는 것이 최소 개선이다. 일반 live에 공개할지는 별도 제품 결정이며 현재 `/preview`는 developer-only다(T61). mode/selection/failure는 색 없이도 식별 가능하게 유지한다. **수락:** mono+unknown color에서 selected/current/disabled/error 구별, theme preview 취소 시 원 appearance revision 및 draft/viewport 보존. 색 대비 수치·색각 사용자 테스트는 시행하지 않았다.

## T51 · glyph·Unicode·폭 — 강점 유지/P2

**Yo 현재.** [Y/surface/text/width.rs](../../../crates/yo-tui/src/surface/text/width.rs)는 `yo-unicode-17.0-narrow/v1`, EastAsian Wide/Fullwidth=2, ambiguous=narrow, RGI/emoji/VS16=2, VS15 text width를 고정한다. combining/ignorable는 base 폭에 결합한다. [Y/surface/tests/text.rs::resolves_non_emoji_grapheme_widths](../../../crates/yo-tui/src/surface/tests/text.rs), `resolves_emoji_grapheme_widths`, `standardized_vs15_uses_non_emoji_width`는 한글/e+accent/ZWJ/flag/VS를 직접 검증한다. Rich/Ascii는 chrome glyph profile이며 사용자 한글을 ASCII로 손실 변환하지 않는다. [Y/runner/tests/appearance/frame.rs::rich_and_ascii_profiles_keep_body_columns_stable](../../../crates/yo-tui/src/runner/tests/appearance/frame.rs)가 profile 전환을 확인한다.

**Pi/Codex.** Pi `visible-width.test.ts`, `regression-regional-indicator-width.test.ts`, `regression-overlay-cjk-boundary.test.ts`는 ANSI-aware width와 overlay wide-cell 경계를 다룬다. Codex `transcript_view/diff_source_tests.rs::wrapped_added_code_keeps_indentation_and_graphemes_when_copied_and_resized`는 표시/복사/grapheme 연결을 검증한다. Unicode library의 존재만으로 터미널의 실제 width와 일치한다고 할 수 없다.

**개선·수락.** 주 width engine 교체보다 개발용 `/preview`의 mixed-width matrix와 적용 profile 정보를 제공한다(T61의 공개 범위 제한). **수락:** `A가é👩‍💻🇰🇷♥︎♥️` 입력/selection panel/Markdown/code/footer 80→24→80에서 절반 glyph 잔상·caret 분리·source 변경 없음, ANSI와 HTML projection의 셀 일치. 실제 emulator별 ambiguous/emoji width 차이는 별도 검증 경계로 남긴다.

## T52 · motion·attention·notification — P1 접근성 연결 / P2 capability

**Yo 현재.** reduced motion renderer는 실제로 있다([Y/appearance/activity.rs](../../../crates/yo-tui/src/appearance/activity.rs), [Y/runner/tests/appearance/frame.rs::public_reduced_motion_session_keeps_activity_static](../../../crates/yo-tui/src/runner/tests/appearance/frame.rs)). 그러나 live CLI `crates/yo-cli/src/application/runtime/frontend.rs::build_live_session`은 `MotionPreference::Standard`를 고정한다. config/CLI에서 reduced/animations 전달 경로를 찾지 못했다. API 지원을 사용자가 선택할 수 있는 완성 기능으로 계산하면 안 된다. attention은 [Y/runner/attention.rs](../../../crates/yo-tui/src/runner/attention.rs)와 `state.rs::take_attention_bell`에서 live submission 후 arm, historical silence, actionable committed request, request dedup, queued follow-up 중간 완료 억제를 한다. [Y/runner/tests/attention.rs::secret_question_waits_for_committed_frame](../../../crates/yo-tui/src/runner/tests/attention.rs), `queued_follow_up_suppresses_intermediate_completion_bell`가 근거다. transport는 BEL이며 OS focus를 추적하지 않는다.

**Pi/Codex.** Pi의 working indicator customization/dispose는 source에 있지만 OS desktop notification 동등성은 이 감사에서 확인하지 않았다. Codex `notifications/mod.rs`는 Auto/OSC9/BEL, `tui.rs::should_emit_notification`는 Always/Unfocused, `screen_reader.rs`는 450ms bounded reader detection으로 animation 기본값을 줄이고 명시 설정을 존중한다. 이 역시 reader 완전 지원의 증거는 아니다.

**최소 개선·수락.** 먼저 public CLI config에서 reduced motion을 선택하게 하고 기존 session API에 연결한다. focus-aware notification은 typed focus event와 실패 정책을 설계한 후 별도 추가한다. **수락:** reduced 선택 시 active status는 정적이고 필요한 상태 변화만 redraw, resume에도 유지; 요청당 한 bell, 자동 queue 전환은 중간 bell 없음. BEL이 사용자의 장치에서 들리는지는 보장하지 않는다.

## T53 · mouse·selection·clipboard — P1/P2

**Yo 현재.** [Y/terminal/mode/screen.rs](../../../crates/yo-tui/src/terminal/mode/screen.rs)는 Fullscreen에서만 mouse capture를 켠다; Inline은 native selection/scrollback을 남긴다. [Y/terminal/backend/unix/input.rs::decode_event](../../../crates/yo-tui/src/terminal/backend/unix/input.rs)는 wheel을 ±3 lines로 바꾸고 click/drag/move/horizontal wheel은 `MouseScroll(0)`으로 버린다. `mouse_wheel_decodes_without_turning_pointer_reports_into_keys`가 그 제한을 명시한다. 따라서 Fullscreen에서 mouse를 capture하지만 클릭 caret/list 선택과 앱 내 drag selection은 제공하지 않는다. `/copy`는 latest completed assistant 원문을 OSC52로 보낸다. [Y/terminal/clipboard.rs](../../../crates/yo-tui/src/terminal/clipboard.rs)는 UTF-8 75,000 bytes 경계와 partial-write OSC 종료를 처리하고 `osc52_carries_exact_source_text`, `first_excess_byte_is_rejected_before_output`가 검증한다. 이는 arbitrary visible selection copy가 아니다.

**Pi/Codex.** Pi `mouse-components.test.ts`는 editor click/cursor, list activation, drag-copy, delegated focus를 실제 소스로 다룬다. Codex `transcript_view/selection.rs`는 revision-pinned source selection과 deferred clipboard confirmation/PRIMARY를 갖는다. 둘의 mouse 지원도 개별 surface별 범위이지 모든 pointer gesture 지원은 아니다.

**개선·수락.** 먼저 fullscreen help에 terminal-native selection 우회와 `/copy` 범위를 명시하고 capture-off 선택을 제공한다. 이후 pointer selection을 추가한다면 grapheme/source-offset+committed geometry owner를 재사용한다. **수락:** capture off에서 native selection 사용 가능, on에서 wheel은 draft를 편집하지 않음, `/copy` 성공 문구는 request sent이지 clipboard delivered 확정 아님. 실제 local/SSH/tmux clipboard 수신과 terminal modifier 우회는 미검증.

## T54 · 키 protocol·충돌·대체 경로 — P1

**Yo 현재.** [Y/input/view_binding.rs](../../../crates/yo-tui/src/input/view_binding.rs)는 modifier 없는 F1/F2/F3만 Chat/Transcript/Request로 매핑하고 F4는 없다. registry에 `/chat`·`/transcript`·`/request` 대체 명령을 찾지 못했다. [Y/input/editor/binding.rs::Default](../../../crates/yo-tui/src/input/editor/binding.rs)는 Shift+Enter이며 [Y/input/editor.rs](../../../crates/yo-tui/src/input/editor.rs)는 modifier 없는 Enter를 submit, 정확히 설정된 modifier Enter만 newline로 처리한다. `with_newline_binding`의 사용처는 테스트뿐이다. Unix backend의 mode enum에는 bracketed paste/alternate/cursor/mouse만 있으며 keyboard enhancement enable/probe를 찾지 못했다. enhanced modifier 보존 unit test는 terminal이 해당 bytes를 실제로 보내게 만드는 기능이 아니다. plain Enter로 collapse하는 terminal에서 S-Enter hint는 사용자를 예기치 않은 submit으로 이끌 수 있다. 대표 테스트 [Y/input/editor/tests.rs](../../../crates/yo-tui/src/input/editor/tests.rs)의 configured newline 테스트와 `view_binding.rs::default_bindings_map_plain_function_key_presses_exactly`는 decoded-key 경계만 증명한다.

**Pi/Codex.** Pi `terminal.ts`는 Kitty query→flags 확인→modifyOtherKeys fallback, SSH escape timeout 100ms를 적용하며 `test/terminal.test.ts`가 split reply/normal input replay를 검증한다. Codex `tui/keyboard_modes.rs`와 `tui/tmux.rs`는 enhancement와 tmux extended-keys-format을 고려한다.

**최소 개선·수락.** capability-aware advertised newline과 단순한 대체 gesture(예: 지원 가능한 명시 Ctrl+J; 기존 입력 계약 검토 필요), view slash 경로를 함께 제공한다. 외부 편집기/붙여넣기는 현재 가능하지만 같은 one-keystroke usability라고 계산하지 않는다. **수락:** enhancement 없음/Kitty/SSH/tmux에서 multiline 의도는 submit되지 않고 현재 가능한 hint가 일치; F-key를 OS가 가로채도 명령으로 세 view 진입·Chat 복귀 가능. terminal 실험 전 특정 emulator가 항상 실패한다고 단정하지 않는다.

## T55 · 종료·suspend·복귀 — 강점 유지/P2

**Yo 현재.** [Y/terminal/mode/transaction.rs](../../../crates/yo-tui/src/terminal/mode/transaction.rs)와 `screen.rs`는 mode acquisition/compensation, raw/TTY restoration, primary+cleanup failure를 구분한다. [Y/runner/tests/job_control.rs::ctrl_z_press_requests_terminal_suspension](../../../crates/yo-tui/src/runner/tests/job_control.rs)과 repeat/release/backpressure 테스트는 Ctrl+Z가 exit가 아닌 suspend effect임을 확인한다. [Y/runner/tests/reentry.rs::second_terminal_generation_renders_retained_state_from_a_fresh_frame](../../../crates/yo-tui/src/runner/tests/reentry.rs)는 state를 generation 밖에 보관한 복귀를 확인한다. [Y/terminal/mode/screen/tests.rs::viewport_error_does_not_skip_tty_restoration](../../../crates/yo-tui/src/terminal/mode/screen/tests.rs), `inline_boundary_retains_primary_panic_and_both_cleanup_failures`는 실패에도 복구 시도를 보존한다. 정상 종료의 inline suffix, suspend 무출력은 다른 경로다.

**Pi/Codex.** Pi `interactive-mode.ts::handleCtrlZ`는 keepalive, SIGINT temporary handler, stop→SIGTSTP→SIGCONT start/full-redraw를 구현하고 `interactive-mode-suspend.test.ts`가 실패 handler 정리를 확인한다. Codex `tui/job_control.rs`는 inline/alternate resume placement와 raw restoration을 구분한다. Yo host의 termination lease 우선순위를 upstream signal 처리로 대체하지 않는다.

**개선·수락.** 새로운 lifecycle abstraction보다 user-facing suspend/help 및 반복 recovery 환경 검증을 보완한다. approval/secret/ordinary draft/Detached 각각 Ctrl+Z→shell→resize→fg를 반복해 동일 session/queue/request와 새 full frame, terminal echo/cursor 복구를 검사한다. 자동 tests는 실제 shell job control/PTY/emulator 전체의 대체 증거가 아니다. 필수 subprocess 검사 목록은 root가 정할 부분으로 남긴다.

## T56 · SSH/tmux·image protocol — P2

**Yo 현재.** [Y/terminal/graphics.rs::Graphics::detect](../../../crates/yo-tui/src/terminal/graphics.rs)는 Cells/Kitty/KittyTmux. 기본은 tmux가 아니고 KITTY_WINDOW_ID 또는 ghostty/WezTerm/kitty identity일 때만 Kitty; tmux에서는 자동 Cells, `YO_TUI_IMAGE_PROTOCOL=kitty-tmux`가 명시 경로다. `kitty`를 tmux에서 강제로 지정해도 fallback한다. tmux passthrough는 ESC doubling, cursor coordinates는 wrapper 밖에 남긴다. PNG base64는 4096-byte chunk, owned image id만 삭제하고 frame당 16개 한도다. `tmux_requires_explicit_transport_selection`, `png_transport_is_chunked_and_scoped_to_owned_ids`가 대표 근거다. environment hint이지 end-to-end remote capability probe는 아니다.

**Pi/Codex.** Pi `terminal-image.ts::detectCapabilitiesFromEnvironment`도 tmux 자동 image를 null로 두며 hyperlinks는 tmux feature probe를 사용한다; Kitty/iTerm2와 override를 가진다. Codex `tui/tmux.rs`는 mouse off와 extended keys policy를 읽는다. `pets/image_protocol.rs`에는 Kitty/Sixel 등이 있으나 **pet 전용 renderer**이므로 일반 transcript image 지원의 동등 근거로 확대하지 않는다.

**개선·수락.** `/status` 또는 preview에 현재 graphics=Cells/Kitty/KittyTmux와 선택 이유, remote 불확실성을 표시하고 fallback 원본/설명 경로를 유지한다. iTerm/Sixel 추가보다 unknown transport에서 escape 누출 없는 fallback을 우선. **수락:** tmux auto는 APC 없이 readable cells, explicit KittyTmux만 passthrough, off는 image escape 없음; unsupported/17번째 이미지가 조용히 의미를 잃지 않는지 fallback 경계 확인. 실제 outer terminal의 passthrough 설정과 SSH 환경 전달은 미검증.

## T57 · 비시각 접근·읽기/내보내기 — P1 public 접근 / P2 export

**Yo 현재.** Mono/Ascii/Reduced API와 source-preserving archived projections는 있다. [Y/plain.rs](../../../crates/yo-tui/src/plain.rs)는 목록 렌더러이며 interactive screen-reader mode가 아니다. [Y/runner/archival.rs::project_archived_session_with_options](../../../crates/yo-tui/src/runner/archival.rs)는 Chat/Transcript/Request, Transcript의 None/Preview/Full 및 record limit을 지원하고 backend/terminal ownership 없이 읽는다. [Y/html.rs](../../../crates/yo-tui/src/html.rs) 계열은 completed Surface의 시각 projection이며 자동으로 semantic heading/list accessibility export가 되는 것은 아니다. [Y/runner/tests/appearance/frame.rs::terminal_and_html_project_the_same_completed_appearance_surface](../../../crates/yo-tui/src/runner/tests/appearance/frame.rs), `tests/rendering_parity`가 cell parity를 검증한다. live request focus announcements/읽기 순서/새 content 발표는 이번 surface에서 확인하지 못했다. CLI reduced-motion 연결 결손은 T52와 같다.

**Pi/Codex.** Pi의 HTML export/theme-export는 별도 reading path이나 screen-reader journey를 이번에 입증하지 않았다. Codex `screen_reader.rs`+`screen_reader_tests.rs::persisted_default_loads_and_renders_without_animation`는 reader 탐지와 animation 기본값 감소를 제공한다. reader 탐지만으로 실시간 승인·streaming 낭독 사용성을 보장하지 않는다.

**개선·수락.** 먼저 실제 CLI에서 mono/ascii/reduced 선택 및 비interactive session 읽기 경로를 help에 연결한다. source export와 화면 dump를 명확히 구분한다. **수락:** 색 없이 role/outcome/selection 판별, exported 원문에 장식/scroll wrap 미혼입, 읽기 명령이 새 turn을 생성하지 않음. 실제 NVDA/VoiceOver/Orca로 streaming·request·picker·복귀 검증 전 'accessible' 완결 판정은 보류한다.

## T58 · 화면 응답성·용량 — 유지/P2 계측

**Yo 현재.** [Y/runner/frame.rs::FrameScheduler](../../../crates/yo-tui/src/runner/frame.rs)는 ordinary default 120/optional 60fps, first/resize immediate 예외, 완료 시점 간 cadence, marker deadline coalescing을 갖는다. [Y/runner/unix.rs](../../../crates/yo-tui/src/runner/unix.rs)는 missed resize를 주기적으로 sample하지만 stable geometry면 재paint하지 않는다. [Y/runner/unix/sources.rs::poll_ordinary](../../../crates/yo-tui/src/runner/unix/sources.rs)와 source_schedule은 terminal/agent/workspace/skill 하나씩 공정하게 처리하고 termination을 각 poll 전후 확인한다. [Y/runner/tests/source_scheduling.rs::continuously_ready_sources_are_selected_one_at_a_time_in_cyclic_order](../../../crates/yo-tui/src/runner/tests/source_scheduling.rs), [Y/runner/tests/reentry.rs::idle_geometry_checks_recover_missing_resize_events_without_repainting_stable_size](../../../crates/yo-tui/src/runner/tests/reentry.rs), `zero_width_interval_suppresses_busy_frames_until_one_visible_recovery`가 대표 범위다. bounded scheduling은 긴 synchronous measure/custom callback이 짧다는 증명은 아니다.

**Pi/Codex.** Pi `tui.ts`는 16ms ordinary schedule과 keyboard immediate preemption, `tui-render.test.ts::renders keyboard input without waiting for a throttled frame`을 가진다. Codex `tui/frame_requester.rs`는 request coalescing actor/120fps limiter. 두 설계를 Yo에 복사하기보다 latency 기준을 비교해야 한다.

**개선·수락.** long transcript, huge retained output, heavy renderer에서 input→frame, interrupt dispatch, bytes/frame, idle wake, memory를 계측하는 fixture를 우선한다. 필요 시 width/appearance/item revision keyed layout cache를 현재 owner에 추가한다. **수락:** 지속 ready 모든 source가 전진하고 Esc/resize가 장시간 starving되지 않음, stable idle frame 0, 60/120 정책 및 zero geometry 준수. 이번 감사는 수치 benchmark를 실행하지 않았으므로 '빠르다/느리다' 확정 없음.

## T59 · custom renderer·document·status — 유지/P2

**Yo 현재.** [Y/runner/agent.rs::AgentPoll](../../../crates/yo-tui/src/runner/agent.rs)은 Notice, Document, StatusLine, Links를 별도 typed event로 전달한다. [Y/runner/session/metadata.rs::TuiDocument](../../../crates/yo-tui/src/runner/session/metadata.rs)는 ActivityDocument 검증 후 source snapshot을 보관하고 append의 dedup/retention은 host 책임이다. `TuiStatusLine`은 완전 교체, key 정렬, 최대 16 entries/key 64 bytes/text bounded 및 control escape를 갖고 empty가 clear다. [Y/transcript/layout/activity/document.rs](../../../crates/yo-tui/src/transcript/layout/activity/document.rs)는 invalid custom document projection에 native fallback, [Y/appearance.rs](../../../crates/yo-tui/src/appearance.rs)는 renderer publication을 pinned snapshot에 넣는다. [Y/runner/tests/appearance/document.rs::document_renderer_preserves_ownership_source_and_fallback](../../../crates/yo-tui/src/runner/tests/appearance/document.rs), `host_document_validates_before_publication`, [Y/runner/tests/appearance/activity.rs::host_status_line_updates_without_changing_conversation_source](../../../crates/yo-tui/src/runner/tests/appearance/activity.rs)가 근거다.

**Pi/Codex.** Pi interactive mode는 setStatus, setHeader, setFooter, custom editor, custom overlay, message/tool renderer와 dispose까지 더 넓게 열어 둔다. custom editor 교체 시 action handler 복사 등 lifecycle 부담도 보인다. Codex 내부 HistoryCell/Renderable는 typed component 경계지만 Yo public host extension API의 동등한 일반 plugin surface로 판정하지 않는다.

**개선·수락.** 무제한 custom-widget 삽입보다 현재 host events/renderers에 source-preservation/failure/latency 계약을 문서화한다. host status가 좁은 폭에서 ellipsis되는 경우 전체 detail을 볼 경로가 필요하다. **수락:** malformed document 거절은 기존 frame 보존, status replace/clear는 history/source 무변경, custom renderer가 실패/None/oversize를 낼 때 역할·결과·원문은 읽힘, theme/resize에서도 same revision. callback 무한대기/임의 panic 격리의 보장은 이번 검사로 확인하지 않았다.

## T60 · 화면 전이·역방향 누락 감사 — P1 문서 완성 / P2 UX

**Yo 역대조.** [Y/lib.rs](../../../crates/yo-tui/src/lib.rs) public surface는 input·render뿐 아니라 archived Chat/Transcript/Request/Usage, meter/plain lists, host document/status/link events, preview-capable session과 interview host를 내보낸다. [Y/command/registry.rs::ORDERED_DEFINITIONS](../../../crates/yo-tui/src/command/registry.rs)의 17개는 `/help`, `/model`, `/status`, `/compact`, `/copy`, `/output`, `/preview`, `/attach`, `/exit`, `/find`, `/new`, `/interview`, `/fork`, `/tree`, `/resume`, `/secrets`, `/prompt`. [Y/runner/view.rs::ObservabilityView](../../../crates/yo-tui/src/runner/view.rs)는 Chat, Transcript, Request, Changes, Output의 다섯 view. Usage는 F4 live view가 아니라 독립 command/report 경계다. 일반 view 왕복은 draft와 view-local scroll을 유지하지만 prompt overlay는 다른 view로 나가면 닫힌다; [Y/runner/tests/views/navigation.rs::switching_restores_each_view_local_scroll_state](../../../crates/yo-tui/src/runner/tests/views/navigation.rs), [Y/runner/tests/overlay.rs::switching_away_from_chat_closes_overlay_without_resurrection](../../../crates/yo-tui/src/runner/tests/overlay.rs)이 대표 근거다.

**누락.** 60개 초기 목록은 개발용 `/preview`의 독립 nested session·복귀를 직접 다루지 않는다. 일반 live에서는 unavailable이고 registered command 전체가 public capability는 아니다. [Y/runner/state/preview.rs::open_preview](../../../crates/yo-tui/src/runner/state/preview.rs)는 active/admission/pending request이면 거절, synthetic child를 만들어 실제 parent observations와 분리, `/preview` 또는 `/exit`로 복귀한다. [Y/runner/tests/preview.rs::preview_is_interactive_isolated_and_returns_to_real_session](../../../crates/yo-tui/src/runner/tests/preview.rs), `active_real_turn_cannot_be_hidden_by_preview`가 있다. `/compact` 직접 요청과 실패/cancel은 T24 passive compaction과 다르다. `/interview`의 작업 사본 continue/close/view/discard 흐름은 T18 한 질문과 다르다. startup/resumed/read-only notice 및 History not saved의 지속성도 generic error만으로 묻히면 안 된다.

**Pi/Codex.** Pi `interactive-mode.ts`는 runtime command+extension custom surface까지 있어 registry enumeration만으로 전체 UX를 포착할 수 없다. Codex `slash_command.rs`, `tui.rs`의 owned/overlay presentation, `transcript_mode.rs`도 함께 보아야 한다. source가 공개한 후속 surface를 판별하고 product scope 밖 기능을 무조건 parity 항목으로 만들지 않는다.

**최소 개선·수락.** 추가 감사 ID 제안: T61 offline preview, T62 manual compaction control, T63 interview working-copy lifecycle, T64 startup/recovery/durability notices. 각 action→owner→visible result→cancel/return→draft/scroll retention을 연결한 전이표로 기존 항목과 cross-link한다. **수락:** 17 registry entries/5 view enum/public host surface가 적어도 한 상세 항목에 매핑되고, 도움말의 키가 실제 진입과 복귀를 수행; capability unavailable command는 model input으로 유출되지 않음. 외부 CLI account/connect/session/report UI는 이 내부 TUI 범위 밖임을 명시한다.

## 통합 우선순위와 한계

1. P1: T43 active queue가 interrupt hint를 가리는 정적 확인 결함. 기존 chrome contract 안에서 수정 가능.
2. P1: T54 enhancement 협상/대체 newline 경로 없이 S-Enter를 광고하는 public interaction gap. decoded key tests를 terminal usability 증거로 삼지 말 것.
3. P1 또는 다음 안정화: T46 detached common viewport의 row-only anchor. wrapped content의 읽기 위치 보존을 목표로 별도 기준 수립.
4. P1 접근성 연결: T52 reduced-motion API와 실제 CLI의 hardcoded Standard 사이 간극. T57과 공동 소유.
5. P2: T53 Fullscreen wheel-only capture의 selection tradeoff, T41 exact short identity, T56 capability 설명, T60 보조 surface 누락 보완.

정적 evidence로 특정 terminal의 실제 실패 빈도·repaint performance·screenreader 성공·clipboard 수신을 주장하지 않는다. 모든 개선은 research 제안이며 accepted product contract의 변경을 자동 승인한 것이 아니다. source-preserving rendering, publication receipts, cleanup와 view-local state는 보존할 강점이다.

## T61 · offline preview의 격리·복귀 — P2, 개발 전용 범위

**Yo 현재.** [Y/command/preview.rs](../../../crates/yo-tui/src/command/preview.rs)에는 `/preview`가 등록되지만 [Y/runner/state/commands.rs::command_unavailable_reason](../../../crates/yo-tui/src/runner/state/commands.rs)은 developer_preview_enabled가 없으면 `Offline UI preview is available through the developer chat_preview example.`로 차단한다. 일반 live 사용자 기능으로 집계하면 틀리다. 개발용 세션에서 [Y/runner/state/preview.rs::open_preview](../../../crates/yo-tui/src/runner/state/preview.rs)는 실제 turn/starting submission/pending submission/request가 없을 때 synthetic child state/TestAgent를 만든다. child identity는 `PREVIEW · offline test agent`, child presentation은 Fullscreen, 원 session 출력/publication은 분리한다. `/preview`나 `/exit`로 돌아온다. status/document/approval/image 등 표본을 같은 렌더러로 조작할 수 있다는 강점이다. [Y/runner/tests/preview.rs::ordinary_session_rejects_preview_without_dispatch](../../../crates/yo-tui/src/runner/tests/preview.rs)는 대소문자 직접 입력 모두 draft를 보존하고 model로 안 보냄, `preview_is_interactive_isolated_and_returns_to_real_session`는 synthetic input 미유출·publication 없음·원본 출력 동일·복귀 뒤 real dispatch를 검사한다. `active_real_turn_cannot_be_hidden_by_preview`는 실제 작업 은폐를 막는다.

**Pi/Codex.** Pi의 `tui.ts` custom overlay와 examples, Codex의 `theme_picker.rs` live preview는 구성요소 비교 사례다. 이 감사에서는 Yo처럼 실제 대화와 synthetic multi-scenario child를 통째로 왕복하는 public command의 정확한 counterpart를 확인하지 않았다. theme preview를 전체 offline agent simulator 동등 기능으로 세지 않는다.

**개선·수락.** 개발 문서에 정확한 진입 명령, test-agent label, `/preview`/`/exit` 복귀와 실제 세션 비변경을 명시하고 T01–T60 acceptance matrix의 재현 장치로 활용한다. 일반 live 공개는 별도 결정. **수락:** preview에서 approval decline/queue/long output/resize를 수행하고 나와 real draft·첨부·viewport·history가 유지, 실제 backend dispatch 0; active real turn에서는 진입 거절과 draft 보존. root가 이번에 실행한 unit/parity 범위와 수동 developer preview 여부는 구분해야 한다.

## T62 · `/compact` 직접 요청·보류·완료 — P2

**Yo 현재.** [Y/command/compact.rs](../../../crates/yo-tui/src/command/compact.rs)는 `/compact [guidance]`를 typed `CompactContext` control로 파싱한다. [Y/runner/state/commands.rs::command_unavailable_reason/handle_compact_command](../../../crates/yo-tui/src/runner/state/commands.rs)는 Yo-managed capability와 idle condition(active turn/starting submission/compaction pending)을 검사하고, 거절 시 draft를 복원한다. 수락 후 editor를 비우고 guidance를 optional로 전달한다. [Y/runner/state/input.rs](../../../crates/yo-tui/src/runner/state/input.rs)는 dispatch 대기를 context_compaction_pending으로 추적한다. [Y/runner/state/observation.rs](../../../crates/yo-tui/src/runner/state/observation.rs)의 control outcome은 거절 시 pending을 풀고 `Context compaction was not started`를 표시한다. `/status`는 `Compacting context`를 보고한다. 대표 테스트는 [Y/runner/tests/command_palette.rs::compact_with_guidance_dispatches_one_idle_control_intent](../../../crates/yo-tui/src/runner/tests/command_palette.rs), [Y/runner/tests/status.rs::status_reports_pending_context_compaction](../../../crates/yo-tui/src/runner/tests/status.rs), [Y/runner/tests/appearance/activity.rs::context_compaction_updates_one_styled_notice_without_thinking_or_raw_json](../../../crates/yo-tui/src/runner/tests/appearance/activity.rs)이다. 직접 요청의 waiting/decline과 자동 compaction 활동 표시(T24)는 구분해야 한다.

**Pi/Codex.** Pi `interactive-mode.ts::handleCompactCommand`는 custom instructions를 session.compact에 전달하고 outcome event로 상태를 표시; compaction 중 Esc는 abortCompaction 경로가 있다. Codex `slash_command.rs::Compact`와 `chatwidget/tests/compaction_tests.rs::compaction_status_survives_follow_up_and_preserves_turn_time`는 live status 유지·중복 start timer 불변·후속 submission과 완료를 검사한다. Codex가 compaction 중 follow-up을 허용한다고 Yo의 idle control 정책을 그대로 바꾸면 안 된다.

**개선·수락.** 요청 수락 전/pending/실행 중/완료/실패를 한 문구로 뭉치지 않고, 사용자가 취소할 수 있는 단계만 실제 supported key와 함께 표시한다. idle control이 아직 Turn을 갖지 않을 때의 Esc/Ctrl+C 의미는 별도 확정·검증이 필요하며 이 감사에서는 universal cancel 지원을 입증하지 않았다. **수락:** guidance exact 전달, 이중 Enter가 두 compaction을 만들지 않음, busy/unsupported는 입력 보존·model submit 없음, 실패 뒤 pending이 해제되어 다음 정상 입력 가능, 성공도 provider/model을 몰래 바꾸지 않음. semantic lossy checkpoint 규칙은 `agent.backend.yo-managed-model-loop`가 소유한다.

## T63 · `/interview` 작업 사본의 재개·읽기·폐기 — P2

**Yo 현재.** 이 명령은 새 인터뷰 생성기가 아니다. [Y/command/interview.rs](../../../crates/yo-tui/src/command/interview.rs)는 현재 Session의 live draft continue/discard, 종료 draft read-only view를 안내한다. [Y/runner/interview.rs::command](../../../crates/yo-tui/src/runner/interview.rs)는 contextual copy만 선택하고 live면 `continue`, ended면 `view`, 둘 다 `discard`를 제공한다. empty는 가능한 명령을 문서로 표시; no copy는 `No unfinished interview draft in this Session`; `close`는 저장 후 editor를 비우는 지원 경로다. obsolete list/recover/reopen/send는 거절된다. [Y/runner/state/interview.rs::apply_interview_command](../../../crates/yo-tui/src/runner/state/interview.rs)는 expanded Interview 문서를 만들며 실패 시 입력을 복원하고 request overlay/secret editor와 동기화한다. test [Y/runner/state/interview_tests.rs::dead_draft_is_contextual_read_only_and_has_no_archive_commands](../../../crates/yo-tui/src/runner/state/interview_tests.rs), `live_request_can_restore_draft_only_for_its_exact_activity`, `other_session_cannot_discover_draft`, `duplicate_contextual_drafts_fail_closed_without_deleting_either`, `tui_command_stays_in_session_and_never_starts_new_conversation`는 exact activity liveness·session 격리·모호한 copy 실패·새 conversation 미생성을 확인한다.

**Pi/Codex.** Pi의 extension `select/input/editor/custom` UI와 Codex request-user-input overlay는 질문 조작(T18) 비교로 유용하지만 Yo의 durable Session-scoped working-copy lifecycle과 같은 기능을 확인하지 않았다. 따라서 요청 위젯이 있다는 이유로 recover/discard parity를 주장하지 않는다.

**개선·수락.** 문서 첫 줄에 Live/Read-only/No draft/Unavailable를 일관되게 표시하고 `continue`는 현재 요청에 답한다는 의미, `discard`는 미전송 draft를 없앤다는 효과를 바로 보여 준다. `/interview`는 새 작업/재전송을 뜻하지 않도록 용어를 유지한다. **수락:** live draft 편집→close→continue는 동일 request만 복원; 원 request 종료 후 continue 실패·view만 가능; resume에서 역사 replay를 live로 오인하지 않음; 다른 session copy는 미노출; discard 뒤 event가 작업 사본을 다시 만들지 않음. 실제 디스크 실패/SecretRecoveryDestination 전체는 별도 계약·테스트 소유다.

## T64 · startup/resume·durability 경고와 복구 — P1 보존 / P2 발견성

**Yo 현재.** [Y/runner/session/metadata.rs::TuiSessionInfo::startup_notice](../../../crates/yo-tui/src/runner/session/metadata.rs)는 host가 지정한 new/resumed 상태와 backend/workspace만 문서화하며 각 label 4096 chars에서 제한한다. metadata 없으면 꾸미지 않는다. [Y/runner/state/observation.rs::observe_durability](../../../crates/yo-tui/src/runner/state/observation.rs)는 capacity/storage/integrity 원인과 Known/KnownEmpty/Unknown durable cutoff를 구분하고 `New activity stays in memory. Copy important output before closing yo.`를 표시한다. 같은 상태는 dedup, 실제 Durable outcome만 `The complete session has been saved` recovery를 말한다. chrome의 `History not saved`는 host status보다 우선하지만 optional row가 없으면 transcript notice가 주 단서다. 대표 test [Y/runner/tests/state_edges.rs::storage_failure_discloses_its_cutoff_until_authoritative_recovery](../../../crates/yo-tui/src/runner/tests/state_edges.rs)는 원인/cutoff/dedup/복구를 직접 검증한다. [Y/runner/tests/views/navigation.rs::storage_failure_status_survives_new_chat_and_narrow_resize](../../../crates/yo-tui/src/runner/tests/views/navigation.rs), [Y/runner/tests/appearance/interaction.rs::host_startup_notice_survives_width_changes_without_starting_work](../../../crates/yo-tui/src/runner/tests/appearance/interaction.rs)가 context reset/reflow에서도 사실 보존을 확인한다.

**Pi/Codex.** Pi `interactive-mode.ts`의 failed resume/import/create 경로는 `handleFatalRuntimeError`, export failure는 showError로 드러낸다. Codex `chatwidget.rs::rollout_path`는 저장 경로가 알려져도 첫 user message까지 persistence가 deferred일 수 있음을 명시한다. 둘에서 Yo처럼 저장 cutoff와 live-memory suffix를 persistent chrome에 투영하는 정확한 equivalent는 이번 검사에서 확인하지 못했다. generic error message만 보고 동일 durability UX라 하지 않는다.

**개선·수락.** 저장 경고의 compact label이 사라지는 작은 높이에도 `/status`에서 현재 durable boundary와 다음 행동을 볼 수 있는 경로를 보장한다. recovery가 새 turn completion/queue drain에 묻혀 '저장됨'으로 잘못 읽히지 않게 한다. **수락:** gap→새 message→view 전환→new chat→24 columns에서 warning 상태 유지; unknown은 마지막 저장점을 추정하지 않음; 성공 frame/turn만으로 recovery 선언 없음; authoritative recovery 뒤에만 warning 해제. emergency export는 durable archive가 아닌 아직 memory-only suffix까지 담는지 별도 사용자 여정 검증이 필요하다.

통합 목차·작업 우선순위·루트의 실행 검증은 [64개 TUI 검수표](./tui-review.md)를 따른다.
