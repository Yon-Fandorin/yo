# Review packets

This file is the repository workflow authority for review input readiness,
packet construction, preparation, validation evidence, and experimental wire
versioning. It is routed directly from [`AGENTS.md`](../AGENTS.md).

External authorization and delivery belong to
[Review delivery](review-delivery.md). Verdict evidence, approval, integration,
and cleanup belong to [Review and integration](review-and-integration.md).

## Construction and preparation

After committing a clean candidate and saving declared validation output as
bounded evidence files, check the same versioned request before running
ContextBuild or packet measurement:

```bash
cargo xtask slice review-packet --check-readiness <request.json>
```

Readiness requires the exact bound Slice branch, base, contract, and clean
candidate; a direct ContextBuild request path inside that candidate worktree;
and unique, readable authority, validation, KnowledgeId, lens, and question
inputs. It final-revalidates those inputs before returning. It does not run
ContextBuild, capture the diff, tokenize a packet, create a ReviewId, or publish
an artifact. Methexis still owns and validates ContextBuild request semantics
when resolution starts. A green readiness result covers only these input
boundaries; it is preparation, not review evidence.

For structured validation summaries, readiness also verifies the internal
summary name against the request name, status/exit consistency, and canonical
hash fields. `v1alpha1` and `v1alpha2` summaries must identify the exact clean
candidate execution and record `reused: false`; `v1alpha2` must name the
supported reuse policy. This deliberately happens before ContextBuild,
tokenization, or publication so a wrapper-name typo, stale result, or malformed
execution identity cannot consume a review round. The later Slice gate still
binds the recorded argv count and hash to its exact requested command; the
review-packet request carries no argv values and therefore cannot perform that
last comparison. Legacy `yo.validation-run-summary/v1` remains accepted with
its older name and status guarantees only.

When correctness depends on behavior owned by an external executable rather
than the repository's typed seam—such as Git hook source selection, process
exit, signal, timeout, or filesystem publication ordering—include at least one
validation item named `external-operation/<portable-label>`. Its path must be a
JSON artifact with this shape:

```json
{
  "schema": "yo.external-operation-evidence/v1",
  "candidate_commit": "<full-candidate-commit>",
  "operation": {
    "working_directory": ".",
    "argv": ["git", "commit", "--amend", "--file", "message"],
    "expected_exit": {"kind": "code", "value": 1},
    "observed_exit": {"kind": "code", "value": 1}
  },
  "counterfactual": "The amend must fail before changing HEAD.",
  "observations": [
    {
      "name": "HEAD",
      "expected_relation": "equal",
      "before": "<full-before-commit>",
      "after": "<full-before-commit>"
    }
  ]
}
```

Record the working directory and use an argv array, not a shell transcript.
Exit kinds are `code`, `signal`, or `timeout`; observations use `equal` or
`different` and record explicit before/after values. Readiness requires a
well-formed, internally consistent artifact bound to the exact candidate,
reports how many such items it froze, and revalidates the same bytes before
returning. It does not execute arbitrary commands or prove the author ran them.
Root self-check and independent review still own whether the chosen
counterfactual discriminates the claimed failure boundary. A finding-resolution
delta may explicitly reuse unchanged evidence; an affected
`external-operation/*` item must bind its replacement candidate.

After readiness succeeds, inspect the exact candidate and run the
non-publishing preflight from that unchanged request:

```bash
cargo xtask slice review-packet --preflight <request.json>
```

Keep the named ContextBuild request inside the candidate Slice worktree;
Methexis rejects a request path that escapes that repository before resolving
any context.

The preflight captures and final-revalidates the same request, contract,
ContextBuild, authorities, validation evidence, and base-to-candidate diff used
by publication. It returns the prospective ReviewId, exact complete-packet byte
and token totals, maximum budget, and independently measured content/rendered
cost for each section. Section token counts are independently tokenized,
non-additive diagnostics; never sum them, and use the complete-packet token
count as the authoritative budget value. Preflight writes no eligible packet or
manifest. Any changed input invalidates the result; do not cite it as review
evidence.

For `yo.slice-review-markdown/v1alpha2`, preflight also reports an exact
content-addressed input prefix ending after the repository-authority sections.
Its standalone token count is another non-additive diagnostic, not a cached
token count. Matching prefix bytes create only a cache opportunity; record an
actual cache hit only when the provider exposes matching runtime metrics.

A later independent Methexis activation may use the experimental request
schema `yo.slice-review-packet-request/v1alpha3` with delivery profile
`yo.slice-review-markdown/v1alpha3`. It must add `activation_request_path`
naming the exact activation request inside the clean candidate worktree. The
packet resolves a review-only prospective ContextBuild, binds that request,
the proposed immutable Checkpoint, the proposed active record, its canonical
predecessor, and the trusted `develop` basis, and labels every result and
manifest `prospective`. It grants no approval, activation, or general context
eligibility. Do not infer an activation request or fall back to active
authority when any proposal input disagrees.

The change that introduces or modifies this prospective packet path must use
the already accepted ordinary review protocol. It cannot use the new path to
review its own enabling contract or implementation. Its first eligible use is
a later, independently prepared activation candidate; after that candidate is
integrated, ordinary full Methexis validation must still establish trusted
active authority.

The implementation enforces that bootstrap boundary: trusted `develop` must
contain the exact versioned capability record, and the candidate diff must be
the closed activation-only transition (active record, one immutable
Checkpoint, and the complete registered ContextBuild manifests). A missing or
different trusted capability, or any implementation, workflow, documentation,
or unrelated candidate path, requires the ordinary review protocol.

When the preflight is ready and its inputs are frozen, build the
content-addressed review input:

```bash
cargo xtask slice review-packet <request.json>
```

The request names the ContextBuild request and required included KnowledgeIds, exact Slice Contract,
repository-authority paths not carried by that build, validation evidence,
review lenses and questions, experimental `yo.slice-review-markdown/v1alpha2`,
`o200k_base/v1`, and the maximum managed-payload tokens. The command derives
the base from the bound Slice Contract and the candidate from clean `HEAD`,
captures a no-renames binary diff, and returns only the immutable packet and
manifest paths, hashes, ReviewId, and token count. An over-budget packet fails
without truncation. Deliver the exact `packet.md` bytes as the provider's sole
caller-controlled prompt; do not add parallel instructions or authority.
Publication is content-addressed: the same frozen request and inputs return the
existing packet and manifest with `status:"reused"`. Reuse that result directly;
do not copy it to a new coordination path or count it as another review round.

The current experimental profile renders the ContextBuild and repository authorities
under stable logical wrapper paths before the candidate-specific plan,
contract, evidence, instructions, and complete diff. Its manifest binds the
exact prefix boundary, hash, and standalone token count while preserving all
physical input paths in the complete plan and manifest. It never emits a
prefix-only or reference-only review. Frozen `yo.slice-review-markdown/v1` and
the superseded experimental `yo.slice-review-markdown/v1alpha1` requests and
manifests remain supported for exact in-flight reproduction and may root the
same unchanged review-delta v1 chain. New requests use `v1alpha2`; a published
profile identifier is never reinterpreted in place. `v1alpha3` is reserved for
the explicit prospective-activation request above and does not replace the
ordinary `v1alpha2` route.

For a new original review, prefer one integrated, request-free preparation over
manually copying the ContextBuild, packet, egress, admission, and delivery JSON
files. From the clean candidate worktree, provide only the semantic review
selection and one authorized target:

```json
{
  "schema": "yo.slice-review-prepare-request/v1alpha6",
  "slice": "example-slice",
  "knowledge_ids": ["methexis.review.bounded-packet"],
  "context_max_tokens": 16000,
  "repository_authority_paths": [],
  "repository_authority_policy": "changed-workflow-authority/v1alpha2",
  "validation_evidence": [
    {"name": "xtask", "path": "/absolute/path/to/xtask.json"}
  ],
  "review_lenses": ["fresh-context", "code-quality"],
  "review_questions": ["Is the complete reviewed boundary correct?"],
  "max_managed_payload_tokens": 100000,
  "target": {
    "kind": "managed_model",
    "provider": "qwencloud",
    "account": "default",
    "model": "qwen3.8-max",
    "connection_repository_path": "/absolute/path/to/connections.yaml",
    "session_repository_path": "/absolute/path/to/sessions"
  }
}
```

```bash
cargo xtask slice review-prepare <request.json>
```

The command requires the standard bound contract at
`.local-exclude/coordination/<slice>/slice-contract.json`, writes the ContextBuild
and packet requests only below the candidate worktree's ignored
`.local-exclude/coordination/<slice>/`, and publishes the egress, admission, and
delivery requests only below the shared standard coordination directory. It
evaluates request-free target admission before ContextBuild or packet work,
builds or content-address reuses the packet once, binds the current canonical
standing authorization, requires an empty delivery output directory, and
returns the exact packet budget plus one `deliver_once` or
`deliver_delegated_once` next action. It makes zero Provider
requests and never retries, steers, falls back, or selects another target.

Preparations from v1alpha2 through v1alpha6 append one fixed output-contract
instruction after the caller's review questions. After any explanation, the
reviewer must end with exactly one terminal envelope and no trailing prose:

```text
<<<YO-SLICE-REVIEW-RESULT>>>
{"schema":"yo.slice-review-result/v1alpha1","review_id":"<current review id>","candidate_commit":"<current candidate>","verdicts":[{"lens":"<requested lens>","verdict":"clear"}],"findings":[]}
<<<YO-SLICE-REVIEW-RESULT-END>>>
```

Each requested lens appears exactly once. A `findings` verdict requires at
least one bounded material finding naming that lens, a clear lens cannot be
named by a finding, and findings are empty exactly when every lens is clear.
The same instruction remains part of the immutable review plan visible to a
direct finding-resolution continuation. Frozen review-preparation v1alpha1
does not add this instruction and remains eligible only for the legacy gate
shape with coordinator-declared verdicts. Each preparation request version
emits its same-numbered result schema: v1alpha1 through v1alpha6 remain
distinct frozen boundaries.

The alternate target is
`{"kind":"delegated_host","host":"codex"}` or exact host `grok`, with an
optional absolute `session_repository_path`. It derives the frozen delegated
execution profile and v1alpha2 delivery shape. New v1alpha6 preparation uses
v1alpha4 admission for `host:grok`, which starts that exact read-only profile
with EOF stdin before packet construction and permits no prompt, ACP initialize,
Session, or Provider request. `host:codex` retains v1alpha3 state-ready
admission until an equivalent request-free profile probe is defined. Callers
do not supply Provider or Account coordinates for a delegated host.
An exact rerun reuses every file. Any byte-different existing request or any
claim/result already present in the delivery directory stops preparation
without overwriting or deleting it. Use the individual lower-level commands
for finding-resolution continuations, prospective activation, reproduction of
frozen schemas, or section-by-section packet diagnostics.

For a new review, v1alpha6 preserves v1alpha5's authority routing and
v1alpha4's Usage-bound delivery while adding Grok execution-profile admission.
It derives repository authority from the bound base-to-HEAD paths and requires
`repository_authority_policy:"changed-workflow-authority/v1alpha2"` and an
empty caller `repository_authority_paths`. Frozen v1alpha5 retains the same
authority behavior without the Grok startup probe:

- always include the small root `AGENTS.md` router and every changed nested
  `AGENTS.md` exactly;
- route packet construction, external-review delivery, and
  evidence/integration tooling to their direct owners under `CONTRIBUTING/`;
- route Slice and work-unit orchestration to root `CONTRIBUTING.md`;
- let neutral xtask facades inherit the owner established by concrete
  companion paths; and
- when a workflow-only facade has no companion, or shared workflow
  infrastructure is ambiguous, include all workflow owners rather than omit a
  possibly applicable rule.

Product-only candidates therefore keep only `AGENTS.md`, while a focused
workflow candidate carries only its directly applicable owner. The result uses
`yo.slice-review-prepare-result/v1alpha6`. Frozen v1alpha4 keeps
`changed-workflow-authority/v1alpha1`, which includes root `CONTRIBUTING.md`
for any workflow/tooling path; neither schema nor policy is reinterpreted.

The Usage-bound generated delivery request remains v1alpha4. It publishes a
separate `yo.external-review-provider-usage/v1alpha1` artifact, reopens the
durable Session, binds the exact delivery request identity and target, and
reports presence-aware input, output, total, reasoning, cache-read, and
cache-write values without guessing absent fields.

Start every new experimental wire schema, profile, identity domain, request,
result, manifest, or receipt family at `v1alpha1`. Do not publish a new shape as
`v1` merely because it has no predecessor or only repository-local consumers.
Promotion from `v1alphaN` to stable `v1` is a separate reviewed compatibility
decision after producer, verifier, failure ordering, and migration behavior are
accepted. Existing `v1` identifiers remain frozen and are not renamed. When an
existing family needs changed experimental semantics, preserve that `v1`
behavior and add the smallest unused `v1alphaN`; reserve a stable incompatible
successor such as `v2` for a later explicit promotion rather than using it as an
experimental starting point.

Treat every published wire identifier as a frozen behavior boundary, not only
as a serialization label. If validation or failure semantics change, keep the
old producer/verifier behavior in its existing version-owned module and add the
smallest required `v1alphaN` module; do not reinterpret the old identifier or
jump to v2 while the contract remains experimental. The facade and verifier
must dispatch explicitly from the recorded schema/profile, while shared
capture and rendering code stays version-neutral. A compatibility test must
replay a discriminating artifact accepted by the frozen version and show that
the new version applies only its own stronger rule.
