---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.source.record-format
revision: sha256:098d7b565d9bd48bd5a4ab72ba295da52fed1e457773fcdebb3c4cfe0ffd40f3
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:c69bc6306b97bdad8b3f40e5375744271b25f2cee8811135232f3a03f3d79e3c
---
# Korean Review Projection

## Translation

# Source 레코드 형식

## 선언

각 Source는 `methexis/sources/<kind>/` 아래에 하나의 typed YAML record로 저장해야 합니다. schema는 닫혀 있으며 catch-all payload 또는 알 수 없는 field를 허용하면 안 됩니다. record 내용에서 읽는 안정적인 의미 기반 `SourceId`가 identity이며, directory와 filename은 변경 가능한 정리용 힌트이고 identity를 정의하면 안 됩니다.

record는 원본 content 또는 external locator를 kind별 payload에서 정확히 한 번 소유해야 합니다. code Source는 안전한 저장소 상대 경로, 비어 있지 않은 symbol, 소문자 SHA-256 content hash를 포함해야 합니다. 선택적인 line hint는 탐색 보조 수단일 뿐입니다. path와 symbol은 Source를 찾고, content hash는 drift를 감지하며, symbol은 byte-range extraction boundary가 아닙니다.
