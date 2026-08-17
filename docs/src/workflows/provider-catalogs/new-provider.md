# Add a new Provider

Use this page for Kimi and every later Provider. A new Provider is not just a
new model list: it introduces source authority, endpoint and protocol
semantics, credential use, profile resolution, and durable-state
compatibility.

## Complete the source audit

Answer these questions with official sources before choosing an implementation:

1. Is the model source public documentation, a public API, or an authenticated
   account-scoped API?
2. Does it describe a global product list, a subscription plan, or the exact
   models usable by this credential?
3. Does it provide stable ModelIds plus endpoint, dialect, modalities, tools,
   reasoning behavior, and limits, or only marketing descriptions?
4. What authentication material is required to list and to use a model?
5. Are region, plan, or protocol variants separate catalog profiles?
6. How are removal, deprecation, and a changed field at the same ModelId
   reported?

For Kimi, begin with the official
[Kimi model list](https://platform.kimi.ai/docs/models) and official endpoint
or protocol documentation. Audit whether an authenticated list operation is
account-scoped and sufficiently typed; do not assume Kimi should copy either
OpenRouter's dynamic design or QwenCloud's static design.

## Choose the smallest safe product shape

- Choose runtime discovery only when an authenticated official source can
  safely establish the current account inventory and the fields needed for a
  complete binding.
- Choose a static registry when an official exact allowlist and stable profile
  meaning exist but account-scoped discovery does not.
- Keep explicit manual bindings when neither source is complete enough. A
  convenient list is not a reason to invent capabilities or entitlement.

If the active model-service binding contract does not already cover the
chosen source, profile, availability, or refresh behavior, complete a separate
SOT-first contract Slice and activation before implementation.

## Give the Provider its own boundary

Create one cohesive Provider module under `yo-core/src/model_service`, with
transport and normalization submodules only when each has a distinct
responsibility. Reuse the provider-neutral catalog entry, complete binding,
picker, structural admission, journal, and connection transaction. Do not add a
Provider branch to those shared layers when a typed adapter can supply the
same handoff.

Add `docs/src/workflows/provider-catalogs/<provider>.md` with:

- official source links and what each source proves;
- static or dynamic classification and why;
- accepted profile names, endpoints, and regional boundaries by code owner;
- exact focused validation commands;
- deprecation and refresh procedure; and
- known environmental checks that cannot run in the ordinary baseline.

Keep these facts out of the common guide and other Provider runbooks. Add the
matching Korean Projection and accept its canonical source hash after semantic
translation review.

## Acceptance evidence

The first Provider Slice must prove the happy path and the counterexamples,
not merely parse one sample response:

- exact configured Provider and Account select the intended catalog owner;
- cancellation occurs before secret input and repository mutation;
- a selected complete binding enters the existing recoverable connection
  transaction;
- unsupported rows remain visible but cannot be selected;
- malformed, duplicate, oversized, redirected, stale, or incomplete input
  fails at its owning boundary;
- no secret or private credential revision enters diagnostics or evidence;
- startup and recovery continue to use exact durable bindings; and
- removal from future discovery cannot destroy or silently replace existing
  managed state.
