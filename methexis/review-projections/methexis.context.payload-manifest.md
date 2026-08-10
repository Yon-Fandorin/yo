---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.context.payload-manifest
revision: sha256:cb39ec2ef421ab2d2535232c67a2233b04c1a6fbd30f50e58ff770974125277d
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:59b21e9d42a0592f3d701f9f3471bf107f80cef29a2603c8c083c91792c3d188
---
# Korean Review Projection

## Translation

Pilot artifact는 Git 밖의 BuildId directory에 저장합니다. context.md는 token budget에 포함되는 유일한 canonical agent payload이며 required relation의 위상 순서와 exact body를 담습니다. manifest.json은 authority·selection·omission·tokenizer·BuildId lineage와 context hash를 기록합니다.

### 전체 정본 원문 대조

Pilot artifacts remain outside Git history, for example:

```text
.local-exclude/methexis/builds/<BuildId>/
  context.md
  manifest.json
```

`context.md` is the minimal canonical English payload for the agent and is the
only artifact charged to the request token budget. Its versioned compiler
profile fixes the exact preamble, heading grammar, and emitted relation fields.
Units use a deterministic topological order over `depends_on` and
`constrained_by`, with ascending KnowledgeId as the tie-breaker, so required
units precede their consumers. Each unit emits its stable KnowledgeId, exact
canonical English body, and its included required-relation IDs. Golden fixtures
pin the exact payload bytes and token totals. The payload excludes Korean review
Projections, raw Source content, validation evidence, full approval or
Checkpoint records, and retrieval diagnostics.

`manifest.json` records the Checkpoint and its stable authority-basis commit,
exact candidate-input hash, direct anchors, included and omitted revisions and
reasons, blocked inputs, candidate reasons, compiler and profile identity,
tokenizer and budget, BuildId preimage fields, and the `context.md` hash. It
does not contain its own hash. Agent work records the BuildId it consumed.
