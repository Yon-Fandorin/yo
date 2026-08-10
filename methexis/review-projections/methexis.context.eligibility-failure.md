---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.context.eligibility-failure
revision: sha256:11cc440f22ff2557139be2abe154a24da78244ee0327502832511606b0c31b3b
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:5baa7b8e83aa9b4c57e5120e82eb0021dbdc8ff938c00f99ca737d2dc30029b8
---
# Korean Review Projection

## Translation

Source·approval·evidence freshness mismatch는 Checkpoint를 degrade하고 영향받은 required closure를 실패시킵니다. required 지식의 missing·blocked·budget 초과는 build 실패이고, optional 지식은 manifest에 이유를 기록한 경우에만 제외할 수 있으며 이전 승인 revision으로 fallback하지 않습니다.

### 전체 정본 원문 대조

A Source, approval, or evidence freshness mismatch degrades the Checkpoint and
fails the affected required dependency closure before context is returned.
Optional affected knowledge is omitted with a structured reason. Local
ContextBuild corruption or a determinism collision fails storage verification
but does not alter Checkpoint eligibility. A resolver MUST NOT fall back to an
older approved revision. A new review, evidence run, and activation restore
eligibility.

Missing, blocked, or unaffordable required knowledge MUST fail the build.
Required bodies MUST NOT be silently truncated. Optional knowledge MAY be
omitted only when the manifest records the omission and reason.
