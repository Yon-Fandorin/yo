---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.interface.agent-first
revision: sha256:8093be41b411660c941fd220a269da016840f5f0643578e46e738572a9f470f2
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:6dcb749f5ff269d2396639df4b1028b04aa50ddd9798906055ed165e4301c010
---
# Korean Review Projection

## Translation

모든 Pilot operation은 비대화식·versioned structured I/O·stable error code·actionable failure를 제공하고 큰 artifact 대신 path와 hash를 반환합니다. review는 approval이 아니며 실제 Codex 작업에서 유용하지 않은 interface는 증거에 따라 제거하거나 바꿉니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

The primary Pilot consumer is a code agent. Every operation MUST:

- support non-interactive execution;
- expose versioned structured input and output;
- use stable machine-readable error codes within the Pilot version;
- include affected IDs and actionable next steps in failures;
- return paths and hashes instead of streaming large artifacts through stdout;
- derive human-readable output from the same result.

The responsibility surface includes:

| Methexis | Librarian |
| --- | --- |
| Fast check | Candidate discovery |
| Review packet | Catalog integrity check |
| Exact-revision approval record | Relocation plan |
| Checkpoint activation | |
| Context resolution | |

Exact command names and final JSON fields remain provisional. Review never
implies approval. A CLI cannot prove that its caller is human, so approval still
requires explicit human authorization in the repository review flow.

The current agent path uses versioned JSON request files, conventionally under
`.local-exclude/methexis/requests/`. It writes tracked Projection and approval
proposals, and content-addressed review packets under
`.local-exclude/methexis/reviews/`. Requests and local packets are
non-authoritative and MAY be discarded after their paths and hashes are
returned. A future database MAY retain request history for audit or evaluation,
but remains a reconstructible index rather than authority.

The Pilot MUST be dogfooded during real Codex Surface work. Interface elements
that do not improve safe agent completion SHOULD be removed or reshaped from
evidence rather than preserved for compatibility.
