---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.observability.session-journal
revision: sha256:449870a0ad6568e5e1dda8fe2fa0e177c566344e7fd4b9313a967b022e7416b3
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:823013ae9c2c3d0687cfe1f3333f27c270d837afe3dde5179b63be7b41a07fb3
---
# Korean Review Projection

## Translation

# 영속 세션 관찰 저널

## 계약

하나의 순서가 보장된 영속 세션 저널이 세션 이력과 화면을 재생하는 의미 원본이어야 합니다. 별도의 프로세스 로컬 Live Projection은 반응성 있는 스트리밍에 필요한 아직 커밋되지 않은 꼬리만 소유해야 합니다. 백엔드 전송 델타는 Live Projection을 즉시 갱신하지만 그 자체로 재생 권위가 되어서는 안 됩니다. Session worker만 쓰기를 소유하고, TUI·GUI와 다른 프런트엔드는 안정적인 항목 ID를 이용해 영속된 앞부분과 실시간 꼬리를 합친 읽기 전용 화면을 소비해야 합니다.

원문 텍스트는 해석하거나 공백을 넣는 등의 내용 변경 없이, 변경 불가능하고 순서가 있는 세그먼트로 모아야 합니다. 에이전트 메시지는 버퍼의 UTF-8 텍스트가 16KiB에 도달하거나, 가장 오래된 미커밋 바이트가 1초에 도달하거나, 텍스트가 아닌 순서 경계를 만나거나, 메시지가 종료될 때 세그먼트를 강제로 저장해야 합니다. 도구 출력은 64KiB와 같은 1초, 텍스트가 아닌 순서 경계, 종료 규칙을 사용해야 합니다. 크기로 나눌 때 유효한 UTF-8을 보존해야 하며, 세그먼트를 다시 연결하면 원문과 정확히 같아야 합니다. 세그먼트 경계는 저장 세부사항이며 Chat이나 Transcript에서 메시지 의미를 바꾸면 안 됩니다.

런타임이 관찰한 모든 메시지 종료는 completed, interrupted, failed 중 하나의 타입화된 MessageEnded 결과로 봉인해야 합니다. 종료 결과는 세그먼트를 온전히 재구성했는지 검사할 수 있도록 세그먼트 수와 전체 UTF-8 바이트 수를 포함해야 합니다. 영속 append가 가능하면 마지막의 비어 있지 않은 꼬리와 MessageEnded를 완성된 본문을 중복하지 않고 하나의 물리 커밋으로 원자적으로 저장해야 합니다. 영속 append가 불가능하면 같은 마지막 꼬리와 MessageEnded를 명시적인 휘발성 Live Projection 상태로 원자적으로 공개해야 합니다. 이 휘발성 종료 봉인은 재생 권위나 Continuation Anchor가 될 수 없습니다. 이후의 완전한 Session snapshot은 봉인된 메시지를 포함해야 하며 영속 발행이 끝난 뒤에만 권위가 됩니다. 복구 중 영속 기록에 종료 기록이 없는 메시지는 이후 영속 사건을 받아들이기 전에 interrupted로 봉인해야 하며, 완성된 메시지로 승격하지 않고 부분적인 상태로 명확하게 보여야 합니다.

저널은 이와 함께 백엔드 중립적인 Session·Turn·Activity 사건과 제한된 페이로드 없는 Request 상관관계 및 가용성 기록을 보존해야 합니다. 안정적인 연산 ID, 수락된 요청 ID, 그 요청과 상관 연결된 재개 가능한 결과, 백엔드 종류와 버전, 관찰 경계, 교환 종류와 방향, 페이로드 스키마 ID, 재개에 필요한 버전이 있는 백엔드 Session 위치는 Request 상세가 아니라 Journal 기록에 속합니다. 상세가 없어도 요청, 응답, 알림, 서버 발 요청, 재시도, 종료 결과를 상관관계 기록을 통해 서로 구분할 수 있어야 합니다.

백엔드별 Request Audit 상세는 요청 페이로드, 헤더, revision·attempt 증거를 포함하는 같은 Session Repository 수명주기 아래의 선택적인 별도 진단 도메인이며 의미 권위가 될 수 없습니다. 그 상세가 없어도 Journal 재생, Continuation Anchor 검증, 관련 없는 의미 기록을 막아서는 안 됩니다. 의미는 수집할 때 확정해야 하며 오래된 백엔드 페이로드만 나중에 다시 해석해서 복원해서는 안 됩니다. 상세가 missing, unsupported, volatile, unpersisted 중 어떤 상태인지 명시해야 합니다. 영속 Request 상세를 받아들이기 전에 민감정보를 제거해야 하고, 제거가 해석에 영향을 주면 그 사실도 명시해야 합니다. 자격 증명, 전체 환경 변수, 비공개 추론 값과 금지된 원시 값은 durable Request Audit 저장소에 들어가면 안 됩니다. Schema에 묶인 provider-private replay item은 Request 상세가 아닙니다. 선택된 binding, Connector, backend, continuation, Session Repository 계약이 source, lossless validation, byte bound, binding epoch, durable schema, request projection, 모든 frontend·diagnostic projection으로부터의 제외를 함께 정의할 때만 semantic Journal에 들어갈 수 있습니다. Request-detail admission 경계가 구현되기 전에는 Request 상세를 프로세스 로컬의 휘발성 데이터로 유지해야 합니다.


Durable Session Journal은 bounded payload-free backend correlation과 별도의 bounded model_replay_delta를 구분합니다. Replay delta는 exact visible message role과 bytes, validated function call, bounded function result 및 stable order를 보존하고 Chat 또는 Transcript presentation에서 재구성하지 않습니다. TurnFinished(completed) 뒤와 correlated outcome 및 Continuation Anchor 앞에 같은 physical append로 commit합니다. Tool argument와 output은 Activity update, 후속 model input, replay admission 전에 semantic redaction gate를 통과하며 실제 admitted replacement만 authority가 됩니다.

허용된 provider-private item은 payload-bearing semantic replay로 남고 인접한 visible replay와 atomically commit되어야 합니다. Message segment, Activity, correlation record, Live Projection, Transcript, Request Audit, discovery summary, error, log, diagnostic으로 복사하면 안 됩니다.

## 이유

일시적인 화면 상태와 영속 의미 재생을 분리하면 임의의 백엔드 조각을 Session 계약으로 만들지 않으면서 자연스러운 스트리밍을 제공할 수 있습니다. 제한된 변경 불가능 세그먼트는 장애 시 손실과 레코드 크기를 제한하고, 종료 봉인은 완성된 출력과 복구 가능한 부분 출력을 구분합니다. 안정적인 의미와 제한된 상관관계는 선택적인 Request Audit 상세가 발전해도 백엔드에 종속되지 않습니다.
