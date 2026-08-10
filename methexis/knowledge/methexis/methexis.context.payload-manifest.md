---
schema: methexis.knowledge/v1alpha1
id: methexis.context.payload-manifest
kind: rule
owner: methexis
sources:
  - id: methexis.context-model.payload-manifest
    revision: sha256:df655205687a0e616041c71c2fb90ed7858ecc5973820422715b6f1f1035c74d
relations:
  depends_on:
    - methexis.context.build-identity
---
# Context payload and manifest contract

## Statement

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
