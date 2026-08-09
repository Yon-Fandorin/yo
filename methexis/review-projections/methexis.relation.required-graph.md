---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.relation.required-graph
revision: sha256:a9ada72783cea4be4a1b2e758cda712205f5decf7bcdc0a84db40dd200f5667f
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:60c96d7750199355819b345e6635155de4d851f27786c4150a7c598bcc3b3901
---
# Korean Review Projection

## Translation

# 필수 관계 그래프

## 선언

작성자는 forward relation만 기록해야 하고 consumer는 reverse index를 도출해야 합니다. `depends_on`과 `constrained_by`는 함께 하나의 필수 지식 그래프를 구성하며, 이 그래프에는 cycle이 없어야 합니다. `supersedes`는 별도 그래프를 구성하고 이 그래프에도 cycle이 없어야 합니다. `validated_by`와 `applies_to` anchor는 필수 지식 그래프에 참여하면 안 됩니다.
