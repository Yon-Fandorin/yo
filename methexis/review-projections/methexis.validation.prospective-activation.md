---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.validation.prospective-activation
revision: sha256:6112bf05d3f272ac676a172d7b3d139291ab4f2175a2a53fdb02f506624b310a
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:a466cc65657fe99d22ce06d9b44c2290880c566faf4d0d17ddbc8a85667687a5
---
# Korean Review Projection

## Translation

# Prospective staged activation 검증

## 선언

`methexis check --staged-activation`은 수정된 approval이 trusted `develop`에 들어간 뒤 replacement Checkpoint가 통합되기 전까지의 불가피한 구간을 위한 repository-hook 경로입니다. staged active-record 변경이 없으면 ordinary all-class `check`와 정확히 같게 동작합니다. 변경이 있으면 Git index에서 새 불변 Checkpoint 하나, active record, 등록된 tracked artifact 전체 집합만 허용하고 관련 없는 staged path는 fail closed합니다.

staged 경로는 read-only prospective 경로이지 trusted authority가 아닙니다. `develop`을 한 번 해석하고, 그 정확한 trusted commit에서 proposed Checkpoint를 재현하며, active record의 정확한 predecessor hash와 canonical bytes를 검증하고, 선택한 모든 Source의 freshness와 staged artifact provenance를 요구하며, 반환 전에 Source·proposal index·trusted ref 안정성을 다시 검증합니다. 명시적 `GIT_INDEX_FILE`을 포함해 commit 호출이 고른 정확한 Git index를 고정하고 regular stage-zero가 아닌 entry를 거부합니다. 성공은 candidate를 `prospective`로 표시하며 정확히 검토한 전환이 통합된 뒤 ordinary full `check`를 요구합니다. caller-selected ref, 임의 future tree, working-tree-only candidate bytes, 일반 hook 예외를 허용해서는 안 됩니다.

별도의 명시적인 review-only 작업은 깨끗한 activation candidate worktree 안의 정확한 activation-request 파일을 사용할 수 있습니다. 이 작업은 trusted `develop`을 한 번 해석하고, request와 하나의 불변 proposed Checkpoint, canonical proposed active record를 포착하며, Checkpoint의 stable authority basis가 고정한 trusted commit과 같은지 확인해야 합니다. 또한 active record의 정확한 predecessor, approval closure, Source freshness, 등록 manifest lineage 전체를 검증하고 호출자가 명시한 ContextBuild request만 컴파일하며, 반환 또는 artifact 재사용 전에 모든 proposal 파일, Source 관찰, context request, trusted ref를 최종 재검증해야 합니다. 결과는 별도의 실험 schema를 사용하고 `authority: prospective`, 정확한 trusted commit, proposed Checkpoint를 기록하며 불변 activation-review packet 입력으로만 적격해야 합니다.

review-only 작업에는 닫힌 bootstrap이 있습니다. 이를 가능하게 하는 Source와 Knowledge revision, 구현, 버전 protocol family, `CONTRIBUTING.md` workflow 채택은 모두 기존 ordinary active-ContextBuild review와 authority 순서를 통과한 뒤에야 작업을 활성화할 수 있습니다. 자기 자신을 가능하게 하는 review·approval·activation·workflow 변경을 build·verify하거나 그 evidence를 제공해서는 안 됩니다. 최초 사용 가능 대상은 enabling 구현과 workflow가 trusted이고 이 계약이 active가 된 이후의 별도 activation candidate입니다.

review-only 작업은 caller-provided ref를 고르거나, 임의 future tree를 허용하거나, activation proposal을 추론하거나, ordinary context resolution 실패 뒤 fallback하거나, Checkpoint를 approve·activate하거나, 일반 context eligibility를 충족하거나, candidate를 ordinary authority consumer에 공개해서는 안 됩니다. prospectively 컴파일한 immutable ContextBuild는 활성화 뒤 같은 Checkpoint와 content identity를 공유할 수 있지만 ordinary 재사용은 그때의 active trusted authority와 현재 freshness를 독립적으로 검증해야 합니다.

이 계약은 두 commit authority transition의 후반을 기계화할 뿐 수정 approval과 Checkpoint를 하나의 authority commit으로 만들지 않습니다. 따라서 accepted approval commit과 바로 뒤의 activation commit 사이에서 trusted ref가 의도적으로 불일치할 수 있습니다. 이 제한된 구간 동안 ordinary `check`, ordinary `resolve-context`, 다른 모든 authority-consuming 작업은 계속 실패하거나 유효한 active authority만 사용합니다. prospective review 성공은 approval, activation, context eligibility를 부여하지 않습니다. staged gate는 통합 전에 계속 필수이고 ordinary full `check`는 정확한 transition이 trusted `develop`에 도달한 뒤 계속 필수입니다.
