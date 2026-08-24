---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.checkpoint.immutable-publication
revision: sha256:762be92bf535190df41f18d33d74b5327e62b9910185395a1368735c95f5535e
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:560f11aed358af0ab7dc67df336568ce2db0e28949890e22a66317ee70a3ef15
---
# Korean Review Projection

## Translation

# Immutable Checkpoint 게시

## 명세

Checkpoint 생성은 configured trusted Git ref 하나를 정확한 commit으로 resolve하고, 필요한 Source, Knowledge, approval, 선언된 review-basis evidence를 checkout 없이 그 pinned snapshot에서 읽어야 한다. Canonical 근거 승인은 Projection blob을 요구하지 않는다. Projection 근거 승인은 참조된 정확한 Projection profile, compiler, hash를 요구한다. 참조되지 않은 optional Projection은 선택된 approval closure의 일부가 아니다. Approved closure는 캡처한 해당 byte들만으로 선택해야 한다.

Proposal은 schema, trusted commit, historical Source-status marker, root, 선택된 정확한 revision, selection reason을 `CheckpointId`에 결속하는 deterministic canonical record를 사용해야 한다. 게시 전에 동일한 record를 기록된 commit으로 재현할 수 있어야 한다.

Proposal은 immutable create-if-absent artifact로 게시해야 한다. 기존 artifact가 동일하면 재사용할 수 있다. 서로 다른 기존 byte, invalid closure, 선택한 review basis의 missing 또는 mismatched evidence, unreadable input은 replacement, fallback, partial alternative Checkpoint 없이 실패해야 한다.
