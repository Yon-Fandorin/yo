---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.session.history-discovery
revision: sha256:8cec5ec20eb225af2f5b222d13421024d03cc16bce814df26211bd9704fcdbd5
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:b09ad3072788625b55fb7d159fcd596cb013bc8c0cacce21d7953762fdd02a2a
---
# Korean Review Projection

## Translation

# 저장된 Session 탐색과 읽기 전용 보관 view

## 선언

Session 탐색은 실행 재개 없이 사용할 수 있어야 합니다. 새 Session은 전체 UUIDv7 Session ID, workspace-host identity, host가 정규화한 workspace path, start time을 담은 최소 versioned descriptor를 이후 Session activity보다 먼저 영속해야 합니다. Workspace를 소유한 host가 정규화를 정의하며 client는 remote path에 local path 규칙을 적용하지 않고 host identity와 그 host가 정규화한 path를 비교해야 합니다. Descriptor는 기존 Session Repository 생명주기 아래의 의미적 Session Journal data이지 filesystem index나 두 번째 Session 권위가 아닙니다. 미래 compatibility 계약은 descriptor가 없는 format을 metadata가 명시적으로 unknown인 읽기 가능 format으로 허용할 수 있지만, 현재 compatibility baseline이 거부하는 개발 format은 읽을 수 있는 Session이 아닙니다.

지원되는 모든 물리 Session record는 같은 physical commit 안에 제한된 discovery summary를 가져야 합니다. Summary에는 전체 Session descriptor, writer가 지정한 `updated_unix_millis`, 선택적 binding epoch, 선택적 최신 유효 Continuation Anchor `JournalSequence`가 들어가야 합니다. Writer는 append 직전에 timestamp를 지정해야 하며 그 값은 checksummed envelope와 함께 있을 때만 영속됩니다. Descriptor, binding epoch, anchor reference는 committed Journal prefix에서 재계산할 수 있어야 합니다. Summary는 두 번째 append나 mutable side index로 기록되어서는 안 되고 Journal 권위를 대체해서도 안 됩니다. Reader는 전체 log를 단순히 목록화하려고 scan하지 않고, 제한된 tail read로 마지막 완전한 envelope를 찾아 검증해 현재 discovery metadata를 얻어야 합니다. 여기서 bounded는 discovery가 tail envelope로 제한된다는 뜻이며 유효 envelope 하나의 byte 크기가 고정이라는 보장이 아닙니다. Summary는 discovery hint이지 의미적 증명이 아닙니다. 실행 continuation은 Journal에서 참조된 Anchor를 검증해야 합니다. Summary와 Journal의 불일치를 발견하면 Journal을 권위로 삼고 불일치를 명시적으로 보고하며 writer 소유 복구가 일관된 envelope를 게시할 때까지 continuation eligibility를 `unavailable`로 분류해야 합니다.

`yo session`과 `yo --resume` picker는 기록된 workspace-host identity와 정규화 path가 현재 workspace와 같은 Session을 기본으로 선택해야 합니다. 명시적으로 지원되는 descriptor 없는 Session은 workspace가 unknown이며 모든 workspace의 기본 목록에 넣지 않고 `--all`과 전체 UUID 직접 선택으로 접근해야 합니다. `--all`은 다른 workspace와 unknown workspace도 포함하며 ordinary list 중 이 mode만 workspace column을 표시해야 합니다. `--details`는 선택 집합을 바꾸지 않고 record schema version, continuation eligibility, 전체 기록 path를 추가해야 합니다. `UPDATED`는 filesystem modification time이나 휘발성 화면 activity가 아니라 마지막 유효 durable envelope에 기록된 timestamp여야 합니다. 결과는 이 timestamp, 기록된 start time, 안정적인 Session identity 순서로 결정론적으로 정렬해야 하며 사용할 수 없는 legacy 값은 눈에 보이게 unknown으로 남겨야 합니다.

Continuation eligibility는 backend의 현재 도달 가능성을 약속하는 것이 아니라 durable evidence입니다. Quarantine이나 발견된 summary 불일치가 모든 summary 값보다 우선합니다. 그 외에는 지원되는 record schema의 제한된 summary가 `JournalSequence`로 유효한 Continuation Anchor를 가리킬 때만 `eligible`, 지원되는 record가 유효 anchor 부재 또는 committed prefix quarantine을 증명하면 `unavailable`, 오래되거나 미지원 format이라 제한된 evidence를 제공할 수 없으면 `unknown`입니다. 실제 native resume, replay 지원, transport reachability, lossy-handoff availability는 실행 continuation 시점에만 평가합니다. Picker는 `unavailable` 항목을 흐리게 표시하고 선택을 막아야 합니다. `unknown`은 살펴볼 수 있지만 resumable로 표시하지 않고 continuation 시점 평가를 요구해야 합니다. `unavailable` Session을 `yo --resume SESSION_ID`로 직접 지정하면 durable history를 읽기 전용으로 열고 Continuation Anchor 계약이 허용하는 명시적으로 확인된 fork만 제안할 수 있습니다.

전체 UUID는 `yo session SESSION_ID`, `yo usage SESSION_ID`, `yo --resume SESSION_ID`가 받는 공개 Session identifier입니다. Continuation option 없는 `yo`는 새 Session을 시작합니다. `yo --continue`는 현재 workspace에서 가장 최근에 갱신된 `eligible` Session을 선택해야 하며 후보가 없으면 Session을 만들지 않고 실패해야 합니다.

저장된 Session view는 durable prefix와 process-local tail을 합치는 live frontend view가 아니라 Session Repository의 보관 투영입니다. Local read-only CLI grammar는 다음 형식으로만 이루어져야 합니다.

- 목록은 `yo session [--all] [--details]`;
- Chat을 기본값으로 하는 보관 view는 `yo session SESSION_ID [--view chat|transcript|request] [--ascii]`;
- Transcript만 양수 N을 받는 `--limit N`과 `--content none|preview|full`을 추가로 허용할 수 있음;
- 독립적인 Session Usage report는 `yo usage SESSION_ID [--ascii]`.

`--ascii`는 glyph 선택만 바꿔야 합니다. `--limit`과 명시적으로 지정된 모든 `--content`는 `--content full`을 포함하여 Chat과 Request에서 거부해야 합니다. Usage를 포함한 다른 Session view는 usage error여야 하며 Usage는 Session-view enum이나 route로 표현되어서는 안 됩니다. 두 직접 읽기 명령은 pipe 가능한 plain output을 stdout에, diagnostic을 stderr에 출력해야 합니다. Session이 없거나 읽을 수 없는 경우와 모든 치명적 projection error는 타입화된 local diagnostic을 보존하고 부분 stdout을 출력해서는 안 됩니다.

두 직접 읽기 명령은 local non-creating Session reader를 사용해 durable semantic Journal만 대상으로 하나의 읽기 전용 point-in-time projection을 capture해야 합니다. Repository가 writer lease를 얻지 않고 active writer를 독립적으로 확인할 수 있으면 pending marker는 in-flight append로 취급하여 marker 전 마지막 검증 envelope에서 reader를 멈추고 durable point-in-time snapshot을 보고해야 합니다. Active writer를 확인할 수 없으면 남은 marker는 Session을 quarantine해야 합니다. Live writer를 발견하지 못한 경우 availability를 보수적으로 quarantine할 수 있지만 guarded byte를 허용하거나 snapshot correctness를 약화해서는 안 됩니다. 어느 명령도 이후 append를 subscribe하거나 Agent Backend를 시작하거나 Session을 할당·resume하거나 repository writer lease를 얻거나 storage를 만들거나 torn tail을 repair하거나 repository state를 바꿔서는 안 됩니다. 두 명령은 불완전한 final line을 uncommitted로 무시하고 pending-marker quarantine과 완전한 line corruption을 존중하며 interrupted, incomplete, durability-gap 상태를 연속적으로 완료된 history처럼 보이지 않고 명시적으로 보존해야 합니다.

Storage-neutral read boundary는 JSONL path나 write operation을 CLI에 노출하지 않고 목록과 replay를 제공해야 합니다. 이 boundary는 같은 Session Repository ownership boundary가 구현하는 read port이지 generic append-log abstraction, 별도 Request Audit repository, 실제 remote consumer보다 앞선 공통 local·remote reader interface가 아닙니다. 실행 resume, backend binding persistence, native backend reconnection, semantic replay, lossy handoff, deliberate fork 생성은 이 capability 범위 밖이며 계속 Continuation Anchor 계약의 별도 통제를 받아야 합니다.

## 이유

상용 코딩 에이전트는 보통 최근 Session 선택, Session list, identity 직접 선택을 제공합니다. 제한된 discovery와 read-only history를 resume과 분리하면 workspace identity를 추측하거나 backend work를 시작하거나 불완전한 durable suffix를 안전한 입력으로 오해하지 않으면서 모든 continuation fallback에 필요한 inspection primitive를 제공할 수 있습니다. 기존 durable envelope 안의 summary는 두 번째 writer, 권위, recovery path를 만들지 않고 discovery를 제한합니다. 닫힌 local command grammar는 보관 observability와 Usage report를 각각 접근 가능하게 유지하면서 어느 path도 실행 continuation으로 열지 않습니다.
