---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.knowledge.record-format
revision: sha256:5190f93b930126015f89a0a704ea4391f6325cb6da235b2d68683a773f06d13d
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:46d9f1700b8dc459a39c38941b9be42ae735c08c421796ccc7c4af57912d5797
---
# Korean Review Projection

## Translation

# 지식 record 형식

## 선언

Pilot은 Markdown 파일 하나에 KU 하나를 저장해야 합니다. 각 파일은 machine metadata로 검증되는 닫힌 typed YAML frontmatter와 의미를 위한 제약된 canonical English Markdown 본문을 포함해야 합니다. Frontmatter에는 schema, `KnowledgeId`, kind, `OwnerId`, 정확한 Source reference와 typed relation만 들어가야 합니다. 본문의 canonical statement를 중복하면 안 됩니다.

Canonical frontmatter는 YAML merge key를 사용하면 안 됩니다. Loader는 물리 위치에서 identity를 유도하지 않고 record 내용에서 `KnowledgeId`와 `OwnerId`를 읽어야 합니다.
