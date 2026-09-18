# 런타임 후속 백로그

최초 발견 기준은 2026-09-16의 develop `d21a457e6780`이며, 마지막 정리는
2026-09-19의 통합 develop을 기준으로 한다. 이 문서는 해결되지 않은 런타임 작업과
그 판단 결과만 보존한다.

## 현재 보류 항목

없음.

## 2026-09-19 해결 기록

### 명령 artifact 최종 검증과 spawn 사이의 TOCTOU

**판정:** 기존 계약의 host-environment 한계를 유지한다. 원래 interpreter·script·native
path 의미를 보존하면서 지원 Unix 전체에서 hash bytes를 원자적으로 실행한다고 주장하지
않는다.

**처리:** child waiter 초기화를 artifact 최종 검증 앞으로 옮겨 검증 뒤 남는 host setup을
제거했다. 결정적 post-verification barrier 테스트는 조율되지 않은 publisher가 그 뒤 path를
교체하면 운영체제가 교체된 script를 열 수 있음을 고정한다. 같은 경계에서 취소되면 단일
spawn 전에 Interrupted로 끝나고 script가 실행되지 않는 회귀도 고정한다. 이 결과는
`agent.tool.local-execution-boundary`의 명시적 비원자 path-execution 한계와 일치한다.

### AgentSession terminal failure 전달

**판정:** 별도 failure ledger와 blocking terminal signal을 제거하고 worker-owned terminal
payload 하나로 통합했다.

**처리:** 용량 1의 lane에는 `Changed`만 남기고 `Failure`와 `Closed`는 별도 terminal
payload에 비차단으로 공개한다. 관찰과 공개를 같은 lock 순서로 직렬화해 이미 공개된
`Changed`가 terminal보다 먼저 보이게 했고, frontend가 failure를 poll하지 않아도 shutdown이
회수한다. unread change, failure race, readiness, shutdown 회수, lock 경쟁 회귀를 추가했다.

### credential 절대 경로와 상대 `YO_CONFIG`

**판정:** credential repository의 nonempty absolute-path 계약을 적용하면서 상대
`YO_CONFIG`의 기존 cwd 기준 의미는 보존한다.

**처리:** `LocalCredentialRepository` construction과 모든 storage 진입점은 filesystem 접근
전에 빈 경로와 상대 경로를 거부한다. CLI config 경계는 명시적인 상대 `YO_CONFIG`를 현재
절대 작업 디렉터리에 한 번 고정한 뒤 sibling state 경로를 만든다. 상대 config load에서
credential snapshot capture까지 이어지는 회귀와 absent-store 무생성 동작을 추가했다.

### Session 파일 NOFOLLOW와 root·pending·FIFO 경계

**판정:** local Session reader와 append/lock 경로를 descriptor-relative open으로
강화했다.

**처리:** reader는 열린 repository tree descriptor를 계속 사용하고 Session log와 pending
marker를 `NOFOLLOW`, `NONBLOCK`, `CLOEXEC`로 열어 user-only regular file만 받는다. append와
lock도 pinned parent/root 아래에서 최종 entry를 연다. 열린 뒤 root pathname 교체, Session
log symlink, pending symlink, Session log와 pending FIFO가 follow 또는 block하지 않는 bounded
회귀를 추가했다.

## 이전 해결 기록

- 중복 TLS fixture는 `176384b0`에서 `yo-test-support` 단일 owner로 통합했다. OpenAI
  Responses, Kimi, OpenRouter, QwenCloud runtime fixture는 shared lifecycle을 사용한다.
- Session repository absolute-root 경계는 `a9f9f5fc`, executable helper owner 정리는
  `a13d0d86`, bounded-run signal cleanup은 `2056618e`와 `d21a457e`에서 완료했다.

새 회귀나 새로운 플랫폼 보장 요구가 생길 때만 별도 항목으로 다시 연다. 과거 후보 브랜치와
실험 커밋은 현재 구현이나 작업 상태의 authority가 아니다.
