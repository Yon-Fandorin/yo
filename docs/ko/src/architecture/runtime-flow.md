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
`TERM`이나 `NO_COLOR`를 검사하지 않는다.

계약:
[session publication](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.session-publication.md),
[frame 일관성](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.frame-consistency.md),
[glyph profile](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.glyph-profiles.md),
그리고
[resolved cell style](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.resolved-style.md).

## 활성 Turn 하나

제출된 prompt는 다음 경로를 지난다.

```text
terminal input
    ↓
TuiState::handle
    ↓ AgentIntent::Submit
TuiAgentConnection
    ↓
AgentSession admission and bounded command lane
    ↓
AgentWorker
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
   제출된 prompt를 비우고 frontend에 독립적인 `AgentIntent::Submit`을
   만든다. 이 시점에는 입력을 확정된 이력으로 표시하지 않는다.
2. [`TuiAgentConnection`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/agent/mod.rs)은
   좁은 local adapter다. dispatch와 retry를 전달하고, 하나로 합쳐진
   Session 변경 알림을 `TranscriptReader`의 크기가 제한된 suffix 읽기로
   바꿔 순서가 보장된 record를 TUI에 제공한다. Session이나 provider
   의미는 소유하지 않는다.
3. [`agent_session/admission.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/admission.rs)는
   Submit을 `StartTurn` 또는 `SteerTurn`으로 결정한다. state lock이
   사용 중이거나 크기가 제한된 lane이 가득 찼다면, TUI loop가 다시
   시도할 수 있도록 내부가 드러나지 않는 pending command를 반환한다.
4. [`AgentWorker`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/worker.rs)만
   runtime을 실행하고 polling할 수 있다. 터미널을 소유한 thread는
   provider I/O를 기다리지 않는다.
5. [`AgentRuntime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs)은
   command 검증, backend 수락, semantic commit, Journal publication 순서를
   보장한다. worker가 소유한 durable writer는 text update를 크기가 제한된
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
   별도 product 계약으로 남긴다. 저장된 Session 탐색과 resume도
   아직 현재 runtime 동작이 아니다.
6. [`drain_agent`와 `redraw`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs)는
   이미 확정된 Transcript record를 소비하고 TUI 상태를 갱신한다.
   완성된 `Surface`를 조합해 활성 presenter로 보낸다. `runner/view.rs`는
   같은 record stream에서 Chat, Transcript, Request를 선택한다. Chat의
   사용자 입력은 `StartTurn` 또는 `SteerTurn` command가 이 순서에 나타난
   뒤에만 표시된다.

change lane은 command나 event 내용을 싣지 않으며 용량은 하나다. 따라서
여러 commit이 읽지 않은 알림 하나로 합쳐져도 이력은 사라지지 않는다.
구체적인 local reader가 Journal sequence를 따라 당시 확인한 head까지
계속 읽기 때문이다. backend가 최종 실패해도 adapter는 Journal에 이미
확정된 실패 record를 먼저 모두 공개한 뒤 연결 오류를 보고한다.

Codex JSON과 provider identifier는 backend adapter 밖으로 나오지 않는다.
터미널 input event와 rendering type은 `yo-tui` 밖으로 나오지 않는다.
그 사이를 지나는 command와 event type은 `yo-core`가 소유한다.

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
semantic Journal record
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
macOS는 `$HOME/Library/Application Support/yo/sessions`를 쓴다. read port는
존재하지만 이 조합은 아직 CLI 저장 Session 열기, 실행 가능한 continuation,
remote storage, Request Audit persistence, database나 compression backend,
durable transport를 추가하지 않는다.

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
