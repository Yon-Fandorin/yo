---
schema: methexis.knowledge/v1alpha1
id: methexis.context.request-selection
kind: rule
owner: methexis
sources:
  - id: methexis.context-model.request-selection
    revision: sha256:560d08c97428c43a100b348d1d28c31fdf183a389d204464e5809c7bbe09bc24
---
# Context request and direct-anchor selection

## Statement

Context selection starts from explicit paths, symbols, KnowledgeIds, and
Librarian candidates. The resolver then:

1. resolves one immutable trusted integration commit;
2. loads the tracked active Checkpoint from that commit;
3. captures current local Source bytes and identities into an immutable
   Source snapshot;
4. verifies its cheap freshness guard;
5. filters by active Checkpoint eligibility;
6. expands required and constraining relations;
7. attaches applicable validation evidence;
8. applies priority and token-budget packing;
9. final-revalidates observed mutable Sources;
10. publishes an immutable, traceable `ContextBuild`.

The versioned request MUST contain at least one direct anchor or one Librarian
candidate result reference. A candidate reference is a repository-relative
local path plus the expected SHA-256 of the exact file bytes; the candidate
JSON is not embedded in the request. The resolver captures and verifies those
bytes before parsing and records their hash in ContextBuild lineage. A direct
KnowledgeId, path, or symbol anchor is a required root. An unresolved direct
path or symbol fails explicitly; when it resolves to multiple exact units, all
of them are required roots. Librarian candidates are advisory optional inputs.

Direct anchors resolve only against the KnowledgeSnapshot loaded from the
pinned trusted commit. A KnowledgeId matches its exact semantic ID. A path
matches either the exact canonical repository-relative Knowledge record path or
an exact `applies_to` value; a symbol matches only an exact `applies_to` value.
Code Source locators, Librarian's working-tree catalog, Draft files, and fuzzy
text do not participate. Anchor values use the same typed, trimmed
duplicate-rejection semantics as Librarian requests; the S4 request schema
additionally declares maximum anchor counts and value lengths.
