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

2026-09-10에는 `gpt-6-astra`를 사용한 격리된 공식 Codex 실행에서도 합성 model
요청 두 번으로 이미지 Turn과 새 프로세스 재개를 검증했다. 클립보드 fixture에 대해
정확한 `red,blue` 답변을 받았고, 재개 Turn도 정확한 회상 검사를 통과했다.
continuation anchor 두 개와 기존 journal prefix를 보존했다. 검사한 바이너리 SHA256은
`a20df491ff6445f521bb45d8f03f67f79fb7f72dbea355312ac36c9af394603d`다.
소유한 tmux Session을 종료하고 임시 인증을 제거했다. 이는 검사한 이미지 입력과
복구 경로를 입증하며 바깥 터미널의 이미지 픽셀이나 다른 model/version 조합을
검증하지 않는다. 승인된 요청 두 번의 예산은 모두 사용했다.

### 저장된 명령 규칙과 자동 승인

2026-09-11 Linux Rust yo를 격리된 110×44 tmux에서 실행해 공식 Codex `0.154.0`,
`gpt-6-astra`에 대한 합성 Turn 네 번을 검증했다. 바이너리 SHA256은
`f72d343be2d2aecef3f844033a6e84fa59aa3ffc902c680ced0a6b5bc6200d6a`다.
추론 전에 위임 Codex package 검사와 CLI build가 통과했다.

`approval_policy = "on-request"`, `approvals_reviewer = "user"`,
`sandbox_mode = "read-only"`에서 첫 Turn은 임시 스크립트 하나의 정확한 절대 경로에
대해 실제 TUI의 `Approve + save rule`을 선택했다. Native 규칙 파일에는 해당 allow
prefix만 저장됐고 스크립트는 marker를 정확히 한 번 추가했다. 승인 대기 패널을
40, 20, 110열로 변경하는 동안 실행이나 규칙 저장은 일어나지 않았다. 정상 종료 후
새 프로세스로 재개했을 때 같은 명령은 추가 승인 없이 marker를 한 번 더 추가했다.
기존 Yo journal의 물리적 prefix와 저장 규칙은 그대로 유지됐다.

다른 스크립트에는 새로운 승인 요청이 표시됐다. `Decline and stop`을 선택하자
출력 파일을 만들지 않고 저장 규칙을 유지하면서 Turn을 중단했다. 최초 드라이버는
이 취소된 suffix를 재개하려 했지만, 최신 durable anchor가 없어 현재 continuation
guard가 읽기 전용 기록을 열었다. 이 드라이버 순서 오류는 원래 기록에 보존했다.
완료한 세 시나리오는 재전송하지 않고 네 번째 시나리오는 새 Session에서 실행했다.

새 Session은 같은 read-only sandbox와 빈 규칙 디렉터리에서
`approvals_reviewer = "auto_review"`를 사용했다. 합성 append 한 번이 수동 승인 없이
완료됐다. 캡처한 `item/autoApprovalReview/completed`는 `decisionSource: "agent"`와
`approved`를 보고했고 명령 완료 전에 정확한 command item, thread, turn과 연결됐다.
파일에는 marker가 정확히 하나 있었고 규칙은 저장되지 않았다. 이는 호스트의
[자동 검토 경로](https://learn.chatgpt.com/docs/sandboxing/auto-review)를 검사한 것이며
승인 패널이 없다는 사실만으로 권한 부여를 추정하지 않는다.

오프라인 감사는 선택지 번호, 정확한 wire 결정, native 명령 결과와 Yo 요청·응답
신원을 대조했다. 선택지, 규칙 범위, cleanup, 요청 신원, 명령 상태, 검토 상태,
대상, 순서, 결정 출처, 검토 누락을 변조한 대조군 열 개를 모두 거절했다. 임시 인증,
native 상태와 규칙을 제거하고 작업 폴더를 비웠으며, 소유한 tmux 서버를 종료한 뒤
남은 소유 프로세스가 없음을 확인했다. `turn/start` 네 번은 내부 model이나 reviewer
요청 횟수를 뜻하지 않는다.

### 자동 거절과 네트워크 승인 범위

같은 날 같은 Yo 바이너리와 공식 Codex `0.154.0`을 격리된 120×48 tmux TUI에서
실행해 추가 Turn 열 번을 검증했다. 인증 정보 없는 로컬 Responses fixture가
결정적인 도구 호출과 reviewer 응답을 제공했다. 로컬 model/reviewer HTTP 요청은
20번이었고 실제 서비스 요청은 없었다. 이는 native 정책 실행, adapter 동작과
TUI 표시 결과를 검증하며 서비스 model의 위험 분류 정확도를 측정하지 않는다.

네트워크 검사는 native proxy를 활성화한 이름 있는 permission profile과 격리된
`[experimental_network]` requirements fixture를 사용했다. Mount namespace로 임시
requirements를 테스트 프로세스에만 제공했으며 실제 `/etc/codex`와 사용자의 Codex
설정은 변경하지 않았다. 대상은 수신 요청을 집계하는 loopback HTTP 서버였다.

- 제공된 Session 한정 승인을 선택하자 요청 하나가 도달했다. 실행 중인 같은
  Session의 다음 Turn은 추가 승인 없이 같은 대상에 도달했고, 새 프로세스와
  Session에서는 다시 승인을 요구했다. 취소했을 때는 대상에 도달하지 않았다.
- 제공된 영구 네트워크 허용을 선택하자 loopback 호스트와 HTTP protocol에 대한
  native `network_rule` 하나가 저장됐다. 새 프로세스와 Session에서도 추가 승인
  없이 적용됐다. 다른 호스트 이름에는 여전히 승인을 요구했고 취소하면 대상에
  도달하지 않았다.
- 명시적으로 준비한 네트워크 거부 규칙은 최초 실행과 새 프로세스 모두에서 승인
  요청이나 대상 도달 없이 접속을 차단했다. Yo는 정책이 도메인을 명시적으로
  거부한다는 native 실패 설명을 표시했다. 규칙은 변경되지 않았다.
- 주입한 reviewer 거부 응답은 native `denied` 검토와 `declined` 명령을 만들었다.
  Reviewer 응답을 멈추자 native의 90초 제한에 도달해 `timedOut`과 실패한 명령을
  만들었다. 둘 다 명령의 marker 파일을 만들거나 수동 승인을 요구하지 않았으며,
  Yo에 해당 거부 또는 시간 초과 설명과 함께 `Codex approval warning`을 표시했다.

Codex `0.154.0`은 metadata에 네트워크 amendment 두 가지를 제안했지만
`availableDecisions`에는 허용만 제공했다. 따라서 이 호스트 버전에서는 실제 TUI로
영구 거부를 저장하는 경로를 사용할 수 없다. 준비한 규칙 검사는 차단 적용을
입증하며 대화형 거부 저장 과정을 입증하지 않는다. Adapter 검사
`offered_approval_choices_preserve_exact_scopes_and_wire_payloads`는 호스트가 해당
선택지를 제공할 때 정확한 allow/deny 전송을 별도로 다룬다.

오프라인 감사는 열 번의 결과, 선택지 번호와 정확한 응답, 프로세스 경계, 저장
규칙, 대상 수신, 표시된 경고, 검토와 명령의 신원·순서를 확인했다. 증거를 변조한
대조군 여덟 개를 거절했다. 임시 native 상태, 테스트 규칙과 작업 폴더를 제거하고
소유한 프로세스와 tmux 서버를 종료했으며 loopback listener를 해제했다. Runtime
코드는 변경하지 않았으므로 앞선 package/build 검사 결과가 계속 유효하다.

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
프롬프트를 제출하거나 클립보드를 읽지 않았다. 이 초기 검사는 인증과 빈 Session
시작만 입증한다. 이후 스킬과 재개 검사는 아래에 기록하며 native read-only sandbox는
계속 사용할 수 없다. 일반 Grok suite는 `session/load` 응답 전에
같은 Session의 과거 update 1,025개를 즉시 버리고 이후 새 응답과 resumable outcome을
전달하는 경계를 별도로 검증한다. 다른 Session, 서버 요청, 응답 신원 오류와 기존
무관 메시지 대기열 상한은 계속 적용한다. 이는 결정적인 adapter 검사이며,
실제 문맥 재개 측정 결과는 아래에 기록한다.

최초 격리된 합성 스킬 프롬프트에서 Grok native skill watcher가 요청 없이 보내는
method 없는 `skills-reload` 유지보수 응답을 발견했다. Yo는 continuation anchor를
기록하기 전에 이 문자열 ID를 거절했다. 커밋 `f94d1a42`는 숫자 요청 ID 상관관계를
유지하면서 정확한 유지보수 acknowledgement 형식을 소비한다. Adapter 검사는 시작,
활성 프롬프트, 재개를 다루며 무관한 응답 거절과 프롬프트 조기 완료 방지도 포함한다.

수정된 바이너리는 2026-09-10 격리된 공식 Grok 실행에서 합성 model 제출 두 번과
관측된 watcher acknowledgement 두 번으로 검증을 통과했다. 첫 스킬 답변이 정확히
일치했고, 스킬 원본을 제거한 뒤 새 프로세스 재개에서도 정확한 회상을 검증했다.
continuation anchor 두 개와 journal prefix를 보존했다. 검사한 바이너리 SHA256은
`f72d343be2d2aecef3f844033a6e84fa59aa3ffc902c680ced0a6b5bc6200d6a`다.
소유한 tmux Session을 종료하고 임시 인증을 제거했다. 설치된 호스트는 이미지 prompt
지원을 광고하지 않으며 native read-only review sandbox는 계속 사용할 수 없다.

### Grok 큰 문맥 재개

2026-09-11 같은 바이너리와 Grok `1.0.25 (f7e67d6988e2)`를 격리된 Linux 110×40
tmux에서 실행해 공식 서비스에 대한 합성 제출 일곱 번을 검증했다. 위임 Grok
suite는 64개 검사가 통과했고 환경 검사 두 개는 ignored였다. 앞서 통과한 CLI
build는 변경되지 않았다. 도구, 웹 검색, 하위 에이전트와 memory를 비활성화했고,
인증과 호스트 상태에는 임시 Grok home을 사용했다.

여섯 Turn에 합성 기록 960개, 총 입력 120,272바이트를 제공했고 각각 정확한 확인
답변을 받았다. 정상 종료한 뒤 새 Yo 프로세스에서 같은 Yo Session과 native Grok
Session을 재개해 앞부분·중간·마지막 batch의 무작위 checkpoint 값 세 개를 정확히
회상했다. 재개 prompt에는 checkpoint 이름만 주고 정답 값은 제공하지 않았다.
일곱 Turn이 모두 완료됐고 continuation anchor 일곱 개를 보존했으며, 기존 물리적
journal prefix 192,069바이트가 바이트 단위로 동일했다. 도구나 승인 실행은 없었다.

Native `session/load`는 상관관계가 일치하는 응답 전에 update 19개를 재생했고,
해당 응답 이후 일곱 번째 `session/prompt`가 전송됐다. 재생된 메시지는 Yo 답변을
중복 생성하지 않았다. 오프라인 감사는 정확한 답변, 입력 크기, Session·프로세스
신원, journal prefix, anchor, 순서와 cleanup을 확인했고 증거를 변조한 대조군
여섯 개를 거절했다. 계획한 제출 일곱 번을 재시도 없이 모두 사용했다. 검증 후
임시 인증, native 상태와 소유한 프로세스를 제거했다.

이는 측정한 입력 크기와 일곱 Turn 깊이에서의 재개를 입증한다. Update 1,025개의
mailbox 경계는 별도 결정적 검사이며, 이번 실행은 그만큼의 실제 update 재생,
context compaction이나 호스트 최대 context window에서의 동작을 입증하지 않는다.

## 로컬 tmux와 Linux SSH 검사

이전 사용자 관찰에서 SSH/tmux 터미널에 산 이미지가 표시되고 HTTP 링크가
열리는 것을 확인했다. 원격 README 파일 링크는 열리지 않았다. 해당 파일 링크
제한은 저장소 README에 기록되어 있으며 원격 파일 전송이나 내장 viewer는 없다.
이는 터미널·버전 matrix가 남아 있지 않은 사용자 보고이며 자동 픽셀 검사나 모든
터미널에 대한 주장은 아니다. 이전 로컬 메모의 “픽셀 확인 대기” 상태는 이 응답으로
해소됐다.

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

### 현재 Apple Silicon 빌드와 입력 검사

2026-09-11 `f57e61e5` 기반 수정 트리를 macOS 26.6.2 arm64에서 고정된
`nightly-2026-05-22` toolchain으로 검사했다. 최초 실행은 장치 ID 자료형 불일치,
Grok admission의 Linux 전용 import, FIFO 생성·임시 경로 alias·Unix socket 경로
길이·실행 파일 위치·종료된 socket 설정에 대한 이식 불가능한 테스트 가정을
드러냈다. 수정은 기존 파일 신원·경로 검사를 유지하며 native metadata 경계에서만
Apple의 signed device ID를 정규화하고 정규 경로와 길이가 제한된 fixture를 사용한다.

Native core suite는 706개 검사가 통과했다. CLI 검사는 unit test 524개와 integration
test 7개가 통과했고 환경 검사 18개는 ignored였다. `yo-core`,
`yo-backend-delegated-grok`, `yo-cli`의 Clippy, host-target Unix matrix와 native
오프라인 `chat_preview` 빌드도 통과했다. Linux에서는 core 706개, Grok 64개, CLI
unit test 532개와 integration test 7개가 통과했다. 변경한 workspace-reference
fixture는 별도의 집중 검사 31개를 통과했다. 이 수치는 중복되므로 독립적인 검사
범위인 것처럼 합산하지 않는다.

사용자가 Mac tmux pane에 오프라인 실행 파일을 열고 해당 창의 자동 입력을
명시적으로 요청했다. 주입한 입력으로 한글 텍스트, 커서 이동, Backspace와 삽입,
bracketed 여러 줄 붙여넣기, 초안 지우기, 빈 입력의 `Ctrl+D` 종료, 이후 셸 명령과
canonical 입력·echo 복구를 확인했다. 사용자 소유 tmux pane은 셸에 남겨 두었다.
Native preview 바이너리 SHA256은
`34ebe6cf4633d15e36d826ff8ce9774ec63aa2a7014027a35fa753cd6eb04a85`다.
모델 요청이나 계정 접근은 없었다. 이는 실제 Mac tmux 경로에서 입력을 주입한
검사이며, 물리 키보드·IME 조합·터미널 앱의 Command-V 처리는 사용자 직접 관찰이
별도로 필요하다.

임시 체크아웃, 소스 전송 파일과 테스트 프로세스를 정리했다. 사용자의 tmux Session과
기존 클립보드 설치는 보존했다.

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
