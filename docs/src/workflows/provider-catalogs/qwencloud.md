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
[`catalog.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/providers/qwencloud/src/catalog.rs).
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
cargo test --locked -p yo-provider-qwencloud catalog
cargo test --locked -p yo-cli qwencloud_catalog
cargo test --locked -p yo-cli command::connect::picker
```

This update path intentionally makes no network request to enumerate a
QwenCloud plan. If an official authenticated account inventory becomes the
desired authority, treat that as a new discovery design rather than extending
the static table ad hoc.

## Explicit general API image connection

The admitted general API image envelope is separate from the plan catalogs.
It selects exact `qwen3.8-flash` at the international Chat endpoint. Existing
text bindings keep their behavior until an explicit complete-definition import;
a catalog capability flag does not enable image input. See the accepted
[service binding](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.model.service-binding.md)
and [Chat request contract](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.connector.openai-chat-completions.md)
for the closed envelope, and the current official
[Flash guide](https://docs.qwencloud.com/developer-guides/getting-started/latest-model).

Save this public definition to an absolute path and run
`yo connect --from /absolute/qwen-image.yaml`. Review the endpoint, model,
advisory image accounting, disabled thinking, semantic replay and general API
metering before confirming; enter the general API key at the hidden prompt.

```yaml
provider: qwencloud
provider_display_name: QwenCloud
account: general
account_display_name: General API
base_url: https://dashscope-intl.aliyuncs.com/compatible-mode/v1
profile:
  api_dialect: openai-chat-completions
  tokenizer_profile: utf8-bytes/v1
  input_token_limit: 991808
  max_output_tokens: 131072
  reasoning_parameters: {}
  optional_request_parameters:
    enable_thinking: false
    preserve_thinking: false
  tool_capability_policy: local-tools/v1
  replay_profile: semantic-only/v1
  image_input_profile: qwencloud-general-png-advisory/v1
models:
  - model: qwen3.8-flash
```

Yo stores the key under Provider `qwencloud`, Account `general` in
`credentials.yaml` beside the selected `config.yaml`; the public binding goes
in `connections.yaml`. On Linux the default directory is `~/.config/yo`.
These are local plaintext YAML files with owner-only permissions, not an
encrypted vault. Ordinary startup and recovery reuse the stored key; validation
must preserve it rather than copy it into disposable probe files or ask for it
again. A separate `qwencloud:default` plan account remains distinct.

Start `yo --model qwencloud:general:qwen3.8-flash`, attach a PNG through Ctrl+V
or the existing file-image flow, and submit. Qwen exposes enabled function tools
with explicit `tool_choice: auto`; idle `/compact` omits tools. Request options
remain disabled after compaction and fresh `yo --continue`. See
[terminal checks](../../validation/terminal-matrix.md#qwencloud-general-image-journey)
for the complete journey and free-service evidence.

The general API is metered. For a free actual-service check, independently
confirm this exact model's current free quota and
[`Free quota only`](https://docs.qwencloud.com/resources/free-quota) protection
before dispatch. Yo does not query quota or enable that provider setting, and
advisory image accounting does not establish a free entitlement.

Focused implementation checks:

```bash
cargo test --locked -p yo-core model_service
cargo test --locked -p yo-connector-openai-chat-completions
cargo test --locked -p yo-backend-managed
cargo test --locked -p yo-cli
```
