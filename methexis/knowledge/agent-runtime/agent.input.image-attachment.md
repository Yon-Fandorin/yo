---
schema: methexis.knowledge/v1alpha1
id: agent.input.image-attachment
kind: decision
owner: agent-runtime
sources:
  - id: agent.input-003
    revision: sha256:2735b5399ce6275ce0657c85a5a6282cf3982f4cf85f062d67544ad6e1274f0e
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.core.frontend-independent-boundary
  constrained_by:
    - agent.runtime.command-event-boundary
  applies_to:
    - yo-core::UserInput
    - yo-core::AgentSession
---
# Immutable image input

## Statement

An explicit image attachment MUST become a frontend-independent immutable input
snapshot. A visible marker, Markdown image, filesystem path, workspace reference,
or URL alone MUST NOT establish an attachment. Input images are content transfer;
they do not reinterpret the existing host-owned workspace-reference or skill
contracts. A source-local attachment MUST NOT be presented as a path belonging to
another execution environment. Selection and preparation MUST NOT dispatch a
model request or invoke a tool.

The independent backend foundation MUST own one neutral immutable
`InputImageSnapshot` shared by replay and core input. Core MUST own UserInput,
visible occurrences, whole-input admission and the execution-host preparation
port. A concrete decoder, connector or terminal graphics type MUST NOT enter
that neutral snapshot. The local execution host MUST own bounded cancellable
reads, decoding, normalization and thumbnail preparation outside the UI thread;
the frontend owns draft identities, editing and display. A public or persisted
snapshot constructor MUST NOT substitute for execution-host live validation.

Preparation MUST read each source once under a compressed-byte bound, verify
static PNG or JPEG by content, reject APNG and every other format, and decode the
same bounded source snapshot. File extension, claimed MIME, and dimensions alone
are not validation. Each source MUST fit 4,194,304 compressed bytes, at most 4096
pixels per side and 4,194,304 decoded pixels; one input MUST fit 16 image
occurrences and 8,388,608 total compressed source bytes, charged per occurrence.
An excess byte, occurrence, dimension, checked arithmetic overflow, decode failure,
or cancellation MUST reject preparation before publishing a ready snapshot.

Exact normalization profile `yo.input-image-rgba8-triangle/v1` MUST apply PNG/JPEG
orientation and materialize RGBA8 using the selected image 0.25.10 conversion,
including its 16-bit quantization. It MUST NOT add ICC/gamma color management or
copy embedded profiles. Normalize without enlargement while preserving aspect
ratio: scale is min(1, 2048/max(width,height),
sqrt(2,097,152/(width*height))); each target dimension is its scaled dimension
rounded down, with a minimum of one, and both integer limits MUST be rechecked.
Use Triangle filtering when resizing. The resulting image MUST be at most 2048
pixels per side and 2,097,152 pixels. PNG encoding MUST preserve those normalized
8-bit pixels exactly without copying EXIF or provenance metadata. A changed
normalization algorithm requires a new profile rather than reinterpreting v1.
This contract does not claim original-bit-depth or colorimetric fidelity.

Each canonical PNG MUST fit 9,437,184 bytes, and one input MUST fit that same
9,437,184-byte aggregate, charged per occurrence. Encoding MUST use a bounded
writer that checks the first excess write before extending its owned buffer;
partial encoding MUST NOT become admitted. These are enforced acceptance limits,
not a guarantee that every otherwise valid source can fit the selected encoder.
Existing decoder allocation limits remain best effort; these logical bounds MUST
NOT be advertised as hard process RSS or CPU guarantees. The original source file
MUST remain unchanged.

The semantic snapshot MUST bind exact normalization `profile`, `mime_type`
`image/png`, positive transmitted `width` and `height`, positive PNG `byte_length`,
`sha256` of the exact PNG bytes, and the immutable bytes themselves. Its canonical
wire uses the persistence owner's `data_base64` field rather than retaining a
source path or defining a separate frontend image payload. Its digest uses `sha256:`
followed by exactly 64 lowercase hexadecimal digits. Repeated occurrences MUST
remain distinct ordered semantic parts and count separately even when immutable
in-memory storage is shared. Optional source display metadata MUST remain outside
the model projection and replay authority; source paths and original EXIF MUST
NOT enter checkpoint retained-image snapshots. The same full snapshot MUST feed
submission, model projection, Journal, queue, recall/edit, replay, resume and fork.
Those later paths MUST NOT reopen source paths or normalize again.

Each structured-input image occurrence MUST additionally retain the host-observed
`source_byte_length` outside the neutral model/replay snapshot. It is a positive
unsigned64-bit compressed-byte count no greater than4,194,304. The preparing host
records the actual bounded source length; a caller-supplied count is not admission
evidence. Draft editing, queueing, recall and inherited input history preserve this
count and charge it per occurrence against the8,388,608-byte input source budget,
including when previously admitted occurrences are combined. PNG byte length MUST
NOT substitute for original compressed size. Model replay and checkpoint-retained
snapshots need only their canonical PNG/container budgets and do not reopen or
recharge source reads. A new editable occurrence requires its validated original
input metadata; a replay-only image cannot invent missing source-byte evidence.

One admission host MUST run at most one image preparation job at a time and MUST
bound aggregate ready/pending draft-and-queue PNG ownership to 67,108,864 logical
bytes. Changing or removing a draft MUST invalidate a late preparation result by
its existing draft identity. Whole-request admission MUST validate the complete
ordered input and all existing workspace and skill references before Backend
dispatch or skill body loading. Rejection MUST retain the exact draft, attachment
snapshots and reference annotations. Accepted and Rejected MUST carry the existing
submission identity; Accepted transfers that exact immutable snapshot, and clears
only a still-matching editor draft. No path may silently drop an image, replace it
with a caption, substitute a file, or consume a newer draft.

Model projection MUST preserve original text spans between images, omit empty
spans, and preserve each image occurrence in order. Existing workspace references
retain their established projection. Append the exact existing skill trailer to
the final part only when that part is text; if the final part is an image, append
one new final text part containing the trailer. It MUST NOT move the trailer
across an intervening image into an earlier text part. At most 16 images therefore
permit at most 33 ordered text/image parts, including the trailer. New structured
input MUST fit 16,777,216 complete canonical encoded bytes, including every text,
reference, skill, image base64 and framing byte under the persistence owner.
Individual image limits do not imply whole-input admission. Existing text-only
input versions and bytes MUST retain their existing meanings.

During bounded preparation, derive a thumbnail from normalized pixels, bind it
to the full snapshot digest, and limit each side to 256 and total pixels to 65,536
without enlargement. Its encoded form MUST fit the existing 1,048,576-byte output
preview ceiling using a bounded writer. The TUI MUST use this prepared derivative
instead of decoding the full image synchronously during layout or routing the full
snapshot through the Markdown output data-URI decoder. Keep that output decoder's
existing untrusted-output bound. Attachment details MUST expose source and
transmitted dimensions and normalization/scaling; preview MUST NOT imply that its
thumbnail pixels equal the full transmission image or the original source.

Before image submission, the admission owner MUST require typed support for the
exact selected backend/host and model, permitted PNG input, the applicable image
occurrence/byte bounds, and the reviewed ordered wire projection. Unknown and
unsupported capability MUST remain distinct typed reasons and MUST reject before
dispatch while retaining input. Generic dialect compatibility, a terminal's pixel
support, a provider name, or an unrelated model's catalog row is not this evidence.
Managed requests additionally require the exact effective binding's reviewed image
input/accounting profile. Delegated Codex may admit this common input only when
its reviewed protocol and exact selected-model image modality are available;
other delegated protocols remain subject to their own negotiated capability and
reviewed wire contracts. The common semantic input MUST NOT branch on Provider.
Accounting uncertainty does not mean unsupported media and MUST remain explicitly
labelled under the selected managed profile.

A committed model request that fails with a provider 4xx MUST follow the existing
failed-request and uncertain-continuation rules while retaining its committed
snapshots. HTTP 400 alone MUST NOT be classified as context overflow. This
extension authorizes no automatic resend, stripped-image retry, model change,
synthetic successful compaction, or new Continuation Anchor. Explicit existing
recall/edit/new-Session paths MUST retain attachments. A pre-dispatch preparation
or admission failure instead preserves the draft.

This unit MUST activate only with compatible exact reviewed model-service,
Connector/backend, input persistence/replay, context-checkpoint/compaction,
continuation and frontend revisions. It does not itself approve a new wire
variant, redefine old context-counter fields, or widen an artifact retrieval tool.


### Closed structured-input occurrence

Only image-containing input uses `yo.structured-input/v3`. Its canonical writer
field order is `profile`, `text`, `references`, optional `resolved_skill`, `images`.
Existing reference entries and resolved-skill framing retain their frozen grammar;
explicit null is not absence. Each of1..16 image entries has exactly `start`, `end`,
`projection`, `source_byte_length`, `snapshot`, and optional `display` in that order.
Offsets are unsigned64-bit UTF-8 byte boundaries whose nonempty disjoint visible
slice equals the exact `[image]` projection; entries are in increasing draft order
and cannot overlap another image or reference. Literal unannotated `[image]` text
has no attachment meaning. The snapshot uses the fields and bounds above, with
canonical padded RFC4648 base64 without whitespace or alternate encodings. The
closed optional display object admits only filename, source_mime_type,
source_width, source_height and source_sha256, each optional but never null.
Filename is a nonempty basename up to255 UTF-8 bytes with no path separator or
control character; source MIME is PNG/JPEG; source dimensions are positive
unsigned32-bit original header values under the4096-side/4MP source bounds;
source hash uses the canonical SHA256 spelling. Display fields do not grant
admission. Unknown/duplicate fields, invalid scalar domains, inconsistent decoded
PNG identity/dimensions or complete encoded excess reject the whole new input.

## Rationale

An immutable normalized snapshot makes the submitted image independent of mutable
paths and provider-specific EXIF behavior. Common ordered input supports managed
and delegated agents while distinct capability and accounting evidence keep
unsupported media and advisory context planning visible.
