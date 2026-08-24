---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.observability.view-projections
revision: sha256:a5736f6b1270b1001ad5755533c01aabe3bde15121a5f43d505280abfaf1aeb4
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:d10fb208e862fc2b33ccf545c97e700641c88d3dac1a6ef599fcb6fa5ffc8d66
---
# Korean Review Projection

## Translation

# Chat, Transcript, Request 및 Session Usage 투영

## 선언

Chat과 Transcript에 표시되는 이력은 읽기 전용 의미적 Session Journal에서 파생되어야 합니다. Request는 같은 Session 생명주기 아래에서 Journal의 제한된 연결·가용성 레코드와 선택적 Request Audit 상세를 결합하는 읽기 전용 진단 투영이어야 합니다. 투영이나 상세가 별도 권위가 되어서는 안 됩니다.

Chat은 사용자가 편집할 수 있는 기본 상호작용 화면이어야 합니다. 기존 코딩 에이전트 상호작용 방식에 따라 간결한 의도, 의미 있는 도구·파일 활동, 테스트, 승인, 실패, 결과를 보여주고 반복 탐색과 긴 출력은 접어야 합니다.

Transcript는 Chat을 포함하는 투명한 시간순 상위 집합이며 상세한 의미 사건과 Activity 생명주기, 문맥, 실패, 명시적인 관찰·영속 공백을 추가해야 합니다. 보관된 Transcript만 렌더링 전에 `Option<NonZeroUsize>` 제한으로 최신 양의 N개 의미 Transcript 레코드를 선택할 수 있습니다. `None`은 모든 의미 Transcript 레코드를 선택해야 합니다. `Some(N)`은 최신 N개를 선택하되 시간순 렌더링 순서와 원래의 1부터 시작하는 레코드 번호를 보존해야 합니다.

보관된 Transcript만 레코드를 선택한 뒤 렌더링하기 전에 `none`, `preview`, `full` 콘텐츠 정책을 적용할 수 있습니다. 이 정책은 사용자 입력, Activity 텍스트 delta, Activity 텍스트 snapshot, Activity 실패 메시지, Turn 실패 메시지를 모두 대상으로 해야 합니다. `none`은 각 값에 정확히 `content.type=<type>`과 `content.utf8_bytes=<전체-바이트-수>`를 출력해야 하며, `<type>`은 `user_input`, `activity_text_delta`, `activity_text_snapshot`, `activity_failure_message`, `turn_failure_message` 중 하나입니다. `preview`는 여기에 `content.preview=<value>`와 `content.preview_truncated=true|false`도 출력해야 합니다. Preview 값은 escape 전 UTF-8 인코딩이 최대 256바이트인 완전한 확장 grapheme cluster들로만 이루어진 가장 긴 prefix를 Debug 형식으로 인용하고 escape한 값이어야 합니다. `content.utf8_bytes`는 항상 원본 전체 값의 바이트 수이며, preview prefix가 원본 바이트를 하나라도 생략할 때만 `content.preview_truncated`가 true여야 합니다. 단일 확장 grapheme cluster가 제한보다 크면 빈 인용 preview와 truncation을 보고해야 합니다. `full`은 기존 field 이름과 값을 바이트 단위로 그대로 보존하고 `content.*` metadata를 출력하지 않아야 합니다. Content selector를 생략하면 `full`이 기본값이어야 합니다. 명시적으로 지정한 레코드 제한이나 content selector는 `full`을 명시한 경우까지 포함해 Transcript에서만 허용되며 Chat과 Request는 거부해야 합니다.

Request는 요청 목록 탐색기가 아니라 해당 Session 전체에 속한 Journal의 모든 제한된 연결·가용성 레코드를 시간순으로 보여주는 전체 화면 읽기 전용 진단 흐름이어야 합니다. 관찰 가능한 backend 통신, revision, 시도, 결과, 민감정보 제거, 정확한 관찰 경계와 상세를 사용할 수 없는 타입화된 이유를 보여줘야 합니다. 대화형 화면은 이 흐름 안에서 현재 Chat 또는 Transcript 문맥을 강조할 수 있습니다. 강조된 문맥에 직접 연결된 요청이 없으면 가까운 요청을 대신 선택하지 말고 요청이 없다고 밝혀야 합니다. 연결된 화면 사이를 오간 뒤에는 각 화면의 cursor와 scroll 상태를 복원해야 합니다. 미래 remote reader의 on-demand 상세 조회는 실제 remote consumer가 그 계약을 정의한 뒤에만 추가할 수 있으며, 이 결정만으로 remote Request Audit interface가 생기지는 않습니다.

Session Usage는 완료된 ModelWork Activity의 usage receipt만 대상으로 하는 읽기 전용 투영이어야 합니다. 독립적인 최상위 `yo usage SESSION_ID` 명령만 Usage를 표시할 수 있으며, receipt를 독립적으로 decode하거나 집계하지 않고 공용 타입화 Session Usage 투영을 소비해야 합니다. Usage는 CLI나 TUI view로 노출되어서는 안 되고, `yo session --view usage`는 유효하지 않아야 하며, F4에는 view binding이 없어야 합니다. 투영은 receipt 시간순을 보존해야 합니다. 각 token 집계는 완전, 부분적, 사용할 수 없음 중 하나여야 합니다. 부분적이거나 사용할 수 없는 집계에는 포함/전체 receipt coverage(x/y)를 표시하여 누락된 값을 완전한 값처럼 보여서는 안 됩니다. Cache-read 비율에는 cache-read token data가 명시되고 input-token 분모를 알 수 있는 receipt만 포함해야 합니다. 그 분모에는 그러한 적격 receipt의 알려진 input token만 포함해야 하며 적격/전체 receipt coverage를 표시해야 합니다. 인식된 완료 receipt가 없는 Session도 빈 투영으로 성공해야 합니다. 인식된 receipt schema에서는 보고된 0, 필드 없음, 미지원 상태를 서로 구분해야 하며, 잘못된 data가 있으면 부분 report를 출력하기 전에 전체 투영을 fail-closed 해야 합니다. Codex 집계에는 turn별 usage만 사용하고 누적 `thread_total`은 제외해야 합니다. Usage는 비용, 과금, cache hit, 비캐시 token, 누락된 귀속, provider 간 cache-write 동등성을 추론해서는 안 되며 raw request·response, credential, 비공개 reasoning 내용을 노출해서는 안 됩니다.

## 이유

하나의 의미적 replay 원본은 간결한 작업 흐름과 투명한 시간순 이력을 일치시킵니다. 선택적으로 연결된 상세는 의미 Journal에 모든 wire data를 넣지 않고도 TUI와 미래 GUI에서 통신 수준 진단을 가능하게 합니다. Transcript에만 적용되는 레코드·content 선택은 완전한 Request trace를 약화하거나 기존의 명시적 `full` 표현을 바꾸지 않으면서 진단 출력을 제한합니다. 전용 Usage 명령은 accounting을 live·archived 관찰 view 밖에 두고, 공용 타입화 투영은 receipt 해석 중복을 막습니다.
