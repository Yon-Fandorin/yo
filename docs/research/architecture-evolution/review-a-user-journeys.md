# A 검수: R01–R12 사용자 여정과 삼자 소스 대조

> 일차 검수 보고서 원문이다. 검토 시점의 발견·부분 검토 범위를 보존한다.
> 이후 수정 및 최종 판정은 [전체 검수표](./review-index.md)를 기준으로 읽는다.

2026-10-05. 검수자: module_client_review. 대상은 architecture-evolution의 review-index.md, usability-comparison.md, README.md, implementation.md의 설계 후보다. 제품 구현을 수정하거나 테스트를 실행하지 않았다. 네트워크·추가 에이전트를 사용하지 않았다. 이 파일만 작성했다.

## 범위와 원본

- Y = `/home/yon/projects/yo`; 제품 소스는 pin `880467b3186ac7ace0111acd37cb1aa334c9dc4c`. 문서는 root가 검토 중 수정하는 현재 working copy를 다시 읽었다.
- P = `/tmp/yo-pi-architecture-kqz3vqqm/pi-1.0.2`; upstream pin `cd32f7725fdbddbaecdff5b1e68491563394e0ca`의 Pi v1.0.2. 아래 P 상대경로는 `packages/coding-agent/` 기준이다.
- C = `/tmp/yo-codex-rust-v0.160.0-source/openai-codex-79b1b66`; upstream pin `a956835d020762cb2b570053af06f643a11c0ecc`의 Codex rust-v0.160.0. 아래 C 상대경로는 `codex-rs/tui/` 기준이다. 추출 디렉터리 이름을 pin으로 해석하지 않는다.
- Y의 AGENTS/CONTRIBUTING, 관련 model selection/frontend-independent/continuation 계약을 기준으로 검토했다. 이 보고서는 새로운 accepted contract가 아니다.
- Y 소스의 축약 경로: `command/`, `execution/`, `application/`, `state/config*`는 `crates/yo-cli/src/` 아래이고 `runner/`, `prompt/`, `shell/`은 `crates/yo-tui/src/` 아래다. R10의 `command/help.rs`는 명시적으로 `crates/yo-tui/src/command/help.rs`이다.
- 아래 “읽음”은 명시한 함수·구간·대표 테스트 본문을 읽었다는 뜻이다. 파일 전체나 모든 테스트를 읽었다는 뜻이 아니다. 이름만 검색한 테스트를 통과 근거로 사용하지 않았다.

## ID별 검수표

### R01 설치·첫 실행

| 구분 | 확인 |
|---|---|
| 실제 읽은 Y | 루트 `README.md` 첫 제품/플랫폼/preview 안내; `crates/yo-cli/src/lib.rs`의 Unix cfg; `.github/workflows/unix-compile.yml` 전체의 Linux/macOS check/clippy 작업. `.github`/`tools`의 release/install/package 파일명 검색도 수행했다. |
| 실제 읽은 P/C | P `README.md:24–58`: installer/npm/Node 전제→project cwd→pi→/login→task. C 루트 `README.md:12–78`: installer/npm/brew/binary→codex→auth. 양쪽 모두 문서에 진입 경로가 먼저 나온다. installer 프로그램 본문·release 생성 테스트는 미검토다. |
| 현재/제안 정확성 | usability §2의 “검증된 artifact 전에 설치 명령을 발명하지 않음”과 implementation 설치/운영 matrix는 적절하다. Yo README의 긴 기능·preview 소개는 실제 배포부터 첫 요청까지의 검증된 quickstart를 대신하지 않는다. 검색 결과만으로 Yo 전체에 installer가 없다고 단정하지 않았다. |
| 여정/소유자 | release/CLI owner가 지원 artifact·OS·실행 파일 경로를 확정하고, U0 setup과 연결해야 한다. 사용자가 개발용 preview를 제품 실행으로 오인하지 않아야 한다. |
| acceptance/한계 | clean state에서 실제 배포 artifact 설치→버전→yo→setup→첫 입력→종료 후 재실행. checksum/권한/PATH 실패에서 가능한 조치. 문서 요구는 존재하지만 실행 evidence 없음. R33 담당에게 release implementation 미검토를 이관한다. |
| 판정 | **부분 검토**: 첫 실행 안내/지원 범위는 확인; 실제 installer 내부와 package smoke 미검토/미실행. 문서의 현황 과장 없음. |

### R02 시작 화면·설정

| 구분 | 확인 |
|---|---|
| 실제 읽은 Y | `command/connect.rs` required target/from ArgGroup; `execution/model/startup.rs` no-target failure; `execution/model/startup/tests.rs::new_session_requires_a_target_and_accepts_host_unique_or_complete_references`; `application/runtime/live.rs:run_live_session`에서 config load가 generation보다 먼저 실패; `state/config.rs`, `state/config/parse.rs`의 strict decoding; `state/config/tests.rs::invalid_theme_has_actionable_configuration_error`, empty/missing 설정 관련 본문을 대조. |
| 실제 읽은 P/C | P `src/modes/interactive/interactive-mode.ts::handleLoginCommand`, `showLoginAuthTypeSelector`, selector 취소/복귀; C `src/onboarding/auth.rs` 상태와 browser/device login 취소 테스트. 외부 config 손상에 대한 Pi/Codex 전체 복구 경로는 미검토다. |
| 현재/제안 정확성 | 현재 “target 없음→yo connect 안내→인자 없음 parser 거절”은 실제 불일치다. Session 이전 setup·value-less connect는 새 구현 대상이며 이미 된 것으로 쓰지 않는다. `live.rs`는 config parse 실패 시 setup에 도달하지 못하므로 복구 UI가 실패한 Config에 의존하면 안 된다. |
| 여정/소유자 | CLI entry가 config diagnostic/setup readiness를 먼저 만들고 TUI setup은 Session 없이 소비. selected target 실패는 다른 모델/default로 숨기지 않는다. TTY/noninteractive를 entry에서 나눈다. |
| acceptance/한계 | 빈/기존 default/선택 target 실패/손상 config/pipe 각각 진입과 취소. 손상 config는 파일·필드·원인만 표시하고 secret 없이 편집→다시 읽기. root가 최신 usability §2/§7에 이 요구를 반영한 것을 재확인했다. |
| 판정 | **검토, 선행 finding 반영 확인**. 전체 upstream config recovery parity를 주장하지 않음. 실제 TTY wizard 미구현/미실행. |

### R03 인증·계정

| 구분 | 확인 |
|---|---|
| 실제 읽은 Y | `command/connect/local.rs`: delegated host 자체 login, credential flag 거절, verify 뒤 commit, 첫 default 규칙; `command/connect/external/tests.rs::cancelled_command_stops_before_secret_or_repository_mutation` 본문; `execution/model/startup.rs` exact account/target. |
| 실제 읽은 P/C | P interactive-mode의 `startProviderLogin`, auth-type/provider selectors, onBack 처리; `test/oauth-selector.test.ts` provider-owned options projection 및 configured 상태 표시 테스트. C `src/onboarding/auth.rs` cancel browser/device login reset 테스트와 상태 enum. |
| 현재/제안 정확성 | Pi/Codex의 화면 내 인증 경험을 기준으로 삼되 Yo가 delegated host에 없는 OAuth/device flow를 발명하지 않는 현재 제안이 맞다. 등록 성공≠요청 성공, 첫 default≠이후 계정 추가도 분리되어 있다. |
| 여정/소유자 | 실제 인증은 provider/host owner; setup은 지원 방식과 installed/login/version 상태, Back/Refresh를 표현. account identity는 읽기 쉬운 구분과 내부 exact binding을 함께 보존한다. |
| acceptance/한계 | login 취소→같은 단계, 잘못된 key→수정 가능, 기존 default 불변, key 미노출. SSH browserless에서는 실제 host가 지원하는 방법만 제시. 실제 계정/인증 만료/SSH 재연결은 미실행이며 R29와 공통 여정 필요. |
| 판정 | **대표 경로 검토**. 제안과 현재 구현의 구분 정확. 인증 제공 범위 전체와 외부 host browser behavior는 미검토다. |

### R04 모델 선택·변경

| 구분 | 확인 |
|---|---|
| 실제 읽은 Y | `command/model.rs` Enable/Disable 문법; `runner/model.rs` selection controller/panel; `runner/tests/model_selection.rs` direct model account resolution, failed_model_replacement_preserves_the_previous_controller, active_turn_model_selection_is_reserved_until_durable_completion(현재 exact steer가 기존 Turn에 남는 assertion 포함); `runner/state/image/tests.rs` rejected model에서 image 보존. |
| 실제 읽은 P/C | P interactive-mode `showModelSelector`/selectModel(persist)/current-default; `test/model-selector.test.ts` 현재 표시·save binding 테스트. C `src/chatwidget/model_popups.rs::open_model_popup` cached rows→request-id fetch, startup guard; `src/app/tests/model_defaults_tests.rs` session 선택이 config default를 바꾸지 않는 테스트. |
| 현재/제안 정확성 | `/model`과 busy reservation은 이미 있다. CLI 공통 picker는 새 제안이다. 현재/다음 Turn/default 세 의미를 한 “선택 모델”로 합치면 회귀한다. Codex async request-id refresh 원리는 유용하나 Yo exact target/capability를 그 문자열 model ID로 축소하면 안 된다. |
| 여정/소유자 | model service가 admitted inventory/exact candidate와 capability를 소유; frontend는 current/reserved/default와 실패 이유 표시. 기존 Turn은 예약 선택으로 교체되지 않는다. |
| acceptance/한계 | 작업 중 이미지와 draft를 가진 채 unsupported 후보 선택→이유 표시→기존 current/reserved/draft/bytes 보존, 다음 durable 종료 뒤 성공 적용. latest usability §2/§7에 해당 과제가 추가됨을 확인. stale async catalog가 닫힌 picker/새 선택을 덮지 않는 검증은 implementation의 stale catalog 항목에 연결 필요. |
| 판정 | **검토, 선행 finding 반영 확인**. 모델 capability 전체 조합과 실제 provider rebind는 미실행. |

### R05 프로젝트·실행 위치

| 구분 | 확인 |
|---|---|
| 실제 읽은 Y | `application/runtime/frontend/tests/workspace_label.rs` home-relative와 외부 absolute path; `application/runtime/frontend.rs` presentation root/session host·workspace 표시; `application/runtime/live/presentation.rs` exact host/workspace 필터; `prompt/workspace_reference/tests.rs` 선택 token과 typed identity 유지. |
| 실제 읽은 P/C | P `test/session-cwd.test.ts`: nonexistent saved cwd controlled error와 명시적 cwd override. C `src/workspace_command.rs`: workspace-bound app-server argv/cwd/env, timeout/output limit; resume picker의 SessionTarget cwd 및 page_loading_tests의 listing-cycle cwd filter. |
| 현재/제안 정확성 | 제안의 “다른 workspace 선택 시 실제 위치 표시, 저장된 workspace에서만 재개”는 Yo 계약에 맞다. Pi의 fallback cwd override를 그대로 가져오면 Yo exact resume 의미가 바뀐다. GUI client-local 경로를 host workspace로 보지 않는 implementation 요구도 적절하다. |
| 여정/소유자 | catalog/host workspace가 semantic path identity 소유; TUI는 basename 외 host/실제 위치를 식별 가능하게 표시. project 전환을 현재 세션의 cwd 변경으로 구현하지 않는다. |
| acceptance/한계 | **추가 권고 A5**: 같은 basename인 두 프로젝트, 삭제된 saved cwd, 읽기 권한 없는 workspace에서 선택→오류→기록만 열기/다른 대화로 복귀. 현재 §5는 일반 복구 행동은 있지만 이 조합을 명시하지 않는다. 실제 remote filesystem 테스트는 미검토. |
| 판정 | **대표 경로 검토**. 현재 location은 유지해야 하며 새 capability로 묘사하지 말 것. |

### R06 입력·편집·참조

| 구분 | 확인 |
|---|---|
| 실제 읽은 Y | `prompt/workspace_reference/tests.rs` Unicode/full-token/typed identity; `prompt/skill_reference/tests.rs::same_name_skill_sources_stay_distinct_in_narrow_panel_and_selection` 설정/선택 경로; `runner/tests/prompt_templates.rs` literal insertion→별도 Enter, active Turn steer, rejection retry; `runner/state/image/tests.rs` prepare/submit 분리·late revision·rejection 보존; `runner/state/external_editor/tests.rs` Ctrl+G request와 import/undo/no-submit. |
| 실제 읽은 P/C | P `test/external-editor.test.ts`: private temp, 실패 시 원문, empty 결과; interactive editor 흐름. C `src/bottom_pane/chat_composer.rs` editor responsibility 설명과 `non_ascii_burst_buffers_enter_and_flushes_multiline` 본문: enhanced keyboard가 없어도 Unicode paste burst Enter가 즉시 제출되지 않음. |
| 현재/제안 정확성 | Yo는 template/skill/image/editor 기능을 이미 가진다. Pi 입력 복원 문자열 결합을 Yo typed references/attachments에 복사하지 않는 문서가 맞다. 준비는 전송이 아니고 literal template가 slash command로 실행되면 안 된다. |
| 여정/소유자 | frontend는 draft/selection/editor revision, host는 admitted reference/image identity. input 준비/편집/acceptance cleanup을 분리한다. UI가 표시명으로 identity를 복구하지 않는다. |
| acceptance/한계 | 통합 task(파일+skill+template+image→external editor→취소/모델 변경→제출)가 최신 §7에 추가됨을 확인. 개별 existing unit tests만으로 이 교차 여정 통과를 주장하지 않는다. Pi/Codex의 모든 skill/template/image 대응은 미검토이며 입력 보존·paste/editor 원칙만 비교했다. |
| 판정 | **검토, 선행 finding 반영 확인**. IME 자체/clipboard 구현 전체/실제 editor 실행은 미검증. |

### R07 steer·queue·interrupt

| 구분 | 확인 |
|---|---|
| 실제 읽은 Y | `runner/state/input.rs` active Enter exact Steer, Alt+Q queue, acceptance까지 draft, paused queue recall; `shell/chrome/help.rs` active hints와 queue 우선 분기; `runner/tests/interrupt.rs` Esc/Ctrl+C 및 backpressure urgent lane; `runner/tests/session_lifecycle.rs::interrupted_follow_ups_pause_and_recall_without_overwriting_draft`. |
| 실제 읽은 P/C | P interactive-mode pending messages 표시/restore 및 `core/keybindings.ts` followUp/dequeue. C composer Enter/Tab 의미와 `src/bottom_pane/pending_input_preview.rs` queued/pending/rejected preview. Pi queue restore는 한 editor 문자열 결합이므로 Yo identity-preserving queue manager 설계와 구별했다. |
| 현재/제안 정확성 | active Enter는 이미 steer하지만 footer가 이를 설명하지 못한다. queue도 이미 있고 단순 count·recall 마찰이 문제다. 문서는 새 queue/steer를 발명하지 않고 U0b 표시와 U1 관리로 분리했다. known unsupported 상태의 Queue는 명시 선택이지 silent fallback이 아니다. |
| 여정/소유자 | core admission이 exact Turn/acceptance, frontend는 pending 문구·editor 보존·queued item 편집을 소유. current draft와 recalled queue를 별도 보존해야 한다. |
| acceptance/한계 | Enter 현재 작업/Alt+Q 다음 작업/중단 후 paused 표시; 전달 중/accepted/rejected; queue 2개 중 1개 수정해 현재 draft/첨부 손실0, 자동 재개0. 최신 §3/§7이 명시한다. consumer backpressure/unknown 상태는 R14/R29의 semantic client 검증과 연결. |
| 판정 | **대표 경로 검토; 새로운 material finding 없음**. 실제 키 입력 속도·인지성은 baseline task를 실행해야 한다. |

### R08 승인·질문·비밀 입력

| 구분 | 확인 |
|---|---|
| 실제 읽은 Y | `runner/state/requests.rs` exact pending request/presentation guard/decline 없는 interrupt; `runner/tests/request_responses.rs`: 입력 전 presentation guard, rejected_secret_response_reopens_empty_editor_for_the_same_request(288), secret_previous_question_navigation_discards_value_and_uses_empty_draft(354), question_notes_keep_selection_and_literal_text_after_narrow_reflow(1100), approval_without_decline_defaults_to_interrupt_and_blocks_unseen_typed_choices(1406). |
| 실제 읽은 P/C | P `core/extensions/types.ts` select/confirm/input/custom UI와 interactive/RPC 제공 범위. Pi의 Yo와 동등한 typed secret/Previous 상태기계·tests는 미검토이므로 parity/부재를 주장하지 않는다. C `bottom_pane/approval_overlay/clipping_tests.rs` 40x7까지 choices 유지, configured Ctrl+G 전체 command 이동, 취소 보존; request_user_input module 및 question/notes snapshot. |
| 현재/제안 정확성 | offered choices와 실제 Esc 결과의 일치, 긴 내용 상세, 기존 notes/Previous 보호를 강화하는 방향은 정확. `Esc decline` 무조건 표시를 계속 쓰면 실제 interrupt와 불일치한다. |
| 여정/소유자 | request owner가 offered profile와 exact identity, frontend가 보여 준 choice와 local 일반 draft를 소유. unsupported는 verified transfer 또는 explicit cancel이며 plain fallback 금지. |
| acceptance/한계 | **A8 문구 범위 확인 필요**: §7의 “답변/notes 손실0”은 일반 질문에 한정해야 한다. secret은 거절/Previous에서 고의로 비워 재입력하는 기존 테스트가 있다. 일반 draft 보존 요구를 secret retention으로 확대하지 말 것. root에 전달했다. long payload→상세→복귀→선택/notes, late/stale request, unsupported profile은 existing matrix에 있다. |
| 판정 | **대표 경로 검토; A8 clarification 권고**. secret 보호를 회귀시키는 보존 약속 방지. upstream secret parity와 실제 transfer UI는 미검토/미구현. |

### R09 진행·대기·상태

| 구분 | 확인 |
|---|---|
| 실제 읽은 Y | `runner/tests/status.rs` /status가 backend dispatch 없이 Running/usage 부재 표시, pending compact의 Compacting 표시; `runner/state/presentation.rs` frame assembly; input/requests의 active/request state. |
| 실제 읽은 P/C | P `components/status-indicator.ts` working/retry countdown/compaction reason, `test/status-indicator.test.ts:66–118` 다양한 폭과 retry timer disposal 테스트. C `src/status_indicator_widget/timer_tests.rs` pause/wait timer, effects_tests의 reduced display/toggle snapshot. |
| 현재/제안 정확성 | 실제 Activity에서만 count를 만든다는 제안은 맞다. “작업 중/승인 대기”만으로는 Pi의 retry/compaction과 Codex의 wait-time 구분 수준의 이해 가능한 상태가 충족되지 않는다. 기존 /status/compaction 표시를 새 기능이라고 쓰면 안 된다. |
| 여정/소유자 | semantic projections가 원인, TUI가 우선순위와 설명/상세 행동을 소유. 모델 텍스트에서 가짜 %를 추론하거나 client가 별도 lifecycle authority를 만들지 않는다. |
| acceptance/한계 | **추가 권고 A9**: 같은 fixture에서 running tool, approval waiting, host retry, compaction, cancel requested→terminal을 바꾸고 사용자가 지금 기다릴 대상/가능 행동을 구별하는 task. 지원되지 않는 retry 세부는 Unknown/일반 대기이지 가짜 countdown이 아니다. foreground/background 전환 뒤 중복 진행/타이머 누적도 확인. |
| 판정 | **부분 심화 검토**: 대표 표시/test는 확인; 모든 status aggregation/host retry projection 구현은 미검토. 문서가 실제 지원 상태를 과장하지 않도록 capability 조건 필요. |

### R10 출력·변경·검증 결과

| 구분 | 확인 |
|---|---|
| 실제 읽은 Y | `runner/view/changes.rs` Changes/Proposed/Recorded sections; `runner/view/output.rs` retained/partial literal output; `runner/tests/views/navigation.rs` view-local scroll과 70k-line retained paging/CJK resize 테스트; `command/help.rs` existing Changes/Alt+D/output 안내. |
| 실제 읽은 P/C | P `components/tool-execution.ts` collapsed/fallback rendering, `test/tool-execution-component.test.ts` stale partial image 교체와 custom call/result 분리 테스트. C `src/diff_model.rs::FileChange`, `src/diff_render.rs::create_diff_summary_with_links/create_diff_preview_with_links`와 `large_update_diff_skips_highlighting` 본문. |
| 현재/제안 정확성 | Changes와 Output은 이미 있고 보존되어야 한다. P0 summary에서 기존 detail에 진입, P1 완료요약에 session changes 연결은 정확하다. saved evidence/current file/Git/명령 exit0/검증을 분리한 §4도 맞다. |
| 여정/소유자 | semantic evidence는 core/host, TUI는 preview/detail/scroll. 새 current Git diff는 명시적 bounded read-only 조회이며 saved change attribution을 덮지 않는다. |
| acceptance/한계 | 10k 로그 실패→한 행동으로 해당 retained output→원 scroll/draft 복귀, 70k fixture 기존 bounded behavior 유지. “전체 출력”은 실제 retained 한도 내에서만. 모델 완료 주장과 exit0을 test pass로 오인하지 않음. docs에서 충분히 명시됨. |
| 판정 | **대표 경로 검토; 새 material finding 없음**. 실제 command→saved evidence producer 전체는 R20/R28에 이관; 성능 수치는 미측정. |

### R11 대화 탐색·재개·분기

| 구분 | 확인 |
|---|---|
| 실제 읽은 Y | `application/runtime/live/presentation.rs` host/workspace filter/64 cap/summary 없는 row 처리; `runner/session/continuation.rs` UUID·updated 중심 picker/더 많은 CLI 안내 및 fork entry; `application/runtime/live/resume.rs` prepare 실패 전 current 유지; `command/session/presentation/tests.rs` compact UUID columns, all/details, empty/pipe/narrow; `application/runtime/startup/failure.rs` read-only resume fallback. |
| 실제 읽은 P/C | P `session-selector-search.ts` name/id/allMessages/cwd와 정규식 조건, `test/session-selector-search.test.ts` phrase normalization/regex/recent. C `resume_picker.rs` page25/threshold5, `resume_picker_transcript_preview.rs` bounded local preview/server items, `page_loading_tests.rs` cwd filter는 한 cursor cycle 동안 고정. |
| 현재/제안 정확성 | `yo resume`는 새 grammar이며 --resume/--continue/기존 read-only CLI는 유지된다. “현재 Session이 있어야 /resume”은 slash UI 범위에 한정; 기록 접근 전체가 없다고 말하면 틀리다. 문서는 기존 CLI를 보존한다. Pi 전체 message 검색 방식을 Yo의 bounded catalog에 무제한 복사하면 안 된다. |
| 여정/소유자 | host catalog가 safe metadata/paging/cwd eligibility, TUI는 query/selection/preview. 제목은 safe ordinary text, secret/private payload 제외. fork는 기존 eligibility/lineage를 쉽게 보여 주는 작업이다. |
| acceptance/한계 | 검색→65번째 결과→preview→resume task는 §7에 있다. **A5 추가**: 이전 query의 늦은 응답/preview가 현재 선택을 덮지 않음; missing workspace/corrupt/read-only row에서도 기록/복귀가 가능. upstream fork 전체 구현·Yo 모든 fork eligibility 테스트 본문은 미검토이며 R25와 교차검토 필요. |
| 판정 | **대표 resume 검토; fork 깊이 부분 검토**. 찾기 UI 개선은 정당하지만 durable fork correctness 완료 판정 아님. |

### R12 터미널 사용성·접근성

| 구분 | 확인 |
|---|---|
| 실제 읽은 Y | `crates/yo-cli/tests/terminal_matrix.rs`, `terminal_matrix/ssh.rs`: inline/fullscreen/SSH/tmux 테스트가 local sshd/compatible Codex 조건으로 ignored; `runner/tests/views/navigation.rs` 좁은 크기·scroll; `state/config/tests.rs` default/light/mono validation. |
| 실제 읽은 P/C | P `test/footer-width.test.ts` CJK name/model/provider width, `interactive-mode-suspend.test.ts` Unix SIGCONT 복원과 Windows unsupported. C `src/tui.rs::restore_common/restore_after_exit/reapply_raw_mode_after_resume`; approval clipping 및 non-ASCII paste/no enhanced-key 테스트. |
| 현재/제안 정확성 | §7의 80x24/120x36/40x12/CJK/SSH/tmux는 테스트 계획이며 통과 주장 아님. 현재 source tests가 존재한다고 실제 SSH 시각/termios 검증을 완료한 것은 아니다. DOM accessibility는 별도 R30이며 TUI와 동일 근거로 주장할 수 없다. |
| 여정/소유자 | TUI가 focus/scroll/keyboard 표현, process shell이 raw mode/alternate screen/suspend restoration을 소유. request/summary detail에서 복귀 시 current draft 유지. |
| acceptance/한계 | **추가 권고 A12**: enhanced key 없는 terminal에서 Alt/F-key가 전달되지 않는 조건의 실제 지원 대체 진입, mono에서 색 외 의미 구분, external editor/suspend/failure/exit 후 cursor·termios 복구. 대체키를 추측해 문서화하지 말고 현재 command와 연결하거나 지원 경계를 표시. screenreader 실제 usability는 미검토. |
| 판정 | **대표 정적 검토; terminal 실환경 미검증**. ignored tests는 명시적으로 실행하고 환경/evidence를 기록해야 완료 가능. |

## 발견과 반영 상태

| finding | 중요도 / 현재 상태 | 최소 반영 |
|---|---|---|
| A2 config 오류가 setup 이전 발생 | P0 여정 / 최신 문서 반영 확인 | config-free preflight diagnostic과 edit/reload/exit, Session 호출 없음. |
| A4 current/reserved/default 및 첨부 capability 조합 | P0 회귀 방지 / 최신 문서 반영 확인 | busy switch 실패에서 이전 모델·typed inputs 유지, 현재/다음 Turn 표시. |
| A6 입력 교차 여정 부재 | P0/U1 수용 조건 / 최신 §7 반영 확인 | 기존 파일·skill·template·image·editor 기능을 합친 task. |
| A8 일반 draft와 secret 보존 요구 구분 | P0 문구 정확성 / root에 권고 전달 | 일반 답변/notes 보존에 한정하고 secret 재입력/폐기 규칙은 별도. |
| A9 상태 원인 구별 과제 부족 | P0 수용 조건 / 추가 권고 | 실제 제공된 running/approval/retry/compaction/cancel 상태에서 가능한 행동 식별. |
| A5 cwd 소실·stale search 교차 복구 | P0/U1 수용 조건 / 추가 권고 | 같은 basename, missing cwd, 늦은 query/preview, read-only/corrupt row. exact workspace 유지. |
| A12 terminal modifier/mono/복원 | P0/U1 수용 조건 / 추가 권고 | 전달 가능한 키/명령 대체와 실제 shell 복원. ignored test와 수행 evidence 구분. |

A5/A9/A12는 새 대규모 기능 요구가 아니라 이미 목차에 있는 실패·복구 조건을 사용자 과제로 명확히 하는 작은 보강이다. 현재 문서의 일반 matrix에 관련 제약이 있으므로 중복 구현 owner를 만들 필요가 없다.

## 누락 감사자에게 명시적으로 넘기는 한계

- R01 실제 installer·package build/upgrade internals와 테스트 본문은 미검토. R33의 artifact review와 결합해야 한다.
- R03 실제 provider auth/expiry/browser/device/SSH lifecycle 전체는 미검토. 화면 semantics와 representative cancellation만 확인했다.
- R06 Pi/Codex의 모든 skill/template/image 구현은 미검토. 대응 없는 기능이라고 단정하지 않았다.
- R08 Pi typed approval/secret/Previous equivalence는 미검토. Yo와 Codex의 모든 request/profile 조합을 같은 것으로 취급하지 않는다.
- R09 모든 host의 retry/foreground/background status producer는 미검토. 현재 제공 데이터만 표시하는 projection 계약이 전제다.
- R11 fork 전체 lineage/durability tests는 미검토. 대표 UI entry와 기존 eligibility 보존 방침만 확인했다.
- R12 실제 terminal emulator/assistive technology, Linux/macOS/SSH/tmux matrix는 실행하지 않았다.
- 모든 R항목에서 런타임 테스트·빌드·모델 요청·성능 측정을 실행하지 않았다. source test 본문 확인과 test pass는 구별한다.
- B/C와 겹치는 model/catalog/draft/client semantics는 service/core가 소유하고 UI가 별도 mutable truth를 만들지 않는 것으로 통합해야 한다. 특히 disconnect unknown, request final seal, exact workspace, artifact identity는 A의 화면 편의를 위해 약화하면 안 된다.

최종 판단: R01–R12 모두 확인 범위를 배정하고 대표 원본에 근거한 판단을 남겼다. “미검토 없음” 또는 “사용성 검증 완료”로 요약하면 안 된다. 현재 문서는 기존 queue/Changes/Output을 보존하면서 P0 여정을 앞세우는 방향으로 정확해졌고, 남은 작은 수용 조건과 위 한계를 최종 감사에 전달한다.
