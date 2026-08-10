---
schema: methexis.knowledge/v1alpha1
id: librarian.catalog.snapshot-ranking
kind: rule
owner: librarian
sources:
  - id: librarian.catalog-model.snapshot-ranking
    revision: sha256:59ee99657a9def74a745a9795d32f39ecc2e134878177b740f7e8e1f63bfbe9e
---
# Librarian catalog snapshot and ranking

## Statement

The initial catalog contains every structurally valid KnowledgeUnit, regardless
of approval or eligibility. Searchable fields are its ID, title, canonical
English body, typed relations, physical location, and an exact-revision valid
Korean review Projection when present. Source content, approvals, and
Checkpoints do not contribute text-ranking signals. Structured code Source
locators MAY satisfy an explicit path or symbol anchor without making Source
content searchable.

Librarian builds that catalog from the current working tree, including valid
untracked Draft records inside the declared corpus directories. It does not
resolve `develop` or grant trust to those files. It captures the sorted relative
paths and exact relevant bytes into one immutable catalog snapshot before
ranking. A concurrent change that prevents a consistent capture returns a
retryable failure and no candidate set.

Initial ranking is deterministic lexical evidence, ordered from exact ID and
anchor matches through phrase and token overlap to one-hop relation signals.
Every contribution remains inspectable in the candidate reasons. Librarian
MUST NOT expand required dependency closure. Semantic or vector retrieval, LLM
ranking, fuzzy matching, and language-specific morphological dependencies
remain evidence-gated extensions rather than Pilot defaults.
