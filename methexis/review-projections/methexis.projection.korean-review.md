---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.projection.korean-review
revision: sha256:21911aa16094c5c789703d79c456b5124adfb41d43024307e6f2ccd02d97eeca
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:000d75bf1d21e2b92eeeae0e26e689ade7efed269b67c07d5e2545fee232a035
---
# Korean Review Projection

## Translation

# 필요할 때 생성하는 한국어 검토 Projection

## 명세

Source record와 canonical 영문 Knowledge가 의미 작성 및 agent 검토 표면이다. 완전한 `canonical-approval-on-demand-projection/v1` capability는 한국어 검토 Projection을 모든 승인에 필요한 선행 조건이 아니라, 사람이 이해를 더 확실히 하기 위한 선택적 보조 수단으로 만든다. 이 capability가 없으면 기존 `semantic-first-ko-on-demand/v1` 흐름이 계속 제어한다.

새 capability 아래에서는 authoring, approval, activation, validation, ContextBuild operation 중 어느 것도 한국어 Projection을 암묵적으로 생성해서는 안 된다. 사람이 한국어로 더 정확히 이해하기를 명시적으로 요청하면 `project-review`는 현재 정확한 리비전의 Projection이 있을 때 재사용하고, 없거나 stale이면 하나를 생성하거나 교체한다. 요청은 정확한 현재 `RevisionId`를 지정하며 교체 시 predecessor hash도 지정한다. Projection은 해당 리비전, profile, compiler, deterministic request lineage, 정확한 bytes에 결속된다. 직접 수정, malformed record, lineage drift는 fail closed한다.

Canonical 근거 승인은 Projection을 요구하지 않는다. 참조되지 않은 stale Projection은 검토 증거로 부적격이지만, 일치하는 canonical 승인 또는 activation을 막아서는 안 된다. Projection 근거 승인은 여전히 정확한 영문·한국어 pair를 요구하고 Projection hash에 결속된다. 의미 변경은 영문 검토로 돌아가며, 번역만 바뀌면 human review만 다시 한다. 기존 legacy artifact는 일괄 migration 없이 자신이 승인받은 정확한 리비전에 대해 계속 유효하다.
