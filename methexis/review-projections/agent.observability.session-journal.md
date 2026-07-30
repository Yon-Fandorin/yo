---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.observability.session-journal
revision: sha256:4fe10bfa0f0d1ee7637208d450ab389316417014fd7c95fe03ed304ff9922c2a
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:b06fe0e2a01a6083d0965283fd9c67353942eeec1e33608ef7ce7aca3cea8176
---
# Korean Review Projection

## Translation

# 영속 세션 관찰 저널

## 계약

하나의 순서가 보장된 영속 세션 저널이 세션 이력과 화면을 위한 의미적 재생 원본이어야 합니다. 저널은 백엔드 중립적인 세션·턴·활동 사건과 함께, payload가 없는 bounded Request 연결·가용성 레코드를 기록해야 합니다. 안정적인 operation ID, 수락된 요청의 ID, 연결된 재개 가능 결과, 백엔드 종류와 버전, 관찰 경계, 통신 종류와 방향, payload schema ID, 이어가기에 필요한 버전이 명시된 백엔드 세션 locator는 저널 레코드에 속하며 Request 상세가 아닙니다. 요청, 응답, 알림, 서버가 시작한 요청, 재시도, 최종 결과는 상세가 없어도 연결 레코드를 통해 구분할 수 있어야 합니다.

요청 payload, header, revision·attempt 증거 같은 백엔드별 Request Audit 상세는 같은 세션 저장소 생명주기 아래의 논리적으로 구분된 선택적 진단 영역입니다. 이 상세는 의미 원본이 될 수 없고, 상세가 없어도 세션 저널 재생과 이어가기 기준점 검증은 가능해야 합니다. 의미는 수집할 때 확정해야 하며 오래된 백엔드 payload만 다시 해석해서 복원하면 안 됩니다. 상세가 없거나 미지원·휘발성·미저장 상태라면 이를 명시해야 하며, 이 상태가 관련 없는 의미 레코드의 재생을 막아서는 안 됩니다.

내구 상세를 저장하기 전에는 redaction이 완료되어야 합니다. 자격 증명, 전체 환경 변수, 비공개 추론 값과 그 밖의 금지된 원시 값은 내구 저장소에 들어가면 안 되며, 제거가 해석에 영향을 주면 그 사실을 명시해야 합니다. 이 admission 경계가 구현되기 전까지 Request 상세는 프로세스 로컬의 휘발성 데이터여야 합니다.

## 이유

안정적인 의미와 bounded 연결 정보를 세션 저널에 유지하면 백엔드와 독립적인 재생·이어가기가 가능합니다. 선택적 Request Audit 상세는 두 번째 세션 원본이 되지 않으면서 Codex app-server와 향후 직접 모델 전송을 진단하는 방향으로 발전할 수 있습니다.
