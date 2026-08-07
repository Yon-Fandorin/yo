# 검증

변경된 경계를 기준으로 증거를 고른다. 기대 동작과 중요한 실패를 구분할
수 있는 가장 작은 검사부터 시작한다. 그다음 검사를 넓히고
[Slice](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#slice-contract)를
닫는다.

## 증거 계층

| 계층 | 확인할 수 있는 것 | 예시 |
|---|---|---|
| 프로세스 내부 | 결정론적 상태, protocol, layout, rendering, 주입된 실패 동작 | `yo-core` engine/runtime test, `yo-tui` component test, rendering parity golden |
| 호스트 통합 | 선택 설치 프로그램 없이 실제 호스트 기능을 사용한 동작 | `yo-cli`의 Linux PTY, termios, process signal, 터미널 복원 test |
| 외부 환경 | 설치 프로그램, 인증, 중첩된 터미널 환경과의 호환성 | Codex, tmux, 로컬 `sshd`, SSH, SSH 내부 tmux |

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
| Codex protocol 변환이나 provider ID 연결 | `cargo test -p yo-core backend::codex::tests` | `crates/yo-core/src/backend/codex/tests.rs` |
| 해석된 input, 편집, paste, binding, 종료 gesture | `cargo test -p yo-tui input::` | `yo-tui/src/input` 곁의 test |
| prompt 줄 바꿈, cursor 표시, viewport | `cargo test -p yo-tui prompt::` | `yo-tui/src/prompt` 곁의 test |
| `@` trigger, stale 결과, 선택 치환, local 순위, Git ignore 탐색 | `cargo test -p yo-tui workspace_reference`와 `cargo test -p yo-core workspace_reference` | `yo-tui/src/prompt/workspace_reference.rs`와 `yo-core/src/workspace_reference` |
| `$` trigger, Codex catalog decode, scope filtering, 비활성 행, typed skill 선택 | `cargo test -p yo-tui skill_reference`, `cargo test -p yo-core skill_reference`, `cargo test -p yo-core backend::codex::skill_catalog` | `yo-tui/src/prompt/skill_reference`, `yo-core/src/skill_reference`, `yo-core/src/backend/codex/skill_catalog.rs` |
| 대화 기록 item, streaming revision, scroll | `cargo test -p yo-tui transcript::` | `yo-tui/src/transcript` 곁의 test |
| shell 조합, layout, Surface, Unicode 너비, text flow | `cargo test -p yo-tui` | 소유 `yo-tui` 모듈 곁의 test |
| ANSI operation이나 표시 mode 정책 | `cargo test -p yo-tui terminal::` | `yo-tui/src/terminal` 아래 test |
| Inline 또는 Fullscreen mode 동작 | `cargo test -p yo-tui terminal::mode::` | `yo-tui/src/terminal/mode` 아래 test |
| live loop 순서, backpressure, submission draft 소유권, event Projection | `cargo test -p yo-tui runner::` | `yo-tui/src/runner` 아래 test |
| 같은 완성 frame의 터미널·HTML Projection | `cargo test -p yo-tui --test rendering_parity` | `crates/yo-tui/tests/rendering_parity`와 golden |
| process termination이나 실제 터미널 복원 | `cargo test -p yo-cli pty_tests::` | `crates/yo-cli/src/pty_tests.rs` |
| Unix process coordinator 상태와 보상 | `cargo test -p yo-cli process::termination::tests` | `crates/yo-cli/src/process/termination/tests` |
| Rust test 바로 위에 필요한 설명 | `cargo xtask check test-explanations` | `crates/`와 `tools/` 아래 Rust source |
| Slice 변경이 bind된 로컬 write-set 안에 머무는지 | `cargo xtask check slice-scope` | 하나의 활성 Slice worktree; planner가 먼저 `cargo xtask slice-contract bind <contract.json>` 실행 |
| 두 Slice contract의 현재 통합 기준점이 같고 선언한 소유권이 겹치지 않는지 | `cargo xtask check slice-parallel <left.json> <right.json>` | direct Slice는 `develop`, Wave Slice는 해당 Wave branch 사용 |
| 수용된 Slice가 여전히 검수한 로컬 branch patch와 정확히 같고 안전하게 정리할 수 있는지 | `cargo xtask slice close plan <slice> <plan.json>` 후 `cargo xtask slice close apply <plan.json>` | 깨끗한 통합 worktree에서 실행하고 apply 전에 제거 효과와 보존할 coordination 경로를 검토 |
| 저장소 hook 정책이나 구조화된 개발 검사 | `cargo test -p xtask` | `tools/xtask/src` |
| Linux/macOS 조건부 compile | `bash tools/validation/yo-cli-unix-matrix.sh` | 로컬 host 결과와 두 host를 위한 `.github/workflows/unix-compile.yml` |
| tmux, SSH, SSH 내부 tmux 동작 | [터미널 환경 매트릭스](./terminal-matrix.md) 참고 | ignored `yo-cli` 환경 test |

이 명령들은 시작점이지, 영향받은 인접 경계를 무시해도 된다는 허가가
아니다. 예를 들어 `AgentSession` 수정으로 frontend가 보는 admission
결과가 달라진다면 집중 test와 TUI runner test가 모두 필요할 수 있다.

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

## Slice 종료 기준선

집중 검사가 통과하면 저장소 기준선을 실행한다.

```bash
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
hk check
```

`cargo test`는 일반 test를 실행하고 ignored test를 compile하지만, 환경
의존 ignored test를 실행하지 않는다. `hk check`는 변경 경로에 따라
`hk.pkl`에서 저장소 검사를 고른다. formatting, test 설명, 영향받은
crate 검사, Methexis 검사, Developer Docs 검사가 여기에 포함된다.
설치와 hook 사용법은
[`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#local-checks)가
소유한다.

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

Slice가 platform이나 외부 환경 경계를 바꾼다면 기준선이 이를 검사했다고
주장하지 말고 관련 matrix 명령을 추가한다.

검수한 후보를 squash했다는 이유만으로 바뀌지 않은 기준선을 다시 실행하지
않는다. 정확한 Git diff로 두 commit의 tree가 같고, 통합 과정에 conflict 해소나
다른 수정이 없으며, toolchain과 환경이 같고, 외부 상태 증거가 만료되지 않았고,
commit hook이 통과한 경우에만 후보 결과를 수용 commit의 증거로 재사용한다.
그 밖에는 영향받은 검사를 다시 실행한다. 이 재사용은 후보 자체의 검증이나
검수를 대체하지 않는다.

Slice 종료 정리 명령은 이 검증 기준선의 일부가 아니다. 요청한 파일에 plan을
직접 발행한 뒤 이미 수용된 결과를 소비한다. 로컬 worktree, 표준 임시 Slice
contract, Slice branch를 제거하기 전에 정확한 ref, 검수 trailer, patch
identity, worktree 청결 상태, binding, contract hash, plan hash를 다시
검사한다. plan은 보존할 직계 coordination 항목도 모두 나열하며, apply는 그
목록이 바뀌면 거절하고 해당 항목을 삭제하지 않는다. plan은 제거할 worktree와
해당 Slice coordination 디렉터리 바깥에 저장한다. 통합 workflow는
[`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#review-and-integration)를
참고한다.

## 유용한 소유자

- hook 선택: [`hk.pkl`](https://github.com/Yon-Fandorin/yo/blob/develop/hk.pkl)
- 구조화된 저장소 검사: [`tools/xtask`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/xtask/src/lib.rs)
- Unix host compile 검사: [`tools/validation/yo-cli-unix-matrix.sh`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/validation/yo-cli-unix-matrix.sh)
- rendering parity fixture: [`crates/yo-tui/tests/fixtures/rendering-parity/README.md`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/tests/fixtures/rendering-parity/README.md)
- test 설명 정책: [`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#test-code)
