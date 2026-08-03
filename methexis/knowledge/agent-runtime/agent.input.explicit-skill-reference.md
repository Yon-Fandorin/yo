---
schema: methexis.knowledge/v1alpha1
id: agent.input.explicit-skill-reference
kind: decision
owner: agent-runtime
sources:
  - id: agent.input-002
    revision: sha256:e089ea64856fe347087b7f8db6e10a474d8200e632edea495d6e57653133f7c1
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.core.frontend-independent-boundary
  constrained_by:
    - agent.runtime.command-event-boundary
  applies_to:
    - yo-core::UserInput
    - yo-core::AgentSession
    - yo-cli::skill-catalog-provider
---
# Explicit skill reference

## Statement

An explicitly accepted skill candidate MUST become a frontend-independent typed
reference containing an execution-environment identity, opaque stable skill
identity, provenance, catalog generation, and per-entry revision or digest. Its
prompt projection MUST remain a familiar `$name` token, but that text alone
MUST NOT invoke or identify a skill. Frontend-independent input and Journal
records MUST represent explicit skills as an ordered collection. Version 1
admission policy MUST accept cardinality `0..=1` and MUST reject a second item
both at UI acceptance and at runtime admission while preserving the draft and
existing reference. A later reviewed policy MAY raise that limit without
replacing the typed input shape.

The execution environment that authoritatively owns the skill assets and can
execute their workflow MUST own catalog discovery, eligibility, loading, and
context assembly, independently of where orchestration, the frontend, or an
Agent Backend connector runs. The runtime MUST reach it through the capability
or connector named by the Session topology and MUST NOT present client-local
skills as available in another environment. `yo-cli` MAY wire an in-process
local provider but MUST NOT own these semantics. Backend adapters receive an
already resolved explicit skill and MAY use a native skill input only when it
preserves the same identity and instructions.

The frontend catalog MUST contain descriptors rather than full skill bodies:
opaque identity, display name, concise description, provenance class or label,
user-invocable policy, typed availability with reason, catalog generation, and
per-entry revision. The stable identity MUST remain scoped to its execution
environment and survive catalog refreshes and process reconnects; catalog
generation fences discovery snapshots, while unrelated catalog changes MUST
NOT invalidate an unchanged selected entry.
Workspace, user, plugin or bundled, and managed sources MAY contribute entries.
Same-name entries MUST remain distinct and show provenance; neither discovery
nor acceptance may silently shadow or merge them. Known but unusable entries
MUST remain visible as disabled with an exact reason, MUST be skipped by
navigation, and MUST NOT be accepted. An authority policy MAY hide an entry
entirely; a hidden entry is not advertised as a known frontend candidate.

Selection MUST attach only the typed reference. It MUST NOT load `SKILL.md`,
execute dynamic content, run bundled scripts, or inject instructions into the
agent context. At submission, whole-request admission MUST atomically revalidate
the exact environment, identity, entry revision, user-invocable policy,
environment compatibility, and required assets, then load from that same
validated snapshot to prevent a validation/load race. Every workspace and skill
reference MUST validate before any Backend dispatch or skill body loading
begins. Supporting
references and assets SHOULD remain lazy unless the skill contract requires
them immediately. The selected body, required supporting material, and request
framing are charged to the assembled request context; the complete catalog MUST
NOT be injected merely to support the selector.

A stale, removed, disabled, incompatible, or over-budget skill MUST reject
admission and provide a typed diagnostic for removal, refresh, or reselection.
Ambiguity is only a malformed or legacy-reference failure because an accepted
opaque identity is never resolved by name. The frontend MUST retain an immutable
submitted draft snapshot until `yo-core` returns a result. Accepted and Rejected
MUST carry that submission identity. Accepted is the ownership transfer point
for that exact snapshot; the frontend clears the editor only if its current
draft still matches the submitted snapshot, and otherwise preserves the newer
draft while the older snapshot proceeds. Rejected MUST NOT consume or mutate
either snapshot. The runtime MUST NOT downgrade
it to plain `$name` text, select a same-name replacement, partially load it, or
silently truncate required instructions. Explicit reference semantics do not
decide whether an agent may independently discover and invoke other eligible
skills during its turn.

## Rationale

Exact host-owned identity avoids invoking the wrong same-name skill and makes
remote sessions and durable journals interpretable. Deferring body loading to
submission preserves progressive disclosure and allows all selected input to
be validated as one honest dispatch boundary.
