# Research notes

이 디렉터리는 구현과 제품 결정을 준비하면서 확인한 외부 사례를 보존한다.
Research note는 비교 근거와 upstream 출처를 제공하지만 Yo의 제품 계약이나
설계 권위가 아니다. 승인된 동작은 각 note가 연결하는 Methexis KnowledgeUnit이
소유한다.

## Topics

- [Yo 전체 제품·구조 비판 검수와 재정렬한 배달 순서](./architecture-evolution/critical-review.md)

- [Yo architecture evolution: Pi/Codex comparison and concrete design](./architecture-evolution/README.md)
- [Pi·Codex·Yo module boundaries, ownership, and change impact](./architecture-evolution/module-comparison.md)
- [Pi·Codex 기준 Yo 사용성 개선과 전체 검수표](./architecture-evolution/review-index.md)
- [TUI 내부 UI/UX 64개 요소·전이·모듈별 검수](./architecture-evolution/tui-review.md)
- [Coding-agent context compaction](./context-compaction/README.md)

각 topic은 조사 기준일과 비권위 상태를 표시하고, upstream에서 확인한 사실과
Yo에 대한 적용 판단을 구분한다. Upstream이 변경되면 기존 문장을 현재 동작처럼
조용히 고치지 말고 조사 기준일과 source를 함께 갱신한다.

[TUI ANSI 디자인 예시와 오프라인 갤러리](./architecture-evolution/tui-ansi/README.md)는
Pi·Codex 소스에 근거한 66개 제안 장면과 실제 ESC/TXT, 요소별 검수표를 제공한다.
