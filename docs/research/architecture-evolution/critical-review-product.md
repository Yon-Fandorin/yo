# Yo 사용자 여정 반대심문: R01–R12 및 T01–T64

> Status: non-authoritative research/design audit

2026-10-05. 문서 전용 독립 제품 감사. 이 담당자는 제품/테스트/계약/외부 트리를 수정하지 않았고 빌드·네트워크·하위 에이전트를 사용하지 않았다. 소스와 명시한 테스트 본문을 읽었으며 테스트를 실행한 것으로 쓰지 않는다. 작성 파일은 이 파일뿐이다.

Yo 현재 checkout, Pi `/home/yon/projects/pi`의 지정 pin `cd32f7725fdbddbaecdff5b1e68491563394e0ca` (v1.0.2), Codex `/tmp/yo-codex-rust-v0.160.0-source/openai-codex-79b1b66`의 지정 pin `a956835d020762cb2b570053af06f643a11c0ecc` (rust-v0.160.0)을 비교했다. 최신성/시장 전체 우위 주장이 아니다. 아래 Y/P/C는 각각 위 소스 루트의 상대경로다.

읽은 authority: [Y: AGENTS.md](../../../AGENTS.md), `CONTRIBUTING.md`, 제품 `README.md`, `docs/src/README.md`, `methexis/active-checkpoint.yaml`, `methexis/knowledge/agent-runtime/agent.model.session-selection.md`. 기존 연구 `review-index.md`, `review-a-user-journeys.md`, `usability-comparison.md`, `tui-review.md`의 전체 목록과 기존 input/content/shell 상세 보고의 관련 판단을 재검토했다. 모든 64 요소의 모든 분기를 다시 읽거나 실행했다는 의미가 아니다.

## 제품 판정

현재 Yo를 주 사용 코드 에이전트로 추천할 근거는 부족하다. 처음 쓰는 사용자는 연결에 막히고, 익숙해진 사용자는 주제 전환·실패 증거 탐색·현재 작업 결과 검증·외부로 결과 가져가기에서 계속 도구 밖으로 나가야 한다. 이 결론은 모델 품질이나 실제 장애 빈도 평가가 아니라 확인한 인터페이스와 상태 전이에서 나온다. 고급 renderer와 엄격한 저장/identity 설계는 제품 가치의 재료이지만 일의 시작부터 회수까지 이어지는 경험을 대신하지 않는다.

기존 감사의 개별 사실은 대체로 신중하다. 그러나 다음 세 판단 방식은 수정해야 한다.

1. “이미 있다”는 새로 만들지 말아야 한다는 근거일 뿐, 사용자 과제가 해결됐다는 근거가 아니다. Output에 원문이 있어도 실패 항목을 찾을 수 없으면 실패 진단이 어렵다.
2. “별도 계약/별도 결정”은 구현 경계를 정한다. 제품 우선순위를 P2로 낮추는 이유가 되지 않는다. Tool 검색, 설정 UI, export, 현재 Git 조회가 여기에 해당한다.
3. 17 command/5 view/15 module을 64개 목록에 모두 연결한 것은 구현된 표면의 coverage다. 존재하지 않는 사용자 행동을 검출할 수 없다. 이름 붙이기, queue 비우기, 설정 변경, 외부 회수 같은 공백은 그 목록 밖에서도 찾아야 한다.

## R01–R12 coverage와 재판정

상태: `없음`은 확인한 public surface 부재, `있지만 불편`은 기능이 있으나 과제 완료 비용이 큼, `도달 불가`는 필요한 상태에서 진입이 막힘, `검증 부족`은 실패를 입증하지 않은 영역이다.

| ID | 재판정 / 대표 실제 근거 | 사용자 결과 / 연결 finding |
|---|---|---|
| R01 설치·첫 실행 | 검증 부족. README는 제품 실행보다 수많은 preview/기능 설명에 무게가 있고 Developer Docs는 설치/사용자 가이드가 아니라고 명시한다. `.github/workflows/unix-compile.yml`은 compile/check 경계. 배포물 실설치 smoke는 이 감사에서 확인하지 못함 | 설치부터 첫 성공 과제가 증명되지 않음. 단순 README 재배치만으로 종료하지 않음. F1/F9 |
| R02 시작·설정 | 도달 불가/없음. `execution/model/startup.rs`의 no-target 오류, `command/connect.rs` required target/from, `command/model.rs`의 enable/disable. accepted model-selection은 Session 전 setup/인자 없는 connect/통합 picker를 요구 | 빈 상태에서 onboarding 자체가 없으며 계약 구현 격차. F1 |
| R03 인증·계정 | 있지만 불편/검증 부족. `command/connect.rs`→local/external 분리, `state/config/parse.rs` 엄격한 config admission. Pi `handleLoginCommand` 및 settings flow 대비 화면 안 Back/repair 여정 부족; 실제 만료·SSH auth는 미실행 | provider/host 인증 지식이 제품 진입 비용으로 넘어옴. setup만 띄워도 auth 복구 완료는 아님. F1/F7 |
| R04 모델 선택 | 있지만 불편. `runner/model.rs::panel`은 account section/disabled/current를 보존하지만 검색·user preference 변경 표면이 빈약. CLI picker와 runtime picker 계약도 다름 | 모델이 있다는 것과 사용자가 맞는 연결·capability를 고른다는 것은 다름. F1/F7 |
| R05 프로젝트·실행 위치 | 있지만 불편/검증 부족. resume/tree는 현재 workspace 중심; tree `UUID·depth·anchor` 표시. README file link는 SSH 원격 파일 읽기를 제공하지 않음 | 같은 basename·원격 파일·다른 프로젝트를 실제 작업 대상으로 확인하는 여정 부족. F3/F5/F9 |
| R06 입력·편집·참조 | 강점+있지만 불편. `input.rs`의 rich input snapshot 유지, find/history 별도 draft는 보존. 기존 T04 image undo/T09 template 이름 재입력/T10 마지막 thumbnail 한계는 남음 | grapheme/typed refs가 좋아도 복합 입력 작성·재사용 과제를 자동 통과하지 않음. F7, 기존 W1/W3/W6 별도 유지 |
| R07 steer·queue·interrupt | 있지만 불편/도달 불가. `input.rs::recall_follow_up`, `commands.rs::session_transition_unavailable_reason`, lifecycle 테스트 실제 assert | 중단 후 paused queue가 새 주제 전환까지 막음. 단순 hint 개선보다 큼. F2 |
| R08 승인·질문·비밀 | 알려진 P0 미해결+검증 부족. frame-bound acceptance/secret 별도 보호는 강점. T19/T20/no-decline 문제는 기존 보고를 미해결 제품 결함으로 승계; 이번 새 재현 아님 | 기존 T19/T20을 다시 발견한 것처럼 포장하지 않음. 긴 승인 검토→정확한 요청 복귀도 실제 terminal 과제 필요. F4/F9 |
| R09 진행·대기 | 있지만 불편/검증 부족. `/status`의 Running/Starting/Waiting/Compacting과 최신 usage 관측은 있음. queue/unsupported steer rejection이 실제 상태를 뒤늦게 설명 | 무엇을 기다리는지·어떤 개입이 가능한지 모델별 일관성 부족. F2/F7; 실제 wait/latency는 미측정 |
| R10 결과·변경·검증 | 있지만 불편+없음. `Output`은 마지막 항목 진입, `find::is_searchable`은 tool 제외, Changes는 저장 evidence. registry에는 현재 Git 조회/사용자 직접 실행 없음 | 코딩 결과를 사용자가 스스로 검증하는 고리가 끊김. F4/F5 |
| R11 재개·주제·분기 | 있지만 불편/도달 불가. `show_resume_picker`는 UUID/time, `show_session_tree`는 UUID/depth/anchor, `show_fork_picker`만 input excerpt. busy/queued 전이 gate | 정확한 lineage는 보존하지만 기억한 주제로 선택/이름 붙이기/오래된 대화 찾기는 부족. F2/F3 |
| R12 터미널·접근성 | 검증 부족+일부 도달 불가. local tmux/SSH tests는 ignored 환경 전제. config는 theme/표시 옵션을 갖지만 public settings·reduced-motion 설정 경로가 빈약 | 주 대상 환경 검증을 renderer unit pass로 대체할 수 없음. F7/F9 |

## 제품 blocker 9개

### F1 · P0: 새 사용자가 제품에 들어오는 경로가 막혀 있다

- 분류: **도달 불가 + 없음**, R01–R04/T35/T38/T39/T64.
- 지금: `yo`→target 없음→`yo connect` 안내→필수 TARGET/--from 오류. 모델 이름을 모르는 사용자가 `yo model`을 시도해도 enable/disable만 나온다. 기술 문서로 canonical coordinate와 host 설치/인증을 익힌 뒤 다시 시작해야 한다.
- 목표: 실행→연결 유형/기존 계정 선택→실제 인증 상태 확인/복구→사용 가능 모델 선택→첫 요청. 취소 후 같은 단계. 연결하지 않아도 이전 대화 읽기 가능.
- 근거: [Y: crates/yo-cli/src/execution/model/startup.rs::resolve_new_session_with_tool_restriction](../../../crates/yo-cli/src/execution/model/startup.rs), `command/connect.rs::Arguments`의 required ArgGroup, `command/model.rs`; accepted `agent.model.session-selection`의 setup 요구. [P: packages/coding-agent/src/modes/interactive/interactive-mode.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/interactive-mode.ts) login/model/settings 경로. [C: codex-rs/tui/src/onboarding/auth.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/onboarding/auth.rs).
- 루트가 별도로 실행했다고 전달한 증거: `/tmp/yo-challenge-gpdwrw7_/cli-probes.json`, 격리된 빈 설정으로 `yo` exit1, `yo connect` exit2, `yo model` exit2. 이 담당자가 재실행하거나 실제 TUI/auth 성공을 검증한 것은 아니다.
- 최소 수정: Session 전 startup/onboarding controller, value-less connect/model picker, 잘못된 설정 진단 진입. accepted contract와 맞춘다.
- 구조 교체 대안: CLI의 “항상 먼저 Session/backend를 만든다”는 진입 구조를 `WorkspaceLanding → Setup/History → ActiveSession` state machine으로 교체. 기존 Session/core는 유지한다. 화면만 붙여 config/backend 실패에 다시 막히면 이쪽이 필요하다.
- Owner: CLI application/startup + connection/model service; TUI는 flow 소비자.
- 수락: clean macOS/Linux 배포물에서 YAML/target 좌표 지식 없이 첫 요청 준비; 잘못된 key·미설치 host·손상 config·취소 후 복구. pipe 입력은 wizard가 아니라 정확한 오류. install/로그인 실제 smoke는 별도 실행해야 한다.

### F2 · P1: 중단한 뒤 다른 일로 옮기는 것이 예약 입력 때문에 막힌다

- 분류: **있지만 불편 + 도달 불가**, R07/R11/T12–T14/T36/T37.
- 지금: 작업 A 중 후속 지시 2개 예약→중단→새 주제 B 초안 작성→`/new`/`/resume`는 queue가 있다는 이유로 불가. `Alt+R`는 초안이 비어야 head 하나를 꺼낸다. B를 외부에 보관/지운 뒤 각 queued message를 반복 recall/삭제해야 전환할 수 있다. 이건 단축키 암기 문제를 넘어 작업 전환 제어가 없는 것이다.
- 근거: [Y: runner/state/commands.rs::session_transition_unavailable_reason](../../../crates/yo-tui/src/runner/state/commands.rs)은 paused 상태도 `!follow_ups.is_empty()`로 거절. `runner/state/input.rs::{recall_follow_up,queue_follow_up,next_follow_up}`은 head-only recall/empty-editor 조건. `runner/tests/session_lifecycle.rs::interrupted_follow_ups_pause_and_recall_without_overwriting_the_draft`는 new draft 보존 후 **Ctrl-U→Alt-R**를 정상으로 assert한다. `rejected_manual_steer_guides_queue_and_discard_without_claiming_finished_work_runs`도 한 항목 recall/clear→나머지 resume 절차를 assert한다.
- 비교: P `interactive-mode.ts::restoreQueuedMessagesToEditor`는 모든 pending을 꺼내 현재 draft와 합친다. 조작성은 빠르나 문자열 합치기는 Yo의 typed attachment identity 대체안이 아니다. [C: bottom_pane/pending_input_preview.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/pending_input_preview.rs)는 pending/queued 표현의 비교 기준; 이 감사는 C의 모든 queue 삭제·switch 조합을 검증하지 않았다.
- 목표: 중단→예약 목록에서 특정 항목 수정/삭제 또는 모두 취소→원래 draft를 유지한 채 주제 전환. paused 상태와 다음에 실행될 내용을 항상 확인한다.
- 최소 수정: typed queue manager와 `Discard all queued` action; draft를 비우지 않고 별도 queue editor에 들어가고 복귀. 명시적으로 지우거나 남겨 두는 선택 없이 다른 Session으로 넘기지 않는다.
- 구조 교체 대안: queue/draft를 runner의 일시적인 단일 editor에서 Session-owned `DraftWorkspace`로 분리. 여러 주제를 오갈 요구가 확정되면 parked draft/queue를 Session에 붙인다. durable queue 도입은 별도 수명 계약이 필요하며 무조건 채택하지 않는다.
- Owner: TUI draft/queue state + core admission. 수락: A에 3개 예약/이미지 포함/B 초안 존재→중단→2번만 삭제→모두 명시 취소→new/resume; B·다른 attachment 손실0, 미승인 자동 실행0. 기존 exact Turn/acceptance 보호 유지.

### F3 · P1: 대화 트리가 있어도 사용자가 기억하는 “그 작업”을 고를 수 없다

- 분류: **있지만 불편 + 이름 관리 없음**, R05/R11/T05/T35–T40.
- 지금: `/resume`에 Updated+UUID, `/tree`에 UUID+depth+parent anchor. 사용자는 “토큰 파서 버그”를 기억하지만 화면은 저장소 좌표를 요구한다. 64개 밖은 `yo session`으로 나가 UUID를 찾아야 한다. fork picker의 input excerpt는 강점이지만 tree 전체 주제 탐색을 대신하지 않는다.
- 근거: [Y: runner/session/continuation.rs::{show_resume_picker,show_session_tree,show_fork_picker}](../../../crates/yo-tui/src/runner/session/continuation.rs), `application/runtime/live/presentation.rs` catalog cap, `command/registry.rs`에 rename/name 없음. P `interactive-mode.ts::handleNameCommand`, `components/session-selector-search.ts`; `test/session-selector-search.test.ts`는 whitespace-normalized phrase 검색 결과를 실제 assert한다. [C: chatwidget/interaction.rs::show_rename_prompt](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/chatwidget/interaction.rs), resume picker/preview가 비교 경로다.
- 목표: 사용자가 이름을 붙이거나 안전한 첫 요청 발췌를 보고, 현재/다른 프로젝트 범위를 선택하며 내용 preview로 확신한 뒤 연다. 검색 결과 없음/조회 범위 제한/열 수 없음이 다르게 보인다.
- 최소 수정: user title+safe excerpt projection, rename, query/filter/paging/preview. UUID는 상세 identity로 유지. record corrupt/unavailable 항목은 read-only 열기 제공.
- 구조 교체 대안: 같은 generic SelectionPanel에 모든 정보를 밀어 넣는 방식에서 session catalog browser로 교체(검색/리스트/preview별 상태). tree/fork/resume가 공통 catalog identity를 공유하되 행동을 구별한다.
- Owner: session repository metadata/catalog + TUI session browser. 수락: 동일 basename 2프로젝트·100대화·같은 첫 문장 3개·65번째 결과·계정 불가 항목에서 제목/본문만으로 정확 선택; stale preview가 선택을 바꾸지 않음. secret/private payload는 제목/search에 미포함.

### F4 · P1: 도구 결과를 보존하지만 실패 근거를 다시 찾는 탐색 기능이 없다

- 분류: **있지만 불편 + tool 검색 없음**, R08–R10/T06/T22/T23/T31–T34/T49.
- 지금: 수십 도구 중간에서 실패한 명령·파일명을 `/find`로 검색해도 tool/notice는 corpus에 없다. `/output`을 열면 현재 focus와 무관하게 최신 retained 항목이다. 좌우로 항목을 넘기고 긴 출력은 계속 스크롤한다. “원문 보존”의 실제 이익이 탐색 비용에 묻힌다.
- 근거: [Y: runner/state/find.rs::{corpus,is_searchable}](../../../crates/yo-tui/src/runner/state/find.rs), `find/tests/mod.rs::corpus_includes_only_final_ordinary_user_and_assistant_messages`는 notice·구조화 activity·media를 제외하고 `[3,1]`만 기대한다. `runner/view.rs::open_output`→`Position::latest`; `view/output.rs`는 Left/Right/paging/source anchor이지만 query 상태가 없다. 이건 테스트 실패가 아니라 현 계약 범위가 사용자 과제보다 좁다는 진단이다.
- 비교: [C: transcript_view/search_tests.rs::selection_pauses_find_and_normalized_navigation_resumes_the_same_query](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/transcript_view/search_tests.rs)는 여러 HistoryCell의 source search→선택 중 pause→query 유지 재개를 assert한다(모든 tool payload 조합 증명은 아님). P alt-screen search/expanded tool output은 기존 상세 보고의 비교 근거이며 이 감사는 그 전체 search 구현을 재검증하지 않았다.
- 목표: “실패한 명령/바뀐 파일/이 문자열”로 찾아 해당 evidence에 바로 열고, 해당 원문 fragment를 복사하고, 원 대화/초안으로 복귀.
- 최소 수정: 실패 filter + focused `open_output_for(item)` + 해당 retained output의 bounded find/copy. 현재 Chat 검색 범위를 조용히 넓히지 말고 검색 모드를 표시한다.
- 구조 교체 대안: Chat/Output/Changes의 서로 다른 index 기반 탐색을 typed evidence index(item/source/revision/range)에 통합. 렌더러를 합치지 않고 navigation identity를 공유한다. standalone tool viewer를 계속 늘리면 이 대안의 비용이 낮아진다.
- Owner: transcript/evidence projection + runner view navigation. 수락: 100 tool/20k log/3 failure fixture에서 filename 또는 late error marker로 정확한 항목 열기·복사; truncation 밖은 not retained로 설명; resize/new activity에도 selected source 유지. 원문에 secret을 새로 색인하지 않는다.

### F5 · P1: “지금 무엇이 바뀌었고 맞는가”를 사용자가 독립적으로 확인하기 어렵다

- 분류: **현재 Git 조회 없음 / 사용자 직접 shell 없음 / evidence는 있음**, R05/R10/T22/T30/T31.
- 지금: Changes는 실행 당시 기록된 proposal/publication이다. 저장 뒤 다른 process가 파일을 수정해도 이는 최신 working tree가 아니다. 현재 diff/test를 보고 싶으면 모델에게 부탁하거나 다른 shell로 나가야 한다. renderer가 diff를 예쁘게 보여도 리뷰를 마무리할 작업 공간은 완성되지 않는다.
- 근거: [Y: runner/view/changes.rs](../../../crates/yo-tui/src/runner/view/changes.rs)의 retained sections, `command/help.rs`의 현재 workspace/Git가 아니라는 명시, registry/CLI grammar에 `/diff`·직접 shell action 없음. [C: chatwidget/slash_dispatch.rs::SlashCommand::Diff](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/chatwidget/slash_dispatch.rs)는 실제 workspace runner+cwd로 `get_git_diff` 실행 후 결과 event를 반환한다. P `interactive-mode.ts::handleBashCommand`는 사용자 command를 실행·표시·record하고 `excludeFromContext`를 별도로 보존한다. Pi에 동일한 dedicated Git review view가 있다고 주장하지 않는다.
- 목표: 현재 workspace diff와 saved edit evidence를 구분해서 열고, 검증 명령/종료/출력에 접근하며 “assistant가 됐다고 말했다”와 실제 관측을 구분.
- 최소 수정: 명시적인 bounded read-only current Git status/diff action을 기존 Changes 옆에 연결; 검증 완료 카드가 아니라 실행 evidence 목록. direct shell은 먼저 cwd/권한/문맥 포함 여부가 보이는 작은 수동 실행 경로로 검토.
- 구조 교체 대안: 대화 중심 완료 처리에서 `ReviewWorkspace`(파일 목록/current diff/saved evidence/check results)를 독립 사용면으로 교체. Git attribution을 모델 tool 이벤트에서 추정하지 않는다. Revert/commit 자동화는 이 finding의 필수 수락조건이 아니다.
- Owner: workspace command host + tool evidence + TUI review. 수락: 원래 dirty 파일·Yo 수정·외부 후속 수정·untracked가 섞인 fixture에서 두 evidence 출처를 정확히 구분; 모델 호출 없이 현재 diff 읽기; 명령 exit0을 “전체 테스트 통과”로 변환하지 않음.

### F6 · P0 저장 실패 회수 / P1 일반 공유: 중요한 결과를 앱 밖으로 온전히 가져갈 경로가 없다

- 분류: **TUI 전체 export 없음 / 일부 plain archive 있음 / session import 없음**, R10–R12/T28/T57/T64.
- 지금: 저장 실패 시 `New activity stays in memory. Copy important output before closing yo.`라고 안내한다. 하지만 `/copy`는 마지막 completed assistant text만 가져가고 tool·여러 답변·사용자 입력은 한 번에 회수하지 않는다. `yo session ID`는 durable reader라 아직 memory-only suffix 복구 수단이 아니다. terminal 수동 selection은 구조적 완전 회수를 보장하지 않는다.
- 근거: [Y: runner/state/observation.rs::observe_durability](../../../crates/yo-tui/src/runner/state/observation.rs), `runner/state/commands.rs::CopyAnswer`, `command/copy.rs`, CLI `command/session/show.rs::show_from_reader`, `command/session.rs`. `command/tests.rs::unsupported_output_requests_are_rejected_at_parse_time`는 `session ID --format json` 거절을 실제 assert한다. `yo-tui` HTML renderer/API가 있다는 것과 CLI 사용자 export 경로는 다르다. `yo connect --from`은 connection 정의 import이며 session import가 아니다.
- 비교: [C: chatwidget/transcript_export.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/chatwidget/transcript_export.rs)는 complete conversation clipboard/file picker, `chatwidget/tests/copy_export_picker_tests.rs::copy_export_picker_custom_keys_preserve_payloads_and_composer_draft`는 취소/파일명/초안 보존을 assert. `app/transcript_export_tests.rs`는 hidden review prompts/duplicates 제외를 assert. P `handleExportCommand`는 HTML/JSONL, `handleImportCommand`는 JSONL + 확인 + missing cwd 복구를 가진다. C `/import`는 Claude Code setup/project/chats migration이며 보편적 exact replay가 아니다.
- 목표: current in-memory conversation/evidence를 범위·누락·민감정보 정책과 함께 안전하게 저장/복사. 이동/import는 portable document와 executable resume를 구별. 이 실패 회수를 P2 export 장식으로 미루면 안 된다.
- 최소 수정: live source snapshot export(메시지+공개 tool evidence+durability cutoff+omission 설명), local 파일 실패와 clipboard 미확정 별도 표시. 일반 export부터 제공하고 exact session migration은 별도 scope로 둔다.
- 구조 교체 대안: 여러 renderer가 제각각 export를 만들지 않도록 explicit `ConversationExport` projection+host sink를 도입. replay Journal과 사람용 export를 동일 형식으로 만들지 않는다. portable session schema는 실제 import 수요/identity 계약을 확인한 뒤 확장.
- Owner: session live projection/export + host local-file/clipboard sink; privacy owner 공동. 수락: storage gap 이후 U/A/tool 3개 이상→export→프로세스 종료→파일만으로 공개 내용과 not-saved 범위를 복원; secret/private payload 제외; 한도 초과 누락 공개; read-only disk/clipboard 차단 시 같은 draft 유지. 이 감사에서 실제 gap/export를 실행한 것은 아니다.

### F7 · P1: 사용자가 자신의 작업 환경을 제품 안에서 조정할 수 없다

- 분류: **설정 UI 없음 / backend capability별 일관된 조작 설명 부족**, R03/R04/R06/R09/R12/T08/T09/T35/T38/T50/T52/T54/T59.
- 지금: theme/추론 표시(show_reasoning)/이미지 표시(show_images)/notifications/output 길이 같은 일상 preference가 `config.yaml`의 정확한 필드명으로 존재한다. live 설정/preview/reload 진입은 없다. 모든 setting과 연결 capability를 문서에서 알아야 한다. `/prompt`도 이름 목록을 읽고 다시 입력하는 경로다. “API는 configurable”은 ordinary user가 조정 가능하다는 말과 다르다.
- 근거: [Y: state/config/parse.rs::TuiConfig/parse_snapshot](../../../crates/yo-cli/src/state/config/parse.rs), `application/runtime/live.rs`의 시작 시 `config::load`, 17-command registry. `runner/model.rs::panel`은 service detail/disabled reason을 표현할 재료는 이미 있다. P `interactive-mode.ts::showSettingsSelector`는 theme/images/default model/steering/followUp 등 current value와 callbacks를 갖는다. `test/settings-selector.test.ts`는 fullscreen option cycle 및 theme 탐색 중 configured 값/선택 후보 구별을 assert한다. C `slash_dispatch.rs`의 Theme/Permissions/Status 진입은 비교점; 전체 설정 parity를 주장하지 않는다.
- 범위 구분: `show_images`/`show_reasoning`은 OutputPreferences의 표시 설정이다. 이미지 입력 capability나 모델 추론 계산을 끄는 기능이 아니다. 모델 reasoning effort 변경은 별도의 admitted model-profile/current/default/replacement 제안으로 다룬다.
- 목표: 설정→현재 값/적용 범위(이번 대화/기본값)→preview→적용/취소. 모델 선택에는 작업에 필요한 capability를 설명하고 unavailable은 제출 뒤 실패만으로 학습시키지 않는다.
- 최소 수정: 표시 preference부터 /settings, current/default 구별, trusted update/write/read-back. 권한 설정은 cosmetic setting과 별도 typed policy owner를 사용. 현재 read-only policy를 조용히 넓히지 않는다.
- 구조 교체 대안: monolithic startup config snapshot을 editable preference service와 frozen execution config로 나눈다. renderer preference 변경 때문에 Session/backend를 다시 만들지 않으며 tool/replay contract 변경은 정해진 replacement 경계를 따른다.
- Owner: CLI preference/config + TUI settings + model capability projection. 수락: 밝은/단색·이미지 표시 off·추론 표시 off·notification 변경/취소 후 현재 draft 유지; SSH에서 지원되는 키 대체를 UI로 확인; restart 후 default만 재현, current-session 선택을 default로 오인하지 않음.

### F8 · P1: “핵심 서비스 경험” 대신 렌더링 특수기능이 제품의 중심처럼 보인다

- 분류: **사용자 가치 검증 부족 / 우선순위 편향**, R01/R06/R09/R10/T21/T24–T29/T38/T59/T61–T63.
- 근거: 제품 README 앞부분의 많은 chart/image/preview fixture 설명, `docs/src/README.md`의 개발자 안내 목적, 기존 TUI audit의 풍부한 content subinventory. 이 자체가 렌더러가 나쁘다는 근거는 아니다. 그러나 온보딩/주제 선택/설정/export는 빠진 채 개발 preview가 긴 제품 설명을 차지한다. 실제 경쟁 과제 완료 시간·도움말 왕복·오입력률 evidence가 없다.
- 지금→목표: 기능명을 읽고 실험용 화면을 찾아보는 서비스→코드 변경 요청·중단/수정·결과 검토·다음날 재개를 스스로 끝내는 서비스.
- 최소 수정: 기본 제품 페이지와 first-run을 5개 실제 과제로 다시 구성하고 fixture는 developer docs로 연결. “feature 있음” 대신 성공 조건/실패 복구/소요 행동 수로 개선 우선순위 결정.
- 구조 교체 대안: Chat renderer 중심 roadmap을 task lifecycle 중심으로 교체(시작/진행/검토/회수/재개). 별도 대형 GUI가 필수는 아니나 더 많은 slash command만 추가하는 방식으로 완료를 선언하지 않는다.
- 비교: [P: core/slash-commands.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/slash-commands.ts)의 settings/name/export/import/login/task navigation이 public surface이고, [C: slash_command.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/slash_command.rs)의 diff/review/export/rename/permissions가 작업 완료 동선을 드러낸다. command 수가 많으므로 우수하다는 주장은 아니다.
- Owner: 제품 여정 + CLI/TUI composition. 수락: 같은 fixture/terminal에서 처음 연결, 진행 수정, 실패 진단, 검토/회수, 다음날 찾기 5과제를 help 없이 수행; 성공/실패·행동 수·외부 shell 전환·데이터 손실·오전송을 기록. 모델 품질/성능과 UI 효과를 분리. Pi/Codex에게도 같은 기준을 적용한다.

### F9 · P1 release 신뢰: 핵심 지원 환경에서 작업을 끝낼 수 있다는 증거가 부족하다

- 분류: **검증 부족**, R01/R03/R05/R12/T03/T11/T41–T58/T64.
- 근거: [Y: crates/yo-cli/tests/terminal_matrix/local_tmux.rs](../../../crates/yo-cli/tests/terminal_matrix/local_tmux.rs)의 exit/draft/bracketed paste/status cases는 compatible installed Codex+tmux가 필요해 ignored. `terminal_matrix/ssh.rs`도 local sshd+compatible Codex 전제. 기존 unit/parity 1040+4 결과는 기존 보고 evidence로 인정하되 실제 emulator/SSH/clipboard/accessibility·긴 작업 latency를 증명하지 않는다.
- 지금→목표: “tmux/SSH를 주 대상으로 한다”+모형 test→명시한 OS/emulator/key protocol/SSH tmux 조합에서 입력·승인·중단·export·resume까지 통과한 usable subset.
- 최소 수정: clean artifact+offline host fixture로 자동 PTY 여정을 검증하고, 실제 SSH/tmux 키·clipboard/links/pixels·외부 editor·screenreader는 사람이 관찰한 증거를 기록. 없는 fallback을 문서에 발명하지 않는다.
- 구조 교체 대안: terminal capability negotiation과 keymap을 중앙 owner로 통합하고 semantic action 기반 도움말을 생성. 실제 전송 불가능한 Alt/F-key를 core 기능의 유일 진입으로 두지 않는다. 이 분리는 임의 terminal 전체 지원 약속이 아니다.
- Owner: terminal/process shell + CI/release + accessibility 검증. 수락: 80x24/40x12, CJK, non-enhanced keys, local/SSH/tmux에서 5과제 완료·termios 복구·오전송0. streaming input→visible frame와 interrupt latency는 실제 계측해 예산을 정한다. 이 감사는 느리다/깨진다를 측정하지 않았다.

## 64 요소 재평가: 무엇을 통과로 착각하지 않을 것인가

아래는 64개를 사용자 과제에 재배정한 coverage이며 64개 소스 재실행 인증이 아니다. 직접 읽은 source/test는 위 finding에 명시했다. 나머지는 기존 상세 감사의 사실·한계를 승계한다.

| 요소 | 반대심문 / 결과 |
|---|---|
| T01/T02 | bounded editor/CJK 보존은 유지. Home/End·선택/좁은 빈 상태는 사람이 쓰는 편집 계약으로 검증; 낮은 우선순위가 자동 아님 |
| T03/T04 | newline protocol·paste 오제출 및 image undo의 payload 불일치 기존 결함 유지. text undo 존재가 whole draft undo 동등성 아님 |
| T05/T06 | same-session history/ordinary Chat search의 범위는 정확하지만 이전 작업·tool 실패 검색에는 불충분. F3/F4 |
| T07/T08/T09/T10 | typed reference·skill identity 유지. 하나의 skill 제한, template 이름 재입력, 여러 이미지 식별·삭제까지 과제로 평가. F7 |
| T11 | 외부 editor 왕복 보호는 강점. 마커 편집 거절 뒤 기존 입력 유지가 충분한 복구인지 실제 사용자 왕복 확인. F9 |
| T12/T13/T14 | active steer 정확성 유지. 중단이 주제 전환 완료를 의미하지 않음; paused queue gate+head-only recall 개선. F2 |
| T15 | 등록된 command 수가 부족한 사용자 행동을 발견하지 못함. settings/name/export/direct inspect 새 entry 검토. F1/F3/F5/F6/F7 |
| T16/T17/T18/T19/T20 | frame gate/secret 보호 강점 유지. 기존 request draft·clear·Esc/Tab 의미 P0는 미해결. 이미 문서에 적었다고 제품 문제가 해결된 것은 아님 |
| T21/T22/T23/T24 | streaming·rich tool/reasoning·compaction 자체보다 실패/대기 원인→상세→수정 행동을 검사. F4/F8 |
| T25/T26/T27 | source-preserving Markdown/code/chart 유지. 차트·도식 다양성이 일반 코딩 여정의 blocker를 상쇄하지 않음. F8 |
| T28/T29 | source API/원문 보존과 실제 export·원본 파일 접근을 구분. remote file/image를 열 수 없다면 사용자 한계로 표시. F5/F6 |
| T30/T31/T32 | saved diff·retained output 품질은 강점. 현재 Git·검색·selected source copy·failure navigation 부재를 별도 핵심 격차로 승격. F4/F5 |
| T33/T34 | 진단 Transcript/Request는 원문 탐색/승인 inbox를 대신하지 않음. 기본 UI 노출 비용과 naming 검증 |
| T35/T36/T37 | model/resume/tree의 정확한 identity 유지. 이름·내용·preview·현재/default·search가 사용자 판단 중심이어야 함. F1/F2/F3/F7 |
| T38/T39/T40 | help/disabled reason/generic selection은 재료. 이유를 읽는 것과 복구 행동으로 갈 수 있는 것은 다름. F1/F3/F7 |
| T41/T42/T43/T44/T45 | workspace/상태/주 행동을 우선. queue가 interrupt hint를 밀어내는 기존 결함; request presentation gate 강점 유지. F2/F9 |
| T46/T47/T48/T49 | source-anchor 필요, inline restoration 강점. 긴 대화 탐색→새 출력→복귀를 실제 task로 확인; folding만으로 정보 검색 대체 불가. F4/F9 |
| T50/T51/T52/T53/T54 | color/CJK API 강점. theme/reduced motion/keymap/copy가 actual CLI에서 조작 가능해야 의미가 있음. F6/F7/F9 |
| T55/T56/T57/T58 | raw-mode/cleanup 모형 성공과 실제 SSH/tmux/accessibility/latency는 구별. 주 대상 환경의 증거 부채로 관리. F9 |
| T59/T60 | custom renderer/source-preservation 유지. module/command 역매핑만으로 제품 coverage를 닫지 않음. F8 |
| T61/T62/T63 | developer preview·manual compact·interview 사본 관리의 범위를 정확히 유지. 이들을 일반 사용자의 task workspace/설정/export로 세지 않음. F7/F8 |
| T64 | truthful durability warning은 강점이지만 다음 행동이 latest-answer clipboard뿐이면 회복 UX가 부족. F6 |

## 권고 순서와 구조 판단

첫 묶음은 기존 request P0(T19/T20/잘못된 action hint), F1 첫 실행, F6의 저장 실패 회수다. 다음은 F2 작업 전환, F3 주제 탐색, F4/F5 결과 판단, F7 설정이다. F8의 과제 기반 검증과 F9 terminal 검증을 각 묶음의 수락 과정에 넣는다. 숫자 우선순위는 이 제품 여정 감사 안의 상대 순서이며 발생 빈도 계측치가 아니다.

“현재 구조를 무조건 보존”도 “전체 TUI 교체”도 근거가 없다. 기존 core identity/replay/acceptance, source-preserving renderer, Changes/Output source anchors는 유지할 이유가 구체적이다. 반대로 Session 선행 startup, 단일 draft에 queue 작업을 끼워 넣는 제어, 주제 없는 generic picker, source와 단절된 export/navigation은 부분 구조 교체 후보로 열어 둔다. 최소 패치가 위 수락 과제를 통과하면 채택하고, 불가능하면 명시한 작은 owner 단위 구조를 교체한다.

이번 검수는 발견한 제품 결함의 수정 완료가 아니다. source test 본문 확인과 기존 테스트 성공은 구별했다. 실제 모델 품질·로그인·사용자 완료시간·전체 실행 플랫폼·성능·접근성은 미검증이며, 외부 pin 기능의 존재도 전체 서비스 우월성으로 확대하지 않는다.

통합 판단과 변경된 배달 순서는 [전체 비판 검수](./critical-review.md)를 따른다.
