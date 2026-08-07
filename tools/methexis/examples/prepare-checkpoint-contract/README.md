# Checkpoint-request preparation contract

Run from the repository root once an active Checkpoint exists:

```text
cargo run --locked -p methexis -- prepare-checkpoint
```

`prepare-checkpoint` reads the working-tree active record and its referenced
immutable Checkpoint, then emits the exact
`methexis.checkpoint-request/v1alpha1` wire shape that
`methexis create-checkpoint` consumes, carrying the currently active roots.
The command emits the request on stdout only; it never creates a Checkpoint.
Save stdout as the request file, then run
`methexis create-checkpoint <request.json>`.

Without an active Checkpoint there are no roots to bind, so the command fails
closed; the no-active-checkpoint pair is the stable failure example.
