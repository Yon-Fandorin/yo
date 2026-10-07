# T01–T20 입력·요청 소스 검수

> Status: non-authoritative research audit

2026-10-05. 읽기 전용 검수. Yo `AGENTS.md`, `CONTRIBUTING.md`, `docs/src/workflows/find-the-change.md`, active checkpoint, `agent.runtime.active-turn-input`, `tui.surface.bounded-view`를 읽었다. 제품 코드·테스트를 수정하거나 실행하지 않았다. 아래 테스트는 **본문을 읽은 근거**이며 통과/실제 터미널 재현 주장이 아니다. 외부 소스는 부모가 제공한 Pi v1.0.2 `cd32f7725fdbddbaecdff5b1e68491563394e0ca`, Codex v0.160.0 `a956835d020762cb2b570053af06f643a11c0ecc` 트리다.

경로 약어: Y=`crates/yo-tui/src/`; P=`/home/yon/projects/pi/`; C=`/tmp/yo-codex-rust-v0.160.0-source/openai-codex-79b1b66/codex-rs/tui/src/`. P/C 경로는 해당 pin의 GitHub 소스 링크로 전환할 수 있다. 우선순위는 이번 내부 UI 감사 제안이며 승인된 계약 변경이 아니다. P0은 의도하지 않은 전송/권한/숨겨진 값 또는 핵심 행동 오표시, P1은 실제 사용 마찰, P2는 개선/추가 검증이다.

## 먼저 검토할 구체 문제

1. **T19 P0: secret의 `Ctrl-U clear`가 전체 값을 지우지 않는다.** [Y/input/secret.rs:393](../../../crates/yo-tui/src/input/secret.rs#L393)은 `kill_line_start()`를 쓴다. [Y/input/secret/tests.rs:87](../../../crates/yo-tui/src/input/secret/tests.rs#L87)은 `first\nsecond` → Ctrl-U → Enter의 전송값이 `first\n`이라고 명시적으로 확인한다. footer [Y/shell/chrome/help.rs:47](../../../crates/yo-tui/src/shell/chrome/help.rs#L47)는 clear라고 표시한다. 고정 public state 때문에 숨은 앞부분을 사용자가 확인할 수 없다. 전체 값 clear 의미로 맞추는 최소 변경이 필요하다.
2. **T20 P0 후보: 일반 초안이 새 일반 질문의 답변으로 재분류된다.** `observation.rs:372`, `requests.rs:103`, `input.rs:354,770`, `requests.rs:301` 경로는 ordinary editor를 그대로 사용한다. 질문 전 작성 중인 `write tests next`가 새 질문 표시 뒤 Enter에 `RespondToUserInput`로 갈 수 있다. source-derived 재현 경로이며 이번 실행 재현은 없다. Pi는 extension input 컴포넌트, Codex는 별도 plain-text composer로 주 입력과 분리한다.
3. **T16 P0: 거절 선택지가 없는 승인에서도 footer가 Esc decline이라고 한다.** `presentation.rs:246`은 Approval로 고정, `shell.rs:369`는 request footer 우선, `help.rs:31`은 Esc decline. 실제 `requests.rs:345`는 offered decline이 없으면 Turn Interrupt. 패널의 Stop turn과 footer 의미가 다르다.
4. **T04/T10 P1: 이미지 marker undo가 이미지 복구를 뜻하지 않는다.** `runner/state/image/tests.rs:477`은 `[image]` 복원 뒤 attachments=0을 보장한다. 일반 undo는 존재하지만 text-only이다. 실제 첨부와 동일하게 보이는 잔여 marker를 구분해야 한다.
5. **T18 P1: notes 미지원 choice에서 Tab은 답을 즉시 제출한다.** `request_responses.rs:1100` 본문이 `allow_notes=false`의 Tab → `RespondToUserInput("2")`를 확인한다. footer는 이 상태를 Answer로 분류해 newline을 안내하며 Tab의 제출 의미를 알려주지 않는다.

## T01 입력창 구조·placeholder

- Yo: [Y/prompt.rs:85,134,212](../../../crates/yo-tui/src/prompt.rs#L85), [Y/prompt/viewport.rs:28](../../../crates/yo-tui/src/prompt/viewport.rs#L28)는 측정/그리기와 cursor viewport를 분리한다. placeholder는 input에 포함되지 않으며 폭 30 미만에서 전부 숨긴다. [Y/prompt/tests/scrolling.rs:96](../../../crates/yo-tui/src/prompt/tests/scrolling.rs#L96) 본문은 1열에서 높이 확대 뒤 first row와 cursor를 확인한다. 기존 구현에 bounded prompt와 cursor-follow가 이미 있다.
- Pi: [P/packages/tui/src/components/editor.ts:520](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/tui/src/components/editor.ts#L520)은 terminal 높이의 30%, 최소 5행 viewport 및 위/아래 스크롤 경계를 계산한다. Codex: [C/bottom_pane/chat_composer.rs:4581,4605](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs#L4581)의 측정/렌더 layout은 composer/attachments/popup/footer 영역을 함께 사용한다. 각 체계를 통째로 가져올 필요는 없다.
- **P2 최소 개선:** 좁은 창에도 짧은 빈 상태 문구와 현재 주 행동을 남긴다. 큰 초안은 prompt 자체 scroll이 있으므로 missing multiline editor로 분류하지 않는다. 큰 붙여넣기 축약은 T03의 별도 선택 사항이다.
- 수용: 80→24→80열과 4–10행에서 빈 prompt, 긴 한글 초안의 cursor·문자 내용 불변, visible cursor/주 행동 유지. SSH에서 실제 cursor 위치는 미검증.

## T02 커서·선택·문자 편집

- Yo: [Y/input/editor.rs:291,365](../../../crates/yo-tui/src/input/editor.rs#L291), [Y/input/buffer.rs:91,113,212](../../../crates/yo-tui/src/input/buffer.rs#L91)는 grapheme 이동/삭제, Ctrl+Left/Right 단어 이동, Ctrl+A/E 행 이동, 시각행 Up/Down과 preferred column을 갖는다. [Y/input/editor/tests.rs:248](../../../crates/yo-tui/src/input/editor/tests.rs#L248)은 가족 emoji/한글/combining의 정확한 byte 경계를 확인한다.
- 빠진 것은 기본 CJK 편집이 아니다. [Y/runner/state/input.rs:643](../../../crates/yo-tui/src/runner/state/input.rs#L643)은 nonempty draft일 때 **Up/Down만** view handler보다 우선한다. [Y/runner/view.rs:753](../../../crates/yo-tui/src/runner/view.rs#L753)의 Home/End는 transcript start/tail이고 PromptEditor는 해당 키를 처리하지 않는다. Shift 선택 상태도 editor에 없다.
- Pi: [P/packages/tui/src/keybindings.ts:80](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/tui/src/keybindings.ts#L80) 이후 Home/End, Alt+B/F, Ctrl+B/F, kill/yank-pop 등의 별칭을 제공한다. Codex: [C/bottom_pane/textarea.rs:190,224,292](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/textarea.rs#L190)는 keymap과 Vim editing 상태를 가진다. Yo Ctrl+F는 검색이므로 그 별칭을 그대로 복사하면 충돌한다.
- **P1 최소 개선:** 편집 중 Home/End의 소유권을 명시/일반 편집 기대에 맞추고 transcript 이동은 기존 대체키와 함께 안내한다. 선택 기능 확대는 P2로 분리한다.
- 수용: nonempty multiline/CJK draft에서 Home/End 행동이 화면 도움말과 일치, Ctrl+F 검색 의미와 view navigation 보존.

## T03 개행·붙여넣기

- Yo: [Y/terminal/backend/unix/input.rs:136](../../../crates/yo-tui/src/terminal/backend/unix/input.rs#L136)은 Crossterm Paste를 semantic Paste로 전달한다. [Y/input/editor.rs:185,312](../../../crates/yo-tui/src/input/editor.rs#L185)은 payload를 literal insert하며 plain Enter submit, 선택된 modifier+Enter newline이다. [Y/input/editor/binding.rs:31](../../../crates/yo-tui/src/input/editor/binding.rs#L31) 기본은 Shift. [Y/input/editor/tests.rs:398](../../../crates/yo-tui/src/input/editor/tests.rs#L398)은 `a\n\u{3}b`가 interrupt 없이 literal로 남음을 확인한다. 설정 가능한 modifier+Enter는 이미 있다.
- Pi: [P/packages/tui/src/keybindings.ts:151](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/tui/src/keybindings.ts#L151)은 Shift+Enter와 Ctrl+J 기본 개행. `components/editor.ts:1259`는 tmux CSI-u control decoding, newline normalization, 10행/1000자 초과 paste marker 및 원문 저장을 구현한다. Codex: [C/bottom_pane/paste_burst.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/paste_burst.rs)의 state machine은 bracketed paste가 없는 char/Enter burst를 구분한다. `chat_composer.rs:9029,9089` 테스트 본문은 control sanitization 및 large paste의 marker→원문 제출을 확인한다.
- **P1 최소 개선:** enhanced key 미지원 환경에서 도달 가능한 개행 대체 경로를 명확히 안내한다. **P1 검증 과제:** Yo에는 검사한 event/editor 경로에 unbracketed burst 분류가 없으므로 raw char+Enter paste가 submit하는지 terminal-level 시나리오를 먼저 확인하고 필요할 때 bounded detector를 추가한다. bracketed paste의 정상 동작을 부정하지 않는다. 큰 paste 축약은 P2.
- 수용: 한글+여러 줄+slash를 bracketed/unbracketed 각각 붙여넣고 중간 submit 0회; 실제 Enter만 제출. Ctrl+J가 어떤 semantic key로 decoding되는지는 실제 terminal별 검증 필요.

## T04 undo·kill/yank·초안 복원

- Yo: [Y/input/editor.rs:28,153,267](../../../crates/yo-tui/src/input/editor.rs#L28)은 64 snapshots/2MiB, typing coalescing, atomic paste, Ctrl+U/K/W/Y와 Ctrl+-/7/_ undo를 제공한다. `input/editor/tests.rs:68` 본문은 multiline clear와 삭제를 cursor까지 되돌린다.
- 문제: snapshot이 TextBuffer뿐이다. [Y/prompt/image.rs:36](../../../crates/yo-tui/src/prompt/image.rs#L36)/reference transform은 삭제된 annotation을 제거하고 undo는 payload를 복원하지 않는다. [Y/runner/state/image/tests.rs:477](../../../crates/yo-tui/src/runner/state/image/tests.rs#L477)은 marker만 복원되고 images empty를 의도적으로 확인한다. 이 상태를 attachments 복구로 설명하면 잘못이다.
- Pi: [P/packages/tui/src/components/editor.ts:2115](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/tui/src/components/editor.ts#L2115) snapshot에는 state+pastes+pasteCounter가 포함된다. Codex: [C/bottom_pane/chat_composer.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs)의 draft/Vim undo 설명과 구성은 complete draft를 소유한다. Pi의 paste payload 복구와 실제 typed image identity는 다른 범위다.
- **P1 최소 개선:** 복원된 plain marker와 실제 attachment를 시각적으로 구분하고 재첨부 행동을 안내한다. 더 큰 선택은 bounded whole-UserInput undo지만 image memory/identity 계약 검토가 필요하다.
- 수용: image/reference 삭제→undo→submit에서 실제 payload 개수와 화면 표시가 항상 일치; no silent phantom attachment. redo/다중 kill ring은 후순위다.

## T05 입력 history

- Yo: [Y/runner/state/history.rs:23,169,229,254,266](../../../crates/yo-tui/src/runner/state/history.rs#L23)은 같은 Session의 accepted Start/Steer 32개/32MiB만 보유하며 resumed seed도 지원한다. Ctrl+R 검색, Esc 원 초안/editor+assist 복원, 선택은 편집창 삽입이며 resend하지 않는다. reference/image metadata도 함께 되살린다. `history/tests/mod.rs:242` 본문은 query 갱신 후 fresh frame 전 Enter 차단과 한글 draft/cursor 복원을 확인한다.
- Pi: [P/packages/tui/src/components/editor.ts:424,457](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/tui/src/components/editor.ts#L424)은 최근 100개 string, 중복 제거, Up/Down 진입/원 draft 복원이다. Codex [C/bottom_pane/chat_composer.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs)의 history routing은 persistent text와 local rich entries를 구분하고 Ctrl+R preview/cancel을 제공한다. Yo의 structured local/resumed history는 강점이며 global history 부재를 곧 결함이라고 할 수 없다.
- **P2 최소 개선:** `Recall prompt` 제목에 same session/최근 32개 범위를 알리고, 긴 항목을 선택 전에 더 읽는 경로를 제공한다. 현재 label은 96 grapheme로 축약되며 파일/이미지 수만 detail에 표시한다.
- 수용: 비슷한 앞부분의 두 긴 prompt 구분; cancel 후 cursor/refs/images 완전 일치; secret 및 reject는 검색 결과에 없음. 저장 history의 모든 backend 복원까지 이번 검수로 증명하지 않는다.

## T06 대화 찾기

- Yo: [Y/runner/state/find.rs:26,54,167](../../../crates/yo-tui/src/runner/state/find.rs#L26)의 finalized ordinary user/assistant message 검색은 이미 구현되어 있다. 최대 1024개/2MiB, 메시지 256KiB, 4096개 scan, 100 results이며 cap 상태를 표시한다. snapshot corpus와 query 전용 임시 editor, 원 rich draft 복원을 사용한다. `find/tests/mod.rs:289`는 2×4 숨은 panel에서도 Enter가 saved draft를 보내지 않음을 확인한다.
- Pi: [P/packages/tui/src/tui-alt-screen.ts:723](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/tui/src/tui-alt-screen.ts#L723)에서 alt-screen search toggle/next/previous/close를 routing한다. Codex [C/pager_overlay/transcript.rs:245](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/pager_overlay/transcript.rs#L245)는 transcript view 검색으로 진입하며 [C/transcript_view/search_tests.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/transcript_view/search_tests.rs)에는 live/commit 및 prepended history의 별도 범위가 있다(그 테스트는 이름/경로만 확인, 실행/본문 전부 확인 아님).
- **P2 최소 개선:** corpus가 진입 시점의 finalized Chat snapshot임을 지속 표시하고 새 완료 메시지가 생겼을 때 refresh 행동을 제공한다. tool output/Request 검색 확대는 다른 corpus 계약으로 분리한다.
- 수용: 검색 열린 뒤 assistant 완료→refresh 전/후 결과 차이가 설명됨; cap과 no match 구분; cancel 시 rich draft 불변. 현재의 search를 없다고 기록하지 않는다.

## T07 파일 참조

- Yo: [Y/prompt/workspace_reference.rs:101,146,239](../../../crates/yo-tui/src/prompt/workspace_reference.rs#L101)는 request_id/sequence/terminal 상태를 검사하고 pending 동안 old usable rows를 비활성화하며 typed workspace/root/env identity를 삽입한다. whole trigger 범위 대체, path 종류, provider failure/incomplete 상태가 있다. `workspace_reference/tests.rs:37` 본문은 공백 경로 directory 선택과 typed identity를 확인한다. source 검사상 단순 textual @완성보다 강하다.
- Pi: [P/packages/tui/src/autocomplete.ts:314,447](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/tui/src/autocomplete.ts#L314)은 @ fuzzy file search와 quote/space/directory suffix 보존. Codex [C/bottom_pane/chat_composer.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs) mention routing은 token boundaries와 atomic elements를 관리하며 현재 @가 file뿐 아니라 plugin/skill도 포함한다. Yo와 검색 범위를 동일하다고 하면 안 된다.
- **P2 최소 개선:** 선택 후 파일 이름만 남는 projection에 참조 개수/선택 상세를 접근 가능하게 붙이고, 긴 공통 prefix의 경로를 좁은 창에서도 구분하는 acceptance를 추가한다. stale 차단은 보존한다.
- 수용: 동일 basename, 공백·한글·길이 긴 경로 24열에서 선택 identity 구분; query 변경 후 늦은 결과가 새 선택을 만들지 않음; cursor 이동과 Enter의 결합 검증. 전체 filesystem indexing 품질은 범위 밖.

## T08 skill 참조

- Yo: [Y/prompt/skill_reference.rs:101,225,344](../../../crates/yo-tui/src/prompt/skill_reference.rs#L101)는 scope filter와 같은 이름/source 중복의 최소 구분 prefix, disabled reason, accepted typed identity를 갖는다. `skill_reference/tests.rs:81` 본문은 32열에서 #1/#2 source가 구분되고 disabled 항목이 남는 것을 확인한다. 단순 skill 목록 부족은 아니다. 한 request에 explicit skill 한 개 제한은 분명한 메시지로 구현되어 있다.
- Pi: [P/packages/tui/src/autocomplete.ts:339](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/tui/src/autocomplete.ts#L339)은 slash `skill:` bare-name fuzzy matching과 argument description을 사용한다. Codex [C/bottom_pane/chat_composer.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs) mention menus는 $ skill/app, @ unified menu의 별도 의미를 가진다. upstream의 개수/확장 범위를 Yo의 지원 계약이라고 주장하지 않는다.
- **P2 최소 개선:** 1개 제한은 유지하되 제품 문구에서 `Version 1` 구현 명칭보다 `이미 선택한 skill을 제거/교체`의 다음 행동을 명확히 한다. 다중 skill 확장은 별도 제품 결정이다.
- 수용: 동일명 세 source 24/32열에서 올바른 locator 선택, 필터 중 pending 결과는 수락 불가, 두 번째 trigger에서 교체 경로 이해. provider capability의 실제 remote 동작은 미검증.

## T09 prompt template

- Yo: [Y/runner/state/commands.rs:560](../../../crates/yo-tui/src/runner/state/commands.rs#L560) `/prompt`는 이름을 문서로 출력하고 `/prompt NAME`은 body를 literal editor에 넣는다. popup 목록에서 이름을 직접 선택하는 flow는 없다. missing template는 초안 보존. `runner/tests/prompt_templates.rs:34`는 `/exit` body조차 첫 Enter에 agent input으로 보내고 command로 실행하지 않음을 확인한다. `command/palette.rs:23`은 literal draft가 변경되면 literal 보호를 해제한다.
- Pi: [P/packages/coding-agent/src/core/prompt-templates.ts:70](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/prompt-templates.ts#L70)은 positional/default/slice arguments를 expansion하며 autocomplete command에 argument hint를 넣는다(`P/.../autocomplete.ts:339`). Codex: 검사한 [C/bottom_pane/chat_composer.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs)에는 현재 custom prompt/template 대응 경로를 찾지 못했다. skills/slash UI를 saved-template parity로 대체하여 주장하지 않는다.
- **P1 최소 개선:** `/prompt`를 이름+짧은 preview의 picker로 제공하고 선택은 삽입만 수행한다. body를 수정하면 slash 의미가 바뀔 수 있는 점은 literal/command mode를 사용자가 알 수 있게 한다. parameter language 추가는 P2/별도 결정.
- 수용: 템플릿 목록→선택→검토→Enter 2단계 유지; `/exit`, `$skill`, `@path`, multiline body에 숨은 실행 없음; 수정 후 mode를 명시적으로 검증.

## T10 이미지·첨부 입력

- Yo: [Y/runner/state/image.rs:24,58,143](../../../crates/yo-tui/src/runner/state/image.rs#L24)은 /attach와 Ctrl+V image 준비, 16개/64MiB 소유 한도, 준비 중 submit/queue 방지, stale revision 거절, 모델/요청 전이 eligibility를 구현한다. 출력은 source/transmission/thumbnail 차원을 명시한다. `image/tests.rs:63`은 rejected submission 뒤 이미지 snapshot/metadata까지 동일함을 확인한다. `prompt/image.rs:27`은 마지막 image의 thumbnail만 돌려준다.
- Pi: [P/packages/coding-agent/src/modes/interactive/interactive-mode.ts:3084,3124](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/interactive-mode.ts#L3084)은 copied file→image 임시 파일→text fallback 흐름을 갖는다. Codex [C/bottom_pane/chat_composer.rs:1223,3468](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs#L1223)은 image path와 remote-image 선택 상태를 처리한다. Yo Ctrl+V는 text fallback이 아닌 image action이므로 terminal paste와 구분해야 한다.
- **P1 최소 개선:** 여러 이미지의 filename/count/removal을 작은 attachment 목록으로 제공한다. `[image]` 반복+마지막 thumbnail만으로 어느 사진인지 판별하기 어렵다. 모델 비호환은 draft 보존되지만 가능한 경우 제출 전 지원 상태를 보여준다. undo 분리 문제는 T04.
- 수용: 사진 3개 중 가운데 제거, 준비 중 edit/cancel→늦은 결과 무시, 모델 변경/거절 후 정확한 image identity 보존. 실제 OS clipboard/SSH 전송/EXIF 시각 품질은 별도 실행 과제.

## T11 외부 편집기

- Yo: [Y/runner/state/external_editor.rs:29,67](../../../crates/yo-tui/src/runner/state/external_editor.rs#L29) Ctrl+G는 Chat/무요청/무overlay일 때 snapshot generation을 만들고, import는 text/cursor/identity가 같은지 확인한다. 참조 바깥 edit는 span 이동, annotation 교차 edit는 ambiguous로 거절한다. `external_editor/tests.rs:96` 본문은 import가 한 번 undo 가능하고 submit하지 않음을 확인한다. CLI `crates/yo-cli/src/application/runtime/frontend.rs:196` 왕복과 `execution/process/external_editor.rs:56,306,347`의 VISUAL/EDITOR, 1MiB/UTF-8, foreground group restoration을 읽었다.
- Pi [P/packages/coding-agent/src/modes/interactive/external-editor.ts:38](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/external-editor.ts#L38)은 BOM 및 마지막 newline 하나 제거; Codex [C/external_editor.rs:176](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/external_editor.rs#L176) 이후 temp .md 저장/프로세스 status/원문 read를 사용한다. Yo가 whitespace 보존 및 annotation ambiguity 거절하는 것이 강점이다.
- **P2 최소 개선:** annotation 때문에 전체 결과가 거절된 때 해당 참조/이미지 재선택 필요와 기존 draft 유지 사실을 구체적으로 안내한다. 외부 editor는 기능이 이미 있고 shell subprocess authority를 TUI 안으로 옮길 이유가 없다.
- 수용: plain empty 결과는 의도적 clear, marker가 있는 전체 삭제는 명시적 거절, 실패 후 원 draft/cursor 유지, 한글/newline exact. 실제 termios/tmux 왕복 실행은 부모 검사에 맡긴다.

## T12 Enter·steer

- Yo: [Y/runner/state/input.rs:784,1154](../../../crates/yo-tui/src/runner/state/input.rs#L784)는 observed active Turn의 정확한 TurnRef로 steer하며 pending submission의 input snapshot을 보존한다. acceptance 때 현재 draft가 snapshot과 같을 때만 지운다. UnsupportedSteer는 explicit notice와 Alt+Q 선택을 주며 silent queue fallback 없다. `runner/tests/admission.rs:99` 본문은 older acceptance가 newer draft를 지우지 않음을 확인한다.
- Pi `P/.../interactive-mode.ts:3330` Enter streaming→steer를 호출하고, Codex [C/bottom_pane/chat_composer.rs:3521](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs#L3521)의 queue/submit 구분 및 `pending_input_preview.rs:89`가 pending/rejected/follow-up 내용을 보여준다. Codex rejected-steer end-of-turn behavior는 Yo 계약과 다르므로 그대로 가져오면 안 된다.
- **P0 최소 개선:** [Y/shell/chrome/help.rs:111](../../../crates/yo-tui/src/shell/chrome/help.rs#L111) active footer에 Enter의 현재 의미(현재 Turn에 전달), admission pending/accepted, known unsupported 상태를 보여준다. 현 footer는 queue/interrupt/newline만 있어 핵심 Enter가 빠진다. backend 미지원이 발견되기 전/후를 구분한다.
- 수용: 도움말 없이 Enter vs Alt+Q 구분; finishing race에서 exact-turn reject, draft 유지, 자동 새 Turn 0회; admission 중 newer draft 보존.

## T13 queue·편집·재개

- Yo: [Y/runner/state/input.rs:1213,1265,1277](../../../crates/yo-tui/src/runner/state/input.rs#L1213) queue는 16개/64KiB, independent UserInput snapshots이다. interrupted/rejected 뒤 pause, Alt+R은 빈 editor에 FIFO 한 개 recall, Alt+Q 빈 draft는 resume, 새 queue 추가도 resume한다. `runner/tests/session_lifecycle.rs:190,216` 본문은 rejection 뒤 fresh SubmissionId와 newer draft 보존을 확인한다.
- Pi `P/.../interactive-mode.ts:4648,4668`은 Steering/Follow-up 내용을 출력하지만 dequeue는 모든 queued text와 현재 draft를 합친다. Codex [C/bottom_pane/pending_input_preview.rs:89,173](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/pending_input_preview.rs#L89)은 개별 내용 3행+ellipsis와 edit binding을 별도로 보여준다. Yo의 typed snapshot 분리는 유지할 가치가 크다.
- **P0 표시/P1 편집:** queued count가 footer의 주 action을 대체하는 현상을 없애고 작은 내용 preview/paused 이유를 추가한다. 항목 선택·수정·삭제는 기존 draft를 별도 보관한 bounded picker로 한다. Pi식 문자열 병합은 채택하지 않는다.
- 수용: 이미지가 든 두 queued messages 중 두 번째만 수정하며 현재 draft 보존; pause 중 새 항목 추가가 resume한다면 사전에 명확히 표시; 조작 없는 자동 resume 0회.

## T14 interrupt·exit

- Yo: [Y/input/control.rs:32,71,106](../../../crates/yo-tui/src/input/control.rs#L32) active Esc/Ctrl+C는 interrupt, idle Ctrl+C는 전체 draft clear, empty double Ctrl+C(1s)는 exit, empty Ctrl+D는 active 여부와 무관하게 exit한다. repeated control은 suppress한다. `runner/tests/interrupt.rs:74` 본문은 backpressure에서도 Esc interrupt를 확인한다. `input/control/tests.rs:70`은 active Ctrl+C가 draft를 보존한다.
- Pi `P/.../interactive-mode.ts:4201` Ctrl+C clear/500ms double shutdown, Ctrl+D empty shutdown; Ctrl+C의 active 의미가 Yo와 같다고 가정하면 안 된다. Codex [C/bottom_pane/chat_composer.rs:3468](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs#L3468)은 Esc hint state를 분리하고 popup/empty states를 우선한다.
- **P1 최소 개선:** [Y/runner/state/input.rs:819](../../../crates/yo-tui/src/runner/state/input.rs#L819)이 ExitArmed를 Unchanged로 삼켜 재입력 힌트가 없는 부분을 표시 상태로 만든다. active interrupt 중에도 draft 보존/queue pause를 함께 보여준다. overlay close와 turn interrupt를 구분한다.
- 수용: idle nonempty→clear→undo, empty 첫 Ctrl+C→1초 안내→두 번째 exit, active→interrupt; repeat/release로 accidental exit 없음. terminal signal Ctrl+C delivery는 실제 테스트 별도.

## T15 slash palette

- Yo: [Y/command/palette.rs:23,178,202](../../../crates/yo-tui/src/command/palette.rs#L23)은 prefix query, end-cursor eligibility, fresh overlay receipt, Esc literal escape를 제공한다. `runner/tests/command_palette.rs:344` 본문은 Esc가 `/`를 보존하고 제출하지 않음을 확인, 이어 selected Help 테스트는 local dispatch를 확인한다. exact commands는 일부 frame 없이 실행할 수 있다.
- **구체 격차:** palette는 `available.contains`로 unavailable command를 아예 빼므로 기능 존재/불가 이유가 popup에서 사라진다. argument 들어간 `/prompt NAME` 등의 완성은 generic command_query 범위 밖이다.
- Pi [P/packages/tui/src/autocomplete.ts:339,431](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/tui/src/autocomplete.ts#L339)은 fuzzy command/name+argument hint, Codex [C/bottom_pane/chat_composer.rs:4649](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs#L4649)는 command popup을 composer 위에서 렌더한다. 두 reference의 광범위 command 목록을 Yo에 요구하지 않는다.
- **P1 최소 개선:** unavailable command는 reason이 있는 disabled row로 노출하고 argument-bearing commands의 다음 입력 형태를 보여준다. Esc 후 literal 제출의 의미도 한 번 안내한다.
- 수용: active Turn에서 `/new`가 왜 사용 불가인지 알 수 있고 disabled를 Enter로 dispatch 불가; `/h` 선택/`/help` exact/escaped slash의 결과 일치.

## T16 승인 선택

- Yo: [Y/runner/state/requests.rs:336,401](../../../crates/yo-tui/src/runner/state/requests.rs#L336) offered ordinals를 보존하면서 decline을 첫 행으로 옮기고, decline 없으면 Stop turn을 앞세운다. invalid/unrenderable choices는 fail-closed. `runner/tests/request_responses.rs:1279` 본문은 24열/갱신/fresh-frame와 original ordinal의 전달을 확인한다. safe default와 capability-driven offered choices가 강점이다.
- Codex [C/bottom_pane/approval_overlay.rs:257,334,543](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/approval_overlay.rs#L257)은 request kind에 따라 options/shortcut/action을 구성하고 fullscreen/header를 제공한다. 해당 파일의 `enter_sets_last_selected_index_without_dismissing` 테스트는 기본 Enter Accept를 기대하므로 Yo의 decline-first가 더 보수적이다. Pi `P/.../interactive-mode.ts:2715` showExtensionConfirm은 Yes/No selector이며 Yo의 exact Activity approval/grant 동등 기능 근거는 아니다.
- **P0 최소 개선:** no-decline request의 Esc footer를 실제 Stop turn으로 바꾸고 approve 범위/decline vs interrupt 결과를 구체화한다. `presentation.rs:246`, `shell.rs:369`, `shell/chrome/help.rs:31` 불일치 확인.
- 수용: decline 있음→Esc offered decline; 없음→Esc interrupt, 두 경우 화면 문구/기본 Enter 일치; snapshot 교체 뒤 이전 선택을 승인하지 않음.

## T17 긴 명령·승인 diff

- Yo: [Y/runner/state/input.rs:480](../../../crates/yo-tui/src/runner/state/input.rs#L480), `requests.rs:440`는 Review proposed files를 통해 exact related change로 이동한다. F2 semantic-record diagnostic이 있고 unchanged saved panel은 roundtrip에 복구된다(`requests.rs:137,161`). `runner/tests/request_responses.rs:1524` 본문은 관련 diff만 열고 missing relation은 unavailable notice, 승인 action 미발행을 확인한다. 즉 승인 diff 보기 자체가 없는 것은 아니다.
- Codex [C/bottom_pane/approval_overlay.rs:543](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/approval_overlay.rs#L543)의 open_fullscreen은 선택행과 독립된 FullScreenApprovalRequest를 발행하고 header_view_all_hint를 제공한다. Pi showExtensionConfirm의 title/message는 별도 full command/patch approval pipeline과 동일하다고 주장할 수 없다.
- **P1 최소 개선:** approval 바로 옆 `전체 요청 보기` 행동을 두고 그것이 read-only history인지 현재 판단 대상인지 구분한다. F2는 semantic-record diagnostic이다(T33). 현재 승인 요청의 전문 reader와 동등하다는 근거는 없으며, 긴 명령의 실행 대상·working directory·scope를 확인하는 전용 상세 진입을 구분해야 한다. Changes roundtrip은 보존한다.
- 수용: 긴 multiline command 및 1000행 diff를 좁은 창에서 끝까지 읽고 복귀; selected offered ordinal 유지, 내용 갱신 시 fresh frame 요구, missing relation이 다른 diff로 대체되지 않음. 화면/scroll 전체성은 실제 render 검사 필요.

## T18 질문·choice·notes·Previous

- Yo: [Y/runner/state/input.rs:282,511,538](../../../crates/yo-tui/src/runner/state/input.rs#L282), `requests.rs:103,472`는 typed question, choices/free text, capability allow_notes, Shift+Tab Previous, host draft 복원, notes 중 literal `/exit`를 이미 갖는다. `runner/tests/request_responses.rs:1100` 본문은 24열 reflow 뒤 choice=2와 한글/slash notes를 exact response로 보내는 것을 확인한다.
- **P1 실제 문제:** 같은 테스트에서 allow_notes=false일 때 Tab은 선택을 즉시 보낸다. generic overlay select binding의 결과이며 question footer에서는 Tab-submit을 알려주지 않는다. notes 지원 유무에 따라 Tab의 위험도가 달라진다.
- Codex [C/bottom_pane/request_user_input/mod.rs:499,744,828,842](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/request_user_input/mod.rs#L499)는 per-question drafts, question navigation, last-submit aggregation, unanswered confirmation을 가진다. Pi `P/.../interactive-mode.ts:2728,2784`는 extension input/editor와 focus roundtrip이 대응점이며 built-in durable multi-question interview의 parity는 미확인.
- 최소 개선: notes 미지원 Tab은 이동/무동작 또는 명시된 submit semantics로 정리하고, 현재 질문의 위치와 마지막 전송 여부를 보여준다. host가 Previous를 지원하지 않을 때 synthetic local back를 만들지 않는다.
- 수용: allow_notes true/false의 동일 키 시나리오; notes 유지→Previous→return; 마지막 response durable seal 이후 draft cleanup은 core contract/통합테스트를 별도로 검증해야 하며 TUI만으로 증명하지 않는다.

## T19 secret 입력·관리

- Yo: [Y/input/secret.rs:23,223,246,295](../../../crates/yo-tui/src/input/secret.rs#L23) fixed public state와 별도 bounded editor는 길이를 드러내지 않고 literal paste를 받는다. `runner/state/input.rs:824` retention/recovery 안내와 별도 submit을 처리한다. `input/secret/tests.rs:15`은 `/token\n$HOME`의 public text가 Entered뿐임을 확인한다. no secret history/refs/images fallback 및 reentry contracts가 명확한 강점이다.
- **P0:** Ctrl-U clear가 multiline 앞부분을 남기는 실제 code/test mismatch는 위 핵심 finding 참조. 전체값 삭제의 public state를 Not entered로 전환해야 재입력할 때 숨은 prefix 결합을 막는다.
- **P1 관리:** [Y/runner/state/commands.rs:607,644](../../../crates/yo-tui/src/runner/state/commands.rs#L607) `/secrets delete <64-character public ID>`는 공개 metadata 기반 삭제를 이미 지원한다. 긴 ID를 옮겨 적는 UI보다 destination/scope/expiry 행 선택 후 정확한 entry 삭제가 낫다. 이는 vault 권한/암호화 재설계 요구가 아니다.
- Codex [C/bottom_pane/request_user_input/render.rs:405](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/request_user_input/render.rs#L405)는 is_secret에서 `render_with_mask('*')`를 쓰므로 Yo의 length-independent public-state UX와 다르다. Pi의 일반 extension input에는 이 수준의 request-bound vault semantics를 확인하지 못했다.
- 수용: `first\nsecond` 붙여넣기→clear→`replacement`→submit 값이 replacement만; 길이/내용 노출 0; Previous/cancel 후 explicit reentry; public metadata 행으로 올바른 saved entry만 삭제.

## T20 요청 간 전환·초안

- Yo: [Y/runner/state/observation.rs:372](../../../crates/yo-tui/src/runner/state/observation.rs#L372), `requests.rs:71,103,301`은 FIFO pending requests와 exact refs, activity 종료 제거, stale presentation fail-closed를 갖는다. `runner/tests/request_responses.rs:491` 본문은 두 요청이 각각 정확한 RequestId로 응답함을 확인한다. secret은 별도 editor이며 일반 draft와 격리된다.
- **P0 후보의 구체 경로:** ordinary editor에 draft가 이미 있을 때 첫 일반 question이 오면 editor를 request별로 분리하지 않는다. host question draft는 `editor.text().is_empty()`일 때만 채운다. `input.rs:354` typed_reply가 overlay 선택을 우회하고 제출 후 `input.rs:770`이 pending request 응답으로 dispatch한다. normal draft가 새 질문 답변으로 전송될 수 있다. 앞 question이 externally finished되어 새 질문이 선두가 되는 경우에도 일반 text 소유자가 명시되지 않는다. 새 ordinary draft/질문 draft 소유권 분리가 필요하다.
- Pi `P/.../interactive-mode.ts:2728`은 main editor를 보관한 채 별도 ExtensionInputComponent를 보여주고 복귀한다. Codex [C/bottom_pane/request_user_input/mod.rs:182](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/request_user_input/mod.rs#L182)는 새 plain-text ChatComposer와 per-question AnswerState를 만든다. `mod.rs:250`은 queued request 진입을 reset한다.
- 최소 개선: ordinary draft(editor+annotations)를 suspend하고 request-bound answer draft를 사용한다. incoming request/finish/late update에 owner를 확인하고 원 context 복귀 시만 보통 draft를 복구한다. existing interview durable-draft/secret 규칙을 침범하지 않는다.
- 수용: typing normal draft 도중 질문 도착→Enter가 normal draft를 답으로 소비하지 않음; 일반→secret→일반, FIFO 외부 resolution, late response, 24열 숨은 패널 모두 owner 보존. 원인과 route는 정적 확인, 재현 실행은 아직 필요하다.

## 배분 목록에서 더 명시할 누락

- **입력 payload와 화면 marker의 의미 일치**: T04/T10을 가로지르며 undo, history, 외부 편집기, 모델 거절 각각에서 실제 typed payload 개수와 표시가 다른 문제. 기능 체크로는 잘 안 드러난다. 별도 ID를 늘리기보다 이 두 항목의 공통 acceptance로 유지할 수 있다.
- **요청 수신 순간의 입력 소유권 변경**: T20의 FIFO/late response만으로는 normal→question transition의 유실/의도 변경을 검수하지 못한다. 위 재현 시나리오를 T20에 명시한다.
- **secret clear-all/재입력, 선택적 저장의 삭제 discoverability**: T19에 별도 과제로 포함한다. 관리 명령은 존재하므로 missing secret manager라고 쓰지 않는다.
- **사용자 직접 shell 입력**: Pi interactive-mode.ts:3298의 `!`/`!!`, Codex composer.rs:3468의 bash mode에 대응하는 별도 Yo composer branch를 이번 범위에서 찾지 못했다. command 목록 역방향 검수에 `user-authored shell mode의 존재/의도적 미지원`를 기록할 수 있지만 agent tool approval과 혼동해서 새 기능을 요구하지 않는다.
- **큰 paste의 collapse/expand와 원문 수정**은 T03/T04/T11의 연결 acceptance다. 이미지처럼 원문 identity를 유지해야 하므로 단순 축약 label을 먼저 추가하는 접근은 피한다.

이 보고서는 source inspection과 테스트 본문 분석이다. 인증·모델 호출·빌드·실제 SSH/tmux·IME·screenreader·vault durability failure injection을 하지 않았으며 각각의 결과를 추정하지 않는다. codewright는 whole-surface audit에 비적용이므로 사용했다고 주장하지 않는다.

## 추가로 읽은 upstream 구현/테스트 본문

- T04/T05 Codex [C/bottom_pane/chat_composer.rs:1617](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs#L1617) `snapshot_draft` 본문은 text/elements/local image paths/remote URLs/mention bindings/pastes/cursor를 함께 저장한다. `:1629` restore도 각각 복구한다. [C/bottom_pane/chat_composer/history_search_draft.rs:16,40](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer/history_search_draft.rs#L16)는 search original draft와 preview를 분리하고 background edits를 original에만 반영한다. 이 실제 구조가 Yo text-only undo/일반 요청 editor 소유권 비교의 근거다. Codex의 모든 undo 모드 동등성을 주장하지 않는다.
- T05 Pi [P/packages/tui/test/editor-history-keybindings.test.ts:15](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/tui/test/editor-history-keybindings.test.ts#L15) 본문은 별도로 매핑한 Ctrl+P/N으로 multiline history를 왕복한 뒤 원 draft `draft`와 cursor column=3을 복구하는 것을 확인한다. 기본 keymap은 이 별칭을 비워 둔다.
- T06 Codex [C/transcript_view/search_tests.rs:82](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/transcript_view/search_tests.rs#L82) 본문은 CJK·accent·literal metachar·İ case folding의 원 UTF-8 span을 확인한다. live/prepended-history 테스트들은 위에서 밝힌 대로 이름/경로만 확인했다.
- T07/T08 Codex [C/bottom_pane/chat_composer.rs:2795](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs#L2795)는 선택 mention을 atomic text element로 삽입하고 element ID에 실제 path/sigil을 묶는다. `:4233`은 skill의 display name/description, `$name`, path, `[Skill]` tag를 구성한다. Yo source/scope discrimination과 비교할 실제 binding 경로가 있다.
- T20 Codex [C/bottom_pane/request_user_input/mod.rs:1726](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/request_user_input/mod.rs#L1726) `queued_requests_are_fifo` 본문은 turn-1 뒤 turn-2/3을 enqueue하고 submit 때 각각 순서대로 바뀜을 확인한다. Yo의 FIFO correlation 강점을 없다고 볼 수 없다.

통합 목차·작업 우선순위·실행 검증은 [64개 TUI 검수표](./tui-review.md)를 따른다. 이 보고서의 테스트 미실행 표시는 독립 검수자에게 해당하며, 루트가 실행한 결과는 통합 문서에 별도로 기록했다.
