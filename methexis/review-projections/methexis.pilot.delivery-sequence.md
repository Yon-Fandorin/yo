---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.pilot.delivery-sequence
revision: sha256:385949e6944588af940fb884b8f2ee12b218c78407f70430265b597d7c8c6a3a
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:d23059d0ad4929eeaaf43f31c8c5b3b04e5f13213f95c927ff44040dc6d61f74
---
# Korean Review Projection

## Translation

Pilot는 knowledge foundation에서 approval/checkpoint/source validation과 Librarian discovery를 거쳐 context resolution과 Surface dogfood로 진행합니다. 각 Slice는 end-to-end agent path, structured output, fixture, owner reference, test와 example을 제공해야 합니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

The proposed implementation sequence is:

```text
S1 knowledge-foundation
   |
   +-- S2a approval-projection --> S2b checkpoint-proposal --> S2c source-validation --+
   |                                                                                   |
   +-- S3 librarian-discovery ---------------------------------------------------------+--> S4 context-resolution
                                                                                                |
                                                                                          S5 surface-dogfood
```

S2a and S3 may run in parallel after S1. S2b depends on S2a; S2c owns Source
freshness and is the only stage that may open active eligibility. S4 is the
explicit join of S2c and S3. S5 expands the Surface corpus to roughly 20–50
units and runs the 8–12 task evaluation.

Every Slice must provide one end-to-end agent path, versioned structured
output, success and failure fixtures, owner decision references, tests, and
inspectable example output. The Wave MUST NOT redesign the root Cargo
workspace, introduce database authority, or generalize before evaluation.
