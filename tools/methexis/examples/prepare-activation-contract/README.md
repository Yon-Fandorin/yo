# Activation-request preparation contract

Run from the repository root with the saved stdout of `create-checkpoint`:

```text
cargo run --locked -p methexis -- create-checkpoint <request.json> > create-output.json
cargo run --locked -p methexis -- prepare-activation create-output.json
```

`prepare-activation` binds the result's `checkpoint_id` and `hash` into the
exact `methexis.activation-request/v1alpha1` wire shape that
`methexis propose-activation` consumes. When a working-tree active record
exists, its exact content hash is bound as `replace_active_hash`, the
compare-and-swap predecessor `propose-activation` verifies; the initial
activation omits it. The command emits the request on stdout only; it never
proposes activation. Save stdout as the request file, then run
`methexis propose-activation <request.json>`.

Input that is not a successful `create_checkpoint` operation result fails
closed with `invalid_create_output`.
