---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.context.build-reuse
revision: sha256:54126046e0a163adcb094284128e0466dc09c87c0018616f1bb561545b46d5be
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:3b98e1f8fc003ff579c0d6a487bac016b2e1aee873f71906ccd6cf2102e9b421
---
# Korean Review Projection

## Translation

# ContextBuild 재사용, 내보내기 및 선택적 심층 검증

## 설명

고정 BuildId 저장소는 Pilot의 불변 원본을 소유합니다. 성공한 구조화 결과는 `created` 또는 `reused`, BuildId, 그리고 두 산출물의 경로와 해시를 반환합니다. 이 작업별 결과는 최종 검증에서 관찰한 정확한 현재 trusted commit도 기록하므로, 같은 불변 build를 안전하게 재사용하더라도 작업마다 달라질 수 있습니다. 캐시 재사용은 먼저 BuildId plan을 재현하고 현재 freshness와 저장된 manifest 및 산출물 해시를 검증합니다. 같은 BuildId에 다른 내용이 이미 있으면 손상으로 간주하며 덮어써서는 안 됩니다(`MUST NOT`).

호출자가 선택한 출력 경로는 초기 resolution의 일부가 아닙니다. 이후 read/export 작업은 managed 원본, BuildId, lineage 또는 integrity 검사를 바꾸지 않고 검증된 산출물을 stdout으로 스트리밍하거나 호출자가 선택한 목적지에 복사할 수 있습니다(`MAY`).

Pilot이 `verify-context-build <request.json> <sha256:BuildId>`로 노출하는 별도의 선택적 심층 검증 작업은 정확한 Context Resolution request 하나와 예상 BuildId 하나를 받을 수 있습니다(`MAY`). 이 작업은 request를 캡처하고 현재 trusted authority를 해석해야 합니다(`MUST`). 캡처한 request는 독립적인 비게시 경로에서 컴파일해야 하며(`MUST`), 최종 비교 전까지 이름이 지정된 managed build를 참조하거나 그 파일 또는 바이트를 재사용해서는 안 됩니다(`MUST NOT`). 그런 다음 파생된 BuildId와 managed `context.md` 및 `manifest.json`의 닫힌 파일 집합이 정확히 일치함을 요구해야 합니다(`MUST`). 성공 직전에는 resolution이 사용하는 동일한 전체 작업 일관성 경계를 통해 request, 관찰한 모든 mutable Source, trusted ref, active Checkpoint, 그리고 이름이 지정된 managed build의 디렉터리 정체성, 닫힌 파일 집합, 경로 타입과 심볼릭 링크 상태 및 두 산출물의 바이트를 재검증해야 합니다(`MUST`). 성공 결과는 BuildId, 현재 trusted commit 및 산출물 해시를 식별하는 bounded structured result입니다.

검증기는 제공된 request와 현재 trusted authority만으로 재현하며, 과거 authority를 복원하는 API가 아닙니다. 잘못된 BuildId 문법, 파생 정체성 불일치, managed build의 부재 또는 non-regular 상태, 추가 또는 누락 파일, 심볼릭 링크 경로, 달라진 산출물 바이트, 동시 입력·authority·managed-build 변경은 eligible result 없이 실패해야 합니다(`MUST`). 검증은 ContextBuild를 생성, 교체, 격리하거나 그 밖의 방식으로 변경해서는 안 됩니다(`MUST NOT`). repository-local serialization lock은 검증을 게시 작업으로 만들지 않는 범위에서 사용할 수 있습니다(`MAY`).
