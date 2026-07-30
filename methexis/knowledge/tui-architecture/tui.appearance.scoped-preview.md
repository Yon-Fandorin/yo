---
schema: methexis.knowledge/v1alpha1
id: tui.appearance.scoped-preview
kind: decision
owner: tui-architecture
sources:
  - id: tui.appearance-004
    revision: sha256:e61895857e88ee849cbdfa18509a3e2711104a4e84c84f41805ad121981d21ea
relations:
  depends_on:
    - tui.appearance.session-publication
    - tui.appearance.frame-consistency
  applies_to:
    - yo-tui::settings::appearance-preview
---
# Scoped appearance preview

## Statement

A settings preview MUST provide its draft appearance snapshot explicitly only
to the owning preview subtree. The transcript, prompt, settings chrome, and
other subtrees MUST continue to receive the committed snapshot. Preview state
MUST NOT mutate or temporarily replace the session's committed snapshot.

Each preview MUST use a non-reusable, generation-bearing opaque `PreviewId`,
bind to the owner subtree lifetime, and record its base committed revision.
Cancel, owner close, owner error, and suspend MUST discard the preview. A
committed revision change MUST make a preview based on the prior revision stale
and MUST require explicit invalidation rather than silent rebasing.

Save MUST run as an owner transaction: revalidate the relevant durable
configuration baseline and committed appearance revision, persist successfully,
then publish one complete committed snapshot for the next logical frame. A
persistence failure or conflict MUST preserve committed appearance.

A preview glyph change MUST remeasure only the preview subtree. A global commit
MUST invalidate and remeasure the complete logical frame. Exact persistence
baseline, conflict UI, and durable failure ordering require a later contract
with the concrete settings storage owner. This KnowledgeUnit MUST remain
inactive until that preview implementation and evidence exist.

## Rationale

Explicit subtree injection permits a settings preview to differ without
leaking draft state into the rest of the UI. Generation and revision binding
prevent stale drafts from becoming durable configuration.
