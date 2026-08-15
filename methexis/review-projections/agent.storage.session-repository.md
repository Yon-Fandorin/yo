---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.storage.session-repository
revision: sha256:38952b508ca62a554153e71566ed1b08b9f8108546ea92fec0a0f16bdb0eafe9
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:d2c89fcb49acbfc390c326dd5a9c6adc0ce6f467209c80a0d062b8a226b35f02
---
# Korean Review Projection

## Translation

# Session 저장소와 용량

## 계약

Storage-neutral Session Repository가 파일이나 SQLite 같은 물리 구조를 frontend 계약으로 노출하지 않고 durable Session record lifecycle을 소유합니다. 의미 Session Journal과 Request Audit은 한 소유 경계 안의 서로 다른 논리 domain이지 독립 Session authority가 아닙니다. 첫 구현은 local이며 remote storage, replication, dual-write, conflict resolution, 별도 Request Audit repository, generic append-log abstraction은 도입하지 않습니다.

Local 구현은 Session마다 하나의 append-only versioned JSON Lines log를 사용하지만 JSONL은 교체 가능한 내부 세부사항입니다. 하나의 semantic commit은 하나의 physical envelope이며 command와 그 events 또는 observation batch의 절반만 영속하면 안 됩니다. JournalSequence는 semantic replay 순서, RepositorySequence는 physical append 순서이고 서로 추론하지 않습니다.

모든 새 물리 record는 schema, Session ID, RepositorySequence, kind, 정확한 payload bytes, format compatibility 계약의 discovery 객체 전체를 명시적인 preimage로 하는 versioned CRC32C를 가집니다. Checksum이 이미 든 record를 recursive serialization해 checksum을 계산하면 안 됩니다. Repository writer가 append 직전에 discovery timestamp를 지정하고 committed semantic prefix에서 descriptor, 선택적인 binding epoch, 선택적인 최신 valid Continuation Anchor JournalSequence를 도출해 같은 checksummed envelope에 기록합니다. Timestamp는 envelope와 함께 영속될 때만 durable합니다. 별도 mutable summary authority를 만들지 않습니다.

물리 `yo.session-record/v1` envelope, field grammar, CRC32C preimage, failure behavior는 바뀌지 않습니다. Payload string은 현재 pre-release `yo.semantic-journal-commit/v1` shape 안에서 format-compatibility 계약의 additive provider-private replay item을 encode할 수 있고 schema field와 checksum은 그대로 그 정확한 payload byte를 bind합니다. 현재 semantic reader는 한 Session log에서 이전 replay delta와 확장된 private-item union을 모두 받아들이지만, 이전 reader는 private item을 알 수 없는 variant로 거절합니다. 이후 K3 Turn이 extension을 사용했다는 이유만으로 기존 physical 또는 semantic record를 다시 쓰지 않습니다. Compatibility test는 판별 가능한 이전 replay artifact를 byte-for-byte 보존하고, 뒤의 physical-v1 payload에 private item이 있는 log를 받아들이며, 이전 semantic decoder가 이를 조용히 빼지 않고 실패함을 증명해야 합니다.

같은 repository boundary는 각 Session의 마지막 완전한 envelope를 bounded tail read로 찾아 검증하고 storage-neutral discovery summary를 반환하는 read-only port를 제공합니다. 이 port는 writer lease를 얻거나 storage를 만들거나 record를 repair하거나 JSONL path를 노출하지 않습니다. Active writer와 abandoned pending marker를 구분하기 위한 independent read lock은 사용할 수 있지만 writer lease가 아닙니다.

여러 process가 같은 안정적인 absolute repository root를 동시에 열 수 있고 서로 다른 Session의 writer가 함께 실행될 수 있습니다. 잠금 방식 전환 중에는 모든 신버전 writer-capable repository instance가 기존 root-exclusive writer-lock 파일에 대한 shared compatibility guard를 lifetime 동안 유지합니다. 신버전 writer-capable instance끼리는 이 guard를 공유하지만 live 구버전의 exclusive guard와는 서로 배타적입니다. 따라서 구버전 writer가 살아 있으면 신버전 writer-capable open은 실패하고, 신버전 writer-capable instance가 열려 있으면 구버전 open이 실패합니다. 이 compatibility guard는 root append coordinator가 아니며 신버전 writer들을 직렬화하지 않고 read-only discovery port는 획득하지 않습니다. 각 Session에는 writer owner가 하나뿐이며 그 lease는 Session state를 load하거나 repair하기 전에 얻어 writer lifetime 동안 유지합니다. 특정 Session lease 획득 실패가 다른 Session open이나 write를 막으면 안 됩니다.

모든 append는 해당 Session에만 속한 durable pending marker로 보호합니다. Reader가 marker를 보면 storage를 만들지 않고 대응 Session lease를 검사합니다. Live owner가 있으면 in-flight append로 보고 marker 전 마지막 complete envelope에서 멈추며, owner가 없으면 그 Session만 quarantine합니다. Rollback을 확인하지 못하면 marker를 유지하고 ambiguous complete line을 replay하지 않습니다.

Capacity ceiling은 repository 전체에 적용됩니다. Writer는 최종 repository size 확인, marker publish, append와 sync, 필요한 rollback, marker 제거 동안에만 짧은 root append coordinator를 얻습니다. 이 coordinator는 append 사이에 유지하지 않고 다른 process가 root를 열거나 다른 Session에서 작업하는 것을 막지 않습니다. Writer는 항상 Session lease를 먼저 얻고 root coordinator를 나중에 얻으며 역순으로 얻지 않습니다. Lock과 marker file은 record capacity에 포함하지 않습니다.

복구는 호환성 계약이 지원하는 record만 checksum 검증 뒤 받아들입니다. 완전한 줄을 streaming하고 불완전한 마지막 줄은 uncommitted tail로 보며 다른 완전한 줄의 corruption은 보고합니다. Bounded suffix를 반환하기 위해 log 전체를 materialize하면 안 됩니다. 비어 있지 않은 Session reopen 또는 초기 state load failure recovery 뒤에는 incremental record 전에 complete snapshot이 필요합니다. Message와 tool-output segment는 별도 Session authority가 아닙니다.

Journal과 Request correlation은 하나의 physical availability와 capacity ceiling을 공유합니다. Request detail은 redaction-before-admission gate 전까지 volatile합니다. Durable commit은 append와 sync 뒤에만 in-memory Journal에 publish합니다. Persistence failure 뒤 semantic result는 volatile로 공개하고 durable gap을 유지하며 rollback이라고 보고하지 않습니다. Capacity나 storage failure 때 기존 record는 그대로 두고 Session은 memory에서 계속되며 frontend에 known cutoff, known empty log, unknown cutoff를 구분하는 typed persistent pressure notification을 보냅니다. Known cutoff는 마지막 durable RepositorySequence와, semantic Journal event가 하나도 durable하지 않으면 absent일 수 있는 마지막 durable JournalSequence를 포함합니다. 두 coordinate는 서로 추론하지 않습니다. Gap 뒤 continuous suffix를 주장하지 않으며 capacity가 돌아오면 complete snapshot 뒤에만 incremental persistence를 재개합니다.

Local 저장소는 기본 활성화하고 current-user permission과 configurable capacity ceiling을 적용하며 자동 age expiry나 Session deletion을 하지 않습니다. 첫 구현은 Session마다 synchronous single writer입니다. 측정 evidence 없이 background writer, generic transaction, group commit, compression, index, SQLite projection, alternate encoding, Request Audit physical split을 도입하지 않습니다.


Session Repository는 payload-free Request correlation record와 별도의 bounded payload-bearing `model_replay_delta` semantic record를 같은 checksummed Session log에서 소유합니다. Replay delta, completed outcome, Anchor는 하나의 atomic physical envelope로 기록되어 일부만 durable해질 수 없습니다. Replay delta는 open binding이 정확한 replay profile `kimi-private-local-plaintext/v1`을 가질 때만 provider-private item을 포함할 수 있습니다. 최초의 이 item은 `kimi.assistant-message/v1alpha1`을 사용하고 정확히 제한된 K3 assistant object, binding identity, epoch를 담으며 visible projection이 인접한 semantic replay와 같음을 검증해야 합니다. 이는 payload-bearing Session 의미로 남지만 Transcript, Request trace, discovery, error text, debug output, 모든 frontend read projection에서 제외됩니다. Model replay는 repository append 전에 semantic redaction admission을 통과합니다. Provider-private replay는 correlation이 확인된 완료 Connector response에서만 허용되며 정확한 schema, replay profile, binding, projection, byte-bound 검사를 통과해야 합니다. 내용을 바꾸면 provider continuation을 깨뜨리므로 admission에 실패한 private item은 redact하거나 일부만 저장하지 않고 거절합니다. 허용된 private byte는 다른 Session payload와 같은 user-only local directory와 mode `0600` file에 저장되고 최초 구현에서는 암호화하지 않습니다. 이런 보존 사실은 해당 binding을 선택하기 전에 안내합니다. Repository와 frontend API는 generic record projection으로 그 내용을 노출하면 안 됩니다. Repository capacity와 model context limit은 별개이며 replay/context bound를 넘으면 partial chain이나 silent truncation, summary를 기록하지 않고 Turn을 completed but non-resumable로 남깁니다.


Repository는 binding의 explicit continuation strategy에 따라 replay 유무를 해석합니다. exact_replay(local_client)는 local replay-delta chain에서 validated semantic prefix와 선언된 provider-private extension을 복원합니다. backend_managed_state는 replay delta 없이 payload-free outcome, Anchor, backend locator evidence를 보관하고 Transcript나 Request Audit에서 replay를 합성하지 않습니다. 향후 managed Session Repository는 동일한 semantic Journal과 replay chain으로 exact_replay(managed_server)를 실행할 수 있지만 server와 repository identity, selected boundary, replay-content와 contract digest, binding epoch, availability, retention 검증이 필요합니다. Reserved strategy value만으로 remote storage, replication, conflict handling을 구현했다고 간주하지 않습니다.

## 이유

Legacy shared guard는 신·구 binary가 섞인 전환에서 fail-closed하면서 신버전 writer process끼리는 직렬화하지 않고 read-only discovery도 막지 않습니다. Session-scoped ownership은 같은 semantic history에 writer 둘이 생기는 것을 막고 짧은 append coordinator는 공유 capacity ceiling을 정확하게 보존합니다. 하나의 checksummed envelope와 분리된 semantic 및 physical sequence는 partial durability와 corruption을 명확하게 하며 durable-first publication과 snapshot recovery는 responsive streaming과 정직한 history를 함께 유지합니다.
