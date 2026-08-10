---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.context.build-reuse
revision: sha256:6f36c8f68142907af9849cdcae85f3e96351d1df9eac3d0019b4c234d2e88184
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:186279f1b20f68f6b478e4e6f2946a183b005c472cdaa6b9ac220ef6d0e80692
---
# Korean Review Projection

## Translation

고정 BuildId store가 불변 원본을 소유합니다. 재사용 전 freshness와 stored manifest/artifact hash를 검증하고 같은 BuildId의 다른 content는 덮어쓰지 않습니다. caller-selected path는 초기 resolution 입력이 아니며 이후 검증된 artifact를 원본 변경 없이 stream/copy할 수 있습니다.

### 전체 정본 원문 대조

The fixed BuildId store owns the immutable original in the Pilot. A successful
structured result returns `created` or `reused`, the BuildId, and the paths and
hashes of both artifacts. That per-operation result also records the exact
current trusted commit observed for final verification; it may therefore differ
across safe reuse of the same immutable build. Cache reuse first reproduces the
BuildId plan, verifies current freshness, and verifies the stored manifest and
artifact hashes. Existing different content at the same BuildId is corruption
and MUST NOT be overwritten.

Caller-selected output paths are not part of initial resolution. A later
read/export operation MAY stream a verified artifact to stdout or copy it to a
caller-selected destination without changing the managed original, BuildId,
lineage, or integrity checks.
