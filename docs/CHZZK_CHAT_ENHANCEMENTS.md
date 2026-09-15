# CHZZK 채팅 표시·프로필·시청자 관측

2026-09-12. 사용자 승인 후 추가한 기능이다. 실제 계정·녹화·채팅을 테스트 자료로 사용하지 않는다.

## 확인한 공식 데이터 계약

공식 [CHZZK](https://chzzk.naver.com/)가 제공한 공개 정적 [index-C4sif-4p.js](https://ssl.pstatic.net/static/nng/glive/resource/p/static/js/index-C4sif-4p.js)를 읽어 아래를 확인했다. 번들을 앱에 저장하거나 원격 코드를 재실행하지 않는다.

- CleanBot: 컴포넌트 초기값은 `localStorage.getItem('cleanbot') !== 'false'`; 공식 확인 버튼이 `true`/`false`를 저장한다. Atsumi 초기화는 키가 **없을 때만** `false`를 설정한다. 기존 선택·이후 수동 변경·알 수 없는 값은 덮어쓰지 않는다. 팝업·서버 설정·로그인 API를 조작하지 않는다.
- 최초 필터 안내: `#live-chatting`의 `_container_s1cb2_1._filter_s1cb2_22` 안에 사용자가 지정한 문구 전체가 있는 안내문만 숨긴다. 일반 채팅·운영 공지·약관/동의창은 대상이 아니다. 안내문 숨김은 필터 해제가 아니다.
- 주간 후원: `_container_wl8bq_2`의 접힌 `_ranking_button_wl8bq_141[aria-expanded=false]`만 축소한다. 일반 TOP3 레이아웃의 rank/name/amount 또는 축소 marquee의 **치즈 아이콘으로 확인된** 후원 항목만 사용한다. 통나무 파워를 후원으로 표시하지 않는다. 없는 순위를 생성하지 않고, 중복 marquee 항목은 제거한다. 4초 순환, hover/focus·모션 감소·숨김 상태에서는 순환을 멈춘다. 클릭하면 원래 React 소유 버튼의 펼침 동작을 이용하며 확장 뷰를 복제하지 않는다. 새 후원/랭킹 API 요청은 없다.
- 본문 색: 일반 채팅의 공식 `span`은 `profile.title.color`를 사용한다. 저장은 `#RRGGBB`만 허용한다. 닉네임의 literal 색과 확인된 `CD001..CD040`의 dark 팔레트도 보존한다. 기본색은 공식 `iv(profile, chatChannelId, v_.dark)` 계산(두 ID의 UTF-16 문자합을 40으로 나눈 나머지)을 재현한다. 기존 방송 시작 시각용 live-detail 응답의 검증된 chatChannelId만 메모리에 유지하고, bridge는 실제 받은 userIdHash의 문자합에서 0..39 색상 seed만 전달한다. chatChannelId와 seed는 저장하지 않고 최종 색상만 저장한다. 추가 조회는 없고, 메타데이터 도착 전/실패/ID 누락 시 기본색은 unknown이다. 실제 chatChannelId 대신 방송 채널 ID나 닉네임을 사용하지 않는다. 임의 symbolic 색과 동적 특수 효과는 재현하지 않는다.
- 공개 프로필: 공식 profile-card 버튼·랭킹 링크가 `/${userIdHash}`를 사용한다. 새 일반 채팅에서 실제 받은 32자리 hex ID만 `https://chzzk.naver.com/<id>`로 정규화한다. 프로필 API나 계정 정보를 추가 조회하지 않는다.
- 시청자 수: 공식 live video-info가 `status === OPEN && cvExposure`일 때 `concurrentUserCount`를 `._container_17x81_2 ._data_17x81_70 > strong._count_17x81_83`에 출력한다. VOD·추천 목록·업타임 span은 제외한다. 공개 DOM에서 `278명 시청 중` 형식과 안내문 구조가 추가 확인되었다. 축약값(예: 1.2만), 복수 후보, 누락은 unknown이다.

클래스는 이 시점의 공식 코드에서 확인된 값이다. 바뀐 구조를 넓은 텍스트 검색으로 억지 매칭하지 않고 원본 표시/unknown으로 남긴다. 주간 랭킹의 원래 펼침 동작과 정확한 확인된 selector를 유지하는 회귀 자료 갱신이 필요할 수 있다.

## 저장·개인정보 경계

`ChatRich`의 선택 필드 `textColor`, `profileUrl`을 추가했다. 잘못된 선택 필드는 기본 텍스트 읽기를 막지 않는다. 과거 JSONL을 수정하거나 누락 데이터를 소급 생성하지 않는다.

`senderKey`는 여전히 녹화별 salt 기반 digest이며 원래 ID/salt 필드를 저장하지 않는다. 그러나 **공개 프로필 URL 자체는 개인 식별성을 유지하고 서로 다른 녹화의 동일 계정을 연결할 수 있다**. 이는 프로필 링크 보존 요청에 따른 기존 최소수집 정책의 명시적 확장이다. 원시 profile·인증 토큰·쿠키·이메일은 저장하지 않는다. URL을 익명 데이터라고 설명하면 안 된다.

프로필 열기는 `replay_open_profile(token, sequence)`에서 해당 세션의 저장 행을 찾고 정확한 공개 URL을 다시 검증한다. caller가 임의 URL을 넘길 수 없다. 색은 CSS/HTML 문자열로 해석하지 않으며, 저장된 HTML을 실행하지 않는다. 재생·색인 중 프로필을 자동 방문하지 않고 사용자의 클릭으로만 외부 기본 브라우저를 연다. 과거에 링크가 없는 닉네임은 링크 없이 표시한다.

새 꾸밈 이미지는 기존 공용 캐시와 별도로 녹화 폴더 `replay-assets/<assetId>.json`에 검증된 immutable 사본을 보조 저장한다. 한 worker·128 대기열, 공용 및 각 녹화 최대 4096파일/128MiB, 단일 저장 파일 1.5MB 한도를 적용한다. worker의 녹화별 예산 추적은 최대 64개 녹화이고, 한계·다운로드·복사 실패가 녹화나 채팅 저장을 멈추지 않는다. 재생은 해당 로그에서 확인된 asset ID에 대해 녹화 사본을 우선 사용하고 공용 캐시를 보조 사용한다. 빠진 이미지는 문자로 표시한다. 같은 URL의 이미지가 나중에 바뀌거나 최초 저장에 실패한 경우 원래 모습을 완벽히 복원할 수 없다.

## 시청자 시계열

2026-09-13 확인: 위 공개 index의 `np` 함수는 `jo` 클라이언트(`sa(POLLING, 'v3.1')`)로 `GET https://api.chzzk.naver.com/polling/v3.1/channels/{channelId}/live-status`를 호출한다. 원래 플레이어 effect의 `Ce=3e4`는 **30초**이며 실패가 20회 넘으면 폴링을 중단하는 분기도 있다. 따라서 이전 2초 DOM 관측은 페이지가 갱신할 때까지 같은 30초 스냅샷을 반복 저장했다. 이것이 이번 변경에서 확인된 갱신 주기 차이이며, 특정 사용자 세션에서 공식 폴링 중단이 실제 발생했다고 단정하지 않는다.

같은 공개 경로를 쿠키 없이 읽기 전용으로 조회해 HTTP 200, 약 1.9KiB JSON, CHZZK origin CORS 허용, `Cache-Control: no-cache, no-store, max-age=0, must-revalidate`를 확인했다. 공개 응답의 `livePollingStatusJson.callPeriodMilliSecond`는 10000이었다. 이는 공식 화면 effect의 30초와 별도 필드이며 서버 집계 자체가 10초마다 반드시 바뀐다는 보장은 아니다. 조사 중 실제 계정·사용자 녹화·채팅 자료는 조회/수집하지 않았다.

새 녹화는 기존 채팅 저장 동의(`captureChat`) 후에만 이 **공개 메타데이터 API**를 즉시 1회 및 10초마다 직접 조회한다. `credentials: omit`, `cache: no-store`, `redirect: error`, `referrerPolicy: no-referrer`를 지정하고 인증 헤더/쿠키를 읽거나 전달하지 않는다. 요청은 하나만 진행하며 8초 timeout으로 abort한다. JSON content-type 및 32KiB 스트리밍 본문 상한을 검사한다. 종료·채널 변경·저장 실패 시 타이머/진행 중 요청을 해제하며 늦은 응답은 무시한다. 추가 영상 수신, 플레이어, 채팅 소켓, 광고/추천 조회는 만들지 않는다.

응답의 `channelId`, `status=OPEN`, `openDate`(공식 KST 형식), `cvExposure=true`, 정수 수치를 확인한다. 첫 유효 `openDate`로 녹화의 방송 세대를 고정하며 바뀌면 null을 기록하고 이 녹화의 추가 조회를 중단한다. 요청 사이 seek/source 세대 변경도 해당 응답을 unknown으로 처리한다. 네이티브는 요청 채널 및 이미 확보한 방송 시작 시각과 추가 대조한다. 숨긴 시청자 수, 통신 실패, 제한, 형식 변경은 이전 DOM/응답 값을 다시 저장하지 않고 **null**로 남긴다. 응답 시각의 재생 시계를 기존 ordered queue에 넣으며 요청 시작 시각을 지연된 응답 시각처럼 사용하지 않는다.

기존 순서 보장 채팅 큐/bridge를 재사용하되 일반 메시지와 구분해 `viewer-metrics.jsonl`에 저장한다. viewer-only 배치는 채팅 수나 채팅 연결 상태를 증가시키지 않는다.

각 행은 version=1, source(신규 `chzzk_live_status_api_v1`, 기존 `chzzk_video_info_dom_v1`), receivedAt, offsetSeconds, viewerCount(number|null), 기존 검증 계약의 replayClock을 가진다. 원본 media clock이 검증되면 같은 병합 PTS 대응을 사용하며 그 외는 수신 시각 근사다. 실제 0은 0으로, 표시되지 않는 수치는 null로 기록한다. 네이티브는 시각/세대/source ID/수치 범위를 검증하고 1초 미만 과다 샘플을 저장하지 않는다. 한 파일 최대 32MiB, 한 행 2048bytes다. 전체 API 응답, liveTokenList, chatChannelId, 인증 정보는 저장하지 않는다. 방송 세대 대조에 쓰는 시청자 이벤트의 channel/openDate 역시 viewer JSONL에는 복제하지 않는다.

파생 인덱스 v3는 최대 200,000 샘플을 제한된 메모리로 읽고 시간 순으로 색인하며 샘플별 `hold_seconds`를 갖는다. 원본 변화는 세션을 무효화하며 원본은 수정하지 않는다. 이전 파생 캐시는 필요 시 다시 만들지만 원본 JSONL은 수정하지 않는다. 빈 과거 자료는 `not_recorded`; 관측이 부분적이면 `partial`이다.

`replay_timeline`의 기존 viewerCount(number|null)에 구간별 시간 가중 평균(반올림)을 담고, viewerSampleCount와 viewerCoverageSeconds를 함께 반환한다. sample count는 **성공한 관측 횟수**이지 서버 갱신 수나 시청자 수가 아니다. 신규 API 값은 다음 관측 또는 최대 15초(10초 주기+5초 일정 지연 여유)까지만 유지한다. 검증된 원본 media 매핑이 있을 때만 이 wall-time 한도를 해당 playbackRate로 환산한다(최대 60 media초); 수신 시각 근사는 배속과 무관하게 15초다. 기존 DOM 값의 6초 한도는 변경하지 않는다. 이는 제한된 sample-and-hold 가정이지 지속 관측이나 그 사이 실제 수치의 보증이 아니다. 명시적 null과 더 긴 공백은 연결하지 않는다. 구간의 관측 범위가 없으면 null이며 0으로 채우지 않는다. 통계적인 서버 측 시청자 수, 시청 유지율, 채팅 작성자 수와 동일한 값이라고 설명하지 않는다. 채팅 시간 수동 보정은 동일 관측 타임라인에도 적용된다.

timeline API는 1초 이상 요청 및 최대 2000구간을 허용한다. UI가 2초를 요청할 수 있지만 긴 영상에서는 응답 크기를 위해 더 큰 구간을 반환한다. 더 부드러운 선을 그려도 관측 정밀도가 높아지는 것은 아니다.

## 합성 회귀

JS는 CleanBot 기본값/사용자 선택, 정확한 안내문 식별, 후원과 통나무 파워 구분, 원래 펼침 handler, TOP3 순환·중복 제거, 실제0/unknown/VOD 제외, opt-in/10초 API 관측/timeout/종료/추가 socket 없음, source/seek/channel/live 세대 변경, 숨김·손상·과대 응답, 공개 URL·색과 credentials 배제 등을 합성으로 검증한다.

Rust는 optional legacy 호환, 기본색의 검증된 메모리 전용 채팅 ID와 제한된 seed, 공개 URL allowlist·저장 행 권한·손상된 대형 payload의 SQL 단계 읽기 상한, viewer-only 채팅 상태 불변, 기록/표시 시각 대응, 시간 가중 평균·null 공백·6초 한도·파일 변경, 녹화별 이미지 사본/상한/기존 파일 비대체를 합성 자료로 검증한다. 현대 senderKey 로그의 빈 구간 참여자는 0이지만, ID가 없는 메시지 구간과 과거 전체 미수집 자료는 unknown으로 남는다. 실제 사용자 앱/계정/방송/녹화는 이 검증에 사용하지 않는다.
