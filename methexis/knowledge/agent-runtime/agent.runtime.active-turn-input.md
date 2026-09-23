---
schema: methexis.knowledge/v1alpha1
id: agent.runtime.active-turn-input
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-004
    revision: sha256:a7e1e03e0f654b5c0aaf3dc97f10ac88a2094fce4751b7c8c9edf791d5c4760f
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

## Contextual unfinished interview drafts

The initial TUI MUST keep at most one editable draft for each admitted live
interview request. The draft preserves the original Session, Turn, stable
interview identity, captured batch revision, ordered questions, answers, notes
and current navigation state independently from submitted Session history. Its
UUID and generation MAY remain internal storage and CAS identities, but MUST NOT
be shown as a user selector or make drafts into a general archive. While the
original Activity remains outstanding, submission MUST remain an exactly
correlated Activity response under the existing live-response admission rules.

A draft is discoverable only in the same selected Session and its matching
workspace context. Returning to that context while its Activity remains live
MUST offer `Continue drafting` or `Discard draft` in place. A process restart or
native disk resume is not evidence that the request remains live. When the
original request is dead, Yo MUST identify the draft as non-submittable and
offer only `View draft` or `Discard draft`; viewing MUST NOT make it editable,
restore the RPC, start a Turn, or convert its answers into a new message.

One successful final Activity response MUST remove the separate draft only
after the answer seal is durable. The exact verified durable answer seal is
authoritative on recovery: before classifying a matching request as live or dead,
Yo MUST treat any leftover draft as cleanup-only and durably delete it. Until
that deletion succeeds, the draft content remains unavailable and the TUI MAY
show only a bounded cleanup diagnostic; it MUST NOT offer continue, view, edit or
submission. Cleanup neither resends an answer nor changes conversation history.
Explicit discard also removes an unsubmitted draft. Submitted interviews remain
in ordinary conversation history and MUST NOT be retained or reopened as draft
documents. The normal UI MUST NOT expose interview UUIDs or global list, recover,
reopen, preview-to-new-conversation, or send-as-new-message operations. A failed
or ambiguous response write with no verified answer seal MUST preserve the
unfinished draft and MUST NOT claim successful submission or retry
automatically.

The recognized contextual draft format is exactly
`yo.interview-draft/v1`. Canonical v1, v2 and v3 working-copy files retain their
separately contracted non-contextual behavior, but contextual draft discovery
MUST NOT list, continue, view, migrate, reopen or submit them. Repository
maintenance MAY inspect canonical v3 only to expire its exact-bound recovery
references. No other schema may be decoded or admitted as a working copy;
unsupported, malformed, incomplete or mismatched records MUST fail closed,
remain untouched with a bounded diagnostic, and MUST NOT be migrated or
promoted. V3 recovery references retain only their exact binding and seven-day
cleanup; they are never promoted into the reusable secret store.

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
render only fixed public state such as `Not entered`, `Entered`, `Saved value
available` or `Recovered`; rendering, cursor geometry and receipts MUST NOT
reveal the answer or its length. Committed Unicode text and bracketed paste are
literal answer bytes. Embedded newlines, slash-prefixed text, `@` and `$` text
MUST NOT submit or activate another input path. Only an explicit submit key
press for the currently presented request may submit. An individual secret is
limited to 64 KiB of UTF-8 and all live secrets retained for one batch are
limited to 256 KiB; the first excess byte rejects the input without truncation.

Leaving the current question without submitting it or replacing its exact
Activity request MUST discard that request's process-local editor value. The
same discard is required on cancellation and Turn termination. A separately
completed durable store publication follows its own lifecycle, but it MUST NOT
preserve or automatically refill the discarded editor value; returning requires
new entry or an explicit saved-value recovery followed by a fresh submit press.

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

## Model-authored local retention offer

A typed managed secret request MAY include one public storage offer. Its absence
means `Use once`: Yo MUST NOT ask a retention question or make a durable write.
Its presence carries a stable public secret scope, one recommendation, an
optional suggested duration and a public reason. These fields describe the
request and may influence presentation only. They grant no storage authority,
MUST NOT contain or be derived from the secret value, and MUST NOT be changed in
response to that value. Delegated backends remain unsupported for this offer
until their typed protocol carries equivalent metadata.

Yo owns the local choice and MUST present `Use once`, `Store for…`, and `Store
until deleted`, with `Use once` selected initially even when the model recommends
another choice. The model recommendation and reason MAY be shown, but the
selection MUST remain local and MUST NOT be returned to the model. `Store for…`
requires the user to choose an integer from 1 through 365 days and MUST show the
resulting expiry before acceptance. `Store until deleted` makes no permanence
claim. Selecting a policy MUST NOT itself submit the secret. A newly entered or
recovered value still requires a separate explicit submit key press for the live
request.

## Encrypted local secret store

Only `Store for…` and `Store until deleted` may create a durable entry. The
secret MUST be stored in a separate encrypted local vault and MUST NOT enter an
interview draft, Session Journal, Request Audit, ordinary configuration,
Provider credential mapping, preview, source export, logs or diagnostics. A
separate public metadata record is one authenticated generation whose state is
either `entry` or `tombstone`. An entry generation MAY contain an internal opaque
generation identity, current entry identity, model-authored scope and display
title, exact non-secret destination binding, retention kind, expiry, and the
immediately preceding generation and owned-entry identities used only for
crash-safe rollback and reclamation. A tombstone contains the same public scope
and destination identity but no current entry or retention value. Every
generation MAY additionally contain an independently random
metadata-authentication nonce and its authentication tag; these are public
integrity material and are distinct from the secret-entry nonce. It MUST contain
no secret ciphertext, secret-entry nonce, key material, secret value, secret
hash, plaintext length or backend request identity.

The closed canonical metadata envelope MUST cover its format version, state,
generation identity, current and predecessor identities, scope, display title,
complete destination binding, retention kind, and the expiry including explicit
absent values. Every replacement or deletion additionally uses a dedicated authenticated
transition barrier at a separate stable path. Its canonical envelope binds a
closed barrier format version, operation kind (`create`, `replace` or
`delete`), scope, complete destination, explicitly absent or exact prior
generation and entry identities, and the exact successor entry generation or
tombstone generation. Yo MUST authenticate an
exact envelope before any field may be listed, matched, expired, deleted or
treated as a current, previous, tombstone or transition state. Failed or
unavailable authentication makes that record unavailable, leaves every involved
file untouched and emits only a bounded public diagnostic.

The model-authored scope is 1–128 ASCII bytes matching lower-case letters,
digits, dot, underscore and hyphen, beginning and ending with a letter or digit.
It is meaningful only under the exact stable destination identity. For a
managed backend that destination MUST include the exact Provider and Model plus
a stable authenticated account identity observed from live typed evidence; a
configured account-slot name alone is insufficient and credential bytes remain
excluded. A backend that cannot supply every component is unsupported for
durable secret storage. At most one current entry exists for one destination and
scope. Replacing it MUST publish and authenticate the new entry and metadata
before removing the previous owned entry; failure leaves the previous entry
authoritative.

The vault uses XChaCha20-Poly1305 with one dedicated randomly generated
256-bit master key and an independently random 192-bit nonce for every entry.
Authenticated plaintext carries the exact UTF-8 answer and its internal length,
then pads to the fixed 64-KiB answer capacity before encryption. Associated data
binds the closed secret-entry format version, opaque entry identity, public
secret scope and stable destination identity. Each metadata generation and every transition barrier, whether `create`,
`replace` or `delete`, uses its own independently random 192-bit nonce with the same master
key and an empty plaintext; its XChaCha20-Poly1305 associated data is the
complete canonical metadata or barrier envelope and its output tag is stored
with that public record. A secret-entry nonce, metadata-generation nonce and
transition-barrier nonce MUST never repeat under the master key. The master key is one dedicated raw 32-byte owner-only file beside the
selected absolute Yo configuration path. It is created only on the first
durable secret save with no-follow exclusive creation, mode `0600`, file fsync
and parent-directory fsync. Existing opens require one link, a regular
current-user-owned file, exact mode `0600` and exact length. It MUST NOT be
regenerated over surviving ciphertext. The vault and its public metadata are
rooted under the separately validated absolute Yo state root in owner-only
`0700` directories with regular `0600` files and the same no-follow identity
checks. Relative, missing, substituted, insecure or malformed key, vault or
metadata paths make durable storage unavailable. Key bytes and plaintext never
enter command arguments, environment variables, standard input, child
processes, display/debug output or repository identities.

Before any create, replace, delete or expiry mutation begins under the
repository lease, Yo MUST inspect the stable transition path. A present valid
barrier MUST be completed through only its exact authenticated recovery flow and
durably removed before a later mutation is considered. A present invalid or
unavailable barrier fails closed and blocks every later mutation without
changing it. A new barrier may be published only while the stable path is absent, using an
atomic destination-must-be-absent rename; a destination-exists result MUST retain
the existing barrier and fail the new operation without overwrite. A platform
without that atomic publication primitive makes durable secret mutation
unavailable.

Publishing or replacing a reusable entry is one ordered operation under its
repository lease and uses stable `current` and `previous` metadata slots for its
destination and scope. Yo MUST first create the new encrypted entry exclusively,
fsync that file and fsync its parent directory. It MUST create the authenticated
successor generation at a unique same-directory temporary name and fsync it.
For every publication it MUST create the exact authenticated `create` or
`replace` barrier at a unique same-directory temporary path, fsync that barrier
file, atomically publish it at the stable transition path with that
destination-must-be-absent rename, and fsync the parent directory. After authentication has established each destination slot is
absent, replacement moves the authenticated `current` generation to `previous`
and then the temporary successor to `current` with atomic
destination-must-be-absent renames, fsyncing the metadata directory after each
rename. A destination-exists result retains the barrier and every file and fails
closed without overwrite. First publication uses
a `create` barrier with explicit absent prior identities and omits only the
`previous` move. Only successful completion of the final directory fsync makes
the successor authoritative. Only afterward may Yo durably remove `previous`,
unlink any prior owned ciphertext and fsync their parent directories; it removes
and directory-fsyncs the create or replace barrier last.

Failure before moving `current` leaves it authoritative. Failure after that move
but before the successor `current` is durably published retains the authenticated
`previous` generation and both ciphertexts. Failure or cancellation while either
rename or directory fsync has an ambiguous outcome MUST retain every slot,
temporary generation and plausible ciphertext and MUST NOT retry automatically.
On startup under the same lease, Yo MUST authenticate any transition barrier,
`current`, `previous`, and every present barrier-named metadata or owned
ciphertext file required for authority or cleanup before using or removing it.
An absent expected file is handled only by the exact state below; any present
file that fails authentication leaves every involved file untouched and blocks
mutation. A valid `delete` barrier suppresses every entry immediately and permits
only completion of its exact deletion flow. A valid `create` barrier permits
only its named successor and requires no `previous`. When its exact `current`
and owned ciphertext are valid, Yo MUST successfully fsync both files and their
parent directories in the new process before the first entry becomes
authoritative. When `current` is absent, Yo MUST abort the unpublished create:
it removes only any present successor metadata and ciphertext files that
authenticate to the identities named by the barrier, fsyncs their parent
directories, then removes the barrier and fsyncs the metadata directory. Any
failure retains the barrier and every remaining file and blocks later mutation;
Yo MUST NOT regenerate or publish the successor automatically. An invalid
`current` fails closed and is not this absent-current cleanup state. A valid
`replace` barrier permits only the identities it names. When its exact
prior generation remains in `current` and `previous` is absent, that prior
mapping remains authoritative and Yo MUST abort the unstarted successor. It MUST
remove only those present successor metadata and ciphertext files that
authenticate to the identities named by the barrier, fsync their parent
directories, then remove the barrier and fsync the metadata
directory; it MUST NOT move the prior mapping or publish the successor. Any
failure retains the barrier and every remaining file, keeps the prior mapping as
the sole authority and blocks later mutation. When both slots match the barrier,
Yo MUST successfully fsync the successor `current` and its exact owned ciphertext
plus both parent directories in the new process before the successor becomes
authoritative or prior state is reclaimed; if that durability resolution fails,
`previous` remains the last authoritative mapping and every file is retained. A valid replace barrier whose
exact successor `current` matches while the named `previous` is absent is the
required post-commit cleanup state. Yo MUST successfully fsync that successor
`current` and its exact owned ciphertext plus both parent directories in the new
process before the successor becomes authoritative. It MUST then complete only
any barrier-named prior-ciphertext cleanup and fsync the vault directory, and
finally remove the
barrier and fsync the metadata directory. It MUST NOT roll back or republish the
previous generation. Any failure retains the barrier and every remaining file
without claiming cleanup complete; a later startup repeats only this same
authenticated completion path. If `current` is absent and the named `previous`
and its exact owned ciphertext are valid, Yo MUST durably republish that prior
generation as `current` with a destination-must-be-absent rename and fsync the
metadata directory, remove only authenticated barrier-named successor metadata
and ciphertext without exposing them, fsync their parent directories, and remove
the replace barrier only after that rollback and cleanup are durable. Any
failure retains the barrier, `previous` and every remaining file. A create
barrier is likewise removed only after its named current publication or
fail-closed cleanup has been directory-fsynced.

When both `current` and `previous` are absent under a valid `replace` barrier,
Yo MUST retain the barrier and every remaining file and fail closed. This is not
a recoverable commit or rollback state even when barrier-named successor
metadata and ciphertext authenticate: Yo MUST NOT publish that successor,
recreate the prior generation, remove either authenticated successor file, or
remove the barrier. A later startup repeats only this same fail-closed
classification and every later mutation remains blocked.

Every other slot or required-file combination under a valid `create` or
`replace` barrier that is not enumerated above is an invalid recovery state.
This includes an exact metadata generation whose required owned ciphertext is
absent or invalid, any otherwise authenticated generation in an unexpected
slot, an unexpected occupied slot, and any barrier-named identity mismatch. Yo
MUST retain the barrier and every file, claim no entry authority, perform no
rename, publication, rollback or cleanup, and block every later mutation. A
later startup repeats only this same fail-closed classification.

An invalid barrier fails closed without slot recovery or file mutation. With no
barrier, only a valid `current` whose exact owned ciphertext also authenticates,
with no `previous`, is authoritative. Valid current metadata with absent or
invalid ciphertext remains unavailable and untouched. Any `previous` slot
without its valid transition barrier, a present but invalid `current`, an invalid
`previous`, or any identity mismatch fails closed and remains untouched. Thus absence or damage cannot be interpreted as permission to
restore an older value. A leftover temporary generation or owned encrypted entry
MAY be reclaimed only after valid durable records prove it unreferenced; unknown,
malformed, unsafe or concurrently owned files remain untouched.

A later genuine live secret request may offer an entry only when its scope and
complete destination identity match exactly. Availability never fills or sends
the value automatically. The user MUST explicitly choose the saved value; Yo
then decrypts it only into the hidden request-bound editor, renders `Recovered`,
and requires a fresh submit key press. Ambiguous matches, authentication failure
or any destination mismatch fail closed without partial plaintext or rewritten
evidence. The current public title, question, purpose, recommendation and reason
remain visible so the user can judge the new request before reuse.

Timed entries become unavailable at their authenticated recorded deadline and
are removed by bounded startup or ordinary store maintenance. Until-deleted
entries survive ordinary request completion, draft cleanup and restart. A
dedicated local Secrets view MUST list only authenticated public metadata
sufficient to distinguish entries, including display title, scope, destination
and expiry or `Until deleted`, and MUST provide explicit deletion without
revealing or copying the value.

Deletion and expiry cleanup MUST first create the exact authenticated `delete`
barrier at a unique same-directory temporary path, fsync that barrier file,
atomically publish it at the stable transition path with the same
destination-must-be-absent rename, and successfully fsync the parent directory.
A valid barrier makes the destination and scope unavailable before any slot
changes. Yo then publishes the barrier's exact authenticated tombstone through
the two-slot protocol and durably fsyncs it as `current`, removes `previous` and
fsyncs the metadata directory, and only then removes referenced ciphertext and
fsyncs the vault directory. Each metadata-slot rename uses the same
atomic destination-must-be-absent rule after authentication established that its
destination is absent; a destination-exists result retains the barrier and fails
closed without overwrite.

Startup with a valid delete barrier resumes only the first incomplete step of
that same sequence. An exact prior entry in `current` with no `previous` is moved
to `previous` and directory-fsynced before the exact tombstone is published. An
absent `current` with the exact prior `previous`, or the exact tombstone in
`current` with that `previous`, continues from tombstone publication or prior
cleanup respectively. The exact tombstone in `current` with no `previous`
continues only ciphertext cleanup. When both slots are absent, Yo removes only
remaining owned ciphertext that authenticates to the identities named by the
barrier and fsyncs the vault directory. Only after `previous` and every named
owned ciphertext are durably absent may it remove the tombstone, fsync the
metadata directory, remove the delete barrier, and fsync that directory again.
If the tombstone was already durably removed, the both-slots-absent state skips
that already complete step. No delete-barrier state restores the prior entry.
Any other slot identity, invalid present record, ambiguous publication or cleanup
failure retains the barrier and every remaining file and claims no successful
deletion; a later startup follows only the same authenticated sequence. Absence
after durable barrier removal means no entry. Deletion MUST NOT claim physical
erasure from flash media, filesystem history, backups, swap or crash dumps.
Unknown, malformed, unsafe or concurrently owned files remain untouched and
produce a bounded diagnostic.

## Legacy v3 encrypted recovery

A canonical v3 working copy MAY contain one independently random opaque
recovery-entry identity and fixed public `Recovery available` state for a secret
question. It contains no ciphertext, nonce, key material, secret value, hash,
plaintext length or backend request identity. Its legacy encrypted entry uses
the same owner-only master key and fixed-size XChaCha20-Poly1305 plaintext
profile, but its associated data binds the legacy format version, opaque entry
identity, working-copy identity, captured public batch fingerprint, question
identity and complete stable destination identity. New reusable entries MUST NOT
reinterpret or rewrite that associated data.

Publishing a legacy entry and copy reference remains one ordered operation under
the interview repository lease: write and fsync the exclusive encrypted entry
first, publish the generation-CAS working copy second, then reclaim only a newly
orphaned owned entry after failed copy publication. Recovery accepts only a
reference published by that exact copy generation. Startup MAY remove an owned
legacy encrypted entry that no valid working copy references, but MUST leave
unknown, malformed, unsafe or concurrently owned files untouched. A final
successfully sealed batch, explicit discard or forget, or fixed seven-day expiry
removes the public reference before unlinking its encrypted entry.

Legacy recovery never restores a dead backend RPC and never enters the reusable
Secrets view. Only a new genuine live secret request whose complete destination,
ordered public batch fingerprint, question identity and secret presentation
match exactly may offer that saved value. Explicit local recovery decrypts into
the hidden request-bound editor and still requires a fresh submit key press.
Ambiguous matches, missing key material, authentication failure, destination
replacement, changed public batch or question, an ordinary new-conversation
action and persisted StartTurn MUST NOT receive the value. A legacy working copy
containing any secret question continues to reject every send-as-new-message
path and MUST NOT substitute an empty answer, mask, marker or ciphertext.

Delivery to the requesting backend is intentional. This local store keeps the
value out of Yo's public state and Journal and protects a copied or inspected
vault only while the separate recovery key was not also captured. A backup,
filesystem view, process or account that can read both is outside the guarantee.
Provider retention, transformed, encoded, partial or semantically derived model
or tool output, the system clipboard, process memory, swap, crash dumps,
filesystem history and external backups also remain outside this Yo-local
guarantee. This feature is not Provider credential storage and adds no OS
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
