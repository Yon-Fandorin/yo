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
| Agent-session admission, concurrency, 시작, 종료 | `cargo test -p yo-core agent_session::tests` | `crates/yo-core/src/agent_session/tests` |
| Codex protocol 변환이나 provider ID 연결 | `cargo test -p yo-core backend::codex::tests` | `crates/yo-core/src/backend/codex/tests.rs` |
| 해석된 input, 편집, paste, binding, 종료 gesture | `cargo test -p yo-tui input::` | `yo-tui/src/input` 곁의 test |
| prompt 줄 바꿈, cursor 표시, viewport | `cargo test -p yo-tui prompt::` | `yo-tui/src/prompt` 곁의 test |
| 대화 기록 item, streaming revision, scroll | `cargo test -p yo-tui transcript::` | `yo-tui/src/transcript` 곁의 test |
| shell 조합, layout, Surface, Unicode 너비, text flow | `cargo test -p yo-tui` | 소유 `yo-tui` 모듈 곁의 test |
| ANSI operation이나 표시 mode 정책 | `cargo test -p yo-tui terminal::` | `yo-tui/src/terminal` 아래 test |
| Inline 또는 Fullscreen mode 동작 | `cargo test -p yo-tui terminal::mode::` | `yo-tui/src/terminal/mode` 아래 test |
| live loop 순서, backpressure, event Projection | `cargo test -p yo-tui runner::` | `yo-tui/src/runner` 아래 test |
| 같은 완성 frame의 터미널·HTML Projection | `cargo test -p yo-tui --test rendering_parity` | `crates/yo-tui/tests/rendering_parity`와 golden |
| process termination이나 실제 터미널 복원 | `cargo test -p yo-cli pty_tests::` | `crates/yo-cli/src/pty_tests.rs` |
| Unix process coordinator 상태와 보상 | `cargo test -p yo-cli process::termination::tests` | `crates/yo-cli/src/process/termination/tests` |
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

Slice가 platform이나 외부 환경 경계를 바꾼다면 기준선이 이를 검사했다고
주장하지 말고 관련 matrix 명령을 추가한다.

## 유용한 소유자

- hook 선택: [`hk.pkl`](https://github.com/Yon-Fandorin/yo/blob/develop/hk.pkl)
- Unix host compile 검사: [`tools/validation/yo-cli-unix-matrix.sh`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/validation/yo-cli-unix-matrix.sh)
- rendering parity fixture: [`crates/yo-tui/tests/fixtures/rendering-parity/README.md`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/tests/fixtures/rendering-parity/README.md)
- test 설명 정책: [`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#test-code)
