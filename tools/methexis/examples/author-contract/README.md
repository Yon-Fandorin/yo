# Revision authoring contract

Run from the repository root:

```text
cargo run --locked -p methexis -- author-revision tools/methexis/examples/author-contract/request.json
```

One call derives the SourceRevision, the Knowledge source pin and RevisionId,
the replacement Projection, and the review packet from the request contents,
then writes tracked Draft proposals. The bounded result reports the new
`revision`, `projection_hash`, packet locator, and `changed_paths`; approvals
remain a separate explicit human step (`methexis approve`). Re-running the
same request returns `status: unchanged` once every write has converged.

Failures leave stdout empty and write one structured value to stderr. The
revision-mismatch pair is the stable failure example.
