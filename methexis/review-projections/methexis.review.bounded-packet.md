---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.review.bounded-packet
revision: sha256:233cbdebfc242308560280e2e61862824fccfa2f503f4bc6b6443e5e2d1bc5c3
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:3082a85d7270df54e7399af269adf355b84e747e2879fcd8c90f79f61dd9c5d1
---
# Korean Review Projection

## Translation

# 관리 페이로드 예산이 있는 Slice 리뷰 패킷

## 선언

Slice 리뷰 패킷은 변경 불가능한 base와 candidate 사이의 Git diff 하나, 관련 활성 Knowledge authority를 담은 검증된 Methexis ContextBuild 하나, 해당 build에 포함되지 않은 모든 필수 저장소 authority의 정확한 바이트·경로·hash, Slice 계약의 정확한 바이트와 hash, 요청된 리뷰 렌즈와 질문, 선언된 검증 근거와 hash, 버전이 있는 delivery profile 하나를 묶어야 합니다. manifest는 이 입력과 함께 base와 candidate commit, trusted commit과 활성 Checkpoint identity, ContextBuild와 산출물 hash, 정확한 diff hash, tokenizer profile, 관리 페이로드 token 수와 최대 관리 페이로드 token 수를 기록해야 합니다.

`ReviewId`는 base와 candidate commit, 정확한 diff hash, trusted commit과 활성 Checkpoint identity, ContextBuild와 산출물 hash, 저장소 authority 경로와 content hash, Slice 계약 content hash, 검증 근거 hash, 리뷰 렌즈와 질문, delivery profile, tokenizer profile, 관리 페이로드 예산을 포함하는 버전이 있는 정규 review plan의 domain-separated hash여야 합니다. 출력 경로, 게시 시간, 작업 상태, packet hash, manifest hash는 비의미적이거나 순환하므로 제외해야 합니다. 정규 plan encoding은 결정적이고 모호하지 않아야 합니다.

Delivery profile은 고정 preamble과 wrapper의 정확한 바이트를 정의하고 정규 패킷을 호출자가 제어하는 모델 가시 페이로드 전체로 만들어야 합니다. token 예산은 선언된 tokenizer profile로 이 페이로드의 모든 바이트를 계산해야 합니다. 호출자가 관찰할 수 없는 provider 제어 system, policy, tool-description overhead는 이 관리 페이로드 예산 밖이며 전체 리뷰어 입력 예산의 일부라고 표현해서는 안 됩니다. 패킷은 계산되지 않은 호출자 제어 지시나 authority에 의존해서는 안 됩니다.

생성 과정은 조립 전에 diff, authority 파일, Slice 계약, 검증 근거, ContextBuild 참조를 변경 불가능한 snapshot으로 포착해야 합니다. 새 패킷을 게시하거나 재사용 패킷을 반환하기 직전에 trusted ref, 활성 Checkpoint identity, ContextBuild freshness, 포착한 모든 hash, delivery profile 전체 바이트, candidate worktree 청결성을 최종 재검증해야 합니다. 입력이 바뀌거나 profile이 잘못되거나 정규 패킷이 예산을 넘으면 적격 패킷을 반환하지 않고 실패해야 합니다. diff, Knowledge 본문, authority, 리뷰 질문, 검증 근거를 잘라서는 안 됩니다.

패킷과 manifest는 임시 sibling과 덮어쓰기 없는 설치를 사용해 하나의 원자적 create-if-absent 산출물 집합으로 게시해야 합니다. 기존 ReviewId는 두 파일과 기록된 모든 입력이 정확히 재현될 때만 재사용할 수 있습니다. 누락되거나 추가되거나 일치하지 않는 바이트가 있으면 대체하지 않고 실패해야 합니다.

## 단계

1. 토큰 예산이 있는 Methexis ContextBuild로 관련 활성 Knowledge를 해석하고 반환된 context와 manifest identity를 포착합니다.
2. 정확한 비마이그레이션 저장소 authority, Slice 계약, 검증 근거, 깨끗한 candidate, 선언된 base와 candidate commit 사이의 rename 추론 없는 binary Git diff를 포착합니다.
3. 버전이 있는 정규 review plan을 만들고 domain-separated ReviewId를 파생합니다.
4. 버전이 있는 delivery profile을 선택하고 고정 wrapper와 포착한 모든 입력을 호출자가 제어하는 모델 가시 페이로드 전체로 조립합니다.
5. 선언된 tokenizer profile로 정규 페이로드 전체를 계산하고 요청 예산을 넘으면 fail-closed합니다.
6. trusted authority, 활성 Checkpoint, ContextBuild freshness, 모든 hash, delivery 바이트, worktree 청결성을 최종 재검증합니다.
7. 패킷과 manifest를 원자적으로 게시하거나 정확히 검증한 뒤 경로, hash, ReviewId, 관리 페이로드 token 수만 반환합니다.

## 완료 조건

하나의 변경 불가능한 패킷과 manifest가 정확한 ReviewId plan, base와 candidate commit, trusted authority, 활성 Checkpoint, ContextBuild 계보, 저장소 authority 바이트, Slice 계약 바이트, 검증 근거, diff, 리뷰 지시, delivery profile, tokenizer, 관리 페이로드 수와 예산을 재현하고, 수치가 예산 이내이며, 게시와 재사용 모두에서 최종 재검증이 성공하고, candidate worktree가 깨끗하며, 부분적·추가·상이한 산출물 바이트가 승인되거나 대체되지 않아야 완료됩니다.
