# Follow Grok ACP upstream

Use this workflow when the installed Grok CLI changes its ACP handshake,
authentication methods, Session lifecycle, updates, or permission requests.
This is operational validation guidance, not a second owner for backend
semantics.

## Scope and ownership

`host:grok` is a delegated agent host. It is not a model Provider and does not
use Yo's external-model credential repository. The independent
[`yo-backend-delegated-grok`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-grok/src/lib.rs)
crate owns `grok agent stdio`, ACP v1 JSON-RPC, authentication, Session
correlation, event translation, permissions, cancellation, and process cleanup.
[`yo-backend`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/foundation/src/lib.rs)
owns the generic `BackendAdapter` lifecycle and bounded process mechanisms.
[`yo-core`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/contract.rs)
specializes that lifecycle as the provider-neutral `AgentBackend` contract and
owns the semantic runtime.

The current adapter uses only the installed CLI's advertised `cached_token`
authentication method. Operators authenticate with `grok login`; Yo does not
read, copy, refresh, or persist that token. Direct xAI API or OAuth integration
is a separate Provider design and must not become a fallback inside this host
adapter.

`yo account grok --refresh` reuses only the initialize-authenticate prefix and
then shuts the child down. It maps the exact `subscription_tier` from the
successful authentication metadata into the provider-neutral account snapshot;
email, team identity, and authentication-mode metadata are ignored. It creates
no Agent Session and sends no prompt. Installed Grok CLI 1.0.5 does not expose
the newer `x.ai/billing` extension, so the command does not infer quota windows
from a plan name.

## Compatibility contract

Keep the adapter fail-closed around the wire surface it consumes:

- initialize with ACP protocol version 1 and empty client capabilities;
- require the installed agent to advertise `cached_token`, then authenticate
  with exactly that method;
- accept only a bounded, non-empty, whitespace-exact, control-free subscription
  tier when producing an account snapshot;
- create with `session/new`, and resume with `session/load` only when the agent
  advertises load support;
- correlate every response, Session update, permission request, and terminal
  prompt result to the active request and Session;
- map text, thought, tool, permission, cancellation, and stop-reason messages
  into provider-neutral backend events; and
- bound messages, queues, request waits, retained stderr, and process shutdown.

Do not infer compatibility from the executable version alone. Inspect the
candidate's ACP behavior and retain deterministic fixtures that distinguish
the admitted shape from malformed, mismatched, or unsupported messages.

For external review, admission v1alpha4 also starts the frozen reviewer profile
with stdin already at EOF:

```text
grok --sandbox read-only --permission-mode dontAsk --tools Read,Grep \
  --no-subagents --disable-web-search agent stdio
```

This bounded startup probe happens before ContextBuild or packet publication.
It sends no prompt, ACP initialize, Session request, or private packet. A
sandbox startup failure is an unavailable host, not a warning and not grounds
for an unsandboxed fallback. Grok 1.0.13 can fail this probe in restricted Linux
containers when its bubblewrap profile cannot mask a container-runtime socket;
the installed version string and writable `~/.grok` alone do not prove that the
reviewer profile can run. Current upstream keeps runtime-socket handling inside
the sandbox implementation, so upgrading the host remains the preferred fix
when a release containing that behavior is available.

## Focused validation

Run deterministic adapter tests first:

```bash
cargo test -p yo-backend-delegated-grok
```

With an installed CLI and an existing `grok login`, verify the real initialize,
cached-token authentication, and cleanup boundary without consuming an
inference Turn:

```bash
cargo test -p yo-backend-delegated-grok \
  local_grok_authenticates_and_shuts_down_without_a_session \
  -- --ignored --nocapture

yo account grok --refresh
```

A real prompt or TUI smoke run consumes external service capacity. Run one only
when Turn-level compatibility needs verification, then record the installed
version, authentication state, exact command, observed route, and any
unverified environments. Finish with the
[Slice-close baseline](../validation/#slice-close-baseline) and obtain
fresh-context review for a changed compatibility boundary.
