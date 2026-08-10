# Revision authoring contract

Run from the repository root:

```text
cargo run --locked -p methexis -- author-revision tools/methexis/examples/author-contract/semantic-request.json
```

The v1alpha2 example derives the SourceRevision, Knowledge source pin, and
RevisionId, then writes only those canonical Draft proposals. Projection and
packet creation are explicit later steps. `semantic-success.json` pins that
bounded response.

`request.json` and `success.json` retain the v1alpha1 compatibility contract,
which also derives the replacement Projection and review packet in one call.
Neither version writes approvals. Re-running the same request returns
`status: unchanged` once every version-owned write has converged.

Failures leave stdout empty and write one structured value to stderr. The
revision-mismatch pair is the stable failure example.
