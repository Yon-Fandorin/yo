# 실행 흐름

변경이 크레이트 경계를 지나거나 오류 메시지만으로 소유자를 알기 어려울
때 이 흐름을 사용한다. 여기에는 현재 구현 경로가 담겨 있다. 각 경계가
어떤 의미여야 하는지는 계속 Methexis가 기준이다.

## 시작

프로세스 정책과 agent Session이 준비된 뒤에만 터미널을 획득한다.

```text
yo-cli
  표시 mode와 glyph profile 해석, cwd 확보
  TerminationCoordinator 설치
  Host identity와 Session repository 열기
  workspace 정규화와 SessionDescriptor 생성
  CodexBackend transport 시작
      ↓
yo-core AgentSession
  worker 시작
  descriptor envelope 시도
  CreateSession
      ↓
Codex app-server
  initialize
  thread/start
      ↓
yo-core
  SessionCreated
      ↓
yo-tui
  터미널을 획득하고 Inline 또는 Fullscreen mode 진입
```

| 단계 | 현재 소유자 | 확인할 내용 |
|---|---|---|
| 1 | [`yo-cli/src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) | `run`이 표시 옵션과 작업 디렉터리를 확보하고 종료 coordinator를 설치한다. Host identity와 Session storage를 열고 workspace를 canonicalize한 뒤 시각이 일치하는 UUIDv7 `SessionDescriptor`를 만든다. |
| 2 | [`yo-core/backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `CodexBackend::spawn`이 설정을 검증하고 stdio transport를 시작한다. provider handshake는 아직 하지 않는다. |
| 3 | [`yo-core/agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | `AgentSession::start_cancellable_with_repository`가 backend와 local repository를 `yo-agent-runtime`이라는 worker thread로 넘긴다. 종료 관찰을 막지 않으면서 시작 완료를 기다린다. |
| 4 | [`yo-core/agent_session/worker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/worker.rs) | `AgentWorker::initialize`가 descriptor-only Journal envelope를 먼저 시도한 뒤 `AgentRuntime`을 통해 `CreateSession`을 보낸다. storage pressure가 있으면 descriptor와 이후 activity를 복구 가능한 volatile prefix로 함께 유지한다. |
| 5 | [`yo-core/backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `CreateSession`이 `initialize`와 `thread/start`를 수행하고 semantic engine이 `SessionCreated`를 만든다. |
| 6 | [`yo-tui/runner/unix.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs) | `run_session_with_mode`가 첫 터미널 소유 세대의 input과 터미널 상태를 획득하고 이미 선택된 표시 mode로 들어간다. |

handshake 중에 종료 요청이 오면 `AgentSession::start_inner`가 취소
callback을 관찰하고 backend 중지를 요청한 뒤 worker 정리를 기다린다.
그리고 TUI에 Session을 넘기지 않은 채 반환한다. 이 경우 터미널 mode
코드가 아니라 여기서 조사를 시작한다.

공개 host flag는 표시를 위한 `--inline` 또는 `--fullscreen`과 built-in
ASCII glyph profile을 위한 `--ascii`이며 순서와 관계없이 사용할 수 있다.
표시 flag를 생략하면 Inline, `--ascii`를 생략하면 호환 기본값인 Rich를
사용한다. 알 수 없는 flag, 반복한 `--ascii`, 둘 이상의 표시 flag는
provider나 터미널을 시작하기 전에 실패한다. 선택한 glyph profile로 보존할
`TuiSession`을 생성하므로 준비한 frame과 마지막 plain session output은
같은 committed appearance snapshot을 읽는다. Glyph 선택은 명시적이며
`TERM`이나 `NO_COLOR`를 검사하지 않는다. CLI는 자신이 아는 backend 이름과
홈 경로를 줄여 쓴 작업 디렉터리 label도 보존되는 session에 전달한다. 이
label은 화면 표시용 metadata일 뿐 backend Session을 선택하거나 식별하지
않는다.

계약:
[session publication](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.session-publication.md),
[frame 일관성](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.frame-consistency.md),
[glyph profile](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.glyph-profiles.md),
그리고
[resolved cell style](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.resolved-style.md).

## Workspace reference 도움

Chat에서 유효한 `@query`를 입력하면 agent command와 분리된 다음
nonblocking 경로를 따른다.

```text
PromptEditor + cursor
  ↓ revision에 묶인 trigger snapshot
yo-core local 실행 workspace provider
  ↓ Git ignore를 따르는 파일·디렉터리 + 결정적인 Unicode 정규화 순위
TuiState prompt overlay
  ↓ Tab 또는 Enter
정확한 @query span을 바꾸고 typed identity 보존
```

`yo-tui`는 scan, stale 결과 거절, overlay 입력, editor span 변환을
소유한다. `yo-core::LocalWorkspaceReferenceProvider`가 local 실행 탐색
의미와 background Git·filesystem 작업을 소유하고, `yo-cli`는 이 capability를
생성해 연결만 한다.
candidate와 request/update type은 `yo-core`에 있으므로 remote 실행
provider를 연결해도 filesystem 권한이 frontend로 이동하지 않는다.
inventory는 보이는 파일과 디렉터리를 포함하고 nested Git ignore,
repository exclude, 설정된 global exclude를 따르며 directory symlink를
따라가지 않는다. 각 행은 basename과 dimmed 부모 경로를 왼쪽 읽기 흐름에
함께 두고, 오른쪽 끝은 중립적인 `File` 또는 `Dir` 종류에만 사용한다.
디렉터리 label과 선택 후 token은 `/`로 끝나 입력 중에도 종류가 눈에 보인다.
첫 query는 header에 검색 중 상태를 보여줄 수 있지만 연속 입력 중에는 현재 panel을 유지하고
최신 결과가 도착할 때 한 번만 다시 그려 중간 loading frame이 깜빡이지 않게 한다.
panel title은 `Files`이며 header hint는 활성 binding에서 도출해 key만 강조하고
caption은 dim 처리한다. Rich glyph는 이동에 `↑↓`, ASCII는 `Up/Down`을 쓰고,
익숙한 terminal 표기인 `Enter`, `Esc`, `^C`는 문자 그대로 유지한다.

이 Slice는 structured submission admission 직전에서 의도적으로 멈춘다.
항목을 고르면 token은 눈에 보이게 치환되고 typed reference가 남지만,
그 뒤 Enter를 누르면 draft를 보존하고 아직 structured submission이
연결되지 않았다고 알린다. 승인한 identity를 몰래 plain text로 낮추지
않는다.

## 명시적 skill 지원

유효한 `$query`를 입력하면 같은 prompt trigger 생명주기를 재사용하되,
별도의 frontend 중립 skill port에서 metadata를 찾는다.

```text
PromptEditor + cursor
  ↓ revision-bound $ trigger
CodexSkillReferenceProvider worker
  ↓ 현재 cwd에 대한 Codex skills/list descriptor
Skills overlay
  ↔ Left/Right로 cached 행을 All, Workspace, User, System, Admin 중 하나로 filter
  ↓ Tab 또는 Enter
정확한 $query span을 바꾸고 catalog identity와 revision selector 보존
```

catalog worker는 수명이 짧은 Codex app-server 연결을 소유하며 terminal event
loop를 막지 않는다. Codex가 보고한 `repo`, `user`, `system`, `admin` scope만
사용하고 filesystem path에서 provenance를 추측하지 않는다. 같은 이름도
identity가 다르면 별도 행으로 남고, 비활성 skill은 이유와 함께 보이지만
선택할 수 없다. local adapter는 정확한 `SKILL.md` byte를 hash해 entry revision으로
사용한다. revision을 읽을 수 없는 행은 admission이 나중에 검증할 수 없는
selector를 만들지 않도록 비활성화한다. 새 Skills overlay를 열 때는 새
`skills/list` snapshot을 강제로 읽고 catalog generation을 올린다. 연속 입력은
같은 snapshot을 대상으로 최신 query로 합친다. 선택적인 scope filter는 panel 왼쪽 하단에만 둔다. Left와
Right는 이미 받은 후보만 좁히므로 discovery를 다시 실행하거나 prompt를
재배치하지 않는다.

V1은 accept된 명시적 skill을 최대 하나만 보존한다. 선택은 skill 본문을
읽거나 실행하거나 model context에 주입하거나 draft를 제출하지 않는다.
제출 시점 admission이 정확한 항목을 다시 읽고 검증할 수 있을 때까지 Enter는
draft를 보존하고 실패-폐쇄하며, 보이는 `$name`만으로 충분한 권위라고
간주하지 않는다.

## 활성 Turn 하나

제출된 prompt는 다음 경로를 지난다.

```text
terminal input
    ↓
TuiState::handle
    ↓ 변경 불가능한 InputSubmission
TuiAgentConnection
    ↓
AgentSession queue와 bounded command lane
    ↓
AgentWorker
    ↓ 같은 SubmissionId를 수락 또는 거절
    ↓ AgentCommand::StartTurn or SteerTurn
AgentRuntime
    ├── AgentEngine으로 검증
    ├── AgentBackend를 통해 수락
    ├── AgentEngine으로 commit
    └── command와 event를 SessionJournal에 추가
          ↓
Codex app-server adapter
    ↓ BackendEvent
AgentRuntime
    ↓ commit한 뒤 SessionJournal에 추가
AgentSession의 합칠 수 있는 change lane
    ↓ 내용 없는 깨우기 알림
TuiAgentConnection + TranscriptReader
    ↓ 순서가 보장된 AgentPoll::Record
    ↓
TuiState::observe_record
    ├── 간결한 Chat Projection
    └── chronological Transcript / anchored Request Projection
          ↓ 선택된 view
completed Surface
    ↓
Inline 또는 Fullscreen presenter
```

조사할 때 유용한 지점은 다음과 같다.

1. [`TuiState::handle`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/state.rs)는
   변경 불가능한 `InputSubmission` 하나를 캡처한다. 같은 `SubmissionId`의
   `Accepted` outcome이 올 때까지 plain text를 입력창에 보존한다. 그사이
   사용자가 새 draft를 편집했다면 그 새 text는 지우지 않는다. 거절은 draft를
   보존하며, 중복되거나 오래된 outcome은 아무 영향도 주지 않는다.
2. [`TuiAgentConnection`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/agent/mod.rs)은
   좁은 local adapter다. dispatch, retry, submission outcome을 전달하고, 하나로 합쳐진
   Session 변경 알림을 `TranscriptReader`의 크기가 제한된 suffix 읽기로
   바꿔 순서가 보장된 record를 TUI에 제공한다. Session이나 provider
   의미는 소유하지 않는다.
3. [`agent_session/admission.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/admission.rs)는
   Submit을 `StartTurn` 또는 `SteerTurn`으로 결정한다. `Queued`는 bounded
   worker lane이 command 소유권을 받았다는 뜻일 뿐 최종 수락이 아니다.
   state lock이 사용 중이거나 lane이 가득 찼다면, 같은 `SubmissionId`를
   가진 내부가 드러나지 않는 pending command를 TUI loop가 다시 시도하도록 반환한다.
   첫 dispatch가 그 ID를 Session에 예약하므로 재사용은 다른 backend command가
   실행되기 전에 거절된다.
4. [`AgentWorker`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/worker.rs)만
   runtime을 실행하고 polling할 수 있다. runtime과 backend 수락이 성공한 뒤
   정확한 ID의 `SubmissionOutcome::Accepted`를 공개한다. typed rejection
   channel은 다음 reference-admission Slice를 위해 준비되어 있다. 그전까지
   structured `@`, `$` draft는 실패-폐쇄 상태를 유지한다. 터미널을 소유한
   thread는 provider I/O를 기다리지 않는다.
5. [`AgentRuntime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs)은
   command 검증, backend 수락, semantic commit, Journal publication 순서를
   보장한다. `StartTurn`과 `SteerTurn`은 correlation이 있는 submission 경계로만
   들어오며 일반 command 경계는 `SubmissionId` 없는 두 command를 거절한다.
   worker가 소유한 durable writer는 text update를 크기가 제한된
   immutable segment로 바꾸고, commit된 record를 공개하기 전에 semantic
   commit을 동기식으로 append한다. 권위 있는 backend snapshot은 이미
   durable한 segment를 수정하지 않고 새 message revision을 시작한다. 아직 segment를
   내보내지 않은 연속 replacement는 같은 unpublished revision을 공유하고, 빈 최종
   replacement는 zero-byte terminal seal로 표현한다.
   provider 관찰 결과도 semantic engine을 통해 변환한 뒤 변경 알림을
   공개한다. 거절된
   command와 잘못된 backend event는 commit된 의미로 기록하지 않지만,
   실패를 닫으며 만들어진 terminal event는 기록한다.
   `AgentSession::transcript_reader`는 같은 Journal에서 크기가 제한된 읽기
   전용 suffix 복사본을 제공하며 내부 lock이나 저장 구조는 노출하지 않는다.
   capacity나 storage 실패가 나면 semantic 결과를 volatile live suffix로
   공개하고 `JournalDurability::Gap`을 유지한다. storage가 다시 write를 받고 열린
   모든 message에 실제 terminal seal이 생기면 같은 writer가 complete snapshot
   하나를 공개한 뒤 incremental commit으로 돌아간다. 빈 message도 zero-byte
   terminal seal을 받고, `ActivityStarted` 뒤 첫 text segment 전에 crash가 나면
   recovery가 interrupted zero-byte seal을 만든다. segment가 없는 empty replacement는
   시간이나 ordering 경계에서 `MessageReset`으로 저장하고, 종료 시에는 zero-byte
   terminal seal로 표현한다. adapter가 semantic `ModelWork`로 승인한 관찰 가능한 plan이나
   reasoning summary도 같은 segment와 seal 경로를 쓴다. yo가 받지 않은 숨겨진 reasoning과
   승인하지 않은 backend-specific Request Audit payload는 이 semantic 경로 밖에 남는다. 공유 observation stream은 각 typed
   durability 전환을 영향을 받는 semantic record보다 먼저 정렬하므로 coalesced worker
   wake-up도 Gap-to-Durable 전환을 지우지 못한다. CLI adapter는 이 순서를 정확한 cutoff
   종류와 함께 TUI 상태에 전달한다. Chat·status 행·banner 중 어떤 방식으로 표현할지는
   별도 product 계약으로 남긴다. 저장된 Session 검사는 아래의 별도 read-only
   경로를 따른다. 실행 가능한 continuation은 frontend history Projection에서
   상태를 만들지 않고, 아래의 별도 검증된 recovery 경로를 사용한다.
6. [`drain_agent`와 `redraw`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs)는
   이미 확정된 Transcript record를 소비하고 TUI 상태를 갱신한다.
   완성된 `Surface`를 조합해 활성 presenter로 보낸다. `runner/view.rs`는
   같은 record stream에서 Chat, Transcript, Request를 선택한다. Chat의
   사용자 입력은 `StartTurn` 또는 `SteerTurn` command가 이 순서에 나타난
   뒤에만 표시된다. `@`나 `$` discovery를 dispatch하는 editor mutation은 provider
   결과보다 먼저 즉시 redraw되고, 이전 usable panel은 pending snapshot gate 뒤에
   계속 보인다. animated 작업 marker나 고정 문구 activity sheen을 실제로 그린 Chat
   frame은 보이는 period 중 가장 짧은 값을 반환하고, runner는 터미널 세대 epoch의
   다음 경계를 예약해 event redraw와 합친다. 숨김·좁음·낮음·idle·한 frame·zero-size
   indicator는 timer를 활성화하지 않는다.

승인된 순서, 중단 gesture, 정직한 status 데이터, 반응형 맞춤 정책은
[정적 입력 chrome 계약](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.chrome.input-stack.md)이
소유한다. 이 runtime에서 `shell::chrome`은 활성 상태와 `TuiSessionInfo`로
typed 행을 계산하고 폭에 맞춘다. `shell::chrome::help`는 label을 개행하거나
자르는 대신 우선순위가 낮은 action 전체를 제거한다. 공용
`input::key_notation` formatter는 설정된 semantic binding에서 `Esc`, `^C`,
`^D`, `S-Enter` 같은 terminal 관례 표기를 만들지만, action이 현재 사용
가능한지는 결정하지 않는다. `shell`은 그 영역을 prompt 주변에 조합하고,
`input::control`은 아주 작은 frame이 시각 안내를 표시하지 못해도 mapping된
interrupt intent를 dispatch한다.
정확한 marker cycle과 runner 시간 경계는
[activity motion profile](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.activity-motion-profile.md)과
[activity motion scheduling](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.activity-motion-scheduling.md)
계약이 소유한다.

change lane은 command나 event 내용을 싣지 않으며 용량은 하나다. 따라서
여러 commit이 읽지 않은 알림 하나로 합쳐져도 이력은 사라지지 않는다.
구체적인 local reader가 Journal sequence를 따라 당시 확인한 head까지
계속 읽기 때문이다. backend가 최종 실패해도 adapter는 Journal에 이미
확정된 실패 record를 먼저 모두 공개한 뒤 연결 오류를 보고한다.

Codex JSON과 provider identifier는 backend adapter 밖으로 나오지 않는다.
터미널 input event와 rendering type은 `yo-tui` 밖으로 나오지 않는다.
그 사이를 지나는 command와 event type은 `yo-core`가 소유한다.

## 저장된 Session 검사

저장 history는 live startup 경로에 들어가지 않는다.

```text
yo session [--all] [--details]
  ↓ 기존 Host identity와 repository를 만들지 않고 읽기
LocalSessionReader::discover
  ↓ 검증된 tail summary
workspace로 거른 metadata table을 stdout에 출력

yo session SESSION_ID [--view chat|transcript]
  ↓ writer lease 없는 한 시점의 physical snapshot
yo-core read_stored_session
  ↓ envelope 검증 + Journal recovery + message normalization
StoredSessionHistory
  ↓ yo-tui archived Projection
plain stdout
```

`yo-cli/src/command.rs`는 command 문법, `session.rs`는 선택과 table/output routing,
`config.rs`는 날짜 형식 설정, `storage.rs::open_default_reader`는 writer startup과
분리된 읽기 전용 조합을 소유한다. stdout이 terminal이면 `session.rs`가 관찰한 폭,
Session 전용 열 우선순위와 continuation hint를 범용 `yo-tui::plain` renderer에
전달한다. 먼저 PATH와 DETAIL, 다음으로 continuation/version, 시작 시각, workspace를
접는다. 짧은 label/value pair는 주 행 아래를 왼쪽부터 채우고, 다음 pair 전체가
들어가지 않을 때만 다음 줄로 옮긴다. PATH와 DETAIL은 진행 중인 flow를 끝내고
독립된 한 줄을 사용한다. label/value pair가 전체 폭에 들어가면 같은 줄에 두고,
부족할 때만 개행하는 label block으로 바꾼다. 너무 긴 flow pair도 같은 block 형태로 승격한다.
접힌 상세가 있는 record 사이는 빈 줄 하나로 구분한다. 고정된
identity/status/updated 시각도 들어가지 않으면 공유 table header를 없애고 모든
필드를 label이 있는 세로 card로 바꾼다. 접힌 값은 terminal grapheme cell 경계에서 개행하며
잘라내지 않는다. 하나의 atomic grapheme이 terminal 전체 폭보다 넓으면 쪼개거나
버리지 않고 명시적으로 실패한다. terminal의 heading은 왼쪽 끝에서 굵게 표시하고
값만 두 cell 들여쓴다. stdout이 terminal이 아니면 파이프와 파일 결과가 terminal
폭에 따라 달라지지 않도록 ANSI style이 없는 한 줄 표를 유지한다.

선택 설정 파일은 읽기만 하고 만들지 않는다. Linux는
`${XDG_CONFIG_HOME:-$HOME/.config}/yo/config.yaml`, macOS는
`$HOME/Library/Application Support/yo/config.yaml`을 사용하며, `YO_CONFIG`로
명시적인 경로를 고를 수 있다. 첫 schema는 다음과 같다.

```yaml
version: 1
session:
  list:
    date_format: "%Y-%m-%d %H:%M %:z"
```

날짜 문법은 strftime과 호환되고 UPDATED와 STARTED 모두 보는 머신의 local
timezone으로 표시한다. 설정 파일이 없으면 위 기본값을 사용한다. 파일을 읽을 수
없거나 version/field/크기/date format이 잘못되면 조용히 기본값으로 대체하지 않고
명시적으로 실패한다. reader는 nonblocking descriptor 하나를 열어 regular file인지
확인하고 64 KiB와 판별용 한 byte까지만 읽으므로 FIFO가 command를 멈추거나 동시에
커지는 파일이 상한을 우회하지 못한다. repository가
없으면 빈 목록을 반환하고 상태를 만들지 않는다.
직접 history 읽기는
message-recovery interruption을 semantic record에 보존하며 discovery 불일치 진단은
stderr로 보낸다. Physical `v1` 형식만으로는 종료된 writer에 저장되지 않은 volatile
suffix가 있었는지 증명할 수 없으므로, 저장 history는 완전하다고 단정하지 않고
durability continuity를 `not-observable`로 기록한다. Chat은 간결하고 pipe 가능한
출력을 유지하며 기본 direct command는 이 continuity 경계를 stderr로 알린다. Transcript는
확인한 Journal cutoff, message-recovery 상태, durability-continuity 경계, discovery
consistency, 시간순 semantic record를 더한다. 파일 없음과 파일은 있지만 complete
envelope가 없는 상태도 서로 다른 direct-read failure로 유지한다. 어느 archived
출력도 backend를 시작하거나 이후 append를 구독하거나 저장소를 고치거나 그 자체로
continuation을 제공하지 않는다. live `yo --resume UUID`와 `yo --continue`는 대신
아래의 전용 typed continuation recovery를 사용한다.

## 실행 중인 observation view

선택한 TUI Projection은 표시만 바꾸며 Session authority를 바꾸지 않는다.

```text
읽기 전용 AgentPoll::Record stream
    ├── Chat: 간결한 activity/message Projection + 편집 가능한 prompt
    └── 전체 semantic record Projection
          ├── Transcript: chronological command/event와 Activity detail
          └── Request: 정확한 Chat/Transcript context anchor
                ├── 직접 ActivityRequestRef → Request Audit unavailable
                └── 직접 correlation 없음 → associated request 없음
```

현재 `input/view_binding.rs`의 F1/F2/F3가 Chat/Transcript/Request를
선택한다. 이 mapping은 typed 표시 정책 seam이며 Projection 상태가 아니다.
page·line 이동은 활성 view 자체의 viewport를 갱신하고, Chat과 Transcript는
각자의 context cursor도 보존한다. Request 이동은 anchor된 diagnostic
page 안에서만 scroll하므로 가까운 request를 탐색하는 browser가 되지
않는다. anchor가 같다면 view로 돌아올 때 보존한 상태를 복원한다.

세 mode 모두 session에서 pin한 appearance snapshot과 기존 Transcript
layout·Surface primitive를 쓴다. status 행은 활성 mode와 key를 표시하고,
좁은 frame에서는 `[C]123`, `[T]123`, `[R]123`으로 줄어든다. terminal
행이 하나뿐이어도 그릴 수 있다. Transcript와 Request는 full-page 읽기
전용 mode이므로 input 경로가 prompt editor에 도달하거나 submission을
만들지 않는다.

현재 TUI adapter는 semantic `TranscriptRecord`와 typed durability 전환을
공개하지만 reader record별 `JournalSequence`는 버리고 Request Audit detail은
공개하지 않는다. Transcript는 이 observation boundary를
출력한다. Request는 정확히 anchor된 record가 가진 correlation만 사용하며,
없으면 `no_associated_request`를 보고한다. 정확한
`ActivityRequestRef`가 있으면 `request_audit_detail_unavailable`을
보고하며 인접 record의 correlation을 빌리지 않는다. repository는 이제 live
worker 경로 안에 있지만, 이 추가 observation 좌표는 아직 frontend 계약에
연결하지 않았다.

## Durable Journal 조합 seam

실행 중인 `AgentSession`은 다음 local 조합을 사용한다.

```text
최초 SessionDescriptor (replay sequence 1, semantic cutoff 없음)
    ↓
명시적 JournalSequence를 가진 semantic Journal record
    ↓ runtime이 binding, accepted-request, outcome, Anchor correlation 추가
    ↓ codec/recovery가 완전한 correlation graph 검증
    ↓ 크기가 제한된 MessageSegment 구성
JournalCommit codec
    ↓ semantic commit 하나
JournalRepository
    ↓ durable semantic prefix와 검증
    ↓ physical append 하나
SessionRepository
    ↓ writer 시각 추가; payload와 완전한 discovery summary를 함께 checksum
single-writer versioned JSONL physical v1

versioned JSONL
    ↓ 제한된 suffix 읽기 + semantic decode
Journal recovery
    ↓
RecoveredJournal 또는 명시적인 recovery 오류
    ↓ physical commit마다 binding epoch와 최신 완결 Anchor 도출

기존 repository root
    ↓ LocalSessionReader (생성·수리·writer lease 없음)
각 Session의 마지막 완결 envelope
    ↓ 닫힌 v1 shape와 CRC32C 검증
사용 가능한 discovery summary 또는 typed Session별 unavailable 결과
```

reader는 진단 문자열을 다시 해석하지 않고 격리, 손상, 미지원 schema, 완결 envelope
없음을 구분한다. 지원되는 summary에 Continuation Anchor가 없으면 `unavailable`,
미지원 schema면 `unknown`이다. 이전 writer가 남긴 pending marker가 있으면 후속
writer가 열리지 않으므로, 진행 중 marker를 만든 바로 그 writer만 append 전 cutoff를
보이게 할 수 있다.

backend가 `CreateSession`을 받기 전에 worker는 UUIDv7 Session identity, Workspace
Host identity, 생성 Host의 canonical path bytes, UUID와 일치하는 시작 시각을 담은
descriptor-only incremental envelope 하나를 먼저 시도한다. descriptor는 Journal에
속한 탐색 데이터지만 frontend Transcript에 들어가거나 semantic `JournalSequence`를
소비하지 않는다. 첫 append가 storage pressure를 만나면 기존 gap 정책에 따라 이후
작업도 volatile하게 유지한다. 처음 성공하는 recovery snapshot은 descriptor로
시작하고 그동안의 complete semantic prefix를 함께 담는다.

replay sequence는 정규화한 모든 저장 record의 순서를 나타내고, `JournalSequence`는
command, event, backend correlation fact만 정렬한다. wire shape도 이 차이를 구조로
강제하므로 descriptor와 message record에는 `journal_sequence`를 넣을 수 없다.
recovery는 correlation record를 semantic 좌표로 indexing하고 모든 참조와 binding
transition을 검증한다. accepted request와 완료된 Turn이 durable prefix에서 모두
증명된 경우에만 Continuation Anchor를 공개한다.

live producer는 이제 `SessionCreated` 뒤에 최초 backend binding을 기록하고,
각 `SubmissionId`를 Start/Steer operation identity로 사용하며,
`TurnFinished(completed)`, resumable outcome, Continuation Anchor를 semantic commit
하나로 공개한다. provider adapter는 epoch나 Journal 좌표를 정하지 않고 opaque
evidence만 반환한다. runtime이 그 semantic identity를 소유하고 Journal만 sequence를
배정한다. Transcript Projection은 correlation 전용 record를 제외한다.

Codex adapter는 model override를 보내지 않아 사용자의 effective model 선택을 보존하고,
`thread/start`가 반환한 `model`과 `modelProvider`를 기록한다. ephemeral thread가 아니라
저장되는 thread를 만든다. continuation에서는 versioned Codex locator만 decode하고
`thread/resume`을 정확히 한 번 보낸 뒤, runtime이 재개 상태를 공개하기 전에 반환된
thread·model provider·model identity를 최신 durable Anchor와 검증한다.

pending message text는 non-text 순서 경계 전에 immutable segment로 강제
저장되므로 동시 Activity event의 원래 순서를 보존할 수 있다. crash 뒤 열린
message가 남으면 recovery는 그 event를 버리지 않고 마지막 durable record 뒤에
interrupted seal을 제안한다. replay가 recovery
record를 합성해야 하거나, reopen 뒤 기존 durable prefix와 필요한 recovery
seal을 생략한 snapshot은 physical append 전에 거부한다. 자기 append 실패를
직접 관찰한 writer는 열린 모든 message가 실제 terminal seal을 받은 뒤 live-gap
snapshot 하나로 그 prefix를 완성할 수 있다. 그전까지 뒤따르는 record는 volatile
suffix에 남아 정상적인 snapshot 연기가 integrity 실패로 바뀌지 않는다. capacity
또는 storage-pressure 실패만 이 자동 재시도 경로에 들어간다.
integrity gap이나 예상하지 못한 snapshot gate는 현재 writer에서 memory-only로
남겨 증명할 수 없는 authority를 반복 제안하지 않으며, 이후 recovery owner가
repository에서 명시적으로 다시 구성해야 한다. 이는 구현된 failure
경계를 찾기 위한 설명이며, 동작 계약은
[Session Journal](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.session-journal.md)과
[Session Repository](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.storage.session-repository.md)
KnowledgeUnit가 계속 소유한다.

CLI는 local repository를 기본으로 활성화한다. `YO_SESSION_REPOSITORY`로 root를,
`YO_SESSION_CAPACITY_BYTES`로 기본 1 GiB 상한을 바꿀 수 있다. Linux는 그 외에
`$XDG_STATE_HOME/yo/sessions` 또는 `$HOME/.local/state/yo/sessions`를 쓰고,
macOS는 `$HOME/Library/Application Support/yo/sessions`를 쓴다. `yo session`은
같은 root를 생성이나 writer lease 없이 연다. `yo --resume UUID`는 먼저 선택한
Session을 read-only로 검증하며, 직접 지정한 대상이 실행 불가능하면 저장소를
변경하지 않고 진단과 함께 archived Chat을 연다. `yo --continue`는 현재 Host와
정규화된 workspace에서 가장 최근 eligible Session을 고르고, 후보가 없으면 새
Session을 만들지 않고 실패한다. 실행 가능한 대상은 single-writer lease 안에서
다시 검증하고 같은 Yo Session identity를 복원하며, 최신 durable Anchor 하나만
재개한다. 이전 Anchor로 fallback하지 않는다. remote storage, Request Audit
persistence, database나 compression backend, durable transport는 이 조합 밖에 남는다.

## 일시정지와 재개

`Ctrl+Z`는 application Session을 닫지 않고 터미널 소유권만 닫는다.

```text
Ctrl+Z press
    ↓
guard가 터미널을 복원한 뒤 TUI가 SuspendRequested 반환
    ↓
TerminationCoordinator가 활성 cleanup lease를 최종 확정
    ├── 종료 선택됨: 살아 있는 agent를 정리하고 해당 signal 재생
    └── 종료 없음: Idle로 반환
          ↓
yo-cli가 기본 SIGTSTP 동작을 적용하고 프로세스 정지
          ↓ SIGCONT
물려받은 SIGTSTP 상태 복원
          ↓
새 활성 lease와 터미널 소유 세대 시작
```

프로세스가 정지한 동안 `TuiSession`과 같은 agent 연결은 살아 있다.
터미널 input, raw mode, presenter, viewport 소유권, frame 이력은 남기지
않는다. 재개된 세대는 이 자원을 다시 획득하고 첫 화면 전체를 그린다.
보존된 appearance snapshot과 revision도 재진입 뒤 유지된다. 각 세대의 첫
redraw는 측정 전에 그 snapshot을 pin하고 완성된 `Surface`까지 그대로
운반한다. `process/job_control.rs`는 기본 `SIGTSTP` action을 임시로
설치하고, 재개된 뒤 물려받았던 action과 mask를 복원한다. process host는
resume 때 glyph profile을 다시 만들거나 선택하지 않는다.

`with_active_resource`가 종료 signal 없이 cleanup lease를 최종 확정한
뒤에만 프로세스를 일시정지할 수 있다. 이 경계에서 설정된 종료 signal이
도착하면 resource-cleanup callback이 보존된 agent를 정리하며, 일시정지
대신 바로 그 signal이 우선한다.

계약: [터미널 job-control 일시정지와 재개](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.job-control-suspend-resume.md)

## 종료와 정리

사용자 종료와 프로세스 종료는 프로세스 호스트가 signal 정책을 적용하기
전까지 같은 정리 경로를 사용한다.

```text
exit gesture 또는 typed TerminationEvent
    ↓
yo-tui loop가 종료 이유를 반환
    ↓
terminal guard가 표시 상태를 복원
    ↓
yo_tui::run_session_with_mode가 Exited 반환
    ↓
AgentSession::shutdown
  worker 중지 → backend 중지 → 활성 semantic work 종료
    ↓
TerminationCoordinator가 활성 resource lease를 마무리
    ├── 사용자 종료: yo-cli로 반환
    └── signal: 선택된 signal의 기본 disposition 적용
          ↓
일반 반환에서는 yo-cli가 설치했던 signal 상태를 복원
```

application Session이 끝날 때 TUI는 `UserRequested` 또는
`TerminationRequested`만 보고한다. 어떤 signal인지 식별하거나
프로세스의 마지막 동작을 선택하지 않는다. guard가 있는 runner는 어떤
결과든 반환하기 전에 터미널 상태를 복원한다. `run_agent_generation`은
터미널 연산이 실패했더라도 agent shutdown을 호출하고, 필요하면 두 실패를
모두 보고한다.

일반 반환에서는
[`TerminationCoordinator::shutdown`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/termination/mod.rs)이
설치했던 signal disposition과 설치 thread의 원래 mask를 복원한다.
종료 signal이 선택되면 `with_active_resource`는 TUI 정리 경로가
반환될 때까지 기다리고, 필요한 경우 보존된 agent도 정리한다. 그 뒤
signal을 일반 애플리케이션 오류로 바꾸지 않고 해당 signal의 기본
disposition을 적용한다.

## 첫 소유자 찾기

처음 실패한 경계에 가장 가까운 오류 문맥부터 따라간다.

| 보이는 문맥 | 시작 지점 |
|---|---|
| `starting Codex` | transport 시작을 포함한 `yo-core/backend/codex` |
| `creating the agent Session` | `yo-core/agent_session` 시작과 worker handshake |
| `terminal session` | `yo-tui/runner`와 터미널 mode 정리 |
| `agent cleanup` | `yo-core/agent_session::shutdown`, 그다음 runtime/backend 정리 |
| `process termination session` 또는 `process termination cleanup` | `yo-cli/process/termination` |
| `suspending the process` | `yo-cli/process/job_control` |

뒤이어 발생한 정리 실패를 버리지 않는다. 현재 최상위 경로는 서로
독립적인 정리 경계를 모두 시도하고 각 오류 문맥을 함께 보고한다.

## 계약 소유자

- [command와 event 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md)
- [Session, Turn, Activity 의미](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.session-turn-activity.md)
- [활성 Turn input](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.active-turn-input.md)
- [Session Journal](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.session-journal.md)
- [Session Repository](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.storage.session-repository.md)
- [Codex app-server adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md)
- [typed TUI 흐름](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.typed-flow.md)
- [표시 mode 선택](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.mode-selection.md)
- [터미널 생명주기 복원](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.lifecycle-restoration.md)
- [프로세스 종료 coordinator](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.process-termination-coordinator.md)
- [터미널 job-control 일시정지와 재개](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.job-control-suspend-resume.md)

실패한 경계를 찾았다면 [검증](../validation/)에서 수정 결과를
확인할 증거를 선택한다.
