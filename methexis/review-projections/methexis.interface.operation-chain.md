---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.interface.operation-chain
revision: sha256:c38084f25ec43aba646629aced7632d9913a682ca8d047e959861c4d31a436f7
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:ab2a3341048379ee5e815217397f3a01f18b8e0540da15b28fb77b3d872751eb
---
# Korean Review Projection

## Translation

# Methexis operation chain과 권한 경계

## 명세

`canonical-approval-on-demand-projection/v1`은 다음과 같은 완전한 최소 흐름에 대해서만 노출된다. `author-revision`은 Source와 canonical 영문 Knowledge Draft를 작성한다. 저장소가 소유한 의미 검토가 clear된 뒤 `prepare-approval`은 Projection이나 review packet 없이 정확한 canonical 리비전에 직접 결속할 수 있다. `approve`는 별도의 정확한 human authorization 경계를 유지한다.

사람이 한국어로 더 정확히 이해하기를 명시적으로 요청할 때 `project-review`와 `build-review`는 선택적인 분기를 구성한다. 이 분기는 `prepare-approval`이 Projection 근거 요청을 방출하기 전에 정확한 영문·한국어 pair를 생성하거나 재사용한다. 어떤 operation도 Projection을 암묵적으로 생성하거나, 검토 근거를 바꾸거나, review를 approval로 취급해서는 안 된다.

이 capability는 현재 operation 경로를 선택할 뿐 durable authority 또는 artifact lineage를 만들지 않는다. 기존 `semantic-first-ko-on-demand/v1`과 `methexis.approval/v1alpha1` Projection 기반 record는 일괄 migration 없이 자신의 정확한 리비전에 대해 계속 호환된다.

Agent review 절차, reviewer session 처리, review evidence는 저장소 workflow authority만 소유한다. Methexis는 그 workflow disposition만 소비하며 별도의 provider attestation 또는 reviewer routing policy를 정의하지 않는다. 다른 prepare, Checkpoint, activation, validation, ContextBuild 경계는 각각 approval record의 명시적 review basis를 소비한다는 점 외에는 바뀌지 않는다.
