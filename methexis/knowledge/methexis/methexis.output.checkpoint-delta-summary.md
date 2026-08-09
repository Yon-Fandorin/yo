---
schema: methexis.knowledge/v1alpha1
id: methexis.output.checkpoint-delta-summary
kind: rule
owner: methexis
sources:
  - id: methexis.output-001
    revision: sha256:78c46bdde00c0e2cc427d9200296bf1dcc5f1993408f8e340f614f760b020b61
relations:
  applies_to:
    - tools/methexis/src/checkpoint/operations.rs::create
    - tools/methexis/src/checkpoint/operations.rs::propose_activation
---
# Checkpoint delta success summary

## Statement

A successful `create-checkpoint` or `propose-activation` operation MUST compare its candidate Checkpoint with the active Checkpoint captured from the same pinned trusted snapshot. The success result MUST identify that trusted commit, the candidate Checkpoint ID and hash, the candidate artifact path, and the baseline active Checkpoint ID and hash or explicit baseline absence.

For every KnowledgeId whose presence or RevisionId differs, the result MUST include one entry sorted by KnowledgeId with its before RevisionId or absence and after RevisionId or absence. For every root whose presence differs, it MUST include one entry sorted by root with before and after presence. It MUST also report the total number of units in the candidate required closure and the number of units whose KnowledgeId and RevisionId are unchanged in both Checkpoints. When no active Checkpoint exists, every candidate KnowledgeId and root is an addition and the unchanged count is zero.

The default success result MUST NOT repeat unchanged closure entries or selection reasons. The immutable candidate Checkpoint artifact remains the integrity-pinned owner of its complete roots, units, revisions, and reasons. A failure MUST retain every affected identifier required to diagnose and recover from that failure; delta-first success reporting MUST NOT reduce failure evidence.
