# Review and integration

This file is the repository workflow authority for Slice review, evidence,
approval, integration, and cleanup. It is linked from
[CONTRIBUTING.md](../CONTRIBUTING.md); the routing page does not duplicate
these rules.

Each Slice must include its implementation or docs, discriminating validation, public-contract updates, and known limits.

## Required lenses

Every Slice receives worker self-check and its required review lenses. The
slice planner proposes its risk and required lenses in the Slice Contract;
workers cannot lower them. Escalate risk when implementation reveals
public-contract, failure-behavior, or shared-ownership impact.

Fresh-context contract review is required for public contracts, terminal lifecycle, concurrency, failure behavior, workflow, and SOT changes. Slice integration review is required for Wave Slices and changes that consume shared interfaces or sibling results. A simple independent docs or configuration Slice may omit an additional lens only with a recorded rationale.

Every Slice that changes implementation code, executable validation code, or
Developer Docs theme source receives a code-quality review. This lens checks
responsibility and module boundaries, duplication, naming, unnecessary
abstraction, complexity, diagnostics, cleanup, and test maintainability. The
same fresh-context reviewer may perform both contract and code-quality review,
but must inspect and record the lenses separately.

## Agent review protocol

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
  "schema": "yo.slice-review-prepare-request/v1alpha2",
  "slice": "example-slice",
  "knowledge_ids": ["methexis.review.bounded-packet"],
  "context_max_tokens": 16000,
  "repository_authority_paths": ["CONTRIBUTING/review-and-integration.md"],
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

The v1alpha2 preparation appends one fixed output-contract instruction after
the caller's review questions. After any explanation, the reviewer must end
with exactly one terminal envelope and no trailing prose:

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
shape with coordinator-declared verdicts. The corresponding preparation result
schemas are v1alpha2 and v1alpha1 respectively.

The alternate target is
`{"kind":"delegated_host","host":"codex"}` or exact host `grok`, with an
optional absolute `session_repository_path`. It derives the frozen delegated
execution profile, v1alpha3 state-ready admission, and v1alpha2 delivery shape;
callers do not supply Provider or Account coordinates for a delegated host.
An exact rerun reuses every file. Any byte-different existing request or any
claim/result already present in the delivery directory stops preparation
without overwriting or deleting it. Use the individual lower-level commands
for finding-resolution continuations, prospective activation, reproduction of
frozen schemas, or section-by-section packet diagnostics.

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

When the human has granted a standing external-review authorization, check it
against the exact published packet before asking for another egress approval:

```json
{
  "schema": "yo.external-review-standing-authorization/v1",
  "authority": "human/<owner>",
  "status": "active",
  "routes": [
    {
      "provider": "<provider>",
      "account": "<account>",
      "model": "<model>",
      "max_packet_bytes": 1000000,
      "max_managed_payload_tokens": 200000,
      "allow_original_fresh": true,
      "allow_finding_resolution_resume": true
    }
  ]
}
```

Keep this human-origin record outside Git at the one common-workspace path
`<Git-common-dir-parent>/.local-exclude/authorizations/external-review.json`.
Every worktree reads that current canonical file; copies and caller-selected
paths are ineligible. Create, replace, activate, or revoke it only from an
explicit human statement that names the exact routes and limits; a
Slice disposition, `go`, earlier delivery, or agent-authored proposal is not
standing egress authority. Removing the file or changing `status` from
`active` revokes it. The v1 semantics always exclude reviewer tool execution,
retry, steer, fallback, a second provider, and every additional provider
request. They permit at most one original packet in a fresh Session and one
direct finding-resolution packet resumed in that same reviewer Session. A
second delta, changed lens or scope, replacement route, unavailable-provider
substitution, or larger packet requires a new explicit human decision.

Bind one published manifest, the exact authorization bytes, route, and Session
mode in a transient request:

```json
{
  "schema": "yo.slice-review-egress-request/v1",
  "manifest_path": ".local-exclude/methexis/slice-reviews/<id>/manifest.json",
  "manifest_hash": "sha256:<manifest-hash>",
  "authorization_hash": "sha256:<authorization-hash>",
  "route": {
    "provider": "<provider>",
    "account": "<account>",
    "model": "<model>"
  },
  "session": {"mode": "fresh"}
}
```

After the original provider request durably starts, the coordinator records
the exact identity observed from Yo's durable StartTurn and Session evidence
before preparing a finding-resolution request:

```json
{
  "schema": "yo.external-review-delivery-receipt/v1",
  "review_id": "sha256:<original-review-id>",
  "packet_hash": "sha256:<original-packet-hash>",
  "route": {
    "provider": "<provider>",
    "account": "<account>",
    "model": "<model>"
  },
  "session_id": "<returned-reviewer-session>",
  "provider_request_id": "<returned-request-identity>",
  "provider_request_count": 1
}
```

This receipt is a local operational assertion, not authority. The preflight
proves only its exact bytes and internal consistency; it cannot authenticate
Provider provenance or prove the request count. The coordinator owns comparing
the fields with Yo's durable evidence and must not create the receipt before a
provider request starts. Bind its path and exact hash as `prior_delivery` in
the delta egress request, and use
`{"mode":"resume","id":"<same-reviewer-session>"}` only for that one direct
finding-resolution packet. The preflight rejects a different route, Session,
ReviewId, packet hash, absent request identity, or request count other than
one. Then run:

```bash
cargo xtask slice review-egress <request.json>
```

The command replays the complete original-or-delta manifest chain, verifies
the immutable packet, exact human-origin authorization hash, route, Session
mode, prior delivery receipt when required, byte and token limits, then replays
the complete chain again for final input and trusted-Git stability. It performs
no network or provider operation. `next_action: "deliver_once"` removes a
repeated human prompt only when the coordinator also observes that this exact
ReviewId, route, and Session step has no prior provider request. It is an
eligibility result, not a reusable delivery receipt: after a provider request
starts, record the returned Session/request evidence and never interpret
another preflight run as permission to resend. A delivery attempt that ended
before a durable provider request remains a delivery-system diagnostic and is
not silently retried under this authorization.

Before publishing an immutable delivery claim, inspect the selected target
through one provider-neutral, read-only admission request:

```json
{
  "schema": "yo.external-review-target-admission-request/v1alpha1",
  "target": {
    "kind": "managed_model",
    "provider": "<provider>",
    "account": "<account>",
    "model": "<model>"
  },
  "connection_repository_path": "/absolute/path/to/connections.yaml",
  "session_repository_path": "/absolute/path/to/sessions"
}
```

```bash
cargo xtask slice review-target-admission <request.json>
```

The alternate target shape is
`{"kind":"delegated_host","host":"codex"}` or exact host `grok`; it omits
`connection_repository_path`. Use request v1alpha2 or v1alpha3 for that delegated target;
v1alpha1 remains a frozen preparation-only probe. Managed admission proves that the exact stored
binding exists and reports its newest typed `last_failure`. Authentication,
access-denied, exact-model-unavailable, and local-configuration observations
stop admission. Other failure kinds remain visible but do not become inferred
quota exhaustion or a routing prohibition. A delegated-host admission runs
only the bounded exact executable `--version` probe and records its canonical
path and version; it does not authenticate the host or make a model request.
Frozen v1alpha1 keeps its successful delegated result as `prepared` with
`next_action: "await_delegated_delivery_protocol"`. The new
`yo.external-review-target-admission-request/v1alpha2` preserves managed
behavior and returns `eligible` with `next_action: "deliver_delegated_once"`
for an admitted host. This selects the separate delegated protocol below; it
never makes the host eligible for managed `deliver_once`.

Prefer `yo.external-review-target-admission-request/v1alpha3` for a new
delegated delivery. It preserves the version probe and additionally creates,
writes, and removes one unique sentinel in the existing host state directory
(`$HOME/.codex` or `$HOME/.grok`) before any delivery claim. This request-free
probe catches a read-only or missing host state mount that a version-only probe
cannot see. A successful probe leaves no file and still proves neither account
entitlement nor quota.

The optional Session repository search reads at most the newest 64 Sessions
and returns the latest matching receipt in the first most-recently-updated
matching Session. The result names that selection basis and reports truncation
or any unavailable or unreadable history in the inspected window as `unknown`;
it never calls that bounded observation
a global account total. `account_limit` and its reset remain `unknown` until a
Provider supplies reviewed typed evidence. In particular, Session token totals
and cache counts never become remaining quota.

New managed original and continuation deliveries bind that exact request with
`yo.slice-review-delivery-request/v1alpha2` or
`yo.slice-review-continuation-delivery-request/v1alpha2` by adding
`admission_request_path` and `admission_request_hash`. Delivery evaluates the
same request before and after preparing current-develop Yo, requires its target
to equal the authorized route, and stops before claim when the result is not
admitted or changed. Its v1alpha2 claim records the exact admission-request
hash and typed target. Frozen v1alpha1 delivery remains reproducible and does
not acquire this stronger pre-claim behavior. A delegated-host admission
cannot make any managed route schema launch a host.

Keep delegated-host authority in the separate common-workspace file
`.local-exclude/authorizations/external-review-delegated.json`. Create or
replace it only from a human statement naming the exact hosts and limits:

```json
{
  "schema": "yo.external-review-delegated-authorization/v1alpha1",
  "authority": "human/<owner>",
  "status": "active",
  "targets": [
    {
      "host": "codex",
      "execution_profile": "yo.delegated-review-execution/v1alpha1",
      "max_packet_bytes": 4000000,
      "max_managed_payload_tokens": 500000,
      "allow_original_fresh": true,
      "allow_finding_resolution_resume": true
    }
  ]
}
```

New standing authority should use
`yo.external-review-delegated-authorization/v1alpha2` and replace the two
request-kind booleans with these three explicit fields:

```json
{
  "max_original_fresh_requests": 1,
  "max_finding_resolution_resume_requests": 1,
  "max_total_requests": 2
}
```

Each per-kind value is `0` or `1`, and their sum must equal the total. This
makes the human authorization, egress classifier, and exact one-original plus
one-direct-resolution chain state the same round limit. Frozen v1alpha1 keeps
its boolean meaning.

The closed target set is `codex` and `grok`, so this authorization accepts at
most two unique entries. It never names Provider, Account, or downstream model
coordinates. Bind one exact review manifest, authorization revision, target,
profile, and Session mode in a delegated egress request:

```json
{
  "schema": "yo.slice-review-delegated-egress-request/v1alpha1",
  "manifest_path": ".local-exclude/methexis/slice-reviews/<id>/manifest.json",
  "manifest_hash": "sha256:<manifest-hash>",
  "authorization_hash": "sha256:<authorization-hash>",
  "target": {"kind": "delegated_host", "host": "codex"},
  "execution_profile": "yo.delegated-review-execution/v1alpha1",
  "session": {"mode": "fresh"}
}
```

`cargo xtask slice review-egress <request.json>` replays the same immutable
review chain and limits as managed egress, but returns
`deliver_delegated_once`. Its limit record uses `host_requests` and
`target_switch`; it deliberately has no `tool_execution: false` claim because
read-only host tools remain owned by Codex or Grok.

For every managed or delegated delivery, name one exact output child below the
active Slice's shared coordination directory. Integrated `review-prepare`
creates and checks that child before publishing its frozen v1alpha2 delivery
request. For an individually assembled lower-level delivery, use the new
managed or delegated original or continuation request `v1alpha3`. Before any
claim or external request, alpha3 creates the final directory when only it is
missing, then verifies that the resolved path remains inside the Slice, is a
real empty directory, and accepts a create-and-remove write probe. It never
creates missing parent directories. A symlink, non-empty path, unwritable
directory, or path outside the Slice fails as local preparation and consumes
no external request. Frozen v1alpha1 and v1alpha2 delivery requests retain
their existing-directory precondition and failure order. Alpha3 normalizes to
the existing alpha2 claim, outcome, result, and finalization artifacts after
this new local pre-claim step; finalization never creates an output directory.

For a fresh delegated review, bind that egress and an exact v1alpha2 host
admission to one empty output directory:

```json
{
  "schema": "yo.slice-review-delegated-delivery-request/v1alpha1",
  "egress_request_path": ".local-exclude/coordination/<slice>/egress.json",
  "egress_request_hash": "sha256:<egress-request-hash>",
  "admission_request_path": ".local-exclude/coordination/<slice>/admission.json",
  "admission_request_hash": "sha256:<admission-request-hash>",
  "output_directory": ".local-exclude/coordination/<slice>/delivery"
}
```

Frozen delivery request `v1alpha2` with admission `v1alpha3` requires the
host-state readiness proof and records alpha2 claim/result schemas.
Individually assembled new work uses delivery request `v1alpha3` with the same
admission and artifact schemas plus the output preparation step above.
`v1alpha1` continues to require only frozen admission v1alpha2 eligibility and
neither older request is silently strengthened.

The repository delivery command evaluates egress and admission twice around
the exact current-develop build, publishes an immutable delegated claim, then
launches exactly one `yo -p --model host:<host> --sandbox read-only`. It
requires one byte-identical `StartTurn`, the host-specific alpha binding with
the exact execution profile, one accepted durable request, and one resumable
outcome. That identity is recorded as `host_request_id`, never as a Provider
request. A successful run publishes:

```json
{
  "schema": "yo.external-review-delegated-delivery-receipt/v1alpha1",
  "review_id": "sha256:<review-id>",
  "packet_hash": "sha256:<packet-hash>",
  "target": {"kind": "delegated_host", "host": "codex"},
  "execution_profile": "yo.delegated-review-execution/v1alpha1",
  "session_id": "<reviewer-session>",
  "host_request_id": "<durable-host-request-identity>",
  "host_request_count": 1
}
```

Provider request identity and token or cache usage remain unknown unless the
host publishes independently reviewed exact evidence. Claim reuse, retry,
steer, fallback, target switch, and a second host request are forbidden.

If the launched process succeeded and the exact durable Session later becomes
observable but the original observation failed before `delivery.json` was
published, recover only the receipt:

```bash
cargo xtask slice review-deliver finalize <finalize-request.json>
```

```json
{
  "schema": "yo.slice-review-delegated-delivery-finalize-request/v1alpha1",
  "delivery_request_path": ".local-exclude/coordination/<slice>/delivery-request.json",
  "delivery_request_hash": "sha256:<exact-request-hash>"
}
```

The `yo.slice-review-delegated-delivery-finalize-request/v1alpha1` input binds
only `delivery_request_path` and `delivery_request_hash`. The command replays
authorization, admission, immutable claim, process outcome, published review
and diagnostic hashes, and the exact Session trace. It launches no Yo, host,
or Provider process and records `provider_requests:0` and `host_requests:0` in
an immutable `finalization.json`. A failed process, changed artifact, absent
request, extra request, or wrong binding remains unrecoverable; never use
finalization as retry authority.

Before any authorized finding-resolution resume, create the repository-owned
read-only continuation preflight request against the exact durable Session
repository:

```json
{
  "schema": "yo.slice-review-continuation-preflight-request/v1alpha1",
  "egress_request_path": ".local-exclude/coordination/<slice>/delta-egress.json",
  "egress_request_hash": "sha256:<exact-egress-request-hash>",
  "session_repository_path": "/absolute/path/to/the/reviewer-session-repository"
}
```

```bash
cargo xtask slice review-continuation-preflight <request.json>
```

The command replays the finding-resolution egress authorization, then reads
the named repository through `yo-core`. It requires exactly one StartTurn whose
bytes hash to the prior original packet, exactly one matching managed
Provider/Account/Model binding, exactly one accepted request and resumable
outcome resolving to the prior delivery receipt's request identity, and a
typed executable continuation with the newest durable Continuation Anchor. A
missing or malformed Session, mismatched route or identity, extra request, or
missing Anchor fails before launch. The successful result records the
exact Session, route, candidate, request identity, binding epoch, and Anchor
sequence, but publishes no artifact, acquires no terminal, and performs no
Provider request. Bind that exact request, the exact current target-admission
request, and one new output directory in the continuation delivery request:

```json
{
  "schema": "yo.slice-review-continuation-delivery-request/v1alpha3",
  "preflight_request_path": ".local-exclude/coordination/<slice>/continuation-preflight.json",
  "preflight_request_hash": "sha256:<exact-preflight-request-hash>",
  "admission_request_path": ".local-exclude/coordination/<slice>/admission.json",
  "admission_request_hash": "sha256:<exact-admission-request-hash>",
  "output_directory": ".local-exclude/coordination/<slice>/continuation-delivery"
}
```

Run the same repository delivery command once from the clean candidate
worktree:

```bash
cargo xtask slice review-deliver <continuation-delivery-request.json>
```

The command evaluates the preflight before and after building the exact
current-integration `yo`, requires that the authority and stored Session did
not change, then publishes an immutable continuation claim before launching
one `yo -p --resume <same-session>` process. It pipes only the authorized delta
packet to stdin and never pastes into or reads a terminal. After completion it
requires the unchanged prior Turn and Provider identity, exactly one new
byte-identical StartTurn, one new accepted request and resumable outcome, the
same binding epoch, and a newer durable Continuation Anchor. It then publishes
the same bounded review, diagnostic, outcome, and delivery-receipt artifact
roles used by original delivery.

Delegated finding-resolution uses the parallel experimental schemas
`yo.slice-review-delegated-continuation-preflight-request/v1alpha1` and
`yo.slice-review-delegated-continuation-delivery-request/v1alpha1`. The latter
also binds the exact delegated admission request. Its checks use the same
Session and execution profile and record `prior_host_request_id`; the resumed
launch is exactly `yo -p --resume <same-session>`. The successful receipt keeps
the delegated shape above. It does not reinterpret any managed continuation
artifact or claim Provider visibility.

The preflight and the checks before claim publication are current eligibility,
not retry authority. No other process may write the reviewer Session during
delivery. Once the continuation claim exists the attempt is consumed, even if
process or observation fails; inspect the bounded outcome and request new human
authority rather than deleting the claim or retrying. The path never creates a
fresh Session, switches binding, retries, steers, falls back, invokes a second
Provider, or enables tools.

For one original packet in a fresh Session, perform the authorized effect with
the bounded repository delivery command instead of terminal paste, pane
capture, or direct Session JSONL inspection. Bind the exact egress and current
target-admission requests plus one new output directory under the active
Slice's shared coordination directory in a new experimental request:

```json
{
  "schema": "yo.slice-review-delivery-request/v1alpha3",
  "egress_request_path": ".local-exclude/coordination/<slice>/egress.json",
  "egress_request_hash": "sha256:<egress-request-hash>",
  "admission_request_path": ".local-exclude/coordination/<slice>/admission.json",
  "admission_request_hash": "sha256:<exact-admission-request-hash>",
  "output_directory": ".local-exclude/coordination/<slice>/delivery"
}
```

Run it once from the clean candidate worktree:

```bash
cargo xtask slice review-deliver <delivery-request.json>
```

The command replays `review-egress` authorization before and after preparing
the runtime, resolves the sole checked-out clean integration worktree at the
review manifest's trusted commit (including absence of non-ignored untracked
files), and builds that exact current-integration `yo`. It
then publishes an immutable `yo.external-review-delivery-claim/v1alpha1`
before process launch and pipes the already verified packet bytes directly to
one `yo -p --model <provider>:<account>:<model> --no-tools` process with a
30-minute outer deadline. It uses an isolated durable Session repository
inside the output directory and writes bounded `review.txt` and
`diagnostic.txt` files rather than copying packet, terminal, or raw Session
content into coordinator context.

After process completion, the command reads that isolated repository through
`yo-core`, requires exactly one byte-identical packet `StartTurn`, the exact
authorized managed Provider/Account/Model binding, one durable accepted
request, and one resumable Provider outcome. An outcome without its own stable
identity uses that accepted request identity, as required by the durable
Session contract. After a claim, setup, spawn, write, exit, timeout, capture,
publication, and durable-observation failures are folded into a compact
`yo.external-review-delivery-outcome/v1alpha1` whenever that output can still
be published; each result or diagnostic artifact says whether its bytes were
published. Only the fully successful path also publishes the frozen
`yo.external-review-delivery-receipt/v1` consumed by finding-resolution and
Slice-gate preparation, then returns `next_action: "interpret_review"`.

The claim is intentionally not idempotent: once it exists, the same output
directory can never authorize another launch, even when the process failed,
the terminal disappeared, or durable request evidence is incomplete. Inspect
the bounded outcome and request new human authority for any replacement; never
delete or rename the claim to manufacture a retry. Build and preflight failures
that occur before claim publication perform no Provider effect and may be
corrected normally. The managed v1alpha1 request schemas separately cover an original
packet in a fresh managed-model Session and an authorized finding-resolution
delta in its exact stored Session. Delegated hosts use only the disjoint alpha
schemas above. Retry, steer, fallback, a second target request, and managed
tool-execution claims remain outside both effects.

Because a self-hosting diff may contain the wrapper's own sentinel source,
v1alpha2 section metadata names its reversible sentinel-escape profile. The
encoding doubles literal backslashes and replaces the first less-than byte of
a literal wrapper-sentinel prefix with ASCII `\x3c`; hashes and byte counts
continue to bind the decoded original. This keeps in-body source text from
being mistaken for a real section or payload boundary.

Use a separate fresh-context Codex session by default. Use a configured
different-perspective provider when the lens needs hidden assumptions,
counterexamples, alternatives, or future costs; require its strongest
counterargument before the verdict. A provider is configured only after the
human selects it and local guidance defines invocation, stable identity, setup
failure, and unavailability. Kimi is the current preferred profile, not a
product dependency. Do not invent commands for an unconfigured provider.

Run a Kimi review from the Slice worktree in a new non-interactive prompt
session. The sentinel prevents command substitution from stripping the
packet's trailing newlines and is removed before invocation:

```bash
review_payload="$(cat <packet.md>; printf x)"
kimi -p "${review_payload%x}"
```

Every reviewer receives the immutable `<base>..<candidate>` diff, relevant
authority, requested lens, and validation evidence. Commit the complete
candidate and require a clean worktree; dirty or inferred review surfaces do
not count. Reviewers may rerun a targeted check to resolve a finding or named
uncertainty, but do not repeat a supplied green baseline when its inputs are
unchanged. If a finding changes the candidate, validate the affected boundary
and review the new commit again. When the lens, scope, and reviewer are
unchanged, resume that exact reviewer session and provide the prior candidate
and ReviewId, replacement candidate, exact delta bytes or their
content-addressed artifact, finding dispositions, and the unchanged evidence
the reviewer may reuse. The coordinator validates command syntax, artifact
identity, and supplied green checks before delivery; do not make the reviewer
rediscover those operational facts.

Build that provider-neutral continuation payload from the prior immutable
manifest and a clean replacement `HEAD`:

First record the exact finding set returned by that reviewer. This artifact is
reviewer-authored evidence, not a worker-selected subset:

```json
{
  "schema": "yo.slice-review-findings/v1",
  "review_id": "sha256:<prior-review-id>",
  "candidate_commit": "<prior-candidate-commit>",
  "findings": [
    {"finding_id": "F1", "summary": "The ambiguous state is accepted."}
  ]
}
```

```json
{
  "schema": "yo.slice-review-delta-request/v1alpha2",
  "prior_manifest_path": ".local-exclude/methexis/slice-reviews/<review-id>/manifest.json",
  "prior_manifest_hash": "sha256:<manifest-hash-from-review-packet-result>",
  "prior_findings_path": ".local-exclude/coordination/<slice>/review-findings.json",
  "prior_findings_hash": "sha256:<exact-findings-file-hash>",
  "finding_dispositions": [
    {
      "finding_id": "F1",
      "disposition": "resolved",
      "summary": "The replacement candidate now rejects the ambiguous state."
    }
  ],
  "reused_validation_evidence": ["unchanged-baseline"],
  "affected_validation_evidence": [
    {"name": "focused-finding-check", "path": "/tmp/focused-check.txt"}
  ],
  "delivery_profile": "yo.slice-review-delta-markdown/v1alpha1",
  "tokenizer_profile": "o200k_base/v1",
  "max_managed_payload_tokens": 12000
}
```

```bash
cargo xtask slice review-delta <request.json>
```

The new request `yo.slice-review-delta-request/v1alpha2` fully reproduces the
prior packet and manifest from their captured
inputs before accepting them; the prior manifest may be either the original
review or the latest verified review delta in the same chain. It then requires dispositions (`resolved`,
`not_reproduced`, or `accepted_limit`) for every and only prior finding ID,
requires at least one replacement-specific affected validation item, requires
every prior validation item to be classified as reused or affected, verifies
reused bytes, requires every affected structured summary's internal name to
equal its request name and every affected evidence body to name the exact
replacement commit, and enforces a cumulative evidence bound while capturing.
Those identity checks finish before immutable delta publication, so correct a
summary-name alias or stale candidate in the evidence-producing command rather
than publishing a packet that the later gate cannot consume.
Frozen request v1alpha1 retains its prior capture and failure order and does
not acquire the structured-summary identity precheck. Both request versions
publish the existing canonical v1alpha1 delta packet family; the captured
request bytes keep their producer identity distinct.
Store each candidate's evidence at a new immutable path; overwriting evidence
referenced by an earlier review makes that chain head ineligible. The command
captures the no-renames binary prior-to-replacement diff and returns one
content-addressed `packet.md` plus manifest. Deliver those packet bytes
unchanged to the recorded reviewer session. This command does not start or
select a provider.

Ordinary coordination follows `cargo xtask slice status <slice>`: when it
returns `review_delta`, use the latest applicable manifest as the prior chain
head and do not publish another full base-to-candidate packet. Repeating
the same delta request returns the already published content-addressed artifact
as `reused`; repeating it is not another review or Provider request.

Before replaying a published original review, its verifier validates every
manifest Git revision without invoking Git. It then requires the base and
candidate fields to name those exact commit objects and requires the base to
be an ancestor of the candidate before reading ContextBuild, authority,
validation, contract, or diff inputs. Malformed, tag-object, missing-object,
and unrelated-history identities therefore fail before input replay.

New continuation requests use experimental
`yo.slice-review-delta-markdown/v1alpha1`, which compares affected evidence by
canonical filesystem identity. Published delta-v1 manifests retain their
frozen path-string transition rule so an old alias-shaped chain remains
reproducible; accepting that legacy artifact does not permit a new v1 request.
The verifier selects these semantics from the exact published manifest schema.

The continuation payload and question are bounded. Policy does not impose a
finding-resolution round count, but the verifier has a 64-hop safety limit; at
that boundary, start a fresh review instead of extending the chain. Keep
reusing the session while it can identify the prior review and the
remaining work without broad reconstruction. Start a compact fresh session
when the lens or scope changes, the reviewer is unavailable, exact context is
lost, the reviewer begins broad repository or documentation reinspection, or
the next finding introduces a new design question instead of resolving the
reviewed one. Repeated tool calls that mostly reconstruct supplied evidence are
a signal that accumulated session context is no longer helping. A resumed
reviewer that cannot identify the prior packet fails closed and does not count
as completed review.

For Codex CLI review, resume the recorded session through its supported
non-interactive surface:

```bash
codex exec resume --json <session-id> - < <delta-review-packet.md>
```

Do not return a large reviewer's prompt echo or event stream to the coordinating
agent context. For full and delta packets, direct complete process output to a
task-specific local log, request the final message in a separate file when the
reviewer supports it, and read back only the exposed session identity, terminal
status, and final response. Preserve the exact reviewer-authored finding set
and verdict when findings exist so a later review delta can reproduce them;
discard prompt echoes and event or tool streams. The immutable packet and
manifest already own the review input, so copying them through orchestration
output wastes context and can terminate an otherwise valid review run. Preserve
the local log only while a finding remains unresolved.

For Kimi, use plain `kimi -p`: never `--continue` or `--session`. Omit `--model`
unless its exact configured CLI alias is known, and consult `kimi --help`
instead of guessing flags. Require empty `git status --short
--untracked-files=all` output before and after; reviewer-created changes
invalidate the attempt. Invalid options and aliases are setup failures to fix,
not unavailability. Record the completion hint's session as
`kimi/<session-id>`; without that hint, do not invent an identity or count the
attempt as complete.

For every completed agent review, record the lens, actual provider, exposed
model and session identity, and verdict in the Slice status or handoff; report
the actual provider and model at close. Record missing identifiers rather than
guessing. If a correctly invoked preferred provider cannot finish because the
service or allowance is unavailable, record the requested provider and reason,
then a separate fresh-context Codex session may perform the same lens. Do not
retry an unavailable provider until its state changes. Human exact review is
also valid; the implementing session's self-check is not.

When the provider exposes them, also record managed packet tokens, model-call
and reviewer-tool-call counts, cumulative input, cached input, output, and the
number of finding-resolution rounds. Managed packet size is not a proxy for
the total work of a tool-using review session. Use these measurements to compare
Slice workflows and accepted outcomes, not as a reason to weaken a required
lens or optimize token count in isolation.

If no reviewer completes a required lens, mark it **unreviewed**, record each
attempt and reason, notify the human, and stop before acceptance. Quota
exhaustion, partial responses, self-checks, and failed attempts are not review
trailers.

## Evidence and disposition

The accepted commit records completed lenses as evidence, not as a substitute
for performing them. Put every trailer in one contiguous block at the very end
of the commit message, with no blank lines between trailers and no body text
after them. For example, when all three review lenses apply, the final block is:

```text
Slice-Review: fresh-context - completed - codex/7f3a91 - clear
Slice-Review: code-quality - completed - kimi/2b8c44 - resolved
Slice-Review: integration - completed - human/minseo - clear
```

Use `clear` when the completed review found no actionable findings. Use
`resolved` only after findings were addressed and the same lens re-reviewed the
final diff with no remaining actionable finding. Reviewer IDs are compact
tokens such as `kimi/session-id`, `codex/session-id`, or `human/name`; put
operational detail in the Slice status rather than free text in the trailer.

Accepted commits prepared from review-coverage cutover
`edf376fd33dc10e8fa3e02ca0e4543025249838a` or its descendants also record one
exact ledger entry for every completed lens:

```text
Review-Coverage: fresh-context - exact - model-high/codex/gpt-5.6-sol/session-id - sha256:<canonical-diff>
Review-Coverage: code-quality - exact - model/codex/gpt-5.6-luna/session-id - sha256:<canonical-diff>
Review-Coverage: fresh-context - exact - delegated-high/codex/session-id - sha256:<canonical-diff>
```

`model-high/<provider>/<model>/<session>` records an independently selected
high-capability model and is required for model-performed fresh-context and
integration review. Mechanical code-quality review may use
`model/<provider>/<model>/<session>`. The provider and session must reproduce
the compact `<provider>/<session>` identity in the matching `Slice-Review`.
This records the actual route without hard-coding one vendor as policy.
`delegated-high/<host>/<session>` and `delegated/<host>/<session>` record the
same lens classes for a host-owned review whose downstream Provider and model
are not visible to Yo. They reproduce `<host>/<session>` in `Slice-Review` and
must not invent downstream coordinates.

A person may perform any lens instead:

```text
Slice-Review: fresh-context - completed - human/yon - clear
Review-Coverage: fresh-context - exact - human/yon - sha256:<canonical-diff>
```

Human coverage means that named person inspected the exact patch for that
lens and explicitly returned the recorded verdict. A general `go`, standing
authorization, integration approval, or review of an earlier candidate is not
human review evidence. Human and model review use the same exact-diff gate;
neither is a bypass for the other validation requirements.

The ledger hash is SHA-256 of the canonical binary, full-index, no-renames
base-to-candidate diff carried as `git_diff` by the immutable review packet.
At accepted squash time, commit preflight recomputes the same bytes from the
integration `HEAD` and staged index. Slice close recomputes them from the
accepted commit and its first parent. A changed integration surface therefore
requires review again instead of inheriting an earlier verdict. Commits at or
before the cutover retain their historical evidence and are not backfilled.

After that cutover, accepted integration branches do not use
`git commit --amend`, `-c`, or `-C`: those operations can combine an earlier
accepted surface with only an incremental staged diff. Working `slice/`, `task/`, and
`spike/` branches may still rewrite their disposable candidate history. When
an accepted change needs correction, prepare the complete message in a file
and create a new commit with `cargo xtask slice commit <message>` so both
preflight and Slice close observe the same first-parent surface. The command
invokes a non-amend Git commit through the ordinary editor boundary; the
`prepare-commit-msg` hook rejects ambiguous `-m`, `-F`, `-t`, `-c`, `-C`, and
`--amend` operations before the message is edited. A person may instead use
plain `git commit` without a configured message template, read the complete
message in the editor, and accept it there.

When no additional lens applies, record `Slice-Review: none - <reason>`.
`cargo xtask check slice-review-impact` reads the final trailer block and fails
closed when the disposition is missing or malformed. It requires
fresh-context review for product/shared/tool code, build/Cargo metadata, workflow, and
semantic SOT; code-quality for executable `crates/`, `shared/`, and `tools/` source plus
Developer Docs theme source; and integration review on Wave branches. This is
a minimum path-based safety net, so the planner adds lenses required by semantic
impact. `none` cannot accompany completed lenses, and unfinished or unresolved
reviews never satisfy a lens. Existing accepted commits keep their historical
trailers; new commits and amends use this grammar.

Classify a Slice as **human-attention** when it introduces or changes a product
decision, public contract, failure semantics, dependency choice, permissions,
destructive or external effect, workflow authority, or semantic SOT authority;
when required review has an unresolved finding; when validation is not green;
or when material uncertainty remains. Uncertain classification is
human-attention. It requires explicit human approval for that exact Slice.

A Slice is **routine** only when it implements an already approved exact
contract or performs mechanical repository follow-through, introduces none of
the human-attention conditions, has all declared dependencies accepted and
integrated, passes all required validation, and has no unresolved finding from
any review. Under a standing human authorization, the slice planner may
auto-integrate a routine Slice and MUST record the classification rationale
plus the authorization's human origin and scope in the integration report or
accepted commit. Generated projections, fixtures, approval records, and
immutable Checkpoints are routine only when they preserve an already reviewed
exact revision and add no semantic choice. Selecting roots or changing the
active Checkpoint is always human-attention. Explicit human approval that
already includes the exact activation transition satisfies that Slice's
disposition; do not request a second merge approval. If the standing
authorization is absent or revoked, routine Slices also require explicit human
approval.

## Integration and cleanup

Run the declared baseline on the reviewed candidate. After squash, reuse its
result only under the exact conditions in the Developer Docs
[Slice-close baseline](../docs/src/validation/README.md#slice-close-baseline);
otherwise rerun the affected gates and lenses. Evidence reuse never replaces
candidate validation or fresh-context review.

After the Slice's review disposition is satisfied, squash a direct Slice into
`develop`:

```bash
git switch develop
git merge --squash slice/direct/<slice>
git commit
```

Squash an accepted Wave Slice into its Wave:

```bash
git switch wave/<wave>
git merge --squash slice/<wave>/<slice>
git commit
```

The resulting commit is the durable review unit; Task commits are not preserved. A Wave keeps one commit per accepted Slice and must remain green after each one. Handle integration fixes as separately reviewed Slices.

After the accepted commit exists, close its local Slice in two explicit steps
from the integration worktree:

Prefer deriving
`.local-exclude/coordination/<slice>/close-metrics.json` from the same ready
gate after the accepted commit exists:

```bash
cargo xtask slice close prepare \
  .local-exclude/coordination/<slice>/close-prepare.json
```

The experimental `yo.slice-close-prepare-request/v1alpha1` input names the
Slice and gate request, then records only data the gate cannot know: execution
lanes, review rounds and finding dispositions, review-packet totals, the
elapsed bottleneck, and an optional one-to-one command mapping for each known
unverified environment. Relative `gate_request_path` values resolve from the
shared workspace root. The command finds the already accepted matching patch,
re-evaluates the ready gate in the registered Slice worktree, derives the exact
candidate and accepted commit, and copies the gate-verified validation names,
argv, status, and reuse disposition. It atomically publishes only the standard
metrics file and never commits, plans, or cleans up.

```json
{
  "schema": "yo.slice-close-prepare-request/v1alpha1",
  "slice": "example",
  "gate_request_path": ".local-exclude/coordination/example/gate.json",
  "execution_lanes": [{
    "lane": "integration",
    "mode": "serial",
    "operation_count": 1,
    "max_concurrency": 1
  }],
  "review": {
    "rounds": 1,
    "findings": {
      "reported": 0,
      "resolved": 0,
      "not_reproduced": 0,
      "accepted_limits": 0,
      "remaining": 0
    }
  },
  "review_packets": {
    "publication_count": 0,
    "total_managed_tokens": 0,
    "largest_sections": [],
    "reused_inputs": []
  },
  "unverified_validation": [],
  "elapsed_bottleneck": {
    "name": "independent review",
    "elapsed_milliseconds": 45000
  }
}
```

Writing the complete standard file directly remains supported. Its shape is:

```json
{
  "schema": "yo.slice-close-metrics/v1",
  "slice": "example",
  "slice_candidate": "<full-Slice-HEAD>",
  "accepted_commit": "<full-accepted-commit>",
  "execution_lanes": [
    {
      "lane": "cargo_validation",
      "mode": "serial",
      "operation_count": 3,
      "max_concurrency": 1
    },
    {
      "lane": "integration",
      "mode": "serial",
      "operation_count": 1,
      "max_concurrency": 1
    }
  ],
  "review": {
    "rounds": 2,
    "findings": {
      "reported": 1,
      "resolved": 1,
      "not_reproduced": 0,
      "accepted_limits": 0,
      "remaining": 0
    }
  },
  "review_packets": {
    "publication_count": 2,
    "total_managed_tokens": 32000,
    "largest_sections": [
      {
        "kind": "git_diff",
        "name": "base-to-candidate",
        "rendered_bytes": 24000,
        "rendered_tokens": 6000
      }
    ],
    "reused_inputs": ["context-build/sha256:<content-id>"]
  },
  "validation": [
    {
      "name": "workspace-tests",
      "argv": ["cargo", "test", "--workspace", "--all-targets"],
      "runs": 1,
      "status": "passed",
      "reused": false
    }
  ],
  "elapsed_bottleneck": {
    "name": "independent review",
    "elapsed_milliseconds": 45000
  },
  "known_unverified_environments": []
}
```

Lane names are `discovery`, `editing`, `review`, `cargo_validation`, and
`integration`; modes are `parallel` and `serial`. Record only lanes that ran,
but always record integration. Cargo-heavy validation and shared integration
must be serial with `max_concurrency: 1`; other lanes report their actual mode.
Validation status is `passed` or `unverified`; an unverified item has zero runs,
cannot be reused, and names its missing environment in
`known_unverified_environments`. Human review may legitimately have zero
published packets. Packet measurements are compact close diagnostics, not a
replacement for their immutable manifests.

The close planner requires this standard file, checks internally reconcilable
counts and exact candidate/accepted-commit identity, and binds its path and
hash into the plan. Apply revalidates the same bytes before cleanup. These
checks prevent stale, contradictory, or silently edited records; they do not
prove that the reported operations ran. Root self-check and completed review
still own factual accuracy. Delete the local metrics with other completed Slice
coordination after promoting any aggregate lesson to its proper owner.

```bash
cargo xtask slice close plan <slice> /tmp/<slice>-close.json
cargo xtask slice close apply /tmp/<slice>-close.json
```

Review the directly published, hash-addressed plan; do not copy, reserialize, or
edit it. Store it outside the worktree and Slice coordination directory it
closes. `plan` requires clean
integration and Slice worktrees, the bound contract, accepted review evidence,
and exact Slice/accepted-commit patch identity. It fixes refs, paths, binding,
effects, and the coordination entries covered by cleanup. Generate a fresh plan
if integration advances. Newly generated plans use the experimental
`yo.slice-close-plan/v1alpha1`. A v2 or v3 plan remains resumable under its original
identity and safety checks only when its accepted commit predates the tracked
close-metrics cutover marker; v4 keeps its metrics-bound retained-entry
semantics. Commits containing that marker require v4 or newer even if a caller
rewrites and rehashes a legacy-shaped plan.

`apply` revalidates that state and the v1alpha1 cleanup-entry list, then removes the
registered worktree and binding, the complete standard
`.local-exclude/coordination/<slice>` directory, and the expected local Slice
branch. Slice coordination is temporary: promote stable decisions first and
move genuinely unresolved work to the local backlog before planning close.
The plan binds at most 256 recursively enumerated cleanup paths and apply
rejects any added or removed entry before deletion.
Nonstandard contract paths remain preserved under their own owner. The entire
worktree, including ignored output, is removed, so move retained material
first. A repository lock, final
worktree check, and compare-and-swap ref transaction protect cooperating
applies; raw Git does not use the lock. A stale plan fails closed, while an
interrupted apply can resume the planned contract or branch deletion.

## Wave promotion

Never rebase accepted Slice commits. Parallel sibling Waves may finish in any order, but promotion into `develop` is serialized. Before promotion, a Wave based on an older `develop` merges the latest `develop` into the Wave:

```bash
git switch wave/<wave>
git merge develop
```

If that merge exposes a contract conflict, stop and reconcile the owning SOT before code. If it exposes only a mechanical conflict, use a neutral fresh-context integrator and review the resolution. Rerun Wave integration validation against the combined history.

When the Wave exit gate passes, promote it to `develop` with accepted Slice commits intact:

```bash
git switch develop
git merge --ff-only wave/<wave>
```

Squash an approved `hotfix/*` into `main` and carry it into `develop`. Discard `spike/*`; never merge it.
