---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.review.bounded-packet
revision: sha256:eeae51da2020f1543d96292966cb3ba3c5e4f5b8d57c791c211ef868f49b6594
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:6a59c44074d131cb5a9d5083645892d750aa91943a3918cccd085b0e0d2981dc
---
# Korean Review Projection

## Translation

# 관리 페이로드 예산이 있는 Slice 리뷰 패킷

## 선언

일반 Slice 리뷰 패킷은 변경 불가능한 base와 candidate 사이의 Git diff 하나, 관련 active Knowledge authority를 담은 검증된 Methexis ContextBuild 하나, 그 build에 없는 모든 필수 저장소 authority의 정확한 바이트·경로·hash, Slice 계약의 정확한 바이트와 hash, 요청된 review lens와 질문, 선언된 검증 근거와 hash, 버전이 있는 delivery profile 하나를 묶어야 합니다. manifest는 이 입력과 base/candidate commit, trusted commit과 active Checkpoint identity, ContextBuild와 산출물 hash, 정확한 diff hash, tokenizer profile, 관리 페이로드 token 수와 최대치를 기록해야 합니다.

실험적인 activation-review packet은 버전이 있는 request가 깨끗한 candidate worktree 안의 activation-request 파일 하나를 명시할 때만 prospective ContextBuild를 사용할 수 있습니다. 이 경로는 고정한 trusted `develop`을 기준으로 정확한 불변 Checkpoint 제안과 canonical active-record 전환을 재현하고, predecessor active-record hash, approval closure, 현재 Source freshness, 등록 artifact lineage를 검증해야 합니다. packet, plan, manifest, 구조화된 결과, 모델 가시 지시는 context authority를 `prospective`라고 표시해야 하며 active, trusted, approved, eligible이라고 표현해서는 안 됩니다. 호출자가 고른 ref, 임의의 future tree, 추론한 proposal, ordinary resolution 실패 뒤 fallback, working-tree-only authority는 fail closed해야 합니다.

`ReviewId`는 버전이 있는 canonical review plan의 domain-separated hash여야 합니다. plan에는 authority mode, base/candidate commit, 정확한 diff hash, trusted commit, 정확한 context Checkpoint identity와 stable authority-basis commit, ContextBuild와 artifact hash, prospective mode의 activation-request 경로와 content hash, 저장소 authority 경로와 content hash, Slice-contract content hash, validation-evidence hash, review lens와 질문, delivery profile, tokenizer profile, 관리 페이로드 예산이 들어갑니다. 출력 경로, 게시 시각, 작업 상태, packet hash, manifest hash는 비의미적이거나 순환하므로 제외합니다. canonical plan encoding은 결정적이고 모호하지 않아야 합니다.

게시된 request, plan, manifest, delivery-profile, verifier 식별자는 모두 동결된 동작 경계입니다. prospective activation review는 가장 작은 새 실험적 `v1alphaN` family와 명시적 schema dispatch를 사용해야 하며 기존 식별자를 재해석해서는 안 됩니다. 이전 packet은 정확히 재현 가능해야 하고 호환되는 delta chain의 root로 계속 사용할 수 있습니다.

새 경로에는 닫힌 bootstrap이 있습니다. 이를 가능하게 하는 Source와 Knowledge revision, 실행 구현, 버전이 있는 request/plan/manifest/delivery/verifier family, `CONTRIBUTING.md` workflow 채택은 각각 기존 ordinary active-ContextBuild review·approval·activation·integration 절차를 완료해야 합니다. 모든 enabling 변경이 trusted이고 이 계약이 active가 되기 전에는 경로가 비활성 상태여야 하며, 자기 자신을 가능하게 하는 변경을 build·verify하거나 그 review evidence를 제공해서는 안 됩니다. 최초 사용 가능 대상은 그 이후의 별도 activation candidate입니다. workflow ownership을 바꾸려면 `methexis.workflow.self-hosting-boundary`가 소유하는 완전한 atomic migration이 필요하며 이 경로는 그런 migration이 아닙니다.

Delivery profile은 고정 preamble과 wrapper의 정확한 바이트를 정의하고 canonical packet을 호출자가 제어하는 모델 가시 페이로드 전체로 만들어야 합니다. token 예산은 선언한 tokenizer profile로 해당 페이로드의 모든 바이트를 계산해야 합니다. 호출자가 관찰할 수 없는 provider 제어 system·policy·tool-description overhead는 관리 페이로드 예산 밖이며 전체 reviewer 입력 예산에 포함된다고 표현해서는 안 됩니다. packet은 계산되지 않은 호출자 제어 지시나 authority에 의존해서는 안 됩니다.

생성 과정은 조립 전에 diff, authority 파일, Slice 계약, 검증 근거, ContextBuild 참조, 선택적인 activation request를 불변 snapshot으로 포착해야 합니다. 새 packet을 게시하거나 재사용 packet을 반환하기 직전에 trusted ref, authority mode와 정확한 Checkpoint identity, ContextBuild freshness, 포착한 모든 proposal 파일과 hash, delivery-profile 전체 바이트, candidate HEAD, base-to-candidate diff, candidate worktree 청결성을 최종 재검증해야 합니다. 입력 변경, 잘못된 profile, 예산 초과 시 적격 packet을 반환하지 않고 실패해야 하며 diff, Knowledge 본문, authority, review 질문, 검증 근거를 잘라서는 안 됩니다.

packet과 manifest는 임시 sibling과 no-clobber 설치를 이용해 하나의 atomic create-if-absent artifact set으로 게시해야 합니다. 기존 ReviewId는 두 파일과 기록된 모든 입력이 정확히 재현될 때만 재사용할 수 있고 누락·추가·불일치 바이트가 있으면 교체하지 않고 실패해야 합니다. prospective packet 성공은 review evidence일 뿐이며 활성화에는 여전히 ordinary staged transition gate, trusted integration, post-integration full authority check가 필요합니다.

## 절차

1. request의 명시적 authority mode를 고릅니다. 기존 경로로 ordinary active Knowledge를 해석하거나, 정확한 activation request를 검증해 prospective Checkpoint에 대해 요청 context를 컴파일합니다.
2. 선언한 base/candidate의 no-renames binary Git diff, 마이그레이션되지 않은 저장소 authority, 선택적 activation request, Slice 계약, 검증 근거, 깨끗한 candidate를 정확히 포착합니다.
3. version 소유 canonical review plan을 만들고 domain-separated ReviewId를 파생합니다.
4. 일치하는 versioned delivery profile을 선택하고 고정 wrapper와 모든 입력을 조립해 호출자가 제어하는 모델 가시 페이로드 전체를 만듭니다.
5. 선언한 tokenizer profile로 전체 canonical payload를 계산하고 예산을 넘으면 fail closed합니다.
6. trusted authority, authority mode, proposed 또는 active Checkpoint, ContextBuild freshness, proposal 파일과 hash, delivery bytes, diff identity, worktree cleanliness를 최종 재검증합니다.
7. packet과 manifest를 atomic하게 게시하거나 정확히 검증한 뒤 경로, hash, ReviewId, authority label, 관리 페이로드 수만 반환합니다.

## 완료 기준

하나의 변경 불가능한 packet과 manifest가 version 소유 ReviewId plan, base/candidate commit, trusted basis, authority mode, active 또는 prospective Checkpoint, 선택적 activation request와 canonical transition lineage, ContextBuild lineage, 저장소 authority 바이트, Slice 계약 바이트, 검증 근거, diff, review 지시, delivery profile, tokenizer, 관리 페이로드 수와 예산을 정확히 재현하고, 수치가 예산 안이며, 게시와 재사용 모두에서 final revalidation이 성공하고, candidate worktree가 깨끗하며, 부분·추가·상이·추론·self-enabling·authority-promoting artifact가 수락되거나 교체되지 않을 때만 완료입니다.
