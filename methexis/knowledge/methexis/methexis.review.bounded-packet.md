---
schema: methexis.knowledge/v1alpha1
id: methexis.review.bounded-packet
kind: procedure
owner: methexis
sources:
  - id: methexis.review-001
    revision: sha256:641af3e734a6547c3f12230653f574b600d29234970167442da00cf819869ab5
---
# Managed-payload-bounded Slice review packet

## Statement

An ordinary Slice review packet MUST bind one immutable base-and-candidate Git diff; one verified Methexis ContextBuild containing the relevant active Knowledge authority; the exact bytes, paths, and hashes of every required repository authority not contained in that build; the exact Slice-contract bytes and hash; requested review lenses and questions; declared validation evidence and hashes; and one versioned delivery profile. Its manifest MUST record those inputs together with the base and candidate commits, trusted commit and active Checkpoint identity, ContextBuild and artifact hashes, exact diff hash, tokenizer profile, managed-payload token count, and maximum managed-payload tokens.

An experimental activation-review packet MAY instead use one prospective ContextBuild only when its versioned request explicitly names one activation-request file inside the clean candidate worktree. That path MUST reproduce the exact proposed immutable Checkpoint and canonical active-record transition against pinned trusted `develop`, including the predecessor active-record hash, approval closure, current Source freshness, and registered artifact lineage. The packet, plan, manifest, structured result, and model-visible instructions MUST label the context authority `prospective`; they MUST NOT call it active, trusted, approved, or eligible. A caller-selected ref, arbitrary future tree, inferred proposal, fallback from ordinary resolution, or working-tree-only authority MUST fail closed.

`ReviewId` MUST be a domain-separated hash of a versioned canonical review plan containing the authority mode, base and candidate commits, exact diff hash, trusted commit, exact context Checkpoint identity and stable authority-basis commit, ContextBuild and artifact hashes, the activation-request path and content hash for prospective mode, repository-authority path and content hashes, Slice-contract content hash, validation-evidence hashes, review lenses and questions, delivery profile, tokenizer profile, and managed-payload budget. Output paths, publication time, operation status, packet hash, and manifest hash MUST be excluded because they are non-semantic or circular. Canonical plan encoding MUST be deterministic and unambiguous.

Every published request, plan, manifest, delivery-profile, and verifier identifier is a frozen behavior boundary. Prospective activation review MUST use the smallest new experimental `v1alphaN` family and explicit schema dispatch; it MUST NOT reinterpret an older identifier. Older packets remain exactly reproducible and MAY continue to root compatible delta chains.

This new path has a closed bootstrap. Its enabling Source and Knowledge revisions, executable implementation, versioned request/plan/manifest/delivery/verifier family, and `CONTRIBUTING.md` workflow adoption MUST each complete the existing ordinary active-ContextBuild review, approval, activation, and integration sequence. Before all enabling changes are trusted and these contracts are active, the path MUST remain disabled and MUST NOT build, verify, or supply review evidence for any change that enables itself. Its first eligible use is a later independent activation candidate. Changing workflow ownership instead requires the complete atomic migration owned by `methexis.workflow.self-hosting-boundary`; this path is not such a migration.

The delivery profile MUST define the exact fixed preamble and wrapper bytes and MUST make the canonical packet the complete caller-controlled model-visible payload. The token budget MUST count every byte of that payload with the declared tokenizer profile. Provider-controlled system, policy, or tool-description overhead that the caller cannot observe is outside this managed-payload budget and MUST NOT be described as part of a total reviewer-input budget. The packet MUST NOT rely on uncounted caller-controlled instructions or authority.

Construction MUST capture the diff, authority files, Slice contract, validation evidence, ContextBuild references, and any activation request into immutable snapshots before assembly. Immediately before publishing a new packet or returning a reused packet, it MUST final-revalidate the trusted ref, authority mode and exact Checkpoint identity, ContextBuild freshness, every captured proposal file and hash, the complete delivery-profile bytes, candidate HEAD, base-to-candidate diff, and candidate worktree cleanliness. It MUST fail without returning an eligible packet when any input changed, a profile is invalid, or the canonical packet exceeds its budget. It MUST NOT truncate the diff, Knowledge body, authority, review question, or validation evidence.

The packet and manifest MUST be published as one atomic create-if-absent artifact set using a temporary sibling and no-clobber installation. An existing ReviewId MAY be reused only after both files and every recorded input reproduce exactly; missing, extra, or mismatched bytes MUST fail without replacement. Prospective packet success is review evidence only. Activation still requires the ordinary staged transition gate, trusted integration, and post-integration full authority check.

## Steps

1. Select the request's explicit authority mode. Resolve ordinary active Knowledge through the existing path, or validate the exact activation request and compile the requested context against its prospective Checkpoint.
2. Capture the exact non-migrated repository authorities, optional activation request, Slice contract, validation evidence, clean candidate, and no-renames binary Git diff for the declared base and candidate commits.
3. Build the version-owned canonical review plan and derive its domain-separated ReviewId.
4. Select the matching versioned delivery profile and assemble its fixed wrapper plus every captured input as the complete caller-controlled model-visible payload.
5. Count the complete canonical payload with the declared tokenizer profile and fail closed when it exceeds the request budget.
6. Final-revalidate trusted authority, authority mode, proposed or active Checkpoint, ContextBuild freshness, all captured proposal files and hashes, delivery bytes, diff identity, and worktree cleanliness.
7. Atomically publish or exactly verify the packet and manifest, then return only their paths, hashes, ReviewId, authority label, and managed-payload count.

## Completion Criteria

The operation is complete only when one immutable packet and manifest reproduce the exact version-owned ReviewId plan, base and candidate commits, trusted basis, authority mode, active or prospective Checkpoint, optional activation request and canonical transition lineage, ContextBuild lineage, repository-authority bytes, Slice-contract bytes, validation evidence, diff, review instructions, delivery profile, tokenizer, managed-payload count, and budget; the count is within budget; final revalidation succeeds for both publication and reuse; the candidate worktree remains clean; and no partial, extra, different, inferred, self-enabling, or authority-promoting artifact is accepted or replaced.
