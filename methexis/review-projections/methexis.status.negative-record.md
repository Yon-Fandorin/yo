---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.status.negative-record
revision: sha256:b72db2560b81d7f2f7d2e8be2c3e967b6f990cf916c02935ae7a74a7ed27665d
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:75adfc159ba804b333e87ea5efe44ea779894774bded2c454bd2bacbf24f38a5
---
# Korean Review Projection

## Translation

# Durable negative 상태 record

## 선언

durable review hold와 invalidation은 영향을 받는 정확한 Knowledge revision에 바인딩된 tracked record여야 합니다. review hold는 해결되지 않은 semantic 또는 provenance 불확실성에 대한 `suspect` guard condition을 제공해야 합니다. 명시적인 human invalidation은 `invalid` guard condition을 제공해야 합니다.

어느 record도 approval이나 activation을 부여하면 안 됩니다. 각 record는 우선순위와 전환을 테스트할 수 있도록 해당 condition의 machine-readable evidence를 제공해야 합니다.
