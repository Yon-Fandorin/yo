---
schema: methexis.knowledge/v1alpha1
id: agent.tool.local-execution-boundary
kind: decision
owner: agent-runtime
sources:
  - id: agent.tool-001
    revision: sha256:c7b1e5e6af8c2a106255c4c5aa7eed0edb00bb5f182be49409c418796cc78412
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
`list-files`, `read-files`, `edit-file`, `write-file`, and `run-command`.
Sessions recorded under this `yo.local-tool-registry/basic-files/v1` revision
retain this exact manifest; future Session selection is defined by the
versioned successor below.
The immediately preceding three-tool registry is named
`yo.local-tool-registry/legacy-read-file/v1` and continues to expose only
`read_file`, `list_files`, and `run_command`. Its trusted manifest is the exact
ordered sequence below; a durable projection is untrusted candidate data until
it equals this manifest.

1. ToolId `read-file`, wire name `read_file`, exact description `Read one UTF-8 file inside the current workspace.`, schema version `yo.tool-schema/v1`, `ReadOnly`, automatic;
2. ToolId `list-files`, wire name `list_files`, exact description `List immediate children of one directory inside the current workspace.`, schema version `yo.tool-schema/v1`, `ReadOnly`, automatic; and
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
override them. These existing fixed registries use no separately persisted registry
digest. A running backend keeps its frozen registry for its lifetime. For these fixed
registries, after restart resume MUST select and reconstruct
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
the common semantic-output gate. `list_files` and
`run_command` retain their existing v1 behavior. `read_files` is
`ReadOnly` and automatic;
`edit_file` and `write_file` are `WorkspaceWrite` and automatic; in these v1 registries `run_command`
remains `Process` and approval-required. File deletion and
`apply_patch` are deferred and MUST NOT be inferred from these file tools.

The basic registry's exact model-visible tool descriptions are:

- `list_files`: `List immediate children of one directory inside the current workspace.`;
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

`list_files` MUST inspect only the immediate children of the selected directory.
The worker fixes that starting directory with one no-follow handle and MUST NOT
open any child directory or regular file. Before that open, the complete
`list_files.path` string MUST pass the shared at-most-1,024-UTF-8-byte,
Unicode-control-free, non-empty workspace-relative no-parent-traversal
admission. Absolute and platform-prefix paths are rejected; current-directory
components are ignored and may select the workspace root. Exact raw names `.`
and `..` are ignored and do not consume the entry budget; every other
dot-prefixed name is ordinary except that exact basename `.git` is excluded.
In directory-iteration order, the worker retains the first at most 100,000
other raw names and may
observe at most one further such name solely as an over-bound probe. Observing
that probe sets `truncated = true`, ends enumeration, and never classifies the
probe. Reaching EOF after exactly 100,000 retained names is not entry-bound
truncation. Every retained raw name, including `.git`, an unrepresentable name,
or an eventually excluded type, consumes this budget. The retained raw names
are then ordered by unsigned lexicographic `OsStr` byte order. The selected
over-bound subset therefore depends on filesystem iteration order; neither a
globally sorted subset nor a complete point-in-time snapshot is promised.

After ordering, exact `.git` is omitted without classification. Every other
retained name must decode as exact UTF-8 and contain no Unicode control scalar
before it is model-visible or classified. A name that fails either condition
is omitted and sets `truncated = true`; no lossy replacement or byte-escape wire
exists. Each remaining name receives at most one
`fstatat(..., AT_SYMLINK_NOFOLLOW)` classification. A regular file is rendered
as its workspace-relative path plus LF; a directory is rendered as its
workspace-relative path, exact `/`, and LF. Before publication, the complete
rendered path token, including a directory's `/` suffix but excluding the LF
separator, MUST itself remain at most 1,024 UTF-8 bytes and control-free. An
over-bound token is omitted and sets `truncated = true`. Symlinks and every
other special type are omitted. Because both the selected path and emitted
component pass the shared admission, every published path line is unambiguous
and can be supplied unchanged to the
control-free UTF-8 basic-file path boundary. If a child disappears between
enumeration and classification, exact `ENOENT` omits only that child. Any
observed directory-read failure or any other metadata failure returns
`ToolExecutionOutcome::Failed`, exact output `list_files failed`, and
`truncated = false`, discarding partial output. Cancellation observed at a
required check similarly discards partial output and returns
`ToolExecutionOutcome::Interrupted`, exact output `interrupted`, and
`truncated = false`.

An otherwise successful observation returns `ToolExecutionOutcome::Completed`.
With no omitted unrepresentable name, no omitted over-bound rendered path token,
no over-bound probe, and every rendered line admitted by the caller's byte
bound, its output is the complete ordered path lines and `truncated = false`.
Otherwise it returns `truncated = true`.
The list worker, not the generic scalar-prefix truncator, owns the path-line
boundary: it retains only the maximal leading sequence of complete path lines
whose bytes plus exact common marker `\n[yo: tool output truncated]` fit the
caller bound. The common bounded-output owner then appends that marker without
cutting the reserved path prefix. When the caller bound is at most the marker's
byte length, the path prefix is empty and the final model-visible value is the
first caller-bound bytes of the ASCII marker; otherwise every model-visible
path portion consists only of complete LF-terminated lines followed by the
complete marker. A caller-bound line that cannot be admitted, an
unrepresentable retained name, an over-bound rendered path token, or the
over-bound probe therefore has one explicit incomplete result and never
produces a partial or ambiguous path. Both the basic and preceding
three-tool manifests use this one shallow behavior and the exact description
above. A saved Session carrying the retired recursive description is an unknown
projection and opens read-only; no recursive compatibility registry or
migration branch is retained.

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


## Backend-owned secret interaction tool

The native secret interaction is model-visible but is not a local execution
tool. New `local-tools/v1` Sessions use the two-definition native interaction
suffix specified below; `request_secret_input` remains its final definition
in both request exposure and the ModelReplayContract. Its exact wire name is
`request_secret_input`; its exact description is `Ask the user for one secret value that Yo sends only to the current provider and model. Use only when the task cannot continue without it.` Its schema version is `yo.tool-schema/v1` and
its exact structural parameter schema is
`{"type":"object","properties":{"title":{"type":"string","description":"Short public title for the secret request."},"question":{"type":"string","description":"Public question shown before secret entry."},"purpose":{"type":"string","description":"Public reason the current provider and model need the secret."},"storage_offer":{"type":"object","description":"Optional public proposal for local reuse of this secret.","properties":{"scope":{"type":"string","description":"Stable public lower-case identifier for this secret within the current destination."},"recommendation":{"type":"string","description":"Model recommendation; the user still chooses locally.","enum":["use_once","store_for_days","store_until_deleted"]},"reason":{"type":"string","description":"Public reason for the retention recommendation."},"suggested_days":{"type":"integer","description":"Suggested duration only when recommending store_for_days."}},"required":["scope","recommendation","reason"],"additionalProperties":false}},"required":["title","question","purpose"],"additionalProperties":false}`.

The interaction has no ToolId, ToolEffect, ToolApprovalRequirement, execution
host, command manifest entry or local execution result. The backend owns its
separate admission and response path. `title` MUST contain 1–80 UTF-8 bytes and
no control character. `question`, `purpose`, and a present storage-offer `reason`
MUST each contain 1–4096 UTF-8 bytes, no NUL and no control character other than
tab, LF or CR. A present `scope` MUST contain 1–128 ASCII bytes, use only lower-
case letters, digits, dot, underscore or hyphen, and begin and end with a letter
or digit. `suggested_days` MUST be present exactly when `recommendation` is
`store_for_days` and MUST be an integer from 1 through 365; it MUST be absent for
the other recommendations. The first invalid or excess byte rejects the call
before a request opens.

The title, question, purpose and complete storage offer are public Activity and
current-Turn model input. They MUST NOT contain the answer, a default value,
answer choices, notes or an answer placeholder. The optional offer is advisory:
it authorizes no persistence, receives no retention-choice result, and MUST NOT
be inferred from free-form prose. Its absence selects the no-persistence path.

`request_secret_input` is reserved: configured command tools MUST reject its
wire name and the corresponding reserved identity before backend publication.
The interaction is absent from `no-tools/v1`, summary requests and every exact
known historical replay projection that does not contain it. Resume and fork
MUST reconstruct a known recorded projection exactly and MUST NOT add or revise
the interaction in an older fixed or configured registry. The prior exact
title/question/purpose-only definition has structural parameter
schema
`{"type":"object","properties":{"title":{"type":"string","description":"Short public title for the secret request."},"question":{"type":"string","description":"Public question shown before secret entry."},"purpose":{"type":"string","description":"Public reason the current provider and model need the secret."}},"required":["title","question","purpose"],"additionalProperties":false}`.
It remains a known historical projection and implies no storage offer.
Because the replay contract has no independent
interaction-revision field, absence alone MUST NOT be treated as proof that a
new projection lost the definition. A historical secret-only projection MUST
retain its one exact final secret definition. The closed current and historical suffix rules below
also admit the new ordinary-question interaction; a reordered, duplicated,
altered or otherwise unknown suffix MUST remain unknown. Configured-command
execution-definition manifests continue to cover only local execution tools, so the appended interaction does not change or
enter their digest.

A response may use the interaction only as the sole function call in that model
round. Another function call in the same response, a second interaction in the
Turn, or any function call after secret dispatch is a protocol failure with no
local effect. The correlated secret value returned to the model follows the
managed loop's connector-only terminal-continuation contract and is expressly
excluded from the ordinary rule that local tool outputs become semantic replay.
The user's local retention selection is never included in that result.

## Backend-owned ordinary question interaction

For newly created tool-enabled `local-tools/v1` Sessions, the exact ordered
native interaction suffix MUST be `ask_user` followed by the current
`request_secret_input` definition. Both follow every frozen local or configured
tool in request exposure and the ModelReplayContract. The exact `ask_user`
description is `Ask the user one question and wait for a response. Use for a missing preference or decision. Unanswered provides no answer or permission; answer-dependent decisions remain unresolved.`. Its schema version is `yo.tool-schema/v1` and
its exact structural parameter schema is
`{"type":"object","properties":{"title":{"type":"string","description":"Short public title for the question."},"question":{"type":"string","description":"Question to show the user."},"choices":{"type":"array","description":"Optional ordered choices; the user can always answer with text.","items":{"type":"object","properties":{"label":{"type":"string","description":"Short choice label."},"description":{"type":"string","description":"Public explanation of the choice."}},"required":["label","description"],"additionalProperties":false}}},"required":["title","question"],"additionalProperties":false}`.

`ask_user` is a backend-owned ordinary interaction, not a local execution tool.
It has no ToolId, effect, approval, execution host or command-manifest digest
entry. Its wire name and corresponding identity MUST be reserved against local
and configured tools. Its public arguments contain no secret or default answer.
The tool imposes no question-frequency rule; the model chooses when and what to
ask from task context.

The complete UTF-8 argument string MUST contain at most 16,384 bytes. `title`
MUST contain 1–80 UTF-8 bytes and no control character. `question` MUST contain
1–4096 UTF-8 bytes; each choice `description` MAY be empty and MUST contain at
most 512 UTF-8 bytes. Those text fields MUST contain no NUL or control character
other than tab, LF or CR. A present `choices` MUST be an array of zero through
eight objects with exactly `label` and `description`; labels MUST be distinct,
each contain 1–80 UTF-8 bytes and contain no control character. Omission or an
empty array means free text. Null, unknown fields and the first excess byte or
choice MUST reject before any question opens or local effect occurs.

`ask_user` MUST be the sole function call in its complete successful model
response. A mixed local-tool/question or secret/question response MUST fail
before any local execution or question admission. Ordinary visible assistant
content and a valid required provider-private replay envelope remain admitted.
The same Turn MAY ask another question in a later model round. The interaction
is absent from `no-tools/v1` and summary requests.

The returned function output is compact UTF-8 JSON with exactly one of these
closed shapes, preserving the original function `call_id` through the existing
FunctionCallOutput item:

- Free text: `{"schema":"yo.ask-user-result/v1","status":"answered","kind":"text","text":"<exact submitted text>"}`.
- Choice: `{"schema":"yo.ask-user-result/v1","status":"answered","kind":"choice","choice":1,"label":"<exact chosen label>","notes":"<exact submitted notes>"}`;
  `choice` is the valid one-based ordinal of that outstanding question.
- No answer: `{"schema":"yo.ask-user-result/v1","status":"unanswered"}`, with
  no text, choice, label, notes, permission or nullable placeholder.

Free-text answers MUST contain 1–16,384 UTF-8 bytes; notes MAY be empty and MUST
contain at most 16,384 UTF-8 bytes. These are ordinary user text: preserve their
exact bytes, without trimming or interpreting them as commands. Images, resolved
skills, secrets and previous-question navigation are unsupported. No answer
supplies no decision or permission; answer-dependent decisions remain unresolved.
The model may continue independent work, but Yo does not claim to determine
semantic independence. There is no implicit timeout or automatic choice.

The closed replay suffix set is exactly: no interaction; the preceding
title/question/purpose-only secret definition; the current secret-only
definition; or exact `ask_user` followed by the current secret definition.
Each suffix MUST match complete ordered definitions, including descriptions and
structural schemas, after a known exact local-registry prefix. Resume, fork and
binding replacement MUST preserve the selected recorded form and MUST NOT inject
`ask_user` into a historical Session. Duplicate, reordered, changed, unknown or
partial suffixes MUST reject. Configured command digests remain local-execution
only. The absence of an interaction-revision field does not authorize inferring
the age of a suffix or relabeling a historical manifest.

## User-configured command tools

The optional `tools.commands` configuration MUST define an explicit ordered list of
zero to sixteen command tools. Missing `tools` or `commands` means empty. Whole-field
null, duplicate or unknown configuration fields fail structural admission. Null inside
a parameter-schema enum retains the existing schema grammar. Each command has
required `id`, `name`, `description`, `executable`, `parameters` and optional `script`,
`executable_args`, `argv`; both argument arrays default to empty and absent script is
None. The existing ToolId, wire-name, safe-description and `yo.tool-schema/v1` limits
apply. Duplicate IDs/names, including collisions with any BasicFiles built-in, fail.
There is no directory scanning, PATH lookup, skill-triggered registration, implicit
shell, model interpolation, environment map, cwd override or configuration-defined
effect/approval. Every configured command is Process and approval-required.

Executable MUST be an absolute UTF-8 path. An explicit absolute script MAY select a
host artifact outside the workspace; a relative script MUST remain beneath the saved
Session workspace. Configured and resolved locators are each at most 4096 UTF-8 bytes,
nonempty and Unicode-control-free. An installed executable symlink is permitted only
when its configured mapping is resolved to and verified against one regular-file target;
resolution MUST terminate within forty followed links. Script paths MUST have no symlink
components. Opened descriptors pin the traversal and require regular files, bounded
actual bytes and stable pre/post-read metadata. The selected credential device/inode is
forbidden even through a hard link. FIFO, device, directory, missing, over-bound and
observably changed artifacts fail admission without launch. OS metadata is a read-pass
consistency check, not persisted execution identity.

The launch is exactly configured executable, then `executable_args`, then the resolved
script path when present, then `argv`. Both arrays share at most 32 entries and 16384
UTF-8 bytes; each entry is at most 4096 bytes and contains no NUL. Fixed arguments are
literal data. Scripts use explicit interpreters, including Python, Node or a shell;
Yo MUST NOT infer a shebang, relocate artifacts, rewrite argv[0] or substitute the model's
argument text into command-line positions. Native tools omit script. Original interpreter,
script and native executable path semantics MUST be retained.

The selected Session workspace is cwd. The initial sanitized environment is exactly
`PATH=/usr/local/bin:/usr/bin:/bin` after clearing the inherited environment. Its values
are not copied from the caller or persisted. Descriptions and parameter schemas pass the
existing semantic-admission policy before model exposure. Launch arguments, artifact
paths/bytes and raw host diagnostics MUST NOT leak through validation or execution
failures. Stdout/stderr use the existing semantic output admission before Activity,
retained presentation, model replay or Journal publication.

Startup MUST capture and hash the admitted execution definition in cancellable preparation
before backend publication. Approval binds its digest through execution-host identity
and the existing exact Turn, call, ToolId, normalized argument digest and effect scope.
A running backend retains its frozen manifest. Config edits cannot add, replace or remove
its tools. Observed artifact changes fail the attempted call, never update expected hashes.
No execution-definition snapshot, artifact bytes, environment values or launch argv are
persisted separately. Model-visible definitions remain in ModelReplayContract and the
manifest digest is carried by the managed Session identity defined by
`agent.model.session-selection`. Exact resume/fork requires both that digest and the
recorded model projection; historical calls/results remain replay-only data.

Identity covers the complete execution definition and observed primary executable/script
bytes, not transitive imports, shared libraries, subprocesses, mutable workspace data,
network services or deterministic effects. Installed artifacts and their publishers are
user-controlled host software. The final-check-to-path-execution race against an
uncoordinated publisher remains outside the guarantee; no atomic execute-from-hashed-bytes
claim is permitted.

### Execution-definition digest

The manifest is a closed JSON-domain object with exactly `profile`, `registry`, `tools`,
and `protocols`. Profile is `yo.execution-definition-manifest/v1`; registry is
`yo.local-tool-registry/command-tools/v1`. `tools` is the ordered five built-ins followed
by 1–16 configured commands. Each item has exactly `id`, `name`, `description`,
`schema_version`, `parameters`, `effect`, `approval`, and `launch`. Schema version is
`yo.tool-schema/v1`; effects are the exact strings `ReadOnly`, `WorkspaceWrite`, `Process`;
approval is `Automatic` or `Required`. Built-in launch is null and the other fields equal
the current trusted BasicFiles manifest. Command effect/approval are Process/Required.

Command launch has exactly `executable`, `script`, `executable_args`, `argv`.
Executable and a present script each have exactly `configured`, `resolved`, `sha256`;
Configured/resolved locators follow the path admission above; sha256 is `sha256:` plus
64 lowercase hex digits of the primary file bytes.
Absent script is null. No environment values, file bytes or OS metadata are stored in
this definition. OS identity/metadata is retained only by the current admission pass
for consistency checks; inode numbers must not make a recreated identical installation
incompatible. Configured locator, resolved locator, primary bytes and fixed arguments
remain definition identity and do change its digest.

Protocols is exactly `{stdin:"yo.command-json-stdin/v1",output:"yo.command-text-output/v1",
environment:"yo.command-safe-environment/v1",runner:"yo.command-execution/v1"}`.
These names freeze the stdin bounds/LF, existing run_command output framing and semantic
admission, existing explicit sanitized environment policy, and verification/inactivity/
absolute-deadline/cancel/reap/drain behavior defined by this command-tool contract. A semantic
change to those policies requires a new protocol revision and manifest digest.
The manifest's encoded digest input is limited to 2 MiB including its domain prefix.
The existing 1 MiB config-file cap, per-schema 64 KiB cap, and complete model replay
contract/request budgets also apply; none is widened to accommodate a manifest.

Digest input starts with exact ASCII `yo.execution-definition-manifest/v1` followed by
one zero byte, then one recursively framed manifest value. Framing is independent of
JSON object insertion order and textual whitespace:

- Null: one byte 0x00. False: 0x01. True: 0x02.
- Integer: 0x03, unsigned 64-bit big-endian byte length, then minimal ASCII decimal
  spelling (zero is `0`, negatives have one `-`, no `+` or leading zero). The admitted
  integer domain is the existing serde_json signed-i64/unsigned-u64 domain.
- Floating JSON number representation: 0x04 followed by its finite IEEE-754 binary64 bits in
  big-endian order. Both floating zeros encode positive-zero bits. Integer and floating
  representations remain distinct, including integer 1 versus floating 1.0.
- String: 0x05, unsigned 64-bit big-endian UTF-8 byte length, then those exact UTF-8 bytes.
- Array: 0x06, unsigned 64-bit big-endian item count, then framed items in order.
- Object: 0x07, unsigned 64-bit big-endian member count, then each framed string key and
  framed value, ordered by raw UTF-8 key bytes. Duplicate keys are invalid before framing.

The digest is SHA-256 over those exact bytes. This is a manifest-specific identity
encoding, not a replacement wire format or a generic JSON canonicalization service.
Golden fixtures must distinguish argv order, tool order, scalar kind and artifact bytes;
object-key permutations and floating negative zero must normalize identically. The
same decoded schema values are used for this encoding and the model replay projection.
Execution-host approval identity includes this digest, while existing Turn/call identity
continues to scope the receipt to the selected Session and exact argument digest.

### Per-call verification and terminal behavior

Pre-admission parses/normalizes custom arguments under both 4 MiB bounds (raw and
normalized including LF) before approval or worker creation. The frozen registry owns
this per-definition admission limit and intersects it with the caller's request bound;
custom definitions cannot inherit the BasicFiles ceiling or widen a smaller host bound.
Absent per-definition limits retain existing behavior. The existing built-in bounds stay
unchanged. Startup has one 256 MiB aggregate unique-primary-artifact budget. Both startup
and call verification cap each executable at 128 MiB and each script at 16 MiB; a call
verifies only that tool's primary pair, at most 144 MiB combined. Reads are streaming, count actual bytes and one first-excess probe,
and share the same pass budget rather than resetting for every reference. Every
verification pass has one non-resetting 30-second budget checked with cancellation
between at-most-64-KiB reads. Limit, mismatch, cancellation or read failure prevents spawn.
The default absence of an absolute agent deadline does not remove this verification cap.

Approval presents the frozen manifest identity. Final artifact verification runs after a
matching approval in the existing execution worker; the model-loop availability method
remains a cheap frozen-registry lookup. Verification does not mutate expected hashes,
refresh the manifest, start a second attempt, or publish raw host paths/file contents.
When the optional absolute execution deadline is supplied it starts once when that
approved attempt enters the worker and is not restarted at spawn or stdin completion.
Cancellation and the absolute deadline are checked again immediately before the single spawn. Five-minute stdout/
stderr inactivity applies after spawn; stdin writes never reset output inactivity.

After spawn, stdin writing, stdout/stderr reading, process exit, cancellation and timeouts
are serviced concurrently through the existing process owner. The normalized argument
bytes plus one LF are written once, then stdin is closed. Early EPIPE is one failed
attempt, even if the process reports zero exit. Exit-code, stdout/stderr framing,
truncation, retained presentation and semantic admission reuse run_command rules.
All terminal paths close stdin and apply the existing finite process-group termination,
reap and drain limits; cleanup failure is explicit and never triggers a retry.

The initial platform implementation must retain original interpreter/script/native path
semantics. It verifies configured executable symlink resolution and rejects observed
mapping/content changes, but expressly does not claim atomic execute-from-hashed-bytes
against uncoordinated publishers after final verification. A stalled kernel filesystem
operation is likewise not made preemptible by a user-space elapsed-time check. These are
explicit host-environment limits, not hidden safety guarantees or permission to skip
cancellation between reads and before spawn.

## Versioned OS-confined command execution

This section confines processes launched by the v2 command runner, including
its configured-command children. It does not change the other BasicFiles
execution hosts or create agent-wide Git, credential, or Session protection.
Those file tools retain their preceding contracts. In particular, the command
runner's secret-path and IPC exclusions MUST NOT be claimed for every tool or
for the agent as a whole.

The preceding legacy-read-file/v1, basic-files/v1, command-tools/v1, and
execution-definition-manifest/v1 contracts remain exact historical profiles.
Their ordered definitions, approval values, model-visible bytes, digest domain,
and execution semantics MUST NOT be rewritten or inferred from a successor.
Resume selects the exact recorded profile; an unknown or unavailable profile
opens read-only rather than upgrading or falling back.

Future tool-enabled Sessions without configured command tools use
`yo.local-tool-registry/basic-files/v2`. It retains the ordered five BasicFiles
ToolIds, wire names, schemas, and all definitions except `run_command`.
The v2 `run_command` description is exactly
`Run one shell command in the current workspace. Commands needing broader access or risky changes require approval.`;
its effect is `Process` and its approval requirement is `Planned`. The v1
description and `Required` approval value remain unchanged. Future Sessions
with configured commands use `yo.local-tool-registry/command-tools/v2`, whose
ordered tools are the same five v2 built-ins followed by the existing explicit
one-to-sixteen configured commands. Configured commands remain `Process` and
`Required`; their schemas, executable identity, argument handling, and fixed
launch semantics remain unchanged.

For `command-tools/v2`, the closed manifest retains the v1 object shape
`profile`, `registry`, `tools`, and `protocols`, and each tool retains the v1
fields and value framing. The profile is exactly
`yo.execution-definition-manifest/v2`; the registry is exactly
`yo.local-tool-registry/command-tools/v2`. Its built-ins equal the trusted
BasicFiles/v2 definitions. `approval` admits exactly `Automatic`, `Required`,
and `Planned` in v2; configured commands remain `Required`. Protocols are
exactly
`{stdin:"yo.command-json-stdin/v1",output:"yo.command-text-output/v1",environment:"yo.command-safe-environment/v2",runner:"yo.command-execution/workspace-confined-v2"}`.
The digest preimage begins with exact ASCII
`yo.execution-definition-manifest/v2` and one zero byte, followed by the
existing recursively framed manifest value. The v1 prefix, framing, and
digest remain byte-for-byte unchanged. A profile or registry version is
selected explicitly from the recorded complete manifest; no mixed profile,
implicit migration, or v1-to-v2 digest reuse is permitted. The BasicFiles/v2
registry identity carries the same runner revision for Sessions without
configured commands.

`Planned` means the execution host prepares one immutable command plan before
any approval or spawn and returns exactly one of: automatic with that plan,
approval-required with that plan, or unavailable with a stable failure code.
The plan fixes the exact normalized command, workspace and cwd identity,
platform adapter/profile and classifier revisions, read-only and writable roots,
secret-read exclusions, environment, inherited
descriptors, IPC/network capabilities, and process/cancellation scope. The approval binds the existing Turn, call,
ToolId, normalized argument digest, effect, and execution-host identity plus
the complete plan identity. The approval view names every requested scope
extension, including external paths or network access. Approval executes this
same plan once; it MUST NOT recalculate, widen, or replace the plan. Configured
commands keep their existing always-required approval and bind the plan too.

The baseline plan grants writes in the selected workspace except at
secret-read exclusions and uses private per-call home and temporary
directories. It exposes only explicitly admitted operating-system, runtime,
toolchain, and cache roots read-only. Automatic writable caches MUST reside
inside the admitted workspace or private per-call directories. Writing an
existing cache outside those roots requires the same exact external-root
approval as any other outside-workspace write; identifying a cache in a
frozen plan does not exempt it from approval. The baseline does not expose
the user's general home directory or inherited credential-bearing environment.

The workspace `.git` entry and resolved Git directory and common directory
(including linked-worktree metadata) follow the same write-root boundary as
other workspace data. Metadata inside the selected workspace is writable in
the baseline, permitting ordinary `git add`, `git commit`, and non-destructive
Git operations without approval when no other recognized risk or scope
extension is present. Any resolved Git directory or common directory outside
the selected workspace is admitted read-only for status, diff, and metadata
inspection; writing it requires an explicit per-call approval for those exact
resolved roots. No parent directory or other repository is implicitly granted.
An unresolved `.git` pointer or Git-directory identity makes the plan
unavailable. Root grants never override secret-read exclusions. Destructive
Git forms independently require approval as specified in the classifier table.
This is ordinary command execution under the same frozen plan, not a separate
Git execution service.

Non-secret Yo configuration may be exposed read-only only when its secret
subpaths are separately identified. Secret-read exclusions include
credential stores and helper endpoints, Keychain or equivalent credential
services, Yo authentication tokens and runtime secrets, and active Session
journals, history, attachments, indexes, locks, and control sockets. These
paths and endpoints are not readable or writable and are not inherited through
environment variables, descriptors, or sockets. The general home directory
remains hidden apart from private per-call `HOME` and explicitly admitted
support or cache roots. Secret-read exclusions take precedence over every
readable, writable, or user-approved root; approval cannot make them visible.
If a secret path cannot be reliably separated from a proposed readable root,
that root is hidden, or the plan is unavailable when it is required.

Network access is disabled in the baseline. A recognized request for network
or remote transfer can produce an approval-required plan that includes the
network capability for that call; the approval view states that capability.
A recognized write or delete outside the workspace can produce an
approval-required plan containing only the exact normalized target roots.
Such a root MUST be resolved before approval, MUST preserve all secret-read
exclusions, and MUST be representable by the selected OS adapter as a scoped
grant. An ambiguous, broad, secret-exposing, or
unsupported requested scope is unavailable.
A scope denial, setup failure, or missing OS capability MUST NOT select an
unrestricted runner or automatically retry with another profile. A command
that has started is never retried or widened after any possible effect.
Neither approval nor an external-root grant can relax secret-read exclusions.
A network grant does not inherit host authentication: credential files,
credential helpers, SSH-agent sockets, and equivalent host authentication
services remain excluded. This contract does not promise that every
authenticated external command can execute.

The recognized-risk classifier is owned by the CLI execution host. Its
versioned minimum deletion policy is `yo.command-risk/workspace-v2`. It MUST
recognize the following direct command forms, including a literal executable
path whose final component is the named command. The table assumes targets
are inside baseline writable roots; any requested outside-root write still
requires an exact scope grant, and secret-read exclusions remain absolute.

| Recognized form | Required disposition |
| --- | --- |
| `rm` without `-r`, `-R`, or `--recursive`, with exactly one literal file operand; or `unlink` with exactly one literal file operand | Automatic when the whole call has no other recognized risk or scope extension. `-f`, quoting a literal name, and `--` alone do not change this disposition. |
| `rm` with a recursive option, including combined short options such as `-rf` | Approval-required, even for one literal directory. |
| More than one deletion operand in a call's recognized `rm` or `unlink` commands, or a deletion operand containing an active shell glob | Approval-required. Multiple simple deletion commands within one call count together; a quoted or escaped literal glob character is not an active glob. |
| A recognized `rm` or `unlink` operand whose value depends on parameter, command, or arithmetic expansion | Approval-required because the single-literal-file case is not established; this alone grants no external root. |
| `find` with `-delete`, or `find -exec` / `-execdir` directly invoking `rm` or `unlink` | Approval-required, including both `;` and `+` terminators and a literal executable path. |
| `xargs` directly invoking `rm` or `unlink` | Approval-required, including invocation on a pipeline. |
| Mutating `git clean` | Approval-required; an explicitly dry-run or help-only invocation remains automatic when no other risk is present. |
| `git reset --hard`; `git checkout` or `git switch` with `-f`, `--force`, or `--discard-changes`; path-checkout forms of `git checkout`; and `git restore` that writes the worktree | Approval-required for recognized discarding of worktree changes, regardless of target count. Help-only invocations are excluded. `git restore --staged` without a worktree write is not this trigger. |
| Ordinary `git add`, `git commit`, and Git operations with no recognized destructive form | Automatic when all required writable metadata is inside the selected workspace and no other risk is present. A resolved external gitdir/common-dir write requires its exact per-call root grant. |

The minimum syntax coverage includes simple commands separated by `;`, newline,
`&&`, `||`, or pipelines; shell quoting and escaping; leading literal
assignments; and option terminators. Recognized Git forms remain recognizable
with literal global options such as `-C`, `--git-dir`, `--work-tree`, and `-c`;
those options cannot implicitly expand the frozen root grants. A recognized
checkout form that cannot be distinguished from a discarding path checkout
without executing Git requires approval rather than being labeled an ordinary
branch switch. Classification inspects all such commands before any spawn
and preserves whether an operand is literal or expanded.
It MUST NOT run shell expansion, execute a command, or delete anything to
classify a call. The host must publish its bounded syntax and recognition
coverage alongside the versioned profile and test every minimum row. Nested
interpreters, dynamically constructed command names, shell functions, and
arbitrary script or executable internals are not recursively interpreted by
this minimum classifier. Literal appearances in comments or quoted data are
not independently executable commands.

These are command-form triggers, not a count of filesystem entries discovered
by expansion or a promise that every destructive program is recognized. A
recognized risk uses an approval-required baseline plan when it needs no
additional roots; if an outside-root request cannot be resolved to an exact
admissible grant, that extension is unavailable. Normal edits, formatting,
builds, tests, single-literal-file deletions, and unfamiliar executables may
run automatically within the baseline plan. An unfamiliar executable or
unrecognized syntax alone MUST NOT force approval or make the command
unavailable. OS confinement limits its filesystem and network access; denied
unrecognized access is not retried with wider privileges. Unrecognized or
program-internal deletion or Git-metadata damage inside an admitted writable
root is not guaranteed to prompt. Allowing in-workspace Git metadata writes
also allows an unfamiliar program to modify that metadata within the same
root; the classifier is not an integrity guarantee for Git objects, refs,
index, configuration, or hooks. This policy does not provide copy-on-write
execution, an
original-workspace snapshot, or automatic restore.

The process receives only stdin, stdout, and stderr descriptors. The runner
clears inherited environment values and supplies only its explicit safe
environment, including private `HOME` and `TMPDIR`; it does not pass host
agent, SSH, credential, desktop, or Session sockets. The per-call profile
must deny access to host agent, credential, and Session IPC endpoints,
including pathname and abstract UNIX sockets and Mach service or task-port
access. A Mac plan may explicitly name benign operating-system/runtime
Mach-lookup services needed by its admitted toolchain; their policy grants
MUST remain disjoint from host credential, agent, helper, and Session
endpoints and MUST NOT admit Mach task ports. Broad Mach lookup is not an
allowed substitute. Approved network access does not grant host IPC. Process
visibility,
signaling, child inheritance, cancellation, termination, reap, and output
drain are platform capabilities whose required guarantees are specified by
the selected adapter below and must be qualified with call-scoped fixtures;
a platform/version without demonstrated required behavior returns unavailable
before spawn. Local sockets created inside an admitted writable
root remain possible only where the frozen plan and platform policy permit
them.

The Linux adapter uses an isolated Bubblewrap-style mount/user/PID/IPC
profile, a private network namespace when network is absent, read-only
support roots, scoped writable binds, and private temporary/home paths. It
requires every isolation primitive needed by the frozen plan before spawn;
no additional Landlock dependency is implied.

The macOS adapter uses `/usr/bin/sandbox-exec` with the explicit complete-plan
adapter/profile revision `macos-seatbelt-path-process-group-v1alpha1` and
process scope `process-group-bounded-cleanup`. It shares the v2 registry,
runner, classifier, exact-plan approval, secret-read exclusions, network/IPC
policy, and descriptor/environment sanitization above. Those existing wire
profiles and all Linux adapter identities and isolation guarantees remain
unchanged; no implicit resume migration or unrestricted fallback is admitted.
The distinct Mac adapter revision and process scope MUST enter the complete
plan identity before approval or spawn.

Mac filesystem grants apply to the resolved, frozen named paths through
Seatbelt literal/subpath rules. The runner MUST revalidate captured root and
secret-exclusion identities immediately before spawn; an observed replacement
or unresolved exclusion makes that plan unavailable without recalculation.
This check detects observed changes; it does not make Seatbelt grants
FD-bound or atomic against concurrent rename, replacement, or alias changes.
The contract therefore does not guarantee that every admitted or excluded
root continues to name its captured filesystem object after that check.
The named-path secret exclusions still take precedence over all grants;
this limitation never authorizes deliberately exposing a secret path.

The Mac command starts in its own process group. Every terminal path applies
the existing bounded termination of that original group, direct-child reap,
and stdout/stderr drain; cleanup failure remains explicit and never retries
the command. Descendants remaining in the original group are covered by that
cleanup. A descendant that changes its process group or session, including
through `posix_spawn` attributes, can outlive it; arbitrary escaped-descendant
tracking, termination, or reap is not guaranteed. Child processes MUST still
inherit Seatbelt restrictions when they change group or session. macOS does
not claim the Linux private PID namespace's process-visibility or signaling
isolation. None of these Mac limits relax the shared filesystem, secret,
network, or host-IPC policy.

Binary presence and successful policy parsing are insufficient qualification.
For each admitted macOS release, actual sandbox-child tests MUST demonstrate
workspace writes, automatic in-workspace Git-metadata writes, resolved external
Git-metadata reads with unapproved writes denied, exact approved external
gitdir/common-dir writes, secret-root read denial, other exact approved-root
grants, outside-root denial, network denial and approved-network behavior,
descriptor/environment sanitization, and denial of host UNIX-socket,
forbidden host Mach-lookup, Mach-task, and credential/Session IPC access.
Any explicitly admitted benign runtime Mach-lookup services MUST be tested
alongside the forbidden-service denial. Tests MUST demonstrate
child inheritance and bounded original-process-group termination, direct-child
reap, and output drain, and must distinguish group/session escape from that
cleanup scope. Root-identity checks MUST reject an observed pre-spawn change;
tests MUST NOT present those checks as proof of atomic filesystem binding.
Qualification is capability-specific: an unsupported network or external-root
grant returns unavailable for that requested plan without disabling an otherwise
qualified baseline or selecting a wider profile. A missing baseline guarantee
makes the affected adapter unavailable before spawn for that OS release.
A failed proposed mechanism does not establish that every Mac adapter or
another release is incapable of a stronger guarantee.

Filesystem path confinement protects the named paths and roots. It does not
establish an exhaustive inode-level guarantee against a pre-existing hard link
inside a writable root that aliases a read-only or secret-excluded file
outside that root.
Known credential device/inode exclusions remain required where the worker
directly opens a protected credential, but such checks do not prove that all
hard-link aliases were found. The contract MUST NOT describe protected path
denial as protection against every possible alias.

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
