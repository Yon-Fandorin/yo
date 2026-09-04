# Inspect account capacity

Use `yo account` when the question is how much Provider-managed account capacity
remains now. Use `yo usage SESSION_ID` when the question is how many request,
output, reasoning, and cache tokens a stored Session observed. The two reports do
not share totals or infer one from the other.

## Public commands

```bash
yo account
yo account kimi
yo account --detail
yo account codex --refresh
yo account grok --refresh
yo account kimi:default --refresh
yo account qwencloud:default --refresh
yo account kimi:default --refresh --format json
yo account codex:you@example.com --refresh
```

Omit `SOURCE` to show every currently supported account-capacity source. A bare
Provider such as `kimi` shows every stored account for that Provider, while
`PROVIDER:ACCOUNT` selects one exact account. Without `--refresh`, the command shows
the locally cached last observation and its `Updated` timestamp for each result. An
account that has never been refreshed is shown as `Not refreshed` with `Never` as its
timestamp. `--refresh` applies to the selected scope and saves successful observations
and their timestamps in the local account-capacity cache. A multi-account refresh is
best-effort: every selected source is attempted, successful results are saved and shown,
and failures are reported together with a non-zero exit status.
Expected per-target refresh failures are part of the selected result format only: text
shows them under `Refresh failures`, and JSON keeps them in its existing `errors` array.
The CLI does not append a duplicate generic stderr error for these expected failures.
Fatal setup, serialization, or output-sink failures still use the normal stderr error path.
A cache-persistence failure after a successful observation remains in the same structured
failure result while the freshly observed in-memory record is still shown.

When a Codex app-server reports an unverified minor version within the supported
protocol major, the refresh still succeeds and emits one compatibility warning.
Text puts it in `Refresh warnings` after the account data; JSON keeps its versioned
stdout document unchanged and writes the warning to stderr. The displayed
`userAgent` is bounded and terminal-safe. A different major or an unparseable
version remains a refresh failure.

Text output uses the detailed view by default when the selected scope resolves to one
account, and a borderless column table when it resolves to multiple accounts. The
compact table keeps `PROVIDER`, `ACCOUNT`, `PLAN`, `LIMITS`, and `UPDATED` aligned;
each limit window uses a one-cell vertical level meter beside its exact remaining
percentage. `--detail` forces the detailed view for any scope, whose limit rows use
the same meter family as a horizontal bar. Rich/ASCII glyphs, meter shape, and
`{label}`/`{meter}`/`{percent}` layout are reusable through `yo-tui::meter`, while
semantic colors remain a presentation-layer decision.
Refresh or detail command suggestions are shown only when useful; JSON output never
includes human-oriented command text.

`codex` and `grok` mean the accounts used by their locally installed delegated
hosts. For account-capacity refresh, both hosts require a valid authenticated email and
use it as the human-readable account label. The stable internal account key is kept separately:
Codex preserves its native account id when one is available, while Grok uses the
verified email as its identity evidence. Either the email label or the internal key can
select the cached result. A first run without a cache shows `Local Codex` or `Local Grok`
with `Account  Not resolved` until it is refreshed; use `yo account PROVIDER --refresh`
to ask the local host for the authenticated account. That unresolved row is not itself a
selectable account, and it is not a literal `current` account name. `kimi:ACCOUNT` names one exact account already stored by Yo with either
the `kimi-code-membership/v1` catalog profile or an exact canonical Kimi Code
complete binding, plus its exact Provider-and-Account credential. The binding
fallback retains connections made before catalog-seed persistence; it does not
admit a custom endpoint. A Kimi Platform API account is a different product and
is rejected before a request. `qwencloud:ACCOUNT` accepts an exact stored Token
Plan connection using the canonical Singapore endpoint. It reads the Personal
Token Plan console with the current QwenCloud browser session; the stored
`sk-sp-*` model-inference key cannot authorize that console surface.

`--refresh` deliberately re-observes the selected account source or sources. No route creates
an Agent Session, sends a model prompt, or falls back to another Provider. Codex starts
its local app-server, initializes it, calls `account/read` and
`account/rateLimits/read` once each, and shuts it down. Grok starts `grok agent stdio`,
initializes ACP v1, authenticates
once with the advertised `cached_token` method, reads the exact
`_meta.subscription_tier` and the required email identity, and shuts it down. A host
response without a valid email fails closed instead of being stored under a shared
default account.
The distributed Grok ACP service does not expose its internal billing extension,
so Yo also reads at most the last 1 MiB of Grok's official `unified.jsonl` and
uses only the newest complete `billing: fetched credits config` event whose
weekly period has not ended. If none exists, Yo still reports the authenticated
plan without inventing a usage window. Kimi first
makes one authenticated `GET /coding/v1/me` for the account level name, then
one authenticated `GET /coding/v1/usages` for its limits. Redirects and retries
are disabled and each successful body is bounded to 1 MiB. Provider plan names
are shown exactly; Yo does not derive them from limit sizes. QwenCloud reads the
stored account-session Cookie only for the exact QwenCloud console origin. It
resolves `sec_token` from that Cookie or one bounded dashboard HTML response,
keeps the result only for the current invocation, then sends the console's
`usage`, `subscription`, and `quota-config` requests concurrently. Redirects and
automatic HTTP retries are disabled, every request has an eight-second deadline,
and every successful body is bounded to 1 MiB. The returned 5-hour window is
optional because the Provider can omit it; the 7-day window and active `specCode`
remain Provider-authored observations.

### QwenCloud console session

Qwen model requests still need only the stored `sk-sp-*` inference key and
matching Token Plan endpoint. Account-capacity refresh instead needs the browser
session that can open QwenCloud Billing. Yo does not install another CLI or use
an Alibaba management profile. It stores this session separately from the model
API key under the same Provider-and-Account record in `credentials.yaml`.

1. Sign in at `https://home.qwencloud.com`, then open **Billing > Subscription**.
2. In browser Developer Tools, open **Network**, reload, select an `api.json`
   request to `cs-data.qwencloud.com`, and copy the complete `Cookie` request
   header. It must contain `login_qwencloud_ticket`.
3. Run the refresh from an interactive terminal:

   ```bash
   yo account qwencloud:default --refresh
   ```

   When the account session is absent, Yo prompts for the complete Cookie with
   terminal echo disabled and persists it locally. Later refreshes reuse that
   value. If QwenCloud explicitly reports that the session expired, Yo prompts
   once for a replacement, persists it, and performs exactly one renewed refresh.
   A non-interactive invocation fails with an actionable error when input is
   required.
4. `sec_token` needs no separate input or stored field. Yo derives it from the
   persisted Cookie when present, otherwise from one dashboard response, and
   discards the derived value when the command ends.

Missing or expired console state affects only this account-capacity refresh. It
never replaces or disables the stored Qwen model API key, sends a model request,
or falls back to another Provider.

Before any Cookie capture, Yo acquires the shared connection-operation lane,
recovers a pending connection operation, verifies both the exact stored Token
Plan binding and its API credential, and captures the credential revision. The
account-session mutation is prepared against that observed revision before the
no-echo prompt or remote refresh. A concurrent credential or session change
therefore reports a conflict instead of being silently replanned or overwritten.

Text output is for people. `--format json` emits one exact result under the versioned
`yo.account-capacity/v1alpha3` schema, or an `accounts` array under the
`yo.account-capacity-list/v1alpha2` envelope for `yo account` and Provider scopes,
even when that scope currently contains one account. The `account` field is the
human-readable label; `accountId` is present when the stable internal key differs.
Each cached result also carries its canonical `observedAt` timestamp. A partial refresh
adds an `errors` array and still exits non-zero. `--detail` affects text only; JSON keeps
its fixed machine-readable shape.
Provider percentages are retained to `0.01%`; whole values omit a noisy `.0`.
Count values are normalized conservatively by rounding used capacity up to that
precision, so displayed remaining capacity never overstates the exact ratio.
Their exact reported `used` and `limit` counts remain on each JSON window.
JSON also retains an optional, allowlisted `providerData` object for native data
that cannot be represented by the neutral snapshot. QwenCloud keeps its exact
reported percentages and reset values, active `specCode`, and active-tier quota
values there; authentication material and unvalidated envelope fields are never
included. Missing data stays absent or Unknown and is not synthesized from
Session token usage.

The earlier `v1alpha1` and `v1alpha2` shapes remain historical contracts. `v1alpha3`
adds the account label/key split and refresh-error envelope to the fractional,
count-preserving, allowlisted-provider-data shape; consumers must dispatch on the
recorded schema.

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
- A missing QwenCloud account session starts one no-echo interactive capture. An
  explicitly expired session starts one replacement capture and at most one
  renewed refresh. Yo never substitutes the stored inference key for console
  authentication or sends the browser session to a configurable origin.
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
