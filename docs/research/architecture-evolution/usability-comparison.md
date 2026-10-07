# Pi·Codex를 기준으로 한 Yo 사용성 개선안

> Status: non-authoritative product/design proposal; 구현 완료 아님
>
> 2026-10-05. [고정한 Yo/Pi/Codex 원본](./evidence.md)을 비교한다.
> Pi v1.0.2, Codex rust-v0.160.0은 참고 기준이며 Yo delegated host의 검증 버전을 확대하지 않는다.

목표는 **사용자가 `yo`로 들어와 연결하고, 작업을 맡기고, 중간에 수정하고, 결과를 확인하고,
다음날 같은 일을 쉽게 이어 가는 것**이다. Pi의 짧은 시작·입력 흐름과 Codex의 인증·재개·대기
상태 표현을 기준으로 삼는다. 현재 Yo의 기능 수나 내부 구조를 기준으로 사용성을 평가하지 않는다.

이번 검토는 소스에서 확인한 경로·표시·키 동작과 로컬 빌드의 `yo --help` 관찰이다.
인증·실제 모델 작업·SSH 시각 검증이나 사용자 실험은 수행하지 않았다. 아래 마찰 진단은
정적 근거에 따른 판단이고 완료 시간·승인 감소율은 아직 측정값이 없다.

후속 [TUI 내부 64개 요소 검수](./tui-review.md)는 입력/요청·콘텐츠/탐색·화면/터미널을
별도로 대조하고 기존 TUI 테스트 1,040개와 작은 rendering parity 4개를 실행했다.
U0b에는 일반/질문 draft 소유권 분리, secret의 실제 전체 clear, no-decline·no-notes·
active queue 상태의 action/힌트 일치를 먼저 포함한다. 테스트 통과와 사용자 과제 검증은 구별한다.

후속 [전체 비판 검수](./critical-review.md)에서 기능 존재 중심의 판단을 재검토했다.
queue는 preview뿐 아니라 paused 항목의 편집/삭제와 주제 전환, Output은 원문 보존뿐 아니라
실패 검색·selected source copy, 저장 실패는 정확한 경고뿐 아니라 live rescue export가 필요하다.
제품 prompt/search·grant의 실제 실행 경로와 설치/CI/지원 profile도 초기 과제로 승격한다.

## 1. 먼저 해결할 사용자 문제

| 여정 | Pi·Codex의 기준 | Yo에서 확인한 마찰 | 우선 결정 |
|---|---|---|---|
| 처음 실행 | Pi는 TUI의 login→model→task, Codex는 화면 내 auth onboarding | target 없으면 오류와 `yo connect` 안내. 그런데 현재 인자 없는 connect는 parser가 거절 | P0: Session 생성 전 setup 화면. `yo`와 인자 없는 `yo connect`에서 연결을 끝내기 |
| 모델 선택 | Pi의 검색 가능한 picker와 이번 선택/default 구분, Codex의 model picker | `/model`은 있으나 CLI `yo model`은 enable/disable만 지원. 시작 전 선택 진입이 어려움 | P0: setup/`yo model`/`/model`의 공통 선택 경험 |
| 실행 중 수정 | Pi의 steer/follow-up 구분, Codex의 pending/queued preview | Enter는 현재 Turn에 steer하지만 active footer에는 그 의미가 없음 | P0: 입력창 옆에 Enter의 현재 의미, 전달 상태, 다음 작업 preview 표시 |
| 승인 판단 | Codex의 이유·실행 환경·선택지·긴 내용 상세 | 기존 선택/변경 상세는 있으나 일률적인 `Esc decline`은 실제 Stop turn과 다를 수 있음 | P0: 대상·효과·위치·범위·현재 가능한 행동을 일치시킴 |
| 결과 확인 | 짧은 실행 결과에서 출력·diff로 진입 | Changes/Alt+D와 `/output`은 이미 있으나 주로 단축키·focused item을 알아야 함 | P0: 변경/실패 요약에서 기존 상세로 직접 진입. P1: 기존 세션 변경 목록을 완료 요약에 연결 |
| 다음날 재개 | Pi/Codex는 검색 가능한 대화 목록, 폴더 범위와 대화 내용 표시 | UUID+수정시간 중심. 현재 Session이 있어야 `/resume` 진입; 64개 초과는 CLI 명령 안내 | P0: 실행 없이 대화 검색·미리보기·재개, bounded 다음 페이지 |

P0는 현재 TUI에서 먼저 완료한다. Host crate 추출, Tasks sidebar, GUI가 이 작업의 선행 조건이
아니다. 새 화면을 추가하는 것보다 기존 기능까지 가는 절차와 사용자가 외워야 하는 정보를 줄인다.

## 2. 시작·연결·모델 선택

현재 빈 설정의 경로는 `yo → target 없음 → yo connect 안내 → TARGET 누락 오류`다.
이것은 취향 차이가 아니라 안내와 실제 명령의 불일치다.
[startup](../../../crates/yo-cli/src/execution/model/startup.rs),
[connect parser](../../../crates/yo-cli/src/command/connect.rs),
[model command](../../../crates/yo-cli/src/command/model.rs),
[live startup](../../../crates/yo-cli/src/application/runtime/live/generation.rs)가 근거다.
[승인된 model selection 계약](../../../methexis/knowledge/agent-runtime/agent.model.session-selection.md)도
이미 빈 interactive startup의 setup, 인자 없는 connect, 통합 model picker를 요구한다.

제안 흐름:

```text
yo
  유효한 기존 default → 바로 대화 입력창
  선택한 연결에 문제 → 그 연결의 상태와 복구 행동
  선택한 연결 없음 → 시작 화면

시작 화면: 이 컴퓨터 · 현재 프로젝트
  [연결해서 시작]  [이전 대화]  [설정]
    → Codex 연결 / Grok 연결 / API key 연결 / 고급 가져오기
    → 필요한 인증과 지원 모델 선택
    → 이 연결로 시작
    → 대화 입력창
```

시작 화면은 model/backend/Session이 없어도 동작한다. 지원되는 일반 연결에서는 YAML이나
`provider:account:model` 좌표를 직접 입력하지 않는다. Exact identity는 선택 행의 내부 값이고
기본 화면은 읽기 쉬운 모델명, 계정 구분, 실행 host, 검증된 capability를 보여준다.
선택한 target의 오류를 숨겨 다른 모델로 자동 연결하지 않는다.

Codex/Grok은 기존 host 인증 경로를 사용한다. 초기에는 `로그인 열기` 또는 정확한 명령 안내와
`다시 확인`으로 충분하다. 미설치·미로그인·미지원 버전을 각각 표시하고 실패 후 같은 단계로
돌아온다. Browser 없는 SSH에서는 실제 host가 지원하는 device/auth 경로를 안내한다.
Yo가 provider에 없는 OAuth나 device login을 제공한다고 표시하지 않는다.

이번 대화의 모델과 저장 default는 다른 선택이다. 계약상 첫 연결이 default가 되는 경우도
완료 화면에서 알리고, 이후 연결이 기존 default를 바꾸지 않도록 한다. Private replay 등 실제로
필요한 최초 동의는 해당 단계에서 한 번 설명한다. 완료 뒤에는 provider/profile/registry 용어를
다시 해석해야 입력창에 도달하는 절차를 만들지 않는다. 등록 완료와 모델 요청 성공도 구분한다.

설정 파일 자체가 손상되면 backend를 시작하지 않고 실행 전 진단 경로를 제공한다.
문제가 있는 파일·필드·원인과 `편집해서 수정 / 다시 읽기 / 종료`를 안내하고 secret 값은
표시하지 않는다. 유효한 설정으로 조용히 대체하지 않는다. 이 오류 화면은 실패한 설정을
다시 파싱해야만 열리는 setup에 의존해서는 안 된다.

작업 중 모델 선택은 현재 실행에 적용된 모델과 다음 Turn에 예약된 모델을 구별해 표시한다.
새 후보가 첨부/참조/현재 기록을 지원하지 않으면 적용 전에 이유를 보여주고 기존 선택과
초안을 유지한다. 선택 성공처럼 먼저 header를 바꾸거나 이미 준비한 이미지를 버리지 않는다.

Non-interactive는 wizard를 열지 않고 기존 typed failure와 유효한 명령을 제공한다. 제품
quickstart는 검증된 설치 경로→`yo`→연결→첫 요청→다시 열기로 시작한다. 현재 긴 기능·preview
설명은 상세 문서로 연결한다. 배포 artifact 검증 전에 존재하지 않는 설치 명령을 게시하지 않는다.

참고: Pi [login/model 구현](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/interactive-mode.ts),
Codex [onboarding auth](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/onboarding/auth.rs).

## 3. 작업 중 입력·진행·승인

한 화면에서 사용자가 알아야 할 순서는 현재 작업 상태, 작성 중인 내용, Enter의 의미와 중단,
다음 작업, 부가 정보다. Context 비율·모델의 긴 식별자·view 탭이 기본 행동을 밀어내면 안 된다.
상시 sidebar를 먼저 추가하지 않고 다음 기본 화면부터 검증한다.

```text
api · devbox · 선택한 모델
작업 중 · 파일 2개 읽는 중 · 승인 1건 대기

[짧은 진행 설명과 실제 결과]
[실패·승인·변경이 있으면 해당 항목과 상세 행동]

다음 작업 1개: “회귀 테스트도 추가해줘”   Alt+R 수정
> 현재 처리하는 오류의 빈 입력 경우도 확인해줘
Enter 현재 작업에 전달 · Alt+Q 다음 작업 · Esc 중단
```

이 문구는 설명용이며 제품 표시 언어는 별도 일관된 정책을 따른다. `파일 2개` 같은 숫자는
실제 Activity에서만 만들고 예상 단계/완료율을 꾸며내지 않는다.

| 상황 | 기본 표시와 동작 |
|---|---|
| idle | Enter 보내기. 초안은 실제 acceptance 전까지 보존 |
| 실행 중, steer 가능 | Enter 현재 작업에 전달. `전달 중`과 `전달됨`을 구분 |
| steer 미지원 확인됨 | 제출 전에 미지원 표시. 명시적인 Alt+Q 다음 작업 선택 제공 |
| queue 있음 | 내용 preview·개수·paused 상태를 입력창 위 별도 영역에 표시. 기본 Enter/중단 힌트 유지 |
| interrupt | 중단 요청 중→중단됨. 남은 초안과 paused queue를 보이고 수정 제출/재개 행동 제공 |
| approval | 실제 대상·효과·실행 위치·범위와 offered choices. Esc 거절 또는 작업 중단을 실제 결과에 맞게 표시 |
| 질문 | 본문·선택·notes와 제출 상태를 함께 유지. 전체 수를 알 때만 진행 번호 표시; Previous는 제공될 때만 노출 |
| 새 출력 도착 중 기록 읽기 | 읽던 위치 보존, 새 출력 표시, 한 행동으로 latest 복귀 |

P0에서 단축키를 Pi/Codex와 똑같이 재배치하지 않는다. Yo의 Enter/Alt+Q/Esc 의미와
참조·첨부·초안 보존 계약을 유지하면서 **무엇이 일어나는지 미리 보이게** 한다.
Pi처럼 queue와 draft를 한 문자열로 합치는 복원은 Yo의 메시지/첨부 identity를 손상할 수 있다.
P1 queue editor는 기존 draft를 별도 보관한 채 항목 선택·수정·삭제·재개를 지원한다.
여기서 보존 대상은 일반 Chat/질문/notes 초안이다. Secret은 별도의 보호 수명을 따르며
거절·Previous 등에서 값을 비우는 기존 규칙을 유지한다. 비밀 값을 일반 draft로 복구하지 않는다.

반복 승인은 UI 문구와 실행 권한을 나눠 해결한다. 이미 허용된 typed read는 재질문하지 않고,
승인한 정확한 scope 안의 반복 명령은 grant 단계에서 재사용한다. Scope 확대는 요청 사유와
달라진 효과를 보여준다. 권한을 넓히는 선택을 숨겨 승인 횟수만 줄이지 않는다. Approval 대기는
실행 slot을 점유하지 않지만, 앞선 opaque shell의 충돌 claim을 후속 read가 넘어서지는 않는다.
개발 workflow의 에이전트 검토 승인과 제품 runtime tool 승인은 측정에서도 별도로 센다.
저장한 규칙은 제안 `/permissions`에서 범위·유효기간·상태를 보고 철회할 수 있게 한다.
Tool 상세에서 자동 허용된 이유와 매칭한 규칙을 확인하고, 철회 뒤 queued call도 새 검증을
거친다. 일반 규칙 편집기로 권한을 조용히 확대하지 않으며 controller 권한 회수와 구분한다.

근거: Yo [footer](../../../crates/yo-tui/src/shell/chrome/help.rs),
[input/queue](../../../crates/yo-tui/src/runner/state/input.rs),
[requests](../../../crates/yo-tui/src/runner/state/requests.rs),
[control](../../../crates/yo-tui/src/input/control.rs);
Pi [keybindings](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/keybindings.ts),
[pending/steer/follow-up UI](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/interactive-mode.ts);
Codex [composer](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/chat_composer.rs),
[pending preview](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/pending_input_preview.rs),
[approval](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/approval_overlay.rs),
[question](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/bottom_pane/request_user_input/mod.rs).

## 4. 변경·출력·결과 확인

Yo에는 이미 Changes/Alt+D, proposed-file review, retained Output과 `/output`이 있다.
[help](../../../crates/yo-tui/src/command/help.rs)에 표시된 현재 기능을 새 기능으로 계획하지 않는다.
문제는 작업 흐름 안에서 상세를 발견하고 돌아오는 비용이다.

- P0: file-change 또는 실패 summary 자체에서 해당 Changes/Output으로 진입한다. 복귀 시
  원래 scroll/focus/Chat draft를 유지한다. 긴 명령/출력은 생략 여부와 완전한 retained 내용의
  위치를 표시한다. 보존 한도를 넘은 내용은 전체 출력이라고 부르지 않는다.
- P0: 결과 설명에서 `변경 보고`, `명령 성공/실패`, `검증 여부`를 구분한다. Shell exit 0을
  전체 테스트 통과로 바꾸지 않고, 테스트인지 모르는 명령은 그대로 표시한다. 모델 설명만으로
  검증 badge를 만들지 않는다.
- P1: 이미 retained changes를 모으는 [세션 변경 보기](../../../crates/yo-tui/src/runner/view/changes.rs)를
  완료 요약과 연결해 파일별 상세로 이동한다. 저장된 before/after
  evidence, 현재 관측 파일, Git working tree diff는 다른 관점으로 명시한다. 기존 dirty 변경을
  Yo가 만든 변경처럼 표시하지 않는다. Git 비교가 필요하면 명시적인 read-only 조회로 제공한다.
- P1: 반복 read/search를 간결하게 묶되 실패·승인·파일 변경은 바로 찾을 수 있게 남긴다.
  요청 추적/프로토콜 상세는 기본 대화에서 별도 진단 보기로 이동한다.

첫 개선에 Revert/commit/push 자동화를 넣지 않는다. 이미 있는 inspect 흐름의 발견성과 증거
표시를 고친 뒤 실제 쓰기 capability를 별도로 검토한다. 이 구분은 사용자가 무엇이 바뀌었고
무엇이 확인됐는지 정확히 판단하게 하기 위한 UI 요구다.

## 5. 이전 작업 찾기·재개·실패 복구

현재 [resume projection](../../../crates/yo-cli/src/application/runtime/live/presentation.rs)은
현재 workspace의 항목을 제한해 전달하며 [picker](../../../crates/yo-tui/src/runner/session/continuation.rs)는
UUID/수정시간을 중심으로 표시한다. Pi [session selector](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/components/session-selector.ts)와
Codex [resume picker](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/resume_picker.rs),
[transcript preview](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/resume_picker_transcript_preview.rs)를
기준으로 다음 흐름을 제안한다.

```text
yo resume                 # 새 제안: Session을 만들지 않고 picker
yo resume ID              # 새 제안: 직접 재개
yo --resume ID            # 기존 문법 유지
yo --continue             # 기존 현재 workspace 최신 대화 이어가기 유지

검색: 토큰 파서
현재 폴더 / 전체
  토큰 파서 오류 수정     어제 · api · 모델명
  빈 입력 회귀 테스트    3일 전 · api · 모델명
오른쪽/아래: 선택한 대화의 bounded 최근 내용과 재개 가능 상태
Enter 재개 · 기록 열기 · 다음 결과
```

`yo resume` 문법은 새 CLI 제안이다. 기존 `/resume`과 공통 catalog를 쓰되 Session/default/auth가
없어도 목록과 읽기 전용 기록을 연다. 제목은 사용자가 지정한 이름이나 첫 일반 요청의 안전한
짧은 발췌를 사용한다. 자동 제목 생성만을 위한 모델 호출은 하지 않는다. Secret/private payload를
색인·미리보기·제목에 넣지 않고, UUID는 상세/복사 기능에 남긴다.

검색은 bounded pagination과 검색 범위를 표시하며 다음 결과를 같은 화면에서 연다. 전체
프로젝트는 해당 host의 허용된 저장소 범위이며 다른 컴퓨터의 세션을 자동 수집하지 않는다.
다른 workspace를 선택하면 실제 실행 위치를 명시하고 저장된 workspace에서만 재개한다.

재개 실패 화면은 이유와 가능한 다음 행동을 함께 제공한다. `연결 복구`, `기록 열기`,
`새 대화 시작` 중 실제 가능한 행동만 표시한다. 새 대화는 이전 실행을 재개한 것으로 표시하지
않고 초안이나 문맥을 몰래 복사하지 않는다. 후보 준비 실패는 현재 대화/초안을 보존한다.
Fork는 기존 의미와 eligibility를 그대로 유지하고 P1에서 선택 지점과 상속 범위를 쉽게 보여준다.

## 6. 실행 우선순위와 코드 소유자

| 순서 | 사용자에게 달라지는 점 | 주 변경 위치 | 의존성 |
|---|---|---|---|
| U0a / P0 | 처음 실행해서 연결·모델 선택을 완료 | CLI startup/connect/model, TUI setup/picker; 기존 model selection service | host 추출·GUI 불필요. 승인된 setup 계약과 현재 구현 정합성 복구 |
| U0b / P0 | 일반/질문 draft 분리·secret 전체 clear·Enter/Esc/Tab 실제 action 일치 | TUI chrome/input/requests/presentation | 기존 capability와 acceptance 상태 사용; runtime grant와 독립 |
| U0c / P0 | 이름/내용으로 대화 찾기, 결과에서 Changes/Output으로 이동 | CLI session catalog, TUI resume/detail navigation | 새 CLI grammar와 bounded catalog projection delta. 현재 Session 없이 읽기 |
| A/B/C | versioned 제품 prompt/지침/search, structured process+grant, 독립 read batch | CLI 순수 product assembler/core/managed/tool owners | opaque shell conflict는 유지. synthetic disjoint approval과 실제 승인 감소를 별도 측정 |
| S0 / P0–P1 | live rescue export·저장 용량/수명 관리·설치/지원 profile·자동 PR/배포물 검사 | release/repository/export/preflight owners | 현재 TUI에서 독립 시작. G/MCP/GUI 완료 불필요 |
| U1 / P1 | queue edit/delete·주제 전환, 이름/search/page/설정, 실패 Output 검색/copy·current Git | 기존 TUI projections와 catalog/workspace service | 각 실제 소유권/행동 계약 선행; U0 전체 완료를 기다리는 일괄 gate 아님 |
| D/E / P2 | 같은 여정을 GUI/web에서 수행 | 제안 host client/service + DOM | 기능 갯수 대신 아래 같은 사용 과제를 통과 |

U0는 거대한 묶음 승인이나 순차 gate가 아니다. 서로 다른 owner의 작은 수정부터 병행할 수 있다.
첫 묶음은 U0의 setup/의도 보존, A의 실제 coding context/search, C의 반복 승인 경로,
S0의 실패 회수·출시 기반이다. 힌트만 바꾸어 해결할 수 없는 소유권/전이 문제는 구조를 바꾼다.
새로운 직접 workspace inspection과 설정·queue 관리도 별도 사용자 과제로 검증한다.

## 7. Pi·Codex와 같은 과제로 검증

아래는 **제안 수용 기준**이며 이번에 통과한 테스트가 아니다. 같은 terminal 크기와 입력·출력
fixture에서 비교하고, 모델/실행 capability 차이를 기록한다. UI fixture와 실제 host 여정을
구분한다. 인증 실험은 테스트 계정·명시한 환경에서 하며 model 품질을 UI 성능과 섞지 않는다.

| 과제 | Yo 목표 |
|---|---|
| 빈 설정에서 지원 연결로 첫 요청 준비 | YAML 편집·canonical target 입력·문서 왕복 없이 setup→입력창. 취소/실패 후 같은 단계 복귀 |
| 실행 중 수정 지시를 보내고 다음 작업 예약 | help 없이 Enter 의미 파악, 현재 전달/다음 작업 구분, silent queue fallback 0건 |
| 승인 중 긴 명령 확인·거절/중단 | 상세 진입/복귀 각각 한 행동. 표시된 Esc 결과와 실제 결과 일치 |
| 같은 허용 scope에서 routine read/명령 반복 | typed read 불필요 승인 0회; grant가 구현된 scope에서 같은 허가 재질문 0회. 범위 확대는 별도 |
| 저장한 허용 규칙을 확인하고 철회 | matched grant 상세→철회→queued call 재검증→restart 후 철회 유지. 범위를 다시 넓히면 새 명시적 승인 |
| 작성 중 질문을 받고 답한 뒤 돌아오기 | 일반 Chat draft와 답변/notes 손실 0건. 다른 질문에 제출 0건. Secret은 보호 수명·의도적인 값 제거 규칙 준수 |
| 파일·skill·template·이미지로 입력 구성 후 외부 편집기 왕복 | 참조 범위/첨부 대상이 표시되고 취소·실패·모델 변경에도 초안 보존. 전송 전 준비와 실제 제출을 구별 |
| 작업 중 모델 변경과 첨부 capability 충돌 | 현재/다음 모델 구분, 미지원 후보 거절 뒤 기존 모델·이미지·초안 보존 |
| 손상된 config로 시작 | Session/모델 호출 없이 정확한 오류·수정/다시 읽기 경로 제공. 비밀 노출·조용한 default 교체 없음 |
| 두 queued messages 중 하나 수정 | U1에서 임시 clipboard/재입력 없이 현재 draft와 첨부 보존, 의도하지 않은 queue 재개 0건 |
| 10,000줄 로그의 실패 확인 | summary에서 한 행동으로 해당 retained output 진입, 복귀 시 읽던 위치/초안 보존 |
| 결과의 파일 변경 확인 | summary에서 해당 상세 접근. saved evidence/current file/Git 상태를 혼동시키는 표시 0건 |
| 많은 대화 중 어제 작업 찾기 | UUID 복사 없이 검색·미리보기·재개. 65번째 결과도 같은 화면의 다음 페이지에서 접근 |
| 중단/재개 실패/연결 상실 | 실제 종료·대기·읽기 전용 상태와 다음 행동 식별, 초안 보존, 자동 중복 실행 0건 |
| 모델 첫 응답 대기·조용한 도구·승인·compaction·취소 정리 | 실제 상태를 구분하고 가능한 행동 표시. 자동 retry가 없는 backend에 retry countdown을 만들지 않음 |
| 사라진 workspace·느린 대화 검색 | 재개 실패 뒤 기록 열기 제공, 늦게 도착한 이전 검색 결과가 현재 선택을 덮지 않음 |
| durability gap | direct memory-only와 durable-required service의 실제 admission 상태를 다르게 표시. 허용되지 않은 실행을 계속 가능하다고 안내하지 않음 |
| service ledger 용량 도달 | 상태 조회와 exact 중단/거절이 가능. 종료 확인→명시적 rotation→재초기화 뒤 새 작업, old 요청 자동 재전송 0건 |

기록할 값은 과제 완료율, 첫 올바른 행동까지 시간, 잘못된 제출/키 입력, help 호출, 화면 밖
명령·문서 왕복, 승인 횟수, 초안/첨부 손실, 중단·복구 성공 여부다. 현재 Yo baseline과 Pi/Codex
reference를 먼저 측정하고 이후 같은 조건으로 후보를 비교한다. 임의로 “30% 개선” 같은 목표를
성능 근거처럼 쓰지 않는다. Backend 지연과 사람의 판단 대기는 UI 처리 지연과 별도 기록한다.
성능 baseline은 cold startup→first paint, 입력→첫 visible acknowledgement, 첫 model output을
분리한다. Slow reader·대량 출력에서는 queue bytes/RSS/retained-output 증가와 취소 응답을
함께 관측하며 설정된 보존 한도와 실제 사용량을 혼동하지 않는다.

80×24, 120×36과 좁은 40×12, CJK/multiline, SSH/tmux에서 핵심 행동 접근성을 확인한다.
좁은 화면에서 모든 정보를 동시에 보여줄 필요는 없지만 대상·선택지·상세 보기·복귀 경로가
사라지면 실패다. 새 출력은 읽던 위치를 빼앗지 않고 paste는 의도치 않게 submit하지 않아야 한다.
Enhanced keyboard가 없는 환경도 실제 명령/선택 메뉴로 핵심 행동에 도달해야 한다.
Mono에서 상태·focus가 구분되고 종료/오류 뒤 terminal 복구를 확인한다. Ignored SSH/tmux test나
golden 파일의 존재만으로 해당 환경을 통과했다고 보고하지 않는다.

팀은 시작/연결, 작업 중 입력, 결과/재개 여정을 각각 원본과 대조했다. 위 우선순위는 새 화면
갯수보다 현재 TUI의 사용 과제 완성을 앞세운 통합 판단이다. 모듈 설계는
[삼자 모듈 비교](./module-comparison.md), 전체 runtime 제약은 [통합 설계](./README.md)를 따른다.
