---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.validation.executable-evidence
revision: sha256:c5f31b0ae924a9b9399f954446e60fea6049bd34294701625f0e18aa14a61e33
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:7d184ccade8ab02989ca86ffd63000ef869ec940ac17a85d24332b9b6ec277fc
---
# Korean Review Projection

## Translation

Checkpoint 활성화는 승인, Source freshness, dependency closure, supersession 제외, 실행 증거와 review Projection 재현성을 검증합니다. 실행 증거는 content-addressed이며 context resolution은 전체 suite 대신 SOT-007 freshness guard를 실행합니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

Checkpoint activation additionally verifies:

- approval and Source freshness;
- complete required dependency closure;
- exclusion of replaced old knowledge;
- current executable evidence;
- reproducible human-review projection.

Executable evidence is content addressed. Unchanged code, knowledge, command,
and tool inputs reuse prior evidence. Related changes stale only affected
evidence. Context resolution consumes an active Checkpoint and does not rerun
the entire validation suite, but it MUST run the freshness guard defined by
`SOT-007` before using cached eligibility.
