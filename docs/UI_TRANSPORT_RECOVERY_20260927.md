# UI transport, responsiveness and recovery — 2026-09-27

## Scope

Keep the shared 2849 checkout and preserve existing SSD thumbnail caching, decoded
image budgets, request coalescing/cancellation, integrity checks, HDD pacing and
background searches. Do not reset the library, retry failed downloads, or release
the app as part of this change.

Implementation sequence:

1. Bounded local diagnostics for renderer liveness, delivery counters and reload
   provenance. Record no image bodies, input text or account credentials.
2. Deliver thumbnail bodies through capability-scoped binary resources, keeping
   the existing coordinator. Bound both the number and bytes of outstanding
   resources and invalidate them on document replacement.
3. Coalesce progress-only UI updates and separate them from gallery metadata and
   structural state changes. Preserve failure, review, cancellation and completion
   transitions and revision fencing.
4. Foreground-only, bounded renderer hang recovery with evidence before reload;
   never restart backend workers. Restore lightweight UI navigation state.
5. Regression tests, development build and a local controlled stress comparison.

Download failure investigation uses read-only SQLite queries. Counts and outcomes
will be recorded separately from newly introduced implementation changes.

## 구현 결과

2026-09-27, 활성 작업 폴더 `2849`에 반영했다. 사용자 종료 확인 후 작업했으며,
실사용 앱을 다시 실행하거나 다운로드를 재시도하지 않았다. DB·앨범·설정·즐겨찾기를
삭제하거나 수정하지 않았고, 커밋·푸시·릴리즈도 하지 않았다.

### 이미지 전달

- 기존 중앙 썸네일 스케줄러, 요청 합치기, 우선순위, 취소, SSD 캐시 및 표시 이미지
  메모리 예산을 유지했다. 이미지마다 대기 스레드를 만드는 방식으로 돌아가지 않는다.
- 실제 앱의 `thumbnail:ready` 이벤트에는 이미지 숫자 배열 대신 1회용 토큰과
  크기·형식만 넣는다. 본문은 `tauri::ipc::Response`의 바이너리 응답으로 전달한다.
- 미수신/미확인 이미지 전달분은 최대 128개·64MiB, 유효기간 45초다. 취소,
  증거 이미지 무효화, 문서 교체 시 폐기한다. 본문 수신 중 UI가 취소되면 Blob을
  새로 만들지 않는다. Blob URL은 기존 수명 관리와 함께 해제한다.
- 이 한도는 전달 구간의 한도이지 WebView 전체 메모리를 64MiB로 제한한다는 뜻은 아니다.

### 진행률과 목록 계산

- 순수 진행률은 전체 작품 Map·작가 목록·검색 메타데이터를 갱신하지 않고,
  해당 카드/활동 항목만 구독하는 저장소에서 처리한다. 표시 갱신은 100ms 단위로 합친다.
- 네이티브 다운로드 알림은 항목별 최신 리비전을 보관하며 화면이 멈춰도 계속 JS에
  밀어 넣지 않는다. UI는 한 번에 한 요청만, 250ms 간격으로 최대 128개를 받는다.
  보관 한도는 4,096항목이며 초과 시 DB 재조회 신호를 보낸다. 다운로드 이력의 원본은
  계속 DB이고, UI 알림은 그 최신 상태를 보여 주는 용도다.
- 실패·검토·완료 등 상태 변경은 순수 진행률과 구분한다. 오래된 리비전이 최신 상태를
  덮어쓰지 않는다. 종료/재시도 이력을 UI 전달 큐로 대체하거나 삭제하지 않는다.
- 자동 탐색 결과 조회가 겹치면 한 요청과 추가 갱신 표시로 합친다. 백그라운드 검색은
  유지하며 검색창 입력 때마다 저장 작품 전체를 다시 계산하는 부분도 메모화했다.

### 진단과 제한적 복구

- 입력 횟수(입력 내용 제외), UI 응답 간격, 긴 JS 작업, JS 힙, 표시 이미지 수·바이트,
  다운로드 전달/알림 수, WebView 프로세스별 private memory를 함께 기록한다.
- F5 키 수신, Hitomi 결과 목록 새로고침, 트레이 새로고침, 자동 복구 요청, 실제 문서
  로드를 구분한다. 과거 F5로 회복된 것을 자동 복구 성공으로 소급 해석하지 않는다.
- 개발 실행본은 표본 메모리 할당 위치를 수집한다. 메모리가 커지거나 복구 직전이면
  힙/DOM 집계와 상위 표본 할당 위치도 남긴다. 원격 디버깅 포트는 열지 않는다.
  릴리즈의 표본 할당 추적은 `ATSUMI_UI_PROFILE=1`일 때만 켜진다.
- 새 로그는 다음 앱 실행부터 `%APPDATA%\local.atsumi.next\diagnostics\ui-health-*.jsonl`에
  쌓인다. 2MiB × 3개 순환 파일, 128개 비차단 기록 큐를 사용한다. 이미지·입력 텍스트·
  검색어·쿠키·전체 IPC 인자는 기록하지 않는다. 트레이의 **진단 기록 저장**으로 집계
  스냅샷도 요청할 수 있다. 디버거의 전체 힙 덤프를 상시 저장하지는 않는다.
- 복구는 메인 렌더러 종료 또는 전면 화면의 지속 무응답에만 적용한다. 정상 시
  heartbeat는 2초 간격이며 무응답 판정은 60초(네이티브 무응답 신호가 있으면 30초)다.
  시작 60초·전면 복귀 40초 유예, 절전 복귀 유예, 복구 직전 재확인을 둔다.
- 메모리 수치만으로 새로고침하지 않으며 백그라운드·최소화 상태의 지연은 멈춤으로
  보지 않는다. 복구는 5분에 최대 2번이다. 다운로드/해시 백엔드 프로세스는 재시작하지 않는다.
- 검색 탭·선택 탭·페이지·스크롤·다운로드 필터를 최대 128KiB의 sessionStorage에 저장한다.
  이미지/작품 목록 전체/다운로드 작업은 저장하지 않는다. 복구 후 선택한 검색 탭부터
  기존 검색 세션을 다시 읽으며, 캐시가 사라진 경우에만 해당 검색을 다시 요청한다.
  다른 복구 탭은 선택할 때 불러와 동시에 수십 검색을 재시작하지 않는다.

## 실제 WebView 비교

임시 프로필·숨겨진 창·로컬 테스트 서버·합성 WebP만 사용했다. 실제 앱 DB나 계정은
열지 않았다. 512×512, 786,526바이트 이미지, 4개 동시 전달, 64장씩 비교했다.

| 측정 | 기존 숫자 배열 이벤트 | 새 토큰 + 바이너리 |
|---|---:|---:|
| 64장 전달·디코딩 | 9.685초 | 0.576초 / 반복 0.604초 |
| 처리 직후 렌더러 private memory | 445.3MiB | 52.0MiB / 반복 63.2MiB |
| 처리 직후 JS 사용 힙 | 364.4MiB | 0.90MiB / 반복 0.97MiB |

처리 직후 수치는 강제 GC 전의 표본이며 최고점 측정이 아니다. 각 비교의 시작 조건을
맞추기 위해 **테스트에서만** 단계 사이 GC를 수행했다. 실제 앱에서 주기적 강제 GC를
수행하는 패치는 넣지 않았다. 786,526바이트의 이미지가 기존 숫자 배열 JSON에서는
2,808,351바이트가 되었고 Windows 스크립트 문자열 변환 비용도 추가된다.
새 방식 128건은 모두 본문 수신·반납했고 잔여 전달 토큰/바이트는 0이었다.

이 결과는 이미지 전달 구간의 일시적 메모리 증폭과 비용 감소를 확인한 것이다.
전체 다운로드가 16배 빨라졌다는 뜻도, 어제의 장기 메모리 증가 원인을 모두 증명했다는
뜻도 아니다. 실사용 장시간 재발 여부와 남은 메모리 소유자는 새 계측으로 확인해야 한다.

재현 코드: `src-tauri/examples/thumbnail_transport_probe.rs`, 같은 이름의 `.js`.

## 히토미 다운로드 중단 조사

9월 27일 앱 종료 후 SQLite를 `mode=ro` + `query_only`로 확인했다. 기준 DB는
`%APPDATA%\local.atsumi.next\atsumi-next.sqlite3`이며 조사 전후 상태 수가 동일했다.

### 현재 상태

- 완료 6,817 / 실패 4 / hashing 상태 5 / 검토 필요 172 / 격리 403 / 취소 416.
- 앱은 꺼져 있으므로 hashing 5개가 현재 CPU에서 계산 중이라는 뜻은 아니다.
  다음 앱 시작 때 복구될 이전 실행 상태다.
- 취소·검토·격리는 실패와 별개다. 이를 모두 다운로드 오류로 합산하면 안 된다.

9월 25일 이후 `JOB_INTERRUPTED` 137건은 모두 **9월 27일 13:26:39 KST**에 기록됐다.
이는 그 순간 137개의 전송이 각각 실패했다는 뜻이 아니다. 시작 복구 코드가 이전
실행의 미종료 작업(대기/메타데이터/다운로드/해시/검증/재시도)을 일괄 정리한 기록이다.
일반 종료·강제 종료·백엔드 종료의 어느 하나였는지는 이 코드만으로 특정할 수 없다.
화면만 F5로 새로고침하는 동작 자체가 이 DB 복구를 발생시키는 것은 아니다.

그 137개 작업의 현재 분포는 완료 108, 취소 11, 격리 3, 검토 필요 8, hashing 5,
실패 2다. 따라서 상당수는 이미 재개 후 완료됐지만 전부 해결된 상태는 아니다.

### 현재 실패 4건

| 작품 | 확인한 원인 | 현재 판단 |
|---|---|---|
| 2897405 | 최근 `IMAGE_CANDIDATES_EXHAUSTED` 8회. 11·14페이지의 HTTP 200 WebP를 디코딩하지 못함 | 현재도 재현. 아래의 잘린 응답 확인 |
| 4180352 | 최근 `SOURCE_INVALID_DATA` 4회. 요청 ID 4180352와 서버 메타데이터 ID **4214994** 불일치 | 현재도 재현. 다른 작품으로 잘못 저장하는 것을 막는 검증이며 임의로 우회하지 않음 |
| 2406818, 3668987 | 8월 28일의 `DOWNLOAD_OVERLAP_CHECK_FAILED` | 오래된 미해결 상태. DB에 페이지 15/15·5/5가 등록돼 있지만 이번에 실제 중복 판정을 재실행하지 않아 해결 여부를 확정하지 않음 |

2897405는 이미지 다운로드 어댑터와 같은 Referer로 해당 페이지의 WebP 후보 경로만
제한적으로 확인했다. 1차 경로는 HTTP 200, 나머지 WebP 후보들은 404였다.

| 페이지 | 이미지 헤더가 선언한 전체 길이 | 실제 응답 길이 | 남은 구간 요청 |
|---|---:|---:|---|
| 11 | 1,337,452바이트 | 1,048,576바이트 | HTTP 416, `bytes */1048576` |
| 14 | 1,521,326바이트 | 1,048,576바이트 | HTTP 416, `bytes */1048576` |

두 응답 모두 Content-Length와 ETag가 1MiB 파일을 가리킨다. 서버/CDN이 현재 제공하는
리소스 자체가 이미지 내부 길이보다 짧으며, 단순히 이 PC가 수신을 중간에 멈춘 것으로
설명되지 않는다. 실제 원본 서버 디스크 상태까지 검사한 것은 아니므로 서버 파일과
중간 캐시 중 정확히 어느 저장 지점이 손상됐는지는 확정하지 않는다. 메모리·CPU·HDD
예산을 늘리거나 디코더 한도를 풀어 해결할 유형은 아니다.

조사 코드: `src-tauri/examples/hitomi_source_diagnostic.rs`. 기본 메타데이터 검사,
`--primary-images`는 지정한 두 페이지의 WebP 후보만, `--tail-check`를 추가하면
1차 경로와 누락 범위 확인만 수행한다. 이미지·메타데이터를 파일로 저장하거나 DB를
열지 않는다. 일반 앱 시작 경로에서 실행되지 않는다.

### 함께 보완한 오류 기록과 남은 일

- 원래 페이지별 기록에는 오류 코드만 남고 디코더 상세 사유는 버려졌다. 앞으로는
  파서/디코더의 제한된 상세 사유(최대 256문자, URL 제외)도 기록한다.
- WebP의 RIFF 길이를 먼저 대조해, 잘린 응답을 일반 디코더 오류가 아니라
  **선언 길이/수신 길이가 다른 응답**으로 구분한다. 손상 데이터를 완료 처리하거나
  SHA-256·원본 동일성·무결성 검증을 생략하지 않는다.
- 4180352의 다른 ID를 자동으로 따라가거나 기존 파일을 다른 작품에 재귀속하지 않았다.
  번호 변경을 지원한다면 별도 확인과 이력 보존이 필요하다.
- 오래된 중복 비교 실패 2건은 선택적으로 다시 비교할 수 있지만 이번 조사에서 전체
  라이브러리 검사나 재다운로드를 시작하지 않았다.
- 기존 실패 건의 DB 메시지와 이력을 덮어쓰지 않았다. 더 구체적인 기록은 이후 새
  실패부터 남는다. 모든 실패가 이번 변경으로 해결됐다고 볼 수는 없다.

## 검증 및 전달 상태

- Rust 전체: **898 통과 / 26개 opt-in 등 무시 / 실패 0**.
- TypeScript 타입 검사와 Vite 빌드 성공.
- 프런트엔드 전체: 1,287 통과, Edge 헤드리스 DOM 측정 실패 12개.
  해당 12개를 임시 Chrome 프로필로 순차 재실행해 모두 통과했다.
  테스트 단언을 삭제하거나 실패를 skip으로 바꾸지 않았다.
- 바이너리 수신 중 취소·길이 불일치·늦은 이벤트, 진행률 리비전, 큐 한도/재동기화,
  복구 횟수·백그라운드·절전 유예, 검색 상태 저장 검증 포함.
- `cargo build --bin atsumi` 성공. 바탕화면 **Atsumi (디버그)** 바로가기가 `2849`의
  `tools/start_debug_app_hidden.ps1`로 연결된 것도 확인했다.
- 실사용 장시간 실행을 자동 시작하지 않았다. 앱을 켜면 남아 있는 이전 작업들이
  원래 복구 정책에 따라 재개될 수 있다. 릴리즈 설치본에는 아직 배포하지 않았다.
