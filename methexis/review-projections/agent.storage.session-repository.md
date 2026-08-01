---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.storage.session-repository
revision: sha256:d5ea34e4593ea0bb42b6ec72489e51f7713bad9c5af503edd7372d18106643b3
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:53e033ac8c07ba018bc75b86831cdc7a824b9ebcb468c86873d3b54ede2a3bf1
---
# Korean Review Projection

## Translation

# 세션 저장소와 용량

## 계약

저장 방식에 종속되지 않는 Session Repository가 파일·SQLite 같은 물리 구조를 프런트엔드 계약으로 노출하지 않고 영속 Session 기록의 수명주기를 소유해야 합니다. 의미 Session Journal과 Request Audit은 그 소유 경계 안의 논리적으로 다른 도메인이며 서로 다른 Session 권위가 아닙니다. 현 제품은 별도 Request Audit 저장소 인터페이스나 범용 append-log 추상화를 만들지 않습니다. 로컬 구현을 먼저 제공하고, 원격 저장소와 Request Audit의 물리 분리는 증거가 생긴 뒤에만 추가합니다. 복제, 이중 쓰기, 충돌 해결은 첫 로컬 구현 범위가 아닙니다.

첫 로컬 구현은 Session마다 하나의 추가 전용 버전 JSON Lines 로그를 사용하고, JSONL은 프런트엔드 계약이 아닌 교체 가능한 저장 세부사항으로 남아야 합니다. 하나의 의미 커밋은 하나의 물리 repository 봉투로 인코딩해야 합니다. 0개 이상의 결과 사건을 가진 명령이나 관찰 사건 묶음의 일부만 별도 물리 append로 영속되어서는 안 됩니다. JournalSequence는 의미 재생 순서를 나타내고 RepositorySequence는 물리 append 순서를 나타내며, 어느 한쪽을 다른 쪽에서 추론하면 안 됩니다.

새로 쓰는 모든 물리 레코드는 스키마, Session ID, RepositorySequence, 레코드 종류, 정확한 페이로드 바이트를 명시적으로 연결한 입력에 대해 버전이 있는 CRC32C를 가져야 합니다. 복구는 포맷 호환성 계약이 명시적으로 지원하는 이전 레코드만 읽어야 하며, 체크섬이 있는 레코드는 받아들이기 전에 검증해야 합니다. 체크섬 필드 자체가 들어 있는 전체 JSON을 다시 직렬화해 체크섬을 계산해서는 안 됩니다.

두 논리 도메인은 첫 구현에서 하나의 물리 가용성 경계와 용량 상한을 공유합니다. 저장소는 제한된 페이로드 없는 Request 상관관계의 초기 영속 위치입니다. Request 상세는 영속 저장 전에 민감정보를 제거하는 수용 게이트가 구현되기 전까지 프로세스 로컬의 휘발성 데이터로 남고, 첫 구현에서는 독립적인 보존·축출 정책을 갖지 않습니다.

복구는 완전한 줄을 스트리밍하고, 불완전한 마지막 줄은 커밋되지 않은 꼬리로 처리하며, 완전한 줄의 손상은 보고해야 합니다. 제한된 suffix를 반환하기 위해 전체 로그를 메모리에 올리면 안 됩니다. 저장소 root마다 writer 하나만 허용하고, 열 때 root를 안정적인 절대 위치로 확정해야 합니다. 모든 물리 append는 영속 pending marker로 보호해야 하고, rollback을 확인할 수 없으면 marker를 남겨 이후 reader가 모호한 로그를 격리해야 합니다. 비어 있지 않은 Session을 다시 열거나 초기 Session 상태 로드 실패 뒤 복구할 때는 다음 증분 기록보다 먼저 완전한 snapshot을 요구해야 합니다. 의미 커밋 안의 메시지·도구 출력 세그먼트는 내용 저장 세부사항일 뿐 다른 Session 권위가 아닙니다.

영속에 성공한 의미 커밋은 append와 필요한 동기화가 끝난 뒤에만 메모리 Journal에 공개해야 합니다. 프로세스 로컬 화면 갱신은 휘발성이라고 명시된 경우에만 이 규칙 밖에 있으며, 이를 영속 기록으로 노출하면 안 됩니다. 의미 작업은 완료됐지만 저장에 실패하면 결과를 휘발성으로 공개하고 영속 공백을 고정하며, 의미 작업이 롤백됐다고 보고해서는 안 됩니다.

로컬 저장소는 기본 활성화하고, 디렉터리와 파일 접근을 현재 사용자로 제한하며, 설정 가능한 용량 상한을 제공해야 합니다. 시간 만료나 Session 자동 삭제는 하지 않습니다. 상한이나 기반 저장소의 어떤 실패로든 다음 append가 막히면 기존 기록을 바꾸지 않고 활성 Session은 메모리에서 계속 진행합니다. 저장소 소유자는 연결된 모든 프런트엔드에 지속적인 타입화된 저장 압력 알림을 보내고, 내구성 cutoff가 알려진 지점인지, 알려진 빈 로그인지, 알 수 없는지 구분해야 합니다. 알려진 지점은 마지막 영속 JournalSequence와 마지막 RepositorySequence를 모두 포함해야 하며, 영속된 의미 Journal 사건이 하나도 없으면 JournalSequence는 없을 수 있습니다. 어느 좌표도 다른 좌표에서 추론하면 안 됩니다. 공백 이후를 연속 suffix라고 주장하면 안 됩니다. 공간이 돌아오면 완전한 Session snapshot을 먼저 저장한 뒤 이후 증분 기록을 영속 데이터로 받아들입니다.

첫 구현은 동기식 단일 writer 경로로 유지합니다. 측정된 동기화 지연과 append 비율 증거 없이 background writer, 범용 transaction API, group commit을 도입하지 않습니다. 압축, 인덱스, SQLite projection, 대체 인코딩, group commit, Request Audit 물리 분리도 측정된 증거가 있어야 하며, 나중에 분리하더라도 Session 의미를 바꾸거나 Session 수명주기 조정을 옮기면 안 됩니다.

## 이유

로컬 우선 구현은 데이터베이스 선택을 고정하거나 과거 작업을 조용히 버리지 않으면서 즉시 재개와 진단을 지원합니다. 원자적 봉투, 분리된 의미·물리 순서, 체크섬 레코드는 일부만 저장되거나 손상된 상태를 명확하게 만듭니다. 영속 후 공개, 명시적인 저장 압력, snapshot 복구는 실시간 화면을 반응성 있게 유지하면서도 정직한 이력을 보존합니다.
