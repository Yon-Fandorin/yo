# 터미널 환경 매트릭스

터미널 동작의 기준은 실제 PTY 출력이다. HTML fixture는 진단과 parity
review를 돕지만 이 검사를 대신하지 않는다.

## 일반 test가 검사하는 범위

Linux에서 ignored가 아닌 `yo-cli` test는 실제 PTY를 만들고 Inline 종료,
Fullscreen 종료, signal에 의한 복원, 두 mode의 두 번 연속
`Ctrl+Z`/`SIGCONT` 세대를 실행한다. tmux, `sshd`, 설치된 Codex는
필요하지 않다.

```bash
cargo test -p yo-cli pty_tests::
```

process coordinator test는 handler 설치, rollback, shutdown compensation,
thread ownership, 격리된 subprocess signal 동작을 별도로 실행한다.

```bash
cargo test -p yo-cli execution::process::termination::tests
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

위임 Codex image 경계는 adapter와 core의 offline test로 검증한다. 정확한
`0.153.4` wire 증거, 선택한 model의 정확한 `inputModalities`, start와 steer 양쪽의
순서가 있는 반복 immutable PNG projection, 보수적인 inherited-history resume/rebind
admission, 완전한 32 MiB outbound JSONL 경계를 검사한다. 경계 test는 정확히 32 MiB를
허용하고 첫 초과 byte를 peer에 보내기 전에 거절한다. server response도 같은 경계를
사용하며 overflow는 protocol failure로 분류한다. 이 검사는 fake JSONL peer를 사용하고
model service에 접속하지 않는다.

## 설치된 Grok 검사

Session 생성이나 model 추론 요청 없이 ACP 초기화, cached-login 인증,
정상 종료를 검사한다.

```bash
cargo test --locked -p yo-backend-delegated-grok \
  runtime::tests::session::local_grok_authenticates_and_shuts_down_without_a_session \
  -- --ignored --exact
```

2026-09-10 Linux에서 Grok `1.0.25 (f7e67d6988e2)`로 이 검사가 통과했다.
독립 96×32 tmux 실행에서도 실제 Rust `yo --fullscreen --model host:grok`으로
빈 입력창까지 도달한 뒤 Ctrl+D로 정상 종료했다. 검사한 바이너리 SHA256은
`42f7c955d966d56825213c18a8ce59c7d655acb17532fa606dc2f449f2b83d7a`다.
프롬프트를 제출하거나 클립보드를 읽지 않았다. 인증과 빈 Session 시작만 입증하며
인증된 Turn, 실제 과거 대화, 스킬, native read-only
sandbox에는 각각 별도 증거가 필요하다. 일반 Grok suite는 `session/load` 응답 전에
같은 Session의 과거 update 1,025개를 즉시 버리고 이후 새 응답과 resumable outcome을
전달하는 경계를 별도로 검증한다. 다른 Session, 서버 요청, 응답 신원 오류와 기존
무관 메시지 대기열 상한은 계속 적용한다. 이는 결정적인 adapter 검사이며 실제로
긴 Grok Session을 측정한 결과는 아니다.

이후 격리된 합성 스킬 프롬프트는 Grok 자체 기록에서 예상한 marker로 완료됐지만,
Yo는 continuation anchor를 기록하기 전에 unsigned 응답 ID protocol 오류로 실패했다.
새 프로세스에서 재개를 시도하지 않았으며 수락된 요청을 재전송하지 않았다.
스킬 본문을 포함한 오프라인 leader mode Responses API 검사는 숫자 ID를 반환해
이 오류를 재현하지 못했다. 이제 adapter는 잘못된 ID의 값을 노출하지 않고 JSON
종류만 오류에 표시한다. 이는 진단 개선이며 실패 원인을 확정하거나 해결한 것은 아니다.

## 로컬 tmux와 Linux SSH 검사

Linux 또는 macOS의 두 표시 mode에서 로컬 tmux를 검사한다.

```bash
cargo test -p yo-cli --test terminal_matrix local_tmux_ \
  -- --ignored --nocapture --test-threads=1
```

Linux의 두 표시 mode에서 SSH와 SSH 내부 tmux를 검사한다.

```bash
cargo test -p yo-cli --test terminal_matrix ssh:: \
  -- --ignored --nocapture --test-threads=1
```

로컬 tmux test는 지원하는 두 Unix host에서 실행할 수 있으며 호환되는
`tmux`와 Codex가 설치되어 있어야 한다. SSH test는 Linux 전용이다.
localhost에 격리된 `sshd`를 시작하고 임시 key를 생성한 뒤 fixture
디렉터리를 제거한다. 호환되는 로컬 `ssh`, `sshd`, `ssh-keygen`, Codex,
`USER`가 로컬 SSH account 이름으로 설정되어 있어야 한다. 중첩된 경우에는
tmux도 필요하다.

각 경로는 빈 입력 `Ctrl+D` 종료와 두 번 연속
`Ctrl+Z` → job 정지 → `fg` terminal generation을 모두 검사한다.
job-control 검사는 매 정지 구간의 터미널을 해당 경로의 실제 interactive shell
termios와 비교하고, `yo` 프로세스가 커널 stopped 상태인지 확인하며, 각 `fg`
뒤 요청한 표시 mode를 다시 획득하는지 확인한다. SSH 내부 tmux는 바깥 SSH
PTY 복구도 추가로 확인한다.

필요한 명령이나 assertion을 사용할 수 없으면 test는 실패한다. 빠진 환경을
성공한 skip으로 바꾸지 않는다.

## macOS 실제 host 증거

2026-07-30에 `develop` commit `085e763`으로 수용된 tree를 macOS 26.2
arm64에서 실행했다. 해당 host에서
`cargo test --workspace --all-targets`가 통과했다.

그다음 80x24 실제 zsh PTY에서 두 표시 mode를 실행했다. 두 mode 모두
raw/no-echo 입력에 진입하고 빈 입력 `Ctrl+D`로 정상 종료했으며,
`Ctrl+Z` → job 정지 → `fg` 세대를 두 번 완료했다. Fullscreen은 각
세대마다 alternate screen을 해제하고 다시 획득했으며, Inline은
alternate screen에 진입하지 않았다.

동일한 시나리오는 `-f /dev/null`과 격리된 socket을 사용한 tmux 3.6a에서도
통과했다. 매 정지 구간에서 shell termios가 복원됐고 각 `fg` 뒤 요청한
mode를 다시 획득했다. 이는 명시적인 실제 host 관찰이며 일반 cross-platform
test set에 포함된 검사가 아니다.

그다음 `develop` commit `af546a5`로 수용된 정확한 tree를 대상으로 80x24
zsh PTY에서 SSH 경로를 실행했다. SSH가 소유한 interactive zsh는 두 mode
모두 빈 입력 `Ctrl+D` 종료와 두 번의
`Ctrl+Z` → job 정지 → `fg` 세대를 완료했다. Inline은 alternate screen
밖에 머물렀고 Fullscreen은 매 세대마다 이를 해제하고 다시 획득했다. SSH
session 종료 뒤 로컬 PTY termios도 바뀌지 않았다.

동일한 SSH session 구조에서 `-f /dev/null`과 격리된 socket을 사용해 tmux
3.6a에도 접속했다. 매 정지 구간과 최종 종료 시점에 pane은 zsh로 돌아오고
alternate screen을 해제했으며 기준 termios와 일치했다. 각 `fg` 뒤에는
pane이 `yo`로 돌아오고 raw terminal 설정과 요청한 표시 mode를 다시
획득했다. 중첩 session 종료 뒤 바깥 로컬 PTY도 복원됐다. 이 SSH 관찰은
실제 원격 host를 사용했으며 일반 test set이 아니라 증거 기록이다.

## Mac 클립보드에서 Linux yo로

2026-09-10에 candidate `8343292f0281b9e8d7321fd504bed97ead205b7c`의 이미지
첨부를 macOS 26.6.2 arm64에서 SSH Unix 소켓 전달을 거쳐 격리된 tmux의 실제
Linux Rust 바이너리까지 검증했다. Mac은 `pngpaste` 0.2.3과 candidate의
`tools/clipboard_bridge.py`를 사용했으며 Homebrew 경로를 명시해야 했다.
설치한 helper의 해시는
`5e3fa05d25b1d856a89927dc1e5cd236d9801b80a4c6ac42af6704ace0b57c58`였다.

Mac의 기존 pasteboard 항목을 읽어 Mac 메모리에만 보관하고 일반 pasteboard에
64 × 32 합성 PNG를 넣었다. 임시 guard는 실제 `pngpaste` 출력을 Mac 안에서
버퍼링하고 fixture 픽셀과 변경되지 않은 pasteboard revision을 확인한 뒤에만
바이트를 전달했다. `tmux send-keys`로 전달한 Ctrl+V가 20·40·96열에서 이미지를
첨부했다. 기존 pasteboard를 복원한 뒤 guard가 다음 캡처를 거절했을 때도 yo는
기존 이미지와 초안을 모두 유지했다. 모델 Turn은 제출하지 않았다. 앱은 정상 종료했고
검증이 만든 SSH 연결, helper, 소켓, tmux와 fixture를 정리했다.

연결 실행기는 별도로 Linux 실제 PTY에서 Ctrl+Z → foreground 복귀 두 번과
빈 입력창의 Ctrl+D 종료를 검증했으며 셸 터미널 설정도 복원됐다. 이 관찰은 native
획득, 전달과 Rust 입력 처리를 확인한다. 실제 Mac 키보드 단축키나 터미널의 픽셀
이미지 프로토콜 검증은 포함하지 않는다. 클립보드 원본은 여전히 프로세스마다 명시적으로
선택해야 한다.

이후 정식 SSH 설정 경로도 같은 Mac fixture, 첨부 폭, 붙여넣기 실패 시 보존과
정상 종료 검사를 Python 실행기·bridge 서비스·전달 소켓 없이 통과했다.
해당 Rust 바이너리의 SHA256은
`f124a62a87ab04c06a163fa9c6752859eb4171cd0debaeaf5fbde07f3b8739cc`였다.
테스트 전용 SSH shim이 guard가 있는 native reader를 선택했으며 실제 제품 worker,
SSH 인증과 내장 원격 supervisor가 획득을 수행했다. 기존 클립보드를 복원하고 임시
fixture를 제거했으며 모델 요청은 없었다. Python supervisor의 별도 합성 검사는
시간 초과·연결 종료 신호에서 reader와 자식 정리를 확인하고 실패한 캡처의 부분 출력을
거절한다.

## 플랫폼 검사 범위

현재 실행 가능한 환경 매트릭스의 범위는 다음과 같다.

| host와 경로 | Inline | Fullscreen | 증거 |
|---|---:|---:|---|
| Linux 직접 실제 PTY | Yes | Yes | 일반 `yo-cli` test가 두 mode의 종료·반복 일시정지/재개와 Fullscreen termination을 검사 |
| Linux 로컬 tmux | Yes | Yes | ignored test가 정상 종료와 두 번의 shell 기반 일시정지/재개를 검사 |
| Linux SSH | Yes | Yes | ignored test가 정상 종료와 두 번의 원격 shell 기반 일시정지/재개를 검사 |
| Linux SSH 내부 tmux | Yes | Yes | ignored test가 정상 종료, 두 번의 중첩 일시정지/재개, 바깥 PTY 복구를 검사 |
| macOS compile | — | — | 실제 macOS 26.2 arm64 host에서 workspace all-target test 통과 |
| macOS 직접 실제 PTY | Yes | Yes | 실제 host에서 정상 종료와 두 번의 shell 기반 일시정지/재개를 검사 |
| macOS 로컬 tmux | Yes | Yes | ignored test가 정상 종료와 두 번의 shell 기반 일시정지/재개를 검사하며, 실제 host 실행으로 mode 재획득과 shell termios 복원도 확인 |
| macOS SSH | Yes | Yes | 실제 host에서 정상 종료, 두 번의 원격 shell 기반 일시정지/재개, mode 재획득, 바깥 PTY 복원을 검사 |
| macOS SSH 내부 tmux | Yes | Yes | 실제 host에서 정상 종료, 두 번의 중첩 일시정지/재개, pane mode·termios 전환, 바깥 PTY 복원을 검사 |

`tools/validation/yo-cli-unix-matrix.sh`는 현재 Unix host의 모든 `yo-cli`
target을 검사한다. 출력은 해당 실행만 설명한다. 현재 host는 verified이고
다른 host는 `unverified(not run on current host)`이다. 이는 다른 host를
사용할 수 없다는 뜻이 아니며, 별도로 기록된 실제 host 증거를 지우지도
않는다. CI workflow는 Linux와 macOS에서 동일한 compile 검사를 독립적으로
실행한다. compile은 터미널 동작 증거를 대신하지 않는다.

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
같은 host의 SSH나 SSH 내부 tmux까지 verified로 표시하지 않는다.

계약:
[rendering 검증 기준](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.validation-matrix.md)

실행 결과를 passed, failed, unverified로 분류하려면
[검증](./#결과-읽기)으로 돌아간다.
