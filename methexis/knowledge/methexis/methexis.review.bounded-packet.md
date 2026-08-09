---
schema: methexis.knowledge/v1alpha1
id: methexis.review.bounded-packet
kind: procedure
owner: methexis
sources:
  - id: methexis.review-001
    revision: sha256:4adac6cc627a8664ed1a270ce7ef18146b89407e552e70d437560d2a8e78e8cd
---
# Managed-payload-bounded Slice review packet

## Statement

A Slice review packet MUST bind one immutable base-and-candidate Git diff; one verified Methexis ContextBuild containing the relevant active Knowledge authority; the exact bytes, paths, and hashes of every required repository authority not contained in that build; the exact Slice-contract bytes and hash; requested review lenses and questions; declared validation evidence and hashes; and one versioned delivery profile. Its manifest MUST record those inputs together with the base and candidate commits, trusted commit and active Checkpoint identity, ContextBuild and artifact hashes, exact diff hash, tokenizer profile, managed-payload token count, and maximum managed-payload tokens.

`ReviewId` MUST be a domain-separated hash of a versioned canonical review plan containing the base and candidate commits, exact diff hash, trusted commit and active Checkpoint identity, ContextBuild and artifact hashes, repository-authority path and content hashes, Slice-contract content hash, validation-evidence hashes, review lenses and questions, delivery profile, tokenizer profile, and managed-payload budget. Output paths, publication time, operation status, packet hash, and manifest hash MUST be excluded because they are non-semantic or circular. Canonical plan encoding MUST be deterministic and unambiguous.

The delivery profile MUST define the exact fixed preamble and wrapper bytes and MUST make the canonical packet the complete caller-controlled model-visible payload. The token budget MUST count every byte of that payload with the declared tokenizer profile. Provider-controlled system, policy, or tool-description overhead that the caller cannot observe is outside this managed-payload budget and MUST NOT be described as part of a total reviewer-input budget. The packet MUST NOT rely on uncounted caller-controlled instructions or authority.

Construction MUST capture the diff, authority files, Slice contract, validation evidence, and ContextBuild references into immutable snapshots before assembly. Immediately before publishing a new packet or returning a reused packet, it MUST final-revalidate the trusted ref, active Checkpoint identity, ContextBuild freshness, every captured hash, the complete delivery-profile bytes, and candidate worktree cleanliness. It MUST fail without returning an eligible packet when any input changed, a profile is invalid, or the canonical packet exceeds its budget. It MUST NOT truncate the diff, Knowledge body, authority, review question, or validation evidence.

The packet and manifest MUST be published as one atomic create-if-absent artifact set using a temporary sibling and no-clobber installation. An existing ReviewId MAY be reused only after both files and every recorded input reproduce exactly; missing, extra, or mismatched bytes MUST fail without replacement.

## Steps

1. Resolve the relevant active Knowledge through a token-bounded Methexis ContextBuild and capture its returned context and manifest identities.
2. Capture the exact non-migrated repository authorities, Slice contract, validation evidence, clean candidate, and no-renames binary Git diff for the declared base and candidate commits.
3. Build the versioned canonical review plan and derive its domain-separated ReviewId.
4. Select a versioned delivery profile and assemble its fixed wrapper plus every captured input as the complete caller-controlled model-visible payload.
5. Count the complete canonical payload with the declared tokenizer profile and fail closed when it exceeds the request budget.
6. Final-revalidate trusted authority, active Checkpoint, ContextBuild freshness, all hashes, delivery bytes, and worktree cleanliness.
7. Atomically publish or exactly verify the packet and manifest, then return only their paths, hashes, ReviewId, and managed-payload count.

## Completion Criteria

The operation is complete only when one immutable packet and manifest reproduce the exact ReviewId plan, base and candidate commits, trusted authority, active Checkpoint, ContextBuild lineage, repository-authority bytes, Slice-contract bytes, validation evidence, diff, review instructions, delivery profile, tokenizer, managed-payload count, and budget; the count is within budget; final revalidation succeeds for both publication and reuse; the candidate worktree remains clean; and no partial, extra, or different artifact bytes are accepted or replaced.
