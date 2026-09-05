# Maintain the QwenCloud catalogs

QwenCloud is the release-known static-registry case. Alibaba Cloud publishes
exact plan allowlists and plan-specific endpoints. Registration performs
structural admission only; ordinary model use later determines whether the
supplied credential can actually use the selected row.

## Official sources

Use the official Alibaba Cloud pages for the applicable profile:

- [Coding Plan](https://www.alibabacloud.com/help/en/model-studio/coding-plan)
  for the exact model allowlist and Coding Plan endpoints;
- [Token Plan (Team Edition)](https://www.alibabacloud.com/help/en/model-studio/token-plan-overview)
  for the exact model and capability table; and
- the applicable quick-start page when endpoint, region, protocol, or key type
  needs confirmation.

Do not infer a model version from a nearby name. Do not treat membership in an
official plan list as proof that a particular account has an active seat,
quota, or entitlement.

## Code ownership

The static profile definitions, endpoints, rows, typed capabilities, and
deterministic ordering live in
[`qwencloud_catalog.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/qwencloud_catalog.rs).
Configuration resolves a profile into a non-routable seed in
[`config.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/state/config.rs).
The shared selection and recoverable connection transaction live in
[`external.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command/connect/external.rs)
and
[`picker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command/connect/picker.rs).

## Update procedure

1. Select one existing catalog profile and confirm its exact official
   allowlist and endpoint. If the plan or regional meaning no longer matches
   that profile, define a new versioned profile through SOT-first work instead
   of silently reinterpreting the old one.
2. Produce a field-level old/new table for ModelId, modalities, tool support,
   reasoning presentation, context limit, output limit, endpoint, and dialect.
   Mark missing official evidence rather than filling it from another vendor's
   page or a model-family assumption.
3. Update only the relevant `CatalogDefinition` and `CatalogRow` data or the
   smallest helper that correctly represents the official facts. Keep valid
   image-only or otherwise unsupported rows visible and disabled when yo lacks
   the required runtime interface.
4. Add exact row/order assertions for the changed profile. Test duplicate and
   unknown profiles, disabled-row rejection before secret input, picker
   cancellation before secret or mutation, and exact three-part selection.
5. Retain the stale-managed-row regression: a previously stored row outside
   the current registry remains usable for startup/recovery, but cannot become
   a new catalog candidate.

Focused checks:

```bash
cargo test --locked -p yo-core qwencloud_catalog
cargo test --locked -p yo-cli qwencloud_catalog
cargo test --locked -p yo-cli command::connect::picker
```

This update path intentionally makes no network request to enumerate a
QwenCloud plan. If an official authenticated account inventory becomes the
desired authority, treat that as a new discovery design rather than extending
the static table ad hoc.
