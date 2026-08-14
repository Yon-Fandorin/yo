---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.context.build-identity
revision: sha256:a15ba45126f7344468f808ffa82242dff31971e82a05077f426ab45f6f82a9f5
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:c4735115efac95b71a243c248cfb40a3491646459d7d8e9b095b2034dceba6bf
---
# Korean Review Projection

## Translation

# ContextBuild 식별성과 무효화

## 선언

사용자 작업은 요청마다 context를 다시 만드는 것이 아니라 context를 해석합니다. content-addressed 입력이 같아도 authority mode에 맞는 freshness guard를 통과한 뒤에만 기존 `BuildId`를 재사용합니다. 관련 Knowledge, relation, compiler, projection, tokenizer, direct anchor, 정확한 candidate 입력 바이트 또는 예산이 바뀌면 영향을 받는 결과만 무효화합니다. candidate 입력의 정확한 hash는 BuildId 식별 입력이고 실제 파일 경로는 위치를 찾는 수단일 뿐입니다.

`BuildId`는 버전이 있고 길이로 구분된 canonical build plan을 domain-separated SHA-256으로 계산한 값입니다. plan에는 정확한 context Checkpoint 식별자와 hash, 안정적인 authority-basis commit, 선택된 Knowledge revision과 필수 relation, reason code가 있는 결정적 포함·제외 결정, 그 결정에 영향을 준 모든 Source 및 evidence 관찰, 정규화한 direct anchor, 정확한 candidate 입력 hash, compiler와 payload profile, tokenizer profile, 최대 예산이 들어갑니다. 현재 `develop` 관찰값, 입력·출력 경로, 시각, 결과 상태, 산출물 hash, 그리고 같은 Checkpoint를 trusted active authority로 관찰했는지 명시적인 activation-review-only prospective guard로 관찰했는지는 제외합니다.

그러므로 같은 정확한 Checkpoint를 trusted-active 방식과 activation-review-only prospective 방식으로 해석하면 같은 의미 payload를 컴파일하므로 같은 BuildId가 나옵니다. 구조화된 작업 결과와 이를 소비하는 모든 review plan은 authority mode를 BuildId 밖에 기록하고 그 mode에 맞는 final guard를 적용해야 합니다. 활성화 전 prospective 산출물은 정확한 activation request와 lineage를 포착한 변경 불가능한 activation-review packet에만 사용할 수 있습니다. 활성화 후 ordinary resolution은 같은 Checkpoint가 그때 active trusted authority이고 현재 Source freshness가 유지되며 일반 managed-build 검증이 성공함을 독립적으로 증명한 뒤에만 이 변경 불가능한 바이트를 재사용할 수 있습니다. prospective 성공만으로 build가 일반 용도로 적격해지지는 않습니다.

따라서 관련 없는 trusted-ref 전진은 최종 authority와 freshness 검증 뒤 같은 build를 재사용할 수 있지만, 관련 의미 입력이 하나라도 바뀌면 재사용할 수 없습니다. prospective review 관찰이 trusted active 관찰로만 바뀌었다면 동일한 build를 중복 생성하지 않지만 authority mode별 guard는 항상 다시 실행합니다.

초기 resolver request에는 model 또는 permission 필드가 없고 첫 profile도 model별·permission별 filtering을 하지 않습니다. 향후 버전 profile은 신뢰할 수 있는 파생 출처, 선택 의미, BuildId 참여 규칙을 함께 정의할 때만 그런 입력을 추가할 수 있으며 호출자 문자열만으로 content eligibility를 부여할 수 없습니다.
