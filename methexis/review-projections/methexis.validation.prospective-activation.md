---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.validation.prospective-activation
revision: sha256:63581e2a8a35419d3bef85e2e3fff4dc5d87675cc5543672a337421ee45c93a2
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:3d507017b3e284fde689e9e7473fce80b453b598855bd79540c0823b08fce005
---
# Korean Review Projection

## Translation

staged-activation check는 승인 commit과 바로 뒤의 활성화 commit 사이를 검증하는 read-only prospective 경로입니다. 정확히 한 Checkpoint 전환과 등록 artifact만 허용하고, 성공하더라도 승인·활성화·context eligibility를 부여하지 않습니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

`methexis check --staged-activation` is the repository-hook path for the
otherwise unavoidable interval after revised approvals reach trusted
`develop` and before their replacement Checkpoint is integrated. Without a
staged active-record change it has exactly the ordinary all-class `check`
behavior. With one, it accepts only one new immutable Checkpoint, the active
record, and the complete registered tracked-artifact set in the Git index;
unrelated staged paths fail closed.

The staged path is read-only and prospective, never trusted authority. It
resolves `develop` once, reproduces the proposed Checkpoint from that exact
trusted commit, verifies the active record's exact predecessor hash and
canonical bytes, requires every selected Source to remain fresh, checks staged
artifact provenance, and revalidates Source, proposal-index, and trusted-ref
stability before returning. It pins the exact Git index selected by the commit
invocation, including an explicit `GIT_INDEX_FILE`, and rejects non-regular or
non-stage-zero entries. Success
labels the candidate `prospective` and requires ordinary full `check` after the
exact reviewed transition is integrated. It MUST NOT accept caller-selected
refs, arbitrary future trees, working-tree-only candidate bytes, or a general
hook exception.

This check mechanizes the second half of a two-commit authority transition; it
does not make revised approvals and their Checkpoint one authority commit. The
trusted ref may therefore be intentionally inconsistent between the accepted
approval commit and its exact back-to-back activation commit. During that
bounded interval ordinary `check` and authority-consuming operations continue
to fail or use only the prior still-valid active authority; prospective success
never grants approval, activation, or context eligibility.
