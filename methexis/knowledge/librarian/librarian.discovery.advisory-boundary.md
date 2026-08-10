---
schema: methexis.knowledge/v1alpha1
id: librarian.discovery.advisory-boundary
kind: rule
owner: librarian
sources:
  - id: librarian.discovery-model.advisory-boundary
    revision: sha256:bfc692bf549bf9558eeecd5cbd372a3a3497d6a0cfbabc128892a1372f0fad8c
---
# Librarian advisory discovery boundary

## Statement

Final context selection is deterministic. Librarian is an advisory discovery
and catalog component that MAY:

- propose candidate KnowledgeIds and explain each reason;
- map stable semantic IDs to physical locations;
- recommend classification and placement;
- detect duplicates, orphans, and broken references;
- generate reviewable relocation plans.

Librarian MUST NOT approve meaning, mutate canonical authority silently, or
bypass a Checkpoint. Search and LLM output are candidate signals only.

The first agent path accepts a versioned request containing at least one of a
non-empty natural-language query or one or more code-path, symbol, and
KnowledgeId anchors. It returns a deterministic, versioned candidate set.
Each candidate contains a stable KnowledgeId and machine-readable reasons;
Librarian never labels it approved, active, or safe to use. Methexis owns
required-closure expansion and final eligibility filtering.

An unresolved anchor and a query with no matches are successful observations
and remain explicit in the result. A request with neither query nor anchors is
invalid. An invalid catalog produces a structured failure and no partial
candidate set; silently skipping a damaged record could hide required
knowledge.

The discovery command writes exactly one complete structured success value to
stdout. It writes a structured failure to stderr, leaves stdout empty, and
returns non-zero. Callers MAY pipe or redirect successful JSON to a file; the
Pilot MUST NOT create or own a persistent candidate artifact or cache. The
result identifies the request, catalog snapshot, compiler, ordered candidates,
reasons, unresolved anchors, and truncation. S4 hashes the exact candidate input
it consumes into the ContextBuild lineage.
