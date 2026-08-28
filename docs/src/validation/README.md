# Validation

Choose evidence by the boundary that changed. Start with the smallest check
that can distinguish the expected behavior from its important failure, then
widen before closing the
[Slice](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#slice-contract).

## Evidence layers

| Layer | What it establishes | Examples |
|---|---|---|
| In-process | Deterministic state, protocol, layout, rendering, and injected failure behavior | `yo-core` engine/runtime tests; `yo-tui` component tests; rendering parity goldens |
| Host-integrated | Behavior of real host facilities without optional installed services | Linux PTY, termios, process signal, and terminal-restoration tests in `yo-cli` |
| External environment | Compatibility with installed programs, authentication, and nested terminal environments | Codex, Grok, tmux, local `sshd`, SSH, and tmux inside SSH |

The first layer gives fast diagnosis but cannot prove an OS terminal lifecycle.
The host-integrated layer exercises real Unix boundaries but cannot prove every
terminal multiplexer or remote session. The external layer closes those gaps
only for the environment where it actually ran.

An ignored or unavailable environment check is **unverified**, not passed.
Record the missing command, host, credential, or platform instead of weakening
the assertion or silently skipping it.

## Start from the changed boundary

| Change area | First useful command | Closest evidence |
|---|---|---|
| Session, Turn, Activity, engine, or runtime semantics | `cargo test -p yo-core` | `crates/yo-core/src/tests` and the owning module tests |
| Typed input spans, submission identity, or fixed-v1 structured-reference rejection | `cargo test -p yo-core input::tests` and `cargo test -p yo-core journal::codec` | `crates/yo-core/src/input/tests.rs` and Journal wire-compatibility tests |
| Agent-session admission, concurrency, startup, or shutdown | `cargo test -p yo-core agent_session::tests` | `crates/yo-core/src/agent_session/tests` |
| Backend lifecycle, evidence, or bounded child-process transport extraction | `cargo test --locked -p yo-backend` followed by `cargo test --locked -p yo-core backend::evidence` and `cargo test --locked -p yo-core journal::codec::tests::correlation` | `crates/backends/foundation/src`, the `yo-core` specialization, and Journal wire/recovery compatibility tests |
| Codex protocol translation or provider-ID correlation | `cargo test --locked -p yo-backend-delegated-codex` | `crates/backends/delegated-codex/src/tests.rs` |
| Grok ACP translation, permissions, authentication, or Session correlation | `cargo test --locked -p yo-backend-delegated-grok` | `crates/backends/delegated-grok/src/tests.rs` and `protocol.rs` |
| Decoded input, editing, paste, bindings, or exit gestures | `cargo test -p yo-tui input::` | Tests beside `yo-tui/src/input` |
| Prompt wrapping, cursor visibility, or viewport behavior | `cargo test -p yo-tui prompt::` | Tests beside `yo-tui/src/prompt` |
| `@` trigger, stale result, selection replacement, local ranking, or Git-ignore discovery | `cargo test -p yo-tui workspace_reference` and `cargo test -p yo-core workspace_reference` | `yo-tui/src/prompt/workspace_reference.rs` and `yo-core/src/workspace_reference` |
| `$` trigger, Codex catalog decoding, scope filtering, disabled rows, or typed skill selection | `cargo test -p yo-tui skill_reference`, `cargo test -p yo-core skill_reference`, and `cargo test -p yo-backend-delegated-codex skill_catalog` | `yo-tui/src/prompt/skill_reference`, `yo-core/src/skill_reference`, and `backends/delegated-codex/src/skill_catalog.rs` |
| Transcript items, streaming revisions, or scrolling | `cargo test -p yo-tui transcript::` | Tests beside `yo-tui/src/transcript` |
| Shell composition, layout, Surface, Unicode width, or text flow | `cargo test -p yo-tui` | Tests beside the owning `yo-tui` module |
| ANSI operations or presentation-mode policy | `cargo test -p yo-tui terminal::` | Tests under `yo-tui/src/terminal` |
| Inline or Fullscreen mode behavior | `cargo test -p yo-tui terminal::mode::` | Tests under `yo-tui/src/terminal/mode` |
| Live-loop ordering, backpressure, submission draft ownership, or event projection | `cargo test -p yo-tui runner::` | Tests under `yo-tui/src/runner` |
| Terminal and HTML projection of the same completed frame | `cargo test -p yo-tui --test rendering_parity` | `crates/yo-tui/tests/rendering_parity` and its goldens |
| Process termination or real terminal restoration | `cargo test -p yo-cli pty_tests::` | `crates/yo-cli/src/pty_tests.rs` |
| Unix process-coordinator state and compensation | `cargo test -p yo-cli process::termination::tests` | `crates/yo-cli/src/process/termination/tests` |
| Shared bounded YAML parsing, inference, and failure budgets | `cargo test -p yo-yaml` | `shared/yo-yaml/src/lib.rs` |
| Required explanations immediately above Rust tests | `cargo xtask check test-explanations` | Rust sources under `crates/`, `shared/`, and `tools/` |
| Slice changes remain inside their bound local write-set | `cargo xtask check slice-scope` | One active Slice worktree; the planner first runs `cargo xtask slice-contract bind <contract.json>` |
| Two Slice contracts have a common current integration base and disjoint declared ownership | `cargo xtask check slice-parallel <left.json> <right.json>` | Direct Slices use `develop`; Wave Slices use their Wave branch |
| One clean Slice candidate has validation, review, risk, and approval evidence bound to the same identity | `cargo xtask slice gate <request.json>` | Returns exactly one next action without rerunning validation or review |
| A ready Slice needs an exact commit message and close record without identity transcription | Run `cargo xtask slice commit prepare <gate.json> <message-source> <message-out>`, commit the exact squash, then run `cargo xtask slice close prepare <request.json>` before `close plan/apply` | The first prepare runs in the clean Slice worktree; close prepare runs in the clean integration worktree after the accepted commit |
| Repository hook policy or structured development checks | `cargo test -p xtask` | `tools/xtask/src` |
| Prospective activation ContextBuild and review-packet identity | `cargo test -p methexis activation_review_context` and `cargo test -p xtask review_packet::tests::prospective` | Exact activation request, proposed Checkpoint/active record, authority mode, packet replay, and active-authority cross-use rejection |
| Linux/macOS conditional compilation | `bash tools/validation/yo-cli-unix-matrix.sh` | Local host result plus `.github/workflows/unix-compile.yml` for both hosts |
| tmux, SSH, or nested tmux behavior | See the [terminal environment matrix](./terminal-matrix.md) | Ignored `yo-cli` environment tests |

These commands are entry points, not permission to ignore affected neighboring
boundaries. For example, an edit to `AgentSession` can require both its focused
tests and the TUI runner tests when the admission result observed by the
frontend changes.

For model-connector request and stream validation, run the concrete Connector
crate that owns the changed dialect, such as
`cargo test --locked -p yo-connector-openai-chat-completions` or
`cargo test --locked -p yo-connector-kimi`, together with
`cargo test --locked -p yo-connector-transport` when shared byte lifecycle
mechanics are affected. Run `cargo test --locked -p yo-core` at close for the
neutral vocabulary and managed-loop consumer. Environment-integrated Connector checks use only local
`127.0.0.1` HTTPS listeners and require the external `python3` and `openssl`
commands to create and serve their ephemeral test certificates. Missing
prerequisites fail the command rather than skip its assertions; record the
host/platform, prerequisite versions, and pass/unverified result for each
validation run.

## Reading a result

- **Passed** means the named command ran its assertions successfully in the
  stated environment.
- **Failed** means the command ran and found a mismatch, timeout, panic, or
  cleanup error. Follow the first owning boundary, then retain any additional
  cleanup failures.
- **Unverified** means the check did not run in the required environment. Keep
  it visible as a coverage gap.

Goldens and snapshots establish an exact projection of their fixture. Review
the diff when intentionally updating one; do not treat regeneration alone as
evidence that the new output is correct.

## Keep agent-facing output bounded

Run verbose validation through `tools/validation/bounded-run.sh` when its
output will return to an agent context. The wrapper preserves the command's
exit status and complete combined output under the worktree-local
`.local-exclude/validation-runs/` directory. A successful run returns one JSON
summary line. A failed run returns the same summary and at most the final 16
KiB of diagnostic output; inspect the complete local log only when that tail
does not identify the owning failure.

By default the summary schema is the frozen
`yo.validation-run-summary/v1alpha2`. It records the
launch `HEAD`, whether the worktree was clean, a boundary-aware hash and count
of the exact command arguments, the complete log's byte count and SHA-256, and
the `reviewed-descendant/v1` reuse policy.
This makes a clean candidate's result self-binding when the Slice gate compares
it with the declared command. A dirty summary remains useful for local
diagnosis but is not candidate evidence. The summary always reports
`"reused":false` because it records an actual execution; it does not discover
or reuse an earlier run automatically. A later gate may declare
`"reused":true` only for a passing summary with the same exact command when
trusted Git proves that its clean launch HEAD is an ancestor of the reviewed
final candidate. Frozen `yo.validation-run-summary/v1` and `v1alpha1`
artifacts remain gate-compatible with their original meaning; v1alpha1 does
not permit reuse.

For a command whose result is determined entirely by local repository bytes,
add `--reusable-local`. This opt-in emits
`yo.validation-run-summary/v1alpha3` with the
`reviewed-descendant-context/v1` policy. Besides the v1alpha2 bindings, it
records the target OS, architecture, and a Rust/Cargo toolchain fingerprint.
At a later reused gate, Yo observes those values again and fails closed if they
changed. Its `external_state:"none-declared"` assertion excludes commands that
depend on a network, clock, account, service, or other external state; rerun
such commands instead. The option does not search for an earlier receipt and
never changes an existing summary.

To retain a summary for review and gate preparation without copying stdout,
create its ignored parent and publish it directly:

```bash
mkdir -p .local-exclude/coordination/<slice>/validation
bash tools/validation/bounded-run.sh \
  --summary-out .local-exclude/coordination/<slice>/validation/workspace-tests.json \
  --reusable-local \
  workspace-tests -- cargo test --workspace --all-targets
```

The output file and stdout line are byte-identical. Publication is atomic and
create-only: a missing parent or existing target stops before the validation
command, and a concurrent target collision is never overwritten. Add the
published file to the immutable review packet so its manifest supplies the
path and hash to `slice gate prepare`. This stores new evidence only; it does
not reuse an earlier result. A reuse decision belongs to the later reviewed
Slice gate request, not this runner.

The wrapper changes presentation, not validation semantics. Its logs are
temporary operational artifacts: keep a required failure log only while the
finding is unresolved and discard completed logs with the Slice worktree.

## Consolidate one candidate gate

Once a Slice candidate is a clean commit, save each bounded validation JSON
summary and each final review response as a separate local file. Record their
exact hashes, the candidate commit, canonical diff hash, required lenses,
known unverified environments, risk classification, and human-origin approval
in a `yo.slice-gate-request/v1alpha1` request. Then run:

```bash
cargo xtask slice gate /tmp/<slice>-gate.json
```

A minimal request with one declared check and one completed lens has this shape
(repeat the evidence entries when more checks or lenses apply):

```json
{
  "schema": "yo.slice-gate-request/v1alpha1",
  "candidate_commit": "<full-commit>",
  "required_lenses": ["fresh-context"],
  "validation_evidence": [{
    "name": "workspace-tests",
    "argv": ["cargo", "test", "--workspace", "--all-targets"],
    "result_path": "/tmp/workspace-tests.json",
    "result_hash": "sha256:<summary-hash>",
    "candidate_commit": "<full-commit>",
    "reused": false
  }],
  "review_evidence": [{
    "lens": "fresh-context",
    "reviewer": "provider/session",
    "route": "model-high/provider/model/session",
    "verdict": "clear",
    "candidate_commit": "<full-commit>",
    "diff_hash": "sha256:<canonical-diff-hash>",
    "result_path": "/tmp/fresh-context.txt",
    "result_hash": "sha256:<response-hash>"
  }],
  "known_unverified_environments": [],
  "risk": {
    "classification": "human-attention",
    "rationale": "changes workflow authority"
  },
  "approval": null
}
```

After exact human approval, replace `null` with `kind: "exact_candidate"`, a
`human/<identity>` authority and scope, plus the same `candidate_commit` and
`diff_hash`. A routine request may instead use `kind: "standing_routine"` and
omit those two exact identity fields only when its human-origin scope covers
the work and no unverified environment remains.

The command checks the bound Slice scope, clean `HEAD`, path-derived minimum
lenses, evidence file hashes, candidate/diff identities, review routes, and
approval shape. It returns a single `yo.slice-gate-result/v1alpha1` JSON line
with exactly one `next_action`: `validate`, `review`, `approve`, or `integrate`.
It never runs those actions itself. A changed candidate, stale diff, mutated
evidence file, or omitted path-derived lens fails closed rather than producing
a next action.

This is an evidence-consistency check, not proof that the declarations are
true. The coordinator still owns the completeness of the validation plan,
semantic review lenses, risk classification, and recorded verdict. Keep the
request and evidence in ignored coordination storage or outside the worktree,
then remove them when the Slice closes.

## Slice-close baseline

After focused checks pass, run the repository baseline:

```bash
bash tools/validation/bounded-run.sh workspace-tests -- cargo test --workspace --all-targets
bash tools/validation/bounded-run.sh workspace-clippy -- cargo clippy --workspace --all-targets -- -D warnings
bash tools/validation/bounded-run.sh hk-check -- hk check
```

`cargo test` runs the normal test set and compiles ignored tests; it does not
execute ignored environment tests. `hk check` selects repository checks from
`hk.pkl` according to the changed paths, including formatting, test
explanations, affected crate checks, Methexis checks, and Developer Docs checks.
Installation and hook usage belong to
[`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#local-checks).

For a staged Methexis change containing only `methexis/sources/` and
`methexis/knowledge/` paths, the hook first requires the working Methexis tree
to match the index exactly and contain no untracked Methexis paths, then runs
only the `records` and `relations` classes. This admits a semantic-first
candidate while its prior Projection is intentionally stale. Any staged
Projection, approval, Checkpoint, active record, or other Methexis path keeps
the complete authority-aware validation path.

Use focused checks from the local Slice contract while editing, then run this
Slice-close baseline once the outcome is complete. For the exact staged
Methexis activation interval, `hk` uses prospective validation and defers the
ordinary Methexis tests; immediately after integration, run the ordinary full
Methexis check and tests against trusted `develop`.

Prepare that activation worktree from clean `develop` with
`cargo xtask slice create-activation <request.json>`. The generated contract
leases the active record, the Checkpoint tree, and the two registered context
manifests. Its focused `methexis check --staged-activation` admits exactly one
new immutable Checkpoint. Slice creation is coordination setup, not evidence
that the prospective transition is valid.

For a later independent activation review, use the explicit v1alpha3 review
request only after the enabling workflow implementation is already trusted.
The focused tests above prove the trusted-capability bootstrap, exact
activation-only path boundary, proposal identity, and canonical packet replay;
the candidate still requires staged activation validation before integration
and ordinary full Methexis validation immediately afterward.

If the Slice changes a platform or external-environment boundary, add the
relevant matrix command rather than claiming the baseline covered it.

Do not rerun the unchanged baseline merely because a reviewed candidate was
squashed. Its result remains evidence for the accepted commit only when an
exact Git diff proves both commits have the same tree, integration added no
conflict resolution or edit, the toolchain and environment are unchanged, no
external-state evidence expired, and commit hooks passed. Otherwise rerun the
affected checks. This reuse never replaces validation or review of the
candidate itself.

The Slice-close cleanup command is not part of this validation baseline. After
the gate returns `integrate`, `slice commit prepare` appends its exact review
trailers to a human-authored semantic message without staging or committing.
After the matching accepted commit exists, `slice close prepare` derives its
identities and passed validation rows from that same ready gate. Its compact
`yo.slice-close-prepare-request/v1alpha1` input retains only operational facts
the gate does not know: execution lanes, review and packet totals, elapsed
bottleneck, and commands for known unverified environments. The command
publishes the standard `close-metrics.json`; it does not plan or apply cleanup.

The close plan
publishes its plan directly to the requested file, then consumes the already
accepted result afterward and rechecks the exact refs, review trailers, patch
identity, worktree cleanliness, binding, contract hash, and plan hash before
removing the local worktree, standard transient Slice contract, and Slice
branch. Directly writing the complete metrics file remains supported. The plan binds that
record to the exact Slice candidate and accepted commit; apply rejects changed
metrics. The plan also lists every immediate coordination entry it will
retain, including the metrics; apply rejects a changed list and never removes
those entries. Store the plan outside both the removed worktree and that
Slice's coordination directory. See the integration workflow in
[`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#review-and-integration).

## Useful owners

- Hook selection: [`hk.pkl`](https://github.com/Yon-Fandorin/yo/blob/develop/hk.pkl)
- Structured repository checks: [`tools/xtask`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/xtask/src/lib.rs)
- Unix host compile check: [`tools/validation/yo-cli-unix-matrix.sh`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/validation/yo-cli-unix-matrix.sh)
- Rendering parity fixture: [`crates/yo-tui/tests/fixtures/rendering-parity/README.md`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/tests/fixtures/rendering-parity/README.md)
- Test explanation policy: [`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#test-code)
