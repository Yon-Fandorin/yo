# 전체 작업 목차와 검수표

> Status: non-authoritative design work index
>
> 2026-10-05. 대상은 Yo의 사용성과 모듈 설계를 Pi v1.0.2·Codex rust-v0.160.0에
> 대조한 개선안이다. [원본 pin](./evidence.md), [통합 설계](./README.md).

이 목차는 이번 검토의 범위를 고정한다. 특정 발견이 눈에 띈다는 이유로 다른 영역을 생략하지
않고, 각 항목을 **현재 사용자 여정 → 삼자 소스 대조 → Yo의 격차 → 개선할 동작/소유자 →
실패·복구 → 검증 과제** 순서로 확인한다. 기능이 있다는 사실만으로 사용성이 좋거나 모듈
경계가 적절하다고 판정하지 않는다.

현재 문서는 설계 검수 대상이다. `검수 완료`는 해당 범위의 설계·소스 검토를 뜻하며, 구현·
성능·실제 인증·OS 격리·SSH 픽셀 검증의 완료와 구별한다. 모든 코드 줄을 읽었다는 주장도 아니다.

TUI 내부는 별도 [64개 요소 검수표](./tui-review.md)로 더 세분화했다. R06–R12를 중심으로
입력/요청·콘텐츠/탐색·화면/터미널 담당을 배분하고, command/view/module에서 역방향으로
누락을 확인한다. 요청 도착 시 초안 소유권, secret clear, action/힌트 불일치를 U0b 첫 작업으로
추가한다. 기존 36개 항목 검수와 새 TUI 상세 검수의 증거·미실행 범위는 각 문서에 구분한다.

후속 [전체 제품·구조 비판 검수](./critical-review.md)는 R01–R36을 다시 배분해 현재 기능의
존재가 사용자 과제를 해결하는지 반박했다. 해당 문서의 K0–K2와 implementation의 S0가
최신 배달 우선순위다. 기존 보고서의 ‘설계상 해결’은 기능 구현/제품 수용 완료를 뜻하지 않는다.

## 1. 작업 목차

### I. 처음 사용하고 작업에 들어가기

1. **R01 설치·첫 실행**: 실제 배포 경로, 지원 OS/terminal, 설치 오류, 첫 입력까지의 단계.
2. **R02 시작 화면·설정**: 빈 설정, 기존 default, 잘못된 설정, TTY/non-interactive 분기.
3. **R03 인증·계정**: 기존 host login, API key, account 구분, SSH 인증, 실패·취소·재연결.
4. **R04 모델 선택·변경**: 검색/표시, 현재/default 구분, capability, busy switch, 실패 시 보존.
5. **R05 프로젝트·실행 위치**: cwd/workspace/host 표시, 참조·경로 해석, 다른 프로젝트 선택.
6. **R06 입력·편집·참조**: multiline/paste/CJK, 파일·skill·template, 이미지, 외부 편집기, 초안.

### II. 작업 중 개입하고 결과 확인하기

7. **R07 steer·queue·interrupt**: 입력 의미, acceptance, 다음 작업 편집, 취소 정리와 재시작.
8. **R08 승인·질문·비밀 입력**: 대상/효과/범위, 긴 내용, 선택·notes·Previous, 지원 불가 화면.
9. **R09 진행·대기·상태**: 모델/도구/승인/compaction 구분, 실제 수치, foreground/background.
10. **R10 출력·변경·검증 결과**: existing Output/Changes, 실패 원인, saved evidence/current Git 구분.
11. **R11 대화 탐색·재개·분기**: 제목/검색/페이지/preview, 기록만 열기, lineage, 복구 행동.
12. **R12 터미널 사용성·접근성**: inline/fullscreen, focus/scroll, narrow/mono, SSH/tmux, 키 대체.

### III. 실행 구조와 모델 문맥

13. **R13 조립·설정·의존 방향**: neutral startup, public/private API, concrete factory, 수명·cleanup.
14. **R14 client·service·protocol**: local/remote 공통 동작, attachment 수명, pending/unknown, optional build.
15. **R15 Session·Runtime·Engine·backend**: 상태 authority, worker, loop ownership, delegated compatibility.
16. **R16 provider·catalog·capability**: account/model inventory, exact binding, discovery/auth 관측, 확장 축.
17. **R17 connector·stream·request dispatch**: dialect/transport 경계, stream 실패, retries, accounting/cache/permit.
18. **R18 프로젝트 지침·리소스**: AGENTS/skills/templates, discovery/snapshot, scope/우선순위·예산.

### IV. 도구와 에이전틱 실행

19. **R19 context·compaction·replay**: request assembly, active/idle/protected 경로, 손실·정확 재개.
20. **R20 tool 정의·registry·실행**: typed search/files/command, frozen schema, progress/cancel/cleanup.
21. **R21 permission·grant·sandbox**: 매 호출 권한과 재사용, revoke, OS enforcement, 권한 확대 UX.
22. **R22 병렬 scheduler·resource claims**: 승인 HOL, 충돌 순서, window/용량, publication/retire owner.
23. **R23 MCP·확장·codemode lifecycle**: 설치/연결/discovery, 시작 전 효과, 제한·취소, drift, unsupported 응답과 codemode 보류 경계.
24. **R24 Goal·Plan·child tasks**: budget, delegation, worktree, 결과 join, parent 부재·실패·복구.

### V. 데이터·연결·다중 화면

25. **R25 Journal·저장·continuation**: sole writer, codec/storage 경계, durability, crash/recovery, 호환성.
26. **R26 장기 기억**: recall/contribution, scope, 정확한 입력 snapshot, off/forget/CAS/index와 UI.
27. **R27 credential·개인 정보**: secret 수명·저장/로그, 제목/search/preview, export·history 경계.
28. **R28 첨부·artifact·파일 evidence**: 입력/출력, 제한·provenance·currentness, URL/clipboard/remote 경계.
29. **R29 host·remote·reconnect**: auth/controller, wire evolution, gap/suffix, backpressure, shutdown.
30. **R30 GUI·웹·desktop·mobile**: 동일 사용자 여정, DOM 접근성, onboarding, unsupported handoff, 배포.

### VI. 품질·운영·전체 연결

31. **R31 오류·진단·관측**: error taxonomy, 복구 행동, timing/trace, 외부 효과 불명, 사용자 표시.
32. **R32 테스트·평가·CI**: unit/consumer/PTY/integration, 실제 여정, task oracle, required-job 결과.
33. **R33 배포·업데이트·호환성**: packaged install, upgrade/rollback, schema/host/client 버전, state 보존.
34. **R34 도움말·문서·개발 workflow**: quickstart/발견성, 유효 명령, authority, 불필요한 재승인.
35. **R35 속도·용량·자원**: startup/first visible progress, output growth, memory/queue budget, 느린 소비자.
36. **R36 영역 간 연결·누락 감사**: 끝까지 이어지는 여정, 문서 간 모순, 미검토 범위, 검수자의 누락.

## 2. 작업 항목·소유자·검수 배분

한 행은 위 목차의 구현/설계 작업이다. A는 사용자 여정, B는 실행·모듈, C는 데이터·운영을
맡는다. 각 담당은 자기 행의 Pi/Codex/Yo 근거와 현재 설계의 대응 위치를 보고한다. 발견이
없어도 읽은 경로와 확인한 불변조건을 남긴다. 소스 미확인은 `미검토`, 기능 부재는 `미구현`,
의도적인 범위 제외는 이유가 있는 `보류`로 구분한다.

| ID | 구체적으로 보강할 작업 | 주 소유자 / 단계 | 설계 연결 | 담당 |
|---|---|---|---|---|
| R01 | 검증된 설치→실행 quickstart와 package smoke 연결 | release/CLI · U0/G | usability §2, README §11 | A |
| R02 | Session 이전 setup, value-less connect, 실패 화면 내 복구 | CLI startup/TUI · U0a | usability §2 | A |
| R03 | 지원 인증의 선택/Back/Refresh, 계정·host 상태 표현 | CLI connection/provider · U0a | usability §2, README §10 | A |
| R04 | 공통 model picker와 선택/default/disabled/실패 의미 | model service/TUI · U0a | usability §2, README §8·10 | A |
| R05 | 실제 host/workspace 표시와 탐색 범위·재개 위치 | CLI/workspace/TUI · U0 | usability §2·5, README §8 | A |
| R06 | 편집·참조·첨부의 준비/전송/취소와 초안 보존 | TUI input/host inputs · U0/U1 | usability §3, README §4·9 | A |
| R07 | Enter 의미, queue preview/edit, 취소와 next action | TUI input/core admission · U0b/U1 | usability §3 | A |
| R08 | capability에 맞는 힌트, request draft, 정확한 scope 표시 | TUI requests/core request · U0b/C | usability §3, README §5·9 | A |
| R09 | 실제 진행/대기 원인·취소 중 표시와 연결된 상세 | TUI projections · U0b | usability §3 | A |
| R10 | 완료 요약→existing Changes/Output, evidence 구분 | TUI views/tool evidence · U0c/U1 | usability §4, README §9 | A |
| R11 | 이름/검색/preview/page, session 없이 기록·복구 | CLI catalog/TUI · U0c/U1 | usability §5 | A |
| R12 | 좁은 화면의 기본 행동, terminal restore/SSH 검증 | TUI/process shell · U0/U1 | usability §7, README §9 | A |
| R13 | Config/PreparedAgent 분리, private factory 주입 | CLI→host assembly · D | module §1·5·6 | B |
| R14 | neutral client API, local token/remote unknown 분리 | host client/service · D | module §2·5 | B |
| R15 | private 상태 전이, actor ownership과 호환 adapter 유지 | core/backend · B/F/D | module §3, README §3·6 | B |
| R16 | provider/connector 확장 축 유지, capability-admitted 선택 | provider/core model · G | module §4, README §10 | B |
| R17 | 공통 request admission/dispatch, protected receipt/permit | managed/connector · F1b/G | module §3·4 | B |
| R18 | bounded instruction/resource snapshot과 의미 분리 | host discovery/managed · A | README §4 | B |
| R19 | context count/dispatch/replay 정합성, checkpoint ownership | managed/core Journal · A/F | module §3, README §4·6 | B |
| R20 | schema 유지, typed search, 실행 수명과 renderer 분리 | core tool/host adapters · A/B/G | module §4, README §5 | B |
| R21 | scoped grant와 launch capability, 실제 confinement | core permission/host adapter · C | README §5 | B |
| R22 | slot enum, ordered proposal/barrier/retire, claims | managed/host coordinator · B | module §3·4, README §5 | B |
| R23 | pre-spawn profile, bounded discovery, lifetime cleanup | host MCP/registry · G | README §5 | B |
| R24 | standalone native plan mutation/projection, Goal reservation, fresh child seed와 parent/child join | core agentic/host tasks · F1a/F1b/F2 | README §6 | B |
| R25 | semantic codec adapter와 storage 경계·복구 점검 | core Journal/repository · 관련 schema 변경 | module §3, README §6 | C |
| R26 | host CRUD, worker snapshot admission, off/forget race | host memory/core input · F3 | module §4, README §7 | C |
| R27 | secret route와 public projection/검색의 범위 검증 | core secret/host/frontend · D/F/G | README §7·8·9, usability §5 | C |
| R28 | host artifact identity, bounded input/evidence, UI attribution | host inputs/artifacts/frontend · D/E | README §8·9 | C |
| R29 | attachment/controller/intent/suffix/transport bounds | host service/client/server · D | module §2·5, README §8 | C |
| R30 | 기본 사용자 여정 재사용, 실제 capability별 UI | DOM/desktop/host client · E | README §9, usability §6·7 | C |
| R31 | failure→실제 가능한 행동, unknown과 rejected 구분 | 각 owner+frontend error mapping · U0/D/G | usability §3·5, README §10·11 | C |
| R32 | 단위 검증과 실제 작업 과제·평가 연결 | owner tests/CI · U0/A/G | implementation, usability §7 | C |
| R33 | 설치 package/실제 외부 consumer/호환 upgrade | release/host adapters · G/H | README §11, implementation | C |
| R34 | quickstart/help/source authority와 재승인 구분 | product docs/CONTRIBUTING · U0 | usability §2, README §12 | C |
| R35 | latency/pressure baseline과 첫 excess·취소 검증 | runtime/host/TUI · B/D/G | README §5·8·11, usability §7 | C |
| R36 | 전체 목차↔근거↔설계↔검증의 빠진 연결 확인 | 통합 작성자+별도 누락 검수자 | 이 문서와 전체 문서 | C+독립 감사 |

## 3. 공통 검수 질문

각 담당은 다음을 모두 적용한다. 해당 없는 항목은 이유를 적고 넘어간다.

1. **현황 정확성**: 기존 구현을 없는 기능으로 쓰거나 제안/테스트 fixture를 완성 기능으로 쓰지 않았는가?
2. **삼자 비교**: Pi/Codex의 실제 대응 모듈·사용 흐름을 봤는가? 대응 기능이 없으면 비교 불가를 명시했는가?
3. **모듈 설계**: public/private API, mutable owner, dependency 방향, 생성/drop, 변경 파급이 명확한가?
4. **사용성**: 진입·주 행동·상태 표시·취소·복귀가 이어지는가? 사용자가 내부 좌표나 명령을 추측해야 하는가?
5. **경계 조건**: 빈 상태, stale, 첫 용량 초과, unsupported, 실패/unknown, 동시성, restart를 다뤘는가?
6. **검증 가능성**: 실제 before/after와 대표 사용자 과제 또는 consumer test가 있는가? 아직 실행하지 않은 증거를 통과라 쓰지 않았는가?

## 4. 검수자 누락을 확인하는 방법

일차 검수 이후 독립적인 fresh-context 검수자에게 **목차와 세 담당의 결과 및 수정 후보**를
전달한다. 다음 항목을 직접 교차 점검하게 한다.

- R01–R36 모두 근거·판단·검증 계획이 있는지, `검토함`만 있고 실제 읽은 경로가 없는 행은 없는지.
- A의 UX 요구가 B/C의 API·상태·저장·권한에서 구현 가능한지, B의 구조 변경이 사용성 개선을
  불필요하게 막는지, C의 운영 제약이 UI에서 복구 불가능한 막다른 길을 만드는지.
- Setup→model→input→approval→tool→결과→resume 여정과 remote disconnect→reconnect 여정이
  담당 경계에서 끊기는지. Task budget→child→memory→restart도 같은 방식으로 연결 확인.
- 기존 29-layer 표와 새 36항목 사이에 사라진 영역이 있는지. 신규 발견은 R항목과 설계 owner에
  연결하고 원 담당의 보고서만 믿지 않고 필요한 원본을 다시 읽는지.
- 해결된 발견과 미검토/보류/실행 검증 대기를 구별했는지. 최종 후보에서 material finding을
  반영하고 영향을 받은 범위를 다시 검수했는지.

Root는 보고서의 개수로 완료를 판정하지 않는다. 발견을 평가해 작은 유효 변경을 채택하고,
중복·과도한 제안은 이유와 함께 보류한다. 문서 링크 검사는 이 의미 검수를 대체하지 않는다.

### 이전 29개 레이어와 새 목차의 대응

기존 검토 영역이 목차 재구성 중 사라지지 않았는지 확인하는 역방향 색인이다.
한 레이어가 여러 사용자 여정·모듈에 걸치면 모두 표시한다.

| 기존 layer | 이번 검수 항목 |
|---|---|
| 1 product entry/install | R01, R02, R33 |
| 2 composition | R13 |
| 3 identity/workspace | R05, R16 |
| 4 semantic engine | R15 |
| 5 managed loop | R15, R17, R19 |
| 6 providers/connectors | R04, R16, R17 |
| 7 instruction/context | R06, R18, R19 |
| 8 accounting/cache | R17, R19, R24, R35 |
| 9 basic tools | R20 |
| 10 tool authorization | R08, R21 |
| 11 execution isolation | R21, R23 |
| 12 concurrency | R22, R35 |
| 13 extension/MCP | R23 |
| 14 codemode | R23; H 단계의 명시적 보류 |
| 15 steering/questions | R07, R08 |
| 16 plan/goal | R24 |
| 17 multi-agent/task | R24 |
| 18 compaction | R19 |
| 19 long-term memory | R26 |
| 20 persistence/recovery | R25 |
| 21 portability | R19, R25, R33 |
| 22 TUI | R06–R12 |
| 23 GUI | R30 |
| 24 browser/remote | R14, R29, R30 |
| 25 multimodal/artifact | R06, R28 |
| 26 diagnostics/telemetry | R09, R31, R35 |
| 27 testing/evals | R32 |
| 28 CI/release | R01, R32, R33 |
| 29 docs/workflow | R34 |

## 5. 검수 현황

| 검수 | 범위 | 상태 |
|---|---|---|
| 기존 레이어 재검수 | [29-layer](./layer-review.md) | 이전 후보 완료; 새 목차의 대체 근거로 자동 승계하지 않음 |
| 모듈 비교 재검수 | composition/client/state의 삼자 대조 | 완료; public token/replay 문구 정정 반영 |
| 사용성 기초 조사 | 시작/입력/결과·재개 | 완료; 전체 목차 검수와 별개 |
| A 전 영역 검수 | R01–R12 | [보고서](./review-a-user-journeys.md) 완료. 일부 소스 심화 검토·실환경 검증은 명시적으로 남김 |
| B 전 영역 검수 | R13–R24 | [보고서](./review-b-execution.md)와 B1–B3 수정 후 재검토 완료 |
| C 전 영역 검수 | R25–R36 | [보고서](./review-c-data-operations.md)와 C1–C3 수정 후 재검토 완료 |
| 독립 누락 감사 | A/B/C 보고와 전체 목차·수정 후보 | [독립 감사 보고서](./review-gap-audit.md) 및 수정 후보 재검토 완료. 검토한 설계 범위에서 남은 material finding 없음 |
| 후속 TUI 내부 상세 검수 | 64개 상위 요소·10개 콘텐츠 하위 요소 | [소스 검수·독립 최종 재검수](./tui-review.md) 완료. 기존 TUI 1040 + parity 4 pass; 제품 수정과 실기기 검증은 별도 |
| 전체 비판 재검수 | R01–R36 재판정·기존 기능의 제품 과제와 구조 대안 | [최종 후보 재검수](./critical-review-final.md) 완료. S0/K0–K2 우선순위와 구현 목차 재정렬; 격리 CLI 진입 실패 재현, 제품 개선은 미구현 |

### 발견 반영 상태

| 발견 | 연결 항목 | 처리 |
|---|---|---|
| A2/A4/A6: 손상 config 복구, busy model/첨부, 입력 도구 왕복 | R02/R04/R06 | usability §2·7 반영, A가 재확인 |
| A5/A8/A9/A12: workspace/stale 검색, secret 예외, 상태 구분, terminal 대안 | R05/R08/R09/R11/R12 | A 보고 이후 usability §3·7에 반영; 독립 감사자가 수정 후 재확인 |
| B1: MCP 연결·실패·중지 관리 흐름 | R23 | README §5와 implementation 관리 과제 반영, B 재확인 |
| B2: Goal/Task 사용자 행동과 worker 제어면 | R24 | README §6·8과 implementation revision/opID 과제 반영, B 재확인 |
| B3: cache provenance와 warming/prewarm 구분 | R17/R35 | README §10, warming H 보류와 accounting 과제 반영, B 재확인 |
| C1: 임시 upload 총량·소유권·정리 | R28/R35 | README §8, self-contained bytes 보존과 ACK-loss/restart 과제 반영, C 재확인 |
| C2: direct/service의 durability Gap 안내 불일치 | R31/R36 | README §11, 실제 admission 상태별 UI 과제 반영, C 재확인 |
| C3: first paint/ack/slow-reader baseline 누락 | R35 | README §11와 usability §7 반영, C 재확인 |
| G1: ledger 용량 도달 뒤 stop·회복 경로 | R07/R08/R14/R29/R31/R35 | exact urgent/maintenance 공간 선예약, quiescent epoch rotation, retired replay 거절·재초기화; 독립 감사 재확인 |
| G2: 저장한 grant 확인·철회 경로 | R08/R21/R29/R34 | `/permissions`, matched provenance, revision-bound durable revoke와 queued call/restart 과제; 독립 감사 재확인 |
| G3: host 전체 connection/attachment/queue 용량 | R29/R35 | aggregate reservation과 첫 초과 시 기존 controller/urgent 보존; 독립 감사 재확인 |
| G4: 과거 검사 수치와 현재 검토 범위 혼동 | R34/R36 | 이전 275-link/5-file 기록을 역사적 후보로 구분, 36항목의 부분 검토·실행 대기 범위를 보존; 독립 감사 재확인 |

### 남아 있는 범위와 구현 전 결정

- **부분 소스 검토**: installer 전체, 모든 auth/host protocol/profile, fork/저장 codec 전 경로,
  모든 search 알고리즘과 외부 extension, Pi experimental server 전체, 상용 Codex GUI 내부.
  보고서마다 실제 읽은 범위가 다르므로 36행을 모두 전수 검토로 묶지 않는다.
- **실행 검증 대기**: 아래 실환경 검사와 새 기능별 consumer tests. 현재 계획/기존 test 본문
  검토를 제품 동작 개선의 증거로 대체하지 않는다.
- **D 구현 전 고정**: 임시 upload의 aggregate bytes/count/concurrency와 lease 기간,
  전체 attachments/clients의 finite profile. 무제한으로 구현한 뒤 나중에 제한하는 선택은 없다.
- **의도적 보류**: codemode/warming, GUI Queue/Revert, mutable MCP lifetime, 공유 memory scope,
  public multi-user hosting/self-updater 등은 각 H 조건을 충족할 때 재검토한다.

실행하지 않은 검증: 실제 인증/모델 task benchmark, terminal 실사용 matrix, OS sandbox/MCP 효과,
remote/browser 재접속, packaged 설치/업그레이드. 해당 구현 단계의 수용 조건으로 유지한다.

### 최종 문서 검증

Local 및 고정 Pi/Codex source 하이퍼링크, local heading anchor, R01–R36 목차/배분/보고서
대응과 기존 layer 1–29 대응을 검사했다. Untracked 파일을 포함한 whitespace/final newline/
code fence 검사, `python3 tools/context.py check`, `git diff --check`도 통과했다.
기존 `CONTRIBUTING.md`는 이번 작업 baseline과 byte/hash가 같음을 확인했다.
검사 도구는 경로·문서 형식·목차 대응만 확인하며 의미·실행 성능·보안을 증명하지 않는다.
