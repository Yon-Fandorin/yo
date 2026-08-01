---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.session.history-discovery
revision: sha256:b12b9746a6a35caaf064e3100ffa068fdb675b8f5fb39bc55b6da8ccf1f879a5
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:b35bdab665e321197d14c8993f56f68503256d2dd71bda68323399c6eca84fc7
---
# Korean Review Projection

## Translation

# 저장된 Session 탐색과 읽기 전용 기록

## 결정

Session 탐색은 실행 재개 없이 사용할 수 있습니다. 새 Session은 UUIDv7, workspace-host identity, host-normalized path, start time을 먼저 영속 descriptor로 기록합니다. Host가 path normalization을 정의하고 client는 remote path에 local 규칙을 적용하지 않습니다. Descriptor 없는 옛 Session은 unknown metadata로 읽을 수 있습니다.

모든 durable Journal envelope는 같은 physical commit에 timestamp, binding epoch, valid anchor 존재 여부를 담은 bounded discovery summary를 포함합니다. Summary는 Journal prefix에서 재계산 가능하며 별도 append나 mutable index가 아니고 Journal authority를 대체하지 않습니다. Reader는 bounded tail만 읽습니다. Continuation은 Journal anchor를 다시 검증합니다. Summary와 Journal 불일치가 발견되면 Journal이 우선하고 discrepancy를 알리며 writer recovery 전까지 unavailable로 처리합니다.

기본 목록과 picker는 현재 host와 workspace만 선택합니다. Unknown legacy는 `--all`과 전체 UUID로 접근합니다. `--all`에서만 workspace column을 보여주며 `--details`는 schema, eligibility, full path를 보여줍니다. UPDATED는 마지막 valid durable envelope timestamp입니다.

Eligibility는 reachability 보장이 아니라 durable evidence입니다. Quarantine과 summary discrepancy가 최우선입니다. 그 외 valid anchor가 있으면 eligible, 없다고 증명되면 unavailable, bounded evidence가 없으면 unknown입니다. Picker는 unavailable을 흐리고 선택하지 못하게 합니다. Direct `yo --resume SESSION_ID`는 unavailable history를 읽기 전용으로 열고 계약이 허용하는 explicit fork만 제안할 수 있습니다.

`yo`는 새 Session, `yo --continue`는 현재 workspace의 최신 eligible Session을 재개하며 없으면 새로 만들지 않고 실패합니다.

Stored-session view는 live tail을 합치는 frontend view가 아니라 durable Journal만 보는 archival repository projection입니다. `yo session SESSION_ID`는 기본 Chat을 stdout에, `--view transcript`는 Transcript를 출력하고 diagnostics는 stderr입니다. Active writer를 독립적으로 확인한 상태의 pending marker는 in-flight append이므로 marker 이전 validated envelope에서 snapshot을 멈춥니다. Writer를 확인하지 못한 remaining marker는 quarantine입니다. Writer를 놓치면 보수적으로 quarantine할 수 있지만 guarded bytes를 읽지 않습니다. View는 subscribe, backend 시작, writer lease, storage 생성·복구를 하지 않습니다.

Read port는 같은 Session Repository boundary에 속하며 JSONL write, generic append log, 별도 Request Audit authority, 실제 remote reader 전의 공통 local-remote interface를 노출하지 않습니다.

## 이유

동일 envelope 안의 derived summary로 두 번째 writer나 authority 없이 목록 비용을 bounded하게 만들고, 읽기와 실행 재개를 분리해 불완전 history를 안전한 continuation으로 오인하지 않기 위함입니다.
