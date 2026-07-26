---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.model-ownership
revision: sha256:d1529670a39e3d9ca4cda0fcaf822c2afee833043334b1894931f25e821bcd24
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:872e1f71910b744a26616dc06f11b99f88aac59d8dd7fdcdd094d6aec09bd659
---
# Korean Review Projection

## Translation

`Surface`는 완성된 2차원 셀 상태를 결정론적으로 소유해야 합니다. 터미널 진입·복구·커서 정책·I/O와 viewport보다 큰 논리적 스크롤 위치는 모델 밖에 있어야 합니다.

완성된 프레임은 터미널과 HTML adapter가 함께 사용할 수 있지만, lifecycle과 application state는 환경과 mode마다 달라지기 때문입니다.
