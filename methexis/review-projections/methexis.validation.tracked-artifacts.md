---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.validation.tracked-artifacts
revision: sha256:cb58a10bb9d3585b99f2e41ee01855a3369d81349b8910b1da8278b7c00c2f64
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:1b11276d0bca6513391ed901e084aa92297b2347842a13788066716145fd12d6
---
# Korean Review Projection

## Translation

# 추적 산출물 검증 경계

## 설명

`artifacts` class는 trusted authority에서 파생된 추적 contract 산출물만 검증합니다. 이 Pilot에서는 등록된 context manifest의 Checkpoint ID, 해시 및 authority-basis commit을 active trusted Checkpoint와 대조합니다. 바이트 단위 재생성을 보장하지 않으며, 재구축 가능한 `.local-exclude/` ContextBuild 캐시를 검사하거나 gate하지 않습니다. 일반 Rust 테스트, lint 및 formatting은 Methexis check class가 아니라 Cargo와 `hk`의 책임으로 유지됩니다. 등록된 추적 산출물 경로가 하나도 없는 저장소 또는 격리 fixture에서 `artifacts` class는 비어 있으며 통과합니다. 등록 경로가 하나라도 존재하면 닫힌 집합이 활성화되고, 그 뒤에는 모든 등록 산출물이 필요합니다. active trusted Checkpoint가 없으면 `authority` 평가는 통과할 수 있지만(`MAY`) `artifacts`는 `blocked`입니다. 따라서 요청한 검증은 불완전하며 전체 report는 실패하고 호출자에게 active trusted authority를 수립하도록 안내합니다.

별도로 호출되는 ContextBuild 심층 검증기는 현재 trusted authority 아래에서 호출자가 이름을 지정한 재구축 가능 local build 하나를 재현하고 비교할 수 있습니다(`MAY`). 이 작업은 다섯 번째 check class도, `artifacts`의 전제 조건도, 기본 `check` 선택도, `hk` gate도 아니며, 이름이 지정되지 않은 캐시 항목을 스캔해서는 안 됩니다(`MUST NOT`). 그 결과는 local cache를 tracked authority로 승격하거나 등록 manifest 검사를 대체할 수 없습니다.
