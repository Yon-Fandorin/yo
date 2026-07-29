# Validation

Use the narrowest discriminating check while developing, then run the
repository checks before closing a Slice.

```bash
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
hk check
```

Tests establish deterministic code behavior. Ignored environment tests are
separate evidence: they run only where the required terminal, tmux, SSH, or
Codex installation is available and must not silently pass when unavailable.

See [Terminal environment matrix](./terminal-matrix.md) for those checks.
