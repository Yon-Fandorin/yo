---
schema: methexis.knowledge/v1alpha1
id: agent.input.workspace-reference
kind: decision
owner: agent-runtime
sources:
  - id: agent.input-001
    revision: sha256:d3801b346e909779fb4e85d8242afdcfa6daefdb1632178c0308520ad3f82c38
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.core.frontend-independent-boundary
  constrained_by:
    - agent.runtime.command-event-boundary
  applies_to:
    - yo-core::UserInput
    - yo-cli::workspace-reference-provider
---
# Execution-workspace reference

## Statement

An explicitly accepted workspace candidate MUST become a frontend-independent
typed reference containing an execution-environment identity, opaque workspace
and root identities, normalized root-relative path, and kind of file or
directory. Its prompt projection
MUST remain a familiar `@path` token, but that text alone MUST NOT establish the
reference. Multiple accepted workspace references MAY appear in one request in
their draft order.

The execution environment that authoritatively owns the workspace MUST own
discovery, authorization, and submission-time resolution, independently of
where orchestration, the frontend, or an Agent Backend connector runs. The
runtime MUST reach that environment through the capability or connector named
by the Session topology. The client MUST NOT offer a client-local path as though
it belongs to another execution environment. Discovery MUST be independent of
a particular Agent Backend and MUST publish only compact semantic candidates to
the frontend. `yo-cli` MAY wire an in-process local provider but MUST NOT own
these semantics.

Version 1 discovery MUST include files and directories under the supplied
workspace roots. It MUST honor the effective Git ignore stack, including
nested ignore files, repository exclude, and configured global excludes; MUST
exclude Git internals; MUST NOT follow a directory symlink or admit a resolved
path outside the workspace; and MUST include hidden entries that are not
otherwise ignored. Ignore behavior is a relevance policy rather than a
permission boundary. A stronger deny policy, explicit include-ignored mode,
line-range reference, and non-workspace attachment require later contracts.
Discovery visibility and authorization are separate decisions: a path becoming
ignored after selection MUST NOT by itself invalidate the reference, while an
authorization denial MUST. Unreadable roots, malformed ignore inputs, bounded
traversal, and provider failures MUST produce typed incomplete or error status
rather than silent omission presented as an exhaustive result.

Search MUST run outside the UI thread and MAY use a lazily invalidated cached
inventory. A bounded scan that cannot prove completeness MUST publish a typed
incomplete status rather than silently presenting its result as exhaustive.
Ranking MUST be deterministic and Unicode-normalized, prioritizing exact path
and basename matches, then prefixes and path boundaries, then contiguous and
ordered fuzzy matches. Stable ties MUST prefer fewer gaps, shallower and
shorter paths, then lexical workspace-relative path. Version 1 MUST NOT use
recency or frequency. A result SHOULD display basename as its label and parent
path plus typed file or directory kind as detail.

Selecting a candidate MUST NOT read its file contents or recursively attach a
directory. Immediately before dispatch, the runtime MUST revalidate that the
same execution environment, workspace, root, path, kind, resolved containment,
and authorization still hold. Ordinary file-content changes MUST NOT invalidate
a path reference; a changed root mapping, kind, symlink target, containment, or
authorization MUST. Whole-request admission MUST validate every reference
before any Backend dispatch or skill loading begins. The frontend MUST retain
an immutable submitted draft snapshot until `yo-core` returns a result. Accepted
and Rejected MUST carry that submission identity. Accepted is the ownership
transfer point for that exact snapshot; the frontend clears the editor only if
its current draft still matches the submitted snapshot, and otherwise preserves
the newer draft while the older snapshot proceeds. Rejected MUST NOT consume or
mutate either snapshot. A missing, changed, denied, or environment-incompatible reference
MUST reject admission with a typed diagnostic and preserve the draft and
annotation for removal or reselection.
It MUST NOT silently drop the reference, substitute another path, or escape the
workspace. A Backend adapter MAY translate the validated reference into its
native structured input or visible path form, but MUST preserve this meaning.
Accepted structured references MUST remain structured in semantic input and
Journal records rather than being reconstructed later from visible `@path`
text.

## Rationale

Execution-host discovery keeps local, remote, TUI, and future GUI behavior
honest. A path reference rather than eager content attachment avoids hidden
context cost and gives the agent freedom to inspect only the files needed for
the submitted task.
