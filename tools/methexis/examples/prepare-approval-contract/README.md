# Approval-request preparation contract

Run the default direct-canonical form from the repository root after semantic
review clears:

```text
cargo run --locked -p methexis -- prepare-approval \
    --canonical tui.relocated \
    --revision sha256:54e7e0b515b400d95c7a3578fbd72fef0ab0e1d71c6f1b057637ee0f2998874d \
    --reviewer tui-architecture
```

It produces the
[`canonical-approval-request.json`](canonical-approval-request.json) v1alpha2
shape and neither needs nor creates a Projection. When a human explicitly
wants the additional Korean pair, run the Projection form after `build-review`
has published a packet:

```text
cargo run --locked -p methexis -- prepare-approval \
    tools/methexis/examples/prepare-approval-contract/manifest.json \
    --reviewer tui-architecture
```

That form binds the manifest's KnowledgeId, RevisionId, and Projection hash
into the exact [`approval-request.json`](approval-request.json) v1alpha1 shape.
Both forms emit the preparation wall clock as `reviewed_at`; the checked-in
goldens mark it `<wall-clock>`. With `--replace-current` the command
additionally binds the current approval record's RevisionId as
`replace_revision`, and fails closed when no approval record exists yet.

The command emits the request on stdout only. It never writes
`methexis/approvals/` and never records an approval: human authorization
remains the separate explicit `methexis approve` step. The reviewer is
validated against the tracked Owner foundation at preparation time; the
unknown-reviewer pair is the stable failure example. Save stdout as the
request file, then run `methexis approve <request.json>` once a human has
explicitly accepted the exact basis named by that request.
