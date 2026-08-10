---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.migration.complete-transition
revision: sha256:1fd57d188e0ad2b44446887d53f7c28ccaa56f51ef683eed92f3348ac6cc66ec
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:cfbebab9e35dc57e494f26c054457c612e78df0288041ddc9400b6c3cea4339c
---
# Korean Review Projection

## Translation

승인된 SOT scope는 tracked active Checkpoint가 선택한 정확한 KU revision으로만 권위를 가집니다. complete-transition root는 scope registry, 교체 보호 규칙, 전체 owner closure를 함께 닫습니다. 교체는 forward CAS Checkpoint 한 번으로 수행하며 trusted 전까지 현재 revision이 권위를 유지합니다. active KU closure 밖의 repository prose는 대체 권위가 아닙니다.
