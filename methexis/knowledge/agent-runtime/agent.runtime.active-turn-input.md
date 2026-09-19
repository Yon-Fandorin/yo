---
schema: methexis.knowledge/v1alpha1
id: agent.runtime.active-turn-input
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-004
    revision: sha256:cb4d759cfe26654c7e8d0473ad5f9b350e66397fc480b220e61a19f76f15338f
relations:
  depends_on:
    - agent.runtime.command-event-boundary
---
# Input during an active Turn

## Statement

When the initial TUI has observed `TurnStarted(turn)` and has not yet observed
that Turn finishing, a normal prompt submission MUST be interpreted as an
exact-turn steer intent carrying that `TurnRef`. It MUST NOT use a generic
submission whose meaning can be reclassified from newer worker state. The
agent Session MUST admit the intent only if that exact `TurnRef` remains its
active Turn. If the worker has already applied `TurnFinished`, no Turn is
active, or a different Turn is active, it MUST reject the exact-turn intent
explicitly and MUST NOT reinterpret it as `StartTurn`--even when the frontend
has not yet polled the finishing event.

Dispatch backpressure and retry MUST retain the same exact-turn intent and
`TurnRef`; they MUST NOT turn it into later work. A generic new-Turn submission
path MUST be used only for input that the frontend did not classify against an
observed active Turn. A response to an outstanding approval or agent-requested
input remains an exactly correlated Activity response, not a steer or a new
Turn.

Queueing input for a later Turn is a distinct deferred operation. When the
selected backend cannot steer, `yo-core` MUST return an explicit unsupported
result and MUST NOT silently reinterpret the input as queued work.

## Rationale

Binding an active-Turn submission to the Turn the frontend actually observed
prevents scheduling and polling races from changing temporal intent. Explicit
steer, Activity-response, and queue meanings also prevent hidden backend
capability from deciding whether text corrects current work or starts later
work.

## Nonsecret interview working copies

The initial TUI MUST support a separate editable working copy of an admitted
nonsecret interview. Its identity MUST include the original Session, Turn,
stable interview identity and captured batch revision, plus a durable UUIDv4
copy identity and an independent mutable CAS generation. Question order,
selected options, free text, notes and per-question navigation MUST retain
that revision; a changed or missing question schema MUST NOT reinterpret old
answers against a different batch. While the original Activity is outstanding,
submission MUST remain an exactly correlated Activity response under the
existing live-response admission rules.

After application restart, the TUI MUST offer recovery of the last durably
saved working copy with an explicit saved-state indication. Returning to an
already answered interview MUST create a new editable copy while retaining the
original submitted answers and Session history unchanged. Neither operation
MUST pretend to restore a dead backend RPC, resume an interrupted Turn or undo
previously executed work. Native disk resume alone is not evidence that a
pending question request still exists.

For recovered or previously submitted interviews, the TUI MUST expose an
explicit “send as a new conversation” action. Before dispatch it MUST show
an editable plain-text preview containing the declared original questions and
current answers in their captured order. Additional request context MUST be
user-entered; recovery MUST NOT automatically replay an old conversation,
provider-private payload or backend RPC identity. The explicit action MUST
start a new Session and its initial Turn through the ordinary typed command
boundary, preserving the selected backend and model and their admission
rules. It MUST NOT become an Activity response, exact-turn steer, queued Turn,
implicit resume or provider fallback. An active Turn MUST cause an explicit
busy result while preserving the working copy. Backpressure and retry MUST
retain this distinct new-Session intent and the same immutable preview.

The final preview MUST NOT exceed 64 KiB of UTF-8 text. The first excess byte
MUST prevent dispatch without truncating or deleting the working copy. A failed
or ambiguous dispatch MUST preserve the copy and MUST NOT claim successful
submission or automatically retry. A new-conversation copy may be marked submitted only from the observed
accepted initial StartTurn request. A live interview copy instead uses its exact
final Activity-response request, successful response write and observed response
Activity completion; it MUST NOT require a model accepted-request identity.
Both paths preserve the original history and distinguish submission from Turn
completion.
Reopening a submitted copy MUST remain a deliberate user action.

Missing, unsupported, oversized or incomplete legacy capture MUST remain
explicitly unavailable for complete interview recovery; the TUI MUST NOT infer
unseen questions from presentation or optional backend Request Audit.

## Live secret interview answers

A secret answer MAY be admitted only for a current, exactly correlated Activity
question whose typed backend-neutral presentation explicitly marks it secret.
The answer's content MUST NOT be inspected to infer secrecy. This first profile
admits only free-text secret questions with no choices and no notes. Unsupported,
malformed or oversized secret presentation MUST fail closed before answer input
is enabled and MUST NOT fall back to the ordinary plain-text editor. A frontend
MUST wait for the request presentation before accepting question input, even
when it has already observed the enclosing UserInputRequest Activity start.

The TUI MUST collect a secret through a request-bound editor that does not use
ordinary prompt history, kill/yank state, command or skill expansion, workspace
references, attachments, notes, working-copy edits or preview text. It MUST
render only a fixed public state such as `Not entered`, `Entered`, `Recovery
available` or `Recovered`; rendering, cursor geometry and receipts MUST NOT
reveal the answer or its length. Committed Unicode text and bracketed paste are
literal answer bytes. Embedded newlines, slash-prefixed text, `@` and `$` text
MUST NOT submit or activate another input path. Only an explicit submit key
press for the currently presented request may submit. An individual secret is
limited to 64 KiB of UTF-8 and all live secrets retained for one batch are
limited to 256 KiB; the first excess byte rejects the input without truncation.

Secret recovery is an explicit opt-in on each entered answer and MUST remain off
by default. Enabling it MUST name the local retention boundary before any
durable write and MUST NOT itself submit the answer. Leaving an unsubmitted
question, replacing its request, cancelling or terminating its Turn discards the
process-local value; an opted-in encrypted recovery entry follows its separate
lifecycle below. Without opt-in, restart and interview recovery retain only the
public questions, public answers, submission evidence and `Re-entry required`.

The live response MUST carry the exact answer only to the backend that owns the
same outstanding ActivityRequestRef. Before backend dispatch, existing bounded
backpressure MAY retain that same request-bound intent. Once transport write is
attempted, a failure or disconnect has an unknown delivery outcome: Yo MUST
discard the process-local value, MUST NOT automatically retry it and MUST reject
another response attempt for that request. A successful backend command,
including one that only stages an intermediate answer, permits only a
payload-free semantic receipt to cross the command-commit and Journal boundary;
only the final aggregate response write and final seal establish batch
submission. For a batch containing secret questions, earlier secret answers MAY
remain process-local until that one final response is written and MUST otherwise
be discarded from memory on final success, failure, cancellation or Turn
termination.

## Opt-in encrypted recovery vault

An opted-in answer MUST be stored only in a separate local recovery vault. It
MUST NOT enter the interview working-copy JSON, Session Journal, Request Audit,
ordinary configuration, Provider credential mapping, preview, source export,
logs or diagnostics. A v3 secret working copy MAY retain one independently
random opaque recovery-entry identity and the fixed public state `Recovery
available` for the corresponding question. It MUST contain no ciphertext,
nonce, key material, secret value, hash, plaintext length or backend request
identity. Existing v1 nonsecret and v2 redacted copies remain valid without
migration; neither gains recovery authority.

The first local vault profile uses XChaCha20-Poly1305 with a freshly generated
256-bit recovery key and an independently random 192-bit nonce for every entry.
The authenticated plaintext carries the exact UTF-8 answer and its internal
length, then pads to the fixed 64-KiB answer capacity before encryption so the
entry size does not disclose answer length. Authenticated associated data binds
the closed format version, opaque entry identity, working-copy identity, public
batch fingerprint, question identity and stable destination identity. For a
managed backend that identity MUST include the exact Provider and Model plus a
stable authenticated account identity observed from the live typed destination
evidence; a configured account-slot name alone is insufficient and credential
bytes MUST remain excluded. For a delegated backend it MUST include the exact
host kind, observed Provider and Model, and authenticated account identity from
the same class of live evidence. A backend that cannot supply every applicable
stable non-secret component, including authenticated account identity, is
unsupported for secret recovery. Replacement or
mismatch of any component, or authentication failure, MUST fail closed without
returning partial plaintext or rewriting evidence.

The recovery key is a dedicated raw 32-byte owner-only file beside the selected
absolute Yo configuration path; it is not an API credential. It is created only
on the first opted-in save with no-follow, exclusive creation, mode `0600`, file
fsync and parent-directory fsync. Existing opens require one link, a regular
current-user-owned file, exact mode `0600` and exact length. The vault is rooted
under the separately validated absolute Yo state root in an owner-only `0700`
directory with regular `0600` entries and the same no-follow identity checks.
Relative, missing, substituted, insecure or malformed key and vault paths make
secret recovery unavailable and MUST NOT trigger regeneration over surviving
ciphertext. Key bytes and plaintext never enter command arguments, environment
variables, standard input, child processes, display/debug output or repository
identities.

Publishing an entry and its public working-copy reference is one ordered
operation under the interview repository lease: write and fsync the exclusive
encrypted entry first, publish the generation-CAS working copy second, then
reclaim only a newly orphaned owned entry after a failed publication. Recovery
accepts only a reference published by that exact copy generation. Startup may
remove an owned encrypted entry that no valid working copy references, but MUST
leave unknown, malformed, unsafe or concurrently owned files untouched. A final
successfully sealed batch, an explicit forget action, or a fixed seven-day
expiry removes the public reference before unlinking the encrypted entry.
Deletion makes the entry unavailable but MUST NOT claim physical erasure from
flash media, filesystem history, backups, swap or crash dumps.

Recovery never restores a dead backend RPC. The user MUST first select the
saved copy, then receive a new genuine live secret request whose stable
destination, ordered public batch fingerprint, question identity and secret
presentation match the vault binding exactly. Only that live request may offer
the fixed `Recovery available` state. An explicit recovery action decrypts into
the hidden request-bound editor and changes the public state to `Recovered`;
it does not send. A fresh explicit submit key press sends it through the same
live-response admission as newly typed text. Ambiguous or multiple matches fail
closed. Provider/model replacement, host-account replacement, changed public
wording/options/order, an ordinary new-conversation action and a persisted
StartTurn MUST NOT receive the recovered value.

A working copy containing any secret question continues to reject `send as a
new conversation`; it MUST NOT send an empty answer, mask, recovery marker or
encrypted bytes as a substitute. Loss of the recovery key leaves public copies
readable and explicitly marks their secret recovery unavailable. Yo MUST offer
an explicit forget operation that removes its own public reference and vault
entry without exposing the value. It MUST NOT export, reveal or copy a restored
secret to the clipboard.

Delivery to the requesting backend is intentional. The local recovery profile
keeps the value out of Yo's public state and Journal and protects a copied or
inspected vault only while the recovery key was not also captured. It makes no
location-based backup guarantee: a backup, filesystem view, process or account
that can read both the owner-only recovery key and vault is outside the
protection, including when the selected configuration path places both under one
backup root. Provider retention, transformed, encoded, partial or semantically
derived model or tool output, the system clipboard, process memory, swap, crash
dumps, filesystem history and external backups also remain outside this
Yo-local guarantee. It is not Provider credential storage and does not add OS
keychain behavior.

## Native selected-destination disclosure

When a Yo-managed model uses the separately contracted native secret-request
interaction, the typed secret presentation MUST show the exact Provider and
Model from the current effective binding before entry is enabled. It MUST also
state that explicit submission sends the value to that Provider and Model and
that they may retain it. A fixed host-owned notice, separate from the
model-supplied question, MUST state before entry that submission permits at most
one final assistant answer and then makes the current Session unavailable for any
later Turn, model replacement or resume, including when transport never starts or
no answer arrives. It MUST also state that any final answer is withheld until the
complete response passes an exact byte-for-byte secret-echo check. Account
identity, credentials, endpoint query data and other connection internals MUST
NOT be added to that disclosure. The displayed title, question and purpose are
public model-supplied request material; they MUST remain separate from the hidden
answer.

Each request requires a fresh explicit submit key press after that disclosure.
Ordinary prompt submission, approval, a previous secret receipt and a model
function call alone are not consent. For this native path, Yo MUST withhold the
complete bounded terminal answer from display and storage until a non-empty secret
is absent as one exact contiguous UTF-8 byte sequence; a match yields only a static
redacted failure. This check claims no protection from transformed, encoded,
partial or semantically derived output, which remains outside the Yo-local
guarantee. A backend that cannot provide the typed request, exact destination,
Session-wide terminal barrier, exact-echo exclusion, non-persistent delivery and
no-retry semantics MUST fail closed without opening the secret editor.
