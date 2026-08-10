---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.projection.korean-review
revision: sha256:ee1d095ce3698562126d18ff724e6f0fe4c2446d846a2315276bfdc8055d8f62
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:e3ba521b5f9cafb1a72fb97fd0ece87a645509a3e4bad05054886afa3c0c4072
---
# Korean Review Projection

## Translation

# 요청 시점 한글 검토 투영본

## 규칙

Source 레코드와 정본 영문 Knowledge가 의미 작성 및 에이전트 검토 표면이다. 이 흐름은 완전한 `semantic-first-ko-on-demand/v1` capability가 있을 때만 제공된다. capability는 현재 작업 경로만 선택하며 영구 권한이나 artifact 계보를 만들지 않는다. capability가 없으면 기존 흐름이 기준이며 여전히 정확한 사람 승인을 요구한다.

capability가 있으면 `author-revision`은 Source와 Knowledge만 변경하고 한글 Markdown을 입력받거나 생성하거나 복사하거나 교체하지 않는다. 오래된 기존 Projection은 현재 검토나 승인 증거가 아니며 authority validation이 거부한다.

정확하고 깨끗한 의미 후보가 필수 검토를 통과한 뒤, 사람이 명시적으로 요청할 때만 `project-review`가 추적 한글 Projection 하나를 생성하거나 교체한다. 요청은 정확한 현재 `RevisionId`와 교체 시 이전 hash를 지정한다. Projection은 revision, profile, compiler, 결정적 요청 계보 및 정확한 바이트를 묶는다. 직접 편집, revision 불일치 또는 계보 불일치는 구조적 실패다.

사람은 정확한 영문 revision과 한글 Projection을 함께 검토하고 approval은 revision과 Projection hash를 묶는다. 의미 변경은 영문 전용 검토로 돌아가고 번역만 바뀌면 사람 검토를 반복한다. 기존 legacy artifact는 일괄 이관 없이 정확한 승인 revision에 계속 유효하다.
