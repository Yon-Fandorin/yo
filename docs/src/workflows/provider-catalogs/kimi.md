# Maintain the Kimi catalog and connector

Kimi is an authenticated runtime-discovery Provider with separate Platform AI
and Code Membership products. Each product has a small reviewed execution
overlay. The account inventory remains visible, but only rows whose complete
request, stream, limit, and replay behavior Yo knows are selectable. Do not
turn either overlay into a guessed family-name allowlist or mix one product's
endpoint, entitlement, or request policy into the other.

## Official sources

Use Kimi's official product-specific API and model guides:

- [Chat API](https://platform.kimi.ai/docs/api/chat) for the request and
  streaming response shape;
- [Kimi K3 quickstart](https://platform.kimi.ai/docs/guide/kimi-k3-quickstart)
  and [reasoning effort](https://platform.kimi.ai/docs/guide/use-reasoning-effort)
  for K3 limits and reasoning settings; and
- [Kimi K3 tool calling](https://platform.kimi.ai/docs/guide/kimi-k3-tool-calling-best-practice)
  for complete assistant-message replay across tool rounds;
- [Kimi Code models](https://www.kimi.com/code/docs/en/kimi-code/models.html)
  for the Code Membership model IDs, context limits, and recommendation; and
- [Kimi Code documentation](https://www.kimi.com/code/docs/en/) for the Code
  endpoint, preserved-thinking request shape, and session cache affinity.

The Platform profile uses `https://api.moonshot.ai/v1/`; the Code Membership
profile uses `https://api.kimi.com/coding/v1/`. An authenticated `GET models`
result proves only that product Account's current inventory. It does not by
itself define a safe Yo execution profile or entitlement for the other
product. Treat every response byte as untrusted and bounded.

## Code ownership

| Responsibility | Owner |
|---|---|
| Product-specific Account seed, bounded discovery transport, normalization, reviewed overlays, and typed disabled reasons | [`kimi_catalog.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/kimi_catalog.rs) and its `kimi_catalog/` children |
| Exact Platform/Code profile admission, Kimi request and stream grammar, typed private payload codec/projection, and encoded-size accounting | [`connectors/kimi`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/connectors/kimi/src/lib.rs) |
| Opaque provider-private envelope bounds, physical persistence, replay-profile/schema correlation, neutral projection comparison, and Provider-neutral per-Session cache-affinity hint creation | [`yo-backend evidence/replay.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/foundation/src/evidence/replay.rs), [`yo-core backend/evidence.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/evidence.rs), [`journal/codec`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/journal/codec), and [`backends/managed`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/managed/src/lib.rs) |
| Config seed, picker, disclosure, and recoverable connection transaction | [`config.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/config.rs), [`external.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command/connect/external.rs), and [`picker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command/connect/picker.rs) |

## Current update procedure

1. Select the exact configured catalog profile first:
   `kimi-platform-ai/v1` for Platform or `kimi-code-membership/v1` for Code.
   Capture one bounded authenticated `/models` response from that profile's
   endpoint and compare every valid exact ModelId with only that product's
   overlay. The first valid duplicate wins and 4,097 or more rows reject the
   entire snapshot.
2. Keep unknown, retired, or capability-conflicting rows visible and disabled
   with their typed reason. Add a selectable row only after its complete
   connector envelope and replay behavior have an accepted contract.
3. Recheck context evidence independently. Platform K3 accepts remote context
   from 131,073 through 1,048,576 inclusive; Platform K2.7 and K2.6 accept
   32,769 through 262,144 inclusive. Code `k3` accepts the documented 262,144 through
   1,048,576 tiers, while `k3-256k`, `kimi-for-coding`, and
   `kimi-for-coding-highspeed` require exactly 262,144. A remote value outside
   the selected product's reviewed envelope disables the row instead of
   widening local admission.
4. Preserve the exact replay boundary. Every selectable K3 or K2.7 row across
   both products requires `kimi-private-local-plaintext/v1`; Platform K2.6
   remains `semantic-only/v1`. Never infer consent from ModelId or connector.
5. Verify the connection preview still states that bounded Kimi assistant
   state is retained unencrypted in current-user local Session records before
   publishing a managed private-replay binding.
6. Exercise one complete tool round. The visible assistant/function projection
   and one provider-private assistant item must persist atomically, correlate
   to the same binding epoch, and reconstruct the next Kimi request without
   duplicating visible content or tool calls.
7. For Code, verify one Session reuses one opaque cache-affinity hint across
   ordinary and resumed requests. The Connector alone serializes it as
   `prompt_cache_key`; Platform and other connectors ignore it, and no hint enters binding
   identity, replay evidence, logs, diagnostics, transcripts, or traces.

Focused checks:

```bash
cargo test --locked -p yo-connector-kimi
cargo test --locked -p yo-core kimi
cargo test --locked -p yo-core journal::codec::tests::correlation::continuation
cargo test --locked -p yo-core backend::native
cargo test --locked -p yo-cli kimi
```

Do not retain credentials, raw private reasoning, or live account responses in
the repository, review packet, or runbook. A row refresh does not rewrite a
previously stored managed binding, Session record, or consent decision.
