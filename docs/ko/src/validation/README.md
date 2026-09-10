# 검증

변경된 경계를 기준으로 증거를 고른다. 기대 동작과 중요한 실패를 구분할
수 있는 가장 작은 검사부터 시작한다. 일반 작업은 영향받는 검사만 사용한다.
아래의 formal 기준선은 명시적으로 선택한
[Slice](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#slice-contract)에만 적용한다.

## 증거 계층

| 계층 | 확인할 수 있는 것 | 예시 |
|---|---|---|
| 프로세스 내부 | 결정론적 상태, protocol, layout, rendering, 주입된 실패 동작 | `yo-core` engine/runtime test, `yo-tui` component test, rendering parity golden |
| 호스트 통합 | 선택 설치 프로그램 없이 실제 호스트 기능을 사용한 동작 | `yo-cli`의 Linux PTY, termios, process signal, 터미널 복원 test |
| 외부 환경 | 설치 프로그램, 인증, 중첩된 터미널 환경과의 호환성 | Codex, Grok, tmux, 로컬 `sshd`, SSH, SSH 내부 tmux |

첫 번째 계층은 빠르게 진단할 수 있지만 OS 터미널 생명주기를 증명하지
못한다. 호스트 통합 계층은 실제 Unix 경계를 실행하지만 모든 터미널
multiplexer나 원격 session을 증명하지 못한다. 외부 환경 계층은 실제로
실행한 환경에 대해서만 그 빈틈을 채운다.

무시되었거나 실행할 수 없는 환경 검사는 passed가 아니라
**unverified**다. assertion을 약하게 만들거나 조용히 건너뛰지 말고 빠진
command, host, credential, platform을 기록한다.

## 변경된 경계에서 시작하기

| 변경 영역 | 처음 실행할 유용한 명령 | 가장 가까운 증거 |
|---|---|---|
| Session, Turn, Activity, engine, runtime 의미 | `cargo test -p yo-core` | `crates/yo-core/src/tests`와 소유 모듈 test |
| typed input span, submission identity, 고정된 v1의 structured-reference 거절 | `cargo test -p yo-core input::tests`와 `cargo test -p yo-core journal::codec` | `crates/yo-core/src/input/tests.rs`와 Journal wire-compatibility test |
| Agent-session admission, concurrency, 시작, 종료 | `cargo test -p yo-core agent_session::tests` | `crates/yo-core/src/agent_session/tests` |
| backend lifecycle, evidence 또는 bounded child-process transport 추출 | `cargo test --locked -p yo-backend` 뒤 `cargo test --locked -p yo-core backend::evidence`와 `cargo test --locked -p yo-core journal::codec::tests::correlation` | `crates/backends/foundation/src`, `yo-core` specialization, Journal wire·recovery 호환성 test |
| Codex protocol 변환이나 provider ID 연결 | `cargo test --locked -p yo-backend-delegated-codex` | `crates/backends/delegated-codex/src/runtime/tests.rs` |
| Grok ACP 변환, permission, 인증, Session 연결 | `cargo test --locked -p yo-backend-delegated-grok` | `crates/backends/delegated-grok/src/runtime/tests.rs`, `observation/tests.rs`, `protocol.rs` |
| 해석된 input, 편집, paste, binding, 종료 gesture | `cargo test -p yo-tui input::` | `yo-tui/src/input` 곁의 test |
| prompt 줄 바꿈, cursor 표시, viewport | `cargo test -p yo-tui prompt::` | `yo-tui/src/prompt` 곁의 test |
| `@` trigger, stale 결과, 선택 치환, local 순위, Git ignore 탐색 | `cargo test -p yo-tui workspace_reference`와 `cargo test -p yo-core workspace_reference` | `yo-tui/src/prompt/workspace_reference.rs`와 `yo-core/src/workspace_reference` |
| `$` trigger, Codex catalog decode, scope filtering, 비활성 행, typed skill 선택 | `cargo test -p yo-tui skill_reference`, `cargo test -p yo-core skill_reference`, `cargo test -p yo-backend-delegated-codex skill_catalog` | `yo-tui/src/prompt/skill_reference`, `yo-core/src/skill_reference`, `backends/delegated-codex/src/skill_catalog.rs` |
| 대화 기록 item, streaming revision, scroll | `cargo test -p yo-tui transcript::` | `yo-tui/src/transcript` 곁의 test |
| shell 조합, layout, Surface, Unicode 너비, text flow | `cargo test -p yo-tui` | 소유 `yo-tui` 모듈 곁의 test |
| ANSI operation이나 표시 mode 정책 | `cargo test -p yo-tui terminal::` | `yo-tui/src/terminal` 아래 test |
| Inline 또는 Fullscreen mode 동작 | `cargo test -p yo-tui terminal::mode::` | `yo-tui/src/terminal/mode` 아래 test |
| live loop 순서, backpressure, submission draft 소유권, event Projection | `cargo test -p yo-tui runner::` | `yo-tui/src/runner` 아래 test |
| 같은 완성 frame의 터미널·HTML Projection | `cargo test -p yo-tui --test rendering_parity` | `crates/yo-tui/tests/rendering_parity`와 golden |
| process termination이나 실제 터미널 복원 | `cargo test -p yo-cli pty_tests::` | `crates/yo-cli/src/pty_tests/` |
| Unix process coordinator 상태와 보상 | `cargo test -p yo-cli execution::process::termination::tests` | `crates/yo-cli/src/execution/process/termination/tests` |
| 공통 bounded YAML parse·inference·failure budget | `cargo test -p yo-yaml` | `shared/yo-yaml/src/lib.rs` |
| Rust test 바로 위에 필요한 설명 | `cargo xtask check test-explanations` | `crates/`, `shared/`, `tools/` 아래 Rust source |
| 작업 context 경로, 변경 파일에 연결된 문서 안내, 로컬 참조 검사 | `python3 -m unittest discover -s tools -p test_context.py`와 `python3 tools/context.py check` | `tools/context.py`, 기존 Markdown 경로 표와 로컬 링크 대상 |
| Slice 변경이 bind된 로컬 write-set 안에 머무는지 | `cargo xtask check slice-scope` | 하나의 활성 Slice worktree; planner가 먼저 `cargo xtask slice-contract bind <contract.json>` 실행 |
| 두 Slice contract의 현재 통합 기준점이 같고 선언한 소유권이 겹치지 않는지 | `cargo xtask check slice-parallel <left.json> <right.json>` | direct Slice는 `develop`, Wave Slice는 해당 Wave branch 사용 |
| 하나의 깨끗한 Slice 후보에서 검증, 리뷰, 위험, 승인 증거가 모두 같은 identity에 결속됐는지 | `cargo xtask slice gate <request.json>` | 검증이나 리뷰를 다시 실행하지 않고 다음 행동 하나만 반환 |
| ready Slice의 정확한 commit message와 close 기록을 identity 전사 없이 준비하는지 | `cargo xtask slice commit prepare <gate.json> <message-source> <message-out>` 실행 후 exact squash를 commit하고, `close plan/apply` 전에 `cargo xtask slice close prepare <request.json>` 실행 | 첫 prepare는 깨끗한 Slice worktree, close prepare는 accepted commit 이후 깨끗한 통합 worktree에서 실행 |
| 저장소 hook 정책이나 구조화된 개발 검사 | `cargo test -p xtask` | `tools/xtask/src` |
| Prospective activation ContextBuild와 review-packet identity | `cargo test -p methexis activation_review_context`와 `cargo test -p xtask review_packet::tests::prospective` | 정확한 activation request, 제안 Checkpoint·active record, authority mode, packet 재생, active-authority 교차 사용 거절 |
| tmux, SSH, SSH 내부 tmux 동작 | [터미널 환경 매트릭스](./terminal-matrix.md) 참고 | ignored `yo-cli` 환경 test |

이 명령들은 시작점이지, 영향받은 인접 경계를 무시해도 된다는 허가가
아니다. 예를 들어 `AgentSession` 수정으로 frontend가 보는 admission
결과가 달라진다면 집중 test와 TUI runner test가 모두 필요할 수 있다.

model-connector request와 stream 검증은 변경한 dialect를 소유하는 concrete Connector
crate(예: `cargo test --locked -p yo-connector-openai-chat-completions` 또는
`cargo test --locked -p yo-connector-kimi`)를 실행하고, 공용 byte
lifecycle mechanics가 영향받으면 `cargo test --locked -p yo-connector-transport`도 함께
실행한다. 종료할 때는 중립 어휘와 managed-loop consumer를 위해
`cargo test --locked -p yo-core`를 실행한다. 환경 통합 Connector 검사는 로컬
`127.0.0.1` HTTPS listener만 사용하며 ephemeral test certificate를 만들고
serve하기 위해 외부 `python3`와 `openssl` 명령을 요구한다. 필수 조건이
없으면 assertion을 skip하지 않고 명령이 실패한다. 각 validation 실행마다
host/platform, prerequisite version, passed/unverified 결과를 기록한다.

## 실제 스킬 선택과 재개

사용이 승인된 인증 backend에서 임시 workspace와
`.agents/skills/yo-proof/SKILL.md`를 만든다. `name: yo-proof`, 설명과 함께
스킬 본문에만 등장하는 새 임의 토큰을 그대로 답하라는 지침을 넣는다.
managed backend는 해당 workspace의 `.agents/skills`를 `scope: workspace`인
`skills.roots` 항목으로 설정한다. Codex는 자체 authoritative 스킬 카탈로그를 사용한다.

실제 TUI에서 `Use $yo-proof`를 입력하고 활성 Skills 결과가 표시되면 Enter로 선택한다.
draft가 아직 전송되지 않았는지 확인한 뒤 Enter를 다시 눌러 제출한다.
커밋된 StartTurn의 `yo.structured-input/v2`에서 reference span, projection,
locator, scope, environment, revision digest와 전체 원본 지침을 정확히 대조한다.
해당 turn의 완료된 assistant 메시지가 토큰과 일치하는지 확인한다.

정상 종료 후 임시 스킬 파일만 삭제하고 새 process에서 같은 Session을 재개한다.
새 prompt에 토큰을 포함하지 않고 앞선 토큰을 요청한다. 두 번째 완료 turn의 별도
assistant 답변과 보존된 frozen input을 확인한다. typed journal과 함께
`yo session UUID --ascii`의 표시 기록도 검증한다. durable 메시지는 별도 segment나
마지막 inline segment를 사용할 수 있으므로, 교체된 이전 streaming revision까지
고려해 activity와 revision을 맞춘다. backend별 결과, 정리 결과, 관찰한 도구 실행이나
대화형 요청을 기록한다.

## 실제 managed 명령 승인과 복구

사용이 승인된 backend에서 새 임시 workspace와 저장소를 사용한다. 합성 토큰을 파일에
추가하는 정확한 `run_command` 하나를 요청하고 실제 TUI에서 거절한 뒤 파일이 없음을
확인한다. 별도의 추가 명령을 요청하고 인자·도구 ID·효과·digest가 정확할 때만 승인한다.
승인 패널의 크기 변경으로 어느 명령도 실행되면 안 된다. 승인한 파일에는 토큰이 정확히
한 번만 있어야 한다. 덮어쓰기 대신 추가를 사용해야 중복 실행을 탐지할 수 있다.

저장된 응답을 요청 activity·turn·request ID와 연결하고, 승인 텍스트·typed ToolOutput·
replay 결과의 명령·call ID·실행 host·outcome을 대조한다. 거절한 호출은 실패한 도구
결과로 모델에 전달되며 그 자체로 turn을 중단하지 않는다. 응답 전 승인 텍스트는 미종료
메시지이므로 `message_ended`를 기다리지 말고 최신 durable segments를 읽는다.

작업이 끝나고 입력이 비어 있으면 Ctrl-D로 정상 종료한다. `/exit`를 사용한다면 표시된
명령 항목을 선택한다. Esc는 변경하지 않은 슬래시 텍스트를 모델에 보낼 수 있게 하는
명시적 동작이다. 합성 결과 파일만 삭제하고 새 프로세스에서 재개한 뒤 토큰이나 도구 사용
없이 앞선 토큰을 요청한다. 정확한 답변·새 도구 활동 없음·물리 journal prefix 보존을
검사한다. 실패한 검증기 기록은 이후 감사·계속 실행 결과와 구분하고, 의도하지 않은 테스트
입력까지 실제 발생한 모든 turn을 포함한다.

Codex 위임 backend는 임시 실행 설정에서 승인을 사용자에게 전달하도록 하고 명시적인
명령 승인을 요청한다. 사용자의 영구 정책은 바꾸지 않는다. 저장된
`yo.activity-approval/v1`을 파싱하고 한 요청만 승인하는 `Approve request`의 번호를
선택한다. 규칙 저장이나 세션 전체 허용을 고르지 않는다. 구조화된 패널은 제시된 번호를
받으며 일반적인 `y`는 받지 않는다. 거절 선택지가 `Decline and stop`이면 turn이
중단되므로 최종 assistant 답변이 없어도 된다. 실제 선택지와 turn outcome을 확인한다.
managed FunctionCall replay를 요구하지 말고 `commandExecution` 인자와 최종 typed
결과를 대조한다. 새 프로세스 재개에서 native session locator와 물리 journal prefix를
보존해야 한다. 파일 추가·부재·크기 변경·도구 없는 회상·정리 검사도 동일하게 적용한다.
이 검증은 사용자에게 전달된 명시적 승인 경로를 확인하며 자동 승인 검토나 영구 정책
변경까지 검증한 것은 아니다.

## 채팅 시각 preview

### 대화형 테스트 에이전트

실제 `yo` 채팅에서 `/preview`를 입력한다. 별도 Python launcher나 앱 없이
격리된 임시 child TUI state와 오프라인 테스트 에이전트가 열린다. 임의의 텍스트로
스트리밍 응답을 받거나 `tools`, `error`, `long`, `markdown`, `tables`, `diff`로 모의 도구 출력·실패 복구·
스크롤·본문 및 코드 서식·반응형 표·코드 변경 내용을 확인한다. 시나리오 이름은 sandbox 안의 일반 메시지다. `Esc`는 중단이며,
`/preview`, `/exit`, 빈 입력창 종료 동작은 원래 대화로 돌아간다.
실제 Turn·submission·request가 대기 중이면 진입을 거절한다.

처음 펼쳐지는 호스트 안내 문서는 코드·문서, 파일·검색, 도구·터미널, 승인·인터뷰,
진행·상태, 차트·미디어별로 예시를 묶는다. 일반 문서 테마와 줄바꿈을 사용하며
진입 직후 Alt+Up으로 안내 시작에 이동할 수 있다. 파일·내용 검색, shell 변형,
합산 diff와 모델 전환 안내도 포함한다.

별도 오프라인 터미널을 계속 열어두려면
`cargo run --locked -p yo-tui --example chat_preview`를 실행하고 `/preview`를 입력한다.
모델 연결이나 테스트 실행기의 시간 경고 없이 실제 TUI를 사용한다.
각 시나리오는 `Enter send`와 빈 입력창을 확인한 뒤 전송한다.


parent가 실제 observation과 session output을 계속 소유한다. 프리뷰 command와
합성 record는 child에만 머물고, frame은 같은 appearance를 pin하되 프리뷰 scrollback
publication은 비활성화한다. 복귀 시 프리뷰 draft·transcript·navigation은 버린다.
원래 대화는 보존하고 command 텍스트 자체는 소비한다. 편집·paste·resize·view navigation과
렌더링은 production TUI를 사용한다. 작업 중 제출은 명시적으로 거절하고 sandbox draft를
보존한다. steering·approval·provider 동작·영속 저장·실제 도구 실행은 모의하지 않는다.
idle preview는 주기적 poll을 예약하지 않는다. 대기 중인 합성 출력이 자체 deadline을
제공하며, motion과 실제 agent backpressure deadline을 함께 고려한다.

구현은 `command/preview.rs`가 command 등록, `runner/state/preview.rs`가 격리와
lifecycle, `runner/preview_agent.rs`가 합성 event 생성을 담당한다.
`cargo test --locked -p yo-tui`와 인접 `yo-cli` PTY test로 검증한다.

승인 프로필은 선택적인 `related_change`로 같은 Turn에서 관측한 파일 변경 활동의
0이 아닌 ID를 참조할 수 있다. 프로바이더 wire ID와 독립적이며 필드가 없는 이전
프로필도 호환된다. 이 승인 중 `/changes`는 더 최근의 무관한 변경 대신 연결된 활동의
첫 파일을 연다. 선언한 활동이 없으면 다른 파일을 선택하지 않고 부재를 설명한다.
변경 보기는 승인을 제출하지 않으며 결정에는 기존 표시 프레임 검증이 계속 적용된다.
Codex는 같은 Turn에서 정확한 item ID의 파일 변경을 관측한 경우에만 참조를 제공한다.
요청이 먼저 도착하면 이후 시작·완료 항목에서 변경 본문을 발행한 뒤 미응답 승인
프로필을 갱신한다. 이미 답변한 요청은 갱신하지 않으며 연결 갱신은 한 번만 발생한다.
최대 활동 ID의 인코딩 공간을 최초 수락 시 확보하므로 뒤늦은 연결이 프로필 크기 한도를
넘지 않는다. 연결 정보나 연결된 파일 변경의 본문·완료 상태가 갱신되면 새 승인 프레임을 표시해야 한다.
갱신 전에 준비한 프레임을 뒤늦게 반영해도 키보드 선택·숫자 입력 승인은 다시 허용되지
않는다. 무관한 파일이나 다른 Turn의 같은 숫자 ID는 요청 패널을 무효화하지 않는다.
이 검증은 화면의 최신성을 확인하며 사용자가 모든 diff 행을 읽었음을 증명하지는 않는다.
완료된 파일 변경도 보관된 transcript에서 참조할 수 있으며 Codex 어댑터의 항목
매핑은 Turn 종료까지 유지한다.
`/preview approval-diff`는 더 최근의 무관한 변경이 있어도 정확한 제안 파일을 검토하는
예시다. 모든 선택은 오프라인에 머무른다.

승인 요청에는 보고된 명령·작업 디렉터리·환경·권한·네트워크·정책 제안·제공된 결정과
알 수 없는 추가 필드를 보존한다. 파일 변경의 쓰기 루트에는 요청된 세션 범위를 표시한다.
크기 제한을 둔 직렬화로 표시 한도를 넘는 요청을 승인 가능한 상태로 게시하기 전에 거부한다.
`Approve request`는 보고된 범위를 따르며 일회성 권한이라고 약속하지 않는다.
명시적인 `availableDecisions`는 `ActivityApproval` (`yo.activity-approval/v1`)로
원래 선택지 순서·범위 설명·권한을 부여하지 않는 기본값을 보존한다. UI는 거절/취소를
먼저 놓으며 해당 선택지가 없으면 Enter의 기본 동작은 turn 중단이다. 선택 및 번호 입력은
프로필 갱신 뒤를 포함해 실제 frame이 표시된 후에만 제출한다.
`ApprovalDecision::Offered`는 원래 1-based 번호와 요청 identity를 closed command codec에
저장하며 SubmissionId를 새로 만들지 않는다. Codex는 이를 원래 요청의 일반 승인·세션 승인·
거절·취소·정확한 명령 정책 변경·영구 네트워크 허용/거부 값으로 연결한다. 파일 변경 요청은
명령/네트워크 정책 객체를 제출할 수 없다. 미지/잘못된 결정은 비활성화하며 범위 밖 번호는
wire 응답이나 결과 기록을 만들지 않는다. 선택지 문자를 표시할 수 없으면 중단만 제공하며
번호 입력으로 이 대체 패널을 우회할 수 없다. 선택지는 최대 64개, 인코딩된 프로필은 16 MiB로
제한하며 초과 시 일반 승인 버튼으로 대체하지 않고 요청을 거부한다. 결정 목록 생략/null은
Codex 0.145.0·0.146.0·0.149.0에서 확인한 기본 규칙을 적용한다. 네트워크 요청은 일반 승인·
세션 승인·처음 제안된 영구 허용 규칙(있을 때)·취소를 제공한다. 그 외 추가 권한 요청은
일반 승인·취소만 제공하고, 일반 명령은 제안된 명령 규칙이 있을 때만 이를 추가한다.
파일 변경은 일반 승인·세션 승인·취소를 제공한다. 빈 목록을 포함한 명시 목록은 항상 우선하며,
정책 선택지 생성에 사용하는 잘못된 정보는 binding 전에 거부한다. 명시적인 서버 제한이
없을 때 기존 binary API 승인/거절 응답은 유지하며, UI 번호는 생성된 목록에만 연결한다.
고정 버전의 [명령 기본 규칙](https://github.com/openai/codex/blob/rust-v0.149.0/codex-rs/tui/src/approval_events.rs)과
[파일 승인 메뉴](https://github.com/openai/codex/blob/rust-v0.149.0/codex-rs/tui/src/bottom_pane/approval_overlay.rs)를 참고한다. Managed·Grok은 미지원 선택지 번호를 승인 상태 소비 전에
거부한다. 응답 쓰기가 성공한 뒤에만 범위를 포함한 결정 기록을 게시하며 사용자 지정 가능한
activity 스타일을 적용한다. 오프라인 `approval-scopes`는 권한·정책·파일 변경 없이 범위별
선택을 확인한다. 실제 provider의 정책 영속 적용은 아직 검증하지 않았다.

### 정적 비교 fixture

`transcript::layout::markdown`은 assistant message 본문만 해석해 styled cell로 만든다.
제목, 중첩 강조, 목적지를 표시하는 링크, 후속 행 들여쓰기가 있는 목록, 인용문,
task checkbox, 원문 코드 블록을 지원한다. 일반 문장은 공백에서 줄바꿈하고 긴 token은
grapheme 단위로 나눈다. 코드 패널도 폭 안에 들어가는 단어를 붙여 두되 들여쓰기·
확장된 탭을 포함한 모든 공백 glyph와 원문 위치를 유지한다. 폭보다 긴 토큰은
grapheme 경계에서 나눈다. 코드 패널은 전용 본문 배경과 별도 언어 헤더 배경을 사용한다.
일반 텍스트 fence(`text`, `txt`, `plaintext`)는 반복 라벨 행을 생략하고 본문 배경·
여백·원문을 유지한다. 프로그래밍 언어와 diff 라벨은 계속 표시한다.
`tui.code_padding` / `OutputPreferences::with_code_padding`은 패널 좌우 여백을 설정한다.
기본 1칸이며 0도 허용하고 8을 넘으면 8로 제한한다. 좁은 패널은 본문 공간을 남기도록
여백을 줄인다. assistant Markdown·호스트/도구 문서·파일 diff에 공통 적용하며 테마
변경에도 설정을 유지하고 내보내기 원문은 변경하지 않는다. Rust 프리뷰에서
`--code-padding=N`으로 확인할 수 있다.

diff 추가·삭제 배경은 안쪽 여백, 오른쪽 빈 공간, 자동 줄바꿈 행까지 이어진다.
어두운 테마의 낮은 채도, 밝은 테마의 파스텔, 흑백의 색 없는 표현은 중앙에서
결정하며 코드 배경은 본문 폭과 스크롤 경계를 따른다.
GFM 표는 셀 강조, 한글 폭, 좌측/가운데/우측 열 정렬을 보존한다.
자연스러운 열 너비의 합이 본문 폭을 넘으면 `markdown/table.rs`가 열을 줄이고
셀 안에서 줄바꿈하며 정렬과 스타일을 보존한다. 이진 탐색으로 공통 폭 상한을 구하고
남은 칸은 앞 열부터 배분하므로, 줄일 칸마다 모든 열을 다시 검사하지 않는다.
회귀 테스트는 서로 다른 열 너비, 동률 배분, 긴 셀 32개의 폭 변경을 확인한다.
각 열에 여덟 칸(원래 폭이 더 작으면
그 폭)을 확보할 수 없으면 열 이름이 붙은 필드로 전환하고 행 사이에 빈 줄을 둔다.
값은 자르지 않는다.
셀의 탭과 제어문자는 정렬 전에 정규화한다. `diff`·`patch` fence와 타입이 지정된 파일 변경 activity에서 추가/삭제/메타정보
테마 역할을 사용하며, 모든 palette에서 원래 부호를 유지한다. 파일 header는 `---`나
`+++` 뒤에 구분자가 있어야 하므로, 반복 부호로 시작하는 변경 본문을 메타정보로
오인하지 않는다.
GitHub식 각주는 정의된 `[^이름]` 참조와 정의에 같은 `[이름]` 표기를 사용한다.
정의 본문은 들여쓰기하며 Markdown 링크·인라인 코드·중첩 목록을 유지한다. 표기는
기존 굵은 quote 역할을 사용하고 본문 폭·테마 설정을 따른다. 정의는 원문 위치에
남으며 참조 표기가 내부 이동 동작을 뜻하지는 않는다. 미정의 참조와 코드 블록은
원문이며, 이후 snapshot에 정의가 도착하면 참조를 연결한다. `footnotes` 프리뷰는
반복 참조와 한글 이름을 포함한다.
사용자 텍스트, 도구 로그, notice, 저장 record, plain output은 원문 기호를 유지한다.
승인·인터뷰 요청, 답변·결정 기록과 typed notice는 markup을 해석하지 않고 단어 경계에서
줄바꿈한다. 원문의 들여쓰기와 명시적 줄바꿈을 보존하며 폭보다 긴 단어만 grapheme 경계에서
나눈다. 공통 `text/flow.rs` 문장 엔진은 Markdown도 처리하되 기존 선행 공백 정책은 유지한다.
원시 로그 배치와 입력 편집기의 커서 좌표는 기존 원문 flow를 사용한다. UTF-8 원문 위치,
결합 문자, 좁은 한글/emoji, 들여쓰기, 원문 내보내기와 요청·응답 frame 테마를 테스트한다.
닫히지 않은 streaming fence, 한글·이모지 경계, 제어문자 표기, 줄바꿈 뒤 style span,
원문 출력 보존을 테스트한다. event 전용 `pulldown-cmark` dependency는 HTML/CLI
feature와 terminal/platform 호출을 사용하지 않으며 `markdown.rs`가 parser 교체 경계다.
여기서는 Linux 컴파일과 렌더링을 검사한다. macOS runtime 동작은 platform matrix에서
추가로 확인해야 한다.

도구 호출, 도구 결과, 파일 변경 observation은 typed presentation 상태를 가진다.
제목, 원문 로그, outcome footer는 각각 테마 스타일을 받으며 로그 안의 실패처럼 보이는
문자열로 오류 스타일을 선택하지 않는다. `transcript/activity.rs`는 종료 상태, 직접 삽입한
footer의 경계, Final phase를 하나의 revision으로 확정하고, overflow에서는 부분 변경
없이 거부한다. Turn 실패/중단 notice도 같은 상태 역할을 사용한다. 도구 호출 중단은
상태 제목을 유지하되 중복된 중단 문구를 추가하지 않는다. 실제 event-to-frame 스타일,
로그 안의 가짜 실패 문구, 원문 보존, 불변 finalization을 테스트한다.

Codex의 `runtime/events.rs`는 `item/started`의 명령·작업 디렉터리를 출력 delta보다
먼저 전달하고, 완료 시 실제 출력·종료 코드·소요 시간이 포함된 최종 snapshot으로 교체한다.
파일 변경은 경로·구조화된 rename 종류·diff 원문을 보존한다. MCP/dynamic 도구는
이름·인수·결과·오류를 남긴다. `runtime::tests::coding_events`는 실제 adapter의 이벤트
순서와 payload를 검사하며 모델을 호출하지 않는다. 설치된 프로그램의 initialize smoke는
연결·정리만 검증하며 실제 코딩 Turn이나 해당 버전의 전체 호환성을 증명하지 않는다.

선택 실행하는 `live_agent_session::local_codex_completes_a_real_file_change` 통합 검증은
인증된 Codex 모델 한 턴과 임시 디렉터리를 사용한다. `pwd`를 실행하고 패치 도구로 고정된
내용의 파일 하나를 생성한다. 완료된 ToolCall·FileChange Activity, 일치하는 diff 추가 행,
완료 Turn과 정확한 파일 바이트를 요구하며 종료 후 임시 디렉터리를 삭제한다.
`cargo test --locked -p yo-backend-delegated-codex --test live_agent_session local_codex_completes_a_real_file_change -- --ignored --exact`로 실행한다.
명령 수락 재시도는 같은 PendingCommand를 최대 30초 보존하고, 거절은 즉시 실패로 보고한다.
명령이 수락된 뒤에만 180초 완료 대기를 시작한다. Backpressured를 무시하면 요청이 전달되지
않았는데도 Activity 0개인 시간 초과가 발생할 수 있다. 예상하지 않은 대화형 요청은 승인하지
않고 실패로 끝낸다. 통과는 이 제한된 실제 코딩 경로를 입증하며 TUI 상호작용·승인 선택이나
모든 프로바이더 기능을 입증하지는 않는다.

같은 target의 선택 실행 test인 `local_codex_resumes_a_durable_session_and_remembers_prior_input`은
backend 종료, 저장소 재열기, durable continuation 복구를 사이에 두고 텍스트 모델 두 턴을
수행한다. 두 번째 prompt는 첫 턴의 무작위 nonce를 포함하지 않으며, 마지막 응답은 이를
정확히 재현해야 한다. 같은 Session·descriptor·binding·locator, 다음 Turn ID와 기존 durable
기록 prefix 보존도 요구한다. 임시 저장소와 read-only host profile을 사용하고, 예상하지 않은
도구·대화형 Activity를 거부하며 workspace가 비어 있는지 확인한다. 시작·수락·각 턴에는
명시적 시간 제한이 있다. 위 명령의 test 이름을 바꾸어 실행한다. 이 검증은 설치된 Codex
환경의 native continuation을 입증하며 managed replay나 compaction을 입증하지는 않는다.

`yo-backend-managed`의
`backend::tests::context_replay::automatic_compaction_survives_disk_resume_with_exact_retained_connector_input`은
실제 로컬 저장소와 재시작을 거치는 자동 압축을 검증한다. 일반 입력 세 턴이 요약 한 번을
유발하고, 후속 connector는 요청 시작 전에 checkpoint를 디스크에서 읽을 수 있는지 확인한다.
종료·저장소 재열기 후 새 backend는 복구 중 요청을 보내지 않고 네 번째 입력에 요청 하나만
보내야 한다. 전체 문맥의 순서, binding, context epoch와 기존 저장 기록 prefix 보존을 비교한다.
기록용 connector와 token counter는 제어된 fixture이므로 인증된 provider나 tokenizer 검증은 아니다.
인증된 provider의 자동 압축 검증은 새 임시 workspace와 합성 기록을 사용한다. 요약될 과거
턴에만 고유 회상 토큰을 넣고, 별도의 짧은 턴을 보존한 뒤 Rust TUI로 설정 임계치를 넘는
새 입력을 제출한다. `/compact`는 사용하지 않는다. 자동 checkpoint 하나, 보존한 턴과 현재
입력의 정확한 bytes·순서, 줄어든 요청 계산량을 확인한다. 정상 종료 후 새 프로세스에서
재개하고 토큰을 포함하지 않는 질문으로 회상을 확인한다. 질문 직전 유효 문맥에서는 토큰이
요약에만 있어야 하며, 물리 journal은 기존 prefix를 보존해야 한다. 설정된 counter profile을
함께 기록한다. 보수적인 UTF-8 byte 계산량은 provider가 보고한 token usage가 아니다.
Journal 순서 검증은 위의 제어된 disk-before-dispatch 테스트를 보완하며, 그 자체로 fsync
시점을 측정하지는 않는다. tmux를 조작할 때는 `paste-buffer -p -r`로 개행을 보존하고
의도한 prompt뿐 아니라 실제 저장된 입력도 비교한다.

요약 요청은 보이는 기록을 단일 JSON user 메시지로 전달하며 역할·도구 호출 관계를 자료로 보존한다.
자동·명시 압축 모두 private replay를 제외하고 도구를 비활성화한다. 일반 replay의 provider 형식은
유지한다. 해당 테스트는 실제 connector 입력과 JSON escaping을 포함한 16 MiB 인코딩 한도를 확인한다.
완료·검증된 idle 요약이 입력 한도 안에 들지만 크기를 줄이지 못하면 압축 거절을 알리고
다음 입력에 원래 문맥을 계속 사용한다. core worker 테스트는 명령 전달 시점과 요약 polling 이후의
거절을 모두 검증한다. usage가 유효하고 완전히 종료된 idle 응답의 요약 형식이 잘못된 경우에도
문맥을 교체하지 않고 거절한다. 검증 규칙은 엄격하게 유지하고 자동 재요청은 보내지 않는다.
알림에는 고정된 형식 진단만 표시하며 모델 본문은 포함하지 않는다. 문맥 압력·불완전한 응답·
프로토콜 오류·usage 누락·정리 실패는 기존처럼 실행을 중단한다. 자동 압축의 실패 동작도 유지한다.
managed 디스크 회귀는 거절 직후 종료하고 복구한 binding·epoch·Anchor·전체 replay를 확인한 뒤
다음 입력을 실행한다. 단독 `CompactContext` 기록은 기존 Anchor를 보존한다. 이후 일반 요청이
수락되면 대응하는 완료 결과와 Anchor가 저장될 때까지 기존 Anchor를 계속 무효화한다.

타입이 지정된 파일 변경은 `transcript/layout/activity.rs`와 기존 literal diff renderer를
사용한다. 소스 내부 fence가 Markdown이나 이미지로 해석되지 않는다. 추가·삭제 행 수는
화면에만 표시한다. 접기 시 glyph와 배경 행을 함께 이동하고 종료 상태를 남긴다. 변경 패널은 경로·hunk·첫
변경을 볼 수 있도록 처음 여섯 행과 마지막 행을 남기며 본문 열두 행을 넘으면 접는다.
`/preview`의 `changes`는 실제 activity 경로, `diff`는 Markdown fence 예시를 확인한다.

Codex 추가·삭제 payload는 파일 원문이므로 `+`/`-` 줄과 마지막 개행 없음 표시로 변환하며
수정 파일의 unified diff는 그대로 둔다.
[Codex 표시 변환](https://github.com/openai/codex/blob/main/codex-rs/tui/src/app_server_approval_conversions.rs)을 참고한다.
`/changes`는 보존된 타입 지정 FileChange를 읽는 전용 화면이며 Git 작업 트리 스캔이 아니다.
좌우 키로 파일을 선택하고 위아래·휠로 접지 않은 diff를 읽으며 F1로 Chat에 돌아간다.
명시적 파일 metadata가 있으면 내부 `diff --git` 행은 새 파일 구역을 만들지 않는다.
스크롤 중에도 상단에 선택한 명시적 파일 경로가 고정된다. 좁은 폭에서는 파일 번호와
경로를 우선하고, 필요하면 앞부분을 생략 표시로 줄인다. 경로 끝의 문자 묶음은 Surface와
같은 셀 폭으로 계산한다. 전체 경로와 diff 원문은 유지한다. 한 칸 폭의 한글처럼
문자 묶음이 들어갈 수 없으면 이스케이프 표시임을 알리고 줄바꿈·diff 색상을 유지한다.
이스케이프 구간은 원문 위치에 대응하므로 해당 구간 안에서 스크롤해도 화면을 넓히면
원래 문자와 읽던 위치를 복원한다. 공통 읽기 전용 페이지 생성자가 이스케이프 행을 원문 위치에 대응하며,
변환 구간의 임시 위치 정보는 생성 뒤 폐기한다. `changes`
프리뷰에 한글·이모지를 포함하여 이 경계를 확인한다. 원시 Git 헤더는 인용된
파일명을 추측하지 않고 일반 제목을 유지한다.
`runner::tests::views::navigation`은 좁은 폭의 안내, 파일 선택, 스크롤, Chat 복귀와
검토 화면 입력이 모델 요청을 제출하지 않는 경계를 검사한다.
선택한 파일은 usize 행 수를 쓰는 `TextPages`로 배치하고 항목·파일 구간·revision·폭별
캐시를 재사용하여 보이는 페이지만 그린다. 원래 논리 행의 위치로 추가·삭제·hunk 스타일을
판정하므로 줄바꿈된 행에도 배경과 여백이 유지된다. Markdown과 검토 화면은 같은 diff
분류를 사용한다. 테마 변경은 원문 페이지를 다시 만들지 않고 새 스타일을 반영한다.
End는 새 snapshot의 끝을 따르고, 과거 위치는 폭 변경 시 원문 byte 위치로 복원하며
파일 전환은 제목부터 시작한다. 개별 diff가 inline u16 높이 예산을 넘으면 변경 요약과
`/changes` 안내를 표시한다. inline 펼침 상태에도 적용하며 보존 원문·내보내기와 전체
검토 화면은 유지한다. 테스트는 7만 행, 실패한 frame 뒤 이동 재시도, snapshot 추가,
파일 전환, 줄바꿈 배경, 사용자 색상과 폭 왕복을 검증한다.

Chat·Transcript·Request의 누적 레이아웃과 스크롤 위치는 usize 문서 행을 사용한다.
보이는 논리 행만 기존 u16 Surface 좌표로 바꿔 glyph·코드 띠·사용자 배경·문맥 항목·이미지를
배치한다. live 자연 높이도 usize로 측정하고 inline live 합성 경계에서 실제 터미널 높이로
제한한다. 산술 오버플로는 typed 오류로 남긴다. 테스트는 누적 65,535·65,536·98,308행,
Home/End, usize::MAX 근처 위치, 두 diff를 펼친 8만 행 Chat, 긴 대화 뒤의 코드·diff·이미지,
큰 미게시 inline suffix를 검증한다. 여러 메시지의 합산 높이 제한을 없앤 것으로, 단일
rich 메시지 내부의 배치 한도나 하나의 persistent inline 게시 Surface가 갖는 u16 한도를
없앤 것은 아니다. 너무 큰 게시 후보는 cursor 승인 전에 명시적으로 실패한다. 현재도
보존 항목 전체를 준비하며 단일 메시지 본문까지 완전히 가상화하는 작업은 별도로 남는다.

Codex `turn/plan/updated`는 Turn별 ModelWork 하나를 갱신하고 Turn 완료 전에 종료한다.
늦게 도착한 계획은 재개하지 않으며 남은 단계를 임의로 완료하지 않는다. `plan`은 같은
snapshot 투영을 보여준다. `item/tool/requestUserInput`은 비밀이 아닌 질문을 순서대로
제시하고 마지막 답변 뒤 원래 JSON-RPC ID로 답변 map을 한 번 전송한다. 선택 번호는
label로 변환하고 직접 쓴 Unicode 답변은 보존한다. Core runtime 테스트로 후속 요청의
상관관계, 중복 응답, 부분 답변 중단과 늦은 resolved를 확인한다. 잘못되거나 비밀인 질문은
내용을 표시하지 않고 거절한다. 비밀 입력이나 모델용 이미지 입력을 추가하지 않는다.


`runner/chat.rs`는 `SessionUsageProjection`으로 단일 사용량 snapshot을 검증하고,
해당 activity가 완료된 뒤에만 하단의 최신 관측값을 갱신한다. `Last`는 최근 보고값이며
세션 누적값·잔여 한도·현재 context 점유율·추정 비용이 아니다. 미지원·미보고 값은 명시하고,
잘못된 영수증은 사용량 불가로 표시한다. 저장된 원본 record는 바꾸지 않는다.
`usage`, `mcp`는 모델 호출 없는 프리뷰 시나리오다.

activity 회귀는 `runner::tests::activity_projection`에서 완료·실패·중단 시 `Working`
행과 motion demand가 사라지고, 도구의 종료 heading이 payload를 바꾸지 않고 진행 문구를
대체하는지 검사한다. `terminal::mode::fullscreen`은 frame 단위 ANSI 출력과 부분 쓰기·
flush 실패 후 복구를 검사한다. fullscreen frame은 diff와 마지막 cursor를 동기화 출력
시작·종료 시퀀스(CSI ?2026h/l)로 감싸서 한 batch로 쓴다. 지원하는 terminal에서는
label 갱신의 중간 상태를 숨긴다. batching만으로 원자적 화면 표시를 보장하지는 않는다.
쓰기·flush 실패 시 원래 오류를 바꾸거나 frame을 commit하지 않고 동기화 해제를 시도한다.
미지원 terminal과 지속적인 I/O 실패는 host 제약으로 남는다. 이번 fullscreen 전용 개선은
inline renderer를 바꾸지 않는다.

사용자가 요청한 Rich spinner 개선은 `⠋ ⠙ ⠸ ⠴ ⠦ ⠇` 순서다. 항상 점 3개를 유지하고
마지막→첫 frame을 포함해 테두리에서 한 칸씩 이동한다. 간격은 133,333,333 ns이며
한 바퀴 약 800 ms를 유지한다. ASCII는 80 ms를 유지한다. `appearance::tests`가
점 mask, 시간 경계, cell 폭을 검증한다.

marker 전환에는 16 ms sheen tick과 독립적인 deadline이 있다. scheduler는 FPS 간격과
직전에 관측한 render 비용을 이용해 다음 marker slot을 예약하고, 가까운 일반 redraw를
그 slot으로 합친다. `runner::unix::timing::output_timing`은 가상 시계로 실제 ANSI
쓰기를 검사한다. 60/120fps, 7 ms 간격 입력 요청, 고정 0/2 ms 쓰기 비용을 조합한다.
가변적인 host나 terminal 지연은 여전히 눈에 보이는 흔들림을 유발할 수 있다.

shell의 `Working` label은 TrueColor에서 단어 전체가 함께 밝아지며 위치와 굵기는
고정한다. Limited/Unknown에서는 label ink를 정적으로 유지한다. marker는 일정한
밝기로 회전한다. `shell::chrome::tests`는 좁은 행을 포함해 두 주기 동안 label 색의
균일함과 geometry·굵기의 고정을 검사한다. runner는 새로 표시되는 turn을 첫 marker에서
시작하며 terminal generation 내 redraw에서는 epoch를 유지한다. `timing::motion_tests`는
identity 전환을, `runner::tests::reentry`는 오래된 generation 시각을 주어도 처음 출력한
glyph가 첫 marker인지 검사한다. 입력 시간은 generation clock을 그대로 사용한다.
이 local 개선들은 아래 accepted motion 계약과 다르며, selection-panel title sheen은
그대로 유지한다.

비교 대상인 pi의 [기본 Loader](https://github.com/badlogic/pi-mono/blob/main/packages/tui/src/components/loader.ts)는
10-frame·80 ms이며 timer callback마다 index를 증가시킨다. Yo는 의도적으로 6-frame과
경과 시간 기반 선택을 유지하므로 늦은 wake에서 지난 phase를 재생하지 않고 건너뛸 수 있다.
pi의 [main-screen renderer](https://github.com/badlogic/pi-mono/blob/main/packages/tui/src/tui-main-screen.ts)와
yo의 fullscreen renderer 모두 동기화 출력을 사용하지만, 시각적인 움직임이 같거나
host 수준의 글자 흔들림이 해결됐다는 증거는 아니다.

이번 요청의 worktree 변경은 기존 10-frame
선택을 대체하지만, accepted Methexis checkpoint는 아직 이전 profile을 기술하며
재활성화하지 않았다. integration 전에 authority 정합성 조정이 필요하다.

terminal feedback loop는 checkout에서 `python3 tools/chat_preview.py`로 연다.
첫 실행 시 build하고 모델 호출 없이 alternate-screen viewer를 표시한다.
키는 `1` 빈 화면, `2` 대화, `3` 작업 중, `4` Markdown, `5` 표, `6` diff, `7` 도구 완료, `8` 실패, `9` 중단, `[`/`]` 이전/다음 장면(긴 초안·과거 기록·명령 선택·긴 도구 로그·여러 turn 포함), `w` 폭, `c` palette(default, light, mono, indexed light, ASCII),
`b` rebuild, `r` reload, `s` snapshot, `q` 종료다. 선택한 폭과 28행 이상이
필요하며 작은 terminal에서는 fixture를 잘라 그리지 않고 크기 안내를 표시한다.
정상 종료나 Python 예외 시 terminal 설정을 복원한다.

viewer를 열어둔 채 코드를 수정한다. 다른 pane이나 agent에서
`python3 tools/chat_preview.py --build-only`를 실행하면 renderer test 성공 후
새 generation을 자동으로 불러오며 선택한 시나리오·폭·palette는 유지한다.
실패하거나 불완전한 build는 마지막 성공 generation을 교체하지 않는다.
log, immutable generation, snapshot은 ignored `target/chat-preview/`에 쌓이며
`--directory`로 다른 local 출력 경로를 선택할 수 있다. snapshot에는 정확한 generation과
fixture를 기록한 `frame.json`, `frame.ansi`, `frame.html`이 있어 특정 화면을 기준으로
피드백할 수 있다. artifact는 수동 정리 전까지 보존한다. 이 도구 자체는 tmux를 제어하거나
다른 process에 키를 보내거나 live chat 입력을 받지 않는다.

feedback 도구 검증은 `python3 -m unittest discover -s tools -p test_chat_preview.py`로
실행한다(Unix PTY 필요).

`YO_TUI_PREVIEW_DIR=/tmp/yo-chat-preview cargo test --locked -p yo-tui chat_preview`를
실행하고 `/tmp/yo-chat-preview/chat.html`을 연다. test는 빈 화면, 대화, 작업 중, Markdown, 표, diff, 도구 완료, 실패, 중단, 긴 초안, 과거 기록, 명령 선택, 긴 도구 로그, 여러 turn, 승인, 인터뷰, 구문 강조, 차트, 이미지, 미디어 대체 상태를
20, 40, 88열의 실제 Session-to-Surface frame으로 내보낸다. default, light, mono
palette와 indexed light, ASCII/unknown-color fallback을 포함한다. Light fixture
카드는 밝은 host 배경을 사용한다. 개별 HTML로 원하는 화면만 캡처할 수 있다.
바깥쪽 browser card는 fixture 장식이지 terminal UI가 아니며, 기본 배경은 하나의
host theme을 가정한다.
짝을 이루는 `.ansi` 파일은 production `FrameDiff` → `TerminalOps` → `AnsiEncoder`
경로를 사용한다. fixture 이상의 크기로 비운 terminal에서 재생할 수 있다. 절대 cursor
위치가 포함되므로 종료 시 terminal 상태를 복원하는 alternate-screen viewer로 연다.
이는 정적 frame 재생이지 live agent나 공개 `yo preview` command가 아니다.

요청 띠의 대비, 차분한 답변 본문, 입력창 rule, key hint를 확인한다.
시각 reference는 공식 [Claude Code terminal refresh](https://www.anthropic.com/news/enabling-claude-code-to-work-more-autonomously)와
[Cursor CLI Ask mode](https://cursor.com/changelog/cli-jan-16-2026) screenshot이다.
절제된 강조, 분명한 입력 경계, 입력창 가까운 조작 안내를 차용하되 미지원 control이나
브랜드는 복제하지 않는다. 넓은 idle frame은 `@ files`를 표시하고, 좁은 frame은 핵심
keyboard help로 줄인다. test는 welcome과
placeholder 문구가 대화 출력에 들어가지 않는지도 검사한다. 이 preview가 실제 terminal
lifecycle이나 모든 host palette 호환성을 입증하지는 않는다. 그런 주장은 위 terminal
evidence layer로 확인한다.

### Whole-chat visual audit

개별 Markdown 예시뿐 아니라 전환과 조작 요소를 포함한 전체 대화를 검토한다.
아래는 점검 경로이며 accepted design contract나 모든 실제 terminal을 검증했다는
증거가 아니다.

| Elements | Inspect |
|---|---|
| 시작 안내, 빈 입력, 모델과 작업 경로 | 입력을 유도하고 metadata는 답변보다 조용하게 표시 |
| 사용자 표시, 요청 배경, 줄바꿈된 입력 | 명확한 turn 경계와 이어지는 행의 정렬 |
| 답변 표시, 문단, 제목, 강조 | 일반 문장을 제목처럼 강조하지 않는 읽기 위계 |
| 목록, 체크리스트, 인용, 링크 | 내어쓰기, 보이는 링크 목적지, 한글과 결합 문자 |
| 인라인 코드, 코드 블록, 언어 표시 | 공백 보존, 빈 영역까지 이어지는 배경, 스트리밍 중 미완성 fence |
| 표와 diff | 좁은 폭의 대체 표시, 색 없이도 식별되는 추가·삭제 |
| 도구 제목, 로그, 완료, 중단, 실패 | typed event에 따른 상태, 긴 로그 접기, 실제 실패 사유 보존 |
| 긴 대화와 과거 기록 위치 | 과거 위치 표시, End로 최신 추적 복원, 작업 중 표시 유지 |
| 빈 입력, 여러 줄·스크롤 입력, 커서 | 긴 초안에서도 대화 공간 유지, 아래 구분선에 행 범위 표시, 원문 보존 |
| 입력 구분선, 진행 행, 키 도움말 | idle/active 배치 안정성, 표시 모드보다 전송·중단 우선 |
| 명령·파일·모델 선택 화면 | 포커스, 비활성 항목, 축약, 닫기·선택 안내, 열린 메뉴에서 전송 안내 금지 |
| 구문 강조 | Rust/Python/JSON 토큰, 여러 줄 주석, 미지원 언어 대체, 원문 공백과 코드 배경 |
| 차트와 이미지 | 음수·0, 공통 기준축, 원래 수치, 잘못된 데이터, PNG/JPEG 제한, 단색 명암, 크기 변경과 스크롤 |
| 승인과 agent 질문 | 기본 거절, 명시적 요청 승인, 요청 ID를 보존한 답변, 후속 질문, 취소, frame 표시 전 선택 방지 |
| Default, light, mono, 제한 색상, ASCII | 지원 색상과 속성으로 의미별 역할을 읽을 수 있음 |

frame 사이에 들어온 방향키는 순서대로 적용한다. 연속 입력, 경계에서의 반대 방향
이동, frame 실패 후 재시도, 폭 변경을 개별 키 처리와 비교한다. 코드 패널은 장식 선·모서리·이어짐 화살표 없이
언어 헤더와 안쪽 여백으로 구분한다. 코드의 이어지는 줄은 원문 들여쓰기를 본문 폭의 ¼ 이내로 유지하며 원문에 문자를 추가하지 않는다. 좁은 폭에서는 오른쪽 여백을 줄이고,
줄바꿈·스크롤 중에도 원문과 diff 부호를 보존한다. Fullscreen은 SGR
마우스 휠을 받아 이벤트당 세 행을 이동하고 클릭·수평 이동은 편집 없이 소비한다.
Inline은 터미널 기본 스크롤을 유지한다. tmux `mouse on`이면 휠이 앱에 전달되며
tmux copy mode는 별도의 터미널 스크롤백이다. 종료·panic·suspend·부분 진입 실패에서
마우스 캡처 해제를 확인한다.

Chat의 긴 도구 본문은 렌더링된 여덟 행을 넘으면 접힌다. 앞부분과 최신 행, 숨겨진
행 수를 표시하며 Ctrl+O로 live Chat의 전체 출력을 전환한다. 실패 footer, 보존된
record, 일반 텍스트 출력, 이미 발행된 native scrollback은 완전한 내용을 유지한다.
`/preview` 안의 `long-tools`로 스트리밍을 확인한다. 긴 초안은 prompt viewport를
사용하고(대략 shell 높이의 1/3, 작은 화면용 최소 할당 상한 적용) 커서를 계속 보여준다.
입력 범위와 과거 기록 안내는 저장된 대화에 들어가지 않는다.

공식 [Codex CLI 화면](https://learn.chatgpt.com/docs/codex/cli),
[Claude Code 조작·transcript 안내](https://code.claude.com/docs/en/interactive-mode),
[pi 화면 구성·도구 펼치기](https://github.com/earendil-works/pi/tree/main/packages/coding-agent)를
비교한다. 절제된 metadata, 상황별 조작 안내, 상세 내용 펼치기의 참고 자료다.
yo의 청록·슬레이트 색감을 유지하면서 원칙을 적용하고, 조작 안내는 yo에 실제로
구현된 동작과 일치시킨다.

## 결과 읽기

- **Passed**: 적어둔 명령이 해당 환경에서 assertion을 성공적으로 실행했다.
- **Failed**: 명령이 실행되어 mismatch, timeout, panic, cleanup error를
  발견했다. 처음 실패한 소유 경계를 따라가고 뒤이은 cleanup failure도
  보존한다.
- **Unverified**: 필요한 환경에서 검사가 실행되지 않았다. coverage gap으로
  계속 보이게 둔다.

golden과 snapshot은 fixture의 정확한 Projection을 증명한다. 의도적으로
갱신할 때는 diff를 검토한다. 다시 생성했다는 사실만으로 새 출력이
올바르다고 판단하지 않는다.

## 에이전트에 반환하는 출력 제한하기

자세한 검증 출력이 에이전트 context로 돌아갈 때는
`tools/validation/bounded-run.sh`로 실행한다. wrapper는 command의 exit status와
합쳐진 전체 출력을 보존하고 worktree-local
`.local-exclude/validation-runs/` 디렉터리에 둔다. 성공하면 JSON summary 한 줄만
반환한다. 실패하면 같은 summary와 마지막 diagnostic output 최대 16 KiB를 반환한다.
그 tail만으로 소유 실패를 찾을 수 없을 때만 전체 local log를 확인한다.

기본 summary schema는 frozen `yo.validation-run-summary/v1alpha2`이다. 실행을 시작할 때의
`HEAD`, worktree가 clean이었는지, 정확한 command 인자 개수와 경계를 구분하는 hash,
전체 log의 byte 수와 SHA-256, `reviewed-descendant/v1` 재사용 정책을 기록한다.
따라서 Slice gate는 clean 후보의 결과를
선언된 command와 자체 결속된 evidence로 비교할 수 있다. dirty summary는 local
진단에는 쓸 수 있지만 후보 evidence로는 쓸 수 없다. summary는 실제 실행을
기록하므로 항상 `"reused":false`이며 이전 실행을 자동 탐색하거나 재사용하지 않는다.
후속 gate는 동일한 정확 command의 통과 summary이고 trusted Git이 clean 실행 HEAD를
검토된 최종 후보의 조상으로 증명할 때만 `"reused":true`를 선언할 수 있다. frozen
`yo.validation-run-summary/v1`과 `v1alpha1` artifact는 원래 의미로 gate 호환성을
유지하며 v1alpha1은 재사용을 허용하지 않는다.

결과가 local 저장소 byte만으로 결정되는 command에는 `--reusable-local`을 추가한다.
이 opt-in은 `reviewed-descendant-context/v1` 정책을 가진
`yo.validation-run-summary/v1alpha3`을 출력한다. v1alpha2 결속에 더해 target OS,
architecture, Rust/Cargo toolchain fingerprint를 기록한다. 후속 reused gate에서 Yo는
이 값을 다시 관측하고 달라졌으면 fail-closed한다. `external_state:"none-declared"`
선언은 network, clock, account, service 또는 그 밖의 external state에 의존하는
command를 제외한다. 그런 command는 재실행한다. 이 옵션은 이전 receipt를 탐색하지
않으며 기존 summary를 변경하지 않는다.

stdout을 복사하지 않고 review와 gate preparation에 쓸 summary를 보존하려면 ignored
부모 디렉터리를 만들고 직접 발행한다.

```bash
mkdir -p .local-exclude/coordination/<slice>/validation
bash tools/validation/bounded-run.sh \
  --summary-out .local-exclude/coordination/<slice>/validation/workspace-tests.json \
  --reusable-local \
  workspace-tests -- cargo test --workspace --all-targets
```

output file과 stdout 한 줄은 byte-identical하다. 발행은 atomic create-only다. 부모가
없거나 target이 이미 있으면 validation command 전에 중단하고, 동시에 생긴 target도
덮어쓰지 않는다. 발행한 파일을 immutable review packet에 추가하면 manifest가 경로와
hash를 `slice gate prepare`에 제공한다. 이는 새 evidence 저장만 수행하며 이전 결과를
재사용하지 않는다. 재사용 판단은 이 runner가 아니라 후속 검토가 끝난 Slice gate
request가 소유한다.

wrapper는 표시만 바꾸고 검증 의미는 바꾸지 않는다. log는 임시 운영 artifact다.
필요한 실패 log는 finding이 미해결인 동안만 보존하고, 완료한 log는 Slice
worktree와 함께 폐기한다.

## 한 후보의 게이트 통합하기

이 절은 formal Slice에만 적용한다. 일반 작업은 검사 결과와 한계를 직접
보고하며 gate request나 immutable evidence chain을 만들 필요가 없다.

`cargo xtask slice status <slice>`는 packet 없이 직접 검토한 경우에도 현재
후보의 gate 요청을 재사용한다. 다음 단계가 gate 실행이고 일치하는 요청이 하나면
`run_gate`, `next_argv`,
Slice의 `next_working_directory`를 반환한다. 반환된 디렉터리에서 해당 argv를
실행한다. 요청이 여러 개면 명시적으로 선택해야 한다. 요청 발견은 검증이나
승인이 아니다. gate가 증거를 다시 확인하고 실제 다음 행동을 반환한다.
후보가 dirty이거나 발행된 검토의 조상 관계가 깨졌다면 먼저 해결해야 한다.

Slice 후보가 깨끗한 commit이 되면 bounded validation JSON summary와 각 최종
review 응답을 별도 local file로 저장한다. 정확한 hash, 후보 commit, canonical diff
hash, 필수 lens, 알려진 미검증 환경, 위험 분류, human-origin 승인을
`yo.slice-gate-request/v1alpha1` request에 기록한 뒤 다음을 실행한다.

```bash
cargo xtask slice gate /tmp/<slice>-gate.json
```

선언한 검사 하나와 완료한 lens 하나가 있는 최소 request 형태는 다음과 같다.
검사나 lens가 더 필요하면 해당 evidence entry를 반복한다.

```json
{
  "schema": "yo.slice-gate-request/v1alpha1",
  "candidate_commit": "<full-commit>",
  "required_lenses": ["fresh-context"],
  "validation_evidence": [{
    "name": "workspace-tests",
    "argv": ["cargo", "test", "--workspace", "--all-targets"],
    "result_path": "/tmp/workspace-tests.json",
    "result_hash": "sha256:<summary-hash>",
    "candidate_commit": "<full-commit>",
    "reused": false
  }],
  "review_evidence": [{
    "lens": "fresh-context",
    "reviewer": "provider/session",
    "route": "model-high/provider/model/session",
    "verdict": "clear",
    "candidate_commit": "<full-commit>",
    "diff_hash": "sha256:<canonical-diff-hash>",
    "result_path": "/tmp/fresh-context.txt",
    "result_hash": "sha256:<response-hash>"
  }],
  "known_unverified_environments": [],
  "risk": {
    "classification": "human-attention",
    "rationale": "changes workflow authority"
  },
  "approval": null
}
```

사람이 exact approval을 완료하면 `null`을 `kind: "exact_candidate"`,
`human/<identity>` authority와 scope, 같은 `candidate_commit`과 `diff_hash`로
교체한다. routine request는 human-origin scope가 이 작업을 포함하고 미검증 환경이
남지 않았을 때만 `kind: "standing_routine"`을 사용하고 두 exact identity field를
생략할 수 있다.

이 command는 결속된 Slice 범위, 깨끗한 `HEAD`, 경로에서 도출한 최소 lens,
evidence file hash, 후보/diff identity, review route, approval 형태를 확인한다.
그리고 `validate`, `review`, `approve`, `integrate` 중 정확히 하나를
`next_action`으로 담은 `yo.slice-gate-result/v1alpha1` JSON 한 줄을 반환한다.
그 행동 자체를 실행하지는 않는다. 후보 변경, stale diff, 변조된 evidence file,
경로 기반 lens 누락은 다음 행동을 만들지 않고 fail-closed된다.

이는 증거 일관성 검사이지 선언이 참이라는 증명은 아니다. validation plan의
완전성, semantic review lens, 위험 분류, 기록한 verdict의 정확성은 여전히
coordinator가 소유한다. request와 evidence는 ignored coordination storage 또는
worktree 밖에 두고 Slice 종료 시 제거한다.

## Slice 종료 기준선

이 절은 명시적으로 선택한 formal Slice에 적용한다. 일반 작업은 영향받는
package와 consumer 검사, 필요한 문서 검사, `git diff --check`로 마무리한다.
공유 runtime/build 변경, release, package 간 영향이 불명확한 경우에는 전체
workspace suite를 실행한다. 무관한 문서 변경이나 국소 구현 변경에는 요구하지 않는다.

Formal Slice에서는 집중 검사가 통과하면 저장소 기준선을 실행한다.

```bash
bash tools/validation/bounded-run.sh workspace-tests -- cargo test --workspace --all-targets
bash tools/validation/bounded-run.sh workspace-clippy -- cargo clippy --workspace --all-targets -- -D warnings
bash tools/validation/bounded-run.sh hk-candidate -- \
  hk check --check --from-ref BASE_SHA --to-ref CANDIDATE_SHA
```

`cargo test`는 일반 test를 실행하고 ignored test를 compile하지만, 환경
의존 ignored test를 실행하지 않는다. `hk check`는 변경 경로에 따라
`hk.pkl`에서 저장소 검사를 고른다. formatting, test 설명, 영향받은
crate 검사, Methexis 검사, Developer Docs 검사가 여기에 포함된다.
설치와 hook 사용법은
[`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#local-checks)가
소유한다.

staged Methexis 변경이 `methexis/sources/`와 `methexis/knowledge/` 경로로만
이루어지면 hook은 먼저 작업 중인 Methexis tree가 index와 정확히 같고 추적되지
않은 Methexis 경로가 없는지 확인한 다음 `records`와 `relations` class만 실행한다.
따라서 이전 Projection이 의도적으로 stale인 semantic-first 후보를 commit할 수 있다.
staged Projection, approval, Checkpoint, active record 또는 그 밖의 Methexis 경로가
하나라도 있으면 authority를 포함한 완전 validation 경로를 그대로 사용한다.

편집 중에는 로컬 Slice contract에 선언한 집중 검사를 사용하고, 결과가
완성되면 이 Slice 종료 기준선을 한 번 실행한다. 정확한 Methexis activation
후보가 staged된 구간에는 `hk`가 prospective validation을 사용하고 일반
Methexis test를 잠시 미룬다. 통합 직후에는 trusted `develop`에서 일반 전체
Methexis check와 test를 실행한다.

해당 activation worktree는 clean `develop`에서
`cargo xtask slice create-activation <request.json>`으로 준비한다. 생성된
contract는 active record, Checkpoint tree, 등록된 context manifest 두 개를
lease한다. 집중 검사인 `methexis check --staged-activation`은 새 immutable
Checkpoint를 정확히 하나만 허용한다. Slice 생성은 coordination setup일 뿐
prospective transition이 유효하다는 증거가 아니다.

이후 독립 activation을 검수할 때는 이를 지원하는 workflow 구현이 이미 trusted인
경우에만 명시적인 v1alpha3 review request를 쓴다. 위 집중 test는 trusted-capability
bootstrap, 정확한 activation-only 경로 경계, proposal identity와 canonical packet 재생을
확인하지만, 후보는 통합 전 staged activation 검증과 통합 직후
일반 전체 Methexis 검증을 여전히 거쳐야 한다.

Slice가 platform이나 외부 환경 경계를 바꾼다면 기준선이 이를 검사했다고
주장하지 말고 관련 matrix 명령을 추가한다.

검수한 후보를 squash했다는 이유만으로 바뀌지 않은 기준선을 다시 실행하지
않는다. 새 fast acceptance는 integration HEAD가 candidate base이고,
`hk check --check --from-ref BASE_SHA --to-ref CANDIDATE_SHA`의 두 SHA가 gate의
정확한 base와 candidate이며, 결과가 성공·비재사용·외부 상태 없음으로 기록됐고 OS, architecture,
Rust/Cargo fingerprint가 여전히 일치할 때만 candidate-bound 결과로 중복 commit
hook을 대신할 수 있다. 이 선택은 `candidate_hk_receipt`로 기록한다. 그 밖에는
`git_hooks`를 기록하고 hook을 실행한다. 이 재사용은 후보 자체의 검증이나
검수를 대체하지 않는다.

새 `slice accept prepare`는 `yo.slice-accept-prepare-request/v1alpha3`을 사용한다.
ready gate와 사람이 작성한 message source만 필요하고 `push_remote`는 선택 사항이다.
이 명령은 작은 `yo.slice-close-prepare-request/v1alpha2`와 candidate, validation,
review evidence 수를 파생한다. compact 경로는 미검증 환경의 누락 command를 파생할
수 없으므로 알려진 미검증 환경이 없어야 한다. 그 매핑을 보존해야 하는 gate는 고정된
observed-metrics 경로를 사용한다. 따라서 사람이 실행 lane,
packet, 경과 시간 합계를 다시 만들어야 cleanup이 진행되는 병목이 없다. 고정된
이전 요청은 기존 observed-metrics 모양을 유지한다. close preparation은 표준
`close-metrics.json`을 발행하지만 cleanup을 직접 plan하거나 apply하지 않는다.

close plan은 요청한 파일에 plan을 직접 발행한 뒤 이미 수용된 결과를 소비한다.
로컬 worktree, 표준 임시 Slice contract, Slice branch를 제거하기 전에 정확한
ref, 검수 trailer, patch identity, worktree 청결 상태, binding, contract hash,
plan hash를 다시 검사한다. 전체 metrics 파일을 직접 작성하는 방식도 계속
지원한다. plan은 이
기록을 정확한 Slice candidate와 accepted commit에 결속하며 apply는 변경된
metrics를 거부한다. plan은 metrics를 포함해 보존할 직계 coordination 항목도
모두 나열하며, apply는 그 목록이 바뀌면 거절하고 해당 항목을 삭제하지 않는다.
plan은 제거할 worktree와 해당 Slice coordination 디렉터리 바깥에 저장한다.
통합 workflow는
[`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#review-and-integration)를
참고한다.

## 유용한 소유자

- hook 선택: [`hk.pkl`](https://github.com/Yon-Fandorin/yo/blob/develop/hk.pkl)
- 구조화된 저장소 검사: [`tools/xtask`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/xtask/src/lib.rs)
- Unix host compile 검사: [`tools/validation/yo-cli-unix-matrix.sh`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/validation/yo-cli-unix-matrix.sh)
- rendering parity fixture: [`crates/yo-tui/tests/fixtures/rendering-parity/README.md`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/tests/fixtures/rendering-parity/README.md)
- test 설명 정책: [`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#test-code)


### Rich content preview

`/preview` 안에서 `showcase`는 `syntax`, `charts`, `images`, `media-errors`를
함께 보여준다. `image /absolute/path.png`는 명시한 일반 로컬 파일(최대 1 MiB)만
이 오프라인 프리뷰로 읽으며 파일을 수정하지 않는다. 일반 Markdown layout은 경로나
URL을 가져오지 않는다. 내장 PNG/JPEG data URI는 인코딩된 이미지 파일 1 MiB,
각 변 2048픽셀, decoder 할당 예산 32 MiB로 제한한다.
PNG/JPEG EXIF 회전·반전은 썸네일과 원본 해상도 PNG 변환 전에 적용하며 표시 크기도
보정된 방향을 따른다. 작은 썸네일은 원본 크기를 넘겨 확대하지 않는다.
이미지 색상 능력은 코드 글자색과 독립적이다. 코드색을 terminal/기본색 또는 RGB로
바꿔도 이미지 셀과 native raster 조건을 유지한다. 단색·색상 미확인 fallback은 유지한다.
Rust 프리뷰 `--terminal-code-text`와 `images` 또는 `image-orientation`으로 확인한다.
`image-orientation` 프리뷰는 옆으로 저장된 JPEG를 바로 세워 표시한다.
크기를 제한한 썸네일 네 개와 원래 해상도의 PNG payload를 캐시해 reflow에 재사용한다.
셀 이미지는 최대 64열·20행이며 단색·ASCII에서는
명암 문자를 쓴다. Fullscreen은 원래 해상도의 PNG를 Kitty
프로토콜로 전송할 수 있다. JPEG와 EXIF 방향 보정이 필요한 이미지는 변환하며,
변환이 필요 없는 PNG는 원본 바이트를 유지한다. 이미지 전체가 보이고 overlay가 덮지 않을 때 배치하며,
잘리거나 가려지면 셀 표시를 쓴다. Inline은 셀 표시를 유지한다. 확인 가능한 직접 연결
Kitty 호환 터미널은 자동 사용하며 `YO_TUI_IMAGE_PROTOCOL=cells`로 끈다. 알 수 없는
터미널과 tmux는 셀 표시가 기본이다. 외부 지원과 `allow-passthrough on`을 확인한 운영자가
명시적으로 `kitty-tmux`를 선택하면 DCS passthrough를 사용한다. iTerm2/Sixel encoder와
모델 이미지 입력은 포함하지 않는다. 프로토콜 바이트 테스트는 실제 터미널 픽셀 호환성의
증거가 아니다.

언어 fence는 Syntect의 내장 문법을 쓴다. 미지원 언어, 64 KiB를 넘는 블록,
4096바이트를 넘는 줄은 구문 색상 없이 원문 코드로 남긴다. `chart` fence는
비어 있지 않은 `label: finite-number` 행을 최대 64개, `sparkline` fence는 공백으로
구분한 유한한 숫자를 최대 64개 받는다. 잘못된 데이터는 원문을 유지한다. 차트 수치는
항상 표시하며 음수 막대는 같은 0축을 공유한다. 검증한 샘플은 그래프 계산용
유한값과 입력 숫자 토큰을 따로 보존한다. 막대의 숫자는 좁은 폭과 정렬된 폭 모두에서
지수 표기·명시적 양수 부호·음의 0·소수점 뒤 자릿수를 유지한다. 전체 원문·plain output 보존은
화면 projection과 별도로 검증한다.


구성 요소의 owner는 `transcript/layout/markdown/code.rs`(구문 역할), `chart.rs`(검증된
수치와 반응형 차트, 막대는 meter 사용), `image.rs`(제한된 디코딩·캐시와 셀 대체)다.
`surface/raster.rs`는 프로토콜과 독립적인 PNG 배치 metadata를 소유한다.
`terminal/graphics.rs`는 Kitty chunk, 프로세스별 이미지 ID, passthrough와 삭제를
소유한다. Fullscreen frame은 출력 성공 후 commit하며, 셀 쓰기가 이미지와 겹치면
원본 배치 metadata를 무효화한다. 실제 지원 터미널에서 크기 변경, 오래된 배치, 실패한
frame 재시도와 정리를 확인한 뒤 native 호환성을 주장한다.

`stepchart`는 같은 유한 수치 배열을 받아 다음 표본까지 값을 수평 유지한 뒤 수직으로
변경한다. 표본 간격은 동일한 순번이며 타임스탬프를 추측하지 않는다.
[Plotly 선 차트 문서](https://plotly.com/javascript/line-charts/)의 hv 표현을 참고했다.
기존 chart 색상 역할, 값·표본 축, 64개 제한, 원문 대체와 좁은 폭의 sparkline을 공유한다.
`charts` 프리뷰에 동시 작업 수 예제가 있다.

`linechart`는 값·샘플 축을 가진 연결된 Braille 선 차트를 추가하며 좁은 폭에서는
sparkline으로 전환한다. 막대는 공간이 있으면 이름·값 열을 정렬하고 좁으면 이름을
위로 쌓는다. 양수와 음수는 같은 0축을 공유한다.
추세 정규화는 0과 극소 유한값의 차이를 유지하고 반대 부호의 큰 값 사이에서도
오버플로를 피한다. 선 차트는 실제 지수 표기 라벨 폭을 확보해 여섯 축 행을 정렬한다.
마지막 값 안내는 지수 표기를 수백 자리 소수로 펼치지 않고 입력 숫자 토큰을 유지한다.
`charts` 프리뷰에 두 수치 경계 사례를 포함한다.


### 출력 설정과 소스 비교

공개 `OutputPreferences`는 세션의 레이아웃 입력이다. CLI는 config.yaml의
`tui.max_body_width`(양의 u16), `tool_head_rows`, `shell_tail_rows`, `diff_head_rows`(u16)를
`TuiSession::with_output_preferences`로 전달한다. 해석된 값은 appearance snapshot이
소유하고 컴포넌트는 CLI·파일 경로가 아닌 레이아웃 설정을 받는다. 1열 본문은 한글 표시를 위해 2열로 조정한다. 기본 앞부분 행 수는 2·6이다.
접힌 activity는 지정한 앞부분과 마지막 세 행을 남기며 앞부분+6행을 넘고 안내 행이 공간을
절약할 때만 접는다. 앞부분 0행을 허용하고 포화 연산으로 최대 u16도 유효하게 처리한다.
`/changes`는 접기를 우회한다. `runner/tests/appearance.rs`는 실제 frame, 도구·diff 독립 설정,
개행, 원문 보존과 테마 변경 후 설정 유지를 검사한다. CLI는 0열·음수·범위 초과·오타를 거절한다.

`show_images`는 기본 true이며 false이면 디코딩을 생략하고 대체 설명과 표시 해제 안내를 남긴다.
`image_max_width`는 양의 u16으로 기본값은 64이며 더 큰 값은 64로 제한한다.
두 옵션은 `OutputPreferences` builder로 전달되고 테마 변경 후에도 유지된다.
프레임 테스트는 1·8·64열 배치, 숨긴 이미지의 raster 제거, 원본 PNG 바이트와 일반 출력 보존을
검사한다. 숨긴 잘못된 이미지도 디코딩하지 않으며 잘못된 bool과 0·범위 초과 폭은 거절한다.


비교 소스는 pi `7d8ab31a477ecc07b36f56ffcae58c79307a68be`, Codex
`e1eb98461cd42730e7b1a890c3100303d7091bc2`이다.
[pi 출력 컴포넌트](https://github.com/earendil-works/pi/tree/7d8ab31a477ecc07b36f56ffcae58c79307a68be/packages/coding-agent/src/modes/interactive/components),
[Codex history cells](https://github.com/openai/codex/tree/e1eb98461cd42730e7b1a890c3100303d7091bc2/codex-rs/tui/src/history_cell)를 참고한다.
이 비교는 기능 동등성의 증거가 아니다. 추가 도구별 콘텐츠 스타일, 애니메이션 상태 palette,
추론 표시 선택과 일부 host 관측은 구현·검증이 남아 있다.

Codex webSearch는 ToolOutput을 담은 ToolCall snapshot으로 검색어·열린 URL·찾는
문자열을 보존하며, 커스텀 렌더러에 원본 query/action JSON을 제공한다. 선택적인 results가
null이 아니면 명시적인 빈 배열과 미지 결과 형식·필드까지 result 객체와 원문 내보내기에
보존한다. 생략·null이면 결과 목록을 만들어 넣지 않는다. 앱 서버 스키마는 개별 결과를
불투명한 JSON으로 정의하므로 기본 화면에는 원문 JSON을 표시하고 커스텀 렌더러에
전체 값을 제공한다. action 상세가 없거나 비어 있으면 항목의 query를 사용한다.
알 수 없거나 잘못된 action 필드도 읽을 수 있는 JSON으로 유지한다. 기존 본문 폭·접기·
코드 색상을 적용하며 URL이나 이미지를 가져오지 않는다. 타입 지정 콜백, 80/24/80 화면,
원문 내보내기와 wire 시작·완료를 검증한다. 구조화된 인용 해석은 남은 작업이다.

공개 `item/reasoning/summaryTextDelta`는 완료 전에도 기존 ModelWork activity를 갱신한다.
문단 번호별 내용을 따로 누적해 순서대로 snapshot을 보내며 초기 요약을 보존하고 완료 요약으로 교체한다.
변경 전에 thread·turn·item 식별자와 reasoning item 소유 관계를 검사한다.
원시 `item/reasoning/textDelta`는 이 표시 경로에 포함하지 않는다.
추론 완료는 `summary` 배열만 보존하며 공개 요약이 없다고 raw `content`를 대신 쓰지 않는다.
`runtime::tests::coding_events`가 wire→semantic payload를 검사한다.
`search`, `reasoning`은 모델 호출 없는 출력 프리뷰다.


`ThemeOverrides`는 기본 팔레트와 독립적인 `ThemeRole` 색상을 제공한다. `ThemeColor`는 RGB와
터미널 기본색을 지원하고 CLI `tui.colors`는 정확한 snake_case 역할명과 따옴표로 감싼
`#RRGGBB`/`terminal` 값을 사용한다. appearance snapshot이 기본 테마와 재정의 값을 함께
보존하며 테마 변경 시 재적용하고 빈 재정의로 기본 팔레트를 복원한다. RGB는 제한된 터미널에서
가장 가까운 고정 256색 cube·grayscale 색으로 변환하며 색상 미지원과 Mono는 기본색을 쓴다.
강조 속성과 레이아웃은 보존한다. `runner/tests/appearance.rs`는 TrueColor/Limited/Unknown과
테마 왕복에서 실제 사용자·코드·diff 셀을 검사하고 설정 테스트는 잘못된 역할·RGB·비ASCII
문자열을 터미널 시작 전에 거절한다. 오프라인 Rust 예제의 `--custom-colors`가 공개 API를
사용한다. 애니메이션 gradient와 도구별 renderer 확장은 아직 별개로 남아 있다.
참고한 pi의 고정 커밋 `theme/theme.ts`도 의미별 전경·배경과 색상 인코딩을 분리한다.

완료된 ModelWork 요약은 소유한 진행 제목을 완료·중단·실패 제목으로 바꾸고 redraw를 요청한다.
본문의 같은 문구는 보존하고 Plan·사용량은 자체 제목을 유지한다. activity 투영과 inline
publication 테스트로 이 종료 전환을 검사한다.


MCP 결과와 동적 도구의 `inputText` 블록은 실제 줄바꿈을 유지해 표시한다.
텍스트 리소스는 URI·본문·메타데이터를, 리소스 링크는 식별 정보와 URI를 표시하며 가져오지는 않는다.
`structuredContent`·확장 필드·미지원 또는 잘못된 블록은 JSON fallback으로 보존한다.
`coding_events`는 혼합 콘텐츠·동적 결과·빈 콘텐츠·미지의 형식이 소비되는 snapshot을 검사한다.
`/preview mcp`는 리소스 출력을 기존 폭·접기 스타일로 보여준다.
MCP·동적 도구의 내장 PNG/JPEG 블록은 아래 ToolOutput profile과 native·셀 이미지 경로를 사용한다.
원격 URL을 가져오지는 않는다.


`ToolRenderer`·`ToolRenderInput`은 `TuiSession::with_tool_renderer`를 통해
세션별 도구 본문 Markdown 콜백을 제공한다. `expanded`는 세션의 활동 펼침 요청 상태이며
렌더러는 접힌 요약과 펼친 상세를 다르게 반환할 수 있다. Ctrl+O는 같은 콜백 핸들이어도
레이아웃 설정을 바꿔 캐시를 갱신한다. 반환 본문에는 일반 호스트 접기 규칙도 적용한다.
독립 실행 `--custom-tools` 예시가 docs.search에 이를 사용한다.
`outcome: Option<TranscriptActivityOutcome>`은 실제 관측한 완료·실패·중단을 전달한다.
None은 아직 종료 관측이 없다는 뜻이며 물리 실행이 시작됐다는 증거가 아니다. 본문 단어나
구조화 error 필드로 상태를 정하지 않는다. 원문이 같아도 결과 변경은 항목 배치를 갱신한다.
`mcp-failure`는 일부 결과와 실패 상태를 보여주며 독립 실행 예시의 접힌 요약에도 반영된다.
기존 activity presentation이
ToolCall·ToolResult 종류를 보존하고 layout이 해당 본문에만 콜백을 적용한다.
상태 제목·결과 footer는 콜백 바깥에서 유지하고 일반 출력은 콜백을 우회한다.
`None` 반환 또는 렌더링 불가능한 내용은 원문으로 되돌린다.
콜백은 순수하고 결정적으로 동작해야 한다. 핸들은 공유 할당 식별자로 비교하므로
교체하면 레이아웃 캐시를 갱신하고 복제한 핸들은 같은 식별자를 유지한다.
테마·출력 설정 변경은 핸들을 보존하며 사용자 Markdown에도 이미지 표시·폭 제한을 적용한다.
접기 구간과 겹친 native raster는 제거하고 완전히 남은 것은 셀과 함께 이동한다.
프레임 테스트는 코드·표·PNG, 실패·원문 보존, 표시·폭 설정, fallback·핸들 해제,
다른 activity 제외와 접힌 raster 위치를 검사한다. Rust 오프라인 예시는 `--custom-tools`를 지원한다.
콜백은 검증된 구조화 데이터를 `output: Option<&ToolOutput>`으로 함께 받는다.

`yo_core::ToolOutput`이 정확한 `yo.tool-output/v1` 표시 profile을 소유한다.
`to_snapshot`·`from_snapshot`은 인코딩 크기를 16 MiB로 제한하고 빈 도구 이름,
미지 envelope·output 필드와 다른 schema를 거절한다. 원본 arguments·result·contentItems·error
JSON은 보존한다. Codex MCP·동적 도구 snapshot은 이 profile을 사용하며 일반 텍스트 투영은
내장 미디어의 base64 대신 형식·크기를 안내한다. Journal enum·discriminator·record 필드는
변경하지 않는다. Profile은 기존 message segment·seal로 저장하는 텍스트다.
Durable 테스트는 여러 segment를 복구해 같은 JSON·미디어 바이트인지 검사하고,
admission 테스트는 정확한 상한과 첫 초과 바이트를 검사한다.

ToolCall·ToolResult 표시 경로에서만 profile을 해석한다. 기본 출력은 JSON 인수 패널,
리터럴 텍스트 코드 패널, 리소스 본문과 명시적인 PNG/JPEG 이미지를 사용한다.
도구 텍스트의 fence가 Markdown 미디어로 빠져나가지 않도록 처리한다.
동적 도구의 data-image URL도 같은 decoder를 사용하며 원격 URL·오디오는 텍스트 안내를 남긴다.
원본 JSON은 콜백과 journal에 남고 미지 schema·잘못된 데이터는 일반 텍스트로 표시한다.
일반 출력은 `plain_text`를 사용하며 원본 record는 전체 profile을 유지한다.
프레임 테스트는 기본 MCP·동적 이미지, 원본 PNG, 표시 해제·폭 설정, 일반 출력과 타입이 있는
콜백 입력을 검사한다. `mcp-image`는 128×64 PNG를 담은 오프라인 snapshot 예시이며
모델 호출이나 이미지 입력을 의미하지 않는다.


`ActivityNotice`는 title·리터럴 message·info/warning level을 가진 정확한
`yo.activity-notice/v1` ModelWork 표시 profile을 소유한다. 도구 출력과 같은
크기 제한 텍스트 snapshot을 사용하며 journal record variant를 변경하지 않는다.
종류가 보존된 ModelWork 표시 경로에서만 안내를 해석한다. 일반 ModelWork의 스타일과
접지 않는 동작은 유지하며 안내 제목은 warning·heading 팔레트를 사용한다.
전달된 안내가 “Model work completed”로 바뀌지 않고 일반 출력도 읽을 수 있다.
테스트는 정확한 schema·level·필드 해석과 완료 후 경고색 프레임을 검사한다.
안내 전달이 중단·실패하면 소유한 footer를 profile 바깥에 유지하며 빈 delta가
안내 식별을 지우지 않도록 한다.

Codex `error`의 `willRetry: true`는 턴 완료 전에 안내를 내보내고 해당 턴에 저장한
이전 오류를 제거한다. 턴을 종료하거나 별도 모델 호출을 시작하지 않는다.
이후 최종 실패에는 최종 오류 증거를 사용하고 완료된 턴의 뒤늦은 재시도 안내는 무시한다.
willRetry가 없으면 기존의 재시도 아님 해석을 유지하며 bool이 아니면 protocol 오류로 거절한다.
백엔드 테스트는 성공·실패 종료와 뒤늦은 이벤트 제외를 검사한다.
`retry`는 오프라인 프리뷰 예시이며 재시도 횟수·대기 시간을 추측하지 않는다.
세션 범위 `warning`·`configWarning`·`deprecationNotice`·`guardianWarning`은 기존 observer로 전달한다.
`CodexWarning`과 소스 호환 별칭 `CodexCompatibilityWarning`을 제공한다.
전역 경고는 호출 중에도 관측하며, 스레드 경고는 검증된 바인딩을 기다리고 polling에서
다른 스레드 경고를 제외한다. 표시 본문은 8 KiB로 제한하고 제어 문자를 이스케이프하며
한 번의 client poll은 최대 32개 경고를 소비한다. CLI collector는 중복을 제거하고
서로 다른 경고 32개와 초과 안내 하나를 유지한다. TUI는 `AgentPoll::Notice`로 유휴 화면을
깨우고 제목·경고 색상을 보존하며 stderr와 publication 커서를 공유하여 중복하지 않는다.
세션 안내는 일시적인 transcript 항목으로 가짜 Turn 식별자나 journal record를 만들지 않는다.
print 모드는 stderr 진단을 유지한다. 테스트는 스레드 격리·상한·유휴 깨우기·한 번만 표시와
턴 시작 전 실제 프레임의 색상을 검증한다. `/preview warning`으로 모델 호출 없이 확인한다. 프리뷰 안에서
`deprecation`은 전환 안내를, `approval-warning`은 승인 검토 경고를 보여 준다.
두 예시는 Turn을 시작하거나 권한을 변경하지 않는다.
frontend poll 중 관측한 경고는 같은 poll의 원래 결과(종료·실패 포함)보다 먼저 전달한다.
세션이 원래 결과를 보관하므로 화면 재진입에도 유지되며 경고를 비우는 동안 agent 결과를
추가로 소비하지 않는다. 사용 중단 안내와 스레드 범위 승인 경고는 구분된 제목과 기존
경고·본문 색상을 사용한다. 알림 필드는 [Codex 프로토콜 소스](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/src/protocol/v2/notification.rs)를 따른다.


Codex `contextCompaction` 항목은 info 수준 `ActivityNotice`를 가진 하나의 ModelWork
활동으로 연결한다. 관측한 app-server schema에는 status·summary·토큰 수 필드가 없다.
`item/started`는 “Compacting context”, `item/completed`는 “Context compacted”로 같은
항목을 갱신하며 수치나 요약을 추측하지 않는다. 실제 thread·turn·item 바인딩과 기존 중단
정리를 따르고, 중단 후 뒤늦은 완료는 표시하지 않는다. 제목은 사용자 지정 `accent` 색상을
따른다. 백엔드 테스트는 진행·완료 순서와 중단을 검증하며 TUI 프레임 테스트는 같은 항목
교체·색상·원문 내보내기·profile JSON 비노출을 검사한다. `/preview compaction`에서 모델
호출 없이 전환을 볼 수 있다. 호스트의 자동 압축 관측이며 delegated `/compact` 명령을
지원한다는 뜻은 아니다. 제공된 요약 본문은 별도 명시적 profile로 표시한다.
pi는 요약과 압축 전 토큰 수를 제공하지만 이 Codex 항목에는 해당 데이터가 없다.


`ActivitySummary`는 크기를 제한한 `yo.activity-summary/v1` ModelWork 텍스트 profile이다.
`SummaryKind::Compaction`, `Branch` 또는 `Reasoning`, 전체 Markdown `summary`, 압축에 한해 선택적인
관측값 `tokens_before`를 가진다. 미지 종류·필드, 음수 수치, 분기·추론의 토큰 수, 16 MiB를
초과하는 첫 바이트를 거절한다. 표시 형식이며 압축 실행·분기 생성·journal record 변경을
하지 않는다. TUI는 accent 제목과 실패·중단 footer를 소유하고 빈 delta에서도 식별을
유지하며 빈 요약에는 안내를 표시한다. Markdown은 기존 코드·표·차트를 사용하지만 첨부
권한은 부여하지 않는다. `tool_head_rows`로 요약의 접힌 행 수도 조절하며 Ctrl+O로 도구와
요약을 펼쳐도 원문 내보내기는 유지된다. 본문 폭과 기존 팔레트 설정도 적용된다.
테스트는 profile 경계, 두 종류, 접기·펼치기, 실패 footer, 사용자 지정 accent, 원문 보존,
좁은 폭·빈 요약·이미지 대체 프레임을 검사한다. `/preview summary`와 `/preview branch`는
전체 snapshot 오프라인 예시다. 본문 없는 Codex contextCompaction에서 요약을 추측하지 않는다.


Codex 공개 추론의 시작·summaryTextDelta·완료는 명시적 Reasoning profile을 전달한다.
공개 요약의 희소 문단 순서를 유지하고 완료 요약으로 스트리밍 본문을 교체한다. 빈 공개
요약도 명시하며 raw reasoning content·textDelta를 대신 표시하지 않는다.
`tui.colors.reasoning_text`와 `ThemeRole::ReasoningText`는 공개 추론 본문과 숨김 안내의
색상을 별도로 지정한다. 기본값은 `muted`를 따르며 명시적인 추론 색상이 우선한다.
압축 요약·코드·문법 색상·상태 footer와 원문 내보내기는 기존 역할을 유지한다.
독립 색상, 표시·숨김 상태와 80/24/80 폭 변경을 실제 프레임에서 검증하고 CLI 설정
파서도 새 역할을 받는다. `tui.show_reasoning`은 기본 true다. `OutputPreferences::with_reasoning(false)`는 짧은
숨김 안내를 표시하며 실패 footer, 다른 활동, 저장 기록과 원문 내보내기를 유지한다.
설정을 바꾸면 레이아웃 캐시를 갱신하고 같은 원문을 복원한다. 테스트는 타입이 있는 백엔드
전달·스트리밍 순서·비공개 내용 제외·CLI bool 검증과 경고·계획·압축을 함께 둔 실제
숨김/표시/숨김 프레임을 검사한다. `chat_preview --hide-reasoning`으로 오프라인 확인한다.


Mermaid fence는 `markdown/diagram.rs`와 고정 버전
[`mermaid-text` 0.57.0](https://docs.rs/mermaid-text/0.57.0/mermaid_text/)으로 연결한다.
기본 배치를 만든 뒤 Yo의 셀 폭 규칙으로 모든 행을 측정하며 접두부·여백을 제외한 본문보다
넓으면 원본 코드를 유지한다. 라이브러리가 문서화한 좁은 라벨 겹침을 피한다.
코드 팔레트·배경·본문 폭·원문 내보내기는 Yo가 소유한다. `tui.show_diagrams`는 기본 true,
`OutputPreferences::with_diagrams(false)`는 fence 원문을 유지한다. 공개 추론 요약의
Mermaid는 다이어그램 설정과 관계없이 원문을 유지한다. 외부 렌더러 프로세스·브라우저·
이미지 프로토콜은 실행하지 않는다. 렌더링 전 입력을 8 KiB, 개행/세미콜론 구간 64개,
공백 토큰 512개, 소스 행당 512바이트로 제한한다. 결과가 256행·64 KiB를 넘거나
제어 문자·표시 불가능 문자가 있으면 원문을 표시한다. Gantt는 라이브러리가 생략 날짜를
현재 시각에서 추론할 수 있어 원문을 유지한다. style/classDef/class/click/linkStyle도
조용히 버리지 않고 원문으로 표시한다. 파싱 실패와 폭 제한에는 짧은 이유를 함께 표시한다.
흐름도·시퀀스·원문 대체·첫 초과 문장·팔레트·표시 설정·캐시 갱신을 테스트한다.
`/preview diagrams`는 실제 Rust 오프라인 예시다. 다른 Mermaid 종류는 고정 라이브러리
지원 범위를 따르며 Mermaid.js와 완전한 동등성을 주장하지 않는다.


`ActivityPlan`은 크기를 제한한 정확한 `yo.activity-plan/v1` ModelWork 텍스트 profile이다.
선택적인 리터럴 설명과 순서가 있는 `PlanStep { text, status }`를 가진다.
`PlanStepStatus`는 pending/in_progress/completed만 허용한다. Codex turn/plan/updated는
wire 상태를 명시적으로 매핑하고 잘못된 설명 타입을 거절하며 턴당 하나의 활동을 갱신하고
턴 종료 뒤 계획은 무시한다. journal 형식은 바뀌지 않는다. TUI는 완료/진행/대기 단계에
success/accent/muted를 사용하고 진행 단계는 굵게, 완료 수는 보고된 값으로 표시한다.
본문이 6열 이상이면 이어지는 줄을 단계 본문에 맞추고 더 좁으면 전체 리터럴 줄바꿈을 쓴다.
설명과 단계는 Markdown이나 첨부로 해석하지 않는다. 빈 계획은 비율 대신 “No steps provided.”를
표시한다. 활동 완료·실패는 보고된 단계 상태를 바꾸지 않고 실패 footer도 유지한다.
테스트는 profile 검증, 타입이 있는 백엔드 갱신 순서, 사용자 지정 프레임 색상, 좁은 폭,
빈 계획, 리터럴 원문 내보내기와 결과 보존을 검사한다. `/preview plan`은 같은 구조화된
출력 profile을 사용한다. 별도 proposed-plan Markdown 스트림은 아래 문서 profile을 사용한다.


`ActivityDocument { title, markdown }`은 정확한 `yo.activity-document/v1` ModelWork
텍스트 profile이며 비어 있지 않은 리터럴 제목과 16 MiB로 제한한 snapshot을 가진다.
기존 journal record와 요약 Markdown 표시 경로를 재사용하되 제안 계획을 압축·추론 요약으로
분류하지 않는다. 문서는 본문 폭·accent/코드 스타일·다이어그램 설정·Ctrl+O 접기를 공유하고
행동 승인이나 이미지 첨부 권한을 부여하지 않는다. Codex plan 항목은 시작 본문을 item
바인딩에 보존하고 검증된 `item/plan/delta` 조각을 합쳐 전체 문서 snapshot으로 전달하며
`item/completed` 본문으로 교체한다. 잘못된 대상 종류·스레드와 상한을 넘은 시작/delta/최종
문서는 protocol 검증에서 거절한다. 일반 assistant delta 경로는 기존 동작을 유지한다.
표시 조각을 저장하는 대신 최종 원문을 폭에 맞춰 다시 렌더링한다. 테스트는 타입이 있는
백엔드 순서·대상 거절·profile 식별, 좁은/넓은 프레임 재배치, 최종 본문의 초안 교체,
중단 footer와 추론 숨김 중에도 유지되는 원문 내보내기를 검사한다.
`/preview proposed-plan`은 실제 오프라인 Rust 에이전트에서 시간차를 둔 초안/최종 전환을
보여준다. 빈 문서는 텍스트 안내를 표시한다.

Codex `item/commandExecution/terminalInteraction` 알림은 독립 문서로 표시한다.
빈 stdin은 백그라운드 터미널 대기, 비어 있지 않은 stdin은 입력 전달을 표시하며
프로세스 ID와 알려진 명령을 포함한다. 명령 식별 정보는 항목 완료 후에도 턴 종료까지
유지하고 턴 완료 뒤 알림은 무시한다. 알 수 없는 명령, 다른 스레드·턴 대상, 문자열이
아닌 입력은 검증에서 거절한다. 입력 안의 모든 backtick 연속 길이보다 긴 fence로
Markdown과 제어 문자 원문을 보존하며 명령 stdout에 섞거나 새 입력을 보내지 않는다.
기존 코드 색상·본문 폭·접기를 적용한다. 백엔드 테스트는 대상 검증과 수명 주기를,
프레임 테스트는 좁은/넓은 재배치·리터럴 이미지 문법·제어 문자 표기·일정한 원문
내보내기를 검사한다. `/preview terminal-wait`와 `/preview terminal-input`은
프로세스와 상호작용하지 않는 오프라인 Rust fixture를 사용한다.

턴 종료 시 선택적 Codex `durationMs`(음수가 아닌 int64)를 `ActivityNotice`로
표시한다. 실제 완료·실패·중단 상태, 형식화한 소요 시간, 정확한 밀리초 값과 제공자를
함께 표시한다. 누락/null이면 시간 안내를 만들지 않고 0은 보고된 0으로 유지한다.
잘못된 값은 종료 상태 변경 전에 거절한다. 계획·항목 중단 정리 뒤 안내를 내보내고
마지막에 턴을 종료하며, 중복 완료 알림으로 둘을 다시 만들지 않는다. 기존 안내의
accent/경고 색상과 폭별 줄바꿈을 적용한다. 테스트는 세 결과, 누락/null/0, 정밀도,
int64 첫 초과 값, 순서, 중복 억제, 좁은/넓은 프레임과 원문 내보내기를 검사한다.
`/preview turn-duration`은 오프라인 예시이다. API·도구별 실행 시간이나 실제 시각을
추정하지 않는다.

기본 구조화 도구 렌더러는 정확한 `read`, `read_file`, `readFile` 이름과 문자열
`file_path`/`path` 인수의 확장자로 소스 언어를 추론하고 제목에 경로를 표시한다.
내장 텍스트 리소스는 URI 확장자를 사용하며 식별 정보와 코드를 분리한다. 알려진
프로그래밍 파일 확장자는 기존 Syntect 의미 색상·코드 배경을 사용하고 알 수 없는 확장자와
Markdown·다이어그램은 리터럴 텍스트로 남긴다. `error` 또는 결과의 `isError: true`는
성공한 소스로 취급하지 않는다. 표시에만 사용하는 확장자 추론이며 경로 해석·파일 읽기·
네트워크 요청은 하지 않는다. 원래 인수·결과 메타데이터·텍스트 내보내기는 유지하고
사용자 `ToolRenderer`가 우선한다. 프레임 테스트는 사용자 키워드 색상, 좁은/넓은
소스 줄바꿈, 리터럴 fence·이미지 문법, 오류·알 수 없는 도구의 대체 표시와 콜백 우선권을
검사한다. `/preview file-read`은 오프라인 Rust 결과를 제공한다. Managed 백엔드도
의미 검증 뒤 같은 profile을 제공한다. 쓰기 제안과 여러 파일 읽기는 아래 표시 경로를
사용하며 수정 제안과 내장 변경 결과 요약도 아래에 설명한다.


Managed 도구 완료는 현재 턴의 모델 기록에 이미 보존된 검증·마스킹된 함수 인수와
검증·출력 제한을 거친 문자열로 `ToolOutput`을 만든다. 실행 전용 원본 인수는 이 표시
경로에 들어오지 않는다. JSON·미디어처럼 보이는 결과도 단일 리터럴 텍스트 블록으로
전달하고, 메타데이터에는 호출·도구·호스트 식별과 완료·실패·중단 결과를 보존한다.
모델 기록과 다음 커넥터 요청은 정확히 같은 검증된 결과 문자열을 유지한다. 별도 보존용
출력이 없는 결과에서 표시용 확장
데이터가 16 MiB profile 상한을 넘으면 이미 수행된 작업을 실패시키지 않고 기존
호출 ID/출력 receipt로 표시한다. 시작 결과 메타데이터·승인·실행 일정은 유지한다.
테스트는 타입 표시·모델 기록·다음 요청의 마스킹된 인수/결과, 세 실행 결과와
미디어 모양 JSON의 리터럴 보존을 검사한다. 2 MiB 제어 문자 결과가 표시용 확장 뒤
상한을 넘더라도 모델 기록은 허용되고 원문 receipt로 표시되는 경우도 검사한다.
형식이 맞는 실행 메타데이터는 명령·결과 뒤의 **Execution details** 텍스트 블록 하나로
묶고 기존 출력 잘림 경고를 유지한다. 알 수 없거나 잘못된 필드는 기존 일반 표시를
유지한다. 20·40열 surface 테스트는 원본 snapshot·텍스트 내보내기·사용자 renderer 입력
보존도 확인한다. 코드 색상·사용자 렌더러는 구조화 TUI
프레임 테스트에서 검증하며 실제 제공자 호출을 의미하지 않는다.

정확한 `write`/`write_file`/`writeFile` 도구의 문자열 `content` 인수는
**Proposed file content**로 표시하고 경로 기반 코드 색상을 적용한다. 화면의 인수
JSON에서만 중복 content 필드를 빼며 원래 타입 인수와 텍스트 내보내기는 유지한다.
빈 내용은 빈 파일 안내를 표시하고 잘못된 content는 인수에 그대로 남긴다. 쓰기 실패
후에도 제안과 실제 결과·오류는 분리한다. 기존 본문 폭·코드 색상·접기·사용자 렌더러
우선권을 적용하며 렌더러는 파일을 쓰거나 파일시스템에서 diff를 계산하지 않는다.
Managed FunctionCallDone은 실행·승인 일정 전에 검증된 인수를 ToolOutput으로
전달하고 상한 초과 시 기존 receipt를 유지한다. 구조화 인수만 있고 결과·내용·오류가
없는 완료 ToolCall은 화면과 내보내기에서 **Tool call prepared**로 표시해 실행
성공과 구분한다. 테스트는 결과 전·실패한 제안, 빈/잘못된 내용, 좁은 줄바꿈, 사용자
키워드 색상·콜백 인수, 준비 제목과 검증된 값만 전달하는 백엔드 경로를 검사한다.
`/preview file-write`는 파일을 쓰지 않는 오프라인 예시이다.

정확한 `read_files` 도구의 내장 텍스트 결과는 순서가 있는 파일 패널로 표시한다.
완전한 `{"results":[...]}` 객체에서 파일별 path/status와 성공 시
start/end/total/content·선택적 next_offset, 실패 시 error 문자열을 읽는다.
보고된 줄 범위·언어별 코드·다음 읽기 위치·빈 파일·개별 오류를 표시한다.
본문 줄 수와 범위·이어서 읽기 정보의 일관성을 검증한다. 알 수 없는 필드·상태,
잘못되거나 잘린 JSON, 8개 초과 항목, 빈 결과 배열, 256 KiB 초과 입력은 전체 원문으로
표시한다. 작업공간에 이미 사용하는 serde_json 의존성을 이용하며 입출력은 수행하지 않는다.
타입 결과와 텍스트 내보내기는 유지하고 사용자 ToolRenderer가 기본 표시보다 우선한다.
프레임 테스트는 순서·사용자 코드 색상·80/24/80 줄바꿈, 빈/오류 상태, 잘못되거나
확장된 입력, 8/9개 항목과 정확한 바이트 상한/첫 초과 값, 원문 보존과 콜백 인수를
검사한다. `/preview files-read`는 Rust 파일 일부·읽을 수 없는 파일·빈 파일을
포함한 오프라인 예시이다.

정확한 `edit`/`edit_file`/`editFile` 도구의 유효한 oldText/newText 쌍은
**Proposed replacements**로 표시한다. edits 배열과 단일 쌍을 지원한다.
화면 인수에서만 인식한 교체 필드를 빼고 원래 타입 인수와 내보내기는 유지한다.
Diff 블록은 기존 추가·삭제 색상과 도구 접기를 사용하며 교체 번호·삭제와 old/new
텍스트의 마지막 개행 유무를 보존한다. 파일 위치를 만들어내거나 제안이 적용됐다고
표시하지 않는다. 빈 oldText, 알 수 없는 교체 필드, 모호한 형식, 256쌍 초과,
old/new 합계 256 KiB 초과는 리터럴 인수로 남긴다. 정확한 path/status/수치 필드를
가진 성공한 내장 edit_file/write_file 결과는 보고된 교체 수·쓴 바이트를 요약하며,
잘못되거나 확장된 결과·실패 결과는 원문을 표시한다. 결과 파싱은 16 KiB로 제한하며
파일시스템 입출력을 하지 않는다. 프레임 테스트는 사용자 diff 색상·좁은 재배치·실패·
삭제/개행·콜백 원본 인수, 256/257쌍과 정확한/첫 초과 본문 바이트, 성공/잘못된
변경 수치를 검사한다. `/preview file-edit`는 아무것도 적용하지 않았음을 명시하는
오프라인 교체 제안이다. 실제 파일 전체 diff는 파일 변경 이벤트와 호스트 증거가 담당한다.

정확한 `run_command`/`bash`/`powershell` 도구의 문자열 command 인수는
별도 코드 패널로 표시하고 화면 JSON에서만 중복 필드를 뺀다. timeout 같은 나머지
인수, 원래 profile·내보내기·콜백 인수는 유지한다. 빈 명령은 안내하고 잘못된 값은
JSON으로 남긴다. Bash 계열 명령은 기존 코드 색상을 사용하며 지원하지 않는 언어는
리터럴 코드로 표시한다. 정규 정수 또는 signal 상태와 모호하지 않은 stderr 구분자
하나를 가진 완전한 내장 run_command 결과는 종료 상태·stdout·stderr를 나눠 표시한다.
실패한 명령과 빈 스트림 안내도 지원한다. 잘림 표시·잘못된 머리말·반복 구분자가 있으면
합쳐진 원문을 유지하고 시간·채널 경계·프로세스 상태를 만들어내지 않는다. 코드 fence로
출력의 Markdown·미디어 문법이 해석되지 않게 한다. 프레임 테스트는 사용자 bash 키워드
색상, 80/24/80 줄바꿈, 상태/signal/빈 스트림/오류, 모호하거나 잘린 결과의 대체 표시,
제어 문자·리터럴 이미지 문법·정확한 콜백 인수를 검사한다. `/preview shell`은
프로세스를 시작하지 않는 오프라인 예시다. 별도 원시 스트림 식별 정보와 제공자의 전체
출력 파일 메타데이터 연결은 이후 확장 대상으로 남아 있다.

Managed 도구 완료는 실행 호스트의 잘림 보고 또는 로컬 출력 바이트 제한에 따른
잘림을 boolean `result.truncated`로 보존한다. 의미 검증과 모델 기록 문자열은 유지한다.
내장 `list_files` 표시는 이 값과 성공 결과를 요구하며 순서가 있는 리터럴 파일/디렉터리
행, 반환 항목/디렉터리 수, 빈 반환 안내와 명시적인 부분 목록 안내를 표시한다.
내장 계약에서 끝의 슬래시는 디렉터리를 뜻한다. 잘림이 보고됐을 때만 마지막 잘림
마커를 제거하므로 완전한 목록에서 같은 이름의 파일은 그대로 보인다. 메타데이터 누락,
제어 문자, 끝나지 않은 행, 1024바이트 초과 경로, 1024개 초과 항목, 256 KiB 초과
입력은 전체 원문을 유지한다. 디렉터리를 열거나 트리 계층·디렉터리 전체 크기를 추정하지
않는다. 코드 배경·폭·접기·사용자 렌더러 인수를 재사용한다. 백엔드 테스트는 호스트
잘림과 정확한/첫 초과 로컬 출력 상한을, 프레임 테스트는 좁은/넓은 재배치·빈/부분/미확인
상태·리터럴 이름·원문/콜백 보존과 항목/경로/입력의 정확한/첫 초과 상한을 검사한다.
`/preview files-list`는 오프라인 부분 목록 예시다. 실제 실행은 더 큰 목록도 허용하며
표시 상한은 실행 결과를 버리지 않고 리터럴 표시를 선택하는 기준이다.

내장 `resource` 블록은 URI·MIME 형식·본문과 함께 바깥 annotation·미지 필드 및
리소스 내부 메타데이터를 유지한다. 올바른 타입의 text/blob 본문 중 정확히 하나가
필요하며 잘못되거나 모호한 본문은 전체 리터럴 JSON으로 남긴다. 성공한 텍스트는 URI
확장자의 코드 하이라이팅을 유지한다. PNG/JPEG blob은 기존 이미지 디코더·표시·폭
설정을 사용하고 나머지 바이너리는 가져오거나 디코딩하지 않고 안내를 유지한다.
테스트는 내외부 메타데이터, 80/24/80 재배치, 잘못된 본문, 리터럴 텍스트, 원문 및
사용자 콜백 보존을 검사한다. 기존 구조화 이미지 테스트도 내장 PNG 바이트와 지정 폭,
이미지 표시/숨김을 확인한다. `/preview embedded-resource`는 오프라인 예시다.

`tui.colors.chart` / `ThemeRole::Chart`는 차트 선·막대·축 색상을 별도로 지정한다.
생략하면 명시적으로 바꾼 accent도 그대로 상속한다. 차트 색상을 지정하거나 해제해도
제목 스타일과 원문은 유지되며 프레임 테스트는 80/24/80 선·막대 재배치와 설정/해제를
검사한다. Rust 프리뷰의 `--custom-colors`는 보라색 제목과 호박색 차트를 표시한다.

이름 있는 `linechart`·`stepchart`·`scatterchart`는 선택적인 `height: N` 다음에
계열별 `이름: 값들` 행을 사용한다. 제어 문자 없는 고유 이름 1–4개(각 1–64 UTF-8 바이트),
계열별 유한 표본 1–64개를 허용한다. 선·계단은 표본 수가 같아야 하고 산점도는 달라도 된다.
모든 계열은 X/Y 범위를 공유하며 선·계단의 X는 0부터 시작하는 표본 인덱스다. 번호가 있는
점은 범례와 대응하고 `×`는 정확한 수학적 교차를 주장하지 않고 그리기 격자 점의 중첩을 뜻한다.
같은 문자 셀 안에서 점이 다르면 Braille 모양을 합치고 중립 색상으로 표시한다.
색상은 `chart`·`chart_2`·`chart_3`·`chart_4` / `ThemeRole::Chart`부터 `Chart4`를 쓴다.
흑백에서도 점 번호를 유지하며 ASCII는 Braille을 `*`, 중첩 표시를 `x`로 표시한다.
좁은 폭은 범례·범위·계열별 원래 수치를 유지한다. 중복·초과 이름, 초과 계열/표본,
비유한 숫자, 선·계단의 불균일한 표본 수는 전체 원문으로 돌아간다. 테스트는 공통 축,
불균일 산점도 X, 점 색상, 중첩, 상수·극단 값, 상한·첫 초과, 좁은 폭, 테마와 원문을
검사한다. `/preview chart-series`는 세 계열을 비교한다. 프로바이더별 계약을 추가하지 않는다.

MCP `resource_link` 블록은 이름·주소와 선택적인 제목·MIME 형식·바이트 크기,
리터럴 설명 및 나머지 메타데이터를 구분한다. 필수 이름·주소와 선택 필드의 타입을
검증하며 잘못된 필드나 256 KiB 초과 JSON은 전체 리터럴 블록을 유지한다.
리소스를 가져오거나 로컬 링크 권한을 추정하지 않는다. 테스트는 80/24/80 줄바꿈,
리터럴 설명, 미지 메타데이터, 0/u64 크기 경계, 잘못된 크기, 카드 바이트 상한과
첫 초과, 원문과 사용자 콜백 보존을 확인한다. `/preview resource-link`는 리소스를
가져오지 않고 공통 렌더러를 보여준다.

정확히 `find`인 도구는 리터럴 glob 패턴과 결과 경로를 구분한다.
양의 정수 `details.resultLimitReached`는 부분 검색 안내로 표시하고, 선택적인
`truncation`은 shell 출력과 같은 검증된 행·바이트 메타데이터를 사용한다.
알 수 없는 키, 잘못된 값, 일치하지 않는 잘림 정보는 details JSON 전체를 유지한다.
경로 수나 클릭 목적지를 추정하거나 파일시스템에 접근하지 않는다.
프레임 테스트는 80/24/80 재배치, 리터럴 이름, 정수 경계, 잘못된 메타데이터,
원문 보존과 사용자 렌더러의 원본 typed output 수신을 검사한다.
`/preview files-find`는 공통 도구 렌더러를 사용하는 오프라인 예시다.

정확히 `grep`인 도구도 검색 렌더러를 공유하며 내용 패턴 제목, 원문 검색 결과,
양의 정수 `matchLimitReached`, boolean `linesTruncated`를 표시한다.
보고된 일치 개수 제한과 짧아진 행은 별도로 안내하고 false는 경고를 추가하지 않는다.
나머지 검색 조건(glob·대소문자·문맥)은 JSON으로 보인다. 오류 결과는 메타데이터와 함께
리터럴로 유지하며, 미지 키나 잘못된 메타데이터도 details JSON 전체를 보존한다.
TUI에서 파일명·줄 번호를 분해하거나 정규식을 실행하거나 일치 개수를 추정하지 않는다.
테스트는 좁은 폭, 콜론 경로, 리터럴 Markdown, 제한 경계, 잘못된 값과 오류 결과,
원문 출력 및 사용자 렌더러 입력을 검사한다. `/preview content-search`는 오프라인 예시다.
실제 프리뷰의 고정 폭 검증 뒤에는 원래 tmux 크기 정책을 복원해야 사용자 터미널 크기에
따라 줄바꿈이 계속 조정된다.

잘림 boolean 메타데이터는 true일 때 읽기 쉬운 안내로 표시하고 false일 때 생략한다.
이미 부분 목록 안내가 있으면 중복하지 않는다. 형식이 잘못된 디렉터리 결과도
원문과 함께 잘림 안내를 유지한다.

Codex `commandExecution`은 사용자 설정 가능한 명령 패널에 합산 출력과 보고된 상태·
종료 코드·시간을 표시한다. 스트리밍은 완전한 ToolOutput 스냅샷으로 누적하며 최종
출력으로 교체한다. 생략되거나 null인 명령·cwd·출력은 관찰된 값을 유지한다.
`/preview codex-shell`은 오프라인 예시다. 백엔드 테스트는 누적·희소 완료·최종 교체를,
TUI 프레임은 80/24/80열·사용자 색상·리터럴 출력·렌더러 콜백을 검증한다.
공통 명령 렌더러는 `bash`·`powershell`·`run_command`가 보고한 `status`·`exitCode`·
`durationMs`도 표시하며 특정 프로바이더 이름에 의존하지 않는다. 합산 텍스트는 원문으로
유지하고 내장 stdout/stderr 형식으로 재해석하지 않는다. 지원하지 않는 문법은 명령
원문으로 표시한다. 여러 도구의 프레임에서 메타데이터·원문 내보내기·사용자 콜백을 검증한다.

기본 셸 렌더러는 명령·결과 정보를 유지하고 텍스트 출력 블록마다 마지막 5개 표시 행을
남긴다. 명확한 내장 결과는 stdout/stderr별로 적용한다. `tui.shell_tail_rows`로 개수를
설정하고 0이면 일반 도구 전체 접기를 사용한다. Ctrl+O는 전체 출력을 복원한다.
사용자 렌더러 출력에는 일반 접기를 유지한다. 폭·스트리밍·설정 변경은 기존 캐시를 갱신한다.
테스트는 80/24/80열, 탭·한글·제어 문자·Markdown 원문, 행 수 경계, 스트리밍, 전체 펼치기,
0/최대 설정, 사용자 콜백과 원문 내보내기를 검증한다. `/preview shell-tail`은 긴 출력의
오프라인 예시다. 전체 출력 파일 탐색은 남은 작업이며 내장 프로세스 스트리밍은 아래에서 검증한다.

셸 결과 `details`는 고정된 pi BashToolDetails/TruncationResult 형식의 전체 출력 경로,
줄·바이트 수와 제한, 부분 줄 플래그를 표시한다. 인식 가능한 일관된 메타데이터이며
content가 텍스트 블록에 이미 존재할 때만 요약한다. 알 수 없거나 잘못된 필드, 4096바이트
초과·제어 문자 경로, 256 KiB 초과 details는 원본 JSON을 유지한다. 원본 프로필·렌더러
콜백은 보존하며 셸 접기에서도 안내는 남는다. 경로는 파일·네트워크 접근 없이 리터럴로
표시한다. `/preview shell-truncated`는 오프라인 예시다. 80/24/80열, 줄·바이트·완전한
출력, 리터럴 경로, 원본 콜백, 경로와 메타데이터 크기의 정확한 한도·첫 초과를 검증한다.

내장 명령의 진행 출력은 ToolExecution::take_progress → managed 백엔드의 진행 승인
→ 완료 전 ToolOutput 스냅샷 경로를 사용한다. 기존 호스트·정책은 기본적으로 진행 출력을
보내지 않는다. 로컬 파이프는 제한된 stdout/stderr를 100 ms당 최대 한 번 전달하고,
미완성 UTF-8 끝부분은 다음 바이트를 기다리며 완료·취소 후 대기 스냅샷을 닫는다.
LocalSemanticAdmission은 완전한 인증 문자열을 검사하고 각 스트림 끝의 미완성 인증
문자열 접두사는 보류한다. 잘렸거나 원시 크기를 넘은 진행 출력은 보류하며 승인 거절·
승인 후 크기 초과는 기존 실패 정리 경로로 처리한다. replay와 다음 모델 요청에는 최종
결과만 넣는다. TUI는 progress=true일 때 종료 코드 없이 두 스트림을 표시하고 최종
출력으로 교체한다. 종료 파일을 기다리는 실제 셸의 완료 전 출력, UTF-8, 정책 허용·기본·
거절·크기·잘림, 인증 문자열, 정리, replay와 80/24/80열을 검증한다.
`/preview shell-progress`는 시간을 두고 진행·완료를 표시하는 오프라인 예시다.

내장 모델의 incomplete/failed 종료는 부분 답변과 보고된 사용량을 보존하고 열린
activity를 닫으며 미완성 호출 실행과 replay 발행을 하지 않는다. ResponseLimit은 기존
커스터마이징 가능한 안내 렌더러로 ActivityNotice 경고를 표시한다. 미완성 도구 호출
도중의 length/max_output_tokens, 제공자 실패 분류, 미완성 호출을 남긴 completed
종료의 엄격한 거부를 테스트한다.

접힌 셸 출력은 공통 grapheme 배치 엔진에서 마지막 행만 제한적으로 보관한다. 전체
표시 행 수는 usize로 세고 보관한 셀 좌표는 u16을 유지한다. 일반·커서 배치의 기존
높이 한도와 오류 동작은 유지한다. 탭·제어 문자·CR/LF·전각·결합 문자와 원본 바이트
위치를 보존하고 제거한 행 버퍼를 재사용한다. 최대 높이·첫 초과, 10만 행, 끝의 빈 행,
긴 한 줄과 7만 행·150만 문자 한 줄의 실제 80/24/80열 TUI 프레임을 검증한다.
원본 ToolOutput과 콜백 source는 완전하게 유지한다. 전체 펼치기는 기존 좌표 한도를
사용한다. 텍스트 내보내기는 항목별로 배치한 행을 usize 위치로 연결하므로 대화 전체
행 수는 화면 높이 한도를 넘을 수 있다. marker·빈 줄·간격·부분 대화의 선행 맥락을
보존한다. 일반 메시지와 도구 출력은 128행 페이지 단위로 내보내므로 단일 항목도 화면
높이 한도를 넘을 수 있다. 구조화 도구는 시각 콜백이나 원본 스키마 우회 없이 승인된
plain_text와 소유한 제목·footer를 내보낸다. 제어 문자 표기와 설정한 본문 폭도 유지한다.
ModelWork 계획·문서·요약·경고도 원문 페이지를 사용한다. 문서·요약 해석과 계획
제목·상태 표시는 화면 렌더러와 공유하고 계획의 이어지는 줄 들여쓰기와 빈 줄을 유지한다.
알 수 없는 프로필은 원문으로 남긴다. 내보내기는 단일 항목을 화면 좌표로 제한하지 않으며
보관 프로필 자체의 크기 제한은 유지한다. 테스트는 여러 항목의 98,000행,
실패 footer가 있는 7만 행 도구, 정상 종료 시 초대형 사용자 메시지, 각각 7만 행의
계획·문서·요약·경고 보존을 확인한다.


`/output`은 보관된 ToolCall/ToolResult 원문을 독립적으로 페이지 탐색한다. 열 때 마지막
도구를 선택하며 좌우 키로 도구를 바꾼다. 위·아래, PageUp/PageDown, Home/End와 마우스
휠로 이동하고 F1으로 Chat에 복귀한다. End는 위로 이동하기 전까지 추가되는 출력을
따라간다. 구조화 profile은 보관된 plain_text를 표시하며 렌더러 콜백, 첨부 파일 읽기,
원격 경로 해석이나 실행을 수행하지 않는다. 기존 최대 본문 폭과 도구 본문 테마 색상을
적용한다. boolean 생략 관측은 스크롤 중에도 `Partial output` 헤더로 남으며 좁은 폭에서는
`Partial` 또는 `!`로 표시한다. 새 snapshot은 이전 관측을 대체한다. 메타데이터 부재,
문자열이나 파일 경로만으로 생략 여부 또는 보관된 출력의 완전성을 판단하지 않는다.
표시 불가능한 셀은 안내와 함께 ASCII escape로 대체하며 원본 기록은 유지한다.
공유 grapheme scanner가 표시 문자열과 128행 간격의 희소 인덱스, usize 행 수를 만든다.
선택한 항목·revision·폭이 바뀔 때만 캐시를 갱신하며 스크롤은 요청한 페이지만 배치한다.
폭 변경 시 읽던 원문 byte 위치를 유지하고 그 위치를 포함하는 표시 행을 복원한다.
End는 다시 개행된 끝을 따른다. 행별 원문 위치는 빈 줄과 탭 확장도 보존하며 직접
이동하면 새 기준점을 저장한다. 공통 `TextPages::with_escaped_fallback`은 명시적 줄바꿈과
빈 행을 보존하고 이스케이프 행을 원문 위치에 대응한다. 대체 표시 안에서 스크롤하거나
원문 표시로 복원해도 기준점을 유지한다. 편집기와 엄격한 flow 경로는 변경하지 않는다. 테스트는 7만 행 탐색, 인덱스·u16
경계, 긴 한 줄, 제어 문자·탭·한글, 마지막 빈 행, 원문·폭 캐시 갱신, 스트리밍,
실패한 frame 재시도, 로컬 입력과 대기 중 request의 상관관계를 검증한다.
내장 명령은 이 뷰어를 위한 출력을 별도 한도로 수집한다. Rust 설정
`NativeModelBackendConfig.maximum_retained_tool_output_bytes`의 기본값은 8 MiB이며
`None`은 보존용 수집을 끈다. 호스트는 framing 공간을 제외한 예산을 stdout/stderr에
나눠 적용하며 모델용 결과의 기본 4 MiB 한도와 독립적이다. 각 스트림의 보존 한도를
넘으면 앞뒤를 남긴다. 무제한 보존이나 모델 컨텍스트 증가를 의미하지 않는다.

두 표현 모두 기존 의미 검증을 통과해야 한다. 큰 보존 원문은 `ToolOutput.plain_text`,
짧은 결과는 result content와 모델 기록에 담는다. `retainedOutput.truncated`는 뷰어의
보존분을, `result.truncated`는 모델용 결과의 생략을 각각 나타낸다. 기본 렌더러는
`/output` 안내를 표시하고 사용자 렌더러에도 같은 메타데이터를 전달한다. 미지·잘못된
메타데이터는 리터럴로 남긴다. 원시 출력을 별도 파일로 쓰지 않으며 검증된 profile은
기존 journal segment·저장소 용량·저장 압력 처리를 따른다. 영구 보존에는 저장 성공이
필요하다. 보존 원문의 한도 초과·검증 거절 또는 16 MiB profile 인코딩 초과는 모델 제출
전에 명시적으로 실패한다. 명령을 반복하거나 큰 원문을 짧은 결과로 몰래 대체하지 않는다.
테스트는 스트림·검증 한도의 마지막·첫 초과 byte, 중간에만 있는 credential, 세 종료
결과, 모델 요청 분리, profile 용량과 durable segment 복구를 검증한다.
`/preview shell-retained`는 120행 오프라인 예시이다.
외부 전체 출력 파일의 저장·접근은 별도 남은 작업이다.


### 터미널 하이퍼링크

Markdown 링크는 고정된 Codex terminal_hyperlinks.rs처럼 목적지를 보이는 셀과 분리해
보관한다. 기존 url 2.5.8로 호스트를 가진 절대 HTTP(S) URL을 검증하며 제어 문자와
8 KiB를 넘는 주소는 터미널 제어로 출력하지 않는다. 상대·file·실행 스킴은 텍스트로
남기며 파일 해석이나 브라우저 열기를 수행하지 않는다. 목적지는 Arc로 공유하며 넓은
grapheme의 전체 footprint가 소유한다. Markdown span ID가 강조·인라인 코드·표·개행을
따라 링크를 유지한다. 코드 fence는 원문으로 남고 목적지만 바뀌어도 FrameDiff에 반영한다.

TerminalOps는 스타일·커서 좌표와 별개로 OSC 8 링크를 선택하고 닫는다. 각 diff span과
inline scrollback 발행 행의 끝에서 링크를 닫는다. 부분 쓰기 실패 시 중단된 제어 문자열을
끝내고 링크를 닫은 뒤 커서 표시 또는 동기화 갱신 종료를 복원한다. 다음 시도는 링크 상태를
초기화하고 다시 그린다. frame 검증 실패는 새로운 출력보다 먼저 처리한다. 테스트는
원문·제어 문자·크기 경계, 넓은 셀의 원자적 덮어쓰기·지우기, OSC 바이트, 목적지 변경,
scrollback 발행, 부분 실패·복구, 80/24/80 Markdown·표 frame을 검증한다.
`tui.hyperlinks`의 기본값은 true이며 false는 글자·색상·원문을 바꾸지 않고 링크 정보를
제거한다. Rust API는 OutputPreferences::with_hyperlinks이고 CLI는 false와 잘못된
설정을 검사한다. `/preview links`로 Rust 표시 경로를 확인한다. 실제 클릭 동작은 외부
터미널에 달려 있으며 ANSI 캡처만으로 입증하지 않는다. Markdown 본문·표의 일반 HTTP(S)
주소는 파싱이 끝난 block/cell을 개행 전에 검사한다. HTML entity나 강조 때문에 파서가
나눈 URL도 완전한 목적지를 유지한다. 문장 부호와 짝이 맞지 않는 끝 괄호는 제외하고,
주소 내부의 짝이 맞는 괄호와 IPv6 대괄호는 유지한다. 명시한 링크(거절된 목적지 포함)와
인라인·fence 코드는 자동 인식으로 덮어쓰지 않는다. 기존 링크 설정·팔레트·주소 검증을
적용한다. 테스트는 문장 부호, 디코딩된 query, 기존 링크·코드 보존, 표 개행과 스트리밍
텍스트 갱신을 확인한다. 호스트가 확인한 파일 링크는 아래 경로로 지원한다. 자동 워크스페이스 경로 해석과 구조화 인용은 남은 작업이다.
Markdown이 아닌 원문 로그와 입력 중인 문장은 이 자동 인식 경로에 포함하지 않는다.


### 호스트가 확인한 파일 링크

TuiSession::with_link_resolver(Some(LinkResolver::new(...)))로 호스트가 명시적 Markdown
목적지를 검증된 surface::Hyperlink에 연결한다. None을 반환하면 기본 HTTP(S) 처리를
유지한다. fence·독립 인라인 코드, 이미지(표 이미지 포함), 원문 도구 로그, 원문 내보내기,
비활성 링크에서는 callback을 호출하지 않는다. 일반 URL 자동 인식은 기존 웹 전용 경로를
유지한다. 제어 문자나 8 KiB를 넘는 목적지는 callback에 전달하지 않는다. 매핑이 바뀌면
handle을 교체해 캐시를 무효화한다. callback은 결정적이고 제한된 작업만 하며 I/O가 없어야 한다.

Hyperlink::from_file_path는 호스트가 명시적으로 사용하는 절대 UTF-8 파일 경로 생성자다.
호스트가 렌더링 밖에서 파일 소유·존재·워크스페이스 범위와 symlink 해석을 먼저 확인한다.
생성자는 파일시스템을 읽지 않으며 부모 경로 요소, 제어 문자, 원격 authority와 인코딩 후
8 KiB를 넘는 file URL을 거부한다. 파일명의 URL 메타 문자는 경로 데이터로 인코딩한다.
Hyperlink::new는 여전히 원시 file URL과 실행 스킴을 거부한다. 렌더링 중 파일 열기 명령을
실행하지 않는다.

```rust,ignore
// 호스트가 레이아웃·렌더링 밖에서 known_file을 확인한 상태다.
let target = Hyperlink::from_file_path(&known_file).expect("bounded authorized path");
session = session.with_link_resolver(Some(LinkResolver::new(move |destination| {
    (destination == "README.md").then(|| target.clone())
})));
```

해석한 목적지는 웹 링크와 같은 셀 메타데이터·줄바꿈·팔레트·OSC 8 인코더·복구 처리를
사용한다. 원래 표시 이름과 Markdown은 바꾸지 않는다. 파일명 인코딩, 인코딩 한도와 첫
초과, 거부 경로, callback 입력 한도, 문단·표 개행, resolver 교체·제거, 비활성 callback,
웹 fallback, 원문 보존과 정확한 파일 링크 OSC 바이트를 검증한다.

네이티브 예시를 --custom-links로 실행한 뒤 `/preview file-links`를 입력한다. 등록한
README 목적지만 시작 시 canonicalize한 저장소 파일에 연결하며 등록하지 않은 파일과
원시 file URL은 일반 텍스트로 남는다. 이는 예시 매핑이며 실제 앱의 기본 워크스페이스
해석기는 아니다. 외부 터미널이 파일 링크 열기 지원과 방법을 결정하므로 OSC 출력만으로
줄 번호 이동이나 실제 클릭을 보장하지 않는다. 고정한 Codex terminal_hyperlinks.rs의
일반 웹 링크와 명시적으로 신뢰한 파일 목적지 구분을 참고했다.

### 세션 시작·재개 안내

터미널 시작 경로는 확인된 신규·재개 여부를 TuiSessionInfo::with_startup_notice에
전달한다. 내장 연결은 프로바이더와 설정 모델을, 위임 연결은 `host:<id>`와 일치하는
활성 호스트가 보고한 모델을 함께 표시한다. 호스트 정보가 없거나 일치하지 않으면
`model unreported`를 표시하고 계정 식별자는 포함하지 않는다. 모델 전환 성공 뒤에도
새 모델과 실행 주체를 함께 유지한다. 이 조합은 CLI가 소유하며 공통 TUI는 정규화된
표시값만 받는다. 세션은 호스트가 알고 있는 백엔드·작업 공간으로 일회성 info 안내를 표시하며
턴을 시작하거나 journal event를 추가하지 않는다. 기존 안내 색상과 본문 폭 설정을
사용한다. 호스트가 옵션을 생략하면 상태 표시줄만 유지한다. 제어 문자는 가시 표기로
바꾸고 안내의 각 표시값은 4096문자로 제한하며 초과 시 말줄임표를 붙인다. 기존 상태
표시줄 값은 바꾸지 않는다. 신규·재개, 빈 호스트·기본값, 정확한 길이 경계와 80/24/80열
재그리기에서 중복 안내나 작업 시작이 없는 것을 검증한다. 백엔드 표시값은 제공자가
턴마다 실제로 사용한 모델이나 원격 세션 ID를 증명하지 않는다.


### 제공자의 모델 경로 변경

Codex `model/rerouted` 알림은 보고된 fromModel·toModel·reason을 ActivityNotice 경고로
표시한다. 로컬 app-server 스키마는 threadId·turnId와 이 필드를 정의한다. 런타임은
thread와 활성 turn을 확인하고 누락·문자열 아닌 필드와 과도한 크기는 발행 전에 거부하며
완료된 turn의 늦은 알림은 무시한다. 알림은 모델 선택 설정이나 최종 turn 결과를 바꾸지
않는다. 기존 경고 색상·본문 폭·원문 내보내기를 사용한다. 완료·실패 순서, 늦은 알림,
잘못된 대상·필드·크기와 사용자 지정 경고 표시를 검증한다. 사유는 알 수 없는 문자열도
보고된 그대로 보존하며 yo가 변경 원인을 추정하지 않는다. `/preview reroute`는 모델을
바꾸지 않는 오프라인 예시다.


### 턴 전체 변경 내용

로컬 app-server 스키마에 따르면 Codex `turn/diff/updated`는 턴 전체의 최신 unified
diff를 전달한다. 런타임은 턴당 FileChange activity 하나를 유지하며 갱신마다 스냅샷을
교체하고 개별 도구 변경과 구분하도록 `Turn aggregate diff`를 표시한다. 빈 갱신은 이전
diff를 지우고 남은 변경이 없다고 안내한다. activity는 턴의 완료·실패·중단 결과로 닫고
완료된 턴의 늦은 갱신은 무시한다. thread·turn·문자열 타입·표시 바이트 한도는 발행 전에
검증한다. 기존 diff 색상·접기·폭과 `/changes` 파일 탐색을 사용한다. 앞의 설명은 별도
파일 항목을 만들지 않고 첫 파일에 포함한다. 교체·빈 갱신·모든 종료 결과, 잘못된 대상·
타입, 정확한 크기 한도와 첫 초과, 80/24/80열 파일 탐색을 검증한다. `/preview turn-diff`는
두 파일의 오프라인 예시다. 파일을 읽거나 diff를 추론하지 않으며 개별 도구 변경은 별도
관측으로 유지한다.


### 간결한 사용량 관측

검증되어 완료된 사용량은 Activity 접힘 상태에서 입력·출력 수치만 짧게 표시한다.
Ctrl+O로 캐시·추론·보고된 컨텍스트 창 상세를 다시 펼친다. 좁은 폭에서는 요약도
줄바꿈하고 기존 muted 테마 역할을 사용한다. 성공적으로 완료된 관측의 표시만 지정하며
비슷한 일반 문자열, 잘못된 관측, 중단·실패한 관측은 기존 표시를 유지한다.
각 관측은 별도 Activity로 남고 전체 원문 내보내기에도 상세 내용을 모두 보존한다.
백엔드 이벤트·관측 스키마·세션 사용량 집계 규칙은 바꾸지 않는다. Codex·Grok 관측을
함께 넣어 개별 수치, 80/24 폭, 접기·펼치기 왕복과 원문 보존을 검증한다.
관측을 합치는 대신 각 패널의 높이를 줄이는 변경이다.

### 일반 답변 렌더링

일반 답변은 `AssistantRenderer` / `TuiSession::with_assistant_renderer`로 표시를 바꾼다.
콜백은 원본 본문·현재 폭·finalized 여부(성공을 뜻하지 않음)를 받는다. 사용자·도구·승인·
문서와 원문 내보내기는 제외한다. 실패·중단 문구는 변환 Markdown 밖에서 원문으로 유지한다.
`None`, 256 KiB 초과, 레이아웃/결과 문구를 합친 높이 초과 시 원래 본문을 표시한다.
핸들 교체로 캐시를 갱신하며 콜백은 I/O 없이 결정적이고 제한된 작업을 해야 한다.
기존 테마·출력 설정을 적용한다. Rust 프리뷰 `--custom-answers`와 `markdown` 사례는
변환된 답변에 현재 폭과 streaming/final 상태를 표시한다.

Codex `imageGeneration` 항목은 도구의 진행·완료 상태, 프롬프트, 저장 경로 메타데이터,
실패 정보와 생성 PNG를 `ToolOutput`으로 전달한다. 커스텀 도구 렌더러는 첫 콘텐츠
블록의 `source`에서 전체 원본 항목을 확인할 수 있다. 이미지 바이트는 기존 공통 미디어
렌더러와 이미지 설정을 따른다. 빈 결과는 이미지 데이터가 없음을 명시하며, 표시 어댑터는
저장 경로를 직접 열지 않는다. 기존 `mcp-image` 프리뷰로 같은 미디어 경로를 확인한다.

Codex `imageView`도 조회 시작·완료와 원본 경로를 `ToolOutput`으로 전달한다. 이 항목은
이미지 바이트를 포함하지 않으므로 화면에 이를 명시하고 경로를 원문으로 표시한다.
커스텀 도구 렌더러는 첫 콘텐츠 블록의 `source`에서 전체 원본 항목을 확인하며,
알 수 없는 메타데이터도 읽을 수 있는 형태로 유지한다.

Codex 협업 도구 항목도 같은 `ToolOutput`을 사용한다. 도구 완료와 각 에이전트의 보고
상태를 구분하므로, 완료된 대기 호출에도 실행 중이거나 실패한 자식이 표시될 수 있다.
발신·수신 주체, 요청 모델·추론 수준, 프롬프트, 자식 메시지와 미지 메타데이터를 유지하며
커스텀 도구 렌더러에는 원본 항목도 제공한다. 협업 도구의 interrupted 상태는 중단된
Activity로 연결한다. `/preview agent-tasks`는 이 상태들을 보여 주는 오프라인 예시다.

Codex 리뷰 모드 진입·종료 항목은 ModelWork 문서로 전달한다. 보고된 리뷰 본문은
“Review started” 또는 “Review ended” 제목 아래 Markdown으로 보존하며 기존 문서
렌더러, 코드 스타일, 접기와 원문 내보내기를 사용한다. 빈 종료 본문을 문제없는 리뷰
판정으로 바꾸지 않는다. 형식이 맞지 않거나 한도를 넘으면 원문을 유지한다. 수신 이벤트의
표시만 연결하며 `review/start` 명령이나 외부 리뷰 요청을 추가하는 것은 아니다.

Codex `subAgentActivity`는 항목 ID별로 하나의 ModelWork 알림을 갱신한다. 보고된 자식
상태·경로·스레드는 부모 Activity의 완료와 구분하며 미지 메타데이터도 원문으로 유지한다.
`sleep`은 공통 ToolOutput을 사용하고 durationMs를 요청한 대기 길이로 표시한다.
0을 포함한 부호 없는 정수의 정밀도를 유지하며 경과 시간이나 카운트다운을 추정하지 않는다.
커스텀 도구 렌더러에 원본 대기 항목을 제공하고, 알림은 기존 의미별 색상을 사용한다.

Codex의 기록된 `functionCallOutput`은 ToolResult Activity로 전달한다. 시작·완료는
하나의 식별자를 공유하며 이름공간을 포함한 함수 이름과 전체 원본 항목을 ToolOutput에
보존한다. 텍스트는 원문으로 표시하고 input_text/image/audio 블록은 기존 공통 미디어
형식으로 변환하되 충돌하는 필드를 덮어쓰지 않는다. 빈 출력은 명시하며 잘못된 본문과
미지 블록은 원문으로 유지한다. 수신한 함수 결과의 기록이며 새 도구 실행을 요청하지 않는다.

`hookPrompt`는 새 사용자 제출이 아닌 별도의 “Hook context” ModelWork 문서로 기록한다.
훅 실행 ID, fragment 순서, 정확한 본문과 미지 메타데이터를 보존하며, 원문보다 긴
backtick fence로 문맥을 원문 표시한다. 빈 fragment에는 안내를 표시하고 형식이 맞지
않거나 한도를 넘으면 원본을 유지한다. 기존 문서 렌더러, 접기와 코드 설정을 적용한다.
원본 Codex의 문맥 파서와 전체 대화 보기에서도 훅이 추가한 문맥을 일반 사용자 발언과
구분한다.







텍스트가 아닌 답변은 백엔드 공통 `MessageContent` 프로필을 사용한다. Grok ACP의
이미지·리소스·미지 답변 블록은 원본 필드를 유지하고 앞뒤 텍스트를 원래 순서로 나눈다.
공통 TUI의 도구 미디어 렌더링과 이미지 설정을 재사용하며, 답변 렌더러 콜백에는 원본
프로필을 전달한다. 원문 내보내기는 메타데이터를 포함한 블록 JSON과 실패 문구를 보존한다.
리소스 URI는 가져오지 않는다. `/preview message-image`로 오프라인 경로를 확인한다.

긴 공개 요약과 공급자 추론도 본문 접기를 적용한다. 첫 부분·끝부분·실패 footer를 유지하고,
폭을 바꾼 뒤에도 Ctrl+O로 전체 본문을 복원한다. 접힌 행 안내에도 추론 색상을 유지한다.

공급자가 전달한 추론은 공개 요약과 별개인 `ActivityReasoning` (`yo.activity-reasoning/v1`)을 사용한다.
Grok ACP의 추론 텍스트는 같은 메시지 안에서 누적하며, 비텍스트 블록은 원본 JSON을 유지하고
텍스트 스트림을 순서대로 나눈다. 공통 화면은 “Agent reasoning” 제목과 `show_reasoning`·추론 색상을
적용하고 원문·실패 footer를 보존한다. DocumentRenderer는 원문 또는 리터럴 JSON fence를 받으며,
커스텀 렌더러가 있어도 숨김 설정을 유지한다. 비텍스트 내용은 이미지·첨부 권한을 부여하지 않는다.
`/preview agent-reasoning`으로 모델 호출 없이 확인한다.
profile은16MiB로 제한하며 초과한 Grok 추론은 표시 설정을 우회하는 일반 텍스트로 바꾸지 않고 명시적으로 실패한다.

### 사용자 정의 문서 렌더링

`DocumentRenderer`와 `TuiSession::with_document_renderer`는 ModelWork에 담긴 명시적
ActivityDocument 본문과 ActivityReasoning의 문서 표현을 커스터마이징한다. 콜백은 원본 문서, 관측된 종료 상태, 펼침 여부와
적용된 본문 폭을 받는다. Markdown을 반환하면 기존 코드·테이블·차트 렌더러를 사용하고
None이면 원래 문서를 유지한다. 제목·상태·실패 footer와 원문 내보내기는 호스트가 관리한다.
요약·도구·승인 요청은 이 콜백에 전달하지 않는다. 문서 이미지는 안내만 표시하고 첨부 권한을
부여하지 않는다. 16 MiB 초과 결과나 렌더링할 수 없는 레이아웃은 원래 본문으로 돌아간다.
빈 사용자 정의 본문은 빈 상태로 두고 원문은 독립적으로 보존한다.

ToolRenderer와 마찬가지로 콜백은 I/O 없이 결정적이며 제한된 작업이어야 한다. 핸들을
교체하면 캐시가 갱신되며 캡처한 가변 상태만 바꿔서는 레이아웃이 갱신되지 않는다.
타입 입력·종료 상태, 반복 프레임, Ctrl+O, 좁은 폭, 실패·원문 보존, 콜백 교체·제거와
레이아웃 실패 대체를 검증한다. 독립 Rust 예시의 `--custom-documents` 옵션을 켜고
`/preview terminal-wait`로 호스트의 접힌 요약·펼친 상세 표시를 확인한다. 임의의 대화형
위젯이나 세션 수준 상태·문서 채널을 제공하는 기능은 아직 아니다.

### Grok 구조화 도구 결과

Grok ACP ToolResult는 공통 ToolOutput 프로필을 사용한다. rawInput/rawOutput/content/
locations/_meta 필드는 부분 갱신에서 생략하면 유지하고 명시적인 빈 content는 이전 내용을
지운다. 도구 이름은 보고된 name 또는 호출 ID이며 title에서 추측하지 않는다. 커스텀
도구 렌더러와 읽을 수 있는 원문에 인자를 제공한다. 단순 ACP content 래퍼는 원래 내부
텍스트·이미지·리소스 블록으로 연결하고 추가 필드가 있는 래퍼와 terminal 항목은
JSON으로 보존한다. 터미널·파일 I/O는 추가하지 않는다. 원시 결과·metadata는 원래 키를
유지한다. failed 상태는 기존 실패 Activity 결과·설명을 유지하며 임의 결과 필드를 오류로
추측하지 않는다. 큰 프로필은 필드 원문으로 대체한다. 종료된 호출의 추가 결과 사본을
해제하고 결과 이벤트는 보존한다. 승인 ID, 부분 필드, 이미지·diff 보존, 명시적 삭제,
변경 인자, 실패·정리를 검증한다. 실제 이미지 표시와 Grok 서비스는 별도 검증이 필요하다.

ACP diff 항목은 리터럴 `title`, unified `text`, 원본 항목 전체인 `source`를 가진
공통 `type: "diff"` 콘텐츠로 변환한다. 어댑터에서 oldText·newText를 비교해 주변 세 줄을
포함하며 null oldText는 새 파일이다. 합산 입력이 256 KiB를 넘거나 필드가 잘못되면
JSON으로 유지한다. diff 탐색 제한은 50 ms이며 근사한 변경 구간이 나올 수 있다.
패치 헤더의 경로는 인용 처리한다. 공통 TUI는 프로바이더별 분기 없이 diff 색상과
줄바꿈을 적용하고 빈 diff에는 `No textual changes`를 표시한다. 커스텀 도구 렌더러는
원본을 계속 사용할 수 있다. `/preview tool-diff`는 모델 호출·파일 쓰기 없이 실제 Rust
프리뷰에서 공통 콘텐츠를 확인하는 예시다. 여러 변경 구간, 새 파일·동일 내용, 경로
이스케이프, 크기 경계, 좁은 폭 변경, 테마 역할과 원문 내보내기를 검증한다.
처음에 edit/delete/move로 분류된 호출은 같은 FileChange Activity에도 보고된 diff를
전달하며 `/changes` 파일 이동을 위한 명시적 인용 경로 헤더를 제공한다. 부분 갱신은
파일을 유지하고 명시적인 빈 content는 지운다. 원래 호출의 실패·중단 상태가 기준이며
패치 표시는 파일 쓰기의 성공을 뜻하지 않는다. 다른 도구 종류는 기존 분류를 유지한다.
파일 Activity나 로컬 파일 읽기를 추가하지 않는다. 같은 Activity에서 여러 파일,
경로 이스케이프, 부분 갱신, 삭제와 실패를 검증한다. 실제 인증된 서비스는 별도 검증이 필요하다.

### Grok 승인 선택지

ACP 승인 선택지는 원래 순서대로 공통 ActivityApproval 레이어에 전달한다. 알려진 일회성
허용·거절과 기억되는 허용·거절은 정확한 option ID로 연결하고 미지원 종류는 표시하되
비활성화한다. 기억되는 범위는 에이전트가 관리하며 응답 성공이 정책 저장을 입증하지 않는다.
기존 이진 승인은 allow_once/reject_once만 선택하고 기본 선택은 reject_once다.
중복 ID, 64개 초과 선택지, 잘못된 식별자는 게시 전에 거부한다. 16 MiB 초과 프로필은
보고된 reject_once 선택지로 거절한다. 잘못된 번호, 비활성 선택지, 범위 밖 번호는
요청을 소비하거나 응답을 보내지 않는다.

읽을 수 있는 요청에는 보고된 도구 호출 ID와 rawInput 인수를 넣고 임의 도구 metadata는
의미적 승인 내용에 복사하지 않는다. 같은 Turn에서 이미 관측한 미종료 파일 변경만 정확한
toolCallId로 기존 `/changes` 검토 게이트에 연결한다. 다른 종류와 종료된 호출은 연결하지
않는다. 호출보다 먼저 온 요청은 제한된 ID를 보존하고 일치하는 미종료 FileChange가
도착하면 요청 순서대로 같은 승인 프로필을 갱신한다. 선택지나 응답 권한은 바꾸지 않는다.
게시 전에 가장 큰 관련 ID의 인코딩 크기를 예약하며 정확한 바이트 경계도 검증한다.
모르는 ID만으로 승인할 수는 없지만 ID 전용 요청은 같은 Turn의 미종료 호출 제목 또는
이름·인수 설명과 인수를 사용할 수 있다. 명시적인 새 인수는 재구성한 설명에서도 이전
인수보다 우선한다. 다른 호출에서 설명이나 파일 연결을 가져오지 않는다. 기존 TUI의
프로필 갱신·새 프레임 검사는 뒤늦은 연결이나 관련 diff가 오면 이전 검토 기록을 무효화한다.
정확히 일치하는 기존 미종료 호출은 승인 요청의 rawInput/content/locations로 도구
스냅샷을 먼저 갱신한 뒤 승인을 게시한다. 최신 diff가 이전 `/changes` 본문을 교체하고
명시적인 빈 content는 지운다. 요청의 status/kind로 호출을 종료·재분류하지 않으며
rawOutput·임의 metadata는 요청에서 가져오지 않는다. 검증과 중복 요청 검사는 갱신보다
먼저 수행한다. 요청의 content·locations도 승인 이력에 리터럴 원문으로 남긴다.
아직 모르는 제한된 호출 ID는 같은 프로필 바이트 한도 안에서 표시 필드를 임시 보존한다.
실제 호출이 오면 명시된 필드가 빈 값·null을 포함해 우선하며, 생략된 필드만 그 값을
제공한 가장 최근 미응답 요청에서 보충한다. 정확한 ID·Turn으로 연결하고 가짜 도구
Activity를 만들지 않는다. 반영 또는 요청 정리 뒤 임시 사본을 해제하며 원래 요청 이력은
남는다. 연결된 도구 스냅샷은 일반 리치 diff 렌더러를 사용한다. 승인 Activity가 시작되기 전에 갱신된 파일 본문과 ToolOutput
원본이 전달되는지, 명시적 삭제와 요청의 completed 상태도 함께 검증한다.
응답 쓰기가 성공한 뒤 선택한 이름·범위만 결과에 남기며 인수를 반복하지 않는다.
네 가지 지원 종류, 원래 순번, 비활성·잘못된 선택, 중복 ID, 64/65개 경계, 큰 요청 거절,
정확한 파일 식별과 일회성 응답 소비를 검증한다. 실제 에이전트의 정책 저장과 터미널
승인 상호작용은 인증된 검증이 필요하다. [ACP 승인](https://agentclientprotocol.com/protocol/v1/tool-calls)을 참고한다.

### Grok ACP 계획

Grok 어댑터는 [ACP v1 계획](https://agentclientprotocol.com/protocol/v1/agent-plan)을 공통
ActivityPlan 스냅샷으로 변환한다. 매 갱신은 순서 있는 전체 목록을 교체하고 우선순위를
`[high]`, `[medium]`, `[low]` 접두부로 보존한다. 빈 목록도 명시한다. 같은 Turn에서 도구
호출이 끼어도 한 계획 Activity를 유지하고 기존 활성 Activity 제한에 포함하며 Turn 결과로
종료한다. 다음 Turn은 새 Activity를 받는다. 완료·중단으로 보고된 단계 상태를 바꾸지 않는다.
잘못된 필드·상태·우선순위와 인코딩된 표시 한도 초과는 부분 계획 게시 전에 거부한다.
목록 교체, 빈 계획, 도구 실행 사이 갱신, 중단, 다음 Turn ID와 정확한 바이트 경계를 검증한다.
TUI는 기존 계획 색상·레이아웃을 사용하며 Grok 전용 렌더링을 추가하지 않는다.

### 관리형 도구 승인 상세

관리형 백엔드는 도구 이름, 해당 호출 범위, effect, 실행 호스트, 호출·도구 ID와
연결된 인자 해시를 읽을 수 있는 승인 문장으로 표시한다. 인자는 실행 원본이 아니라
해당 호출의 의미 보존 정책을 통과한 replay 항목의 정확한 문자열만 사용하며 recorded
view로 구분한다. 정책이 가린 값은 그대로 가린다. 기존 Approved/Declined 응답과 정확한
실행 바인딩은 유지하며 offered 선택지나 영구 권한을 활성화하지 않는다. 실행 전 상세
표시, 승인 후 단일 실행, 정책 대체 전 인자의 비노출을 검증한다.

### 선택 가능한 인터뷰 답변

ActivityQuestion (`yo.activity-question/v1`)은 활성 UserInputRequest의 읽을 수 있는 질문과
순서 있는 QuestionChoice 이름·설명을 전달한다. Codex 순차 질문이 이 선택적 프로필을
발행하며 기존 숫자→이름 응답 변환과 요청 ID가 답변 연결을 결정한다. 선택지 64개와
ToolOutput 스냅샷 바이트 한도를 적용하고 초과하면 기존 일반 텍스트 질문을 유지한다.
TUI는 Chat·내보내기에 질문 원문을 표시하고 요청 패널에는 선택지를 보여준다.
질문·승인 프로필의 원래 1부터 시작하는 번호, 전체 이름과 여러 줄 설명도 Choices 목록으로
대화·내보내기에 추가한다. 비활성 승인 선택지는 unavailable로 표시한다. plain_text에
설명이 없어도 보존하며 요청 완료로 패널이 닫힌 뒤에도 읽을 수 있다. 원시 Transcript
기록은 변경하지 않고 일반 도구 로그의 같은 JSON을 요청으로 해석하지 않는다.
인터뷰·승인 패널은 남는 높이에서 최대 여섯 줄까지 선택한 항목의 이름과 설명을 목록 아래에
줄바꿈하고 기존 detail 색상을 사용한다. 위·아래 이동 시 설명도 갱신하며 높이가 부족하면
생략 표시와 PgUp 전체 설명 안내를 보여준다. PageUp/PageDown으로 대화의 읽을 수 있는
요청 내용을 스크롤하며 선택은 계속 대기 상태로 유지한다. 낮은 터미널에서는 선택지 행이 우선한다.
F2/F3 기록 화면을 왕복할 때 같은 요청의 내용이 유지된 경우에만 선택을 복원한다.
복귀 시 새 패널 token을 사용하므로 이전에 표시한 frame으로 제출할 수 없다.
다른 화면에 있는 동안 요청이 갱신되면 저장한 선택을 버린다. 현재 F2의 전체 구조화
payload는 읽기 전용 상세 페이지가 아니라 이스케이프된 JSON으로 표시된다.
전체 요청은 대화 기록에 보존하며 설명 영역은 선택지 번호나 승인 범위를 변경하지 않는다.
위·아래로 고르고 Enter로 답하며 패널이 실제 표시된 뒤에만 선택을 수락한다. 직접 입력한 답변은
기존 자유 입력 경로를 사용한다. 선택지가 바뀌면 패널 token을 바꾸므로 늦거나 다른
요청이 수락 결과를 사용할 수 없다. 기존 패널 색상·포커스·개행·설명을 사용한다.
`/preview interview`의 두 질문도 같은 프로필을 사용한다. 정확한 크기·개수 경계,
숫자·이름 전달, 표시 전 수락 차단, 좁은 화면과 읽을 수 있는 내보내기를 검증한다.
비밀 입력은 여전히 지원하지 않는다. 고정한 Codex request_user_input 구현의
options_len_for_question/option_label_for_index처럼 isOther는 비어 있지 않은 선택지 끝에
“None of the above”를 추가한다. 선택하면 같은 요청 ID로 그 이름을 답한다. 생략·false면
추가하지 않고 잘못된 타입은 거부하며 자유 입력은 이 플래그와 독립적으로 유지한다.
인터뷰 프리뷰의 첫 질문에서 확인할 수 있다.

호스트는 ActivityQuestion.allow_notes로 선택 항목과 메모의 동시 제출을 활성화한다.
이전 프로필의 기본값은 false이며 Codex와 오프라인 인터뷰는 활성화한다. 표시된 선택지에서
Tab을 누르면 항목 이름·설명을 유지한 메모 패널을 연다. Enter는 선택 번호와 추가 메모를
함께 보내고 Tab은 초안을 보존한 채 선택지로 돌아간다. 같은 패널 색상·폭 처리를 사용한다.
슬래시로 시작하거나 여러 줄인 메모도 답변 원문으로 유지한다. 질문 갱신은 연결된 선택을
초기화하고 새 패널이 표시된 뒤에만 수락한다. 자유 입력 답변은 기존 경로를 유지한다.
AgentIntent::RespondToQuestion과 ActivityResponse::QuestionAnswer는 원래 요청 ID·선택값·
구조화 메모 원문을 admission과 저널 왕복에서 보존하며 SubmissionId를 추가하지 않는다.
Codex adapter는 전송 전에 원래 선택지에 대한 1부터 시작하는 번호를 검증한다. 원래 항목
이름과 비어 있지 않은 경우 별도 `user_note: <앞뒤 공백을 제거한 메모>` 답변을 보내며,
고정한 request_user_input renderer의 형식을 따른다. 빈 메모는 이름만 보낸다. 기본 기능
플래그, 0·첫 초과·u32 최댓값 선택, 정확한 wire ID, 순차 응답, 오래된 표시, 24열 화면,
메모 원문과 저널 코덱 왕복을 검증한다. 비밀 입력과 이미 제출한 질문으로 돌아가기는 남은 작업이다.

### 호스트 상태 줄

CLI는 현재 로컬 백엔드들에 세션 소유 공통 호스트 표시 연결을 설치한다.
TUI 밖에서 수집 사이에5초를 기다리며 관측한 Git 브랜치를 갱신하고 최초 커밋 전 브랜치 이름과 detached HEAD를
구분한다. 실패하거나 잘못된 관측은 `Git status unavailable`로 표시한다. Git 환경 덮어쓰기는 제거한다.
프로세스마다2초·출력1024바이트 제한을 적용하고, 종료 시 대기를 취소하고 소유한 프로세스 그룹을
종료·회수한다. 호스트 snapshot은 최신 한 개로 합치고 원래 연결과 번갈아 처리하며 오류·종료를
대체하지 않는다. worker는 터미널 재진입 동안 유지하고 세션과 함께 종료한다.
같은 연결이 새 `AgentPoll::Links` 핸들도 전달한다. 코어 로컬 작업공간 코드가 탐색과 경로 검증을
소유하며 Git은 추적·비추적 파일과 표준 ignore를 사용하고 fsmonitor는 끈다. 일반 디렉터리는
심볼릭 링크를 따라가지 않는 제한된 탐색을 사용한다. 루트 디렉터리 핸들을 고정하고
대기 중인 모든 경로 요소를 `openat(O_DIRECTORY | O_NOFOLLOW)`로 열어 열거한다.
회귀 테스트는 대기 디렉터리와 그 조상을 외부 심볼릭 링크로 교체했을 때 외부 항목을
읽기 전에 실패하는지 확인한다.8192항목·4MiB를 넘으면 실패하고 파일 시스템
검증은 각 작업 사이에 취소와2초 제한을 확인한다. 개별 파일 시스템 호출 지연은 OS에 따른다.
변경·삭제·심볼릭 링크·작업공간 밖 경로는 제외하며 실패하거나 한도를 넘으면 매핑을 해제한다.
상대 경로·`./`·정확한 canonical 절대 경로를 검증된 파일 URL로 연결한다. 퍼센트 인코딩은 한 번만
풀어 기존 목록에서 찾고 플러스는 그대로 유지하며 잘못된 인코딩은 연결하지 않는다. raw file URL이나 미확인
경로는 기존 fallback을 유지한다. 불변 resolver를 교체해 레이아웃 캐시를 갱신하며 원문·코드는
그대로 유지한다. 링크는 이동 수단이며 첨부·입력 승인 권한이 아니다. 갱신 사이에 경로가 바뀔 수 있다.
현재 지원하는 로컬 실행 작업공간에만 적용하며 원격 경로를 프런트엔드 파일로 바꾸지 않는다.

TuiStatusLine::new는 고유 키와 표시 값으로 이루어진 최대 16개 항목의 전체 snapshot을
받는다. 키는 비어 있지 않고 제어 문자 없이 64바이트 이하여야 한다. 각 값은 제어 문자를
보이는 표기로 바꾸기 전후 모두 1024바이트 이하로 제한한다. 키 순서로 정렬한 값을
구분자로 연결하고 빈 값은 생략한다. 중복 키, 첫 초과 항목·바이트와 한 줄로 표시할 수
없는 텍스트는 갱신 값 생성 전에 TuiStatusError로 거부한다. 기본값은 상태 줄을 지운다.

호스트는 터미널 소유 전이나 재진입 사이에 TuiSession::set_status_line을 호출하거나,
실행 중 기존 readiness 알림과 함께 AgentPoll::StatusLine(status)를 전달할 수 있다.
이는 호스트 표시 관측이며 프로바이더 wire, Turn, 메시지 출력, 사용량 집계와 세션 저널을
변경하지 않는다. 데이터 집계와 문맥 변경 시 교체는 호스트 책임이다. 같은 live snapshot은
다시 그리기를 요청하지 않는다. 터미널 재진입에서는 세션 값을 유지하며 오프라인
프리뷰는 별도 상태를 사용한다.

상태 행은 metrics와 키 안내 사이에 놓이며 기존 muted 의미 색상을 사용한다. 입력·키 안내와
최소 대화 공간을 확보한 뒤 여유가 있을 때만 한 행을 예약한다. 좁은 폭에서는 grapheme을
온전히 유지하며 ASCII 점으로 생략을 표시한다. 높이가 부족하면 선택적 행을 숨긴다.
원문 내보내기는 변경하지 않는다. 이는 명시적으로 제공하는 일반 텍스트 상태 줄이며
임의의 인터랙티브 위젯이나 승인 UI 교체 기능은 아니다.

```rust,ignore
let status = TuiStatusLine::new([
    ("checks", "Tests: 18 passed"),
    ("worker", "Worker: ready"),
])?;
// 호스트 AgentConnection::poll 구현에서 반환한다.
Ok(AgentPoll::StatusLine(status))
```

고정한 pi footer의 확장 상태 루프는 키 정렬·텍스트 정리 후 폭을 제한한 행을 추가한다.
`/preview status`, status-update, status-clear는 모델 Turn이나 대화 기록 없이 yo의 공통
호스트 poll 경로를 확인한다. 한도, 제어 문자, 정렬, 80/24/3/80 화면, 갱신·삭제, 의미 색상
변경, 원문 보존과 높이 0–30을 검증한다.

### 입력 줄 편집

공통 입력 편집기에서 Ctrl+A/E는 실제 줄의 시작·끝으로 이동하고 Ctrl+U/K는 줄 시작·끝
방향으로 삭제한다. Ctrl+Y는 최근 삭제 내용을 현재 커서에 다시 넣는다. 줄 경계에서
다시 삭제하면 개행을 제거해 이웃 줄을 연결한다. 연속된 뒤쪽 삭제는 보관 내용 앞에,
앞쪽 삭제는 뒤에 합친다. 문자 입력·커서 이동·붙여넣기·초안 교체는 새 삭제 묶음을 시작한다.
이 보관 영역은 편집기 내부의 최근 삭제 내용이며 시스템 클립보드, undo 기록 또는
여러 삭제 기록을 순환하는 kill ring이 아니다.

화면 줄바꿈이나 터미널 폭이 아니라 실제 LF/CRLF가 삭제 범위를 결정한다. grapheme
경계를 지켜 한글·결합 문자·이모지 시퀀스를 나누지 않는다. 키 release와 추가 modifier는
이 단축키를 실행하지 않는다. 일반 메시지와 인터뷰 메모가 같은 편집기를 사용하며
편집만으로 Turn을 제출하거나 중단하지 않는다. Home/End는 기존 대화 이동 역할을 유지한다.

고정한 pi packages/tui/src/components/editor.ts의 deleteToStartOfLine·deleteToEndOfLine과
Codex bottom_pane/textarea.rs의 kill action 분기를 비교했다. 테스트는 유니코드,
연속 삭제 순서, 개행 연결, CRLF 원자성, 24/80열 레이아웃과 복원 인터뷰 답변을 다룬다.
`/preview interview`에서 Tab으로 메모를 추가하고 답한 뒤 Shift+Tab으로 돌아와
Ctrl+U로 메모를 교체한다. 최종 제출 결과에 수정한 값이 남는지 확인한다.

### 이전 인터뷰 질문 수정

ActivityQuestion은 선택적으로 previous_question, draft, draft_choice를 전달한다.
필드가 없으면 이전 이동을 비활성화하고 편집기를 변경하지 않는다. 복원 선택은 실제
선택지의 1부터 시작하는 번호여야 하며 메모 지원과 초안이 필요하다. 잘못된 프로필은
기존 원문 fallback으로 처리한다. 직렬화된 프로필에는 기존 16 MiB 한도를 적용한다.

지원하는 질문에서 **Shift+Tab**을 누르면 현재 텍스트 또는 선택·메모를 보관하고 이전
질문으로 돌아간다. 공통 PreviousQuestion intent/response는 정확한 활성 요청 ID와
미제출 초안을 admission·backpressure·저널 codec을 통해 전달한다. TUI는 최신 요청
패널이 표시된 뒤에만 이동하며, 비어 있는 편집기에 초안을 한 번 복원한다. 이후 스냅샷
갱신으로 사용자가 수정한 내용을 덮어쓰지 않는다. 복원 답변의 Enter 제출도 최신 표시가
필요하며 slash로 시작하는 복원 텍스트는 로컬 명령이 아닌 답변으로 보낸다. 기능이 없는
일반 질문은 기존 입력 동작을 유지한다.

Codex 어댑터가 전체 질문과 초안을 소유하며 이동마다 새로운 요청 ID로 질문을 발행한다.
이동 시 JSON-RPC 답변을 전송하지 않는다. 다시 답하면 해당 질문의 기록을 교체하고
마지막 질문에서 수정된 전체 답변 map을 전송한다. 첫 질문의 이전 이동, 오래된 요청과
범위를 벗어난 선택을 거부한다. 이전 초안이 질문 프로필 한도를 넘으면 해당 이동을
노출하지 않는다. Grok은 이 기능을 제공하지 않으며 기존 미지원 사용자 입력 경로로
거부한다. 공통 TUI는 Codex wire 필드나 질문 묶음 상태를 읽지 않는다.

`/preview interview`도 독립된 초안과 새 요청 ID로 같은 제스처를 지원한다. 첫 답변 →
둘째 초안 → Shift+Tab → 첫 답변 수정 → 둘째 초안 복원 → 최종 제출 순서와 80/24/80
폭 변경을 확인한다. 회귀 테스트는 프로바이더 payload, 오래된 요청, 선택 프로필 검증,
초안 복원, 명령 저널 왕복과 최신 화면 확인을 다룬다. 실행 중 프로바이더 인터뷰 재시작,
암호화·비밀 초안은 이 테스트로 검증하지 않는다.

### 기록된 인터뷰 답변

Codex 질문은 입력 전에 전송 시점을 안내한다. 중간 답변은 로컬에 기록하며 마지막
질문을 제출할 때 모든 답변을 전송한다. 질문이 하나면 제출 시 해당 응답을 전송한다고
표시한다. 어댑터가 기존 공통 질문 프로필로 문구를 전달하며 다른 프로바이더의 전송
방식은 변경하지 않는다. 지원하는 질문에서는 아래 방식으로 이전 답을 수정할 수 있다.

Codex 사용자 입력 응답 Activity는 질문, 해석된 선택 이름 또는 직접 입력한 답, 별도로
표시한 메모를 보존한다. 중간 답변은 후속 질문을 기다리며 기록되었음을 표시하고,
마지막 완료 표시는 JSON-RPC 응답 쓰기가 성공한 뒤에만 큐에 넣는다. 쓰기 실패는 전송
완료 표시를 만들지 않으며 요청은 미응답 상태로 남는다. wire 답변 배열은 바꾸지 않는다.
표시 내용은 할당 전에 ToolOutput::MAX_SNAPSHOT_BYTES로 제한한다. 초과하면 표시 생략을
명시하고 원래 응답은 저널에 남는다. 답·메모의 정확한 한도와 첫 초과 바이트를 검증한다.

TUI는 typed UserInputResponse를 “Answer recorded” 제목으로 보존하며 기존 활동 성공·실패
색상, 본문 스타일과 본문 폭 설정을 사용한다. 좁은 화면에서 제출한 답이 숨지 않도록
일반 도구 접기를 적용하지 않는다. Markdown·이미지·링크 문자열은 원문으로 표시하고
도구 본문 callback에는 전달하지 않는다. 실패·중단 footer와 전체 일반 텍스트 내보내기를
보존한다. 80/24/80 화면, 사용자 색상, 원문, 내보내기, 순차 질문 ID, 인터뷰 중단과 응답
쓰기 실패를 검증한다. 오프라인 인터뷰 프리뷰도 같은 Activity 종류를 사용하며 로컬 기록에
오프라인임을 명시한다. 고정한 Codex history_cell/request_user_input.rs의 필드 구분을 참고했다.
비밀 입력은 남은 작업이다.

### 미완료 인터뷰 요약

대기 중인 인터뷰가 중단되거나 Turn이 실패·종료되거나 에이전트가 질문을 닫으면,
adapter는 기록/전체 답변 수와 미응답 수, 각 질문의 Recorded·Unanswered 상태를 한 번
표시한다. 수치는 메뉴 선택이나 편집기 초안이 아닌 보관된 답변에서 계산한다. 제출이
미완료임을 명시하며, 이미 전체 답변을 전송한 경우에는 미완료 경고를 만들지 않는다.
종료 시 요청 binding을 제거하므로 늦은 resolved 알림이나 응답이 요약을 중복하거나
질문을 다시 열지 못한다. 미응답 요청을 둔 정상 Turn 완료는 기존 core의
RequestStillUnanswered 검증에서 계속 실패한다. 표시를 위해 답변을 합성하지 않는다.

연결 EOF와 수신 실패에서도 요청 binding을 닫고 런타임이 최종 실패를 처리하기 전에
요약을 전달한다. 원래 수신 실패를 보존하며 재관찰은 닫힌 연결을 다시 읽거나 경고를
중복하지 않는다. 답변 쓰기가 실패한 경우에는 전송 완료 기록을 만들지 않는다. 오류는
원래 실패 종류·메시지에 앞서 기록된 답변 수와 전체 질문 수를 덧붙이고 최종 제출이
확인되지 않았음을 표시한다. 답변·메모를 다시 출력하거나 쓰기 실패만으로 상대가 아무
바이트도 받지 않았다고 단정하지 않는다. 마지막 전송 시도 답변은 이전 기록 수에 넣지 않는다.

기존 ActivityNotice 경고·본문 색상과 폭 설정을 사용하고 질문 문자열은 원문으로
표시한다. 직렬화된 전체 notice는 기존 출력 스냅샷 한도에 맞춘다. 질문 목록이 한도를
넘으면 목록 생략을 명시하고 수치와 종료 맥락은 보존한다. 마지막 허용 크기·첫 초과,
0개·일부 답변, 모든 원격 종료 경로, 전송 성공 시 경고 없음, 중복 종료와 오래된 답변을
검증한다. 네이티브 오프라인 인터뷰도 취소 시 이전 답변 기록을 보존한 채 경고를 표시한다.
첫 답변 전과 한 번 답변한 뒤의 취소를 검증한다.

채팅의 `Alt+↑/↓`는 현재 폭으로 배치한 항목 시작 사이를 이동한다. 항목 중간에서 위로 이동하면 해당 항목 시작으로 돌아가며, 마지막 시작을 지나 아래로 이동하면 최신 출력 추적을 재개한다. 프롬프트 초안은 유지한다. 80·24열과 frame 반영 전 여러 키 입력을 확인한다. `Alt+O`는 반영된 화면의 활동 항목을 토글한다. 스크롤 중에는 첫 표시 항목, 최신 추적 중에는 마지막 항목을 대상으로 한다. 이동이 반영되기 전 토글은 무시한다. 항목별 상태는 폭 변경에도 유지되며 ToolRenderer·DocumentRenderer의 expanded 값에 반영된다. Ctrl+O는 개별 설정을 지우고 전체 상태를 바꾼다. 개별 설정이 있으면 inline publication을 멈춰 영구 출력이 해당 선택을 잃지 않게 한다. 원문 내보내기는 유지한다.

`histogram` fence는 공백으로 구분한 유한한 숫자 1~64개를 받는다. 첫 줄의 `bins: N`으로 동일 폭 구간을 1~32개 지정한다(기본 8개). 구간은 왼쪽 경계를 포함하고 오른쪽 경계를 제외하되 마지막 구간에는 최댓값을 포함한다. 상수는 한 구간으로 표시하고, 부동소수점 경계가 중복되면 실제 구간 수를 줄인다. 경계는 수치로 왕복 가능한 표기를 쓰며 입력 숫자 표기도 유지한다. 잘못된 옵션·비유한 값·65번째 표본은 원문으로 표시한다. 기존 차트 막대 색상·단색 대체·반응형 배치를 재사용한다. `/preview` → `histogram`은 실제 Rust 오프라인 예시다. 경계별 개수, 구간32/33, 표본64/65, 상수·최소 양수·범위 overflow와80/24/12열을 확인한다. [plotext 히스토그램 안내](https://plotext.readthedocs.io/en/latest/bar.html)의 구간별 빈도 표현을 참고했으며 Python 의존성은 추가하지 않는다.

`scatterchart`는 공백으로 구분한 유한한 `x,y` 좌표 1~64개를 받는다. 입력 순서와 독립적으로 X 수치 간격을 적용하고 점을 연결하지 않는다. 기존 Braille 차트·팔레트·ASCII 대체 표시를 재사용한다. X/Y 범위와 입력 좌표 표기를 유지한다. 상수 축은 중앙에 점을 놓으며 극단적인 유한 범위도 제한된 정규화로 처리한다. 축이 들어가지 않는 폭에서는 범위와 좌표 목록을 남긴다. 잘못된 좌표나 65번째 점은 원문으로 표시한다. `/preview` → `scatter`는 실제 Rust 예시다. 연결선 없는 점·불균일 X 간격·80/24/12열·상수/최소 양수/극단 축·64/65 한도를 검사한다. 산점도와 연결선의 구분은 [plotext 기본 그래프](https://plotext.readthedocs.io/en/latest/basic.html)를 참고했다.

`linechart`·`stepchart`·`scatterchart` 첫 줄에 `height: N`을 지정할 수 있다(그래프 2~16행, 기본 6행). 축·범위·원래 수치 표기는 유지하며 주석 행은 그래프 높이에 별도로 더해진다. 잘못된 값이나 범위 밖 높이는 원문을 표시한다. 좁은 폭의 대체 표시는 그대로이며 지정한 그래프를 할당하지 않는다. 프로바이더 설정이 아닌 개별 fence 커스터마이징이다. `/preview` → `chart-heights`에서3행 선 그래프와10행 산점도를 비교한다. 80/24열의 실제 축 행 수,12열 대체 표시,16/17 경계와 잘못된 옵션을 검사한다.

호스트는 `TuiDocument::new(ActivityDocument { title, markdown })`로 검증한 값을 `AgentPoll::Document`로 보낸다. 모델 Turn을 시작하지 않는다. TuiDocument는 불변 공유 값이며 기존의 빈 제목 금지·인코딩된 profile16MiB 한도를 전달 전에 검증한다. 실제 poll과 오프라인 프리뷰는 Chat에 임시 문서 항목을 추가하며 core 이벤트·백엔드 입력·저널 record를 만들지 않는다. 기존 DocumentRenderer·테마·개별 펼침·원문 내보내기를 적용한다. 반복 전달은 별도 항목으로 추가되므로 중복 방지는 호스트가 맡고 프로세스 재시작 시 자동 복원하지 않는다. `/preview` → `session-document`는 표·코드 안내 예시다. 실제 apply_agent_poll 경계, 활성 Turn 없음,80/24/80 커스텀 펼침 렌더링, 원문 보존, 정확한 크기 한도/첫 초과와 제출 수락·문서만 내보내는 프리뷰를 검사한다.

`TuiDocument::with_expanded(bool)`은 해당 문서의 초기 펼침 상태를 선택한다. 생략하면 전체 활동의 펼침 상태를 따른다. 기존 항목 식별자 기반 설정과 inline publication 보호를 사용하며 이후 Alt+O·Ctrl+O 사용자 조작은 그대로 적용한다. Markdown과 원문 내보내기는 바꾸지 않는다. session-document 프리뷰는 처음부터 펼쳐 표와 코드를 보여준다. 두 전역 상태와 초기 옵션 생략/false/true,80/24/80에서 이후 사용자 조작을 검사한다.

실제 `/help`는 처음부터 펼친 TuiDocument를 표시한다. 명령 목록은 기존 registry에서 생성하고 읽기·편집·승인·인터뷰 안내는 command/help.rs가 소유한다. 기존 DocumentRenderer·테마를 적용한다. 명령 초안을 비우는 로컬 동작이며 Turn을 시작하거나 대기 요청에 응답하지 않는다. 등록된 모든 명령·키 안내·80/24/80 처음/끝 탐색·접힌 행 없음·원문 보존을 검사한다. 오프라인 프리뷰 밖의 `/help`와 대기 요청 테스트를 함께 확인한다.
