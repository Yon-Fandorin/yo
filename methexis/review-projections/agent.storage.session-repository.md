---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.storage.session-repository
revision: sha256:81350456780ff59743ce1c1a924b1dcc3160215ede250f93f816ded647fb76b1
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:d31fc472056736c9236c0e0c2ad49c5a7f6fcd7feb5f018c666593ef9f00f759
---
# Korean Review Projection

## Translation

# Session 저장소와 용량

## 계약

Storage-neutral Session Repository가 파일·SQLite 같은 물리 구조를 frontend 계약으로 노출하지 않고 durable Session record lifecycle을 소유합니다. 의미 Session Journal과 Request Audit은 한 소유 경계 안의 서로 다른 논리 domain이지 독립 Session authority가 아닙니다. 첫 구현은 local이며 remote storage, replication, dual-write, conflict resolution, 별도 Request Audit repository, generic append-log abstraction은 도입하지 않습니다.

Local 구현은 Session마다 하나의 append-only versioned JSON Lines log를 사용하지만 JSONL은 교체 가능한 내부 세부사항입니다. 하나의 semantic commit은 하나의 physical envelope이며 command와 그 events 또는 observation batch의 절반만 영속하면 안 됩니다. `JournalSequence`는 semantic replay 순서, `RepositorySequence`는 physical append 순서이고 서로 추론하지 않습니다.

모든 새 물리 record는 schema, Session ID, `RepositorySequence`, kind, 정확한 payload bytes, format compatibility 계약의 discovery 객체 전체를 명시적인 preimage로 하는 CRC32C를 가집니다. Repository writer가 append 직전에 discovery timestamp를 지정하고, committed semantic prefix에서 descriptor, 선택적인 binding epoch, 선택적인 최신 valid Continuation Anchor `JournalSequence`를 도출해 같은 checksummed envelope에 기록합니다. Timestamp는 envelope와 함께 영속될 때만 durable합니다. 두 번째 checksum이나 별도 summary append를 만들지 않습니다.

같은 repository boundary는 각 Session의 마지막 완전한 envelope를 bounded tail read로 찾아 검증하고 storage-neutral discovery summary를 반환하는 read-only port를 제공합니다. 이 port는 writer lease를 얻거나 storage를 만들거나 record를 repair하거나 JSONL path를 노출하지 않습니다. Active writer와 버려진 pending marker를 구분하기 위한 독립 reader lock은 사용할 수 있지만 writer lease는 아닙니다.

복구는 호환성 계약이 지원하는 record만 checksum 검증 뒤 받아들입니다. Checksum 자신을 포함한 record를 재직렬화해 checksum을 계산하지 않습니다. 완전한 줄을 streaming하고 불완전한 마지막 줄은 uncommitted tail로 보며 다른 완전한 줄의 corruption은 보고합니다. Bounded suffix를 위해 전체 log를 materialize하지 않습니다. Root마다 writer 하나만 허용하고 안정적인 absolute root를 확정합니다. Append마다 durable pending marker를 사용하며 rollback을 확인하지 못하면 이후 reader가 log를 quarantine합니다. 비어 있지 않은 Session reopen 또는 초기 state load failure recovery 뒤에는 incremental record 전에 complete snapshot이 필요합니다.

Journal과 Request correlation은 현재 하나의 physical availability와 capacity ceiling을 공유합니다. Request detail은 redaction-before-admission gate 전까지 volatile합니다. Durable commit은 append와 sync 뒤에만 in-memory Journal에 publish합니다. Persistence failure 뒤 semantic result는 volatile로 공개하고 durable gap을 유지하며 rollback이라고 보고하지 않습니다. Capacity나 storage failure 때 기존 record는 그대로 두고 Session은 memory에서 계속되며 frontend에 typed persistent pressure notification을 보냅니다. Capacity가 돌아오면 complete snapshot 뒤에만 incremental persistence를 재개합니다.

Local 저장소는 기본 활성화하고 current-user permission과 configurable capacity ceiling을 적용하며 자동 age expiry나 Session deletion을 하지 않습니다. 첫 구현은 synchronous single writer입니다. 측정 evidence 없이 background writer, generic transaction, group commit, compression, index, SQLite projection, alternate encoding, Request Audit physical split을 도입하지 않습니다.

## 이유

하나의 checksummed envelope와 분리된 semantic·physical sequence는 partial durability와 corruption을 명확하게 합니다. Discovery read port는 저장 방식과 frontend를 분리하면서도 별도 index authority를 만들지 않습니다. Durable-first publication, explicit pressure, snapshot recovery는 responsive streaming과 honest history를 함께 유지합니다.
