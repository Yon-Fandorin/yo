---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.relation.vocabulary
revision: sha256:4d572a9a49727ddf5cc6d2eabf76a1fe2ef238e5eb7b586a157d0019d97d8e94
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:80c4cae15f605e0855de8e7abc739d09f7a6b12e0f0c1f4e3c0ccd8afa81b905
---
# Korean Review Projection

## Translation

# 관계 어휘

## 선언

정식 관계 어휘는 다음 다섯 가지 typed relation으로 닫혀 있습니다.

- `depends_on`은 완결성을 위해 필요한 KnowledgeUnit을 가리킵니다.
- `constrained_by`는 허용되는 동작을 제한하는 KnowledgeUnit을 가리킵니다.
- `validated_by`는 실행 가능한 근거를 제공하는 test 또는 fixture를 가리킵니다.
- `applies_to`는 file, module, symbol, mode처럼 범위에 포함되는 code anchor를 가리킵니다.
- `supersedes`는 의미적 identity가 대체되는 KnowledgeUnit을 가리킵니다.

도출과 뒷받침은 Source provenance에 속합니다. 번역과 요약은 Projection lineage에 속합니다. 약한 `related_to` signal은 Librarian의 advisory discovery data이지 정식 relation이 아니며, SOT eligibility 또는 invalidation에 영향을 주면 안 됩니다.
