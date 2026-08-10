---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.context.build-publication
revision: sha256:acecd1259c996854a9c44e64c28ffb396f74a88e157667cecee8f0681d1a1652
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:09af437defcaf8f41b19bfb56f0b64a53032049bebdd9a4b25095dda5b30005b
---
# Korean Review Projection

## Translation

ContextBuild artifact는 atomic create-if-absent로 publish하며 교체하지 않습니다. 기존 BuildId가 있으면 manifest와 모든 artifact hash를 안전한 directory handle 기준으로 검증해 exact match만 재사용하고, mismatch는 기존 artifact를 보존한 채 corruption collision으로 실패합니다.

### 전체 정본 원문 대조

Artifact publication is atomic create-if-absent, never replacement. The
publisher builds in a temporary sibling and installs it with a no-clobber
primitive. If the BuildId destination already exists, it verifies the manifest
and every artifact hash and reuses the exact match. Existing-build verification
rejects symlinked build or artifact paths, resolves them relative to retained
directory handles, and retains those handles through verification and result
construction. A mismatch is a determinism or corruption collision: quarantine
the new temporary output, keep the existing artifact unchanged, and fail.
Partial output is never eligible.
