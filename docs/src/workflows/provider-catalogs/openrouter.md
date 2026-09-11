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
| Bounded authenticated transport | [`discovery/transport.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/providers/openrouter/src/discovery/transport.rs) |
| Response parsing, normalization, availability, and authored overrides | [`discovery.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/providers/openrouter/src/discovery.rs) and [`discovery/normalize.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/providers/openrouter/src/discovery/normalize.rs) |
| Configured discovery seed | [`config.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/state/config.rs) |
| Connect orchestration and picker handoff | [`external.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command/connect/external.rs) and [`picker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command/connect/picker.rs) |

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
cargo test --locked -p yo-provider-openrouter
cargo test --locked -p yo-cli command::connect::external::discovery_tests
cargo test --locked -p yo-cli command::connect::picker
```

## Explicit free image connection

Image input is initially available for the exact NVIDIA Nemotron 3 Nano Omni
free model below. Save this definition to a local YAML file, then run
`yo connect --from /absolute/vision-free.yaml`. Review the connection plan and
provide your OpenRouter key. The normal confirmation shows the exact model and
endpoint, NVIDIA-only zero-price routing without fallbacks, PNG image support
and advisory accounting before you accept. The distinct `vision-free` account label keeps
this plan separate from a `default` account; importing into an existing account
replaces its complete model group, so inspect the plan's removals.

```yaml
provider: openrouter
account: vision-free
base_url: https://openrouter.ai/api/v1
profile:
  api_dialect: openai-chat-completions
  tokenizer_profile: utf8-bytes/v1
  input_token_limit: 256000
  max_output_tokens: 65536
  reasoning_parameters: {}
  optional_request_parameters:
    provider:
      only: [nvidia]
      allow_fallbacks: false
      require_parameters: true
      max_price: {prompt: 0, completion: 0, request: 0, image: 0}
  tool_capability_policy: local-tools/v1
  replay_profile: semantic-only/v1
  image_input_profile: openrouter-free-png-advisory/v1
models:
  - model: nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free

```

Select the new connection with `/model` or start
`yo --model openrouter:vision-free:nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free`.
Use the existing Ctrl+V clipboard flow, inspect the preview, and press Enter.
For SSH/tmux, configure the same explicit clipboard source described in
[the image input flow](../../architecture/runtime-flow.md#clipboard-acquisition-and-ssh-forwarding).
`/attach /absolute/image.png` remains available for a file on the execution host.

The complete profile is required; discovery and binary upgrades do not add it
to existing connections. Every request—including text-only turns, tool results,
resume and summaries—keeps NVIDIA-only routing, zero-price limits and disabled
fallbacks. A capacity or price rejection is shown without automatic resend.
Other OpenRouter models retain their existing text behavior.

Images retain their exact normalized PNG bytes and order in saved Sessions.
Context pressure uses a labeled advisory estimate (image-free serialized UTF-8
bytes plus 2000 per image occurrence and a single 1024 reserve when images exist).
This is not measured provider usage or a guaranteed upper bound. The immutable
snapshot, source-size and summary bounds are the same as other admitted image
bindings. Responsibility stays with core complete-binding admission, the neutral
Chat Completions request projection, and managed complete-request accounting.
