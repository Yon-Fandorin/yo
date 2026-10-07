# Yo 전체 제품·구조 비판 검수와 우선순위 재정렬

> Status: non-authoritative research/design proposal; 구현·제품 계약 변경 아님
>
> 2026-10-05. Yo `880467b3186ac7ace0111acd37cb1aa334c9dc4c`,
> Pi v1.0.2 `cd32f7725fdbddbaecdff5b1e68491563394e0ca`,
> Codex rust-v0.160.0 `a956835d020762cb2b570053af06f643a11c0ecc`.
> 고정 소스의 비교이며 최신 release 또는 Yo adapter의 지원 버전 선언이 아니다.

이번 판단은 **Yo에 기능이 있다는 이유만으로 제품 과제가 해결됐다고 인정하지 않는다**는
기준을 적용한다. 사용자가 처음 연결하고, 저장소를 이해시켜 작업을 맡기고, 중간에 방향을
바꾸고, 결과를 검증하고, 실패를 복구하고, 다음날 이어 갈 수 있어야 한다.
현재 Yo는 이 전체 여정을 추가 조작 없이 수행할 수 있는 제품이라고 판정하기 어렵다.
첫 진입의 끊어진 안내, 내장 코딩 에이전트의 기본 문맥/검색 부족, 반복 승인과 직렬 실행,
작업 전환·결과 탐색·저장 실패 복구의 불완전성이 연결되어 있다.

이 판단은 모델 task 성공률이나 응답 속도가 Pi/Codex보다 낮다고 측정했다는 뜻이 아니다.
코드에서 확인한 제약, 직접 실행한 격리 CLI probe, 아직 증거가 없는 품질 항목을 구별한다.
정확한 replay, bounded resource, typed evidence 같은 기반은 유용하지만 그것만으로 사용자
여정이 성립하지는 않는다. 기존 타입·파일·화면 배치도 개선을 방해하면 교체 대상이다.

## 1. 검수 기준과 배분

- **부재:** 현재 제공하는 경로/owner에서 필요한 능력이 없음. 검색 범위를 명시한다.
- **불완전:** 기능은 있지만 필요한 사용자 과제를 끝내지 못함.
- **미노출:** 내부 capability가 있어도 실제 CLI/TUI 사용자가 선택·발견하지 못함.
- **불일치:** 표시·기본값·키·실제 effect 또는 draft 소유권이 어긋남.
- **미입증:** 존재하는 테스트/구조로는 실제 속도·코딩 품질·설치·복구를 증명하지 못함.

상위 [36항목 목차](./review-index.md), [64개 TUI 요소와 10개 하위 콘텐츠](./tui-review.md)를
범위 기준으로 사용한다. 등록점 coverage는 제품 completeness의 증거가 아니다.

| 담당 | 범위 | 반박할 전제 | 보고서 |
|---|---|---|---|
| 사용자 여정 담당 | R01–R12, 관련 TUI 전이 | command/API 존재나 CLI fallback이 충분한 UX인가 | [제품 검수](./critical-review-product.md) |
| 독립 상위 모델 구조 담당 | R13–R24 | 기존 모듈 보존·추상화 추가·병렬 slot 증가가 실제 문제를 해결하는가 | [구조 검수](./critical-review-architecture.md) |
| 서비스·운영 담당 | R25–R36 | fail-closed·내구성 표시·unit test 수가 운영 가능한 제품의 증거인가 | [서비스 검수](./critical-review-service.md) |

각 담당은 실제 source와 대표 test body를 확인한다. 루트는 판단과 우선순위를 통합하고,
수정된 최종 후보의 독립 재검수 상태를 마지막에 기록한다. codewright는 구현 slice가 아닌
이번 전체 감사에는 적용하지 않았다.

## 2. 기존 결론에서 바꾸는 것

| 기존 표현이 만들 수 있던 오해 | 이번 결정 |
|---|---|
| 기존 기능이 있으니 발견성만 보강하면 됨 | 실제 과제가 끊기면 동작과 상태 소유권을 바꾼다. queue의 head recall만으로 새 작업 전환을 해결할 수 없고 latest-answer copy만으로 저장 실패를 복구할 수 없다. |
| 작은 owner 변경을 우선하므로 기존 구조는 보존 | 범위가 작은 변경을 선택하되 실패하는 책임 배치는 유지하지 않는다. 일반/질문 editor의 공동 소유, CLI에 묶인 제품 조립, 고정 승인 플래그는 재설계 후보다. |
| instruction snapshot을 넣으면 내장 coding agent 보강 완료 | 새 Session의 버전 있는 기본 지침·실제 도구 사용 규칙·workspace/AGENTS/skill 문맥의 조립과 task 평가가 함께 필요하다. |
| 병렬 scheduler가 승인 중 독립 read를 해결 | 실제 resource claims가 겹치면 여전히 막힌다. 현재 승인 요구 shell이 workspace-exclusive라면 workspace read와 겹친다. typed search·좁은 effect·grant 재사용과 함께 평가한다. |
| 정확한 저장 cutoff 표시와 복사 안내면 recovery 충분 | memory-only suffix·도구 evidence·초안까지 실제로 회수할 경로가 필요하다. durable reader나 최신 답변 하나는 대체재가 아니다. |
| release/CI/eval은 기능 완성 뒤 G에서 추가 | PR test gate·설치 artifact·지원 profile·첫 사용자 과제 평가를 처음부터 병행한다. 결과가 없으면 배포·성능·호환성 완료로 표시하지 않는다. |
| Pi/Codex의 기능이므로 모두 채택 | 해결할 과제가 있는 기능만 채택한다. 두 reference도 모든 approval 대기를 병렬 우회하지 않으며, 일부 resource 표시에서는 Yo가 더 많은 근거를 보존한다. |
| 모든 protocol/control-plane 설계를 먼저 완성해야 GUI 가능 | 지원 범위를 명시한 실제 host와 한 개 consumer의 완결된 여정부터 검증한다. 공개 다중 사용자 서버·모든 UI 동등성을 첫 vertical slice에 넣지 않는다. |

## 3. 전체 36개 영역 재판정과 해야 할 작업

한 행은 독립 기능 수나 전수 실행 검증 건수가 아니다. 실제 근거·반례·한계는 해당 담당 보고서에 있다.
TUI 64개 행은 이전 상세 보고서에서 유지하고 여기서는 사용자 과제의 우선순위와 구조 선택을 바꾼다.

| ID | 현 상태의 핵심 부족 | 다음 작업과 판정 |
|---|---|---|
| R01 설치·첫 실행 | 개발 checkout 실행과 설치 가능한 서비스의 차이; packaged smoke 미입증 | 검증된 설치물·첫 실행 안내·clean-state smoke를 초기 배달 범위로 승격 |
| R02 시작·설정 | target 없으면 실패 후 실행 불가능한 인자 없는 connect 안내; 설정 화면 없음 | Session 이전 setup/config 흐름과 화면 내 복구 신설 |
| R03 인증·계정 | host/native 인증 경로·지원 조건을 사용자가 조합해야 함 | 계정/연결 상태·지원 인증·실패/Refresh/Back을 실제 adapter에 연결 |
| R04 모델 | 검색·현재/예약/default 선택의 발견성 부족; CLI model은 enable/disable | 공통 catalog/query/selection 경험, 선택 실패 때 이전 draft/target 보존 |
| R05 workspace | 경로·실행 host·저장소 문맥을 알고 들어가야 함 | 작업 위치/범위 표시와 instruction discovery의 기준을 동일 workspace에 고정 |
| R06 입력 | rich draft 일부는 견고하지만 marker undo/외부 편집/개행 fallback에 혼선 | plain text와 rich payload의 소유권 정리; 실제 terminal 입력 과제 검증 |
| R07 작업 전환 | queue가 남으면 새 대화/재개 차단, 빈 editor로 head recall만 가능 | 항목별 inspect/edit/remove 및 현재 draft와 함께 명시적 정리·이동 |
| R08 승인·질문 | 일반 draft 재분류·secret partial clear·Esc/Tab 의미 불일치 | request별 draft 분리와 실제 action 기반 안내를 우선 수정 |
| R09 진행·대기 | 내부 pending/approval/저장 상태와 사용자가 아는 기다림 원인이 다름 | runtime 근거로 기다리는 대상·다음 행동 표시; 추정 진행률 금지 |
| R10 결과 검증 | 기존 Changes/Output이 focused task·검색·복사와 완결되지 않음 | 선택 tool→해당 output/diff→원문 복사→원위치, 결과 단위 탐색 |
| R11 재개·분기 | UUID/시각 중심, 제목·내용 검색·rename·페이지 부족 | 내용 중심 Session catalog, 현재 Session 생성 없이 읽기/재개 |
| R12 터미널 | Shift+Enter 협상/대체, reduced motion 노출, detached reflow와 실환경 근거 부족 | 입력 capability와 사용자 설정 연결, 실제 terminal별 수락 matrix |
| R13 조립 | print도 TUI-shaped PreparedAgent/LiveOptions를 거쳐 Session을 꺼냄 | 기존 print를 neutral consumer로 사용해 즉시 조립 결과 분리; 새 host crate/GUI를 기다리지 않음 |
| R14 client/service | 현재 local handle은 remote-neutral API가 아님 | local token/host-owned lifetime을 보존하는 좁은 client contract와 실제 consumer |
| R15 runtime/backend | 상태·effect·복구 책임이 얽힌 owner의 변경 난도 | private lifecycle 상태와 request/tool/context coordinator 분리; replay authority 유지 |
| R16 provider/model | exact profile 설정·지원 조건과 user catalog 간극 | profile 설명/검사/선택을 제품 기능으로 제공; adapter별 지원을 명시 |
| R17 connector/요청 | 계약·usage 검사는 있으나 실제 오류복구/품질/latency 비교 미입증 | bounded request lifecycle, unknown delivery 구분, source별 usage와 measured eval |
| R18 지침·resources | 한 문장 기본 prompt와 자동 project instruction 조립 공백 | 버전 있는 제품 prompt+bounded AGENTS/skill snapshot, no-tools/재개 negative tests |
| R19 context/compaction | exact replay 기반은 있으나 실제 장기 task 품질 평가 부족 | counted=sent=recorded를 유지하며 loss/요약 후 과제 지속성 검증 |
| R20 기본 도구 | native registry에 typed file/content search 부재; shell 승인으로 우회 | grep/find 성격의 bounded typed search, 결과 continuation과 agent guidance |
| R21 권한/sandbox | 고정 approval이며 일반 tool 입력은 shell string; structured grant가 연결될 실제 입력 경로 부재 | versioned structured process tool을 선택해 effect/enforcement/grant/철회와 연결; 기존 shell 경계 유지 |
| R22 병렬성 | single tool/approval state의 head-of-line blocking | bounded scheduler+충돌 순서; 실제 shipping tool에서 줄어드는 대기와 synthetic overlap을 구별 |
| R23 MCP/확장 | delegated 표시와 native 연결·관리의 차이; 좁은 profile의 실제 유용성 미입증 | 유용한 실제 server/tool inventory 하나를 호환 수락 대상으로 고정; unsupported schema를 숨기지 않음 |
| R24 Goal/child | ActivityPlan 표시는 native plan mutation/Goal/budget와 다름 | 작은 native plan owner를 먼저 구현하고 hard Goal budget·child funding과 분리; 표시만으로 성공 선언 금지 |
| R25 저장 | cutoff 공개가 있어도 아직 memory-only인 내용을 구조적으로 구조할 경로 부족 | live rescue export와 저장/읽기/손상 복구 흐름을 초기 기능으로 완성 |
| R26 기억 | replay/skills와 장기 memory가 혼동될 수 있음 | workspace opt-in recall/inspect/edit/off/forget, contribution과 stale 검증 |
| R27 개인정보·수명 | secret entry 관리와 대화 전체 보존/삭제는 다름 | Session archive/delete/backup·용량 관리; lineage/live reference 검증과 preview |
| R28 첨부/artifacts | 원문 보존과 사용자가 inspect/copy/export하는 경로가 다름 | source identity·부분 결과·remote 경계를 유지하는 사용 가능한 결과 회수 |
| R29 host/remote | SSH 안 TUI 실행과 host 연결/재접속 서비스는 다름 | 한 controller의 attach→작업→끊김→receipt 확인→복구 여정부터 구현 |
| R30 GUI/web/mobile | production consumer 부재, terminal HTML은 앱이 아님 | 하나의 지원 frontend로 같은 host 여정을 검증; 화면별 지원/미지원 행동 명시 |
| R31 오류·진단 | 진단 문자열이 있어도 사용자가 복구를 끝낼 수 없는 상태 | 원인→가능한 행동→성공 여부를 연결; live/durable/unknown 결과 구별 |
| R32 테스트/eval | 수많은 local test와 PR 차단·사용자 성공 증거는 다름 | PR affected tests+consumer journey+고정 task corpus를 초기 rollout에 편입 |
| R33 배포·호환 | 수동 compile workflow와 capability별 host version gates가 분리 | 실제 지원 manifest·패키지 smoke·upgrade/resume/compatible rollback 검증 |
| R34 문서/workflow | 긴 기능 설명·CLI 탈출이 사용자의 기억 부담을 덜지 못함 | 설치→첫 task quickstart, 화면별 도움말, 실제 승인 원인과 작업 절차 구별 |
| R35 성능·용량 | bounded scheduling/capacity는 응답속도/한도회복의 증거가 아님 | input→frame·tool/model wait·approval count·메모리·저장 cap 복구를 측정 |
| R36 전체 연결 | 개별 기능/등록점 coverage가 실제 end-to-end 실패를 가림 | 아래 대표 여정마다 완결 oracle와 누락/미지원/실패 분류 유지 |

## 4. 가장 먼저 배달할 결과

### K0 · 첫 작업과 의도·데이터 보존

다음 네 흐름은 서로 독립적인 부분부터 병렬 구현한다. 전체 host 추출이나 GUI를 기다리지 않는다.

1. **시작해서 작업 맡기기:** 빈 설정 `yo`/`yo connect`→지원 연결→모델→workspace→첫 입력.
   제품 prompt/AGENTS/typed search를 함께 준비해 ‘접속은 되지만 저장소를 이해하지 못하는’ 결과를 피한다.
2. **잘못된 입력 방지:** 일반 draft와 request draft, secret clear와 recovery forget,
   no-decline/notes/queue 상태의 키 의미를 일치시킨다. 기존 visibility/fresh-frame gate는 유지한다.
3. **반복 승인 줄이기:** typed read/search로 routine 탐색의 shell 의존을 줄이고, 허용 범위와 실제
   enforcement가 연결된 grant를 재사용한다. 첫 경로는 버전 있는 structured process tool로
   선택한다: executable/literal argv/workspace-relative cwd와 host-owned environment/profile을
   실제로 admission한다. 기존 shell string을 파싱해 자동 허용 규칙으로 변환하지 않는다.
   structured argv라도 interpreter 실행은 광범위한 효과를 낼 수 있으므로 이름만으로 read-only로 간주하지 않는다.
4. **저장 실패에서 회수하기:** live retained history와 아직 저장되지 않은 suffix, 읽을 수 있는
   tool evidence, 별도 draft를 명시적으로 export한다. secret/금지 payload는 기존 공개 정책에 따라
   제외하고 누락 범위를 표시한다. diagnostic dump나 재개 가능한 archive로 잘못 표시하지 않는다.

**동시에 시작할 출시 기반:** PR affected tests, 지원 profile manifest, 격리된 설치 artifact smoke,
첫 작업/승인/복구 fixture. CI 결과를 나중에 문서로 덧붙이는 순서로 두지 않는다.

### K1 · 일상 작업의 왕복 완성

- queue 전체를 보며 항목을 수정/삭제하고 현재 초안을 보존한 채 새 주제/세션으로 이동.
- remembered topic으로 이전 Session을 검색·preview·rename·재개; overflow는 페이지/검색으로 해결.
- 실패 tool·파일명·오래된 결과를 찾아 정확한 Output/Changes/source로 이동하고 필요한 범위 복사.
- 현재 workspace 변경과 저장된 tool evidence는 서로 다른 출처로 보여 줌.
- 모델/설정/terminal capability/사용량·권한 상태를 작업 화면에서 확인하고 되돌릴 수 있음.
- 저장 cap을 만났을 때 archive/backup/delete를 reference-aware하게 수행하고 다시 저장.
- native plan을 생성·갱신·이어 가는 작은 owner와 사용자 명시 workspace note의 recall/edit/off/forget을 독립 배달. hard Goal budget/child/자동 memory extraction을 기다리지 않음.

### K2 · 긴 작업·확장·다른 consumer

- 도구 batch의 disjoint work를 병렬 처리하고 effect conflict/출력 순서/취소를 검증.
- hard Goal budget와 parent-owned child tasks, progress, 결과 join 및 실제 변경 검토.
- 명시적 notes/recall의 사용자 제어를 보존하며 수요가 확인된 memory contribution을 보강.
- native MCP connection/registry 관리와 실패/중지/재연결.
- neutral host와 첫 consumer의 attach→task→approval→disconnect→reconcile→resume. 첫 지원 mutation부터 controller generation과 durable intent/receipt subset을 포함하고 WS/browser/artifact만 후속으로 분리.

K2를 한 거대한 출시 조건으로 묶지 않는다. memory·provider·MCP·child·consumer는 각자 필요한
contract만 선행시킨다. 기존 G의 release/CI와 A의 기본 prompt/search는 K0로 앞당긴다.
고급 codemode, public multi-user hosting, 자동 updater, 모든 UI의 동등 지원은 별도 수요와
검증 근거로 선택한다. 단순히 어렵다는 이유로 기본 사용 과제를 뒤로 미루지는 않는다.

## 5. 모듈 설계에서 교체할 것과 보존할 것

| 결정 | owner와 변경 범위 | 피해야 할 우회 |
|---|---|---|
| 공동 editor에서 request-bound draft로 재설계 | TUI runner state는 전환/identity, prompt는 rich input, secret editor는 비밀 수명 | 화면 안내만 바꾸고 ordinary text를 질문 답으로 재해석 |
| 제품 prompt 조립 신설 | CLI의 순수 product assembler가 admitted capability와 준비한 workspace/instruction snapshot을 받음. OS 읽기는 startup, count/send/replay는 managed | 재사용 backend 안에서 파일을 읽거나 delegated host에 두 번째 base prompt 주입 |
| 실제 process 입력과 authorization 연결 | versioned structured process tool→host의 실행 identity/effect→exact scope/grant/revision→enforcement→scheduler claims. fixed command wrapper만 지원하는 대안은 자동화 범위를 그 wrapper로 명시 제한 | grant DB만 만들고 일반 shell은 계속 매번 승인시키면서 개선 완료 선언 |
| ToolBatch로 serial state 교체 | managed 내부 coordinator와 기존 host execution port; semantic publication order 유지 | 승인 대기 shell과 충돌하는 read를 '독립'이라 표시해 실행 |
| 결과 navigation과 원문 회수 완성 | retained content owner와 view selection, live rescue exporter, durable reader를 구별 | renderer가 파일/네트워크를 임의 읽거나 `/copy`를 백업이라고 표시 |
| neutral 조립과 consumer 경계 추출 | CLI의 TUI-shaped PreparedAgent를 neutral execution 결과+frontend extras로 교체. 이미 존재하는 print가 Session을 직접 소비 | GUI를 기다리거나 public setter/범용 event bus/거대한 host crate만 먼저 추가 |
| shared lock 안의 확장 대기 제거 | worker-owned staged preparation→epoch/turn 재검증→commit. 새로운 permit/child/memory wait를 admission mutex 안에 추가하지 않음 | 기존 race 보호를 없애는 단순 unlock; 근거 없는 현재 deadlock 단정 |
| dispatch mechanics 통합 | ordinary/summary/protected 요청이 공통 private gate에서 permit을 소비하고 stream owner를 얻음. 요청별 전제와 secret receipt는 분리 유지 | 사후 accepted event로 hard budget 소급 검사; generalized billing framework |
| plan과 Goal/child 분리 | standalone native Plan은 각 Session worker/Journal이 소유하고 ActivityPlan으로 투영. parent Goal/task link/budget는 별도 parent owner | 표시된 plan step 완료를 검증 성공이나 Goal 완료로 해석 |
| 보존할 불변조건 | exact identity/replay, known vs unknown effect, typed provenance, 공개 frame 수락, bounded resource, secret 폐기/공개 정책 | 기존 모듈 배치와 불변조건을 같은 것으로 취급해 필요한 구조 변경 거절 |

이 표의 동작 변경은 관련 authority delta를 구현 단계에서 다룬다. 현재 권위 문서를
조용히 바꾸거나, 연구 문서의 제안을 이미 제공하는 기능처럼 도움말에 추가하지 않는다.

## 6. 실제 사용자 과제로 판정하기

각 과제에서 시작 상태·model/profile·tool/permission budget·workspace fixture·기대 결과를 고정한다.
같은 모델을 쓸 수 없으면 모델 품질과 제품 UX를 분리해 비교한다. 시간/승인 횟수는 실행값만
기록하고 이번 정적 검토의 예상치를 성능 측정값으로 쓰지 않는다.

| 과제 | 완료 oracle | 확인할 비용/실패 |
|---|---|---|
| 처음 설치해 간단한 repo bugfix | 지원 연결로 시작, AGENTS 준수, 대상 파일 탐색·수정·회귀 확인 | 사용자 명령/파일 편집 수, 인증 우회, task 성공률 |
| 관련 파일 여러 개 조사 후 테스트 | typed search/read 사용, 필요한 test 실행과 정확한 결과 | routine approval 수·tool/model rounds·대기 원인 |
| 승인 하나 대기 중 독립 작업 | 실제 claims가 disjoint인 admitted 작업만 전진 | source-order conflict, 권한 확대, synthetic fixture와 실제 제품 사례 구분 |
| 작업 중 초안+대기 지시를 정리해 새 주제로 전환 | 선택한 항목만 이동/삭제, 나머지 draft/첨부 보존 | 덮어쓰기·원치 않은 질문 답변·중복 전송 0 |
| 20개 tool 중 실패 원인·관련 변경 검토 | 정확한 item·retained limit·Proposed/Recorded/current workspace 출처 판별 | 키/명령 수, 검색 실패, 잘못된 source copy |
| 저장 실패 중 결과 구조→재시작 | 공개 가능한 memory-only 결과/초안 회수, export 누락/재개 가능 여부 정확 표시 | silent loss·secret export·저장 성공 오표시 0 |
| 저장 한도·개인정보 정리 | 필요한 기록 backup/삭제, live/lineage 참조 영향 표시, 새 저장 가능 | dangling continuation, 삭제된 데이터 재등장, 용량회복 실패 |
| 업그레이드 후 기존 작업 이어가기 | 지원 profile·schema로 정확히 재개하거나 mutation 전에 명확히 거절 | adapter gate 불일치, 기존 state 오염, rollback 과장 |
| 한글·SSH/tmux·좁은 terminal | 의도한 개행/중단·copy·원위치 복귀 | 조기 submit·draft 손실, input→frame latency, terminal restore |
| 긴 작업·다음날·다른 consumer | compaction/기억/child/재접속 중 필요한 근거·ownership 보존 | stale/unknown 재실행, 잊힌 scope, parent 부재 처리 |

위 oracle는 새 개선안의 종료 기준이며 이번에 모두 실행한 결과가 아니다.
기존 unit tests가 불편한 동작을 그대로 assert하는 경우, 테스트를 통과한다는 사실을 개선 거절
근거로 사용하지 않는다. 의미를 바꾸는 계약과 테스트를 함께 갱신한다.

## 7. 이번 실행 근거

현재 source에서 `cargo build --offline -p yo-cli --bin yo`를 bounded helper로 수행해 성공했다.
`YO_CONFIG={}` 파일, 별도 `XDG_STATE_HOME`/`YO_SESSION_REPOSITORY`를 `/tmp`에 두고
아래 명령을 실행했다. 실제 사용자 설정/자격증명을 사용하지 않았고 provider task는 호출하지 않았다.
stdin/stdout은 pipe였다. interactive setup 전체를 PTY에서 시험했다는 뜻이 아니다.

| 명령 | 실제 결과 | 판정 |
|---|---|---|
| `yo` | exit 1, `no startup target is selected`, `yo connect` 안내 | 첫 실행에서 작업에 진입하지 못함 |
| `yo connect` | exit 2, `<TARGET\|--from <PATH>>` 필수 | 바로 앞 안내를 그대로 실행해도 연결하지 못함 |
| `yo model` | exit 2, enable/disable 하위 명령 안내 | 사용자가 기대하는 시작 전 model picker가 아님 |
| `yo session` | exit 0, stdout/stderr 모두 비어 있음 | 빈 catalog의 이유/다음 행동이 표시되지 않음 |
| `yo --help` | exit 0, account/connect/disconnect/default/model/session/usage | 등록 grammar 확인이며 전체 UX 또는 network/host 지원 검증 아님 |

원본 build summary와 probe JSON은 `/tmp/yo-challenge-gpdwrw7_/{build,cli-probes}.json`이다.
build log SHA-256은 `ba1243ea0af964a76b08be22d62882bb251f2a320e62163939c665c1fa856f36`.
[기존 TUI 검수](./tui-review.md)의 1040 unit + 4 parity pass는 동일 제품 소스의 이전 실행
근거로 재사용한다. 이번 새 제안의 수락시험으로 재계산하지 않는다.
제품 코드/테스트/Methexis 계약은 이번 감사에서 수정하지 않았다.

## 8. 최종 검수 상태와 남은 증거

세 담당이 R01–R12/R13–R24/R25–R36을 나눠 source와 대표 test를 검토했고,
독립 상위 모델 검수자가 통합 후 최종 수정본까지 재검수했다.
[최종 판정](./critical-review-final.md)은 **검토한 문서 delta에서 미해결 중요 지적 없음**이다.
중간 지적 4건은 첫 local service의 controller/intent 선행, standalone Plan의 Session 소유권,
잘못 연결된 managed source 링크, 화면 표시 설정과 모델 실행 설정의 구분이며 모두 정정·재확인했다.
제품의 부족함이 해소되었다는 판정은 아니다.

문서 검증은 전체 연구 문서의 local/pinned-source 링크·whitespace·fence, 새 R01–R36의
12/12/12 담당 대응과 각 9/8/9개 지적, 제품 보고서의 T01–T64 재배정,
기존 64+10 TUI 목차/17 command/5 view/15 module 및 29 layer 연결을 확인했다.
`python3 tools/context.py check`, `git diff --check`도 통과했다.
기존 `CONTRIBUTING.md`는 이번 작업 baseline과 byte 단위로 동일하다.
링크의 존재는 source 의미를 증명하지 않으며 실제 owner 오연결은 독립 검수에서 따로 수정했다.

실제 모델 coding benchmark, 실기기 SSH/tmux·screenreader, native OS sandbox enforcement,
packaged install/upgrade, browser/remote consumer 여정은 이 감사에서 미실행이다.
지원하지 않는 기능은 제안으로, 존재를 확인하지 못한 기능은 확인 범위와 함께 기록한다.
