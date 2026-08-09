---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.knowledge.identity
revision: sha256:744628ec91eb5121d7705cb18d88bf5bb5e1fd8a2c7e091feb6eb8d0ddfc8d3f
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:8366d3737dc2e700176677849014ec30638c75e08e4b27d62c7094462937e4d5
---
# Korean Review Projection

## Translation

# 지식 식별자

## 선언

모든 KU는 record 내용에서 읽는 안정적인 의미 기반 `KnowledgeId`를 가져야 합니다. 디렉터리와 파일 이름은 변경 가능한 정리용 힌트이며 identity를 정의하면 안 됩니다. 유효한 record를 옮겨도 `KnowledgeId`는 유지되어야 합니다.

`KnowledgeId`는 소문자 점 구분 의미 segment로 구성되어야 합니다. 각 segment는 ASCII 문자로 시작하고 ASCII 문자 또는 숫자로 끝나야 하며, 소문자 ASCII 문자·숫자·내부의 단일 하이픈만 포함해야 합니다. ID는 물리 경로, record kind, revision 또는 첫 consumer를 인코딩하면 안 됩니다.
