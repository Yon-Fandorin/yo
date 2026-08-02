# 코드 지도

구체적인 타입이나 함수를 검색하기 전에 이 지도로 변경의 소유 경계를
고른다. 모든 소스 파일을 나열하지 않고 안정적인 책임과 진입점을
설명한다.

## 크레이트를 가로지르는 경로

프로세스 호스트는 provider와 frontend를 만든 뒤, frontend에 독립적인
세션 의미를 통해 둘을 연결한다.

```text
yo-cli main
├── yo-core CodexBackend
├── yo-cli TuiAgentConnection
└── yo-tui runner
        ↕ AgentConnection
    yo-core AgentSession
        ↕ bounded command lane + 합쳐지는 Journal 변경 lane
    worker-owned AgentRuntime
        ├── AgentEngine
        └── AgentBackend
```

현재 구현 경계는 다음과 같다.

- 프로세스 정책과 정리 순서는 `yo-cli`에 있다.
- Session, Turn, Activity, command, event의 의미는 `yo-core`에 있다.
- 터미널 상호작용과 화면 표시는 `yo-tui`에 있다.

승인된 책임과 향후 GUI에 대한 제약은
[프런트엔드 독립 코어 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.core.frontend-independent-boundary.md),
[모듈 및 호스트 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.architecture.module-boundaries.md),
[UI 전용 크레이트 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.crate.ui-only-boundary.md)가
계속 소유한다.

## yo-cli: 프로세스 호스트

| 경계 | 소유하는 책임 | 소유하지 않는 책임 |
|---|---|---|
| [`src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) | 인자 해석, 터미널 획득 전 표시 방식과 glyph profile 선택, 작업 디렉터리 확보, provider 시작, 터미널 세대 재진입, 최상위 정리 결과 취합 | 에이전트 의미나 터미널 렌더링 |
| [`src/agent/mod.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/agent/mod.rs) | 구체적인 local Transcript cursor를 포함해 `yo-core::AgentSession`을 TUI의 `AgentConnection` 포트에 맞게 연결 | provider 프로토콜 변환 또는 시기상조인 local·remote reader trait |
| [`src/command.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command.rs), [`src/session.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/session.rs), [`src/config.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/config.rs) | live startup과 `yo session` 목록/직접 읽기 문법 분리, 현재 workspace 또는 `--all` 선택, Session 목록 날짜 설정, TTY 폭에 따른 열 우선순위, 저장 Chat/Transcript의 stdout/stderr routing | physical Session decode, semantic recovery, 범용 반응형 plain-text layout, 실행 가능한 continuation |
| [`src/storage.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/storage.rs) | 사용자별 플랫폼 상태 루트와 별도 override 가능한 Session repository 루트 선택, local writer와 생성하지 않는 reader 경로를 분리해 조합. writer startup은 Host identity를 확립하지만 read-only command는 기존 identity와 repository만 관찰 | Host identity의 의미나 physical Session record 의미 |
| [`src/process/job_control.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/job_control.rs) | 기본 `SIGTSTP` 동작 적용, 프로세스 일시정지, `SIGCONT` 뒤 물려받은 signal 상태 복원을 하나의 transaction으로 처리 | TUI 상태나 터미널 복원 |
| [`src/process/termination`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/termination/mod.rs) | Unix signal 설치·관찰·복원과 마지막 처리 | 터미널 상태 복원 |

프로세스 시작이나 종료가 실패하면 `main.rs`에서 오류 문맥에 표시된
소유자로 이동한다. signal coordinator는 `yo-cli`에 있다. TUI는 어떤
signal인지 알 필요가 없는 typed `TerminationEvent`만 받는다.
[프로세스 종료 coordinator](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.process-termination-coordinator.md)가
이 계약을 소유한다.

## yo-core: 에이전트 의미

[`src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/lib.rs)는
공개 facade다. GUI, TUI, provider adapter가 함께 쓸 새 기능이 필요할 때
여기서 시작한다.

| 모듈 | 소유하는 책임 | 다음 탐색 지점 |
|---|---|---|
| [`command.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/command.rs), [`event.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/event.rs), [`session.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/session.rs) | provider에 독립적인 command, 관찰 가능한 event와 outcome, typed identity, versioned `SessionDescriptor`. 릴리스 기준 Session identity는 저장소와 독립적인 UUIDv7이며 그 내부 시각은 descriptor 시작 시각과 일치한다 | 허용되는 상태 전이는 `engine` |
| [`host`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/host/mod.rs) | 불투명한 random UUIDv4 `WorkspaceHostId`, 원자적으로 만들고 권한을 제한한 local 사용자별 identity 파일, 생성 Host가 만든 lossless canonical workspace path 값 | Host identity가 일치할 때의 workspace 비교와 remote Host transport |
| [`engine`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/engine/mod.rs) | 결정론적인 Session, Turn, Activity, request 상태 전이 | 전이가 provider 경계도 지난다면 `runtime` |
| [`journal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/journal/mod.rs) | commit된 command와 semantic event를 하나의 순서로 보존하는 live Projection, sequence 기반의 제한된 Transcript 읽기, 동기식 durable publication, typed gap 상태, revision을 인식하는 크기 제한 `MessageSegment` 구성, recovery 검증 | capture 지점은 `runtime`, physical durability는 `session_repository` |
| [`session_repository`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/session_repository/mod.rs) | 저장 형식에 독립적인 append·replay·저장 Session 탐색/읽기 포트, snapshot 복구 gate, typed storage pressure, 첫 single-writer versioned-JSONL 로컬 구현. `LocalSessionReader`는 writer lease나 변경 없이 기존 저장소를 열고 Session마다 검증한 tail envelope 하나로 목록을 만들며, 존재 여부를 포함해 한 시점으로 고정된 history를 읽는다. `read_stored_session`은 파일 없음과 파일은 있지만 complete envelope가 없음을 구분하고, physical envelope와 semantic recovery를 검증하고 저장 전용 message segment를 semantic snapshot으로 합치며, message-recovery interruption, discovery 불일치, `v1`만으로는 종료 뒤 durability continuity를 관찰할 수 없다는 사실을 typed history metadata로 보존한다. local `reader`와 `file` 모듈은 관찰과 변경 책임을 나눈다. `JournalRepository`는 candidate를 durable semantic prefix와 검증하고 semantic commit 하나를 physical append 하나와 조합 | 실행 가능한 continuation, remote storage나 transport, Request Audit persistence, database나 compression 대안 |
| [`runtime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs) | backend 수락, semantic commit, Journal capture 순서, backend 관찰 결과 변환, 실패 시 활성 작업 종료 | provider port는 `backend/contract.rs` |
| [`agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | frontend를 막지 않는 접근, 크기가 제한된 command lane, 용량 1의 Journal 변경 알림, worker 소유권, 시작 취소, 종료 조율 | worker가 소유한 의미 처리는 `runtime` |
| [`backend/contract.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/contract.rs) | provider capability, command, semantic event, polling, 취소, failure kind, 명시적 정리 | 구체적인 adapter |
| [`backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `codex app-server` 생명주기, JSON transport와 protocol 분류, provider ID 연결, core event로 변환 | 새 provider 동작을 노출하기 전 `backend/contract.rs` |

`AgentBackend`가 현재 provider 교체 지점이며 Codex wire value는
`backend/codex` 아래에 있다.
[command와 event 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md)와
[Codex app-server adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md)가
각 동작 제약을 소유한다.
[Session Journal](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.session-journal.md)과
[Session Repository](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.storage.session-repository.md)가
durable replay와 storage 계약을 소유한다. 구현된 조합은 semantic commit을
encoding하고 message content를 크기가 제한된 `MessageSegment` record로
만든다. 권위 있는 replacement snapshot은 이미 durable한 segment를 바꾸지
않고 새 immutable message revision을 시작한다. non-text 순서 경계보다 먼저
pending text를 강제 저장하며, 같은 writer의 live gap snapshot과 reopen
recovery를 구분한다. 첫 릴리스 전 codec은 semantic commit `v1`만 쓰고 읽으며,
개발 중간 형식을 호환성 약속으로 남기지 않는다. frontend에 보이는 `JournalSequence`는 semantic cutoff만
나타내고 첫 descriptor와 normalized segment record는 비공개 replay 좌표로 정렬한다.
descriptor는 replay sequence 1을 쓰지만 semantic `JournalSequence`를 만들지 않으므로
descriptor-only 첫 physical envelope에는 semantic cutoff가 없다.
`JournalRepository`는 JSONL을 append마다 다시 읽지 않고 복구한 상태에 새
suffix만 증분 검증한 뒤 local repository에 연결한다. 각 physical discovery
summary의 descriptor도 이 검증된 semantic prefix에서 만들고, local writer는 같은
checksummed append 직전에 `updated_unix_millis`를 추가한다. 자기
storage-pressure 실패를 직접 관찰한 live writer는 snapshot 하나로 보존한
prefix를 완성할 수 있지만, reopen 뒤의 replacement snapshot은 그 prefix와
필요한 recovery seal도 보존해야 한다.

이제 실행 중인 `AgentSession` worker가 `JournalRepository` 호출 경로를
소유한다. CLI는 durable한 local Workspace Host identity를 확립하고 local
repository를 연 뒤, UTF-8을 lossy하게 바꾸지 않은 채 workspace를 canonicalize한다.
UUIDv7 시계 읽기 한 번으로 `SessionDescriptor`를 만든다.
`YO_SESSION_REPOSITORY`로 Session record 위치를 옮겨도 Host identity는 바뀌지
않는다. worker는 backend `CreateSession` 전에 descriptor를 첫 Journal envelope로
시도한다. 이 append를 할 수 없으면 descriptor와 semantic prefix를 complete
snapshot 하나로 함께 저장할 때까지 뒤의 activity도 memory-only로 남는다.
저장 형식에 독립적인 `StoredSessionReader`는 이제 제한된 discovery, typed
continuation eligibility, durable history replay를 제공한다. 미지원 schema는
`unknown`으로 계속 살펴볼 수 있고, 격리 상태와 Anchor가 없는 지원 record는
`unavailable`이다. `yo session`은 현재 workspace 목록, `--all` 탐색, full UUID
직접 지정의 읽기 전용 Chat/Transcript 출력에 이 포트를 사용하지만 어떤 항목도 실행 가능하게 만들지는 않는다.

`yo-core`는 아직 `0.0.0` 내부 Pilot API다. 이 Slice는 persistent startup이 bare
`SessionId` 대신 완전한 `SessionDescriptor`를 요구하도록 의도적으로 바꾸고,
descriptor만 durable한 동안 `JournalDurability::Durable`에 semantic cutoff가 없을 수
있게 한다. `SessionId`는 이제 UUIDv7만 허용하므로 `as_uuid`는 UUID를 직접 반환하고,
허용된 identity에 대한 `SessionDescriptor::for_session`은 실패하지 않는다. 이는
compatibility shim을 추가하는 대신 계약을 바로잡는 의도적인
source-breaking 전환이다. public API가 확정되기 전까지 caller는 이 repository와 함께
migration해야 한다.

worker는 commit된 semantic 결과를 공개하기
전에 durable record를 쓴다.
streaming text는 크기·시간·ordering·종료 경계가 durable segment나 empty
revision의 `MessageReset`을 강제하기 전까지 process-local live revision으로 남는다.
명확히 append를 거부한 capacity나 storage-pressure가 발생하면 Session은 memory에서
계속 실행하면서 typed gap을 유지하고, 열린 message가 실제 terminal seal을 받은 뒤
complete snapshot이 성공하면 durability를 복구한다. 결과가 모호한 repository 실패는
현재 writer가 자동 재시도하지 않는 integrity gap으로 바뀔 수 있다. 공유
Transcript observation stream은 gap과 복구 전환을 그 영향을 받는 semantic record보다
먼저 순서대로 보존한다. CLI 연결은 이 typed observation을 전달하고 TUI 상태는 시각적 표현 정책을 선택하지
않은 채 최신 값을 보존한다. 저장된 Session 탐색과 읽기 전용 history는 연결했지만
resume은 아직 연결하지 않았으므로 durability만으로 현재 CLI를 재개할 수는 없다.
local repository는 root 전체에 single-writer lock을 두므로 두 번째 live `yo`
process가 같은 default root를 열 수 없다. multi-process writer coordination은 현재
구현 범위가 아니다.

backend adapter가 semantic `ModelWork`로 승인한 관찰 가능한 plan이나 reasoning
summary도 같은 durable message 경로를 따른다. yo가 받지 않은 숨겨진 model
reasoning과 승인하지 않은 backend-specific Request Audit payload는 이 semantic
경로 밖에 남는다.
remote storage, Request Audit persistence, database나 compression 선택, durable
transport는 이 경로의 범위 밖이다. `StoredSessionReader`는 Session 전용 read port이며
local·remote 공통 구현을 미리 주장하지 않는다. 실제 remote reader가 생길 때만
transport 공유 구조를 추출한다.

## yo-tui: 터미널 frontend

[`src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/lib.rs)는
실행 중인 session을 위한 runner facade를 좁게 유지하면서, 완성된 화면
상태와 터미널 연산, HTML Projection에 재사용할 타입을 공개한다.

| 모듈 | 소유하는 책임 | 다음 탐색 지점 |
|---|---|---|
| [`runner`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/mod.rs) | 실행 중인 session의 공개 facade, 터미널을 단독 소유하는 loop, input·event 조율, 마지막 정리 결과 보고, 터미널에 독립적인 저장 Chat/Transcript Projection | UI의 의미 상태 전이는 `runner/state.rs`, 저장 출력은 `runner/archival.rs`, 실행 중 조율은 `runner/unix.rs` |
| [`appearance`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/appearance/mod.rs) | session이 소유하는 불변 appearance snapshot, 단조 증가 revision, resolved style role, 공개된 built-in Rich/ASCII glyph profile 선택 | profile을 받는 생성과 소유권은 `runner/session.rs`, frame pinning은 `runner/state.rs` |
| [`plain`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/plain/mod.rs) | terminal cell 폭에 맞춰 고정 열을 유지하고 짧은 접힌 label/value pair는 폭 안에서 flow로 채우며 block 값은 독립된 한 줄을 사용하되 필요할 때만 label과 값을 분리하고, grapheme을 자르지 않고 개행한 뒤 필요하면 세로 card layout으로 전환하는 plain 목록 | 열의 의미와 접기 우선순위 또는 continuation hint, 설정, stdout TTY 정책, terminal 소유권 |
| [`input`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/input/mod.rs) | 해석이 끝난 semantic key event, 편집 buffer, 설정 가능한 binding, 종료 gesture, prompt 편집, typed view-switch 표시 정책 | 보이는 cursor 배치는 `prompt`, 선택한 Projection은 `runner/view.rs` |
| [`transcript`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/transcript/mod.rs) | 순서가 있는 사용자·에이전트 item, streaming revision, 대화 기록 layout, scroll 상태 | prompt와 조합하는 일은 `shell` |
| [`runner/view.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/view.rs) | 읽기 전용 Chat·Transcript·Request 선택, 전체 Journal record Projection, 정확한 Request anchor와 typed unavailable 사유, mode별 context·viewport 상태, compact mode chrome | Journal 관찰과 editor dispatch는 `runner/state.rs`, 공통 layout·scroll은 `transcript` |
| [`prompt`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/prompt/mod.rs) | editor 내용과 cursor가 보이는 상태를 측정하고 그리기 | 편집 의미는 `input/editor` |
| [`shell`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/shell/mod.rs), [`layout`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/layout/mod.rs) | 대화 기록과 prompt 영역 배분, 완성된 frame 하나 조합, cursor 위치 보고 | cell 쓰기는 `surface` |
| [`surface`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/surface/mod.rs), [`text`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/text/mod.rs) | adapter에 독립적인 cell 상태, Unicode grapheme과 너비, 경계가 있는 view, diff span, 터미널에 독립적인 text flow | Projection은 `terminal` 또는 `html` |
| [`terminal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mod.rs) | typed terminal operation과 ANSI encoding | 표시 정책은 `terminal/mode`, Unix effect는 `terminal/backend` |
| [`terminal/mode`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mode/mod.rs), [`terminal/backend`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/backend/mod.rs) | 공유 transactional restoration, Inline·Fullscreen presenter, panic routing, crate-private platform boundary | 프로세스 signal 정책이 바뀔 때만 `yo-cli/process` |
| [`html`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/html/mod.rs) | 완성된 `Surface` 상태를 결정론적으로 브라우저에 Projection | 터미널과 브라우저 출력이 다르면 `surface` |

`runner::TuiSession`은 한 번의 터미널 소유 기간보다 오래 유지할 수 있는
간결한 Chat 대화 기록, editor, 대기 중인 요청, 세 observability view,
backpressure로 전달되지 못한 agent dispatch 상태와 하나의 committed
appearance snapshot을 소유한다. Chat은 편집 가능한 기본 mode다. 현재
Chat, Transcript, Request의 typed 표시 정책 binding은 각각 F1, F2, F3이며
Projection 상태는 이 key 선택을 소유하지 않는다. Transcript는 같은 읽기
전용 Journal 경로에서 받은 모든 committed command와 event를 그린다.
Request는 Chat이나 Transcript에서 선택한 정확한 context를 유지하며 인접
record를 검색하는 대신 `no_associated_request` 또는
`request_audit_detail_unavailable`을 보고한다. Transcript와 Request는
prompt를 대체하며 editor submission을 dispatch하지 않고 input을 소비한다.
각 view는 자체 context와 viewport 상태를 보존한다.

현재 실행 중인 `AgentConnection`은 순서 있는 Transcript record와 별도의
durability transition을 제공한다. adapter는 아직 각 record의
`JournalSequence`를 버리고 Request Audit detail도 제공하지 않으므로, view는
누락된 값을 추론하지 않고 이 제한을 드러낸다. 이 view layer는 Request
Audit을 persist하거나 또 다른 Journal owner를 만들지 않으며, worker가
소유한 repository 연결은 frontend 경계 아래에 남는다.

각 redraw는 측정 전에 appearance revision을 pin하고, paint와 완성된
`Surface`까지 같은 resolved snapshot을 사용한다. plain session output도
같은 session-owned 설정을 pin한다. `TuiSession::new`는 호환 기본값인 Rich
glyph를 선택하고, `TuiSession::with_glyph_profile`은 mutable theme state를
노출하지 않은 채 process host가 built-in ASCII profile을 선택하게 한다.
보존된 상태에는 해당 agent Session의 식별자가 있으므로 재진입할 때도 같은
agent 연결을 유지한다.
`runner/unix.rs`는 매 터미널 소유 기간마다 터미널 입력, presenter,
viewport 소유권, frame 이력을 새로 얻으며, 이 자원들은 `TuiSession`으로
옮기지 않는다. 정리가 성공한 `Ctrl+Z`는 이 세대 전용 자원을 모두 복원한
뒤에만 `TerminalOutcome::SuspendRequested`를 반환한다.

Appearance 계약:
[session publication](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.session-publication.md),
[frame 일관성](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.frame-consistency.md),
[glyph profile](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.glyph-profiles.md),
그리고
[resolved cell style](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.resolved-style.md).

`surface`는 공통으로 완성된 상태다. 터미널과 HTML Projection은 이를
각자 소비하며, 어느 쪽도 다른 쪽의 layout 의미를 정의하지 않는다.

## 저장소 개발 도구

[`tools/xtask`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/xtask/src/lib.rs)는
`yo` 제품이 아니라 이 저장소를 관리하는 구조화된 검사를 소유한다. 변경
경로와 commit trailer를 분류하여 Slice 검수와 Developer Docs 영향을
확인하고, Rust test에 이해 가능한 인접 설명이 있는지도 검사한다.
`hk.pkl`은 언제 실행할지를 결정하고, `xtask`는 규칙의 구현과 test를
담당한다. Methexis와 Librarian은 각자의 지식 domain 책임을 유지하며,
단순한 외부 명령 조합은 `hk` 또는 작은 검증 script에 남는다.

소유자를 골랐다면 [검증](../validation/)을 변경 경계에서 증거로
이어지는 단일 지도로 사용한다. 실제 터미널 동작이 관련되면
[터미널 환경 매트릭스](../validation/terminal-matrix.md)를 따른다.
[Slice](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#slice-contract)를
닫기 전에는 변경이 실제로 지나간 경계에 대해서만 검사를 넓힌다.
