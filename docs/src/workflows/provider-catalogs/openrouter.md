# Maintain the OpenRouter catalog

OpenRouter is the runtime-discovery case. yo queries the configured account at
connection time, normalizes the authenticated response, and presents supported
and unsupported rows through the shared picker. Do not replace this with a
release-baked model list merely to make an update easier.

## Official source

Use OpenRouter's official
[Models API](https://openrouter.ai/docs/api/api-reference/models/list-all-models-and-their-properties).
It documents the authenticated `GET /api/v1/models` response and model
metadata. Treat the live response as untrusted input even though the source is
official.

## Code ownership

| Responsibility | Owner |
|---|---|
| Bounded authenticated transport | [`openrouter_discovery/transport.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/openrouter_discovery/transport.rs) |
| Response parsing, normalization, availability, and authored overrides | [`openrouter_discovery.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/openrouter_discovery.rs) and [`openrouter_discovery/normalize.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/openrouter_discovery/normalize.rs) |
| Configured discovery seed | [`config.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/config.rs) |
| Connect orchestration and picker handoff | [`external.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/external.rs) and [`picker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/input/picker.rs) |

## Update procedure

1. Compare the current official schema with only the fields yo consumes. An
   added response field is not automatically a yo capability; a renamed or
   redefined consumed field requires a contract and compatibility audit.
2. Update transport bounds or normalization only with a discriminating fixture
   for the old and new shape. Retain same-origin redirect policy, secret-safe
   diagnostics, response bounds, and typed disabled reasons.
3. Check authored-field provenance separately. Remote context/output limits
   apply when those exact fields were not authored; unrelated authored model
   fields must not suppress remote limits.
4. Exercise the authoritative picker handoff, not a shadow list or count. The
   rendered Provider, Account, disabled reason, and selected exact ModelId must
   come from the normalized rows consumed by connect.
5. Do not add a persistent cache or background refresh in a catalog update.
   Either would change freshness and failure behavior and therefore needs its
   own accepted design.

Focused checks:

```bash
cargo test --locked -p yo-core openrouter_discovery
cargo test --locked -p yo-cli connection::external::discovery_tests
cargo test --locked -p yo-cli connection::input::picker
```
