---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.session.history-discovery
revision: sha256:0fda64b5cb1eefbc4b1659c7c056fdfb27b19225935b2015ee8bac5583c3caee
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:eb164830607e7b2f18c8aebe0485c31a0caa862a9d03455eef829c21ce58d87d
---
# Korean Review Projection

## Translation

# 저장된 Session 탐색과 읽기 전용 기록

## 결정

Session 탐색은 실행 재개 없이 사용할 수 있습니다. 새 Session은 전체 UUIDv7 Session ID, workspace-host identity, host-normalized workspace path, start time을 가진 descriptor를 이후 활동보다 먼저 영속합니다. Path normalization은 workspace 소유 host가 정의하며 client가 remote path에 local 규칙을 적용하지 않습니다. Descriptor는 의미 Journal 데이터이지 파일 index나 두 번째 Session 권위가 아닙니다. 현재 거부되는 개발 포맷은 읽을 수 있는 Session이 아닙니다. 미래 호환성 계약이 descriptor 없는 포맷을 명시적으로 지원한다면 workspace metadata를 unknown으로 노출할 수 있습니다.

지원되는 모든 물리 Session 레코드는 같은 physical commit 안에 discovery summary를 가집니다. Summary에는 전체 descriptor, writer가 append 직전에 지정한 `updated_unix_millis`, 선택적인 binding epoch, 선택적인 최신 유효 Continuation Anchor `JournalSequence`가 들어갑니다. Timestamp는 envelope와 함께 영속되는 사실이며 filesystem metadata에서 추론하지 않습니다. 나머지 필드는 committed Journal prefix에서 재계산할 수 있어야 합니다. Summary는 별도 append나 mutable side index가 아니며 Journal 권위를 대체하지 않습니다. Reader는 전체 로그가 아니라 마지막 완전한 envelope를 찾아 검증합니다. 여기서 bounded는 tail envelope 하나로 제한한다는 뜻이지 envelope byte 크기가 고정이라는 뜻은 아닙니다. Summary와 Journal이 다르면 Journal이 우선하고 discrepancy를 알리며 writer recovery 전까지 unavailable로 분류합니다. Continuation은 summary가 가리킨 anchor를 Journal에서 다시 검증합니다.

기본 목록과 picker는 현재 host와 workspace가 일치하는 Session만 선택합니다. 명시적으로 지원되는 descriptor 없는 Session은 `--all`과 전체 UUID로만 접근합니다. `--all`만 workspace column을 보여주고 `--details`는 schema, eligibility, full path를 보여줍니다. UPDATED는 마지막 유효 envelope에 기록된 durable timestamp이며 filesystem mtime이나 휘발성 화면 활동이 아닙니다. 동일 timestamp는 start time과 Session ID로 안정적으로 정렬합니다.

Quarantine과 summary disagreement가 eligibility보다 우선합니다. 그 외 지원 schema의 summary가 유효 anchor `JournalSequence`를 가리키면 eligible, 없음을 증명하면 unavailable, bounded evidence가 없으면 unknown입니다. 이는 backend나 transport reachability 보장이 아닙니다. Picker는 unavailable을 dim 처리하고 선택하지 못하게 하며 unknown은 inspect할 수 있지만 resumable이라고 표시하지 않습니다. Direct UUID의 지원되는 unavailable Session은 durable history를 읽기 전용으로 열고 명시적으로 확인한 fork만 제안할 수 있습니다.

`yo`는 새 Session을 시작하고 `yo --continue`는 현재 workspace의 최신 eligible Session을 선택합니다. 후보가 없으면 새 Session을 만들지 않고 실패합니다. Stored-session view는 live tail을 섞지 않는 durable archival projection입니다. `yo session SESSION_ID`는 Chat을 stdout으로, `--view transcript`는 Transcript를 출력하고 diagnostic은 stderr로 보냅니다. Active writer를 독립적으로 확인한 pending marker는 in-flight append이므로 marker 전 마지막 검증 envelope에서 snapshot을 멈춥니다. Writer를 확인할 수 없는 marker는 quarantine입니다. View는 subscribe, backend 시작, Session 할당·재개, writer lease, storage 생성·repair를 하지 않습니다.

Storage-neutral read port는 같은 Session Repository 경계에 속하고 JSONL path나 쓰기 연산을 CLI에 노출하지 않습니다. Generic append log, 별도 Request Audit authority, 실제 remote reader보다 앞선 공통 local-remote reader 추상화를 만들지 않습니다. 실행 재개는 Continuation Anchor 계약이 별도로 통제합니다.

## 이유

검증되는 같은 envelope 안에 discovery metadata를 넣으면 두 번째 writer나 index authority 없이 목록을 bounded하게 만들 수 있습니다. 탐색·읽기와 실행 재개를 분리하면 불완전한 영속 이력을 안전한 continuation으로 오인하지 않습니다.
