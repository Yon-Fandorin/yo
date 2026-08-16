# Maintain the Kimi catalog and connector

Kimi is an authenticated runtime-discovery Provider with a small reviewed
execution overlay. The account inventory remains visible, but only rows whose
complete request, stream, limit, and replay behavior Yo knows are selectable.
Do not turn the overlay into a guessed family-name allowlist.

## Official sources

Use Kimi's official Platform API and model guides:

- [Chat API](https://platform.kimi.ai/docs/api/chat) for the request and
  streaming response shape;
- [Kimi K3 quickstart](https://platform.kimi.ai/docs/guide/kimi-k3-quickstart)
  and [reasoning effort](https://platform.kimi.ai/docs/guide/use-reasoning-effort)
  for K3 limits and reasoning settings; and
- [Kimi K3 tool calling](https://platform.kimi.ai/docs/guide/kimi-k3-tool-calling-best-practice)
  for complete assistant-message replay across tool rounds.

The authenticated `GET https://api.moonshot.ai/v1/models` result proves the
Account's current inventory. It does not by itself define a safe Yo execution
profile. Treat every response byte as untrusted and bounded.

## Code ownership

| Responsibility | Owner |
|---|---|
| Account seed, bounded discovery transport, normalization, reviewed overlays, and typed disabled reasons | [`kimi_catalog.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/kimi_catalog.rs) and its `kimi_catalog/` children |
| Exact Kimi request and streamed assistant-message grammar | [`kimi_request.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_connector/kimi_request.rs), [`connector.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_connector/connector.rs), and [`chat_sse.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_connector/chat_sse.rs) |
| Provider-private replay validation, persistence, correlation, and native reuse | [`backend/evidence/replay.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/evidence/replay.rs), [`journal/codec`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/journal/codec), and [`backend/native`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/native/mod.rs) |
| Config seed, picker, disclosure, and verified connection transaction | [`config.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/config.rs), [`external.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/external.rs), and [`picker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/input/picker.rs) |

## Current update procedure

1. Capture a bounded authenticated `/models` response and compare every valid
   exact ModelId with the current overlay. The first valid duplicate wins and
   4,097 or more rows reject the entire snapshot.
2. Keep unknown, retired, or capability-conflicting rows visible and disabled
   with their typed reason. Add a selectable row only after its complete
   connector envelope and replay behavior have an accepted contract.
3. Recheck context evidence independently. K3 may use a positive remote value
   through 1,048,576; reviewed K2.7 and K2.6 rows may use a positive value
   through 262,144. A remote value outside the reviewed envelope disables the
   row instead of widening local admission.
4. Preserve the exact replay boundary. K3 and the two reviewed K2.7 coding
   variants require `kimi-private-local-plaintext/v1`; K2.6 remains
   `semantic-only/v1`. Never infer consent from ModelId or connector.
5. Verify the connection preview still states that bounded Kimi assistant
   state is retained unencrypted in current-user local Session records before
   publishing a managed private-replay binding.
6. Exercise one complete tool round. The visible assistant/function projection
   and one provider-private assistant item must persist atomically, correlate
   to the same binding epoch, and reconstruct the next Kimi request without
   duplicating visible content or tool calls.

Focused checks:

```bash
cargo test --locked -p yo-core kimi
cargo test --locked -p yo-core journal::codec::tests::correlation::continuation
cargo test --locked -p yo-core backend::native
cargo test --locked -p yo-cli kimi
```

Do not retain credentials, raw private reasoning, or live account responses in
the repository, review packet, or runbook. A row refresh does not rewrite a
previously stored managed binding, Session record, or consent decision.
