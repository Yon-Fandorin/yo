---
schema: methexis.knowledge/v1alpha1
id: agent.observability.view-projections
kind: decision
owner: agent-runtime
sources:
  - id: agent.observability-002
    revision: sha256:697110b69364e987ad433d992418e2fab5cd56fc70962649f110ac2473811553
relations:
  depends_on:
    - agent.observability.session-journal
  constrained_by:
    - agent.core.frontend-independent-boundary
    - tui.crate.ui-only-boundary
---
# Chat, Transcript, Request, and Session Usage projections

## Statement

Chat and Transcript MUST derive their displayed history from the read-only
semantic Session Journal. Request MUST be a read-only diagnostic projection
that joins the Journal's bounded correlation and availability records with
optional Request Audit detail under the same Session lifecycle. Neither the
projection nor its detail may become an independent authority. Chat MUST
remain the editable default interaction surface. Its exposure policy MUST
follow established coding-agent
interaction: show concise intent, meaningful tool and file activity, tests,
approvals, failures, and results while collapsing repetitive exploration and
long output.

Transcript MUST be the transparent chronological superset of Chat and add
detailed semantic and Activity lifecycle, context, failures, and explicit
observation or persistence gaps. An archived Transcript alone MAY select the
newest positive N semantic Transcript records before rendering through an
`Option<NonZeroUsize>` limit. `None` MUST select all semantic Transcript
records. `Some(N)` MUST select the newest N records, preserve their
chronological render order, and retain their original one-based record
numbers.

The archived Transcript alone MAY apply a `none`, `preview`, or `full` content
policy after record selection and before rendering. The policy MUST cover user
input, Activity text delta, Activity text snapshot, Activity failure messages,
and Turn failure messages. For `none`, each covered value MUST emit exactly
`content.type=<type>` and `content.utf8_bytes=<full-byte-count>`, where `<type>`
is one of `user_input`, `activity_text_delta`, `activity_text_snapshot`,
`activity_failure_message`, or `turn_failure_message`. For `preview`, it MUST
also emit `content.preview=<value>` and
`content.preview_truncated=true|false`. The preview value MUST be the
Debug-quoted and escaped form of the longest prefix composed only of complete
extended grapheme clusters whose unescaped UTF-8 encoding is at most 256
bytes. `content.utf8_bytes` MUST remain the byte count of the complete original
value, and `content.preview_truncated` MUST be true exactly when the preview
prefix omits any original bytes. A single extended grapheme cluster larger
than the budget therefore produces an empty quoted preview and reports
truncation. For `full`, rendering MUST preserve the legacy field names and
values byte-for-byte and MUST emit no `content.*` metadata. Omitting the
content selector MUST default to `full`. Any explicitly supplied record limit
or content selector, including an explicit `full`, MUST be accepted only for
Transcript; Chat and Request MUST reject it.

Request MUST be a full-page read-only diagnostic trace that presents all of
the Journal's bounded correlation and availability records for the full
Session in chronological order, not a request-list browser. It MUST show the
observable backend exchange, revisions, attempts, outcomes, redaction, exact
observation boundary, and a typed reason when detail is unavailable. An
interactive surface MAY highlight the context currently viewed in Chat or
Transcript within that trace; a highlighted context with no direct request
MUST say so instead of selecting a nearby request. Returning across linked
views MUST restore each view's cursor and scroll state. A future remote reader
MAY fetch detail on demand only after a real remote consumer defines that
contract; this decision does not create a remote Request Audit interface.

Session Usage MUST be a read-only projection of usage receipts from completed
ModelWork Activities only. The independent top-level `yo usage SESSION_ID`
command MUST be the sole Usage presentation and MUST consume the shared typed
Session Usage projection without independently decoding or aggregating
receipts. Usage MUST NOT be exposed as a CLI or TUI view, `yo session --view
usage` MUST be invalid, and F4 MUST have no view binding. The projection MUST
preserve receipt chronology. Each token aggregate MUST be complete, partial,
or unavailable. Partial and unavailable aggregates MUST expose covered/total
receipt coverage (x/y) so missing values do not appear complete. Cache-read
share MUST include only receipts that explicitly report cache-read token data
and have a known input-token denominator. Its token denominator MUST contain
known input tokens from only those eligible receipts, and it MUST expose
eligible/total receipt coverage. A Session with no recognized completed
receipts MUST succeed with an empty projection. For recognized receipt
schemas, reported zero, absent, and unsupported MUST remain distinct, while
malformed data MUST fail the whole projection closed before any partial report
is emitted. Codex aggregation MUST use per-turn usage only and MUST exclude
cumulative thread_total. Usage MUST NOT infer cost, billing, cache hits,
uncached tokens, missing attribution, or cross-provider cache-write
equivalence, and MUST NOT expose raw request, response, credential, or
private-reasoning content.

## Rationale

One semantic replay source keeps concise work and transparent chronology
aligned, while optional correlated detail permits wire-level diagnosis across
TUI and future GUI frontends without forcing it into the semantic Journal.
Transcript-only record and content selection bounds diagnostic output without
weakening the complete Request trace or changing the legacy explicit `full`
representation. A dedicated Usage command keeps accounting outside live and
archived observability views, while the shared typed projection prevents
duplicate receipt interpretation.
