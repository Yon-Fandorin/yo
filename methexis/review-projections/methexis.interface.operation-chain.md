---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.interface.operation-chain
revision: sha256:41081f3516a7c67e90910579edd6af517d27cc6d85dc17f5a8d67879c44cc380
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:1bd198afa263143bdb514f32d57bda1fcb155cffb9820f76aa20d5382afb0bd5
---
# Korean Review Projection

## Translation

# Methexis 작업 명령 연쇄와 권한 경계

## 규칙

`semantic-first-ko-on-demand/v1`은 완전한 최소 흐름에만 제공된다. `author-revision`은 Source 및 정본 영문 Knowledge Draft를 기록하고, 저장소가 소유하는 의미 검토가 clear가 된 뒤 `project-review`가 사람 요청 때만 한글을 생성하며, `build-review`는 정확한 영문과 한글 쌍을 보여주고, `prepare-approval`과 `approve`는 제안 및 정확한 사람 승인 경계를 유지한다.

capability는 현재 작업 경로만 선택하고 영구 권한이나 artifact 계보를 만들지 않는다. capability가 없으면 기존 흐름이 기준이며 여전히 exact-revision 사람 승인을 요구한다. 기존 legacy record는 일괄 이관 없이 이미 묶은 정확한 revision에 계속 유효하다.

에이전트 검토 절차, reviewer session 처리 및 review evidence는 저장소 workflow authority만 소유한다. Methexis는 그 workflow disposition만 소비하고 별도의 provider attestation이나 reviewer routing 정책을 정의하지 않는다.

의미 검토가 clear가 되면 `project-review`가 사람 요청에 따라 정확한 현재 revision의 한글을 게시한다. 의미가 바뀌면 의미 검토를 다시 시작하고 번역만 바뀌면 사람 검토를 반복한다. 그 밖의 prepare, Checkpoint, activation, validation 및 ContextBuild 경계는 바뀌지 않는다.
