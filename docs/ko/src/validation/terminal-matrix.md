# 터미널 환경 매트릭스

터미널 동작의 기준은 실제 PTY 출력이다. HTML fixture는 진단과 parity
review를 돕지만 이 검사를 대신하지 않는다.

## 일반 test가 검사하는 범위

Linux에서 ignored가 아닌 `yo-cli` test는 실제 PTY를 만들고 일반
Fullscreen 종료와 signal에 의한 복원을 실행한다. tmux, `sshd`, 설치된
Codex는 필요하지 않다.

```bash
cargo test -p yo-cli pty_tests::
```

process coordinator test는 handler 설치, rollback, shutdown compensation,
thread ownership, 격리된 subprocess signal 동작을 별도로 실행한다.

```bash
cargo test -p yo-cli process::termination::tests
```

이 호스트 통합 검사는 일반 package test에 포함된다. 통과했다고 해서 tmux나
SSH 동작까지 실행되었다는 뜻은 아니다.

## 설치된 Codex 검사

model Turn 없이 stdio initialize와 shutdown 경계를 검사한다.

```bash
cargo test -p yo-core local_codex_initializes_and_shuts_down \
  -- --ignored --nocapture --test-threads=1
```

버려도 되는 workspace에서 인증된 model Turn 하나, tool 실행, 파일 변경,
semantic event, 명시적 cleanup을 검사한다.

```bash
cargo test -p yo-core local_codex_completes_a_real_file_change \
  -- --ignored --nocapture --test-threads=1
```

두 번째 검사는 외부 model 연산을 수행한다. Turn 대기는 최대 180초이며
전체 실행 시간에는 Codex 시작과 종료도 포함된다. 호환되는 Codex 인증과
쓰기 가능한 Codex 상태가 있는 환경에서만 실행한다.

## Linux tmux와 SSH 검사

두 표시 mode에서 로컬 tmux를 검사한다.

```bash
cargo test -p yo-cli --test terminal_matrix local_tmux_ \
  -- --ignored --nocapture --test-threads=1
```

두 표시 mode에서 SSH와 SSH 내부 tmux를 검사한다.

```bash
cargo test -p yo-cli --test terminal_matrix ssh:: \
  -- --ignored --nocapture --test-threads=1
```

로컬 tmux test에는 호환되는 `tmux`와 Codex가 설치되어 있어야 한다. SSH
test는 localhost에 격리된 `sshd`를 시작하고 임시 key를 생성한 뒤 fixture
디렉터리를 제거한다. 호환되는 로컬 `ssh`, `sshd`, `ssh-keygen`, Codex,
`USER`가 로컬 SSH account 이름으로 설정되어 있어야 한다. 중첩된
경우에는 tmux도 필요하다.

필요한 명령이나 assertion을 사용할 수 없으면 test는 실패한다. 빠진 환경을
성공한 skip으로 바꾸지 않는다.

## 플랫폼 검사 범위

현재 실행 가능한 환경 매트릭스의 범위는 다음과 같다.

| host와 경로 | Inline | Fullscreen | 증거 |
|---|---:|---:|---|
| Linux 직접 실제 PTY | Unverified | Yes | 일반 `yo-cli` test가 현재 Fullscreen을 검사 |
| Linux 로컬 tmux | Yes | Yes | ignored 환경 test |
| Linux SSH | Yes | Yes | ignored 환경 test |
| Linux SSH 내부 tmux | Yes | Yes | ignored 환경 test |
| macOS compile | — | — | 실제 macOS CI host의 `cargo check` |
| macOS 터미널 동작 | Unverified | Unverified | 아직 실제 host 환경 실행이 없음 |

`tools/validation/yo-cli-unix-matrix.sh`는 현재 Unix host의 모든 `yo-cli`
target을 검사하고 다른 host는 unverified로 보고한다. CI workflow는
Linux와 macOS에서 동일한 compile 검사를 독립적으로 실행한다. compile은
터미널 동작 증거를 대신하지 않는다.

## 매트릭스 실행 결과 보고하기

결과는 작지만 명확하게 기록한다.

```text
Host:
Route and mode:
Command:
Result: passed | failed | unverified
Observed failure or missing prerequisite:
```

한 경로의 결과로 다른 경로를 추정하지 않는다. 로컬 tmux가 통과했다고 해서
SSH, SSH 내부 tmux, macOS까지 verified로 표시하지 않는다.

계약:
[rendering 검증 기준](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.validation-matrix.md)

실행 결과를 passed, failed, unverified로 분류하려면
[검증](./#결과-읽기)으로 돌아간다.
