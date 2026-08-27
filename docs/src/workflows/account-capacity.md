# Inspect account capacity

Use `yo account` when the question is how much Provider-managed account capacity
remains now. Use `yo usage SESSION_ID` when the question is how many request,
output, reasoning, and cache tokens a stored Session observed. The two reports do
not share totals or infer one from the other.

## Public commands

```bash
yo account codex --refresh
yo account kimi:default --refresh
yo account kimi:default --refresh --format json
```

`codex` means the locally installed Codex account. `kimi:ACCOUNT` names one exact
account already stored by Yo with either the `kimi-code-membership/v1` catalog
profile or an exact canonical Kimi Code complete binding, plus its exact
Provider-and-Account credential. The binding fallback retains connections made
before catalog-seed persistence; it does not admit a custom endpoint. A Kimi
Platform API account is a different product and is rejected before a request.

`--refresh` deliberately makes the read live. Neither route creates an Agent
Session, sends a model prompt, or falls back to another Provider. Codex starts
its local app-server, initializes it, calls `account/rateLimits/read` once, and
shuts it down. Kimi first makes one authenticated `GET /coding/v1/me` for the
account level name, then one authenticated `GET /coding/v1/usages` for its
limits. Redirects and retries are disabled and each successful body is bounded
to 1 MiB. The Kimi account level name is the exact Provider value shown as the
plan; Yo does not derive it from limit sizes.

Text output is for people. `--format json` emits the same provider-neutral
snapshot under the versioned `yo.account-capacity/v1alpha1` schema for agents.
Provider count values are normalized conservatively: the used percentage is
rounded up, so the displayed remaining percentage never overstates the exact
remaining ratio. Missing data stays absent or Unknown; it is not synthesized
from Session token usage.

## Referenced upstream code

Every Provider adapter should record the exact upstream revision and file that
established its wire behavior. These links are implementation evidence, not a
second Yo contract, and are pinned so a later upstream change cannot silently
rewrite the rationale.

| Feature | Pinned upstream source | Yo adaptation |
|---|---|---|
| Codex account capacity | OpenAI Codex commit `89650c66f2f3ff0d028d3f5d6d0b187b2ed49be5`: [app-server rate-limit request and fields](https://github.com/openai/codex/blob/89650c66f2f3ff0d028d3f5d6d0b187b2ed49be5/codex-rs/app-server/README.md#7-rate-limits-chatgpt) and [v2 account protocol types](https://github.com/openai/codex/blob/89650c66f2f3ff0d028d3f5d6d0b187b2ed49be5/codex-rs/app-server-protocol/src/protocol/v2/account.rs) | [`delegated-codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-codex/src/lib.rs) owns app-server lifecycle and [`protocol.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-codex/src/protocol.rs) maps the returned buckets. |
| Kimi Code account capacity | MoonshotAI Kimi Code commit `21f7ef64f0851504227617f4501bf8359031d9a5`: [`managed-userinfo.ts`](https://github.com/MoonshotAI/kimi-code/blob/21f7ef64f0851504227617f4501bf8359031d9a5/packages/oauth/src/managed-userinfo.ts) for the canonical `/me` request and `user_level_name`, plus [`managed-usage.ts`](https://github.com/MoonshotAI/kimi-code/blob/21f7ef64f0851504227617f4501bf8359031d9a5/packages/oauth/src/managed-usage.ts) for `/usages`, the weekly summary, rolling windows, and fixed-point booster balance | [`usage.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/kimi_catalog/usage.rs) keeps the product check, two exact requests, bounded parsers, and neutral snapshot mapping beside the Kimi catalog seed. |

When changing an adapter, inspect a new upstream commit, pin the new source link,
update discriminating fixtures, and validate the exact live boundary. Do not cite
an unpinned `main` branch or infer a private endpoint from UI output.

## Failure boundaries

- A missing stored Kimi account or credential is a local configuration error and
  sends no request.
- Non-success status, redirect, wrong media type, malformed JSON, missing or
  unsafe Kimi level name, invalid reset time, zero limit, excess rows, or excess
  bytes fails the refresh instead of returning a partial healthy-looking report.
- Secrets are used only for the exact authenticated request and are absent from
  errors, snapshots, text, JSON, and test evidence.
- Account-capacity failure never starts a Session, retries a Provider, or changes
  stored connection state.
