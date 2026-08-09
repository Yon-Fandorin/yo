---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.validation.snapshot-construction
revision: sha256:cb139722d55bd1daef24fe4b5b227bf1b23b32821594f1da5f0785d346c19aac
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:f62c714091dd784f96f96a6bae00ba4146683e0ef77850836802763a53dcc521
---
# Korean Review Projection

## Translation

Fast editing validation은 하나의 all-or-nothing 구조 snapshot을 순서가 정해진 두 단계로 구성해야 합니다.

로컬 `records` 단계는 authority root 아래의 Git 미추적 추가분을 포함해 working tree에서 발견된 모든 owner, Source, Knowledge 및 관련 record를 파싱하고 schema, field, identity, relation shape, 필수 body 진단을 모아야 합니다. 전역 `relations` 단계는 모든 record가 로컬 검증을 통과한 뒤에만 실행하며, 중복 identity, 누락된 owner 또는 relation target, graph cycle을 모아야 합니다.

탐색 과정은 authority root, authority directory, 추적 record 경로가 symbolic link이면 이를 따라가지 않고 거부해야 합니다. Snapshot revision과 unit identity는 검증 반복, record 탐색 순서, 물리 위치 이동에 상관없이 결정적으로 유지되어야 하며, canonical Knowledge identity는 `methexis.knowledge.identity`가 계속 소유합니다.

진단은 안정적인 code와 결정적인 phase·path·code·line·column·message·affected ID 순서를 사용해야 합니다. 로컬 또는 전역 구조 진단이 하나라도 있으면 보고서에 snapshot revision이나 unit set을 실어서는 안 됩니다. `records` 단계가 실패하면 `relations` 단계는 실행하면 안 됩니다. 이후 validation class의 계획과 차단은 `methexis.validation.check-classes`가 계속 소유합니다.
