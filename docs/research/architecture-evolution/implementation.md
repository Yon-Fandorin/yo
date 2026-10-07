# Implementation order and verification

> Status: non-authoritative execution proposal
>
> 기준: [통합 설계](./README.md), 2026-10-05

아래 단계는 실제 구현 후보와 종료 기준이다. 이 문서를 쓰거나 읽는 것은 해당
기능이 구현됐다는 뜻이 아니다. 이전 단계가 이후 모든 작업을 막는 거대한
approval gate가 되지 않도록 코드·계약 의존성만 연결한다.
모듈별 최소 변경과 의존 방향은 [Pi/Codex/Yo 모듈 비교](./module-comparison.md)를 함께 따른다.
전체 작업 범위와 담당별 검수는 [검수표](./review-index.md), 사용자 여정의 수용 기준은
[사용성 비교](./usability-comparison.md)에 있다.

TUI 구현은 [64개 요소 검수의 W1–W9](./tui-review.md)를 세부 목차로 삼는다.
U0b의 첫 수정은 요청별 초안 소유권, secret 전체 지우기, 실제 action과 표시의 일치다.
기존 frame gate·typed references·Changes/Output·terminal restoration을 보존하며
현재 owner 단위로 나눈다. 새로운 전면 TUI 프레임워크나 host 추출은 선행 조건이 아니다.

## 단계와 소유 경계

[전체 비판 검수](./critical-review.md)의 K0/K1/K2를 제품 배달 순서로 적용한다.
아래 문자 단계는 owner/contract 작업 묶음이며 A→H 직렬 일정이 아니다.
S0를 신설해 배포·실패 회수·자동 회귀 검증을 앞당기고, A의 기본 prompt/search와
C의 반복 승인 개선을 U0와 함께 시작한다. 기존 G 완료까지 출시 기반을 기다리지 않는다.

| 단계 | 구현 범위와 첫 소유자 | 완료를 증명할 관측 | 실제 선행 조건 |
|---|---|---|---|
| S0 | 설치/version/지원 profile, PR test gate, artifact smoke, live rescue export와 저장 수명·진단/계측; release/repository/consumer owners | clean-state 첫 여정, required job 실패 감지, memory-only 공개 결과 회수, cap 회복과 지원 근거 | 현재 TUI/native/delegated profile; GUI/MCP/child 불필요. 각 owner 작업 병렬 |
| U0 | Session 전 setup/model, 일반/질문 draft 분리, secret 전체 clear, Enter/queue/request 실제 action 표시 | 연결 없이 setup/기록 진입, UUID·YAML 암기 없이 일반 여정, 의도치 않은 답변/숨은 secret 잔존 방지 | 현재 CLI/TUI/service owner; host·GUI 불필요. U0a/b/c 독립 작업 |
| A | 버전 있는 제품 base prompt, native instruction/skill snapshot, typed search; CLI 순수 조립/model/tool owner | admitted tools/workspace/AGENTS가 실제 request에 포함, search가 shell 없이 동작, old resume exact prompt 유지, task oracle | 각 새 tool/context delta, S0의 affected validation; backend의 임의 filesystem read 금지 |
| B | ToolBatch read-only scheduler; managed + host claims | 4-slot window 안 독립 read 진행, 결과 semantic publication/model replay source order, raw result reservation·cancel/cleanup bounds | claims port와 exact admission, coordinator live Session cap 8; MCP/host 추출은 불필요 |
| C | versioned structured process tool + sandbox adapter + call/session/explicit persistent grants | 승인된 scope의 routine 명령 자동 실행, scope 외 거절/새 요청, revoke race 거절 | host identity·sandbox capability·명시 grant 의미; task scope는 F2 |
| U1 | queue 항목 편집/삭제·주제 전환, session title/search/page, 설정, focused result search/copy와 current Git inspection | 현재 draft를 비우지 않고 queue 정리, 주제로 대화 선택, failed tool 원문 탐색, saved/current evidence 구별 | 각 흐름의 기본 입력/소유권 안정화; 새 저장/grammar/capability별 delta. U0 전체 완료의 일괄 gate 아님 |
| D | workspace-bound services/Session factory와 trusted local client vertical. 첫 지원 mutation부터 controller/intent/receipt subset 포함; WS/browser/artifact는 후속 | TUI/print behavior 보존, idle Start/exact Steer, 초기화 실패 cleanup, 2 clients 동일 Session, Journal suffix/gap recovery | 실행 조립의 독립 소비자, protocol DTO, closed persistence deltas |
| E | same-origin browser 한 consumer의 SSH vertical 우선; desktop thin shell은 후속. Changes inspect-only, typed request profile | 실제 host로 setup→start→steer→approval/interview→diff→interrupt→resume→reconnect 완료; unsupported profile의 실제 controller handoff | D; auth/Origin/CSRF/upload/consumer capability; DOM secret·durable interview drafts와 Queue/Revert는 후속 |
| F1a | 작은 native plan mutation owner와 ActivityPlan projection | 생성/갱신/compaction/continuation에서 현재 plan 보존; step 완료와 Goal 성공 구별 | plan/Journal 의미 delta; hard budget/child 불필요 |
| F1b | typed Goal와 managed hard request budget | compact 뒤 objective 지속, 모든 dispatch의 reservation/reconcile | 공통 private dispatch gate와 parent Journal/budget enforcement capability |
| F2 | bounded fresh child Sessions/worktrees, TaskOrigin/seed와 task grant ceiling | active parent의 read delegation, parent/child crash join, complete seed 복원, isolation/result | F1b budget owner, exact task identity/ownership, shared coordinator; D의 전면 GUI 완료 불필요 |
| F3 | 사용자 명시 workspace notes+recall/inspect/edit/off/forget 우선; 자동 contribution은 후속 | 새 chat recall/stale/off/forget, counted=sent=recorded=resume | memory repository/admission snapshot delta; F1a/F1b/F2·worktree 완료 불필요 |
| G | admitted lifetime read-only stdio MCP+closed schema/text, typed model options/auth/직접 dialect | pre-spawn confinement·discovery bounds, registry drift/unsupported profile, 각 wire capability와 consumer smoke | MCP는 C의 launch admission/confinement 필요; provider는 독립. release/eval/계측 baseline은 S0에서 선행 |
| H | measured followups: GUI Queue/Revert, wider MCP, logical-project memory sharing, codemode, lossy migration, public remote/self-updater | 기존 budget/권한/복구를 지키며 실제 병목 또는 수요 해결 | 각 capability의 lifecycle·evidence와 명시 compatibility/security delta |

U0의 기본 사용성은 제품 선행 우선순위이며 모든 runtime 작업의 blocking gate는 아니다.
A의 prompt/지침/검색과 S0의 CI/release/복구는 각자 독립 작업이다. B와 C도 별도 개선이며 B를 승인 재사용
설계 전체가 끝날 때까지 기다리게 하지 않는다. MCP가 없다고 typed basic read
parallelization을 늦추지 않는다. GUI prototype이 instruction/search 개선을 대체하지 않는다.

F1a/F1b/F2/F3은 rollout 경계이며 새 crate 셋을 뜻하지 않는다. F1a의 유용한 native plan을
hard budget까지 기다리게 하지 않는다. F2는 Goal-funded task의 ownership/budget을 위해
F1b에 의존하고, F3의 명시적 workspace notes/recall은 독립적으로 도입한다. 첫 child는 read-only
조사부터 시작한다. Standalone child ownership transfer와 worktree 자동 memory alias는 미룬다.

D의 초기 local transport도 mutation을 지원하면 single controller generation과 durable
intent/receipt reconciliation을 함께 구현한다. trust profile이 좁다는 이유로 ACK loss나
중복 제출 의미를 미루지 않는다. 첫 operation subset을 작게 하되 그 subset은 완결한다.

C의 첫 자동화 입력은 제안 `exec_process` 계열 structured tool로 구체화한다.
실행 identity·literal argv·cwd·host environment·sandbox의 검증이 grant에 연결된다.
기존 opaque shell은 유지하며, fixed wrapper만 먼저 지원하는 대안을 택하면 그 범위로
자동화 완료 주장을 제한한다. 새 registry/approval-policy 의미는 구현 시 authority delta다.
G의 첫 MCP 완료에는 유용한 실제 server/tool inventory 하나의 호환 증거가 필요하다.
closed schema/text/효과 제한 때문에 해당 inventory가 들어오지 않으면 profile을 정식으로
개정하거나 restricted experimental로 표시한다. unsupported 필드를 몰래 버리지 않는다.

## Candidate interfaces

아래는 컴파일된 API가 아니라 구현자가 책임을 확인할 수 있는 예시다. 기존 타입은
재사용하고 이름·generic 모양은 실제 경계 구현 때 고정한다.

```text
core::tool:
  ToolResourceClaims
  ToolAuthorizationDecision = Allowed(exact) | NeedsApproval(request) | Denied(reason)
  ToolPermissionPort::authorize/revalidate/revoke_generation

managed::tools:
  ToolBatch { private indexed SlotState, derived indices, reorder_window }
  reserve_result_allowance(index) -> bounded slot
  poll_ready_and_running(budget) -> progress / prepared ordered BackendEvent proposals
  retire_after_publication_barrier(index, generation)

managed::request:
  private RequestAdmission -> immutable request + accounting
  private RequestDispatcher -> transport only after exact permit / required receipt

host:
  ExecutionSettingsSnapshot, SessionLaunch, neutral diagnostics/errors
  prepare_workspace_services(canonical_workspace, config_revision) -> services candidate
  assemble_session(settings, workspace, replay_source) -> prepared candidate
  service.inventory/open/attach/snapshot/start/steer/respond/interrupt/inspect_operation
  service owns live Session entries; bootstrap injects narrow prepared-session factory
  client.local/remote -> neutral operations / bounded observations / typed errors
  client attachment drop -> detach; host-owner handle -> explicit shutdown
  HostControlIntentLedger { key, semantic_hash, auth_context, dispatch, journal_join }
  operation epoch + retired-before fence; exact urgent closure reservation
  quiescent explicit rotation -> client reinitialize; old mutation always rejected
  permissions.list/read/revoke(exact_grant, expected_revision, authorized_actor)
  sandbox.prepare(policy, command) -> verified execution profile or unsupported
  mcp.admit_server(profile, lifetime_claims, transport_budgets) -> connection candidate
  mcp management: add/list/status/connect/recheck/stop/remove, frozen live registry preserved

core::session:
  StartIfIdle(input), Steer(exact_turn, input)
  ExactInterrupt(exact_turn) -> existing InterruptTurn { turn }
  OpaqueControlOperationToken, ControlAdmission/Outcome

core::agentic:
  GoalState, PlanState, TaskLink, BudgetReservation, TaskOrigin, TaskSeed
  TaskOutcome, EvidenceAvailability
  parent Journal: Goal/Plan/TaskLink/BudgetReservation
  child Journal: TaskOrigin/seed/input/terminal
  worker-private GoalLedger -> pre-dispatch reservation / exact permit
  service goal create/update/pause/resume/cancel/set-budget -> exact worker command
  task list/read/interrupt -> typed capability; join remains parent-worker reconciliation

core::memory:
  MemoryScope, MemoryRevision, immutable admitted MemorySelectionSnapshot
  minimal input admission capability only

host::memory:
  MemoryRepository CRUD/search + local files + generation-CAS + optional index
  bounded admission guard acquired by worker for its own snapshot commit
```

Mutable resource claims와 call authorization은 model-visible tool schema에 임의 field를
더하는 방식으로 구현하지 않는다. Host preparation/admission boundary에서 만들어
managed scheduler에 넘긴다. Old tool manifests의 hash는 변경하지 않는다.

Host ledger는 bounded control intent와 correlation의 저장 owner다. Session Journal의
terminal 의미나 Goal/task/budget을 복제하지 않는다. 모든 API 그룹을 한 번에 구현하지
않고 optional 그룹은 negotiated capability로 노출한다. SecretInput은 ledger 밖의
exact single-use volatile 경로이며 body/hash를 보관하지 않는다.

Client는 TUI AgentPoll/LinkResolver나 core PendingCommand를 wire로 노출하지 않는다.
Service 도입 전 neutral factory 결과는 CLI가 소비하고, 도입 후에는 service가 Session을
소비한다. Local 미-admit retry와 remote delivery unknown은 별도 상태다. Public 면에
Session map/ToolBatch/GoalLedger/store lock이나 unrestricted mutable Session을 추가하지 않는다.
Optional transport/schema/assets와 local-only consumer를 각각 검증한다.

## Representative verification matrix

| 시나리오 | Positive evidence | 실패/경계 evidence |
|---|---|---|
| 지침 탐색 | 선택 cwd chain, override precedence, exact content digest | symlink/invalid UTF-8/budget excess/ancestor scope rejection |
| 지침 resume | saved instruction 그대로 request에 사용 | 현재 AGENTS 변경이 historical contract를 덮지 않음 |
| 검색 | bounded matches, Unicode, correct line and continuation | ignored credential/symlink 제외, regex bound, stale cursor |
| 병렬 도구 | 실제 disjoint claims의 A/C가 approval B보다 먼저 완료 | B/C overlap이면 대기; unknown shell exclusive. 현재 registry의 shell+workspace read는 overlap이며 synthetic case를 제품 개선 실측으로 세지 않음 |
| 결과 정렬/용량 | 역순 physical readiness에도 Journal terminal/model replay source order | index 0 지연+수백 calls에서 첫 4 slots만 dispatch, 첫 excess 대기; raw reservation·encoded bound 유지 |
| 승인 재사용 | C의 call/session/persistent rule로 새 exact call authorization | args/tool artifact/host/workspace 변경, expiry/revoke race 거절; F2 전 task scope 없음, fork 자동 상속 없음 |
| 저장 권한 관리 | 저장→반복 실행→matched grant 확인→철회→restart 후 유지 | queued exact call의 dispatch 전 revalidation, stale grant revision 거절, controller revoke와 혼동 없음 |
| coordinator scope | 같은 coordinator의 충돌 claim은 직렬 dispatch | 별도 direct process/editor는 external writer; observed stale를 OS 전역 atomic CAS로 과장하지 않음 |
| sandbox | permitted read/write/process/network fixtures | 각 restriction의 실제 OS effect 차단, unsupported fail closed |
| factory lifecycle | workspace-bound services 준비 뒤 Session 생성, 진단 반환·실패 cleanup | 다른 cwd/revision services 재사용, 준비 실패 뒤 partial live handle, 숨은 model fallback 없음 |
| 모듈 추출 | print의 neutral factory 소비, production dependency/feature별 consumer build | host→TUI, service→assembly→service 순환, local-only의 web stack 강제 없음 |
| client lifecycle | 같은 service admission/projection, attachment detach와 owner shutdown 구분 | remote unknown을 local retry로 재전송, disconnect가 Session을 종료, private queue token 노출 없음 |
| state/dispatch 경계 | private slot transitions, Runtime publication 뒤 retirement, 전 경로 budget permit | readiness를 commit으로 오인, summary/protected 경로 budget 우회, worker callback/lock 대기 교착 없음 |
| 취소 | approval·ready·running 혼합 batch 전부 seal/cleanup | late progress/result·cleanup failure와 terminal race |
| Session API | admitted operation status와 snapshot+delta gapless | UUID body mismatch, overload, crash unknown effect |
| Start/Steer 시간 경계 | StartIfIdle와 exact TurnRef Steer의 worker admission | A 종료·poll·steer 순서가 바뀌어도 새 B로 변경하지 않음; draft 보존, GUI Queue fallback 없음 |
| host intent ledger | admission 전 record/key/total byte·향후 correlation 공간 예약 | 첫 capacity excess 일반 dispatch 거절, crash join 불일치 unknown, key GC/reuse로 효과 재실행 없음 |
| ledger pressure 복구 | normal capacity-full에도 exact stop/decline/revoke 예약 유지, quiescent epoch rotation | running/approval-pending의 urgent control, pending/unknown rotation 거절, rotation crash·restart 뒤 old epoch replay 거절·자동 재전송 없음 |
| transport budgets | count/byte 한도 안의 여러 client와 paged history | first oversize envelope, byte-full queue, slow client resync; control lane 보존 |
| host connection 총량 | connections/attachments/total queued bytes admission reservation | 첫 초과 attach 거절, 기존 controller/urgent 공간 보존; per-client cap을 host 전체 cap으로 오인하지 않음 |
| operation acceptance | queue·worker acceptance·terminal outcome·durability를 각각 표시 | memory-only/gap를 durable success로 오인, transfer 전 admitted command의 implicit cancel 금지 |
| control completion | respond 소비/seal, exact interrupt 대상 terminal, replace epoch, compact checkpoint/accepted 후 failure를 correlation | generic interrupt가 새 Turn을 선택하지 않음; admission 전 rejected·accepted 후 failure 구분, stale/no-op 영구 queued 없음, secret durable hash 없음 |
| request consumer profile | approval/plain/choice+notes/Previous를 valid presentation과 exact request에 연결 | pending/invalid presentation에서 입력 없음, unsupported profile의 검증된 TUI transfer 또는 explicit cancel; secret plain-text fallback/uncertain resend 없음 |
| interview draft lifecycle | composer와 request draft 분리, 동일 live request/controller 재확인 | transfer/restart/stale draft의 편집·새 메시지 전환 없음; intermediate ACK로 전체 삭제 없음; TUI durable final seal 후 cleanup 보존 |
| controller | observer read와 explicit transfer 성공 | old attachment/generation/turn/request late response 거절 |
| controller loss | 승인 중 controlling tab 종료 후 인증된 새 client가 명시적으로 제어권 회복 | heartbeat expiry/explicit revoke race, 재접속한 old client 응답 거절 |
| 웹/초기 연결 | host-owned workspace/model inventory, TTY setup 안내와 explicit Refresh, authenticated 여정 | client-local path를 remote workspace로 오인, hidden default/probe, refresh draft loss, forged Origin/Host/CSRF/WS auth |
| model inventory selection | native complete ModelTarget와 runtime delegated opaque HostModelSelection 재검증 | stale catalog/cross-account reuse, delegated coordinate persistence·Automatic model 발명 없음 |
| bootstrap lifecycle | single-use short-expiry exchange와 session-bound cookie | duplicate/expired token, revoke/shutdown/restart 후 old cookie/control 거절 |
| native/browser transport | 각 client trust profile의 정상 접속 | raw-native token 경계를 browser cookie로 오인, wrong Origin/credential profile |
| DOM content | text/Markdown/artifact를 제한된 renderer로 표시 | hostile raw HTML/script/URL scheme/remote image와 CSP enforcement |
| 재접속 | 마지막 accepted coordinate 뒤 ordered public suffix, gap+첫 complete recovery snapshot | duplicate delta, filtered sequence, snapshot 부재 unavailable; private replay 노출·volatile progress의 stored 표시 없음 |
| 첨부/변경 | validated upload/artifact ID, complete evidence-backed inspect-only diff | path traversal, hostile MIME, first excess byte, unavailable/before-image 부재를 가짜 diff로 채우지 않음 |
| upload 수명/총량 | host 전체 byte/count/concurrency 예약, worker의 self-contained image snapshot commit 뒤 임시물 정리 | 첫 aggregate excess, 미제출 취소/disconnect lease/restart, ACK 유실을 중복 제출·admitted bytes 삭제로 처리하지 않음 |
| 메모리 | admitted scoped recall snapshot과 CAS | cross-workspace leak, stale source, index loss, secret/forgotten recall |
| memory contribution | 사용자가 켠 workspace에서만 bounded candidate 저장 | 자동 global 승격·숨은 model call·source reference를 truth proof로 오인 금지 |
| memory forget race | CAS/forget 공용 lock과 durable tombstone 복구 | old writer resurrection·crash 중 partial 삭제·tombstone capacity bound |
| recall/off admission race | scope revision lock 안 self-worker input snapshot commit, 실제 bytes exact resume | forget/off 먼저면 selection 폐기/recount, admission 먼저면 immutable history; 다른 writer wait/lock inversion 없음 |
| worktree memory scope | host+canonical workspace 분리와 explicit parent snippet seed | sibling worktree implicit alias/recall 없음, child가 parent scope에 contribution 없음 |
| Goal/Plan | acceptance evidence reference, compact 이후 동일 state | model-only completed claim, pause/cancel를 complete로 오판 |
| Goal transition | pause/resume/needs_input와 child/parent 완료를 구분 | child 성공을 parent criterion 검증으로 오인, stale revision update |
| goal budget | parent/child/summary reservation과 usage reconciliation | concurrent child first excess, missing usage 0 환불 금지, restart fresh allowance 금지 |
| backend budget capability | managed pre-dispatch logical request limit과 verified retry allowance | delegated 사후 usage/retry를 hard rounds로 오인 없음; hard-round Goal의 unsupported child/model replacement 거절 |
| child task | bounded read delegation와 isolated worktree result | depth/session limit 첫 excess, shared Git lock, dirty file 보존 |
| child spawn/fork | active parent에서 fresh seeded task; self-contained child history 복원 | 같은 조건 intentional fork 거절; pending effect/private replay/권한 상속 없음; parent 부재 funded continuation 중지 |
| child creation failure | initial worker acceptance와 parent link 뒤 active 공개 | cancel/partial failure/orphan cleanup, uncertain acceptance reservation 유지, retained patch 보존 |
| parent/child crash join | 단계별 exact task/Journal join과 parent의 단일 usage reconcile | child accepted→parent running 사이 crash, 중복 child/환불 없음; detached child의 parent budget owner 유지 |
| crash | completed safe boundary 재개 | in-flight shell/MCP 자동 재실행 없음 |
| MCP/provider | 기존 closed schema 통과+bounded text result, admitted connector payload | `$ref`/`oneOf`/`default` 삭제 없음; image/resource/structured content의 text 성공 위장·자동 fetch 없음; drift/readOnlyHint spoof |
| MCP lifetime effects | spawn 전에 artifact/env/credential/ceiling 확인, 전용 state와 read-only workspace, reconnect/revoke cleanup | initialize/background/child의 workspace write, state alias, shared external mutation profile의 call lease 우회 거절 |
| MCP discovery bounds | frame/decode/catalog/pages/cursor와 전체 deadline 안 discovery | 첫 excess, repeated cursor, huge unsupported content, stderr flooding/cancel; 실패 server가 local tools/다른 Session을 막지 않음 |
| MCP 관리 여정 | 추가→연결→상태→재확인/중지/제거와 unsupported 뒤 복귀 | live frozen registry의 silent 변경, 비밀 표시, stop 뒤 고아 child/진행 중 call 방치 없음 |
| Goal/Task 제어 여정 | 목표/기준/예산 표시와 revision-bound create/pause/resume/cancel, child inspect | stale update·budget 확대 거절 때 편집 보존, UI의 별도 join/완료 authority 없음 |
| Codex compatibility | 고정 version의 initialize/choices/events/resume/image/secret fixtures | source 최신이라는 이유로 verified version 확대, unknown capability의 silent fallback |
| accounting | admitted quality/policy/estimate/reserve가 pressure·checkpoint까지 동일 | zero-image advisory를 exact로 relabel 금지, missing usage/unknown cap의 hard-budget 과장 금지 |
| cache attribution | reported zero와 absent/source-profile 보존 | cache warming inference의 budget 누락, connection prewarm을 inference로 잘못 집계하지 않음; warming 자체는 H 보류 |
| Gap 표시 | direct memory-only와 durable-required service의 실제 admission 상태에 맞는 안내 | service dispatch가 닫혔는데 실행을 계속할 수 있다고 표시하지 않음 |
| 설치/운영 | Linux/macOS manual clean install/startup/package upgrade/resume와 state 보존 | generated DTO drift, stale client reinit, incompatible API/schema mutation 거절, checksum/permission failure와 rollback action |
| artifact/CI/eval | 실제 package와 외부 SDK consumer, expected eval manifest와 성공 oracle, required-job aggregate | workspace-only dependency 누락, wrong-model/skipped/unscored를 성공·0 비용으로 집계, required job 실패/취소의 성공 처리 없음 |
| UI 접근성 | keyboard, screen reader, CJK, narrow/reduced motion | browser/TUI 각각 long-output/resize/disconnect draft 보존 |

Bounded fixture/child-process tests를 먼저 사용한다. Mock request acceptance만으로
실제 process sandbox, browser reconnect, TTY restoration을 검증했다고 주장하지 않는다.
동일 output directory나 mutable fixture를 쓰는 검증은 직렬화한다. 독립 resource와
계약을 쓰는 구현·검토는 사용자 작업 요청의 authorization 안에서 병행할 수 있다.

## 제품 parity의 실제 평가

비교 fixture는 최소 아래 유형을 포함한다.

1. AGENTS.md 규칙을 따르는 한 파일 bugfix와 회귀 검사.
2. 현재 typed read/search batch와 실제 disjoint approval 사례를 분리. opaque shell 승인 대기+workspace read는 충돌 대기가 정상이며, synthetic disjoint fixture 통과를 승인 절감 실측으로 표시하지 않음.
3. 모호한 shell effect에서 scope 확대를 요청하고 취소 후 cleanup.
4. 긴 대화의 compaction·steering·exact resume/fork.
5. 격리된 두 child 작업의 결과 수집과 변경 검토.
6. 새 대화에서 검증된 project memory recall, stale memory와 forget.
7. 브라우저/desktop/TUI의 같은 host Session 제어권 이동·재접속.
8. packaged install로 모델 연결·artifact 첨부·실패 복구까지 진행.

동일 모델을 사용 가능한 경우 동일 모델·task input·tool/context budget으로
비교하고, 모델/host capability가 다르면 그 조건을 기록한다. 평가 수치는 task
완료율, correction count, human approval count, model/tool rounds, total/p50/p95
latency, cancellation/recovery success, token/usage availability를 포함한다.

초기 수용 목표는 불필요한 반복 scope 승인 0, denied effect 실행 0,
mis-correlated result 0, silent stale approval 0, secret leak 0,
complete journey failure 0이다. 이는 검증해야 할 목표다. 모델 품질과 latency의
Pi/Codex 동등성을 수치 없이 달성했다고 선언하지 않는다. Paid provider eval은
명시된 비용/credential 범위가 있을 때만 실행한다.

Eval manifest는 source/model/profile/registry/context와 fixture hash, permission/budget,
cache 조건, 반복 번호와 성공 oracle을 보존한다. Expected runs와 실제 결과를 대조해
skipped/errored/unscored/unknown usage를 분리하고 pair 제외 이유·유효 표본 수도 보고한다.
CI/설치 검증은 실제 배포 artifact와 격리된 state root를 사용한다. SDK 배포 시 작은
외부 consumer의 공개 factory 사용을 검증한다. 이들은 단위 fixture와 별도 evidence다.

## 계약 delta 지도

| 구현 delta | 현재 확인할 accepted owner | 처리 방향 |
|---|---|---|
| host extraction | [frontend boundary](../../../methexis/knowledge/agent-runtime/agent.core.frontend-independent-boundary.md), [Yo Host](../../../methexis/knowledge/agent-runtime/agent.remote.yo-host.md) | independent consumer와 CLI process ownership 증명; 같은 Host 의미의 배치 |
| instructions/registry | [managed loop](../../../methexis/knowledge/agent-runtime/agent.backend.yo-managed-model-loop.md), [local tools](../../../methexis/knowledge/agent-runtime/agent.tool.local-execution-boundary.md), [format](../../../methexis/knowledge/agent-runtime/agent.persistence.format-compatibility.md) | 새 contract/revision과 legacy exact resume 규칙 |
| batch/grants/sandbox | 같은 managed/local tool owners, [execution topology](../../../methexis/knowledge/agent-runtime/agent.backend.execution-topology.md) | reorder bounds·independence proof·grant lifetime·permission evidence 명시 |
| protocol/controller | [active input](../../../methexis/knowledge/agent-runtime/agent.runtime.active-turn-input.md), [view projections](../../../methexis/knowledge/agent-runtime/agent.observability.view-projections.md), [remote](../../../methexis/knowledge/agent-runtime/agent.remote.yo-host.md) | idle/exact control와 public suffix; frontend가 실행 state owner가 되지 않음 |
| host intent ledger | [repository](../../../methexis/knowledge/agent-runtime/agent.storage.session-repository.md), [Journal](../../../methexis/knowledge/agent-runtime/agent.observability.session-journal.md), format owner | 새 bounded domain/key-retention/correlation; Session terminal meaning 복제 없음 |
| Goal/Task/budget | Journal·format owners, [continuation lineage](../../../methexis/knowledge/agent-runtime/agent.session.continuation-lineage.md) | parent/child closed records와 crash join, backend별 enforceable 단위 |
| memory | 기존 context/accounting/format owners와 신규 작은 scope/repository owner | 실제 bytes snapshot, generation/use-policy admission, canonical workspace scope |
| provider options/retry | [complete selection](../../../methexis/knowledge/agent-runtime/agent.model.session-selection.md), 해당 Connector, [Codex boundary](../../../methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md) | closed wire·secret/no-retry 규칙과 verified capability compatibility |

Accepted owners의 entry route는 [AGENTS.md](../../../AGENTS.md)와
[active checkpoint](../../../methexis/active-checkpoint.yaml)다. 리서치 문장이 그
authority를 자동 supersede하지 않는다. 구현 후보 선택 때 실제 delta에 repository
formal route가 적용되는지 판단한다. 이미 승인된 routine 작업마다 다시 승인
절차를 만드는 방식으로 도입하지 않는다.

## 팀 설계 결정

세 read-only 에이전트가 runtime/권한, UI/API, agentic/memory를 각각 검토하고 서로의
실패 시나리오와 축소안을 대조했다. 주 검토자는 higher-capability `gpt-6-astra`이며
초기 검토는 fresh context였다. 아래는 최종 후보에 채택한 선택과 의도적인 후속 범위다.
현재 코드/authority 근거는 [경계 증거 표](./evidence.md#팀-검토에서-보강한-경계의-직접-근거)에 있다.

| 논점 | 최종 선택 | 보류하거나 피한 확장 |
|---|---|---|
| 병렬 실행 용량 | 4-slot reorder window, dispatch 전 raw allowance 예약, source-order semantic commit | batch 전체 4-call 축소, 무제한 completed slots, RSS 보장 주장 |
| 권한 scope | C는 call/session/persistent, F2 뒤 typed task ceiling; resume 별도 재admit | C의 별도 permission-task framework, replay를 grant로 복원 |
| writer 충돌 | 같은 coordinator의 claim/lease만 보장 | OS 전역 lock service, 강제 daemon, external writer atomic CAS 주장 |
| Start/Steer/Interrupt | worker idle-only Start와 exact TurnRef Steer/Interrupt, stale draft 보존 | 첫 GUI Queue와 generic interrupt target 재선택; 기존 TUI Queue는 유지 |
| 제어·복구 | bounded host intent ledger+neutral control outcome, method별 completion, ordered public suffix/gap recovery | 범용 transaction DB, ACK를 실행 성공으로 표시, raw private Journal 공개 |
| 첫 연결/업그레이드 | host root/model inventory, trusted TTY setup+Refresh, manual start/attach/package upgrade | 자동 probe/default, remote client path 해석, discovery broker/self-updater |
| 변경 검토 | complete evidence 기반 inspect-only Changes, unavailable 명시 | 첫 Revert; 후속은 coordinated-writer/evidence capability 필요 |
| child 생성/복구 | fresh seeded Session, parent/child 각각 Journal owner, exact join/unknown, detached parent owner 유지 | active intentional fork, ambiguous crash redispatch/refund, implicit standalone transfer |
| Goal 예산 | managed hard logical requests; delegated 내부 rounds/spend는 verified enforcer 전 planning | 사후 usage를 pre-dispatch permit으로 해석, unsupported binding의 단위 downgrade |
| 기억 | actual bytes snapshot과 off/forget admission lock, worktree별 canonical scope; F3 독립 rollout | digest-only replay, 자동 project alias, vector DB/background 학습 pipeline |
| MCP | 첫 G는 pre-spawn/lifetime admission+read-only workspace+isolated state, bounded discovery, closed schema/text | background mutating profile과 새 broker, schema 약화, content 성공 위장, full parity 주장 |
| Interactive consumer | 첫 DOM은 typed non-secret 질문/선택/notes/Previous; exact volatile request draft와 supported-controller handoff | DOM secret/durable interview draft 조기 확대, 일반 editor fallback, 불확실 secret 재전송 |

추가 최종 팀이 기존 verdict를 증거로 삼지 않고 29개 레이어의 인용 코드와 인접 호출부를
재점검했다. [레이어별 재검수](./layer-review.md)에 실제 확인 범위, 의도적 보류와 누락
판정을 남긴다. MCP lifecycle/discovery와 interactive consumer profile의 보강을 반영했으며
세 담당이 이 수정된 후보와 레이어별 검토표까지 독립 재검토했다. 검토 범위에서
남은 material finding은 없다.

## 설계 완료 감사

사용자 목표는 구체적인 설계안의 완성이다. 아래 표는 설계 deliverable 증거이며,
아직 구현하지 않은 feature test 통과를 주장하는 표가 아니다.

| 원래 요구사항 | 설계에서 확인할 증거 | 상태 |
|---|---|---|
| Pi/Codex에 비교 가능한 아키텍처 | source pin·29-layer 비교·36항목 검수·채택/보존/재구성 판단 | 삼자 원본 대조, 사용자 여정·모듈·누락 감사 |
| 내부 구조 개선 | 삼자 module/API/state 비교, dependency graph, move/retain map, 변경 파급 | README §1–3 및 module-comparison.md |
| TUI 디자인 | Pi/Codex 기준 setup/input/approval/results/resume 개선, 기존 Changes/Output 보존 | usability-comparison 및 README §9 |
| GUI 기반 | shared DOM와 desktop lifecycle/clipboard boundary | README §8–9 |
| 웹 접근 | authenticated loopback/remote route, API, controller, reconnect | README §8–9 |
| 엔진 | existing runtime ownership, model/tool rounds, cancellation/crash | README §3·5–6 |
| agentic behavior | goal/plan evidence, bounded child tasks/worktrees | README §6 |
| 메모리 | authority/scope/revision/recall/forget/CAS/index | README §4·7 |
| 반복 승인과 병렬 병목 | scoped grants, exact call binding, reserved resource order | README §5·12 |
| 공격적인 개선 포인트 | 기본 prompt/search·의도 보존·반복 승인·실패 회수·출시 기반 우선, 기존 owner 교체 대안 | critical-review K0–K2, S0/U0/U1/A–H |
| simple is best | one optional host crate, no forced daemon, files + derived index | README §1·3·7 |
| 소스 수준 근거 | Yo/Pi/Codex 직접 source와 official-product 경계 구분 | evidence.md와 layer-review.md의 29개 레이어; Codex rust-v0.160.0 commit 고정 |
| 실행 가능한 도입·검증 | actual dependencies, interfaces, adversarial/journey matrix | 이 문서 |
| 독립 검수와 검수자 누락 감사 | 3개 담당의 R01–R36 보고와 별도 fresh-context higher-capability 누락 검수 | review-index의 반영 상태·부분 검토·실행 대기 범위 |

## 현재 검증 상태

이번 변경은 리서치/설계 문서다. 제품 코드·Methexis 계약과 제품 UI는 변경하지 않는다.
이전 source audit의 세 focused tests와 독립 architecture review는 baseline 증거다.
공개 Codex release/tag/commit pin은 2026-10-05 대조한 원본을 유지한다. 비공개
desktop/browser 내부, 새 feature 구현 또는 실제 성능·보안 conformance 검증은 아니다.

이전 29-layer 후보는 인용 함수·인접 호출부를 다시 읽고 MCP lifecycle/discovery와 DOM
request profile 등을 보강한 역사적 검토다. 당시 275개 경로·heading anchor 3개·research
파일 5개 검사는 그 후보의 기록이며, 이후 추가한 문서의 최종 검사로 간주하지 않는다.

현재 후보는 사용성과 모듈 설계를 R01–R36으로 나누고 세 담당의 실제 소스·인접 테스트 본문
검토를 기록했다. 별도 fresh-context 검수자가 보고서의 누락과 영역 간 여정을 다시 확인한다.
발견 반영·수정 후보 판정·미검토 범위는 [전체 검수표](./review-index.md)와 연결된 원보고서에
남긴다. 36개 항목의 배분과 대표 경로 검토를 전체 소스 또는 제품 사용성의 전수 검증으로
표현하지 않는다. 설치·인증·모든 fork/host profile·실제 terminal/browser 등은 한계가 있다.

현재 문서 집합에 대한 최종 검사는 local/pinned source links, 29→36 대응과 R항목별 보고,
heading anchors, untracked 파일을 포함한 whitespace/newline/fence 검사 및
`python3 tools/context.py check`, `git diff --check`로 구성한다. 결과는 전체 검수표에 기록한다.
기존 `CONTRIBUTING.md` 변경은 baseline hash로 보존을 확인한다. Context checker와 링크 존재는
의미나 runtime 보장을 검증하지 않는다. 제품 test suite와 paid provider eval은 재실행하지
않았으며 실제 구현·제품 parity 측정은 단계별 acceptance에 따른 후속 작업이다.
