# TUI 내부 UI/UX 요소별 검수

> Status: non-authoritative research/design audit
>
> 2026-10-05. [전체 36항목 검수](./review-index.md)의 R06–R12와 관련 경계를
> TUI 내부 요소로 세분화한다. [Pi/Codex source pin](./evidence.md)을 유지한다.

기능 목록뿐 아니라 진입 → 표시 → 조작 → 수락/실패 → 취소/복귀 → resize/재접속의
전이를 검토한다. 현재 구현과 개선 제안, source inspection과 실제 terminal 검증을 구분한다.
초기 60개를 배분한 뒤 module/command/view 역방향 대조에서 4개를 추가했다.
현재 **64개 요소**를 다룬다. 한 요소를 새 모듈 하나로 만드는 설계가 아니라,
실제 상호작용의 소유권과 검증 누락을 찾기 위한 목록이다.

후속 [ANSI 디자인 예시](./tui-ansi/README.md)는 이 64개 요소와 10개 하위 요소를
실제 ESC/TXT/오프라인 HTML로 구체화한다. [화면별 역방향 추적표](./tui-ansi/coverage.md)에서
정지 화면 witness와 별도로 필요한 동적 검증을 구분한다.

## 1. 요소 목록과 담당

### A · 입력과 요청

| ID | 요소 | 확인할 상호작용 | 현재 Yo 출발점 |
|---|---|---|---|
| T01 | 입력창 구조·placeholder | 높이·빈 상태·주 행동·긴 초안 | [source](../../../crates/yo-tui/src/prompt.rs) |
| T02 | 커서·선택·문자 편집 | grapheme/CJK·행·단어 이동 | [source](../../../crates/yo-tui/src/input/editor.rs) |
| T03 | 개행·붙여넣기 | Enter 충돌·bracketed/burst paste | [source](../../../crates/yo-tui/src/input/event.rs) |
| T04 | undo·kill/yank·초안 복원 | 복원 단위·참조/이미지 일관성 | [source](../../../crates/yo-tui/src/input/buffer.rs) |
| T05 | 입력 history | Ctrl+R·검색·취소·복귀 | [source](../../../crates/yo-tui/src/runner/state/input.rs) |
| T06 | 대화 찾기 | Ctrl+F·검색 범위·미확정/늦은 결과 | [source](../../../crates/yo-tui/src/runner/state/input.rs) |
| T07 | 파일 참조 | @ 검색·경로·선택·stale cursor | [source](../../../crates/yo-tui/src/prompt/workspace_reference.rs) |
| T08 | skill 참조 | 동명 source·scope·미지원 | [source](../../../crates/yo-tui/src/prompt/skill_reference.rs) |
| T09 | prompt template | 선택·삽입·literal vs command | [source](../../../crates/yo-tui/src/command/prompt.rs) |
| T10 | 이미지·첨부 입력 | 준비/전송·취소·표시·모델 변경 | [source](../../../crates/yo-tui/src/prompt/image.rs) |
| T11 | 외부 편집기 | 왕복·실패·빈 결과·termios | [source](../../../crates/yo-tui/src/runner/state/external_editor.rs) |
| T12 | Enter·steer | 현재 Turn·capability·acceptance | [source](../../../crates/yo-tui/src/runner/state/input.rs) |
| T13 | queue·편집·재개 | 내용 preview·paused·현재 draft | [source](../../../crates/yo-tui/src/runner/state/input.rs) |
| T14 | interrupt·exit | Esc/Ctrl+C/Ctrl+D·상태별 의미 | [source](../../../crates/yo-tui/src/input/control.rs) |
| T15 | slash palette | 필터·자동완성·노출·dispatch | [source](../../../crates/yo-tui/src/command/palette.rs) |
| T16 | 승인 선택 | 효과·범위·기본값·decline/stop | [source](../../../crates/yo-tui/src/runner/state/requests.rs) |
| T17 | 긴 명령·승인 diff | 완전한 내용·상세·복귀·선택 유지 | [source](../../../crates/yo-tui/src/runner/state/requests.rs) |
| T18 | 질문·choice·notes·Previous | 본문·순서·host capability·seal | [source](../../../crates/yo-tui/src/runner/state/requests.rs) |
| T19 | secret 입력·관리 | 마스킹·수명·의도적 폐기·재입력 | [source](../../../crates/yo-tui/src/input/secret.rs) |
| T20 | 요청 간 전환·초안 | pending queue·일반/질문 draft·late response | [source](../../../crates/yo-tui/src/runner/state/requests.rs) |

### B · 콘텐츠와 탐색

| ID | 요소 | 확인할 상호작용 | 현재 Yo 출발점 |
|---|---|---|---|
| T21 | assistant streaming | 미완료 Markdown·점진 표시·완료 | [source](../../../crates/yo-tui/src/transcript.rs) |
| T22 | tool 실행·접기 | 요약·상세·진행·현재 항목 | [source](../../../crates/yo-tui/src/transcript/activity.rs) |
| T23 | 실패·긴 stdout/stderr | 원인·retained 한도·종료 상태 | [source](../../../crates/yo-tui/src/runner/view/output.rs) |
| T24 | reasoning·compaction | 공개 의미·숨김·대기·실패 | [source](../../../crates/yo-tui/src/transcript/activity.rs) |
| T25 | Markdown 문서 계층 | 제목·목록·인용·강조·가독성 | [source](../../../crates/yo-tui/src/transcript/layout.rs) |
| T26 | 코드·syntax·공백 | 언어·highlight·wrap·source copy | [source](../../../crates/yo-tui/src/transcript/layout.rs) |
| T27 | 표·차트·도식 | 좁은 폭 fallback·원래 값·복잡도 | [source](../../../crates/yo-tui/src/transcript/layout.rs) |
| T28 | 링크·copy·export | 실제 목적지·선택 범위·보안 | [source](../../../crates/yo-tui/src/command/copy.rs) |
| T29 | 본문 이미지·media | fallback·EXIF·crop·source | [source](../../../crates/yo-tui/src/transcript/layout.rs) |
| T30 | 파일 diff 표시 | 추가/삭제·line count·unavailable | [source](../../../crates/yo-tui/src/transcript/layout/activity/tool_files.rs) |
| T31 | Changes view | Proposed/Recorded·파일 이동·복귀 | [source](../../../crates/yo-tui/src/runner/view/changes.rs) |
| T32 | Output view | 항목 이동·paging·위치·truncation | [source](../../../crates/yo-tui/src/runner/view/output.rs) |
| T33 | Transcript view | Chat과 차이·source 의미·진입 | [source](../../../crates/yo-tui/src/runner/view.rs) |
| T34 | Request view | diagnostic와 승인 inbox 구분 | [source](../../../crates/yo-tui/src/runner/view.rs) |
| T35 | 모델 picker | current/reserved/default·disabled·검색 | [source](../../../crates/yo-tui/src/runner/model.rs) |
| T36 | 새 대화·resume picker | 읽기 쉬운 이름·preview·실패·초안 | [source](../../../crates/yo-tui/src/runner/session/continuation.rs) |
| T37 | tree·fork picker | 내용·상속 범위·좌표·eligibility | [source](../../../crates/yo-tui/src/runner/session/continuation.rs) |
| T38 | help·status·usage | 발견성·전체/현재 안내·상세 | [source](../../../crates/yo-tui/src/command/help.rs) |
| T39 | 빈/loading/error/disabled/stale | 행·화면 상태와 가능한 다음 행동 | [source](../../../crates/yo-tui/src/overlay.rs) |
| T40 | 공통 selection panel | filter·순서·번호·preview·가시성 | [source](../../../crates/yo-tui/src/overlay.rs) |

### C · 화면 체계와 터미널

| ID | 요소 | 확인할 상호작용 | 현재 Yo 출발점 |
|---|---|---|---|
| T41 | header·실행 identity | host/workspace/model·중복·폭 우선순위 | [source](../../../crates/yo-tui/src/shell/chrome.rs) |
| T42 | 상태줄·진행·대기 | 실제 상태·timer·저장/전송 단계 | [source](../../../crates/yo-tui/src/shell/chrome.rs) |
| T43 | footer·키 힌트 | 현재 Enter/Esc 의미·우선순위 | [source](../../../crates/yo-tui/src/shell/chrome/help.rs) |
| T44 | 레이아웃·공간 배분 | 최소 폭/높이·prompt/overlay 경쟁 | [source](../../../crates/yo-tui/src/layout.rs) |
| T45 | focus·cursor·공개된 frame | 표시한 선택과 실제 action 일치 | [source](../../../crates/yo-tui/src/runner/state/presentation.rs) |
| T46 | resize·reflow·위치 유지 | 80→24→80·selection/draft/viewport | [source](../../../crates/yo-tui/src/shell.rs) |
| T47 | inline/fullscreen·publication | 과거 화면·현재 frame·전환 | [source](../../../crates/yo-tui/src/terminal.rs) |
| T48 | 스크롤·follow latest | 사용자 위치·새 출력·latest 복귀 | [source](../../../crates/yo-tui/src/transcript.rs) |
| T49 | 항목 접기와 전체 접기 | scope·override·focus·history | [source](../../../crates/yo-tui/src/runner/view.rs) |
| T50 | theme·semantic colors | default/light/mono·대비·선택 | [source](../../../crates/yo-tui/src/appearance.rs) |
| T51 | glyph·Unicode·폭 | ASCII/CJK/emoji/combining·정렬 | [source](../../../crates/yo-tui/src/surface/text.rs) |
| T52 | motion·attention·notification | reduced motion·지원·중복·집중 | [source](../../../crates/yo-tui/src/appearance.rs) |
| T53 | mouse·selection·clipboard | 실제 지원 범위·terminal selection·copy | [source](../../../crates/yo-tui/src/terminal.rs) |
| T54 | 키 protocol·충돌·대체 경로 | enhanced 미지원·Alt/F-key·newline | [source](../../../crates/yo-tui/src/input/view_binding.rs) |
| T55 | 종료·suspend·복귀 | raw mode·cursor·editor·실패 | [source](../../../crates/yo-tui/src/terminal.rs) |
| T56 | SSH/tmux·image protocol | 지원 탐지·passthrough·fallback | [source](../../../crates/yo-tui/src/terminal.rs) |
| T57 | 비시각 접근·읽기/내보내기 | 색 외 의미·plain/HTML·screenreader 한계 | [source](../../../crates/yo-tui/src/plain.rs) |
| T58 | 화면 응답성·용량 | frame rate·idle wake·출력 증가·cancel | [source](../../../crates/yo-tui/src/runner.rs) |
| T59 | custom renderer·document·status | 확장 경계·일관성·source 보존 | [source](../../../crates/yo-tui/src/runner/agent.rs) |
| T60 | 화면 간 전이·목록 누락 감사 | 등록 command·view·module 역방향 대조 | [source](../../../crates/yo-tui/src/lib.rs) |
| T61 | offline preview 화면 | 중첩 fixture·키·진입 전 상태 복원 | [source](../../../crates/yo-tui/src/runner/state/preview.rs) |
| T62 | 수동 compaction | 명령 수락·실행 중 거절·실패·완료 | [source](../../../crates/yo-tui/src/command/compact.rs) |
| T63 | interview 작업 사본 lifecycle | live continue/close·종료 사본 view·discard·일반 draft 복귀 | [source](../../../crates/yo-tui/src/command/interview.rs) |
| T64 | 저장 실패·시작 안내 | History not saved·복구 행동·현재 세션 의미 | [source](../../../crates/yo-tui/src/runner/state/observation.rs) |

### D · 교차 검수로 보강한 콘텐츠 하위목록

64개 상위 요소 외에 다음 **10개 하위 요소**를 명시적으로 추적한다. 기존 기능을 모두
‘도구 출력’으로 축약했던 빈틈이다. [독립 검수의 상세 assertion·Pi/Codex 대조](./tui-review-gap.md)에
읽은 테스트 본문과 미확인 범위를 구분했다. 아래 항목들은 이미 구현되어 있다.

| 하위 ID | 별도로 검수한 표시·상태 | 실제 owner | 보강 방향 |
|---|---|---|---|
| T22.a | 계획 단계·진행 중/대기/완료·빈 plan | [plan/notice](../../../crates/yo-tui/src/transcript/layout/activity.rs) | 원래 단계 순서/실제 상태 보존, 현재 단계 발견성 |
| T22.b | notice/warning·보고된 Turn duration | [plan/notice](../../../crates/yo-tui/src/transcript/layout/activity.rs) | severity·정확한 수치/출처·다음 행동; reasoning 숨김과 분리 |
| T22.c | resource link 카드·URI·unknown metadata·크기 초과 | [resources](../../../crates/yo-tui/src/transcript/layout/activity/tool_resources.rs) | literal URI의 명시적 inspect/copy, 자동 fetch 금지 |
| T22.d | embedded text/blob/image·metadata·잘못된 shape | [resources](../../../crates/yo-tui/src/transcript/layout/activity/tool_resources.rs) | 원문 보존·binary unavailable 의미, 지원하는 source 접근 |
| T22.e | batch file reads·range·empty·개별 실패·이어 읽기 | [files](../../../crates/yo-tui/src/transcript/layout/activity/tool_files.rs) | 파일/범위 선택에서 기존 Output 연결, continuation 설명 유지 |
| T22.f | directory listing·부분 목록·빈/미완전 entry | [resources](../../../crates/yo-tui/src/transcript/layout/activity/tool_resources.rs) | shown count와 전체 개수 구별; partial 공개 |
| T22.g | file/content search·match cap·잘못된 metadata | [tool](../../../crates/yo-tui/src/transcript/layout/activity/tool.rs) | literal path 보존, 실제 metadata에 근거한 다음 조회 안내 |
| T22.h | execution details·host/call/outcome·retained output | [resources](../../../crates/yo-tui/src/transcript/layout/activity/tool_resources.rs) | 알려진 outcome을 Output에도 연결; 원래 필드 보존 |
| T25.a | Proposed plan·host/help/status·terminal input 문서 | [document](../../../crates/yo-tui/src/transcript/layout/activity/document.rs) | draft→final source 교체·failure·복귀 보존 |
| T29.a | audio·unknown media fallback | [tool](../../../crates/yo-tui/src/transcript/layout/activity/tool.rs) | playback unavailable 명시; 새 재생 기능은 별도 결정 |

Resource card/embedded body의 풍부한 정보 보존은 Yo의 강점이다. Codex의 일부 MCP summary가
URI만 남긴다는 이유로 그 축약 방식을 도입하지 않는다. T22.f의 별도 limit test는 존재 확인만,
T29.a는 owner branch만 검토했다. 나머지 대표 assertion 검토도 모든 payload 조합 검증은 아니다.

## 2. 요소마다 확인할 내용

- 현재 Yo 타입/함수·인접 호출·대표 테스트와 Pi/Codex의 대응 요소.
- 기본·빈·loading·disabled·실패·stale·취소 상태와 표시한 행동의 실제 결과.
- 키/선택/focus·초안·첨부·읽던 위치의 소유권과 다른 화면에서 돌아온 뒤 상태.
- 긴 내용·좁은 창·CJK·mono·키 protocol 미지원에서의 의미 보존.
- 보존할 강점, 실제 문제, 구체 개선, 우선순위, 사용자 과제/검증 방법.
- 대응 요소나 실행 근거가 없으면 명시. upstream 기능을 전체 동등성 증거로 확대하지 않음.

## 3. 검수 배분과 증거

| 검수 | 범위 | 상태 |
|---|---|---|
| A | T01–T20 입력/명령/질문/승인 | [소스 검수 완료](./tui-review-input.md) |
| B | T21–T40 콘텐츠/상세/선택/탐색 | [소스 검수 완료](./tui-review-content.md) |
| C | T41–T64 화면 체계/터미널/응답성 및 추가 누락 | [독립 상위 모델 소스 검수 완료](./tui-review-shell.md) |
| 누락·전이 교차 검수 | 세 보고서, command/view/module 역방향 대조, 수정 후보 | [독립 최종 delta 검수 완료](./tui-review-gap.md) |

최종 결과에는 정적 근거와 이번에 실제로 실행한 offline 검사만 분리해서 기록한다.
인증·모델 호출·실제 SSH/tmux·screenreader 결과를 추정하지 않는다.
## 4. 이번에 더 구체화한 문제

우선순위는 이 TUI 작업 안에서의 순서다. P0는 잘못된 값/행동을 제출할 가능성,
P1은 일상 작업의 제어·탐색·읽기 방해, P2는 접근 환경과 확장성 보강이다.
아래 경로는 정적 검토 결과이며, 실제 사용자 사고가 발생했다는 주장이 아니다.

| 우선 | 관찰과 사용자 영향 | 최소 개선 및 수락 조건 | 근거·상세 |
|---|---|---|---|
| P0 | 질문 도착 전 작성한 일반 텍스트 초안이 같은 editor에 남고, Enter가 pending request의 답변으로 라우팅될 수 있음 | 일반/요청별 draft 소유권 분리. 질문 표시 전 초안을 답변으로 자동 이전하지 않음. 답변·취소·Previous·다음 요청 뒤 원래 초안/참조 복원 | [입력 경로](../../../crates/yo-tui/src/runner/state/input.rs), [요청 overlay](../../../crates/yo-tui/src/runner/state/requests.rs), A:T20 |
| P0 | Secret footer의 `Ctrl-U clear`는 실제로 현재 줄 앞부분만 지움. `first\nsecond` 후 Ctrl-U→Enter는 `first\n`을 제출하는 테스트가 존재. 마스킹 때문에 잔존 내용을 알기 어려움 | 비밀 입력의 명시적 전체 지우기와 `Not entered` 상태. 일반 editor의 kill-line 동작은 별도 유지. 저장된 recovery까지 지우는 동작과 구분 | [secret](../../../crates/yo-tui/src/input/secret.rs), [현재 테스트](../../../crates/yo-tui/src/input/secret/tests.rs), A:T19 |
| P0 | decline 없는 승인에서 footer는 `Esc decline`, 실제 dispatch는 Interrupt/Stop turn. notes 없는 choice에서 Tab이 즉시 제출하지만 footer와 맞지 않음 | 요청 capability와 실제 action에서 본문·footer·키 dispatch를 함께 도출. 승인 거절과 Turn 중단, 선택 이동과 제출을 구분 | [footer](../../../crates/yo-tui/src/shell/chrome/help.rs), A:T16/T18 |
| P1 | active+queued에서 queue 후보가 interruption 후보보다 먼저 선택되고 work row의 대체 힌트도 억제됨. 충분한 폭에서도 Esc/^C가 사라질 수 있음 | active interruption을 queue 수량/장식보다 먼저 보존. 폭·높이 조합별 완성된 input stack 검사 | [input-stack 계약](../../../methexis/knowledge/tui-architecture/tui.chrome.input-stack.md), C:T43 |
| P1 | 이미지 marker 삭제 후 undo는 `[image]` 문자열만 복구하고 실제 attachment는 복구하지 않음 | detached marker를 정상 첨부처럼 표시하지 않음. 우선 재첨부 안내 또는 명시적 비첨부 표시; byte 복원을 원하면 별도 bounded lifetime 설계 | [현재 테스트](../../../crates/yo-tui/src/runner/state/image/tests.rs), A:T04/T10 |
| P1 | Changes의 좁은 header 후보에서 Proposed/Recorded가 빠지고 경로만 남을 수 있음 | provenance label을 경로 상세보다 먼저 보존. 승인 전 제안과 저장된 결과를 모든 지원 폭에서 구별 | [Changes](../../../crates/yo-tui/src/runner/view/changes.rs), B:T31 |
| P1 | Output 진입이 항상 최신 retained 항목으로 reset됨. `/copy`는 최근 완료 assistant 전체 source 중심 | focused tool의 Output 직접 열기. 복사할 항목/원문 범위 표시, 부분 출력의 retained 한계 유지 | [Output](../../../crates/yo-tui/src/runner/view/output.rs), [copy](../../../crates/yo-tui/src/command/copy.rs), B:T28/T32 |
| P1 | resume는 UUID/시각·64개 제한 중심, model picker는 검색 없음. F2/F3은 진단 화면이며 사용자가 찾는 결과 원문 화면과 다름 | 내용 검색·preview·bounded paging, model 검색, 진단 화면 이름/설명과 명령 진입. 기존 fork의 입력 excerpt·상속 설명은 보존 | B:T33–T38 |
| P1 | detached Chat scroll은 visual row 기반이라 resize 후 같은 내용을 계속 보는 보장이 약함 | Chat에서 item+source offset anchor로 reflow; Changes/Output의 기존 source anchor를 참고. inline의 이미 발행한 native history는 재작성하지 않음 | C:T46–T48 |
| P1 | 기본 S-Enter 개행을 안내하지만 keyboard enhancement 협상/동등한 CLI fallback이 확인되지 않음 | 지원 프로토콜 협상 또는 검증된 대체 개행 경로를 안내. 미지원 terminal에서 Enter 오제출 방지 여정 검증 | A:T03, C:T54 |
| P2 | reduced motion은 TUI API에 있으나 CLI 조립은 Standard 고정. mouse는 wheel 중심, screenreader·SSH/tmux 실사용은 미검증 | 설정 노출과 상태 안내부터. 텍스트/색 외 의미·대체키 보존, terminal별 실제 검증 후 지원 범위 확정 | C:T50–T58 |

보존할 강점도 분리한다. Grapheme 편집, 명시적 참조 snapshot, 이미지 준비 중 초안 보존,
frame/presentation에 묶인 선택 수락, Changes/Output의 source-anchored paging,
partial/unavailable 공개, inline publication 복구와 terminal restoration은 이미 있다.
Yo의 chart/diagram과 Pi/Codex의 렌더링을 단순 기능 유무로 순위화하지 않는다.
특히 비교한 Codex pin에도 Mermaid preview와 source fallback이 있다.

## 5. 구현 목차와 모듈 소유권

전체 TUI 교체나 64개 새 모듈 분할을 선행 조건으로 두지 않는다. 현재의 input → state →
presentation → shell과 view/source 소유자를 유지하면서 아래 책임을 명확히 한다.
이름은 후보이며 기존 타입을 재사용할 수 있다. 동작 계약 변경이 필요한 항목은 구현 시
관련 authority와 테스트를 같이 갱신한다. 이 연구 문서가 계약을 대체하지 않는다.

| 작업 | 범위 | 소유권과 변경 방향 | 완료 기준 |
|---|---|---|---|
| W1 요청과 일반 초안 분리 | T01/T04/T10/T18–T20/T63 | runner state가 일반 draft와 request identity별 draft의 전환 소유. prompt는 참조/첨부 snapshot 소유, secret editor는 별도 폐기/복구 규칙 유지. 초기에는 Session 내 private 상태로 한정 | 질문 도착→선택/notes→Previous→취소/다음 요청→일반 입력의 왕복에서 내용/참조/첨부 혼선 없음; 늦은 응답이 다른 editor를 지우지 않음 |
| W2 action과 힌트 일치 | T12–T18/T42–T45/T54 | capability별 action projection을 presentation에서 만들고 footer/overlay가 동일 의미를 표시. key dispatch는 기존 typed action과 frame gate 유지 | Enter/Tab/Esc/^C가 표시한 동작만 수행. active+queue·no-decline·no-notes·아주 작은 화면 포함 |
| W3 secret clear/복구 구분 | T19 | SecretEditor 전체 지우기와 recovery forget의 별도 action. 평문 editor kill/yank에 비밀 값을 넣지 않음 | multiline paste/중간 cursor 후 clear가 전체 transient value를 제거; 길이 노출 없이 Not entered. recovery 보관 상태는 별도 정확한 표시 |
| W4 항목에서 결과로 이동 | T22–T34/T49 | runner view가 focused item으로 Output/Changes를 열고 해당 view가 scroll/paging 소유. source copy는 content owner의 실제 source에 연결 | 실패 tool→해당 Output, 파일 변경→해당 Changes→원위치. Proposed/Recorded·partial·unavailable 의미 유지 |
| W5 picker 검색·재개 | T05–T09/T15/T35–T40 | 검색/페이지/IO는 model/catalog/prompt owner, overlay는 typed snapshot/선택만 담당. 모델·세션 id를 표시 이름으로 대체하지 않고 별도 보관 | 이름/본문으로 찾고 선택 의미·disabled 이유를 확인. 페이지/취소/실패 뒤 draft 유지; startup의 Session 없는 기록 진입과 연결 |
| W6 읽던 위치와 입력 편집 | T02–T04/T21/T25–T30/T46–T49 | transcript/source anchor와 view별 viewport 분리. Home/End 편집/스크롤 scope 명시. undo는 text와 attachment 의미를 혼동하지 않음 | 긴 CJK/코드/표에서 80→24→80 resize 후 같은 내용 유지; 첨부 없는 marker를 보낸다고 오인하지 않음 |
| W7 terminal·접근 환경 | T44/T50–T58 | terminal이 keyboard/restore/protocol capability, CLI가 사용자 preference, surface가 width/style 소유. 렌더러에서 host 환경을 추정하지 않음 | reduced motion 실제 CLI 진입, monochrome 의미 보존, fallback newline·tmux/SSH·외부 편집기 실패 복구를 terminal matrix로 검증 |
| W8 보조 명령과 상태 | T38/T59/T61–T64 | preview는 developer example에서만 활성화하는 격리된 로컬 fixture, compact/interview는 기존 typed effect, durability는 실제 host observation에서 투영 | 개발 preview 종료 시 기존 view/draft 복원; 일반 세션의 preview 거절/초안 보존; compact 거절/취소 원인; interview 완료/중단; History not saved에 가능한 후속 행동 표시 |
| W9 회귀·누락 방지 | T39/T40/T45/T58/T60 | pure editor/panel 검사 + state transition + 완성 shell + offline PTY 여정을 구분. registry/view/module의 항목 변경 시 이 inventory delta 확인 | §6 전이 과제와 §7 reverse mapping에 미배정 영역 없음. snapshot 통과만으로 실제 terminal/접근성 완료 선언 금지 |

W1–W3을 U0b의 첫 작업으로, W4/W5를 U0c/U1로, W6/W7을 R12·R35로 연결한다.
서로 다른 소유자의 W3, W4, W5 조사·구현은 독립 진행할 수 있다. W2가 모든 작업을 기다리게
하는 통합 리팩터링이 되어서는 안 된다. shared input/presentation 변경만 명시적으로 조율한다.

## 6. 요소 사이 전이의 수락 시나리오

다음은 **추가 구현 시 실행해야 할 과제**이며 이번 실행으로 통과했다고 주장하지 않는다.
단일 component test 대신 실제 state→완성 frame→입력→dispatch를 연결해 확인한다.

| 시나리오 | 확인할 불변조건 | 관련 항목 |
|---|---|---|
| 일반 초안+참조 작성 중 질문 도착→Enter→취소→원래 입력 | 질문 editor는 새 소유자, 일반 초안은 답으로 자동 전송되지 않음 | T07/T10/T18/T20 |
| multiline 비밀 입력→커서 이동→clear→새 값→submit | 숨겨진 이전 값이 남지 않음; recovery 저장값 삭제는 별도 명시 | T19 |
| 이미지 준비/성공→marker 삭제→undo→send | 실제 bytes와 보이는 attachment 상태 일치 또는 분명한 재첨부 요구 | T04/T10 |
| active+queue→좁게/넓게 resize→Esc/^C | 가능한 geometry에서 중단 힌트 보존, 동일 interrupt intent | T12/T13/T43/T44 |
| no-decline 승인→상세 diff→복귀→Esc | 선택/presentation 보존, Stop turn을 decline이라고 표시하지 않음 | T16/T17/T31/T45 |
| notes 없는 choice→Tab, notes 가능 choice→Tab→Previous | 제출과 이동을 혼동하지 않음; request별 draft/choice 복귀 | T18/T20/T63 |
| find/history/model picker 열기→새 request 도착 | overlay 전환 후 키가 이전 선택을 수락하지 않음; 일반 draft 복원 | T05/T06/T20/T35/T40 |
| 과거 tool focus→Output/Changes→resize→Chat | 선택한 tool evidence와 source 위치 유지; 최신 tool로 임의 이동하지 않음 | T22/T31/T32/T46 |
| Proposed diff 좁히기→Recorded diff로 이동 | 경로 축약보다 provenance 의미 우선 | T30/T31/T44 |
| Chat 과거 긴 코드/표 읽기→새 출력→resize→tail 복귀 | detached 의미 유지, 새 출력 도착 여부 확인; 원문 복사 범위 명확 | T21/T26–T28/T46/T48 |
| 줄바꿈 구분 못 하는 terminal→multiline 작성→submit | 실제 지원키/대체키 안내, 의도치 않은 조기 전송 없음 | T03/T54/T56 |
| developer preview→nested fixture→`/preview` 또는 `/exit`, compact 실패, 저장 실패 | 원래 화면/초안 복원, 각 실패에 가능한 다음 행동, 거짓 저장 표시 없음 | T61–T64 |

## 7. 역방향 누락 대조

행을 많이 만드는 것만으로 완전성을 주장하지 않는다. 현재 source의 등록점에서 역으로
한 요소 이상에 연결한다. 아래는 **등록점 수준**의 coverage이고 모든 분기/플랫폼 실행 coverage가 아니다.

### 7.1 등록된 17개 command

[CommandRegistry](../../../crates/yo-tui/src/command/registry.rs)의 전체 built-in definitions:

| command | 담당 요소 |
|---|---|
| help | T38 |
| model | T35 |
| status | T38/T42 |
| compact | T24/T62 |
| copy | T28 |
| output | T32 |
| preview | T59/T61 |
| attach | T10 |
| exit | T14/T55 |
| find | T06 |
| new | T36 |
| interview | T18/T63 |
| fork | T37 |
| tree | T37 |
| resume | T36 |
| secrets | T19 |
| prompt | T09 |

### 7.2 전용 view 5개

[ObservabilityView](../../../crates/yo-tui/src/runner/view.rs)의 Chat→T21/T48,
Transcript→T33, Request→T34, Changes→T31, Output→T32.
F2/F3 진단을 approval inbox나 원본 message selection으로 오해하지 않는다.
`/preview`는 등록되어도 일반 세션에서 사용할 수 있는 기능이 아니다.
developer `chat_preview` example의 명시적 capability가 필요하다.
`/interview`는 새 인터뷰 생성 명령이 아니라 현재 Session의 작업 사본
continue/close/view/discard 경로다. 질문 자체의 입력/Previous는 T18이 소유한다.

### 7.3 최상위 module family 15개

[lib.rs](../../../crates/yo-tui/src/lib.rs)의 선언을 기준으로 한다.

| module | 담당 요소 |
|---|---|
| appearance | T50–T52 |
| command | T09/T15/T28/T38/T61–T63 |
| html | T28/T57 |
| input | T01–T06/T14/T19/T54 |
| layout | T44/T46 |
| meter | T38/T42 |
| overlay | T15–T20/T35–T40/T45 |
| plain | T28/T57 |
| prompt | T07–T10 |
| runner | T12–T20/T31–T37/T45–T49/T55/T58–T64 |
| shell | T01/T41–T44/T50–T52 |
| surface | T25–T30/T44/T50/T51/T57 |
| terminal | T03/T47/T53–T56/T58 |
| text | T25–T30/T46/T51 |
| transcript | T21–T30/T48/T49/T59 |

CLI preference/host status/durability 조립은 T41/T42/T52/T64에서 경계까지 추적한다.
GUI/remote 서버/실제 provider 인증의 전체 검수는 [기존 전체 목차](./review-index.md)가 소유한다.
Pi의 `!`/`!!` 같은 사용자 직접 shell mode는 별도 상호작용이다. 현재 Yo의 대응 입력 경로는
확인하지 못했으며, 도구 실행/Output으로 동등 기능이라고 세지 않는다. 신규 채택은 별도 결정이고
의도적 미지원이라는 제품 계약까지 확인한 것은 아니다.

## 8. 실행 검증과 한계

2026-10-05, Yo `880467b3186ac7ace0111acd37cb1aa334c9dc4c`의 현재 checkout에서
기존 bounded validation helper로 다음을 순차 실행했다. 제품 코드는 변경하지 않았다.

| 실행 | 결과 | 증명 범위 |
|---|---|---|
| `cargo test --offline -p yo-tui --lib` | 1,040 passed, 0 failed; helper 76초 | 기존 unit/state/layout/terminal 모형 테스트. 신규 제안의 수락 시험은 아님 |
| `cargo test --offline -p yo-tui --test rendering_parity` | 4 passed, 0 failed | 작은 공통 Surface fixture의 terminal operations/ANSI/HTML parity. 전체 TUI 시각 검증은 아님 |

로컬 summary는 `/tmp/yo-tui-audit-1sgrnn5p/{tui-tests,rendering-tests}.json`에 있다.
lib log SHA-256: `7797c95645b11553c0a8a3080b3c3c1a0ff2c3b3dfcf374f42bb21914f24d52e`.
parity log SHA-256: `99b9855f91c1610d035b256657fe0bbe0ff80cefc649390aa87052e3dbf5383b`.
임시 로그는 영구 CI artifact가 아니며 위 명령/결과를 이 문서에 보존한다.

실제 모델·인증·SSH/tmux·screenreader·terminal별 key negotiation·사용자 완료시간/오류율은
이번에 실행하지 않았다. 일반 초안→질문 오답 경로는 호출 추적이며 새 재현 테스트를 추가한
것은 아니다. secret partial clear와 image undo는 기존 테스트가 현재 동작을 명시적으로
확인한다. 테스트 전부 통과는 위 사용성 결함이 없다는 증거가 아니다.

참조를 포함한 답변은 `reject_referenced_answer`에서 차단하는 경로가 있다. T20의 위험을
모든 rich draft가 자동 전송된다고 확대하지 않는다. 초안 분리 설계는 그 기존 차단과
첨부/참조 소유권을 함께 보존해야 한다.

## 9. 최종 검수와 문서 검증

입력/요청, 콘텐츠/탐색, 화면/터미널 세 담당의 보고서를 통합한 뒤 별도 fresh-context
검수자가 누락·전이를 확인했다. 화면/터미널 담당은 독립 상위 모델 검수다. 교차 검수에서
10개 콘텐츠 하위 요소를 보강하고 F2 의미, developer preview, interview 작업 사본,
direct shell 범위, 참조 포함 답변 차단에 대한 과장 가능성을 정정했다. 수정된 최종 문서까지
독립 재검수했으며 **검토한 문서 범위에서 미해결 중요 지적은 없다**. 발견한 제품 문제의
수정 완료나 모든 코드/terminal 조합 검수 완료를 뜻하지 않는다.

문서 검증은 17개 연구 Markdown의 링크·whitespace·final newline·code fence,
T01–T64 및 10개 하위 요소와 상세 보고서 대응, source registry 17 command / 5 view /
15 module의 정확한 집합 일치, 기존 R01–R36 / 29 layer 대응을 포함했다.
`python3 tools/context.py check`, `git diff --check`도 통과했고 기존 `CONTRIBUTING.md`는
이번 작업 baseline과 byte 단위로 같았다. 문서 형식 검사 자체는 의미나 성능의 증거가 아니다.
