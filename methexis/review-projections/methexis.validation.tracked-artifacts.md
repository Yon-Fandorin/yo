---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.validation.tracked-artifacts
revision: sha256:25e4e842c1f60c385ff3d44f7bf5924ede1b12f6c01188054ff2d0aa99605a8c
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:fc75d8f962b41ff1d3138d659a044f09f0940d2ddb37a979518c6c3bcfa863fa
---
# Korean Review Projection

## Translation

artifacts check class는 trusted authority에서 파생된 등록된 tracked contract artifact만 검사합니다. 로컬 rebuildable cache나 일반 Rust test/lint/format은 이 class의 scope가 아닙니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

The `artifacts` class validates only tracked contract artifacts derived from
trusted authority. In this Pilot it checks the registered context manifests'
Checkpoint ID, hash, and authority-basis commit against the active trusted
Checkpoint. It does not claim byte-for-byte regeneration and does not inspect
or gate rebuildable `.local-exclude/` ContextBuild caches. Generic Rust tests,
linting, and formatting remain Cargo and `hk` responsibilities rather than
Methexis check classes. A repository or isolated fixture with none of the
registered tracked artifact paths has an empty, passing `artifacts` class.
Presence of any registered path enables the closed set, after which every
registered artifact is required. If no active trusted Checkpoint is available,
`authority` may pass as an evaluation while `artifacts` is `blocked`; the
requested validation is incomplete, so the overall report fails and directs
the caller to establish active trusted authority.
