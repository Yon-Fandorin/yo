---
schema: methexis.knowledge/v1alpha1
id: agent.tool.local-execution-boundary
kind: decision
owner: agent-runtime
sources:
  - id: agent.tool-001
    revision: sha256:a3cc254dc0f0b48fb688dacb963db6c7ec551c49ac357db67396ad009cc2840d
relations:
  depends_on:
    - agent.core.frontend-independent-boundary
    - agent.runtime.command-event-boundary
    - agent.runtime.session-turn-activity
  constrained_by:
    - agent.observability.session-journal
---
# Local model-tool execution boundary

## Statement

A Yo-managed model loop MUST expose only tools admitted through a
frontend-independent registry. Each registered tool MUST have a stable
`ToolId`, a unique wire name, safe description, versioned JSON input schema,
typed effect and approval requirements, and an injected execution-host handle.
The registry and admission policy belong to `yo-core`; the execution host owns
the concrete operating-system or remote workspace effect and MUST NOT expose
that effect to a Model Connector.

The effective tool registry MUST be frozen for one model request. The model
MUST receive only its admitted function-tool projection. Provider built-in
tools, provider-hosted code execution, and direct provider MCP execution are
deferred and MUST NOT be enabled implicitly by an OpenAI-compatible endpoint.

A returned function call MUST resolve one exact registered tool and validate
its complete accumulated JSON arguments before approval or execution. Invalid
JSON, a schema mismatch, an unknown or duplicate call identity, an unavailable
tool, or a request that exceeds configured argument bounds MUST become a typed
Tool Activity failure without dispatching an effect. Approval MUST bind the
exact Turn, call identity, ToolId, normalized argument digest, effect class,
and execution host. A stale or mismatched response MUST NOT authorize a call.

After dispatch, one call permits at most one local execution attempt. Timeout,
transport ambiguity, cancellation, executor failure, or lost output MUST NOT
automatically repeat a potentially effectful tool. The executor MUST return a
typed completed, failed, or interrupted result with bounded textual output and
explicit truncation when applicable. The Session Journal MUST correlate the
exact call, approval, execution attempt, and tool result before that result is
eligible for model submission.

Execution progress and absolute work budgets MUST remain distinct. Every
execution host MUST define a finite progress-inactivity deadline and the exact
signal that resets it. The first local command tool MUST treat each non-empty
stdout or stderr chunk as progress, MUST reset a 5-minute inactivity window on
that progress, and MUST fail the attempt when no such output arrives for 5
minutes even if the process is still alive. Agent policy MAY additionally
supply one absolute execution deadline; it MUST default to absent, begin once
for that attempt, and MUST NOT reset on output or other progress. Cancellation
MUST interrupt both waits, and timeout or cancellation MUST use finite
termination, reap, and output-drain bounds. Diagnostics MUST distinguish
inactivity, an agent-supplied absolute deadline, cancellation, and cleanup
failure. None of these outcomes permits an automatic retry.

Calls MUST execute serially in model order by default. They MAY execute
concurrently only when the scheduler proves that approval scopes and mutable
resource leases are disjoint. Result publication and model submission MUST use
stable model-call order regardless of completion order. Cancellation MUST
prevent undispatched calls, request prompt cancellation of active executors,
and preserve an explicit interrupted result when the host cannot prove that an
effect did not occur.

Tool names, schemas, arguments, and outputs are model-visible semantic history
and MUST follow the Session Journal's bounded persistence and redaction rules.
Execution-host diagnostics and prohibited secrets remain outside semantic
history. Exact replay MUST reproduce the recorded function-call and result
relationship without re-executing the historical tool.

The first registry schema dialect is the closed `yo.tool-schema/v1` subset.
Every node requires one of object, array, string, number, integer, boolean, or
null; only `description`, `properties`, `required`,
`additionalProperties`, `items`, and same-type non-empty `enum` are
admitted. Object schemas MUST set `additionalProperties: false`; arrays require
one item schema; required names MUST be unique declared properties; unsupported
keywords and schema/instance nesting beyond 16 fail closed.

Every validation class MUST expose a stable non-null
`yo.tool.validation.*/v1` failure code separately from diagnostic prose. Before
dispatch, raw validated arguments MUST pass an injected semantic-admission gate.
Tool output MUST pass the same gate before it becomes an Activity, later model
input, or replay: the gate may admit it exactly, replace it with one explicit
bounded redacted value, or fail the Turn. Credentials, complete environment
values, execution-host diagnostics, and configured prohibited literals MUST NOT
cross this boundary, and a concrete tool MUST NOT bypass it. Until a concrete
gate is installed, no local tool registry may be exposed to a native model.

The first concrete workspace-file surface under effective `local-tools/v1` is
the distinct registry revision `yo.local-tool-registry/basic-files/v1`. It MUST
expose exact wire names `list_files`, `read_files`, `edit_file`, `write_file`,
and `run_command` in that order. Their exact ToolIds in the same order are
`list-files`, `read-files`, `edit-file`, `write-file`, and `run-command`. A
newly created Session uses this revision.
The immediately preceding three-tool registry is named
`yo.local-tool-registry/legacy-read-file/v1` and continues to expose only
`read_file`, `list_files`, and `run_command`. Its trusted manifest is the exact
ordered sequence below; a durable projection is untrusted candidate data until
it equals this manifest.

1. ToolId `read-file`, wire name `read_file`, exact description `Read one UTF-8 file inside the current workspace.`, schema version `yo.tool-schema/v1`, `ReadOnly`, automatic;
2. ToolId `list-files`, wire name `list_files`, exact description `List files recursively below one directory inside the current workspace.`, schema version `yo.tool-schema/v1`, `ReadOnly`, automatic; and
3. ToolId `run-command`, wire name `run_command`, exact description `Run one shell command in the current workspace after explicit user approval.`, schema version `yo.tool-schema/v1`, `Process`, approval-required.

The first two legacy tools have the exact structural parameter schema
`{"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative path"}},"required":["path"],"additionalProperties":false}`.
The third has exact structural parameter schema
`{"type":"object","properties":{"command":{"type":"string","description":"Shell command to run from the workspace root"}},"required":["command"],"additionalProperties":false}`.
Manifest equality compares tool order and every listed scalar byte exactly and
compares parameter-schema JSON recursively: object-member order is not
semantic, while array order, member names, JSON value kinds, and scalar values
are exact. ToolId, effect, and approval are reconstructed from the selected
trusted manifest; the durable `ModelReplayContract` does not authenticate or
override them. No separately persisted registry digest is used. A running
backend keeps its frozen registry for its lifetime. After restart, resume MUST
select and reconstruct
the one exact known registry whose complete ordered tool names, descriptions,
schema versions, and parameter schemas equal the Session's durable
`ModelReplayContract`; it MUST NOT replace or merge that contract. A legacy
Session therefore resumes with legacy `read_file` and no new write tools. An
unknown or mixed projection opens the saved Session read-only instead of
guessing or silently upgrading. Historical calls remain replay-only semantic
history and are never re-executed. Future
legacy `read_file` calls use this exact retained execution contract. The backend
supplies an exact 4,194,304-byte output bound for that tool. Its sole `path`
string is interpreted as a UTF-8 workspace-relative path: empty, absolute,
parent-traversing, root, and platform-prefix paths fail before a worker starts;
current-directory components are ignored. Starting the worker opens every
component beneath the retained workspace-directory handle with no symlink
following, requires the final descriptor to be a regular file, and rejects the
selected credential's device/inode even through a hard link. Any such path,
open, type, or credential failure produces `ToolExecutionOutcome::Failed`,
exact model-visible output `tool execution failed`, and no filesystem effect.
Operating-system diagnostic text is not retained.

After a successful open, the worker checks cancellation, reads from byte zero
until EOF or 4,194,305 bytes have been observed, and checks cancellation again.
Cancellation observed at either check returns
`ToolExecutionOutcome::Interrupted`, exact output `interrupted`, and
`truncated = false`. A read error returns `Failed`, exact output
`read_file failed`, and `truncated = false`. Otherwise the first at most
4,194,304 bytes must form UTF-8. Failure of that exact prefix to decode returns
`Failed`, exact output `read_file supports UTF-8 text files only`, and
`truncated = false`; bytes beyond that prefix are not decoded. A complete
prefix at EOF returns `Completed`, the exact UTF-8 file bytes, and
`truncated = false`. Observation of byte 4,194,305 returns `Completed` with
the first 4,194,304 bytes and `truncated = true`; before Activity and replay
publication, the common bounded-output owner replaces the tail on a Unicode
scalar boundary so the complete output is at most 4,194,304 bytes and ends in
exact `\n[yo: tool output truncated]`. No metadata-stability snapshot is
claimed for this frozen legacy reader. Historical and future results still pass
the common semantic-output gate. `list_files` and `run_command`
retain their existing behavior. `read_files` is `ReadOnly` and automatic;
`edit_file` and `write_file` are `WorkspaceWrite` and automatic; `run_command`
remains `Process` and approval-required. File deletion and `apply_patch` are
deferred and MUST NOT be inferred from these tools.

The basic registry's exact model-visible tool descriptions are:

- `list_files`: `List files recursively below one directory inside the current workspace.`;
- `read_files`: `Read 1–8 ordered UTF-8 file windows from the workspace; batch related files in one call. Each result is content or a per-file error. Continue unread lines from next_offset.`;
- `edit_file`: `Atomically replace 1–256 unique, non-overlapping exact text matches in one UTF-8 workspace file.`;
- `write_file`: `Atomically create or replace one complete UTF-8 file under an existing workspace directory.`; and
- `run_command`: `Run one shell command in the current workspace after explicit user approval.`.

Its exact input-property descriptions are `Workspace-relative directory path.` for `list_files.path`; `Ordered file windows to read together.`, `Workspace-relative file path.`, `First logical line, 1-based; default 1.`, and `Maximum logical lines, 1–400; default 400.` for `read_files.files`, each item `path`, `offset`, and `limit`; `Workspace-relative file path.`, `Ordered exact replacements matched against the original file.`, `Non-empty text that must occur exactly once.`, and `Replacement text; may be empty.` for `edit_file.path`, `edits`, `oldText`, and `newText`; `Workspace-relative file path.` and `Complete UTF-8 file content.` for `write_file.path` and `content`; and `Shell command to run from the workspace root` for `run_command.command`. These concise strings state the purpose, model-relevant batch and continuation behavior, and field meanings. Examples and host-internal path, race, credential, publication, and cleanup rules stay in this owner and host validation rather than being repeated in every model request.

All five basic tools use schema version `yo.tool-schema/v1`. Their exact structural parameter-schema JSON values are:

- `list_files`: `{"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative directory path."}},"required":["path"],"additionalProperties":false}`;
- `read_files`: `{"type":"object","properties":{"files":{"type":"array","description":"Ordered file windows to read together.","items":{"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative file path."},"offset":{"type":"integer","description":"First logical line, 1-based; default 1."},"limit":{"type":"integer","description":"Maximum logical lines, 1–400; default 400."}},"required":["path"],"additionalProperties":false}}},"required":["files"],"additionalProperties":false}`;
- `edit_file`: `{"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative file path."},"edits":{"type":"array","description":"Ordered exact replacements matched against the original file.","items":{"type":"object","properties":{"oldText":{"type":"string","description":"Non-empty text that must occur exactly once."},"newText":{"type":"string","description":"Replacement text; may be empty."}},"required":["oldText","newText"],"additionalProperties":false}}},"required":["path","edits"],"additionalProperties":false}`;
- `write_file`: `{"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative file path."},"content":{"type":"string","description":"Complete UTF-8 file content."}},"required":["path","content"],"additionalProperties":false}`; and
- `run_command`: `{"type":"object","properties":{"command":{"type":"string","description":"Shell command to run from the workspace root"}},"required":["command"],"additionalProperties":false}`.

The same manifest equality algorithm used for the legacy registry applies to
these five schemas. The host-enforced numeric, item-count, byte, and semantic
bounds below are not extra JSON-schema keywords and MUST NOT be inserted into
the frozen projection.

`read_files` MUST accept one exact `files` array containing 1 to 8 items in
request order. Each item contains required `path` and optional integer `offset`
and `limit`; no other field is admitted. A path is at most 1,024 UTF-8 bytes,
contains no control character, and satisfies the host's workspace-relative
no-parent-traversal policy. Offset is a 1-based logical line and defaults to 1;
limit is positive, defaults to 400, and is at most 400. The schema remains in
the closed `yo.tool-schema/v1` subset, so the host MUST enforce these numeric,
byte, and item-count bounds after complete argument validation and before
opening any requested path. A malformed item rejects the complete call before
any read. Duplicate paths are valid independent windows and preserve their
input positions.

Each requested file is an independent observation, not a multi-file snapshot.
The host MUST resolve it beneath the already opened workspace directory without
following a symlink, accept only one regular UTF-8 file, and reject the
credential identity even through another hard link. Each opened file is capped
at 16,777,216 bytes. The host captures its device, inode, size, modification
time, and change time, reads that one descriptor forward into one bounded byte
image, captures the same metadata again, and accepts the item only when both
captures and the observed byte length agree. A difference yields
`changed_during_read`; the admitted line total, window, and content are then
derived only from that captured image. This detects ordinary concurrent local
filesystem writes but is not an adversarial snapshot primitive for a
filesystem that can change bytes without changing those observations. A path or file failure
after batch preflight produces one bounded item error and MUST NOT discard
successful sibling items. Cancellation, in contrast, MUST discard partial
batch output and return one interrupted call without opening a later item.

Line windows use LF bytes as separators. A final LF terminates its preceding
line and does not create another line; an empty file has zero logical lines;
and every other byte, including a CR before LF, remains content. Offset 1 on an
empty file returns an empty successful `0-0 of 0` window, while every offset
beyond a non-empty file's total and every offset greater than 1 on an empty
file is `offset_out_of_range`.

`read_files` output MUST be one compact UTF-8 JSON value with no insignificant
whitespace or trailing newline: `{"results":[<item>,...]}`. Items preserve
input order. An admitted item uses keys in exact order `path`, `status`,
`start`, `end`, `total`, optional `next_offset`, and `content`; status is exact
`"ok"`. Its `content` string is the exact captured byte span for the selected
complete logical lines, including their original LF terminators. Empty-file
success is the compact object with exact key/value sequence `path:<path>`,
`status:"ok"`, `start:0`, `end:0`, `total:0`, `content:""` and no
`next_offset`.
An item with unread captured lines includes numeric `next_offset = end + 1`;
an item with no unread line omits that key. An error item uses exact key order
`path`, `status`, `error`, status `"error"`, and one class `unavailable`,
`not_regular`, `non_utf8`, `too_large`, `changed_during_read`,
`offset_out_of_range`, or `line_too_large`. Symlink and credential-identity
rejection use `unavailable`; no raw operating-system diagnostic appears.

All JSON strings use the RFC 8259 escapes produced by this closed rule: quote,
reverse-solidus, backspace, form-feed, LF, CR, and tab use their two-byte short
escapes; other U+0000..U+001F scalars use lowercase `\u00xx`; every other valid
Unicode scalar is emitted as its original UTF-8. Rendering first selects at
most the requested `limit`, at most 400 captured logical lines, and the
remaining lines from `offset`. It serializes the complete admitted item and
removes complete trailing selected lines until that item is at most 16,384
bytes. If the first selected logical line cannot fit with all required item
metadata, the item becomes `line_too_large`. `next_offset` is required whenever
the requested limit, the 400-content-line limit, or the byte limit leaves an
unread captured line. The 400-line ceiling therefore applies only to content,
not JSON framing. Each complete item is at most 16,384 bytes; the complete
eight-item wrapper, including seven commas and `{"results":[` plus `]}`, is at
most 131,093 bytes.

`edit_file` MUST accept one path and a non-empty ordered `edits` array of at
most 256 exact objects `{oldText, newText}`. Every `oldText` is a non-empty UTF-8 string and
MUST match exactly once in the same captured original regular file. Matches are
every candidate byte position at which the complete non-empty `oldText` byte
sequence occurs; overlapping occurrences count. Thus `aa` has two matches in
`aaa` and is ambiguous. Matches are computed against that original, MUST NOT
overlap or nest across edits, and are applied from
the end toward the beginning so array order cannot change their locations. An
absent or ambiguous match, overlap, identical complete result, malformed input,
cancellation before publication, unsafe path, non-UTF-8 file, credential
identity, or original larger than 16,777,216 bytes fails without changing the
file. Empty `newText` is valid, but the complete planned result MUST be at most
16,777,216 UTF-8 bytes. Success atomically replaces exactly that file, preserves
its existing permission bits and every unrelated byte, and returns the exact
success result defined below. It MUST NOT create a missing file or parent
directory.

`write_file` MUST accept exact `path` and complete UTF-8 `content` of at most
16,777,216 bytes, and MUST atomically create or replace exactly one regular
workspace file under an already existing parent directory. The named-file publication MAY create and remove its one
collision-safe same-parent temporary entry and create or replace the named
target entry;
those entry operations MAY cause incidental parent-directory metadata changes.
It MUST NOT create, delete, rename, or change permissions of any directory
object. New files use the ordinary process-umask-derived mode and
replacement preserves the permission bits captured with the original. A
symlink, non-regular target, credential identity, unsafe path component,
cancellation that wins before publication, validation failure, or write failure leaves
the prior target unchanged and removes owned temporary state. Success returns
the exact result defined below. Neither workspace mutation tool may write
outside the anchored workspace, follow a target or ancestor symlink, change
more than its named file, publish a partially written file, or retry an
ambiguous attempt. Both mutation paths use the same at-most-1,024-byte,
control-free workspace-relative path admission as `read_files`.

After generic argument and semantic admission dispatches a mutation, a success
MUST return `ToolExecutionOutcome::Completed`, `truncated = false`, and one
compact UTF-8 JSON value with no insignificant whitespace or trailing newline.
`edit_file` uses exact key order and shape
`{"path":<path>,"status":"ok","replacements":<count>}`; `write_file` uses
`{"path":<path>,"status":"ok","bytes":<content-byte-count>}`. A host
failure uses `ToolExecutionOutcome::Failed`, `truncated = false`, and exact key
order `{"path":<path>,"status":"error","error":<class>}`. The closed
execution classes are `unavailable`, `non_utf8`, `too_large`,
`changed_during_read`, `match_absent`, `match_ambiguous`,
`overlapping_edits`, `no_change`, `scratch_unavailable`, `scratch_changed`,
`write_failed`, `publication_failed`, `cleanup_failed`, and
`operation_failed`; narrower operating-system diagnostics MUST NOT cross the
semantic gate.

After dispatch, the following total condition-to-class map is authoritative.
An unsafe, missing, symlink, non-regular, credential, or inaccessible target or
parent is `unavailable`; this includes a missing `edit_file` target and any
target capture, metadata, or read error. A captured edit source that does not
decode as UTF-8 is `non_utf8`; a captured source or complete planned output
beyond 16,777,216 bytes is `too_large`; and a failed second metadata capture
or a changed device, inode, size, modification time, change time, or observed
length is `changed_during_read`. For `edit_file`, inspect edits in request
order against the captured original: the first old text with zero matches is
`match_absent`, and the first with more than one match is
`match_ambiguous`. After every edit has one match, any overlap or nesting is
`overlapping_edits`; an otherwise identical complete result is `no_change`.
Failure of the cryptographic random source, a non-collision scratch-create
error, or exhaustion of all 16 exclusive scratch-name attempts is
`scratch_unavailable`. Failure while writing the owned descriptor or applying
its publication mode is `write_failed`. The final prepublication pathname
being absent, foreign, non-regular, or the credential identity is
`scratch_changed`. Failure of the one atomic rename is
`publication_failed`. A worker, join, or internal host failure not assigned
above is `operation_failed`; this catch-all MUST NOT replace a condition with
a narrower class in this map.

Pre-dispatch schema and semantic admission run first and use their existing
validation codes; this includes argument-count, string, path-byte,
control-character, and input-content bounds. After the instance lock is
acquired, `edit_file` uses this fixed phase order: resolve and capture the
target; enforce source size; read and capture metadata again; reject a changed
capture; decode UTF-8; evaluate each requested match in array order; evaluate
cross-edit overlap; detect an identical result; enforce planned-output size;
allocate, write, and mode the scratch; check cancellation; verify scratch
identity; and rename. `write_file` uses: resolve the existing parent and
optional target while capturing the replacement mode; allocate, write, and
mode the scratch; check cancellation; verify scratch identity; and rename.

The first primary condition observed in that phase order is retained.
Cancellation wins only when it is observed at a required cancellation check
before another primary failure and before rename begins; cancellation observed
after a primary failure does not replace that failure, and cancellation cannot
win after rename begins. The final check observes cancellation before scratch
identity, so simultaneously visible cancellation and scratch change has
cancellation as its primary outcome. Rename success is final Completed and
requires no cleanup. Every other terminal path runs the required cleanup;
failure to close the owned descriptor, classify the scratch pathname, or
unlink the still-owned entry always overrides the retained primary failure or
Interrupted outcome with `cleanup_failed`. This order is exhaustive: an
implementation MUST NOT choose between two execution classes for the same
observed state. Every string uses the `read_files` JSON escape rule, and every
complete mutation result is at most 2,112 bytes. Successful or failed execution
results become the corresponding correlated Tool Activity and durable replay
bytes. Cancellation with successful cleanup instead uses
`ToolExecutionOutcome::Interrupted`, exact output `interrupted`, and
`truncated = false`. Pre-dispatch schema or semantic rejection remains the
existing stable `yo.tool.validation.*/v1` Tool Activity failure and creates no
execution attempt or mutation-result JSON.

One local execution-host instance owns one exclusive in-memory
workspace-mutation lock shared by its `edit_file` and `write_file` attempts. It
is acquired before target capture or temporary-file creation and held through
cleanup or publication, so only those two mutation tools dispatched through
that exact host instance cannot interleave. `run_command`, a second execution
host in the same process, another Yo process, and every non-Yo editor do not
participate and are uncoordinated external publishers for this portable
revision. Each tool plans complete output in a same-parent temporary regular
file. The host tries at most 16 independently generated names carrying at least
128 bits from the operating system cryptographic random source and creates each
candidate with the platform equivalents of `O_CREAT | O_EXCL | O_NOFOLLOW` and
initial mode `0600`; a collision consumes one candidate, never opens or changes
the colliding entry, and exhaustion fails without changing the target. The
temporary pathname MAY be observed by another same-UID process, but the host
MUST NOT expose it in model-visible output. Complete content is written while
the mode remains `0600`. Immediately before publication the host applies the
captured replacement mode or the new-file umask-derived mode; during that final
window a process authorized by the final mode MAY observe the complete content
under the unpredictable temporary pathname, while the target path remains
unchanged. The host retains the created descriptor and its device/inode identity.
Immediately before publication it checks cancellation, then performs a
no-follow metadata lookup of the scratch pathname and requires that it still
names that exact regular, non-credential inode. An absent scratch path or a
foreign replacement enters terminal cleanup with primary class
`scratch_changed`; the observed foreign entry is not selected for unlink, and
the target remains unchanged. Only an exact identity match proceeds to one
atomic same-filesystem rename; that successful rename is the
operation's linearization and publication point. Once rename begins,
cancellation cannot win. Rename success reports Completed. Rename failure leaves
the target unchanged, runs the same cleanup sequence, and returns
`publication_failed` only when cleanup succeeds.

Every path that terminates before successful rename, including cancellation,
write or mode failure, identity mismatch, and rename failure, MUST stop using
the scratch descriptor, close it, and run one bounded cleanup sequence while
still holding the instance lock. Cleanup performs at most one no-follow identity
lookup and one unlink attempt: an absent name is already clean; an entry
observed as the exact owned inode is selected for one unlink; an entry observed
as foreign is not selected. Failure to classify or unlink the observed owned
entry returns `cleanup_failed`, never Completed or Interrupted, and exposes
neither the scratch name nor content. No cleanup step is retried. The earlier
portable same-UID boundary also applies between this cleanup identity check and
path-based unlink: a later foreign replacement is an uncoordinated namespace
publisher outside the guarantee and MUST NOT be described as protected by an
atomic identity-checked unlink. If cleanup succeeds, cancellation returns Interrupted and every
other path returns its primary stable failure class. A `cleanup_failed` result
truthfully means complete content may remain at the unpredictable scratch path,
possibly with its final mode, while the target path remains unpublished. The
instance lock does not bind any uncoordinated publisher named above.
An uncoordinated external content write, replacement, or `chmod` between Yo's
capture and rename may be overwritten at the publication point; the first
version deliberately provides no portable external-writer compare-and-swap and
MUST NOT claim one. Replacement uses the captured permission bits, so an
uncoordinated concurrent metadata change may likewise be overwritten. Portable supported-Unix APIs cannot make that final pathname-identity check
and the following path-based rename one indivisible operation. An uncoordinated
same-UID process that unlinks or replaces the scratch entry after the check is
an external namespace publisher outside this revision's guarantee; at that
point last-publisher-wins applies and success MUST NOT be described as proving
Yo-authored content, regular-file identity, credential identity, or byte-count
authenticity. The unconditional named-file guarantees above apply only while
the scratch pathname remains bound to the retained inode through rename. This
explicit content, metadata, target-entry, and scratch-entry external-publisher
boundary is the same on supported Unix targets and is not a hostile-same-UID
security boundary. A future platform-specific descriptor-anchored publication
revision may close it.

## Rationale

Delegated backends hide tool policy inside another agent host. A native loop
needs an explicit local boundary so model protocol cannot bypass approval,
repeat side effects, or confuse tool completion order with semantic order.
Separating output inactivity from an optional agent-owned absolute deadline
allows productive long commands to continue while still detecting silent
stalls and preserving cancellation. A single bounded batch reader amortizes
model round trips without offering two competing read schemas; exact-edit and
full-write operations give smaller models a compact mutation surface while one
anchored host keeps path, credential, atomicity, and cleanup rules consistent.
