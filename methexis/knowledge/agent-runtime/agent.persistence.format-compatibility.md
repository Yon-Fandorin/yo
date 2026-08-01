---
schema: methexis.knowledge/v1alpha1
id: agent.persistence.format-compatibility
kind: decision
owner: agent-runtime
sources:
  - id: agent.persistence-001
    revision: sha256:6955bc840b83ee18a401a1fc530708ce3ae11e255fda52ee03954f10e04c247e
relations: {}
---
# Session persistence format compatibility

## Statement

The UUIDv7-only, descriptor-aware semantic Session Journal format
`yo.semantic-journal-commit/v1` and checksummed physical Session-record envelope
`yo.session-record/v1` are yo's initial public-format candidates. Their exact
closed shapes and UUIDv7 Session identity are part of each baseline; a matching
schema tag alone MUST NOT admit a record.

This reset explicitly supersedes the development-only semantic meanings named
`yo.semantic-journal-commit/v1` through `/v4` and physical meanings named
`yo.session-record/v1` through `/v3`. Semantic `/v2`, `/v3`, and `/v4`, physical
`/v2` and `/v3`, and legacy numeric-identity records that reuse either `/v1`
tag MUST fail closed before semantic admission. They MUST NOT be migrated,
reinterpreted, skipped as valid history, or exposed as readable Session data.
Recovery MUST read only formats explicitly supported by an accepted
compatibility contract; at this baseline that set contains only the two current
closed `/v1` shapes.

This contract governs Session Journal and Session-record persistence only.
Other persistent formats, including `yo.workspace-host-id/v1`, remain under
their own owning contracts.

Any further pre-release replacement under either `/v1` tag requires another
explicitly reviewed SOT revision that names the replaced shape and accepts its
data impact. After yo's first public release, evolution MUST preserve published
versions or provide an explicitly reviewed compatibility or migration contract;
it MUST NOT silently reset a published schema tag.

## Rationale

Reusing `v1` before the first release gives the public contract an honest
starting point without preserving experimental numbering. Naming the displaced
development schemas and making closed shape admission part of the baseline
prevents an old record with the same tag from being mistaken for current data.
One shared policy owner keeps physical and semantic compatibility rules aligned.
