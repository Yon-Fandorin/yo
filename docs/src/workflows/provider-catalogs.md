# Maintain Provider catalogs

Use this workflow to refresh an existing model catalog or prepare a new
Provider such as Kimi. It explains source selection, code ownership, and
validation. The accepted behavior remains owned by the
[model-service binding KnowledgeUnit](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.model.service-binding.md);
do not turn this guide into a second catalog contract.

Provider-specific facts do not belong on this page. Keep mutable URLs, API
shapes, profile names, and focused commands in a separate runbook:

- [OpenRouter](./provider-catalogs/openrouter.md) uses authenticated runtime
  discovery.
- [QwenCloud](./provider-catalogs/qwencloud.md) uses release-known static
  registries derived from official plan allowlists.
- [Kimi](./provider-catalogs/kimi.md) uses authenticated runtime discovery plus
  reviewed execution overlays and an explicit local-private-replay consent.
- [Add a new Provider](./provider-catalogs/new-provider.md) classifies the
  source before choosing either design. Later Providers get their own runbook
  when their design is accepted.

## Choose the source model first

Do not start by copying a model-name table. First determine what the official
source can prove.

| Source shape | Suitable catalog | Important limit |
|---|---|---|
| An authenticated API returns the account's usable inventory and enough typed metadata | Runtime discovery | The response still needs bounded parsing, normalization, and fail-closed availability decisions |
| Official plan documentation publishes an exact allowlist, but no reliable account-scoped inventory API exists | Release-known static registry | Membership describes the plan, not whether this credential is currently entitled |
| Neither source can establish a safe complete binding | Explicit manual binding only | Do not infer endpoint, limits, tools, or entitlement from a marketing name |

If the source category, authentication point, or entitlement meaning changes,
that is a behavioral design change. Update and activate the owning Methexis
contract before changing code. A row-only refresh under an already accepted
profile can remain an ordinary implementation Slice.

## Refresh an existing catalog

1. Read this page, the Provider runbook, the active model-service binding
   contract, and the current implementation once.
2. Capture official evidence without credentials: exact URLs, observation
   date, relevant request shape, and the fields used by yo. Keep raw mutable
   responses bounded and local; bind their conclusions to the accepted commit
   or immutable review packet.
3. Diff the official source against the typed catalog. Classify every row as
   added, removed, renamed, capability-changed, limit-changed, or unchanged.
4. Map only evidence-backed fields. A complete binding includes the normalized
   endpoint and connector plus the resolved model profile; a display label is
   not evidence for runtime behavior.
5. Preserve durable state. Removing a row from the current catalog must not
   rewrite or delete a previously stored managed binding. It only stops that
   row from being offered for a new catalog connection.
6. Keep unavailable but valid inventory visible with a reason when the
   accepted UX requires it. Do not silently hide rows with a Provider-specific
   allowlist merely because yo cannot execute them yet.
7. Run the Provider-specific focused checks, the common connection/startup
   regressions affected by the change, and the Slice-close baseline. Obtain a
   fresh-context review when source interpretation or failure behavior changes.

## Keep the typed boundary complete

Review each field independently instead of treating a Provider response or
documentation row as one trusted blob:

| Concern | Evidence to require |
|---|---|
| Identity | Exact Provider, Account, Model, and catalog-profile identifiers |
| Transport | Normalized HTTPS endpoint, API dialect, and derived connector |
| Modalities | Explicit input and output modalities |
| Agent use | Tool-call support and reasoning presentation |
| Capacity | Positive context and output limits with their exact meaning |
| Runtime policy | Tokenizer, structured parameters, tool policy, and verification profile |
| Availability | A typed enabled or disabled result with a user-readable reason |

Reject duplicate or invalid identifiers, incomplete required metadata, unsafe
endpoints, and oversized inventories. Sort presentation deterministically by
normalized display name and then exact ModelId so source order cannot create
unstable UX or tests.

## Prove the failure boundaries

At minimum, make tests discriminate these mistakes:

- a duplicate, malformed, or unknown row/profile is admitted;
- a disabled row disappears or becomes selectable;
- picker cancellation reads a secret or mutates a repository;
- a dynamic response exceeds its byte, row, redirect, or time bound;
- a static catalog unexpectedly performs network discovery;
- a row removed from the current table makes an existing managed binding
  unreadable, or lets that old row bypass new-connect admission;
- changing any complete-binding field at the same coordinate is treated as
  unchanged; and
- display sorting changes with upstream response or documentation order.

Keep secrets and private credential revisions out of fixtures, diagnostics,
review packets, and captured official evidence.

## Add automation only after repetition

This guide remains the durable entry point because source interpretation and
compatibility decisions require judgment. Extract a repository skill only
after at least two Provider refreshes repeat the same safe mechanics. A skill
may fetch a bounded public source, normalize a candidate table, produce a
field-level diff, and run the documented checks. It must not decide source
authority, silently edit contracts, infer unsupported fields, expose a secret,
or publish a catalog without review.
