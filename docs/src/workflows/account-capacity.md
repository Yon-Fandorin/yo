# Inspect account capacity

Use `yo account` when the question is how much Provider-managed account capacity
remains now. Use `yo usage SESSION_ID` when the question is how many request,
output, reasoning, and cache tokens a stored Session observed. The two reports do
not share totals or infer one from the other.

## Public commands

```bash
yo account codex --refresh
yo account grok --refresh
yo account kimi:default --refresh
yo account qwencloud:default --refresh
yo account kimi:default --refresh --format json
```

`codex` and `grok` mean the accounts used by their locally installed delegated
hosts. `kimi:ACCOUNT` names one exact account already stored by Yo with either
the `kimi-code-membership/v1` catalog profile or an exact canonical Kimi Code
complete binding, plus its exact Provider-and-Account credential. The binding
fallback retains connections made before catalog-seed persistence; it does not
admit a custom endpoint. A Kimi Platform API account is a different product and
is rejected before a request. `qwencloud:ACCOUNT` accepts an exact stored Token
Plan connection using the canonical Singapore endpoint. It reads the Personal
Token Plan console with the current QwenCloud browser session; the stored
`sk-sp-*` model-inference key cannot authorize that console surface.

`--refresh` deliberately re-observes the named account source. No route creates
an Agent Session, sends a model prompt, or falls back to another Provider. Codex starts
its local app-server, initializes it, calls `account/rateLimits/read` once, and
shuts it down. Grok starts `grok agent stdio`, initializes ACP v1, authenticates
once with the advertised `cached_token` method, reads the exact
`_meta.subscription_tier`, and shuts it down. Identity metadata is ignored.
The distributed Grok ACP service does not expose its internal billing extension,
so Yo also reads at most the last 1 MiB of Grok's official `unified.jsonl` and
uses only the newest complete `billing: fetched credits config` event whose
weekly period has not ended. If none exists, Yo still reports the authenticated
plan without inventing a usage window. Kimi first
makes one authenticated `GET /coding/v1/me` for the account level name, then
one authenticated `GET /coding/v1/usages` for its limits. Redirects and retries
are disabled and each successful body is bounded to 1 MiB. Provider plan names
are shown exactly; Yo does not derive them from limit sizes. QwenCloud reads the
complete `QWEN_CLOUD_COOKIE` only for the exact QwenCloud console origin. It
resolves `sec_token` from the cookie or one bounded dashboard HTML response, then
sends the console's `usage`, `subscription`, and `quota-config` requests
concurrently. Redirects and retries are disabled, every request has an
eight-second deadline, and every successful body is bounded to 1 MiB. The
returned 5-hour window is optional because the Provider can omit it; the 7-day
window and active `specCode` remain Provider-authored observations.

### QwenCloud console session

Qwen model requests still need only the stored `sk-sp-*` inference key and
matching Token Plan endpoint. Account-capacity refresh instead needs the browser
session that can open QwenCloud Billing. Yo does not install another CLI, use an
Alibaba management profile, or persist the browser cookie.

1. Sign in at `https://home.qwencloud.com`, then open **Billing > Subscription**.
2. In browser Developer Tools, open **Network**, reload, select an `api.json`
   request to `cs-data.qwencloud.com`, and copy the complete `Cookie` request
   header. It must contain `login_qwencloud_ticket`.
3. Paste it into a hidden shell read:

   ```bash
   read -rs QWEN_CLOUD_COOKIE
   export QWEN_CLOUD_COOKIE
   ```

   Press Enter after pasting. The cookie remains process-local and is not written
   to Yo configuration.
4. Yo normally resolves `sec_token` from the logged-in dashboard. Only when that
   fails, copy the request's `sec_token` form field and provide it the same way:

   ```bash
   read -rs QWEN_CLOUD_SEC_TOKEN
   export QWEN_CLOUD_SEC_TOKEN
   ```
5. Refresh once, then remove both values from the shell:

   ```bash
   yo account qwencloud:default --refresh
   unset QWEN_CLOUD_COOKIE QWEN_CLOUD_SEC_TOKEN
   ```

Missing or expired console state fails only this account-capacity refresh. It
never disables the stored Qwen model connection or sends a model request.

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
| Grok account plan and latest billing observation | xAI Grok Build commit `9684fa3cdbf2995e30ea8b9b637f1db008f144fc`: [ACP authenticate response construction](https://github.com/xai-org/grok-build/blob/9684fa3cdbf2995e30ea8b9b637f1db008f144fc/crates/codegen/xai-grok-shell/src/agent/mvp_agent/mod.rs), [typed authentication metadata](https://github.com/xai-org/grok-build/blob/9684fa3cdbf2995e30ea8b9b637f1db008f144fc/crates/codegen/xai-grok-shell/src/auth/meta.rs), and [bounded unified billing log event](https://github.com/xai-org/grok-build/blob/9684fa3cdbf2995e30ea8b9b637f1db008f144fc/crates/codegen/xai-grok-shell/src/extensions/billing.rs). The exact boundary was also observed with installed Grok CLI `1.0.5 (5115b46bc9)`. | [`delegated-grok`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-grok/src/lib.rs) owns the initialize-authenticate-shutdown read; [`billing_log.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-grok/src/billing_log.rs) reads only a bounded tail and maps the official current-period event. |
| Kimi Code account capacity | MoonshotAI Kimi Code commit `21f7ef64f0851504227617f4501bf8359031d9a5`: [`managed-userinfo.ts`](https://github.com/MoonshotAI/kimi-code/blob/21f7ef64f0851504227617f4501bf8359031d9a5/packages/oauth/src/managed-userinfo.ts) for the canonical `/me` request and `user_level_name`, plus [`managed-usage.ts`](https://github.com/MoonshotAI/kimi-code/blob/21f7ef64f0851504227617f4501bf8359031d9a5/packages/oauth/src/managed-usage.ts) for `/usages`, the weekly summary, rolling windows, and fixed-point booster balance | [`usage.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/kimi_catalog/usage.rs) keeps the product check, two exact requests, bounded parsers, and neutral snapshot mapping beside the Kimi catalog seed. |
| QwenCloud Personal Token Plan capacity | OmniRoute commit `825f8feea73daead73cf6832bed7c61531f9c065`: [`qwenTokenPlanQuotaFetcher.ts`](https://github.com/diegosouzapw/OmniRoute/blob/825f8feea73daead73cf6832bed7c61531f9c065/open-sse/services/qwenTokenPlanQuotaFetcher.ts) records the captured QwenCloud console gateway, cookie/`sec_token` split, three personal-plan methods, and optional 5-hour window; its [request and parser fixtures](https://github.com/diegosouzapw/OmniRoute/blob/825f8feea73daead73cf6832bed7c61531f9c065/tests/unit/qwen-token-plan-quota-fetcher.test.ts) distinguish weekly-only, dual-window, expired-session, and token-resolution cases. | [`qwencloud.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/account/qwencloud.rs) uses the session only against fixed QwenCloud origins, performs bounded no-retry reads, and maps the Provider-authored plan and windows into the neutral snapshot. |

When changing an adapter, inspect a new upstream commit, pin the new source link,
update discriminating fixtures, and validate the exact live boundary. Do not cite
an unpinned `main` branch or infer a private endpoint from UI output.

## Failure boundaries

- A missing stored Kimi account or credential is a local configuration error and
  sends no request.
- A missing Grok cached login or an absent, non-string, or unsafe subscription
  tier fails the refresh. An absent or unusable Grok billing log only omits the
  usage window. Yo does not fall back to direct xAI access, read Grok's credential
  file, or expose identity metadata.
- A missing, non-QwenCloud, or expired console cookie fails QwenCloud refresh
  before any model request. Yo never substitutes the stored inference key for
  console authentication, persists the browser session, or sends it to a
  configurable origin.
- QwenCloud does not publish this Personal Token Plan console gateway as a
  stable public API. An upstream console change can require a pinned-source and
  fixture update; Yo fails closed instead of guessing a replacement shape.
- Non-success status, redirect, wrong media type, malformed JSON, missing or
  unsafe Kimi level name, invalid reset time, zero limit, excess rows, or excess
  bytes fails the refresh instead of returning a partial healthy-looking report.
- Secrets are used only for the exact authenticated request and are absent from
  errors, snapshots, text, JSON, and test evidence.
- Account-capacity failure never starts a Session, retries a Provider, or changes
  stored connection state.
