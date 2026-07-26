# Context Resolution contract

Run from the repository root:

```text
cargo run --locked -p methexis -- resolve-context tools/methexis/examples/context-contract/direct-request.json
```

The structured result reports the current trusted `develop` commit and the
created or reused artifact paths and hashes. `context.md` and `manifest.json`
are the exact immutable golden artifacts for that request; an unrelated
trusted commit does not change them.

Failures leave stdout empty and write one structured value to stderr. The
unsupported-tokenizer pair is the stable failure example.
