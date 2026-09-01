# Review delivery

This file is the repository workflow authority for external-review
authorization, target admission, exact-once delivery, continuation, and
reviewer execution. It is routed directly from [`AGENTS.md`](../AGENTS.md).

Packet construction belongs to [Review packets](review-packets.md). Verdict
evidence, approval, integration, and cleanup belong to
[Review and integration](review-and-integration.md).

## Authorization and execution

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

Frozen delegated request v1alpha4 adds Grok's exact native read-only startup
probe. Frozen v1alpha5 preserves that check and, only when the native sandbox
cannot start, probes Yo's bounded `bwrap` read-only no-tools boundary and
records the selected `execution_isolation`. Neither probe supplies a prompt,
starts a Session, or makes a Provider request. Codex continues to use v1alpha3;
do not apply Grok's invocation contract to another host.

Use managed request v1alpha6 for new integrated preparation. It preserves the
exact stored Provider, Account, and Model binding checks, but treats a typed
blocking `last_failure` as current for 5 hours instead of forever. At 5 hours
the observation becomes an explicit stale warning and the already-authorized
exact delivery may revalidate the target once. This creates no probe request
and grants no retry, steer, fallback, target switch, or additional Provider
request. The existing observation path remains the only state update: success
clears `last_failure`; failure records the new kind and time. Frozen admission
v1alpha1 through v1alpha5 retain their prior blocking behavior.

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

The frozen v1alpha1 and v1alpha2 delegated authorizations permit at most one
finding-resolution request. When that resolution reports new material
findings, `yo.external-review-delegated-authorization/v1alpha3` may authorize
a larger explicit `max_finding_resolution_resume_requests` under
`yo.external-review-chain/bounded-multihop/v1alpha1`. It permits one original
request and at most 63 finding-resolution requests, with their sum recorded as
`max_total_requests`. Egress counts the immutable review-delta chain, binds
each step to the immediately preceding delivery receipt and the same Session,
and rejects the first over-limit step before host delivery. This changes
neither the one-request delivery execution limit nor the zero retry, steer,
fallback, and target-switch rules.

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

Choose the reviewer before packet delivery from routes that the human has
already configured and authorized. First match the required lens, then compare
current target admission and the newest available account-capacity or Session
usage observation, and finally prefer the route expected to complete with less
coordination and token cost. An unknown quota is not proof of availability or
unavailability. A typed blocking failure younger than 5 hours remains current;
use the managed v1alpha6 stale-failure rule above when it is older instead of
carrying the failure forward indefinitely. Record the chosen route and this
selection basis in Slice coordination before delivery. No Provider, including
Kimi or Codex, has a permanent default priority. Do not invent commands for an
unconfigured provider.

One independent reviewer is the default for an ordinary implementation Slice.
Use multiple providers only when the human or Slice contract requires
independent perspectives for a consequential design boundary, cross-Provider
semantics, or another named risk whose likely blind spots justify the added
cost. Give parallel reviewers the same immutable candidate and lens-specific
questions, start them concurrently when their effects are independently
authorized, and reconcile findings only after all requested results arrive.
Do not send an already-clear ordinary review to another provider merely to
compare models. When the lens specifically needs hidden assumptions,
counterexamples, alternatives, or future costs, select a configured
different-perspective provider and require its strongest counterargument before
the verdict.

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
guessing. If a correctly invoked selected provider cannot finish because the
service or allowance is unavailable, record the requested provider and reason.
Another configured provider may perform the same lens in a separate
fresh-context Session only under delivery authority that permits that new
effect; it is not an automatic fallback or retry. Do not retry an unavailable
provider until its state changes or its blocking observation becomes stale
under the 5-hour rule. Human exact review is also valid; the implementing
session's self-check is not.

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
