---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.checkpoint.immutable-publication
revision: sha256:7fadc246f40d2c3f23c3df03a0c2dec08af9d356d07ab68bfa48e7e39f529745
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:4c3cb5ba5f4717cd7f2dcd5b44164af086ab866c7c8358aa949b33c26ee8de3e
---
# Korean Review Projection

## Translation

# 불변 Checkpoint 게시

## 선언

Checkpoint 생성은 설정된 하나의 trusted Git ref를 정확한 commit으로 해석하고, checkout하지 않은 채 그 고정 snapshot에서 필요한 Source, Knowledge, Projection, approval blob을 읽어야 하며, 캡처한 byte만으로 승인된 closure를 선택해야 합니다.

제안은 schema, trusted commit, 역사적 Source-status marker, root, 선택된 정확한 revision, 선택 이유를 `CheckpointId`에 결합하는 결정론적 canonical record를 사용해야 합니다. 게시 전에 동일한 record가 기록된 commit에서 재현되어야 합니다.

제안은 immutable create-if-absent artifact로 게시해야 합니다. 기존 artifact가 동일하면 재사용할 수 있습니다. 기존 byte가 다르거나 closure가 유효하지 않거나 입력을 읽을 수 없다면, 대체하거나 fallback하거나 부분적인 다른 Checkpoint를 만들지 않고 실패해야 합니다.
