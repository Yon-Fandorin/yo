# Approval-request preparation contract

Run from the repository root after `build-review` has published a packet:

```text
cargo run --locked -p methexis -- prepare-approval \
    tools/methexis/examples/prepare-approval-contract/manifest.json \
    --reviewer tui-architecture
```

`prepare-approval` binds the manifest's KnowledgeId, RevisionId, and
Projection hash into the exact `methexis.approval-request/v1alpha1` wire shape
that `methexis approve` consumes, so no value is copied by hand. `reviewed_at`
is the preparation wall clock in UTC; the checked-in
[`approval-request.json`](approval-request.json) golden marks it
`<wall-clock>`. With `--replace-current` the command additionally binds the
current approval record's RevisionId as `replace_revision`, and fails closed
when no approval record exists yet.

The command emits the request on stdout only. It never writes
`methexis/approvals/` and never records an approval: human authorization
remains the separate explicit `methexis approve` step. The reviewer is
validated against the tracked Owner foundation at preparation time; the
unknown-reviewer pair is the stable failure example. Save stdout as the
request file, then run `methexis approve <request.json>` once a human has
explicitly accepted the exact packet.
